# Consumer package

The consumer archive contains a built `bin/kp-github-mcp`, the Python 3 standard-library setup helper, account/setup/protocol documentation, and available example configurations. Packaging and extraction do not run the binary, fetch configuration, open a browser, register an MCP server, or change account defaults. App credentials and local developer profiles are not included.

## Download and use

Download the artifact matching your system from the **Consumer Packages** workflow: `consumer-linux-x86_64` for Linux x86-64 or `consumer-macos-arm64` for Apple Silicon macOS. Extract the GitHub artifact wrapper to obtain its `.tar.gz` and `.tar.gz.sha256` files. The package's `MANIFEST.json` records `source_commit`, `source_modified`, target, and payload hashes. Pull-request packages are built from the exact candidate head commit, rather than GitHub's generated merge commit.

Verify and extract the archive from that directory, substituting your target:

```sh
sha256sum -c kp-github-consumer-x86_64-unknown-linux-gnu.tar.gz.sha256
tar -xzf kp-github-consumer-x86_64-unknown-linux-gnu.tar.gz
cd kp-github-consumer-x86_64-unknown-linux-gnu
sha256sum -c SHA256SUMS
python3 scripts/github_events_setup.py --help
```

On macOS, use `shasum -a 256 -c` in place of `sha256sum -c`. The checksum files detect corruption; trust depends on the workflow/source you downloaded them from. Linux packages built on `ubuntu-latest` require a compatible glibc runtime; they are not static or universal Linux binaries. Select the matching architecture.

Use the operator's bundle or known GCS object with the setup helper, following [developer setup](developer-events-setup.md). Then run `scripts/install.sh --github-account YOUR_GITHUB_LOGIN --events-config /absolute/path/events.json`, or `scripts/install.sh --accounts-config /absolute/path/accounts.json`. The stock installer starts a shared local HTTP user service and registers Claude Code; use `--no-register` for another host. Extraction alone changes no installed configuration. `gh` and, for events, `gcloud` are external prerequisites using your own logins.

Ordinary GitHub tools work without an events profile. Listener registration and delivery additionally require a deployed consumer API and developer access policy described in [the listener protocol](listener-api.md). The webhook ingress URL alone is not that API. An open source adopter supplies its own app and receiver unless explicitly onboarded to Meshly's instance.

## Build an archive locally

From a checkout with the desired source revision, build the target explicitly and package that executable:

```sh
python3 scripts/binary_provenance.py build \
  --target x86_64-unknown-linux-gnu \
  --output target/x86_64-unknown-linux-gnu/release/kp-github-mcp
python3 scripts/package_consumer.py \
  --binary target/x86_64-unknown-linux-gnu/release/kp-github-mcp \
  --target x86_64-unknown-linux-gnu \
  --source-root . \
  --output-dir dist
```

Commit the build inputs first. The build helper compiles that commit in a fresh
directory with the pinned Rust version and emits a `.provenance.json` sidecar.
The packager requires this receipt, checks the executable hash and target,
resolves its source commit in Git, and requires the build inputs to match the
current clean checkout. Arbitrary executables, stale source, a missing receipt,
zeroed source commit, or altered bytes fail before an archive is written.
`--expect-commit FULL_SHA` additionally pins the packaging checkout.

A receipt is not a signature. CI separately rebuilds each checked-in platform
binary from its declared source and compares the actual output bytes. Rewriting
a binary and its checksum together cannot satisfy that comparison unless the
rebuild agrees. Trusted review of source and CI remains the trust boundary.

With identical source revision/state, payload bytes, binary, and target, repeated packaging produces identical archive bytes. Archive paths are relative beneath one fixed package directory; permissions, ownership, ordering, gzip header, and timestamps are normalized. The explicit payload allowlist excludes unlisted checkout files, private configuration, credentials, caches, and Git metadata. Optional example files are included only when present under their documented names.

The **Consumer Packages** workflow uploads artifacts for review without repository write permissions. The older **Build Binaries** workflow retains marketplace publication only for an explicit dispatch on `main`; feature and tag builds cannot commit to `main`, and publication fails if `main` advanced after its source was built.

For several sessions sharing explicitly enabled accounts and organizations on a machine, see
[shared local HTTP](shared-local-server.md). The archive includes
`scripts/install.sh` and `scripts/install_local_http.py` for stock installation.
The preparation-only `scripts/setup_shared_http.py` remains available for manual
supervision. The package is a developer client, not the GCP webhook receiver.

## Checked-in binaries

```sh
python3 scripts/binary_provenance.py verify
python3 scripts/binary_provenance.py verify --require-current
python3 scripts/binary_provenance.py reproduce --platform linux-x86_64
```

The first check validates both tracked payloads against their declared source
receipts and is part of ordinary Python test discovery. The second additionally
rejects stale binaries after a source change and runs for each platform in CI.
The third builds the declared
revision from scratch and requires identical bytes; use `macos-arm64` on a
native macOS runner. Full Git history and the receipt's exact Rust toolchain are
required. Cargo registry paths are normalized to the native Actions paths so
panic locations and C source filenames do not depend on the developer's home.
Compiler/SDK changes can still fail reproduction; such failures block the gate
and require investigation, never automatic acceptance of new hashes.

The Linux payload has been rebuilt from the corrected account-scoping source.
The macOS payload still declares its historical source and lacks those changes;
its native replacement build and verification are pending. Do not publish a
combined release until both platforms pass `verify --require-current`. Build
a fresh consumer package to test source changes on either native platform.
The macOS provenance CI job will fail closed until its payload is refreshed.
An explicit feature-branch **Build Binaries** dispatch produces both native
artifacts and receipts without committing to main. Download those artifacts,
run `assemble --artifacts DIRECTORY`, and commit the verified pair for review.
The publication job assembles both artifacts and validates their receipts against
the publication checkout before committing them; it does not skip CI.
