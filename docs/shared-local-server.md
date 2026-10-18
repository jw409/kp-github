# Sharing one local server across sessions

The stock installer starts one supervised Streamable HTTP server per user.
The raw executable still supports stdio with no arguments for existing clients.
`--http` runs one independent process with a Streamable HTTP endpoint. Clients
connect directly to it without starting a per-session proxy. Claude Code supports
this transport: https://code.claude.com/docs/en/mcp#option-1-add-a-remote-http-server .

Each connection initializes its own MCP session. Request IDs, cancellation and
session deletion stay separate. The GitHub HTTP client/connection pool, per-account
credentials, optional event clients/token caches and usage-log handle are shared.
No GitHub response cache is added. Token expiry/401 refresh remains unchanged.
A missing transport session returns 404 so it does not invalidate valid credentials.

## Account and configuration boundary

A daemon can serve multiple explicitly enabled GitHub accounts and organizations.
Use the registry and per-request `github_account` selector in
[account selection](account-selection.md). Clients, credentials and event token
caches are separate per account; each request gets an immutable account scope.
There is no shared mutable active-account selector. A missing credential or bad
event profile affects only the selected account's request/feature.

The daemon's environment supplies configuration, commit-author defaults and the
usage sink. Tool-level commit-author arguments still override those defaults.
Listener cursors and acknowledgement tokens remain explicit tool arguments;
sharing transport does not acknowledge events.

A local bearer key grants all accounts explicitly enabled on that daemon,
including their write tools. It is distinct from GitHub, Google and App
credentials. Only sessions intended to share that authority should share a key.

## Stock installation

From a source checkout or an extracted consumer archive:

```sh
scripts/install.sh --github-account YOUR_GITHUB_LOGIN
# For multiple accounts instead:
scripts/install.sh --accounts-config /absolute/path/accounts.json
```

An extracted archive uses its included executable. A checkout builds from source
with Cargo. Python 3, `gh`, and a running systemd user manager (Linux) or a GUI
launchd session (macOS) are required; event tools additionally use `gcloud`.
GitHub/Google logins already stored by those tools are reused. Setup never opens
a browser or changes either CLI's active account.

The installer copies the binary and its helper scripts to private, content-named
paths in `~/.config/kp-github/shared`, creates or reuses `http.key`, and installs
`kp-github.service` (Linux) or `ai.kp-github` (macOS). The service starts now and at
user login. Existing binaries are replaced atomically, never truncated in place.
All service configuration/key files are mode 0600 in a mode-0700 directory.

After an authenticated local MCP readiness check, installation registers a
user-scoped `kp-github` HTTP entry with Claude Code. It replaces only that named
user entry; other entries remain intact. Claude's
[headersHelper](https://code.claude.com/docs/en/mcp#use-dynamic-headers-for-custom-authentication)
reads the private key locally, so the key does not enter command arguments.
If host registration fails, the running service and generated private client
configuration remain available; repair registration before starting new sessions.
Use `--no-register` to install the service for another MCP host.

Account selections come from explicit `--github-account`, `--accounts-config`,
`--events-config` flags or their `KP_GITHUB_*` environment equivalents. On a repeat
installation without selectors, the saved selection is retained. No stored `gh`
default or ambient GitHub token is inferred. Use `--events-config PATH` for a
single account, or per-account profile paths in the registry. Each event profile
can independently select an existing Google login and multiple organizations;
there is no remote account-pair registration.

The installer saves the selected configuration paths and PATH, never tokens. The
service reads stored credentials when needed. A repeat install with identical
configuration reuses the daemon and its key; a changed executable/configuration
restarts that service. Changes inside a referenced account registry or event
profile require an explicit service restart; the installer does not poll files.
Concurrent installers are rejected. An unrelated occupied port is not adopted
or terminated. Choose `--port PORT` to override 18766; later installs retain it.

Linux status/restart: `systemctl --user status kp-github.service` /
`systemctl --user restart kp-github.service`. macOS status/restart:
`launchctl print gui/$(id -u)/ai.kp-github` /
`launchctl kickstart -k gui/$(id -u)/ai.kp-github`.
Published packages support Linux x86_64 only. The retained launchd helper is
covered by configuration tests; native macOS service execution is unverified.

For manual supervision, the preparation-only helper remains available:

```sh
python3 scripts/setup_shared_http.py
KP_GITHUB_ACCOUNT=YOUR_GITHUB_LOGIN \
  /absolute/path/kp-github-mcp --http 127.0.0.1:18766 \
  --http-token-file "$HOME/.config/kp-github/shared/http.key"
```

That helper only writes `http.key` and `mcp.json`; it starts nothing.

## Connect clients and migrate

The generated `mcp.json` contains the HTTP URL and local Authorization header.
It can be supplied as a Claude Code MCP configuration. To test it in an isolated
new session, use `claude --strict-mcp-config --mcp-config PATH_TO_GENERATED_MCP_JSON`.
For normal use, replace the existing kp-github stdio entry with its generated
HTTP entry and remove/disable the duplicate plugin-provided kp-github entry;
otherwise the plugin still starts its old stdio child. Preserve other MCP entries.

Reconnect each session when convenient. Existing stdio pipes cannot be converted
into HTTP connections in place, so killing the nine old children first would
interrupt their clients. The installer does not edit installed plugin caches or project-scoped entries,
kill stdio clients, or alter machine-wide gh/gcloud defaults. A combined consumer
plugin must switch its own kp-github entry to HTTP as part of its integration;
installing this module cannot rewrite a separately owned plugin.

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
