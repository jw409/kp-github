# Developer events setup

A Meshly developer needs their own existing GitHub login in `gh`, Google credentials for the Meshly consumer service, the operator's shared configuration bundle or exact GCS object path, and a `kp-github-mcp` build with listener support. Setup selects accounts for this process. It never changes the default GitHub or Google account and does not open a browser.

The Meshly GitHub App receives organization events on Meshly infrastructure. Developers do not register another app and do not receive its private signing key, webhook secret, installation token, or anyone else's personal token. Other organizations using an open source copy operate their own app and receiver unless Meshly explicitly onboards them.

## Install your profile

Run from the kp-github checkout, using the **exact object path supplied by the operator**:

```sh
python3 scripts/github_events_setup.py setup \
  --gcs-object gs://YOUR-CONFIG-BUCKET/YOUR-KNOWN-OBJECT \
  --github-account jw409 \
  --google-account developer@meshly.ai
```

The example object path is a placeholder; this change does not publish a Meshly configuration object. Reading one known object needs permission to read that object. Setup does not list the bucket. `gcloud storage cat` receives `--account` when selected, has a 30-second timeout, and does not change `gcloud` configuration.

For copy/paste distribution, use the same versioned bundle:

```sh
python3 scripts/github_events_setup.py setup \
  --bundle 'kpghcfg1_PASTE-THE-OPERATOR-BUNDLE-HERE' \
  --github-account jw409 \
  --google-account developer@meshly.ai
```

Pasting a bundle avoids the GCS read; the listener connection still uses the developer's Google identity. An operator-provided JSON file also works with `--config-file path/to/shared-config.json`.

Setup writes `~/.config/kp-github/events.json` and prints a launch command:

```sh
KP_GITHUB_ACCOUNT=jw409 \
  KP_GITHUB_EVENTS_CONFIG="$HOME/.config/kp-github/events.json" \
  kp-github-mcp
```

Use those same environment variables in the MCP host's server configuration. The profile's GitHub account is the default selection when no explicit `KP_GITHUB_ACCOUNT` override is present. Existing GitHub operations use that developer's credentials; the event receiver independently authenticates the Google identity and verifies its GitHub/repository authorization. A name in a local profile is not an access grant.

The helper creates private directories with mode `0700` and atomically writes files with mode `0600`. An existing output directory must already be private and owned by the current user; setup will not change permissions on an existing home, checkout, or other directory. Symlinks in the destination path or profile files are rejected. Use `--output` to choose a dedicated private directory for a separate profile.

## Cached configuration

Run `setup` again without a source to use the existing profile. Repeating the same GCS source also uses the cache. Neither path revalidates GitHub or Google credentials, reads GCS, or rewrites an unchanged profile. Changing only `--github-account` or `--google-account` updates the local selection without fetching configuration again.

To read an updated shared configuration deliberately, provide its source and `--refresh`:

```sh
python3 scripts/github_events_setup.py setup \
  --gcs-object gs://YOUR-CONFIG-BUCKET/YOUR-KNOWN-OBJECT \
  --refresh
```

A changed source or changed JSON/bundle content also replaces the cached configuration after validation. A fetch failure, timeout, malformed bundle, or rejected secret field leaves the valid cached profile intact. Command stderr is not echoed because it can include sensitive diagnostics.

The private `events.json.source.json` sidecar stores source and profile hashes, not source credentials. Binding these hashes prevents a crash between the two atomic writes from making one source appear to own another source's profile. Missing or mismatched metadata triggers a read only when an explicit source is supplied; a valid local profile remains usable on its own. The helper does not run background refreshes or invalidate configuration after rate limiting. Runtime identity-token caching and relevant authorization failures are handled by the listener client.

## Prepare a shared bundle

The operator supplies a JSON object with only these fields:

```json
{
  "version": 1,
  "endpoint": "https://events.example.test",
  "audience": "receiver-approved-google-oauth-client-id",
  "allowed_orgs": ["Meshlyai", "Meshly-Open-Source"],
  "app_slug": "meshly-open-source-dev-events"
}
```

`endpoint` is the **consumer API**, not GitHub's webhook ingress URL. The example is not a deployed endpoint. `audience` must match the receiver's authentication configuration; a human `gcloud` account cannot mint an arbitrary custom-audience token just because a profile requests one. The receiver owner must provide the compatible audience and identity-verification policy described in [the listener API contract](listener-api.md).

`version`, `endpoint`, `audience`, and `allowed_orgs` are required; `app_slug` is optional. URLs require HTTPS and cannot contain credentials, a query, or a fragment. `allowed_orgs` contains 1–100 distinct GitHub organization names. Audience values contain no whitespace. Unknown fields are rejected, including App keys, webhook secrets, client secrets, and tokens. The local profile adds `github_account` and optionally `google_account`; those individual choices do not belong in the shared bundle.

```sh
python3 scripts/github_events_setup.py pack \
  --config-file shared-events-config.json > developer-events.bundle
```

The `kpghcfg1_` prefix identifies version 1; its payload is URL-safe base64 JSON. Encoding is not encryption. The allowed schema contains configuration only. An operator can publish this file to the known GCS object using their existing write authorization or share the exact bundle for copying. The helper does not create buckets, change IAM, publish objects, create registrations, or modify cloud services.

For offline bootstrap tests, `--allow-http-loopback` permits HTTP only on `localhost`, `127.0.0.1`, or `::1`. This option does not enable HTTP in the production Rust listener client; production profiles must use HTTPS.

## Current delivery boundary

The helper implements local configuration installation and packing. Live listener registration and event delivery additionally require the receiver's consumer API, identity mapping, and per-developer access to be deployed. A successfully written profile is evidence of local setup, not proof that those backend services are available or that a developer has been authorized.
