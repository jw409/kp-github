# kp-github

Token-compressed GitHub MCP server for Claude Code. Wraps the GitHub API with
aggressive response compression so PR/issue/CI reads cost a fraction of the
tokens the raw API returns, and surfaces the fields agents actually act on
(draft state, branch status, full SHAs) instead of truncating them.

Binary: `kp-github-mcp`.

Several sessions on one machine can use the optional [shared local HTTP server](docs/shared-local-server.md), with separate MCP sessions and one pinned account/profile. The default transport remains stdio.

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

The [kinderpowers marketplace](https://github.com/jw409/kinderpowers) offers this
repository directly as the `kp-github` plugin. Its `bin/kp-github-mcp` wrapper
selects the checked-in Linux x86_64 or macOS arm64 binary.

The combined `kinderpowers` plugin mounts this repository at
`mcp-servers/github`; its wrapper runs the binaries inside that pinned submodule.
The submodule is required at runtime. Updating the consumer therefore means
publishing matching binaries here and advancing its submodule pin.

For a standalone archive with the setup helper, documentation, sample config,
and provenance, see [consumer packages](docs/consumer-package.md). The
[GCS configuration contract](docs/gcs-config-contract.md) supplies samples and
inputs for the receiver owner's Terraform implementation.
