#!/usr/bin/env python3
"""Offline process-boundary smoke: real setup helper and real MCP binary."""
import argparse
import json
import sys
import os
from pathlib import Path
import selectors
import subprocess
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, required=True)
parser.add_argument('--source-root', type=Path, default=Path(__file__).resolve().parents[1])
args = parser.parse_args()
ROOT = args.source_root.resolve()
BINARY = args.binary.resolve()


def check_mcp(env, expected_error):
    process = subprocess.Popen([str(BINARY)], env=env, stdin=subprocess.PIPE,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               text=True, bufsize=1)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    try:
        def send(value, response=True):
            process.stdin.write(json.dumps(value) + '\n')
            process.stdin.flush()
            if not response:
                return
            assert selector.select(10), 'MCP response timed out'
            line = process.stdout.readline()
            assert line, 'MCP exited unexpectedly'
            return json.loads(line)

        reply = send({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
            'protocolVersion': '2024-11-05', 'capabilities': {},
            'clientInfo': {'name': 'offline-smoke', 'version': '1'}}})
        assert 'result' in reply, 'Initialize failed'
        send({'jsonrpc': '2.0', 'method': 'notifications/initialized'}, False)
        listed = send({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list', 'params': {}})
        tools = listed['result']['tools']
        names = {tool['name'] for tool in tools}
        assert len(names) == 67
        assert {'github_events', 'github_prs_get', 'github_files_push'} <= names
        events_schema = next(tool['inputSchema'] for tool in tools if tool['name'] == 'github_events')
        assert events_schema.get('type') == 'object', 'MCP input schema must explicitly declare object type'
        arguments = ({'action': 'list'} if expected_error == 'not configured' else {
            'action': 'register', 'owner': 'OutsideOrg', 'events': ['approved'],
            'idempotency_key': 'smoke-key'})
        reply = send({'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call',
                      'params': {'name': 'github_events', 'arguments': arguments}})
        assert expected_error in reply['error']['message']
        return len(tools)
    finally:
        selector.close()
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


with tempfile.TemporaryDirectory(prefix='kp-events-smoke-') as temporary:
    directory = Path(temporary)
    bin_dir = directory / 'bin'
    bin_dir.mkdir()
    log = directory / 'calls'
    gh = bin_dir / 'gh'
    gh.write_text('''#!/bin/sh
test "$1 $2 $3 $4 $5" = 'auth token --hostname github.com --user' || exit 21
test -z "${GH_TOKEN+x}${GITHUB_TOKEN+x}${GH_HOST+x}" || exit 22
printf 'gh:%s\n' "$6" >> "$SMOKE_CALLS"
printf 'synthetic-selected-token\n'
''')
    gh.chmod(0o700)
    gcloud = bin_dir / 'gcloud'
    gcloud.write_text('''#!/bin/sh
printf 'unexpected-gcloud-call\n' >> "$SMOKE_CALLS"
exit 23
''')
    gcloud.chmod(0o700)
    shared = directory / 'shared.json'
    shared.write_text(json.dumps({
        'version': 1, 'endpoint': 'https://events.invalid',
        'audience': 'offline-test-audience', 'allowed_orgs': ['Meshlyai'],
        'app_slug': 'example-events'}))
    profile = directory / 'profile' / 'events.json'
    setup = subprocess.run([sys.executable, str(ROOT / 'scripts/github_events_setup.py'),
        'setup', '--config-file', str(shared), '--github-account', 'profile-dev',
        '--google-account', 'developer@example.com', '--output', str(profile)],
        capture_output=True, text=True, timeout=10)
    assert setup.returncode == 0, 'Setup failed'
    base = {'PATH': str(bin_dir) + os.pathsep + os.environ['PATH'],
            'GH_TOKEN': 'synthetic-ambient-gh', 'GITHUB_TOKEN': 'synthetic-ambient-github',
            'SMOKE_CALLS': str(log)}
    count = check_mcp(base, 'not configured')
    assert not log.exists(), 'Disabled events must not invoke account or gcloud commands'
    configured = dict(base, KP_GITHUB_EVENTS_CONFIG=str(profile))
    check_mcp(configured, 'outside this profile')
    assert log.read_text().splitlines() == ['gh:profile-dev']
    configured['KP_GITHUB_ACCOUNT'] = 'override-dev'
    check_mcp(configured, 'outside this profile')
    assert log.read_text().splitlines() == ['gh:profile-dev', 'gh:override-dev']
    print(json.dumps({'result': 'passed', 'tools': count,
        'cases': ['missing profile', 'setup profile selects stored account',
                  'explicit account overrides profile'],
        'gcloud_calls': 0, 'network_calls': 0}))
