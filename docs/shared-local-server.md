# Sharing one local server across sessions

The default stdio transport starts a separate server for every MCP client.
`--http` runs one independent process with a Streamable HTTP endpoint. Clients
connect directly to it without starting a per-session proxy. Claude Code supports
this transport: https://code.claude.com/docs/en/mcp#option-1-add-a-remote-http-server .

Each connection initializes its own MCP session. Request IDs, cancellation and
session deletion stay separate. The GitHub HTTP client/connection pool, selected
credential, optional event client/token cache and usage-log handle are shared.
No GitHub response cache is added. Token expiry/401 refresh remains unchanged.
A missing transport session returns 404 so it does not invalidate valid credentials.

## Account and configuration boundary

One daemon has one selected GitHub account, events profile, Google identity,
commit-author defaults and usage sink. These come from the daemon's environment,
not each connecting Claude process. Explicit tool-level commit-author arguments
still override the daemon's defaults. Listener cursors and acknowledgement tokens
remain explicit tool arguments; sharing transport does not acknowledge events.

Use the same daemon only for sessions intentionally sharing that configuration.
Different account/profile combinations need separate endpoints/processes in this
implementation. A local bearer key grants the daemon's GitHub authority, including
its write tools; it is distinct from GitHub, Google and App credentials. Possession
of that key is the local access boundary, not an asserted username header.

## Prepare and run

Build from this checkout; checked-in platform binaries may predate this mode:

```sh
cargo build --locked --release
python3 scripts/setup_shared_http.py
```

Setup writes `~/.config/kp-github/shared/http.key` and `mcp.json` with mode 0600 in
a mode-0700 directory. It reuses a valid existing key, and neither starts a service
nor edits any MCP host configuration. Do not commit either generated file.

Start the server once, with your stored GitHub login selected explicitly:

```sh
KP_GITHUB_ACCOUNT=YOUR_GITHUB_LOGIN \
  target/release/kp-github-mcp \
  --http 127.0.0.1:18766 \
  --http-token-file "$HOME/.config/kp-github/shared/http.key"
```

For events, set `KP_GITHUB_EVENTS_CONFIG` on this daemon as well. Keep the daemon
running with your usual user service supervisor. Example systemd user unit, after
installing the freshly built executable at the shown path:

```ini
[Unit]
Description=Shared local GitHub MCP server

[Service]
ExecStart=%h/.local/bin/kp-github-mcp --http 127.0.0.1:18766 --http-token-file %h/.config/kp-github/shared/http.key
Environment=KP_GITHUB_ACCOUNT=YOUR_GITHUB_LOGIN
Restart=on-failure
RestartSec=2
UMask=0077

[Install]
WantedBy=default.target
```

The unit is an example, not an installed service. If `gh`/`gcloud` are outside the
user manager's PATH, set PATH in the unit. Use a stable installed binary path;
rebuilds should not replace a running executable in place.

## Connect clients and migrate

The generated `mcp.json` contains the HTTP URL and local Authorization header.
It can be supplied as a Claude Code MCP configuration. To test it in an isolated
new session, use `claude --strict-mcp-config --mcp-config PATH_TO_GENERATED_MCP_JSON`.
For normal use, replace the existing kp-github stdio entry with its generated
HTTP entry and remove/disable the duplicate plugin-provided kp-github entry;
otherwise the plugin still starts its old stdio child. Preserve other MCP entries.

Reconnect each session when convenient. Existing stdio pipes cannot be converted
into HTTP connections in place, so killing the nine old children first would
interrupt their clients. This implementation does not edit installed plugin
caches, kill clients, or alter machine-wide gh/gcloud defaults.

The daemon binds only a literal loopback IP. Every request checks the bearer key,
Host and any Origin header. Requests are limited to 8 MiB and 256 active sessions.
Sessions idle for 24 hours expire; daemon restart loses transport sessions too.
Clients reinitialize after a 404. Mutating tool calls are never replayed by this
server. A daemon failure affects its connected clients, so supervision/reconnect
is the replacement for per-session process lifetime.

Offline verification:

```sh
python3 tests/smoke_shared_http.py --binary target/release/kp-github-mcp -v
python3 tests/smoke_consumer.py --binary target/release/kp-github-mcp
```

The first suite launches an ephemeral loopback server with synthetic credentials,
connects nine independent sessions concurrently using overlapping request IDs,
checks one credential lookup, session deletion/restart, and HTTP access guards.
It never calls GitHub or gcloud or changes installed client settings.
