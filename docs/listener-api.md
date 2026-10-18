# Developer event consumers: v1 protocol

**Status: client and receiver wire contract implemented.** Each operator supplies
a deployed consumer endpoint, accepted audience, and developer access policy.
Local tests and successful listener API calls do not establish real GitHub event
delivery; verify the webhook-to-read-to-ack path separately. The GitHub webhook
ingress URL is not automatically a consumer API endpoint.

Meshly's instance can serve Meshlyai, Meshly-Open-Source, and subsequently onboarded
organizations. An open source adopter supplies its own app and receiver. This
client does not register arbitrary customers with Meshly's receiver.

## Developer configuration and identity

The optional local profile is read only from an explicit `KP_GITHUB_EVENTS_CONFIG`,
or from a named account entry in `KP_GITHUB_ACCOUNTS_CONFIG`:

```json
{
  "version": 1,
  "endpoint": "https://events.example.com",
  "audience": "receiver-approved-google-oauth-client-id",
  "allowed_orgs": ["Meshlyai", "Meshly-Open-Source"],
  "github_account": "jw408",
  "google_account": "developer@example.com",
  "app_slug": "meshly-open-source-dev-events"
}
```

The example endpoint and audience are placeholders. The receiver operator must
publish the real values. The optional `google_account` pins a stored gcloud
account; supplying it is recommended. GitHub API credentials are selected independently via an explicit process default
or the request's `github_account` selector; this profile cannot change them. The profile does not
prove that those GitHub and Google accounts belong to the same person.

An unconfigured profile leaves ordinary GitHub operations available. An
explicitly named missing profile, invalid profile, or profile symlink disables
only that account's event tool and reports the configuration error. A file at
the default setup output path is never loaded implicitly. Profiles are limited
to 64 KiB and must be regular files. Production consumer endpoints require HTTPS
and cannot contain credentials, queries, fragments, whitespace, or backslashes.
Only test constructors allow HTTP on loopback. Redirects are never followed.

On the first event operation, the client runs the equivalent of:

```sh
gcloud auth print-identity-token --quiet --account developer@example.com
```

This reads existing gcloud credentials without changing the active account or
machine configuration. Acquisition is bounded to 15 seconds, output to 32 KiB,
and the subprocess is killed on cancellation. No app private key, webhook HMAC
secret, service account key, or GitHub access token is sent to the consumer API.
There is no static bearer-token environment override.

The child process explicitly clears credential-file, access-token, and
impersonation overrides, including values inherited from persisted gcloud
properties; otherwise those settings can take precedence over `--account`. The
client also checks that the emitted token's email matches a configured
`google_account`. This is a local consistency check on gcloud output, not a
substitute for the receiver's authentication checks.

**Human gcloud ID tokens cannot request an arbitrary `--audiences` value.** The
installed SDK's `surface/auth/print_identity_token.py` rejects that flag for
credentials that cannot issue audience-specific tokens. This client therefore
omits it. The profile's `audience` must equal an audience actually emitted by the
chosen gcloud credential and accepted by the receiver. It is not inferred from
the endpoint URL. The client compares the locally decoded `aud` claim (string or
array) with that value to report configuration errors before sending a request.

The receiver must independently verify Google's signature, issuer, expiry,
audience, verified identity, and access. Decoding a JWT locally is not verification.
The receiver or its gateway must deliberately support the human gcloud token
audience; an arbitrary Cloud Run service audience or IAP client ID does not become
compatible merely by writing it into this profile. Choose an ingress and verifier
that support these existing human credentials, and test the actual accepted
audience before publishing configuration. A new interactive login portal, account
linking flow, or shared service account key is not part of this design.

## Receiver ownership and authorization

The existing GCP receiver owned by `github-actions-runner` implements the consumer
API below alongside its GitHub webhook ingress. The developer-local HTTP server
is an MCP transport shared by local sessions; it does not receive GitHub webhooks
or grant remote access. Its private bearer key never authenticates to GCP.
Keep GitHub webhook ingress authenticated by the App webhook signature; authenticate
consumer routes separately with Google identity. Do not require Google login on
the webhook route, which GitHub must be able to call. A separate IAM-protected
consumer service or application-level verification on the existing receiver are
valid deployment choices; the receiver owner selects the ingress boundary.

**No Google-to-GitHub linking flow, pair registration, or GitHub credential upload
is required.** Developers independently choose stored `gh` and `gcloud` accounts.
GitHub credentials stay local and authenticate ordinary GitHub API calls directly.
The selected Google identity authenticates event consumption to the GCP receiver.
Changing the GitHub selector never changes the Google principal's access.

The receiver owner implements these checks:

1. Verify the Google ID token as described above and resolve its stable `(iss, sub)`.
2. Require membership in Meshly's configured developer access policy. This can be
   backed by the existing Google/IAM access mechanism or an explicit principal/group
   policy. A valid token from an arbitrary Google account is not sufficient. A
   gateway must pass a verified identity through a trusted boundary; never trust
   caller-supplied forwarded identity headers. Reading the GCS bootstrap object is
   configuration delivery, not an event access grant.
3. Intersect that policy's org/repo scope with the receiver's onboarded App
   installation/repository set. Meshly may grant its developer group all installed
   repositories in Meshlyai and Meshly-Open-Source. That is an intentional event
   access grant independent of each developer's GitHub repository permissions.
   Future orgs need explicit onboarding and policy scope; installing the public
   App alone does not enroll an unrelated organization in Meshly's service.
4. Scope listener ownership, idempotency, deliveries and acks to the verified
   Google principal and the requested account namespace. Deny access across
   Google principals even when the namespace or listener ID matches. Apply
   current policy on reads/acks as well as registration; removed grants or
   uninstalled repositories cannot continue disclosing queued private events.

The App installation establishes the organization's authorization of the App's
repository access; it does not authenticate the caller of this separate API.
`X-KP-GitHub-Account` is an untrusted **namespace label**, never proof of GitHub
identity or an authorization input. Record it as caller-supplied attribution in
audits, alongside the verified Google principal. Account namespaces separate
queues for convenience within one principal, not a security boundary between
identities controlled by the same developer. Names in local profiles and
`allowed_orgs` are convenience guards and cannot widen the receiver policy.

GitHub documents the distinction between [installation authorization](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/authenticating-as-a-github-app-installation)
and [user authorization](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app).
This contract uses the former for capture and Google authorization for consumers;
it does not claim to mirror per-user GitHub ACLs.

## Cache and failure behavior

The client trusts its cached ID token until its expiry approaches (15-second
margin). It does not revalidate credentials, app installation, or org membership
before every request. Concurrent token acquisitions are serialized.

- A 401 invalidates only the token rejected by that request and permits one
  credential refresh and one retry. A second 401 is returned. A late 401 from an
  older request cannot invalidate a newer token.
- A 403, 429, or 5xx is returned without refreshing credentials or automatically
  retrying. Numeric `Retry-After` is retained in the error.
- Network failures and timeouts do not trigger mutation retries. They can leave
  the outcome uncertain. Repeat registration with the **same idempotency key**.
- The first locally obtained token's `(iss, sub)` is pinned for that client
  session, including across expiry and 401 refresh. A change fails rather than
  silently moving listener operations to a different gcloud account. This check
  only detects local identity drift; receiver verification remains mandatory.
- Profile changes take effect when the MCP process restarts, creating fresh
  configuration and token state.

Requests have a 10-second connection timeout and a 45-second total HTTP timeout.
Read waits are bounded to 30 seconds. JSON responses are bounded to 2 MiB, including
chunked bodies. Errors omit token values, gcloud output, and arbitrary remote
error bodies. The client preserves successful JSON exactly; it does not compress
SHAs, remove nulls, rewrite ack tokens, or turn missing evidence into successful
state.

## HTTP API

All routes below are appended to the configured endpoint's base path. Every
request carries `Authorization: Bearer <Google ID token>`. Authentication and
authorization must finish before a mutation is applied; a 401 must mean that no
state change occurred.

| Method and route | Request | Required receiver behavior |
| --- | --- | --- |
| `POST /v1/listeners` | `{ "owner": "ExampleOrg", "repo": "example", "events": ["approved", "merged", "ci_failed"], "idempotency_key": "stable-request-id" }` | Create a listener for the authenticated consumer and account namespace. Omitted `repo` requests organization scope and requires org-wide authorization. Scope idempotency to verified Google principal, case-folded account namespace, and key. Same key/body returns the same result; same key/different body returns 409. |
| `GET /v1/listeners` | No body | List only the authenticated consumer's listeners. |
| `DELETE /v1/listeners/{id}` | No body | Ensure the listener is absent in the caller's account namespace. A missing or another namespace's ID may return 204 without changing another consumer's listener. Repeated removal is safe; documented 204 or 404 response. |
| `GET /v1/listeners/{id}/events?limit=30&wait_seconds=0` | No body | Return at most 100 delivery records; wait at most 30 seconds. Reading does not acknowledge. Preserve each consumer's independent delivery state. |
| `POST /v1/listeners/{id}/ack` | `{ "ack_tokens": ["opaque-receiver-issued-token"] }` | Acknowledge only the specified deliveries for this listener and consumer. Validate the complete batch before advancing delivery state. Reject cross-listener, cross-consumer, forged, or expired tokens. Repeating a successful ack is safe. |

The success envelope for registration is `{ "listener": { "id": "...",
"owner": "...", "repo": null, "events": [...] } }`; listing is
`{ "listeners": [...] }`; ack is `{ "acked": 1 }`. Reads retain the full envelope:

```json
{
  "events": [],
  "more_available": false,
  "retention_seconds": 604800,
  "retention_cutoff": 1791633600,
  "expired_deliveries": 0,
  "retention_gap": false
}
```

The cutoff above is illustrative Unix time; use the values returned by the
receiver. `expired_deliveries` counts expired or missing deliveries encountered
during that read; `retention_gap` reports a missing stream interval. Preserve
both even for an empty batch. Neither an empty batch nor `more_available: false`
proves that no delivery was lost to retention.

The client preserves successful JSON verbatim, including unknown fields and
nulls, without imposing a closed response schema. A 204 response becomes JSON
`null`: it establishes absence in the caller's namespace, not that another
namespace's listener existed or was removed. Registering with a deleted or
expired listener's idempotency key can return 410; create a new key when starting
a new subscription. An existing listener does not replay events from before its
registration.

Listener IDs contain 1–128 ASCII letters, digits, underscores, or hyphens. The
registration idempotency key contains 1–128 ASCII letters/digits or `._:-`.
Allowed event kinds are `approved`, `merged`, and `ci_failed`, without duplicates.
Repo values contain a repository name, never `owner/repo`. Owner matching in the
local allowlist is case-insensitive. Ack batches contain 1–100 nonempty opaque
tokens, at most 4096 bytes each and 128 KiB total. The client does not parse those
tokens or infer their authority.

## Delivery meaning and consumer loop

A receiver-issued delivery should retain stable event provenance, the original
GitHub delivery identifier, event/action, installation/org/repo identifiers, PR
number, event head SHA, current head SHA when known, receipt time, original
conclusion/state, raw payload, and an opaque ack token bound to consumer,
listener, and delivery/lease. Any unknown field remains unknown or `null`. A retry
or redelivery attempt must not replace the underlying event's identity. The
delivery fields include `delivery_id`, `kind`, `head_sha`, `raw_state`,
`raw_conclusion`, and `ack_token`; preserve null values when the source supplies
no fact. A raw-payload reference is not evidence that the payload was fetched.

`approved` means an observed approving review event, not proof that all branch
protection requirements are met. Dismissed reviews, outdated heads, later commits,
and changing policies matter to any future PR-state aggregate. `merged` requires
a closed PR payload with `merged: true`. `ci_failed` must retain source and raw
conclusion so a check failure, workflow failure, legacy status error, cancellation,
and timeout are not silently treated as equivalent. Receiver normalization of
those cases still needs agreement. This implementation does not compute aggregate
PR approval or CI readiness.

A consumer registers once using a stable idempotency key, reads a bounded batch,
persists or processes each event, then explicitly acknowledges the corresponding
tokens. Every consumer has its own delivery position; acknowledging in one
consumer must not consume events for another. Delivery is at least once: consumers
must handle duplicates after crashes, expired leases, and uncertain ack responses.
The client does not auto-ack, silently advance a cursor, or deduplicate away raw
events. An unacknowledged delivery may appear again on an immediate read. Its ack
token can change when its lease expires; do not use the token as the event's
deduplication key. Successful repeated acknowledgement remains safe while the
token and retained event are valid. Retention and lease durations belong to the
receiver; consumers must inspect retention metadata and handle expired tokens
without silently declaring the queue complete.

## Local validation and remaining dependency

`src/events.rs` contains fake HTTP service tests for auth retry/idempotency,
expiry/cache reuse, identity drift, audience mismatch, concurrent refresh,
401/403/429/5xx behavior, redirect refusal, input and profile validation,
cross-listener ack rejection, response limits, and verbatim event preservation.
These tests prove the client's behavior against a fake receiver. They do not prove
receiver authorization, webhook capture, durable publication, deployment, or live
GitHub event delivery. The receiver owner must implement and validate those
properties before an operational endpoint/audience can be published to developers.

## Multiple GitHub identities and organizations

Every consumer request includes `X-KP-GitHub-Account`, the selected profile's
GitHub login used as an account namespace. Accept any syntactically valid account
label for an authorized Google principal; do not look it up in a linked-account
table or infer GitHub identity from it. Case-fold labels consistently. Pairing
one Google login with several GitHub accounts, or vice versa, needs no registration.
Each local account profile selects its own Google login and org set. Different
Google principals retain separate listener state; it is not transferred merely
because their GitHub labels match.

Profiles may list several `allowed_orgs`, including future onboarded orgs.
Registration names an explicit owner/repo. The receiver enforces developer access
policy and installed repository coverage, independently of local account sets.
Acceptance tests must include a permitted Google principal using two arbitrary
account labels without preregistration, denial of an unpermitted principal using
an existing label, denial outside policy/installation scope, isolation of reads
and acks across principals/namespaces, and grant/installation removal.
