# GCS developer-events configuration contract

This is the contract for the receiver/infrastructure owner to implement in their `.tf` configuration. The repository supplies validated sample bytes and a schema. It does not supply deployable Terraform, create a bucket or IAM grant, provide a production receiver endpoint, or claim that a production object has been published.

The existing target bucket is `meshly-corp-tool-configs`. Reuse its established owner and state; preserve uniform bucket-level access and enforced public access prevention. Bucket lifecycle, versioning, retention, IAM membership, publication permissions, and authoritative Terraform state remain the infrastructure owner's responsibility.

## Separate sample and production objects

| Purpose | Exact location |
| --- | --- |
| Proposed production bundle | `gs://meshly-corp-tool-configs/everyone/github-events-config-key` |
| Sample artifacts only | `gs://meshly-corp-tool-configs/everyone/github-events/samples/v1/` |

The sample prefix mirrors the filenames in [examples/github-events](../examples/github-events/README.md): `meshly.sample.json`, `meshly.sample.bundle`, `selfhost.sample.json`, `selfhost.sample.bundle`, `shared-config.schema.json`, `receiver-inputs.sample.json`, and `README.md`. Uploading these samples does not authorize or enable production configuration. Keep the reserved `.invalid` endpoints and placeholder audience in published samples. Never copy sample bundle bytes to the proposed production object.

`receiver-inputs.sample.json` is a separate operator input contract. It records App `5327950` (`meshly-open-source-dev-events`), the two allowed organization installations, the existing `meshly-github-runners` dispatcher context, proposed dedicated Secret Manager names, and missing receiver outputs. It intentionally fails developer bundle validation. Secret-version placeholders must become explicit numeric versions; neither secret contents nor a `latest` binding belong in this input contract. The receiver owner must integrate these references without overwriting the existing runner App's secrets.

Its developer authorization example is deliberately disabled and uses illustrative Google principals and organization/repository scopes. The owner should reuse the existing Meshly developer access policy, with verified Google subjects or group membership. No Google-to-GitHub mapping is required: the GitHub account header is only a caller-supplied namespace. Policy may grant Meshly developers all onboarded installed repositories; that is event access independent of individual GitHub ACLs. Future organizations require explicit onboarding, installation coverage and policy scope. No actual grant is created by these examples. The operator input contract is version 2; the developer bundle remains version 1.

## Exact production object format

The production object contains the output of:

```sh
python3 scripts/github_events_setup.py pack \
  --config-file receiver-approved-shared-config.json
```

The bytes are exactly one ASCII line: `kpghcfg1_`, followed by unpadded URL-safe base64 of canonical compact JSON, followed by one LF (`0x0a`). There is no UTF-8 BOM, carriage return, quote wrapping, JSON envelope, or additional comment line. Canonical JSON uses sorted keys and compact separators, matching the existing `pack` command. The complete object, including LF, must fit within the bootstrap reader's 64 KiB limit. This is serialization of nonsecret configuration, not encryption.

Set production object metadata to `Content-Type: text/plain; charset=utf-8` and `Cache-Control: no-store`. The latter avoids intermediary caching during an explicitly requested fetch; it does not change the client's intentional local cache reuse. Packed sample `.bundle` objects use the same metadata. Sample `.json` files use `application/json`, the schema uses `application/schema+json`, and `README.md` uses `text/markdown; charset=utf-8`.

Shared JSON has required `version`, `endpoint`, `audience`, `allowed_orgs` and optional `app_slug`. It contains no developer names, App PEM, webhook secret, user token, installation token, or client secret. [The JSON Schema](../examples/github-events/shared-config.schema.json) rejects unknown fields. Run `pack` as the final validation and serialization step: portable JSON Schema does not fully express UTF-8 byte limits, case-insensitive organization uniqueness, or all URL-port checks. Enable the validator's URI-format checking when available; JSON Schema 2020-12 normally treats `format` as annotation. [JSON Schema validation specification](https://json-schema.org/draft/2020-12/json-schema-validation)

`endpoint` is the authenticated **consumer API** URL, separate from the GitHub webhook ingress URL. `audience` is the actual Google ID-token audience approved by the receiver's verification policy. A human `gcloud` account cannot mint an arbitrary audience simply because configuration requests it. The owner must supply both values after choosing and validating the receiver authentication mode; see [the listener API contract](listener-api.md). The sample audience `receiver-approved-google-oauth-client-id` is intentionally not a usable value.

## Known-object reads and IAM

Developers use the exact object URI with `gcloud storage cat`; setup never lists objects or buckets. The desired developer binding is `roles/storage.objectViewer` on the existing bucket, scoped by an IAM condition to this exact production object:

```text
resource.type == "storage.googleapis.com/Object" &&
resource.name == "projects/_/buckets/meshly-corp-tool-configs/objects/everyone/github-events-config-key"
```

Do not add a bucket-level `resource.type` alternative to this condition. Listing is evaluated on the bucket, so this object-specific grant is not a way to provide a filtered bucket listing. Other inherited or existing grants can still allow listing or broader reads; this proposal does not remove them or claim that the effective account permissions are narrower. The infrastructure owner should review effective access separately. [Cloud Storage IAM conditions](https://docs.cloud.google.com/storage/docs/access-control/iam?hl=en)

Sample access can use separate explicit object grants or a separately reviewed sample-prefix condition. It must not broaden production access to all of `everyone/`. Use an explicit list of approved IAM principals or groups supplied by the owner. Keep publication permissions separate from developer read permission; do not give developers object write or bucket administration just to bootstrap. Existing unconditional bucket grants, bucket policy ownership, and publisher grants are outside this contract's changes.

## Inputs and outputs for the infrastructure owner

These are proposed names and types for the owner's module, not Terraform declarations:

| Input | Type and meaning |
| --- | --- |
| `config_bucket_name` | Existing bucket name: `meshly-corp-tool-configs`; no new bucket or ownership transfer |
| `config_object_name` | Production object name: `everyone/github-events-config-key`, kept distinct from the sample prefix |
| `receiver_endpoint` | Deployed HTTPS consumer API base URL approved by its owner |
| `receiver_token_audience` | Actual emitted Google ID-token audience accepted by the receiver |
| `allowed_github_orgs` | Explicit organization allowlist; initially `Meshlyai`, `Meshly-Open-Source` |
| `github_app_slug` | Public App slug: `meshly-open-source-dev-events` |
| `developer_reader_members` | Explicit approved IAM principals/groups for the exact-object conditional reader binding |
| `publisher_principal` | Existing or explicitly provisioned operator identity with publication permission, maintained separately from readers |
| `receiver_app_secret_references` | Dedicated Secret Manager names and explicit numeric versions; receiver-only inputs, excluded from the developer object |
| `developer_authorization_records` | Verified Google principals or existing developer group policy, explicit organization/repository scopes, and access-policy evidence; no linked GitHub identities; excluded from the shared bundle |
| `bucket_state_owner` | Existing authoritative state/owner reference for coordinating versioning, lifecycle, IAM, and publication changes |

| Output/evidence | Purpose |
| --- | --- |
| `github_events_config_gcs_uri` | Exact known production object URI for the developer setup command |
| `github_events_config_generation` | Published GCS generation recorded by the operator for provenance and rollback |
| `github_events_config_sha256` | SHA-256 of exact packed object bytes, including the final LF |
| `github_events_config_content_type` | Confirm `text/plain; charset=utf-8` |
| `github_events_config_schema_version` | Confirm shared version `1` |
| `github_events_consumer_endpoint` | The receiver's actual consumer API URL, distinct from ingress |
| `github_events_token_audience` | The receiver-approved actual emitted audience |
| `github_events_reader_condition` | Final exact-object condition and bound principal list, retained as deployment evidence |

The final developer command uses the published output URI, an existing `gh` account, and their selected Google account. Setup trusts a valid local cache; changing the published generation does not force every developer to re-fetch. Operators distribute an explicit refresh instruction when configuration changes. A user's local GitHub name and configured organization list do not grant backend access; the receiver must verify the Google identity and developer access policy and installed repository coverage.

## Acceptance boundary

Local sample tests establish format compatibility and reject secret-bearing or operator-only inputs. They do not establish publication, developer IAM, developer authorization, receiver deployment, or event delivery. The infrastructure owner should record the production generation/hash and a known-object read with an intended developer identity, then separately verify authenticated registration and event consumption. Object versioning and rollback policy remain with that owner; the consumer does not need bucket listing permission for these checks.
