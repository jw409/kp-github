"""Private-key preparation must preserve a running daemon's authority."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / 'scripts/setup_shared_http.py'

class SharedSetupTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve() / 'shared'

    def invoke(self, *args):
        return subprocess.run([sys.executable, str(SCRIPT), '--directory', str(self.root), *args],
                              capture_output=True, text=True, timeout=5)

    def test_private_files_and_cached_key_with_matching_client_config(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        key = (self.root / 'http.key').read_text().strip()
        self.assertEqual(len(key), 64)
        self.assertNotIn(key, result.stdout + result.stderr)
        self.assertEqual(self.root.stat().st_mode & 0o777, 0o700)
        for name in ('http.key', 'mcp.json'):
            self.assertEqual((self.root / name).stat().st_mode & 0o777, 0o600)
        timestamp = (self.root / 'http.key').stat().st_mtime_ns
        result = self.invoke('--port', '18767')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.root / 'http.key').stat().st_mtime_ns, timestamp)
        config = json.loads((self.root / 'mcp.json').read_text())['mcpServers']['kp-github']
        self.assertEqual(config['headers']['Authorization'], 'Bearer ' + key)
        self.assertEqual(config['url'], 'http://127.0.0.1:18767/mcp')

    def test_unsafe_or_invalid_existing_key_is_retained_and_rejected(self):
        self.assertEqual(self.invoke().returncode, 0)
        key = self.root / 'http.key'
        original = key.read_bytes()
        key.chmod(0o644)
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertEqual(key.read_bytes(), original)
        key.chmod(0o600)
        key.write_text('not-a-valid-key')
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertEqual(key.read_text(), 'not-a-valid-key')

    def test_symlink_output_and_invalid_port_are_rejected(self):
        self.assertNotEqual(self.invoke('--port', '0').returncode, 0)
        self.assertFalse(self.root.exists())
        other = self.root.parent / 'elsewhere'
        other.mkdir(mode=0o700)
        self.root.symlink_to(other, target_is_directory=True)
        self.assertNotEqual(self.invoke().returncode, 0)
        self.assertEqual(list(other.iterdir()), [])

if __name__ == '__main__':
    unittest.main()
