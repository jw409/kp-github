# kp-github

Token-compressed GitHub MCP server for Claude Code. Wraps the GitHub API with
aggressive response compression so PR/issue/CI reads cost a fraction of the
tokens the raw API returns, and surfaces the fields agents actually act on
(draft state, branch status, full SHAs) instead of truncating them.

Binary: `kp-github-mcp`.

## Build

```sh
cargo test
cargo build --release   # -> target/release/kp-github-mcp
```

## Use

```sh
claude mcp add kp-github --transport stdio -- /path/to/kp-github-mcp
```

Select a stored account for this process with `KP_GITHUB_ACCOUNT=jw409`, or use
`GH_TOKEN`, then `GITHUB_TOKEN`, then the existing `gh` default. An event profile
can also supply the account. Selection never changes the machine-wide `gh`
default. See [account selection](docs/account-selection.md). Commit author
defaults come from `GIT_AUTHOR_*`.

## GitHub event app

The optional [app registration helper](docs/github-app.md) creates a minimal,
read-only GitHub App for PR approvals, merges, and CI events. It uses explicit
per-process GitHub credentials and keeps app keys private to the operator.
Developers can use the [cached setup helper](docs/developer-events-setup.md) to
load one shared config bundle through GCS or copy/paste. Each developer keeps
their own GitHub and Google credentials; app signing keys stay with the receiver.

The optional `github_events` tool registers, lists, removes, reads, and explicitly
acknowledges listeners for PR approvals, merges, and CI failures. Reads support a
bounded wait and preserve event identifiers and payloads. Its
[proposed receiver API](docs/listener-api.md) is tested against a local fake
service; production use still requires the receiver owner to deploy that API
and publish its config. Meshly's instance serves its own onboarded organizations;
other adopters deploy their own receiver and app.

## Relationship to kinderpowers

Extracted from [jw409/kinderpowers](https://github.com/jw409/kinderpowers),
which consumes this repo as a submodule at `mcp-servers/github` and ships a
pre-built binary at `mcp-servers/bin/kp-github-mcp`. The plugin's `plugin.json`
points at that binary, so **installing the plugin does not require this repo** —
it's a build-time dependency only.

Source changes land here; kinderpowers then bumps its submodule pointer and
rebuilds binaries via its `mcp-v*` tag workflow.
