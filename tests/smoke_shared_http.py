#!/usr/bin/env python3
"""Offline process-boundary regressions for a shared local MCP server."""
import argparse
import concurrent.futures
import http.client
import json
import os
from pathlib import Path
import re
import selectors
import subprocess
import tempfile
import unittest

BINARY = None


class SharedHttpTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='kp-shared-smoke-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.token = 'a9' * 32
        self.key = self.root / 'http.key'
        self.key.write_text(self.token + '\n')
        self.key.chmod(0o600)
        self.calls = self.root / 'credential-calls'
        commands = self.root / 'commands'
        commands.mkdir()
        gh = commands / 'gh'
        gh.write_text("#!/bin/sh\n"
                      "test \"$*\" = 'auth token --hostname github.com --user synthetic-dev' || exit 27\n"
                      "printf 'stored-token\\n' >> \"$SMOKE_CALLS\"\n"
                      "printf 'synthetic-github-credential\\n'\n")
        gh.chmod(0o700)
        gcloud = commands / 'gcloud'
        gcloud.write_text("#!/bin/sh\nprintf 'unexpected-gcloud\\n' >> \"$SMOKE_CALLS\"\nexit 28\n")
        gcloud.chmod(0o700)
        self.env = {'PATH': str(commands) + os.pathsep + os.defpath,
                    'KP_GITHUB_ACCOUNT': 'synthetic-dev', 'SMOKE_CALLS': str(self.calls)}

    def start(self):
        self.process = subprocess.Popen([str(BINARY), '--http', '127.0.0.1:0',
            '--http-token-file', str(self.key)], env=self.env, stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.addCleanup(self.stop)
        with selectors.DefaultSelector() as ready:
            ready.register(self.process.stderr, selectors.EVENT_READ)
            self.assertTrue(ready.select(8), 'HTTP process never announced readiness')
            line = self.process.stderr.readline()
        match = re.search(r'http://127\.0\.0\.1:(\d+)/mcp', line)
        self.assertIsNotNone(match, 'Startup failed: ' + line)
        self.port = int(match.group(1))

    def stop(self):
        if not hasattr(self, 'process'):
            return
        if self.process.poll() is None:
            self.process.terminate()
        try:
            stdout, stderr = self.process.communicate(timeout=8)
        except subprocess.TimeoutExpired:
            self.process.kill()
            stdout, stderr = self.process.communicate()
            self.fail('Shared server did not shut down')
        self.assertNotIn(self.token, stdout + stderr)
        self.assertNotIn('synthetic-github-credential', stdout + stderr)

    def request(self, body=None, session=None, method='POST', overrides=None, duplicate_auth=False):
        headers = {'Host': f'127.0.0.1:{self.port}', 'Authorization': 'Bearer ' + self.token,
                   'Accept': 'application/json, text/event-stream', 'Content-Type': 'application/json',
                   'MCP-Protocol-Version': '2025-11-25'}
        if session:
            headers['MCP-Session-Id'] = session
        for key, value in (overrides or {}).items():
            if value is None:
                headers.pop(key, None)
            else:
                headers[key] = value
        data = json.dumps(body).encode() if body is not None else b''
        connection = http.client.HTTPConnection('127.0.0.1', self.port, timeout=8)
        try:
            connection.putrequest(method, '/mcp', skip_host=True)
            for key, value in headers.items():
                connection.putheader(key, value)
            if duplicate_auth:
                connection.putheader('Authorization', 'Bearer ' + self.token)
            connection.putheader('Content-Length', str(len(data)))
            connection.endheaders(data)
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read().decode()
        finally:
            connection.close()

    def rpc(self, method, session=None, params=None, request_id=42):
        status, headers, body = self.request({'jsonrpc': '2.0', 'id': request_id,
            'method': method, 'params': params or {}}, session=session)
        self.assertEqual(status, 200, body)
        # rmcp uses SSE frames for POST responses, including an empty priming event.
        messages = [json.loads(line[6:]) for line in body.splitlines() if line.startswith('data: {')]
        if not messages and body.startswith('{'):
            messages = [json.loads(body)]
        reply = next(message for message in messages if message.get('id') == request_id)
        return reply, headers

    def initialize(self):
        reply, headers = self.rpc('initialize', params={'protocolVersion': '2025-11-25',
            'capabilities': {}, 'clientInfo': {'name': 'offline-multi-session-test', 'version': '1'}})
        self.assertIn('result', reply)
        session = next(value for key, value in headers.items() if key.lower() == 'mcp-session-id')
        status, _, _ = self.request({'jsonrpc': '2.0', 'method': 'notifications/initialized'}, session=session)
        self.assertEqual(status, 202)
        return session

    def test_nine_sessions_share_one_process_and_one_credential_lookup(self):
        self.start()
        with concurrent.futures.ThreadPoolExecutor(max_workers=9) as pool:
            sessions = list(pool.map(lambda _: self.initialize(), range(9)))
        self.assertEqual(len(set(sessions)), 9)
        def check(item):
            index, session = item
            method = 'tools/list' if index % 2 == 0 else 'resources/templates/list'
            result, _ = self.rpc(method, session)
            self.assertNotIn('error', result)
            if index % 2 == 0:
                tools = result['result']['tools']
                self.assertEqual(len(tools), 67)
                self.assertIn('github_events', {tool['name'] for tool in tools})
            else:
                self.assertIn('resourceTemplates', result['result'])
        with concurrent.futures.ThreadPoolExecutor(max_workers=9) as pool:
            list(pool.map(check, enumerate(sessions)))
        self.assertEqual(self.calls.read_text().splitlines(), ['stored-token'])
        self.assertIsNone(self.process.poll())
        # Deleting one transport session must not close or acknowledge another.
        self.assertIn(self.request(session=sessions[0], method='DELETE')[0], (200, 202, 204))
        self.assertEqual(self.request({'jsonrpc': '2.0', 'id': 42, 'method': 'tools/list'}, session=sessions[0])[0], 404)
        check((0, sessions[1]))
        self.assertEqual(self.calls.read_text().splitlines(), ['stored-token'])

    def test_http_auth_host_and_origin_are_enforced(self):
        self.start()
        for overrides, expected in [({'Authorization': None}, 401),
                                    ({'Authorization': 'Bearer wrong'}, 401),
                                    ({'Host': 'external.example'}, 403),
                                    ({'Origin': 'https://external.example'}, 403),
                                    ({'Origin': 'null'}, 403)]:
            with self.subTest(overrides=overrides):
                self.assertEqual(self.request(overrides=overrides)[0], expected)
        self.assertEqual(self.request(duplicate_auth=True)[0], 401)
        self.initialize()
        self.assertEqual(self.calls.read_text().splitlines(), ['stored-token'])

    def test_key_permissions_and_symlinks_fail_before_auth_lookup(self):
        for mode in (0o644, 0o660):
            self.key.chmod(mode)
            result = subprocess.run([str(BINARY), '--http', '127.0.0.1:0',
                '--http-token-file', str(self.key)], env=self.env, capture_output=True, timeout=8)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.calls.exists())
        self.key.chmod(0o600)
        link = self.root / 'link.key'
        link.symlink_to(self.key)
        result = subprocess.run([str(BINARY), '--http', '127.0.0.1:0',
            '--http-token-file', str(link)], env=self.env, capture_output=True, timeout=8)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.calls.exists())

    def test_non_loopback_and_missing_key_fail_before_auth_lookup(self):
        for args in (['--http', '0.0.0.0:18766', '--http-token-file', str(self.key)],
                     ['--http', '127.0.0.1:18766'], ['--http-token-file', str(self.key)],
                     ['--http', 'localhost:18766', '--http-token-file', str(self.key)]):
            result = subprocess.run([str(BINARY), *args], env=self.env, capture_output=True, timeout=8)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(self.calls.exists())

    def test_second_daemon_cannot_take_over_the_endpoint(self):
        self.start()
        session = self.initialize()
        second = subprocess.run([str(BINARY), '--http', f'127.0.0.1:{self.port}',
            '--http-token-file', str(self.key)], env=self.env, capture_output=True, text=True, timeout=8)
        self.assertNotEqual(second.returncode, 0)
        self.assertIn('Cannot bind shared HTTP listener', second.stderr)
        self.assertNotIn(self.token, second.stderr)
        reply, _ = self.rpc('tools/list', session)
        self.assertEqual(len(reply['result']['tools']), 67)
        self.assertIsNone(self.process.poll())

    def test_closed_sessions_and_restart_require_fresh_initialization(self):
        self.start()
        session = self.initialize()
        self.stop()
        self.start()
        self.assertEqual(self.request({'jsonrpc': '2.0', 'id': 42, 'method': 'tools/list'}, session=session)[0], 404)
        self.assertNotEqual(self.initialize(), session)
        self.assertEqual(self.calls.read_text().splitlines(), ['stored-token', 'stored-token'])


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', required=True, type=Path)
    args, rest = parser.parse_known_args()
    BINARY = args.binary.resolve()
    unittest.main(argv=[__file__, *rest])
