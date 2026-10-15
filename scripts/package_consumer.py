#!/usr/bin/env python3
"""Package an explicitly built kp-github binary and its offline setup helpers."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import io
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess
import sys
import tarfile
import tempfile


_SPEC = importlib.util.spec_from_file_location("binary_provenance", Path(__file__).with_name("binary_provenance.py"))
_provenance = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_provenance)


REQUIRED_FILES = (
    "scripts/github_events_setup.py",
    "scripts/setup_shared_http.py",
    "docs/shared-local-server.md",
    "docs/developer-events-setup.md",
    "docs/listener-api.md",
    "docs/account-selection.md",
    "docs/consumer-package.md",
)
# Explicit names keep personal profiles and credentials out of consumer bundles.
OPTIONAL_FILES = (
    "LICENSE",
    "docs/gcs-config-contract.md",
    "examples/github-events/README.md",
    "examples/github-events/meshly.sample.json",
    "examples/github-events/meshly.sample.bundle",
    "examples/github-events/selfhost.sample.json",
    "examples/github-events/selfhost.sample.bundle",
    "examples/github-events/shared-config.schema.json",
    "examples/github-events/receiver-inputs.sample.json",
)
SOURCE_PATHS = ("Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "build.rs", ".cargo", "src", "scripts/package_consumer.py",
                *REQUIRED_FILES, *OPTIONAL_FILES)
README = """kp-github consumer package

Nothing runs when this archive is extracted. No account or MCP configuration
is changed by packaging or extraction.

Verify the archive's .sha256 sidecar before extraction, then verify SHA256SUMS
inside the extracted package. Read docs/consumer-package.md for commands.

The executable is bin/kp-github-mcp. The Python 3 standard-library helper is
scripts/github_events_setup.py; its --help works without a source checkout.
Use your own existing gh and gcloud credentials. App signing keys, webhook
secrets, tokens, and local profiles are not included.

Event delivery still requires your operator's deployed consumer API and
verified identity mapping. See docs/listener-api.md for that boundary.
""".encode()


class PackageError(Exception):
    """An actionable packaging failure without subprocess output."""


def _git(root: Path, *arguments: str) -> str:
    try:
        result = subprocess.run(
            ["git", "-C", str(root), *arguments], stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, check=False, timeout=10,
            env={key: value for key, value in os.environ.items() if not key.startswith("GIT_")},
        )
    except (OSError, subprocess.TimeoutExpired):
        raise PackageError("Could not read the source checkout's Git metadata.") from None
    if result.returncode:
        raise PackageError("Source root must be a Git checkout with a committed HEAD.")
    return result.stdout.decode("utf-8").strip()


def _source_file(root: Path, relative: str) -> Path:
    path = PurePosixPath(relative)
    if path.is_absolute() or ".." in path.parts:
        raise PackageError("Payload paths must remain inside the source checkout.")
    candidate = root
    for part in path.parts:
        candidate = candidate / part
        if candidate.is_symlink():
            raise PackageError(f"Payload files cannot traverse symlinks: {relative}")
    if not candidate.is_file():
        raise PackageError(f"Missing required payload file: {relative}")
    return candidate


def _digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def _atomic_write(path: Path, data: bytes) -> None:
    temporary: str | None = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=".kp-package-", delete=False) as output:
            temporary = output.name
            output.write(data)
            output.flush()
            os.fchmod(output.fileno(), 0o644)
            os.fsync(output.fileno())
        os.replace(temporary, path)
    finally:
        if temporary is not None:
            try:
                os.unlink(temporary)
            except FileNotFoundError:
                pass


def build_package(binary: Path, target: str, source_root: Path, output_dir: Path,
                  expect_commit: str | None = None) -> tuple[Path, Path]:
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]{1,79}", target):
        raise PackageError("Target must be a plain Rust target triple or platform identifier.")
    root = source_root.resolve(strict=True)
    if Path(_git(root, "rev-parse", "--show-toplevel")).resolve() != root:
        raise PackageError("Source root must name the checkout root, not a subdirectory.")
    commit = _git(root, "rev-parse", "--verify", "HEAD^{commit}")
    if not re.fullmatch(r"[0-9a-f]{40,64}", commit):
        raise PackageError("Could not determine a full source commit ID.")
    if expect_commit is not None and expect_commit != commit:
        raise PackageError("The expected source commit does not match this checkout's HEAD.")
    # Do not let generated output or unrelated private files alter reproducibility
    # or appear in metadata. The build inputs and all permitted payloads count.
    dirty = bool(_git(root, "status", "--porcelain=v1", "--untracked-files=all", "--", *SOURCE_PATHS))
    if binary.is_symlink() or not binary.is_file():
        raise PackageError("--binary must be an explicit regular file, not a symlink.")
    if not stat.S_IMODE(binary.stat().st_mode) & 0o111:
        raise PackageError("--binary must be executable; build the selected target first.")
    binary_bytes = binary.read_bytes()
    if not binary_bytes:
        raise PackageError("--binary must not be empty.")
    # The build helper creates this receipt only after compiling committed source
    # in a fresh directory. It is integrity metadata, not a cryptographic attestation.
    try:
        receipt = json.loads(_provenance.regular(binary.with_suffix(".provenance.json")))
        _provenance.validate_receipt(root, binary, receipt, target, require_current=True)
    except (_provenance.ProvenanceError, OSError, ValueError, AttributeError) as error:
        raise PackageError("A valid current-source build receipt is required: " + str(error)) from None
    payload: dict[str, tuple[bytes, int]] = {
        "bin/kp-github-mcp": (binary_bytes, 0o755),
        "README.txt": (README, 0o644),
    }
    for relative in REQUIRED_FILES:
        mode = 0o755 if relative.startswith("scripts/") else 0o644
        payload[relative] = (_source_file(root, relative).read_bytes(), mode)
    for relative in OPTIONAL_FILES:
        candidate = root / relative
        if candidate.exists() or candidate.is_symlink():
            payload[relative] = (_source_file(root, relative).read_bytes(), 0o644)
    manifest = {
        "schema_version": 1,
        "package": "kp-github-consumer",
        "target": target,
        "source_commit": commit,
        "source_modified": dirty,
        "binary_sha256": _digest(binary_bytes),
        "binary_source_commit": receipt["source_commit"],
        "source_inputs_sha256": receipt["source_inputs_sha256"],
        "rust_version": receipt["rust_version"],
        "provenance": "Build receipt and current source inputs validated; source correspondence is established by the clean build and CI reproduction, not inferred from a checksum or treated as a signature.",
        "files": [
            {"path": name, "sha256": _digest(data), "size": len(data), "mode": f"{mode:04o}"}
            for name, (data, mode) in sorted(payload.items())
        ],
    }
    payload["MANIFEST.json"] = ((json.dumps(manifest, indent=2, sort_keys=True) + "\n").encode(), 0o644)
    checksums = "".join(f"{_digest(data)}  {name}\n" for name, (data, _) in sorted(payload.items()))
    payload["SHA256SUMS"] = (checksums.encode(), 0o644)

    archive_root = f"kp-github-consumer-{target}"
    directories = {archive_root}
    for name in payload:
        parent = PurePosixPath(archive_root, name).parent
        while str(parent) != ".":
            directories.add(str(parent))
            parent = parent.parent
    archive = io.BytesIO()
    # No current time, original filenames, uid/gid, or host paths in either layer.
    with gzip.GzipFile(fileobj=archive, mode="wb", filename="", mtime=0, compresslevel=9) as compressed:
        with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as tar:
            for directory in sorted(directories):
                info = tarfile.TarInfo(directory)
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                tar.addfile(info)
            for name, (data, mode) in sorted(payload.items()):
                info = tarfile.TarInfo(f"{archive_root}/{name}")
                info.size = len(data)
                info.mode = mode
                tar.addfile(info, io.BytesIO(data))
    output_dir.mkdir(parents=True, exist_ok=True)
    archive_path = output_dir / f"{archive_root}.tar.gz"
    checksum_path = output_dir / f"{archive_path.name}.sha256"
    archive_bytes = archive.getvalue()
    _atomic_write(archive_path, archive_bytes)
    _atomic_write(checksum_path, f"{_digest(archive_bytes)}  {archive_path.name}\n".encode())
    return archive_path, checksum_path


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True, help="Explicit built executable; never inferred from checked-in bin/")
    parser.add_argument("--target", required=True, help="Target triple of that executable")
    parser.add_argument("--source-root", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--output-dir", type=Path, default=Path("dist"))
    parser.add_argument("--expect-commit", help="Require this full Git commit, e.g. the workflow's checkout SHA")
    args = parser.parse_args(argv)
    try:
        archive, checksum = build_package(args.binary, args.target, args.source_root,
                                         args.output_dir, args.expect_commit)
    except (PackageError, OSError) as error:
        print(f"Package failed: {error}", file=sys.stderr)
        return 1
    print(archive)
    print(checksum)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
