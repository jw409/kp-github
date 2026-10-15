# Accounts and organizations

Use an explicit process default for work that must stay on one identity:

```sh
KP_GITHUB_ACCOUNT=jw408 kp-github-mcp
```

This is the Meshly default example. Other developers substitute their own login.
The account must already be authenticated in `gh` on `github.com`. Reading a
[named stored token](https://cli.github.com/manual/gh_auth_token) never changes
GitHub CLI's active account or machine-wide settings.

Without multi-account configuration, credentials resolve in this order:

1. `KP_GITHUB_ACCOUNT`, if set, selects that stored account exclusively.
2. `GH_TOKEN`, then `GITHUB_TOKEN`.
3. The existing stored default for `github.com`.

An empty/invalid explicit selector or an unavailable selected credential fails
closed. An event profile never supplies the GitHub credential selector. Profiles
are loaded only through an explicit `KP_GITHUB_EVENTS_CONFIG`; a file appearing
at `~/.config/kp-github/events.json` has no runtime effect. A malformed, missing,
or mismatched event profile disables only `github_events`, with a visible error.
No GitHub credential lookup is performed on behalf of a profile's account name.

## Several accounts in one process

Opt in with `KP_GITHUB_ACCOUNTS_CONFIG=/absolute/path/accounts.json`:

```json
{
  "version": 1,
  "accounts": [
    {"github_account": "work-dev", "events_config": "/home/developer/.config/kp-github/work-events.json"},
    {"github_account": "other-dev", "events_config": "/home/developer/.config/kp-github/other-events.json"}
  ]
}
```

These are placeholders, not account defaults. `events_config` is optional and
must be an absolute path. Configure each profile with `github_events_setup.py
setup --github-account ACCOUNT --output ABSOLUTE_PATH ...`. Each profile can
list multiple organizations in `allowed_orgs`; accounts may share organizations.
Its `github_account` must match the enclosing entry. Each account may choose its
own Google identity and receiver. A bad profile disables only that entry's events.
The single-profile `KP_GITHUB_EVENTS_CONFIG` variable is ignored in this mode.

The file is an explicit allowlist of credentials this process may use; it is
never discovered automatically. There is no implicit first-account default.
Set `KP_GITHUB_ACCOUNT` to an account in the list to choose a default, or supply
`github_account` on every tool call. Ambient tokens and the active `gh` account
are not used in multi-account mode.

Every one of the 67 tools accepts `github_account`. For example:

```json
{"name":"github_prs_get","arguments":{"github_account":"work-dev","owner":"FirstOrg","repo":"service","number":42}}
```

The same account can operate on `SecondOrg` by changing `owner`, and another
request can select `other-dev` without changing anyone else's request. Existing
`owner`, `org`, and search-query arguments still specify the target; they never
choose an identity. GitHub enforces repository and organization permissions.
The local events `allowed_orgs` list is a convenience check, not an access grant.

Named resources use `github://accounts/work-dev/repos/FirstOrg/service`, with
optional `/issues`, `/pulls`, or `/readme`. Legacy `github://repos/...` URIs use
only the explicit default in multi-account mode; without one they fail clearly.

Credentials are loaded lazily once per account, with one lookup for concurrent
first requests. A missing account fails only its request and never falls back.
Successful clients and their connection pools are shared across sessions;
failed lookups are retryable. GitHub API failures do not select another account.
Restart the daemon after replacing GitHub credentials or editing configuration.
No startup GitHub validation request is made. Event ID tokens keep their existing
expiry/401 refresh behavior, in a separate cache per account.

Direct HTTP and `gh` fallback requests use the same selected token. CLI children
receive it through their own `GH_TOKEN`, with conflicting token/host/debug
variables removed. Tokens are never placed in command arguments. A shared HTTP
bearer key grants access to all accounts explicitly enabled on that daemon;
account selection separates requests, not mutually untrusted local users.
Commit-author defaults remain process-wide and separate from API authentication.
