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

Auth follows the ambient GitHub credentials; commit author defaults come from
`GIT_AUTHOR_*`.

## Relationship to kinderpowers

Extracted from [jw409/kinderpowers](https://github.com/jw409/kinderpowers),
which consumes this repo as a submodule at `mcp-servers/github` and ships a
pre-built binary at `mcp-servers/bin/kp-github-mcp`. The plugin's `plugin.json`
points at that binary, so **installing the plugin does not require this repo** —
it's a build-time dependency only.

Source changes land here; kinderpowers then bumps its submodule pointer and
rebuilds binaries via its `mcp-v*` tag workflow.
