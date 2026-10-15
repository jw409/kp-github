#!/usr/bin/env python3
"""Prepare private local HTTP credentials and an MCP client config; start nothing."""
import argparse
import os
from pathlib import Path
import re
import secrets
import sys

from github_events_setup import PrivateDirectory, SetupError


def prepare(directory, port):
    if not 1 <= port <= 65535:
        raise SetupError('Port must be between 1 and 65535.')
    with PrivateDirectory(directory) as private:
        raw = private.read('http.key')
        if raw is None:
            raw = (secrets.token_hex(32) + '\n').encode()
            # A racing creator cannot overwrite another daemon's key.
            try:
                fd = os.open('http.key', os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                             0o600, dir_fd=private.fd)
            except FileExistsError:
                raw = private.read('http.key')
            else:
                with os.fdopen(fd, 'wb') as output:
                    os.fchmod(output.fileno(), 0o600)
                    output.write(raw)
                    output.flush()
                    os.fsync(output.fileno())
                os.fsync(private.fd)
        if raw is None or not re.fullmatch(rb'[0-9a-fA-F]{64}\n?', raw):
            raise SetupError('Existing HTTP key is invalid; it was retained for explicit repair.')
        key = raw.decode().rstrip('\n')
        private.write_json('mcp.json', {'mcpServers': {'kp-github': {
            'type': 'http', 'url': f'http://127.0.0.1:{port}/mcp',
            'headers': {'Authorization': 'Bearer ' + key},
        }}})
    return directory / 'http.key', directory / 'mcp.json'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', type=Path, default=Path.home() / '.config/kp-github/shared')
    parser.add_argument('--port', type=int, default=18766)
    args = parser.parse_args(argv)
    directory = Path(os.path.abspath(args.directory.expanduser()))
    try:
        key, config = prepare(directory, args.port)
    except (OSError, SetupError) as error:
        print(f'Shared HTTP setup failed: {error}', file=sys.stderr)
        return 1
    print(f'Local key: {key}\nMCP client configuration: {config}')
    print('No process started or client configuration changed. Both files are private; do not publish them.')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
