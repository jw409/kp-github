# Selecting a GitHub account

Run each MCP server with the account it should use:

```sh
KP_GITHUB_ACCOUNT=jw409 kp-github-mcp
```

The account must already be authenticated with GitHub CLI on `github.com`. Selection reads its stored credential with `gh auth token --hostname github.com --user ACCOUNT`. It does not switch GitHub CLI's active account or change machine-wide configuration.

Credential selection, in order:

1. A nonempty `KP_GITHUB_ACCOUNT` selects a stored GitHub account.
2. When provided by the developer events profile, `github_account` selects a stored account.
3. `GH_TOKEN` takes precedence over `GITHUB_TOKEN`, matching GitHub CLI.
4. With no selector or environment token, the stored active account for `github.com` is used.

A selected account takes precedence over both token environment variables. Invalid selectors and missing selected credentials fail closed; they do not fall back to another developer's token. Empty or whitespace-only selectors are treated as absent. To use a token environment variable instead of an account selector, remove the selector from both the environment and the profile.

The server resolves credentials once at startup and uses the same token for direct HTTP and every GitHub CLI fallback request. CLI children receive that token through their own `GH_TOKEN`; conflicting `GITHUB_TOKEN`, `GH_HOST`, and debug settings are removed from those children. Credentials are never placed in command arguments. Empty credentials cannot fall back to the machine's active account.

Startup makes no GitHub validation request. Existing cached credentials are used directly; authentication or permission failures are returned without switching accounts. After changing the selected account or replacing its credential, restart the MCP server. A rate-limit response does not change the selected credential.

The event app's signing key and webhook secret remain operator credentials. Developers use their own GitHub identity for GitHub operations; registering a listener does not require distributing app signing credentials.
