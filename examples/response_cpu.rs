//! Offline CPU experiment using the server's actual response transformations.
//! No credentials, network calls, database, or simulated network latency.
#![allow(dead_code)]
#[path = "../src/compress.rs"]
mod compress;
#[path = "../src/format.rs"]
mod format;
#[path = "../src/util.rs"]
mod util;

use chrono::{TimeZone, Utc};
use serde_json::{json, Value};
use std::{hint::black_box, time::Instant};

fn transform(input: &[u8], events: bool) -> String {
    let parsed: Value = serde_json::from_slice(black_box(input)).unwrap();
    if events {
        // github_events preserves its successful response without compression.
        serde_json::to_string(&parsed).unwrap()
    } else {
        let config = compress::CompressConfig {
            max_body: 500,
            time_cutoff_days: 30,
            strip_urls: true,
            fields: None,
            format: compress::OutputFormat::Auto,
            keep_fields: Vec::new(),
            max_decoded_content_bytes: None,
        };
        let now = Utc.timestamp_opt(1_792_281_600, 0).unwrap();
        format::format_output(&compress::compress(&parsed, &config, now), config.format)
    }
}

fn fingerprint(bytes: &[u8]) -> String {
    // A deterministic equality witness, not an artifact security checksum.
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

fn main() {
    let iterations = std::env::args().nth(1).map(|s| s.parse::<usize>().unwrap()).unwrap_or(1000);
    assert!((10..=100_000).contains(&iterations));
    for (name, rows, events) in [("prs_30", 30, false), ("prs_100", 100, false), ("events_100", 100, true)] {
        let records: Vec<Value> = (0..rows).map(|i| {
            if events {
                json!({"delivery_id": format!("synthetic-{i}"), "kind":"ci_failed",
                    "head_sha":"0123456789abcdef0123456789abcdef01234567",
                    "raw_state":"completed", "raw_conclusion":"failure",
                    "ack_token":format!("synthetic-opaque-token-{i}"), "current_head_sha":null,
                    "payload":{"name":"Synthetic test", "output":"line of synthetic output\n".repeat(40)}})
            } else {
                json!({"number":i+1, "title":format!("Synthetic pull request {i}"),
                    "state":"open", "draft":false, "body":"Synthetic description with UTF-8: café. ".repeat(40),
                    "created_at":"2026-10-01T12:00:00Z", "updated_at":"2026-10-17T12:00:00Z",
                    "user":{"login":"example-dev", "id":123, "avatar_url":"https://example.invalid/avatar"},
                    "head":{"ref":"feature", "sha":"0123456789abcdef0123456789abcdef01234567"},
                    "base":{"ref":"main", "sha":"abcdef0123456789abcdef0123456789abcdef01"},
                    "labels":[{"name":"test", "color":"000000"}], "assignees":[],
                    "url":"https://api.github.invalid/pulls/1", "node_id":"synthetic",
                    "html_url":"https://github.invalid/ExampleOrg/example/pull/1"})
            }
        }).collect();
        let value = if events {
            json!({"events":records, "more_available":false, "retention_seconds":604800,
                "retention_cutoff":1791676800u64, "expired_deliveries":0, "retention_gap":false})
        } else { json!(records) };
        let input = serde_json::to_vec(&value).unwrap();
        let expected = transform(&input, events);
        if events { assert_eq!(serde_json::from_str::<Value>(&expected).unwrap(), value); }
        for _ in 0..100 { black_box(transform(&input, events)); }
        let mut samples = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = Instant::now();
            let output = black_box(transform(&input, events));
            samples.push(start.elapsed().as_nanos() as u64);
            // Outside the timer; verify that an optimization preserves output.
            assert_eq!(output, expected);
        }
        samples.sort_unstable();
        println!("{}", json!({"case":name, "iterations":iterations,
            "input_bytes":input.len(), "output_bytes":expected.len(),
            "output_fnv64":fingerprint(expected.as_bytes()),
            "p50_us":samples[iterations/2] as f64/1000.0,
            "p95_us":samples[(iterations-1)*95/100] as f64/1000.0,
            "compiled_avx2":cfg!(target_feature="avx2"),
            "compiled_bmi2":cfg!(target_feature="bmi2"),
            "compiled_fma":cfg!(target_feature="fma")}));
    }
}
