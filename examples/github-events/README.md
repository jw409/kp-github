# Sample files only — never enable these as production configuration

These fixtures show the exact developer configuration format. Both endpoints use the reserved `.invalid` domain and the audience is an explicit placeholder. They do not identify a deployed service and cannot provide a production connection. Replace endpoint and audience with receiver-owner-approved values in a separate production source file; do not promote the sample bytes to the production object.

| File | Purpose |
| --- | --- |
| `meshly.sample.json` | Meshly organization/App metadata with an intentionally unusable service endpoint and audience |
| `meshly.sample.bundle` | Exact `kpghcfg1_` bytes produced from the Meshly sample, with one trailing LF |
| `selfhost.sample.json` | A separate operator's example configuration; Meshly does not host this receiver |
| `selfhost.sample.bundle` | Exact packed bytes produced from the self-hosting sample, with one trailing LF |
| `shared-config.schema.json` | JSON Schema for the nonsecret shared object before packing |
| `receiver-inputs.sample.json` | Separate operator input contract with App/install metadata and proposed secret references; deliberately not a developer bundle |

The shared schema forbids extra fields, including developer profile identities and App signing keys, webhook secrets, or access tokens. `kpghcfg1_` is base64url serialization, not encryption. Schema validation checks structure; the existing pack command performs the remaining byte-length, URL, and case-insensitive organization checks.

`receiver-inputs.sample.json` contains public registration identifiers and proposed resource names only. It is not accepted by `pack`, is not a `.tf` implementation, and contains no secret values. Its `TO_BE_PROVISIONED` endpoint, audience, and secret-version markers require the receiver owner's decisions and resources. Secret names are separate from the existing runner App's secrets, and versions must become explicit numeric versions before deployment.

Regenerate the packed fixtures from the repository root:

```sh
python3 scripts/github_events_setup.py pack \
  --config-file examples/github-events/meshly.sample.json \
  > examples/github-events/meshly.sample.bundle
python3 scripts/github_events_setup.py pack \
  --config-file examples/github-events/selfhost.sample.json \
  > examples/github-events/selfhost.sample.bundle
```

The intended sample publication prefix is `gs://meshly-corp-tool-configs/everyone/github-events/samples/v1/`. The distinct proposed production object is `gs://meshly-corp-tool-configs/everyone/github-events-config-key`. Presence of these repository fixtures does not establish that either path has been published or that production access exists. Publication receipts belong to the operator performing the upload.

The [GCS configuration contract](../../docs/gcs-config-contract.md) describes exact bytes, content types, conditional access, and inputs/outputs for the infrastructure owner's `.tf` implementation. No Terraform is supplied here.
