# Linux CPU tuning and local state

The published server currently targets portable Linux x86_64. macOS work is
deferred. Shared HTTP mode reuses one process and its connection pools and
credential caches across MCP sessions; it does not add a GitHub response cache
or a local database.

## CPU evidence

`examples/response_cpu.rs` measures the actual JSON parsing, response compression
and formatting modules against fixed synthetic PR responses. Its event case
parses and serializes a batch without compressing or acknowledging events.
It makes no network calls and uses no credentials. Each measured result is
checked against the expected output outside the timed region.

```sh
CARGO_TARGET_DIR=/tmp/kp-cpu-baseline RUSTFLAGS='-C target-cpu=x86-64' \
  cargo build --release --locked --example response_cpu
CARGO_TARGET_DIR=/tmp/kp-cpu-v3 RUSTFLAGS='-C target-cpu=x86-64-v3' \
  cargo build --release --locked --example response_cpu
/tmp/kp-cpu-baseline/release/examples/response_cpu 2000
# Run only on a host supporting the complete x86-64-v3 feature set:
/tmp/kp-cpu-v3/release/examples/response_cpu 2000
```

On an i9-14900K under WSL2, Rust 1.99.0, six alternating-order trials per variant
with 100 warmups and 2,000 measured calls per case produced these medians of the
per-trial p50s. CPU affinity and frequency were not controlled; this is a bounded
local experiment, not an end-to-end MCP latency guarantee.

| Synthetic response | Portable p50 | v3 p50 | Portable / v3 |
| --- | ---: | ---: | ---: |
| 30 PRs | 293.5 µs | 292.3 µs | 1.004× |
| 100 PRs | 869.8 µs | 876.3 µs | 0.993× |
| 100 events | 181.2 µs | 145.0 µs | 1.249× |

Output fingerprints matched across variants. The event case saves about 36 µs;
PR processing shows no useful gain in this experiment. Network, credential and
queueing costs are excluded. These results do not justify requiring v3 for every
developer or claim a 25% faster GitHub tool call.

`x86-64-v3` includes AVX2, BMI1/BMI2, FMA, F16C, LZCNT, MOVBE and the preceding
ISA levels. FMA helps floating-point kernels; this server's transforms primarily
process strings, JSON objects and integer metadata. `x86-64-v4` adds a defined
AVX-512 subset, not every extension named AVX-512. The test host exposes no
AVX-512, so no v4 performance or execution claim is made.

Prefer measured, isolated kernels with runtime detection and a baseline fallback
before raising the whole program's minimum CPU. Dependencies may already dispatch
internally: the locked `memchr` version selects its AVX2 implementation at runtime.
Rust documents [per-function feature control and runtime detection](https://doc.rust-lang.org/core/arch/).
Global `target-cpu=native` is a build-host requirement, not portable dispatch.
Any future shipped CPU variant must record its CPU requirements and compiler
flags in provenance and pass correctness, native execution and reproduction
checks independently. The present provenance builder intentionally sanitizes
inherited build flags and still builds the portable server.

## When a local database helps

The current request/response tools do not require one. Shared pools, duplicate
in-flight read coalescing and a bounded response cache can work in memory. A
database becomes useful for features that must survive daemon restarts:

| Desired behavior | Candidate |
| --- | --- |
| Durable listener registrations, per-consumer progress, explicit ack retry state, keyed response records | SQLite transactions |
| Cross-repository event history, CI failure trends, time-window joins and large historical aggregations | DuckDB analytical tables or a projection of the event log |
| Shared connections and credentials | Existing in-memory state |

DuckDB supports concurrent work within one owning process, which fits a shared
MCP daemon, and favors bulk analytical work over many tiny transactions. See its
[concurrency](https://duckdb.org/docs/current/connect/concurrency) and
[workload tuning](https://duckdb.org/docs/stable/guides/performance/how_to_tune_workloads)
documentation. Engine choice must follow the queries and durability requirements;
adding two engines before those requirements exist adds deployment and recovery
work without an established benefit.

A future local store must partition by receiver, verified Google principal,
account namespace and consumer. GitHub response records also need the GitHub
host/account and request representation. Keep observed timestamps, head SHAs,
unknown values and retention gaps. Stored approvals describe observed reviews;
they do not establish current branch-policy approval. Reads still must not
acknowledge events automatically, and persisted retry state must never replay
unrelated GitHub mutations. Credentials remain in their existing local stores.

These database options are design candidates, not implemented capabilities.
