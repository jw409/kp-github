"""Offline archive behavior using a disposable source checkout and fake binary."""

import hashlib
import importlib.util
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest import mock


REPOSITORY = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("package_consumer", REPOSITORY / "scripts/package_consumer.py")
package = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(package)


class ConsumerPackageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.source = self.directory / "source"
        self.source.mkdir()
        for relative in package.REQUIRED_FILES:
            destination = self.source / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            if relative.startswith("scripts/"):
                shutil.copyfile(REPOSITORY / relative, destination)
            else:
                destination.write_text(f"Fixture documentation: {relative}\n")
        # Only this exact optional sample is eligible; nearby personal files are not.
        example = self.source / "examples/github-events/meshly.sample.json"
        example.parent.mkdir(parents=True)
        example.write_text('{"example": true}\n')
        (self.source / "Cargo.toml").write_text('[package]\nname = "fixture"\n')
        self.git("init", "--quiet")
        self.git("add", ".")
        self.git("-c", "user.name=Package Fixture", "-c", "user.email=fixture@example.test",
                 "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", "commit", "--quiet", "-m", "fixture")
        self.commit = self.git("rev-parse", "HEAD").strip()
        self.binary = self.directory / "built-executable"
        self.binary.write_bytes(b"#!/bin/sh\nprintf 'synthetic binary\\n'\n")
        self.binary.chmod(0o755)
        self.receipt = {"source_commit": self.commit,
            "source_inputs_sha256": package._provenance.source_digest(self.source, self.commit),
            "target": "x86_64-unknown-linux-gnu", "rust_version": "1.99.0",
            "binary_sha256": hashlib.sha256(self.binary.read_bytes()).hexdigest()}
        self.binary.with_suffix(".provenance.json").write_text(json.dumps(self.receipt))
        self.output = self.source / "dist"
        self.target = "x86_64-unknown-linux-gnu"
        self.root_name = "kp-github-consumer-" + self.target

    def git(self, *args):
        # Fixture-only commits are isolated from user Git config and hooks.
        return subprocess.run(
            [shutil.which("git"), "-C", str(self.source), *args], check=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            env={"PATH": os.defpath, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
                 "GIT_AUTHOR_DATE": "2000-01-01T00:00:00Z", "GIT_COMMITTER_DATE": "2000-01-01T00:00:00Z"},
        ).stdout

    def build(self, **overrides):
        arguments = dict(binary=self.binary, target=self.target, source_root=self.source,
                         output_dir=self.output, expect_commit=self.commit)
        arguments.update(overrides)
        return package.build_package(**arguments)

    def test_reproducible_bytes_and_normalized_metadata(self):
        archive, checksum = self.build()
        first = archive.read_bytes()
        first_checksum = checksum.read_bytes()
        os.utime(self.binary, (1000000000, 1000000000))
        self.binary.chmod(0o700)
        for relative in package.REQUIRED_FILES:
            os.utime(self.source / relative, (1200000000, 1200000000))
        self.build()
        self.assertEqual(archive.read_bytes(), first)
        self.assertEqual(checksum.read_bytes(), first_checksum)
        self.assertEqual(first[4:8], b"\x00" * 4)  # gzip MTIME
        with tarfile.open(archive, "r:gz") as tar:
            for member in tar.getmembers():
                self.assertEqual((member.uid, member.gid, member.uname, member.gname, member.mtime), (0, 0, "", "", 0))
                expected_mode = 0o755 if member.isdir() or member.name.endswith(("bin/kp-github-mcp", "scripts/github_events_setup.py", "scripts/setup_shared_http.py", "scripts/install_local_http.py", "scripts/install.sh")) else 0o644
                self.assertEqual(member.mode, expected_mode)
                self.assertTrue(member.isfile() or member.isdir())
                self.assertFalse(PurePosixPath(member.name).is_absolute())
                self.assertNotIn("..", PurePosixPath(member.name).parts)
                self.assertEqual(PurePosixPath(member.name).parts[0], self.root_name)

    def test_self_contained_extraction_helper_and_hashes(self):
        archive, checksum = self.build()
        expected, filename = checksum.read_text().strip().split("  ")
        self.assertEqual(filename, archive.name)
        self.assertEqual(hashlib.sha256(archive.read_bytes()).hexdigest(), expected)
        extracted = self.directory / "extracted"
        with tarfile.open(archive, "r:gz") as tar:
            tar.extractall(extracted, filter="data")
        root = extracted / self.root_name
        manifest = json.loads((root / "MANIFEST.json").read_text())
        self.assertEqual(manifest["source_commit"], self.commit)
        self.assertFalse(manifest["source_modified"])
        self.assertEqual(manifest["target"], self.target)
        self.assertEqual(manifest["binary_sha256"], hashlib.sha256(self.binary.read_bytes()).hexdigest())
        for entry in manifest["files"]:
            path = root / entry["path"]
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), entry["sha256"])
            self.assertEqual(path.stat().st_size, entry["size"])
            self.assertEqual(f"{stat.S_IMODE(path.stat().st_mode):04o}", entry["mode"])
        for line in (root / "SHA256SUMS").read_text().splitlines():
            expected, name = line.split("  ")
            self.assertEqual(hashlib.sha256((root / name).read_bytes()).hexdigest(), expected)
        # This executable source helper needs no checkout, credentials, or network.
        result = subprocess.run([sys.executable, str(root / "scripts/github_events_setup.py"), "--help"],
                                cwd=extracted, env={}, capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("setup", result.stdout)
        shared = subprocess.run([sys.executable, str(root / "scripts/setup_shared_http.py"),
                                 "--directory", str((self.directory / "shared").resolve())],
                                cwd=extracted, env={}, capture_output=True, text=True, timeout=10)
        self.assertEqual(shared.returncode, 0, shared.stderr)
        self.assertTrue((self.directory / "shared/http.key").is_file())

    def test_private_and_unlisted_files_never_enter_archive_or_manifest(self):
        for relative in (".config/events.json", "credentials.json", "private-key.pem",
                         "examples/github-events/events.json", "docs/private.md"):
            path = self.source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("synthetic-secret-marker")
        archive, _ = self.build()
        with tarfile.open(archive, "r:gz") as tar:
            names = tar.getnames()
            self.assertIn(self.root_name + "/examples/github-events/meshly.sample.json", names)
            for member in tar.getmembers():
                if member.isfile():
                    data = tar.extractfile(member).read()
                    self.assertNotIn(b"synthetic-secret-marker", data)
                    self.assertNotIn(str(self.source).encode(), data)
            manifest = json.load(tar.extractfile(self.root_name + "/MANIFEST.json"))
        self.assertFalse(manifest["source_modified"])
        self.assertFalse(any(".git/" in name or ".config/" in name or "credentials" in name for name in names))

    def test_modified_source_is_disclosed_without_claiming_binary_attestation(self):
        (self.source / "docs/account-selection.md").write_text("modified fixture\n")
        archive, _ = self.build()
        with tarfile.open(archive, "r:gz") as tar:
            manifest = json.load(tar.extractfile(self.root_name + "/MANIFEST.json"))
        self.assertTrue(manifest["source_modified"])
        self.assertIn("not inferred", manifest["provenance"])

    def test_expected_commit_and_checkout_root_are_enforced(self):
        for overrides in ({"expect_commit": "0" * 40}, {"source_root": self.source / "docs"}):
            with self.subTest(overrides=overrides), self.assertRaises(package.PackageError):
                self.build(**overrides)
        self.assertFalse(self.output.exists())

    def test_ambient_git_dir_cannot_change_source_revision(self):
        with mock.patch.dict(os.environ, {"GIT_DIR": str(self.directory / "not-a-repo")}, clear=True):
            archive, _ = self.build()
        self.assertTrue(archive.is_file())

    def test_binary_must_be_explicit_nonempty_executable_regular_file(self):
        self.binary.chmod(0o644)
        with self.assertRaises(package.PackageError):
            self.build()
        self.binary.chmod(0o755)
        self.binary.write_bytes(b"")
        with self.assertRaises(package.PackageError):
            self.build()
        self.binary.unlink()
        self.binary.symlink_to(self.source / "docs/account-selection.md")
        with self.assertRaises(package.PackageError):
            self.build()
        self.assertFalse(self.output.exists())

    def test_required_payload_and_optional_parent_symlinks_are_rejected(self):
        helper = self.source / "scripts/github_events_setup.py"
        original = helper.read_bytes()
        helper.unlink()
        helper.symlink_to(self.directory / "missing-private-file")
        with self.assertRaises(package.PackageError):
            self.build()
        helper.unlink()
        helper.write_bytes(original)
        examples = self.source / "examples/github-events"
        shutil.rmtree(examples)
        outside = self.directory / "outside"
        outside.mkdir()
        (outside / "meshly.sample.json").write_text("synthetic-secret-marker")
        examples.symlink_to(outside, target_is_directory=True)
        with self.assertRaises(package.PackageError):
            self.build()
        self.assertFalse(self.output.exists())

    def test_changed_binary_and_forged_source_commit_are_rejected(self):
        original = self.binary.read_bytes()
        self.binary.write_bytes(original + b"X")
        with self.assertRaises(package.PackageError):
            self.build()
        self.binary.write_bytes(original)
        self.receipt['source_commit'] = '0' * 40
        self.binary.with_suffix('.provenance.json').write_text(json.dumps(self.receipt))
        with self.assertRaises(package.PackageError):
            self.build()
        self.assertFalse(self.output.exists())

    def test_dirty_runtime_and_missing_receipt_cannot_be_packaged(self):
        (self.source / 'Cargo.toml').write_text('changed build input')
        with self.assertRaises(package.PackageError):
            self.build()
        self.binary.with_suffix('.provenance.json').unlink()
        with self.assertRaises(package.PackageError):
            self.build()

    def test_unsafe_archive_target_is_rejected(self):
        for target in ("../escape", "/absolute", "linux x86", "linux/../../escape", "--target"):
            with self.subTest(target=target), self.assertRaises(package.PackageError):
                self.build(target=target)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
