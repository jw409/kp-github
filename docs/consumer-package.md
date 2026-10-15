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

Use the operator's bundle or known GCS object with the setup helper, following [developer setup](developer-events-setup.md). Configure your MCP host to launch the absolute path to the extracted `bin/kp-github-mcp` and set the profile/account environment variables printed by setup. This archive does not install a machine-wide executable or alter the host's MCP configuration. `gh` and, for events, `gcloud` are external prerequisites using your own logins.

Ordinary GitHub tools work without an events profile. Listener registration and delivery additionally require a deployed consumer API and verified identity mapping described in [the listener protocol](listener-api.md). The webhook ingress URL alone is not that API. An open source adopter supplies its own app and receiver unless explicitly onboarded to Meshly's instance.

## Build an archive locally

From a checkout with the desired source revision, build the target explicitly and package that executable:

```sh
cargo build --locked --release --target x86_64-unknown-linux-gnu
python3 scripts/package_consumer.py \
  --binary target/x86_64-unknown-linux-gnu/release/kp-github-mcp \
  --target x86_64-unknown-linux-gnu \
  --source-root . \
  --output-dir dist
```

The packager requires a nonempty executable file. It reads the checkout's actual Git HEAD; `--expect-commit FULL_SHA` additionally rejects a mismatched checkout. It does not infer whether a supplied executable was built from those sources or whether its architecture matches `--target`. The workflow establishes that correspondence by building the exact checked-out source and passing its target output directly. Binary SHA-256 is recorded independently.

With identical source revision/state, payload bytes, binary, and target, repeated packaging produces identical archive bytes. Archive paths are relative beneath one fixed package directory; permissions, ownership, ordering, gzip header, and timestamps are normalized. The explicit payload allowlist excludes unlisted checkout files, private configuration, credentials, caches, and Git metadata. Optional example files are included only when present under their documented names.

The **Consumer Packages** workflow uploads artifacts for review without repository write permissions. The older **Build Binaries** workflow retains marketplace publication only for an explicit dispatch on `main`; feature and tag builds cannot commit to `main`, and publication fails if `main` advanced after its source was built.
