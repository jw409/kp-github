#!/usr/bin/env python3
"""Install one supervised local HTTP server, using explicitly selected accounts."""
import argparse
from contextlib import contextmanager
import fcntl
import hashlib
import http.client
import json
import os
from pathlib import Path
import plistlib
import re
import shlex
import shutil
import socket
import stat
import subprocess
import sys
import time

from github_events_setup import PrivateDirectory, SetupError
from setup_shared_http import prepare

SERVICE = 'kp-github.service'
LABEL = 'ai.kp-github'
SELECTORS = ('KP_GITHUB_ACCOUNT', 'KP_GITHUB_ACCOUNTS_CONFIG', 'KP_GITHUB_EVENTS_CONFIG')


def run(command, check=True):
    result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, timeout=20)
    if check and result.returncode:
        # Commands/config can be reported; credential-bearing subprocess output cannot.
        raise SetupError(f'{Path(command[0]).name} {command[1]} failed (exit {result.returncode}).')
    return result


def put(directory, name, data, mode=0o600):
    """Atomic replacement, never truncate an executable a running process has open."""
    with PrivateDirectory(directory) as private:
        private.check_target(name)
        temporary = '.install-' + os.urandom(12).hex()
        try:
            fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                         mode, dir_fd=private.fd)
            with os.fdopen(fd, 'wb') as output:
                os.fchmod(output.fileno(), mode)
                output.write(data)
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, name, src_dir_fd=private.fd, dst_dir_fd=private.fd)
            os.fsync(private.fd)
        finally:
            try:
                os.unlink(temporary, dir_fd=private.fd)
            except FileNotFoundError:
                pass


@contextmanager
def installation_lock(directory):
    with PrivateDirectory(directory) as private:
        private.check_target('install.lock')
        fd = os.open('install.lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600, dir_fd=private.fd)
        try:
            info = os.fstat(fd)
            if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600:
                raise SetupError('The installation lock must be private and owned by this user.')
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise SetupError('Another installation is running; retry after it completes.') from None
            yield
        finally:
            os.close(fd)


def load(directory, name):
    with PrivateDirectory(directory) as private:
        raw = private.read(name)
    return json.loads(raw) if raw is not None else None


def key(directory):
    with PrivateDirectory(directory) as private:
        raw = private.read('http.key')
    if raw is None or not re.fullmatch(rb'[0-9a-fA-F]{64}\n?', raw):
        raise SetupError('Missing or invalid local key; run the installer first.')
    return raw.decode().strip()


def absolute(value):
    result = str(Path(value).expanduser().absolute())
    if any(ord(c) < 32 for c in result):
        raise SetupError('Paths cannot contain control characters.')
    return result


def selection(args, previous):
    values = {name: value for name, value in zip(SELECTORS,
              (args.github_account, args.accounts_config, args.events_config)) if value}
    if not values and previous:
        return previous['selectors']
    if not values.get('KP_GITHUB_ACCOUNT') and not values.get('KP_GITHUB_ACCOUNTS_CONFIG'):
        raise SetupError('Select --github-account LOGIN or --accounts-config FILE; no machine-wide default is inferred.')
    login = values.get('KP_GITHUB_ACCOUNT')
    if login and not re.fullmatch(r'[A-Za-z0-9](?:[A-Za-z0-9-]{0,37}[A-Za-z0-9])?', login):
        raise SetupError('Invalid GitHub account name.')
    for name in SELECTORS[1:]:
        if name in values:
            values[name] = absolute(values[name])
    if 'KP_GITHUB_ACCOUNTS_CONFIG' in values and 'KP_GITHUB_EVENTS_CONFIG' in values:
        raise SetupError('Use events_config inside each accounts registry entry for multiple accounts.')
    return values


def service_environment(config):
    # User service managers can inherit unrelated tokens/defaults. Never adopt those.
    environment = {name: os.environ[name] for name in
                   ('USER', 'LOGNAME', 'LANG', 'LC_ALL', 'DBUS_SESSION_BUS_ADDRESS', 'XDG_RUNTIME_DIR')
                   if name in os.environ}
    environment.update(config['credential_locations'])
    environment.update(config['selectors'])
    environment.update(HOME=str(Path.home()), PATH=config['path'])
    return environment


def ready(directory, port):
    """Authenticate a disposable MCP session; never probe GitHub or gcloud."""
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=1)
    headers = {'Authorization': 'Bearer ' + key(directory), 'Content-Type': 'application/json',
               'Accept': 'application/json, text/event-stream', 'MCP-Protocol-Version': '2025-11-25'}
    try:
        body = {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
            'protocolVersion': '2025-11-25', 'capabilities': {},
            'clientInfo': {'name': 'kp-github-installer', 'version': '1'}}}
        connection.request('POST', '/mcp', json.dumps(body), headers)
        response = connection.getresponse()
        session = response.getheader('Mcp-Session-Id')
        status = response.status
        raw = response.read(2 * 1024 * 1024).decode()
        if status != 200 or not session:
            return False
        messages = [json.loads(line[6:]) for line in raw.splitlines() if line.startswith('data: {')]
        if raw.startswith('{'):
            messages = [json.loads(raw)]
        valid = any(m.get('id') == 1 and 'serverInfo' in m.get('result', {}) for m in messages)
        headers['Mcp-Session-Id'] = session
        connection.request('DELETE', '/mcp', headers=headers)
        connection.getresponse().read()
        return valid
    except (OSError, ValueError, http.client.HTTPException):
        return False
    finally:
        connection.close()


def unit_quote(value):
    # systemd ExecStart is not a shell; quote both its expansion syntaxes.
    return '"' + str(value).replace('\\', '\\\\').replace('"', '\\"').replace('%', '%%').replace('$', '$$') + '"'


def supervise(directory, launcher, changed):
    command = [sys.executable, str(launcher), 'serve', '--directory', str(directory)]
    if sys.platform == 'linux':
        unit = ('[Unit]\nDescription=Shared local GitHub MCP server\n\n[Service]\n'
                + 'ExecStart=' + ' '.join(map(unit_quote, command)) + '\n'
                + 'Restart=on-failure\nRestartSec=2\nUMask=0077\n\n[Install]\nWantedBy=default.target\n')
        put(directory, SERVICE, unit.encode())
        run(['systemctl', '--user', 'link', str(directory / SERVICE)])
        run(['systemctl', '--user', 'daemon-reload'])
        run(['systemctl', '--user', 'enable', SERVICE])
        run(['systemctl', '--user', 'restart' if changed else 'start', SERVICE])
    elif sys.platform == 'darwin':
        plist = directory / (LABEL + '.plist')
        put(directory, plist.name, plistlib.dumps({'Label': LABEL, 'ProgramArguments': command,
            'RunAtLoad': True, 'KeepAlive': {'SuccessfulExit': False}, 'ThrottleInterval': 2,
            'Umask': 0o077}))
        agents = Path.home() / 'Library/LaunchAgents'
        agents.mkdir(parents=True, exist_ok=True)
        link = agents / plist.name
        if link.is_symlink():
            if os.readlink(link) != str(plist):
                raise SetupError('Another kp-github launch agent exists; retain it and reconcile its location.')
        elif link.exists():
            raise SetupError('An unmanaged kp-github launch agent exists; it was retained.')
        else:
            link.symlink_to(plist)
        domain = f'gui/{os.getuid()}'
        loaded = run(['launchctl', 'print', domain + '/' + LABEL], check=False).returncode == 0
        if loaded and changed:
            run(['launchctl', 'bootout', domain + '/' + LABEL])
            loaded = False
        if not loaded:
            run(['launchctl', 'bootstrap', domain, str(plist)])
    else:
        raise SetupError('Stock installation supports Linux systemd user services and macOS launchd.')


def register_claude(entry):
    # headersHelper contains paths only; the bearer is never a command-line argument.
    command = ['claude', 'mcp', 'add-json', '--scope', 'user', 'kp-github', json.dumps(entry)]
    result = run(command, check=False)
    if result.returncode:
        message = (result.stdout + result.stderr).decode(errors='replace').lower()
        if 'already exists' not in message:
            raise SetupError('Claude HTTP registration failed; service and private mcp.json are ready for manual configuration.')
        run(['claude', 'mcp', 'remove', '--scope', 'user', 'kp-github'])
        run(command)


def install(args, directory):
    supervisor = 'systemctl' if sys.platform == 'linux' else 'launchctl' if sys.platform == 'darwin' else None
    if supervisor is None or not shutil.which(supervisor):
        raise SetupError('A systemd user manager (Linux) or launchd (macOS) is required.')
    if not args.no_register and not shutil.which('claude'):
        raise SetupError('Claude CLI is missing; use --no-register for another MCP host.')
    previous = load(directory, 'service.json')
    if args.port is None:
        args.port = previous['port'] if previous else 18766
    if not 1 <= args.port <= 65535:
        raise SetupError('Port must be between 1 and 65535.')
    selectors = selection(args, previous)
    if args.binary is None:
        raise SetupError('--binary must name a built executable.')
    binary = Path(absolute(args.binary))
    if binary.is_symlink() or not binary.is_file() or not os.access(binary, os.X_OK):
        raise SetupError('--binary must be a regular executable file.')
    content = binary.read_bytes()
    installed = directory / ('binary-' + hashlib.sha256(content).hexdigest())
    sources = {name: Path(__file__).with_name(name).read_bytes() for name in
               ('install_local_http.py', 'setup_shared_http.py', 'github_events_setup.py')}
    digest = hashlib.sha256(b''.join(sources.values())).hexdigest()
    runtime = directory / ('runtime-' + digest)
    config = {'version': 1, 'binary': str(installed), 'port': args.port,
              'selectors': selectors, 'path': os.environ.get('PATH', os.defpath),
              'credential_locations': {name: absolute(os.environ[name]) for name in
                    ('GH_CONFIG_DIR', 'CLOUDSDK_CONFIG', 'XDG_CONFIG_HOME') if os.environ.get(name)},
              'runtime': str(runtime), 'python': sys.executable}
    # Refuse an occupied port on first installation instead of adopting another daemon.
    if previous is None or previous['port'] != args.port:
        with socket.socket() as probe:
            if probe.connect_ex(('127.0.0.1', args.port)) == 0:
                raise SetupError('The requested local port is occupied; no existing process was changed.')
    put(directory, installed.name, content, 0o700)
    for name, data in sources.items():
        put(runtime, name, data)
    prepare(directory, args.port)
    with PrivateDirectory(directory) as private:
        private.write_json('service.json', config)
    supervise(directory, runtime / 'install_local_http.py', previous != config)
    deadline = time.monotonic() + 10
    while not ready(directory, args.port):
        if time.monotonic() >= deadline:
            raise SetupError('HTTP service did not become ready; inspect the user service log. MCP host settings were not changed.')
        time.sleep(0.1)
    helper = shlex.join([sys.executable, str(runtime / 'install_local_http.py'), 'headers', '--directory', str(directory)])
    entry = {'type': 'http', 'url': f'http://127.0.0.1:{args.port}/mcp', 'headersHelper': helper}
    with PrivateDirectory(directory) as private:
        private.write_json('claude-mcp.json', {'mcpServers': {'kp-github': entry}})
    if not args.no_register:
        register_claude(entry)
    print(f'Shared HTTP server ready: {entry["url"]}\nPrivate client config: {directory / "mcp.json"}')
    print('Claude user HTTP entry registered.' if not args.no_register else 'MCP host registration skipped.')
    print('Existing stdio sessions remain running. Remove duplicate plugin/project stdio entries before reconnecting.')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('operation', nargs='?', choices=['install', 'serve', 'headers'], default='install')
    parser.add_argument('--directory', type=Path, default=Path.home() / '.config/kp-github/shared')
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--port', type=int, help='Default: saved port, otherwise 18766')
    parser.add_argument('--github-account', default=os.environ.get('KP_GITHUB_ACCOUNT'))
    parser.add_argument('--accounts-config', default=os.environ.get('KP_GITHUB_ACCOUNTS_CONFIG'))
    parser.add_argument('--events-config', default=os.environ.get('KP_GITHUB_EVENTS_CONFIG'))
    parser.add_argument('--no-register', action='store_true', help='Start the service; leave MCP host registration to you.')
    args = parser.parse_args(argv)
    try:
        directory = Path(absolute(args.directory))
        if args.operation == 'headers':
            print(json.dumps({'Authorization': 'Bearer ' + key(directory)}))
        elif args.operation == 'serve':
            config = load(directory, 'service.json')
            if config is None or config.get('version') != 1:
                raise SetupError('Missing or unsupported service configuration.')
            os.execve(config['binary'], [config['binary'], '--http', f'127.0.0.1:{config["port"]}',
                      '--http-token-file', str(directory / 'http.key')], service_environment(config))
        else:
            with installation_lock(directory):
                install(args, directory)
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired, SetupError):
        # Configuration/command errors must never echo arbitrary credential-bearing output.
        error = sys.exc_info()[1]
        message = str(error) if isinstance(error, SetupError) else 'Could not read configuration or run the local service manager.'
        print(f'Shared HTTP installation failed: {message}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
