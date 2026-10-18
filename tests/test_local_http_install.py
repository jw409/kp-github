"""Stock HTTP installation must not adopt ambient identities or leak credentials."""
import argparse
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import plistlib
import socket
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1] / 'scripts'
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location('local_http_install', SCRIPTS / 'install_local_http.py')
installer = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(installer)


class LocalInstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.shared = self.root / 'space and $ dollar/shared'
        self.binary = self.root / 'binary'
        self.binary.write_text('#!/bin/sh\nexit 0\n')
        self.binary.chmod(0o700)
        self.env = patch.dict(os.environ, {'HOME': str(self.root), 'PATH': os.defpath}, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)

    def args(self, **overrides):
        values = dict(binary=self.binary, github_account='synthetic-dev', accounts_config=None,
                      events_config=None, port=18766, no_register=False)
        values.update(overrides)
        return argparse.Namespace(**values)

    def install(self, args=None):
        commands = []
        def command(argv, check=True):
            commands.append(argv)
            return subprocess.CompletedProcess(argv, 0, b'', b'')
        with patch.object(sys, 'platform', 'linux'), patch.object(installer.shutil, 'which', return_value='/fake/tool'), \
             patch.object(installer, 'run', side_effect=command), patch.object(installer, 'ready', return_value=True), \
             patch.object(installer.socket.socket, 'connect_ex', return_value=111), contextlib.redirect_stdout(io.StringIO()) as output:
            installer.install(args or self.args(), self.shared)
        return commands, output.getvalue()

    def test_http_service_registration_is_private_and_reinstall_reuses_key_and_service(self):
        commands, output = self.install()
        config = installer.load(self.shared, 'service.json')
        self.assertEqual(config['selectors'], {'KP_GITHUB_ACCOUNT': 'synthetic-dev'})
        self.assertEqual(self.shared.stat().st_mode & 0o777, 0o700)
        for name in ('http.key', 'mcp.json', 'service.json', 'claude-mcp.json', 'kp-github.service'):
            self.assertEqual((self.shared / name).stat().st_mode & 0o777, 0o600)
        secret = installer.key(self.shared)
        self.assertNotIn(secret, output + json.dumps(commands))
        self.assertIn(['systemctl', '--user', 'restart', 'kp-github.service'], commands)
        registration = next(c for c in commands if c[1:3] == ['mcp', 'add-json'])
        entry = json.loads(registration[-1])
        self.assertEqual(entry['type'], 'http')
        self.assertNotIn('command', entry)
        self.assertNotIn('headers', entry)
        headers = subprocess.run(entry['headersHelper'], shell=True, capture_output=True, text=True, timeout=5)
        self.assertEqual(headers.returncode, 0, headers.stderr)
        self.assertEqual(json.loads(headers.stdout), {'Authorization': 'Bearer ' + secret})
        timestamp = (self.shared / 'http.key').stat().st_mtime_ns
        commands, _ = self.install(self.args(github_account=None, port=None))
        self.assertIn(['systemctl', '--user', 'start', 'kp-github.service'], commands)
        self.assertNotIn(['systemctl', '--user', 'restart', 'kp-github.service'], commands)
        self.assertEqual((self.shared / 'http.key').stat().st_mtime_ns, timestamp)

    def test_ambient_tokens_cannot_choose_or_override_accounts(self):
        with patch.dict(os.environ, {'GH_TOKEN': 'do-not-copy', 'GITHUB_TOKEN': 'do-not-copy',
                                    'CLOUDSDK_AUTH_ACCESS_TOKEN': 'do-not-copy', 'KP_GITHUB_ACCOUNT': 'wrong'}):
            self.install(self.args(github_account=None, accounts_config=str(self.root / 'accounts.json')))
            config = installer.load(self.shared, 'service.json')
            environment = installer.service_environment(config)
        self.assertNotIn('KP_GITHUB_ACCOUNT', environment)
        self.assertEqual(environment['KP_GITHUB_ACCOUNTS_CONFIG'], str(self.root / 'accounts.json'))
        self.assertNotIn('do-not-copy', json.dumps(config) + json.dumps(environment))
        with self.assertRaisesRegex(installer.SetupError, 'no machine-wide default'):
            installer.selection(self.args(github_account=None), None)

    def test_occupied_port_never_changes_running_service_or_host(self):
        with patch.object(installer.shutil, 'which', return_value='/fake/tool'), \
             patch.object(installer.socket.socket, 'connect_ex', return_value=0), \
             patch.object(installer, 'run') as run:
            with self.assertRaisesRegex(installer.SetupError, 'occupied'):
                installer.install(self.args(), self.shared)
        run.assert_not_called()
        self.assertFalse((self.shared / 'service.json').exists())

    def test_failed_readiness_does_not_register_host(self):
        with patch.object(installer.shutil, 'which', return_value='/fake/tool'), \
             patch.object(installer, 'supervise'), patch.object(installer, 'ready', return_value=False), \
             patch.object(installer.time, 'monotonic', side_effect=[0, 11]), \
             patch.object(installer.socket.socket, 'connect_ex', return_value=111), \
             patch.object(installer, 'register_claude') as register:
            with self.assertRaisesRegex(installer.SetupError, 'did not become ready'):
                installer.install(self.args(), self.shared)
        register.assert_not_called()

    def test_service_launcher_drops_other_accounts_and_tokens(self):
        self.install()
        with patch.dict(os.environ, {'GH_TOKEN': 'forbidden', 'KP_GITHUB_ACCOUNT': 'wrong'}), \
             patch.object(installer.os, 'execve') as execute:
            self.assertEqual(installer.main(['serve', '--directory', str(self.shared)]), 0)
        binary, argv, env = execute.call_args.args
        self.assertEqual(argv[1:3], ['--http', '127.0.0.1:18766'])
        self.assertEqual(env['KP_GITHUB_ACCOUNT'], 'synthetic-dev')
        self.assertNotIn('GH_TOKEN', env)
        self.assertTrue(Path(binary).is_file())

    def test_registration_replaces_only_the_named_user_entry(self):
        commands = []
        def run(argv, check=True):
            commands.append(argv)
            return subprocess.CompletedProcess(argv, int(len(commands) == 1), b'',
                b'MCP server kp-github already exists in user config' if len(commands) == 1 else b'')
        with patch.object(installer, 'run', side_effect=run):
            installer.register_claude({'type': 'http', 'url': 'http://127.0.0.1:18766/mcp', 'headersHelper': '/helper'})
        self.assertEqual(commands[1], ['claude', 'mcp', 'remove', '--scope', 'user', 'kp-github'])
        self.assertEqual(commands[0], commands[2])

    def test_launchd_config_uses_argv_and_preserves_existing_unmanaged_agent(self):
        commands = []
        def run(argv, check=True):
            commands.append(argv)
            return subprocess.CompletedProcess(argv, 1 if argv[1] == 'print' else 0, b'', b'')
        with patch.object(sys, 'platform', 'darwin'), patch.object(installer, 'run', side_effect=run):
            installer.supervise(self.shared, self.shared / 'launcher.py', True)
        plist = plistlib.loads((self.shared / 'ai.kp-github.plist').read_bytes())
        self.assertEqual(plist['ProgramArguments'][-1], str(self.shared))
        self.assertTrue((self.root / 'Library/LaunchAgents/ai.kp-github.plist').is_symlink())
        self.assertIn(['launchctl', 'bootstrap', f'gui/{os.getuid()}', str(self.shared / 'ai.kp-github.plist')], commands)
        link = self.root / 'Library/LaunchAgents/ai.kp-github.plist'
        link.unlink()
        link.write_text('retain-unmanaged')
        with patch.object(sys, 'platform', 'darwin'), patch.object(installer, 'run') as run:
            with self.assertRaisesRegex(installer.SetupError, 'unmanaged'):
                installer.supervise(self.shared, self.shared / 'launcher.py', True)
        run.assert_not_called()
        self.assertEqual(link.read_text(), 'retain-unmanaged')

    def test_concurrent_installation_and_unsafe_key_fail_without_replacement(self):
        with installer.installation_lock(self.shared):
            with self.assertRaisesRegex(installer.SetupError, 'Another installation'):
                with installer.installation_lock(self.shared):
                    self.fail('concurrent installer acquired lock')
        self.install()
        key = self.shared / 'http.key'
        original = key.read_bytes()
        key.chmod(0o644)
        with self.assertRaises(installer.SetupError):
            self.install()
        self.assertEqual(key.read_bytes(), original)


if __name__ == '__main__':
    unittest.main()
