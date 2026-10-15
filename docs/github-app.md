# GitHub event app registration

`scripts/register_github_app.py` implements GitHub's
[App manifest flow](https://docs.github.com/en/apps/sharing-github-apps/registering-a-github-app-from-a-manifest).
It prepares registration, receives the one-time browser callback, saves the app
credentials privately, and verifies installations through the GitHub API.
Python 3 and authenticated `gh` accounts are required; no Python packages are needed.

The manifest grants repository **read** access to pull requests, checks, Actions,
commit statuses, and automatic metadata. It subscribes to `pull_request`,
`pull_request_review`, `check_run`, `check_suite`, `workflow_run`, and `status`.
It requests no organization permissions or write permissions.

## Register and install

Run on the machine where the browser opens. For a remote development machine,
forward the local callback port with SSH first (`ssh -L 8765:127.0.0.1:8765 HOST`).
Use a new output directory outside all repositories:

```sh
python3 scripts/register_github_app.py \
  --org 'YOUR_ORG:YOUR_GH_ACCOUNT:Your Developer Events' \
  --webhook-url 'https://YOUR_RECEIVER/github/events' \
  --output-dir "$HOME/.config/kp-github/app-registration-$(date +%Y%m%d-%H%M%S)"
```

Repeat `--org` to register one private app per organization. GitHub private apps
can be installed only in their owning account. `--public` permits installing an
app outside its owning organization; it does not provide hosting for anyone.

For one app owned by Meshlyai and installed in both Meshly organizations:

```sh
python3 scripts/register_github_app.py \
  --org 'Meshlyai:jw408:Meshly Developer Events' \
  --install-org 'Meshly-Open-Source:jw409' \
  --public \
  --webhook-url 'https://YOUR_MESHLY_RECEIVER' \
  --output-dir "$HOME/.config/kp-github/meshly-app-$(date +%Y%m%d-%H%M%S)"
```

Create the app once under Meshlyai, then use its installation link twice, selecting
each organization in turn. The helper verifies both using their specified CLI
accounts. GitHub's public setting permits cross-organization installation; the
Meshly receiver must still enforce its own installation/organization allowlist.

The tool prints a local URL; it never opens a browser automatically. Open that
URL yourself. On the setup page, click **Create GitHub App**, confirm the organization and
app name on GitHub, then follow **Install this app on the organization**. Select
the repositories whose events the organization needs. Return to the setup page
and click **Verify installations**. Keep the process running until both steps
are complete; manifest handshakes expire after one hour.

If the helper is interrupted, rerun the same command with `--resume`. It reuses
the setup URL and saved app registration. Reload the page before submitting its
verification form after a restart. Do not create a second app to recover an
already completed registration.

The browser uses its own GitHub login. The API checks use the account named in
`--org`: the helper retrieves its stored token and sets `GH_TOKEN` only in the
child process. It never changes the default `gh` account or git identity.

Each organization's output directory contains:

- `manifest.json`: exact requested registration configuration.
- `credentials.json`: private key, webhook secret, client secret, and app metadata.
  This is **receiver/operator material**, never a developer bootstrap bundle.
- `registration.json`: non-secret confirmation of app creation.
- `installation-ORG.json`: non-secret confirmation of each installation and permissions,
  created only after verification against GitHub.

Directories have mode 0700 and files mode 0600. Callbacks, codes, credentials,
and GitHub error bodies are not logged. Existing output files are never replaced.
If registration returns an unexpected owner or permissions, its credentials are
preserved for operator recovery, and setup does not advance to installation.

## Connect the receiver

New apps start with **webhooks inactive**. Registering or installing an app does
not make its listener functional. Before activation, the receiver must accept
these event families, verify each app's HMAC secret, durably acknowledge accepted
deliveries, and expose authorized feeds with a separate cursor per consumer.
The dispatcher owner should put signing secrets in the receiver's secret store
and then enable the app's webhook and verify real deliveries.

Meshly hosts one receiver for Meshlyai, Meshly-Open-Source, and future organizations
that Meshly explicitly onboards. New organizations need installation and receiver
allowlist entries; public app visibility does not authorize their event feeds.
Other users of this source deploy their own receiver and register their own app.
The optional `github_events` MCP tool implements a [proposed consumer
API](listener-api.md), with [developer setup](developer-events-setup.md) from a
shared config bundle. Receiver implementation, authorization, and live delivery
verification remain separate work owned by the deployment operator.

Event interpretation must preserve the repository, installation, PR, head SHA,
delivery ID, and workflow/check attempt:

- Approved: `pull_request_review`, action `submitted`, review state `approved`.
  A single approval does not prove that all branch review requirements are met;
  later dismissals and new commits must update the listener's state.
- Merged: `pull_request`, action `closed`, `pull_request.merged == true`.
- CI failed: completed checks or workflow runs with a failure conclusion, or a
  commit `status` of `failure`/`error`. Keep canceled, timed out, and action-required
  outcomes distinct; do not apply a prior head's failures to the current head.

Developer setup can reuse each developer's `gh` identity for existing MCP calls
and `gcloud` identity for the internal feed/config service. A shared config
bundle can contain service URLs, app/installation IDs, and feed settings. It
must not give every developer the app signing key or webhook HMAC secret.

## Validate the helper

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -p 'test_app_registration.py' -v
```
