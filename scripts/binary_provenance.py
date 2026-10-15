#!/usr/bin/env python3
"""Verify tracked binaries, reproduce their declared source, or build receipts.

Checksums detect changed bytes; only a clean rebuild comparison establishes
binary/source correspondence. CI runs both. This is not a signing authority.
"""
from __future__ import annotations
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile

TARGETS = {"linux-x86_64": "x86_64-unknown-linux-gnu", "macos-arm64": "aarch64-apple-darwin"}
INPUTS = ("src", "Cargo.toml", "Cargo.lock", "build.rs", "rust-toolchain.toml", ".cargo", "scripts/binary_provenance.py")
ROOT = Path(__file__).resolve().parents[1]

class ProvenanceError(Exception):
    pass

def git(root, *args):
    result = subprocess.run(["git", "-C", str(root), *args], stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, timeout=30, env={k:v for k,v in os.environ.items() if not k.startswith("GIT_")})
    if result.returncode:
        raise ProvenanceError("Source commit is unavailable or Git metadata is invalid (fetch full history).")
    return result.stdout

def sha(data):
    return hashlib.sha256(data).hexdigest()

def regular(path):
    if path.is_symlink() or not path.is_file():
        raise ProvenanceError("Binary and provenance must be regular files, not symlinks.")
    return path.read_bytes()

def source_digest(root, commit):
    # Object IDs include file bytes and the tree includes names/modes. Directory
    # traversal is Git's, not an unbounded walk through worktrees or secrets.
    return sha(git(root, "ls-tree", "-r", "-z", commit, "--", *INPUTS))

def validate_receipt(root, binary, receipt, target, require_current=False):
    if not isinstance(receipt, dict):
        raise ProvenanceError("Provenance must be a JSON object.")
    commit = receipt.get("source_commit", "")
    if not isinstance(commit, str) or not re.fullmatch(r"[0-9a-f]{40}", commit) or commit == "0" * 40:
        raise ProvenanceError("Provenance must contain a nonzero full source commit.")
    git(root, "cat-file", "-e", commit + "^{commit}")
    git(root, "merge-base", "--is-ancestor", commit, "HEAD")
    if receipt.get("target") != target:
        raise ProvenanceError("Binary target differs from provenance.")
    if receipt.get("binary_sha256") != sha(regular(binary)):
        raise ProvenanceError("Binary SHA256 differs from provenance.")
    if not binary.stat().st_mode & 0o111:
        raise ProvenanceError("Binary must be executable.")
    version = receipt.get("rust_version", "")
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        raise ProvenanceError("Provenance must pin a Rust toolchain version.")
    source = source_digest(root, commit)
    if receipt.get("source_inputs_sha256") != source:
        raise ProvenanceError("Source input digest differs from the declared commit.")
    if require_current:
        if source != source_digest(root, "HEAD") or git(root, "status", "--porcelain", "--untracked-files=all", "--", *INPUTS):
            raise ProvenanceError("Binary source is stale or the checkout has uncommitted build inputs; rebuild before consumption.")
    return receipt

def verify(root=ROOT, platform=None, require_current=False):
    manifest = json.loads(regular(root / "bin/consumer-provenance.json"))
    if not isinstance(manifest, dict) or manifest.get("schema_version") != 2 or not isinstance(manifest.get("platforms"), dict) or set(manifest["platforms"]) != set(TARGETS):
        raise ProvenanceError("Expected schema 2 provenance for both supported platforms.")
    for name in [platform] if platform else TARGETS:
        validate_receipt(root, root / "bin" / name / "kp-github-mcp", manifest["platforms"][name], TARGETS[name], require_current)
    return manifest

def clean_build(root, commit, target, rust_version, destination):
    # Build committed source in an isolated directory, with no inherited Rust
    # flags, compiler overrides or Cargo target cache. Never run git hooks.
    with tempfile.TemporaryDirectory(prefix="kp-reproduce-") as temporary:
        source = Path(temporary) / "source"
        source.mkdir()
        archive = git(root, "archive", "--format=tar", commit)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(source, filter="data")
        # No gh/GitHub/Google tokens are needed to compile public dependencies.
        env = {k:v for k,v in os.environ.items() if k in {"PATH", "HOME", "USER", "TMPDIR", "SYSTEMROOT", "CARGO_HOME", "RUSTUP_HOME"}}
        env.update(CARGO_TARGET_DIR=str(Path(temporary) / "target"), RUST_MIN_STACK="16777216")
        # Panic locations and C __FILE__ strings include the Cargo registry
        # path. Match the native Actions path on every workstation; do not let
        # the developer's home directory alter shipped bytes.
        cargo_home = Path(env.get("CARGO_HOME", str(Path(env["HOME"]) / ".cargo"))).resolve()
        canonical_cargo = "/Users/runner/.cargo" if "apple" in target else "/home/runner/.cargo"
        if str(cargo_home) != canonical_cargo:
            env["CARGO_ENCODED_RUSTFLAGS"] = f"--remap-path-prefix={cargo_home}={canonical_cargo}"
            env["CFLAGS"] = f"-ffile-prefix-map={cargo_home}={canonical_cargo}"
        result = subprocess.run(["cargo", "+" + rust_version, "build", "--locked", "--release", "--target", target], cwd=source, env=env)
        if result.returncode:
            raise ProvenanceError("Clean source build failed.")
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(Path(temporary) / "target" / target / "release/kp-github-mcp", destination)
        destination.chmod(0o755)

def reproduce(root, platform):
    receipt = verify(root, platform)["platforms"][platform]
    with tempfile.TemporaryDirectory(prefix="kp-compare-") as temporary:
        rebuilt = Path(temporary) / "kp-github-mcp"
        clean_build(root, receipt["source_commit"], TARGETS[platform], receipt["rust_version"], rebuilt)
        if sha(rebuilt.read_bytes()) != receipt["binary_sha256"]:
            raise ProvenanceError("Clean rebuild differs from checked-in binary; source correspondence is NOT verified.")

def build(root, target, output):
    if target not in TARGETS.values():
        raise ProvenanceError("Unsupported target.")
    if git(root, "status", "--porcelain", "--untracked-files=all", "--", *INPUTS):
        raise ProvenanceError("Commit build inputs before producing a source-bound binary.")
    commit = git(root, "rev-parse", "HEAD").decode().strip()
    # Pin in source rather than moving 'stable'; parse the narrow checked-in TOML.
    toolchain = git(root, "show", commit + ":rust-toolchain.toml").decode()
    match = re.search(r'channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', toolchain)
    if not match:
        raise ProvenanceError("rust-toolchain.toml must pin an exact release.")
    version = match[1]
    clean_build(root, commit, target, version, output)
    receipt = {"source_commit": commit, "source_inputs_sha256": source_digest(root, commit),
        "target": target, "binary_sha256": sha(output.read_bytes()), "rust_version": version}
    output.with_suffix(".provenance.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt

def assemble(root, artifacts):
    receipts = {}
    for platform, target in TARGETS.items():
        binary = artifacts / platform / "kp-github-mcp"
        receipt = json.loads(regular(binary.with_suffix(".provenance.json")))
        binary.chmod(0o755)  # Actions artifacts do not retain executable mode.
        validate_receipt(root, binary, receipt, target, require_current=True)
        receipts[platform] = receipt
    for platform in TARGETS:
        destination = root / "bin" / platform / "kp-github-mcp"
        shutil.copyfile(artifacts / platform / "kp-github-mcp", destination)
        destination.chmod(0o755)
    (root / "bin/consumer-provenance.json").write_text(json.dumps({"schema_version": 2, "platforms": receipts}, indent=2) + "\n")
    verify(root, require_current=True)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["verify", "reproduce", "build", "assemble"])
    parser.add_argument("--source-root", type=Path, default=ROOT)
    parser.add_argument("--platform", choices=TARGETS)
    parser.add_argument("--require-current", action="store_true")
    parser.add_argument("--target", choices=TARGETS.values())
    parser.add_argument("--output", type=Path)
    parser.add_argument("--artifacts", type=Path)
    args = parser.parse_args()
    try:
        if args.action == "verify": verify(args.source_root, args.platform, args.require_current)
        elif args.action == "reproduce" and args.platform: reproduce(args.source_root, args.platform)
        elif args.action == "build" and args.target and args.output: build(args.source_root, args.target, args.output)
        elif args.action == "assemble" and args.artifacts: assemble(args.source_root, args.artifacts)
        else: parser.error("action requires --platform, --target/--output, or --artifacts")
    except (ProvenanceError, OSError, ValueError, subprocess.SubprocessError) as error:
        parser.exit(1, f"Binary provenance failed: {error}\n")
    print("Binary provenance: " + args.action + " passed")

if __name__ == "__main__":
    main()
