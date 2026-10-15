# Developer event consumers: proposed v1 protocol

**Status: client implemented and fake-service tested; receiver agreement, deployment,
identity linking, and end-to-end delivery remain pending.** This document proposes
the contract for Meshly's existing receiver owner. It does not describe a deployed
consumer service. The GitHub webhook ingress URL is not automatically a consumer
API endpoint.

Meshly's instance can serve Meshlyai, Meshly-Open-Source, and subsequently onboarded
organizations. An open source adopter supplies its own app and receiver. This
client does not register arbitrary customers with Meshly's receiver.

## Developer configuration and identity

The optional local profile is read from `KP_GITHUB_EVENTS_CONFIG`, or from
`~/.config/kp-github/events.json` when that variable is absent:

```json
{
  "version": 1,
  "endpoint": "https://events.example.com",
  "audience": "receiver-approved-google-oauth-client-id",
  "allowed_orgs": ["Meshlyai", "Meshly-Open-Source"],
  "github_account": "jw409",
  "google_account": "developer@example.com",
  "app_slug": "meshly-open-source-dev-events"
}
```

The example endpoint and audience are placeholders. The receiver operator must
publish the real values. The optional `google_account` pins a stored gcloud
account; supplying it is recommended. The existing GitHub credential resolver
uses the selected `github_account` for GitHub operations. The profile does not
prove that those GitHub and Google accounts belong to the same person.

An absent default profile leaves ordinary GitHub operations available. An
explicitly named missing profile, an invalid profile, or a profile symlink fails
clearly rather than silently selecting another identity. Profiles are limited
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
compatible merely by writing it into this profile. If the chosen ingress cannot
accept these human tokens, the receiver owner must implement a supported user
login/token-exchange flow before this client can be enabled. Distributing a shared
service account key is not the fallback.

The receiver must bind the authenticated Google subject to a verified GitHub
identity, through a separately implemented account-linking flow or an audited
operator mapping. It must check that identity's authorization to the installed
organization/repository and scope every listener and delivery to that consumer.
The local `allowed_orgs` field is only a convenience guard. It is not authorization,
and neither an email supplied by a caller nor `github_account` in a local file is
proof of identity. Existing listener IDs and ack tokens must not grant access to
a different authenticated consumer.

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
| `POST /v1/listeners` | `{ "owner": "Meshlyai", "repo": "example", "events": ["approved", "merged", "ci_failed"], "idempotency_key": "stable-request-id" }` | Create a listener for the authenticated consumer. Omitted `repo` requests organization scope and requires org-wide authorization. Scope idempotency to consumer and key. Same key/body returns the same result; same key/different body returns 409. |
| `GET /v1/listeners` | No body | List only the authenticated consumer's listeners. |
| `DELETE /v1/listeners/{id}` | No body | Remove only a listener owned by the consumer. Repeated removal is safe; documented 204 or 404 response. |
| `GET /v1/listeners/{id}/events?limit=30&wait_seconds=0` | No body | Return at most 100 delivery records; wait at most 30 seconds. Reading does not acknowledge. Preserve each consumer's independent delivery state. |
| `POST /v1/listeners/{id}/ack` | `{ "ack_tokens": ["opaque-receiver-issued-token"] }` | Acknowledge only the specified deliveries for this listener and consumer. Validate the complete batch before advancing delivery state. Reject cross-listener, cross-consumer, forged, or expired tokens. Repeating a successful ack is safe. |

The proposed success envelope for registration is `{ "listener": { "id": "...",
"owner": "...", "repo": null, "events": [...] } }`; listing is
`{ "listeners": [...] }`; reading is `{ "events": [...], "more_available": false }`;
ack is `{ "acked": 1 }`. The client currently preserves JSON without enforcing
those response schemas, allowing the receiver owner to finalize envelopes before
deployment. A 204 response becomes JSON `null`.

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
or redelivery attempt must not replace the underlying event's identity.

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
events. Retention periods, replay windows, lease duration, pagination of listener
lists, and recovery after retention expiry are receiver decisions still pending.

## Local validation and remaining dependency

`src/events.rs` contains fake HTTP service tests for auth retry/idempotency,
expiry/cache reuse, identity drift, audience mismatch, concurrent refresh,
401/403/429/5xx behavior, redirect refusal, input and profile validation,
cross-listener ack rejection, response limits, and verbatim event preservation.
These tests prove the client's behavior against a fake receiver. They do not prove
receiver authorization, webhook capture, durable publication, deployment, or live
GitHub event delivery. The receiver owner must implement and validate those
properties before an operational endpoint/audience can be published to developers.
