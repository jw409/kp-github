"""Artifact tampering must fail the ordinary offline suite, before native CI."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("provenance", ROOT / "scripts/binary_provenance.py")
proof = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(proof)

class CheckedArtifacts(unittest.TestCase):
    def test_checked_in_binaries_match_real_source_receipts(self):
        # Deliberately runs against the actual repository, not a fixture. A byte
        # flipped in bin/ or a zeroed source_commit must fail normal test discovery.
        proof.verify(ROOT)

class Tampering(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / 'Cargo.toml').write_text('synthetic build inputs')
        env = {"PATH": os.defpath, "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull}
        for args in [('init', '-q'), ('add', '.'), ('-c','user.name=Fixture','-c','user.email=fixture@example.test',
                     '-c','commit.gpgsign=false','-c','core.hooksPath=/dev/null','commit','-qm','fixture')]:
            subprocess.run(['git','-C',str(self.root),*args], env=env, check=True, capture_output=True)
        commit = proof.git(self.root, 'rev-parse', 'HEAD').decode().strip()
        self.manifest = {'schema_version': 2, 'platforms': {}}
        for platform, target in proof.TARGETS.items():
            binary = self.root / 'bin' / platform / 'kp-github-mcp'
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b'synthetic-compiled-output')
            binary.chmod(0o755)
            self.manifest['platforms'][platform] = {'source_commit': commit, 'target': target,
                'binary_sha256': proof.sha(binary.read_bytes()), 'rust_version': '1.99.0',
                'source_inputs_sha256': proof.source_digest(self.root, commit)}
        self.write()

    def write(self):
        (self.root / 'bin/consumer-provenance.json').write_text(json.dumps(self.manifest))

    def test_supported_platform_bytes_are_checked(self):
        for platform in proof.TARGETS:
            binary = self.root / 'bin' / platform / 'kp-github-mcp'
            original = binary.read_bytes()
            binary.write_bytes(original[:-1] + b'X')
            with self.assertRaisesRegex(proof.ProvenanceError, 'SHA256'):
                proof.verify(self.root)
            binary.write_bytes(original)

    def test_invalid_source_target_digest_and_toolchain_are_rejected(self):
        original = copy.deepcopy(self.manifest)
        for field, value in [('source_commit', '0'*40), ('source_commit', 'f'*40),
                             ('target', 'wrong'), ('source_inputs_sha256', '0'*64), ('rust_version', 'stable')]:
            with self.subTest(field=field):
                self.manifest = copy.deepcopy(original)
                self.manifest['platforms']['linux-x86_64'][field] = value
                self.write()
                with self.assertRaises(proof.ProvenanceError): proof.verify(self.root)

    def test_rewritten_checksum_does_not_bypass_rebuild_comparison(self):
        binary = self.root / 'bin/linux-x86_64/kp-github-mcp'
        expected = binary.read_bytes()
        binary.write_bytes(b'changed-binary-and-changed-receipt')
        self.manifest['platforms']['linux-x86_64']['binary_sha256'] = proof.sha(binary.read_bytes())
        self.write()
        def compiler(root, commit, target, version, destination): destination.write_bytes(expected)
        with patch.object(proof, 'clean_build', side_effect=compiler):
            with self.assertRaisesRegex(proof.ProvenanceError, 'Clean rebuild differs'):
                proof.reproduce(self.root, 'linux-x86_64')

    def test_current_source_gate_rejects_uncommitted_input(self):
        proof.verify(self.root, require_current=True)
        (self.root / 'Cargo.toml').write_text('changed')
        with self.assertRaisesRegex(proof.ProvenanceError, 'stale'):
            proof.verify(self.root, require_current=True)

if __name__ == '__main__': unittest.main()
