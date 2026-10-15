#!/usr/bin/env python3
"""Exercise the real packaged installer, binary and Claude CLI in an isolated home.

Only the service manager and gh are simulated. No real credentials are read and
no installed services or MCP host entries are changed.
"""
import argparse
import concurrent.futures
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--installer', type=Path, default=Path(__file__).resolve().parents[1] / 'scripts/install_local_http.py')
    parser.add_argument('--claude', type=Path, required=True)
    args = parser.parse_args()
    script = args.installer.resolve()
    sys.path.insert(0, str(script.parent))
    spec = importlib.util.spec_from_file_location('installer_smoke', script)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    with tempfile.TemporaryDirectory(prefix='kp-install-smoke-') as temporary:
        root = Path(temporary).resolve()
        commands = root / 'commands'
        commands.mkdir()
        (commands / 'claude').symlink_to(args.claude.resolve())
        manager = commands / 'systemctl'
        manager.write_text('''#!/usr/bin/env python3
import json, os, signal, subprocess, sys
from pathlib import Path
root = Path.home()
state = root / 'unit-path'
arguments = sys.argv[1:]
operation = arguments[1]
with (root / 'manager-calls').open('a') as log:
    log.write(json.dumps(arguments) + '\\n')
if operation == 'link':
    state.write_text(arguments[2])
elif operation in ('start', 'restart'):
    pidfile = root / 'service-pid'
    if pidfile.exists():
        if operation == 'start':
            raise SystemExit(0)
        os.kill(int(pidfile.read_text()), signal.SIGTERM)
    directory = Path(state.read_text()).parent
    config = json.loads((directory / 'service.json').read_text())
    with (root / 'service.log').open('ab') as log:
        process = subprocess.Popen([sys.executable, config['runtime'] + '/install_local_http.py',
            'serve', '--directory', str(directory)], stdin=subprocess.DEVNULL,
            stdout=log, stderr=log, start_new_session=True)
    pidfile.write_text(str(process.pid))
''')
        manager.chmod(0o700)
        gh = commands / 'gh'
        gh.write_text('''#!/usr/bin/env python3
import sys
from pathlib import Path
assert sys.argv[1:] == ['auth', 'token', '--hostname', 'github.com', '--user', 'synthetic-dev']
with (Path.home() / 'gh-calls').open('a') as output:
    output.write('selected-synthetic-account\\n')
print('synthetic-local-github-token')
''')
        gh.chmod(0o700)
        env = {'HOME': str(root), 'PATH': str(commands) + os.pathsep + os.environ['PATH'],
               'CLAUDE_CONFIG_DIR': str(root / 'claude'), 'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1'}
        def run(argv):
            result = subprocess.run(argv, env=env, cwd=root, capture_output=True, text=True, timeout=40)
            if result.returncode:
                raise AssertionError(f'Command failed: {argv[0]} (exit {result.returncode})\n{result.stderr}')
            return result
        with socket.socket() as port_socket:
            port_socket.bind(('127.0.0.1', 0))
            port = port_socket.getsockname()[1]
        shared = root / 'private config/shared'
        try:
            for name in ('unrelated-test-server', 'kp-github'):
                run([str(args.claude), 'mcp', 'add-json', '--scope', 'user', name,
                     json.dumps({'type': 'stdio', 'command': '/bin/echo', 'args': ['fixture']})])
            call = [sys.executable, str(script), '--binary', str(args.binary.resolve()),
                    '--directory', str(shared), '--github-account', 'synthetic-dev', '--port', str(port)]
            first = run(call)
            pid = int((root / 'service-pid').read_text())
            config = json.loads((shared / 'service.json').read_text())
            assert config['selectors'] == {'KP_GITHUB_ACCOUNT': 'synthetic-dev'}
            with concurrent.futures.ThreadPoolExecutor(max_workers=9) as pool:
                assert all(pool.map(lambda _: module.ready(shared, port), range(9)))
            second = run(call)
            assert int((root / 'service-pid').read_text()) == pid, 'Repeat install restarted the daemon'
            assert (root / 'gh-calls').read_text().splitlines() == ['selected-synthetic-account']
            key = module.key(shared)
            assert key not in first.stdout + first.stderr + second.stdout + second.stderr
            host_configs = list((root / 'claude').glob('*.json')) + list(root.glob('.claude.json'))
            host = next(json.loads(p.read_text()) for p in host_configs
                        if 'unrelated-test-server' in p.read_text())
            assert host['mcpServers']['unrelated-test-server']['type'] == 'stdio'
            assert host['mcpServers']['kp-github']['type'] == 'http'
            assert key not in json.dumps(host), 'Bearer persisted into Claude host config'
            print(json.dumps({'real_binary_http': True, 'real_claude_registration': True,
                'independent_mcp_sessions': 9, 'reinstall_same_pid': True,
                'credential_lookups': 1, 'other_host_entry_preserved': True,
                'host_and_command_output_contain_no_bearer': True,
                'service_manager': 'isolated_fixture', 'native_systemd_launchd_execution': False}))
        finally:
            pidfile = root / 'service-pid'
            if pidfile.exists():
                try:
                    os.kill(int(pidfile.read_text()), signal.SIGTERM)
                except ProcessLookupError:
                    pass


if __name__ == '__main__':
    main()
