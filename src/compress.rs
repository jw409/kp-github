use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

fn base64_decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::STANDARD.decode(s)
}

/// Configuration for compression pipeline
pub struct CompressConfig {
    pub max_body: usize,
    pub time_cutoff_days: i64,
    pub strip_urls: bool,
    pub fields: Option<Vec<String>>,
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    Auto,
    Json,
    Table,
    Text,
}

impl Default for CompressConfig {
    fn default() -> Self {
        Self {
            max_body: env_usize("KP_GITHUB_MAX_BODY", 500),
            time_cutoff_days: env_i64("KP_GITHUB_TIME_CUTOFF", 30),
            strip_urls: env_bool("KP_GITHUB_STRIP_URLS", true),
            fields: None,
            format: env_format("KP_GITHUB_FORMAT"),
        }
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    match std::env::var(key) {
        Ok(v) => match v.parse() {
            Ok(n) => n,
            Err(_) => {
                tracing::warn!("{key}={v:?} is not a valid usize, using default {default}");
                default
            }
        },
        Err(_) => default,
    }
}

fn env_i64(key: &str, default: i64) -> i64 {
    match std::env::var(key) {
        Ok(v) => match v.parse() {
            Ok(n) => n,
            Err(_) => {
                tracing::warn!("{key}={v:?} is not a valid i64, using default {default}");
                default
            }
        },
        Err(_) => default,
    }
}

fn env_bool(key: &str, default: bool) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| v == "true" || v == "1")
        .unwrap_or(default)
}

fn env_format(key: &str) -> OutputFormat {
    match std::env::var(key).ok().as_deref() {
        Some("json") => OutputFormat::Json,
        Some("table") => OutputFormat::Table,
        Some("text") => OutputFormat::Text,
        _ => OutputFormat::Auto,
    }
}

/// Run the full 5-stage compression pipeline on a value.
pub fn compress(value: &Value, config: &CompressConfig, now: DateTime<Utc>) -> Value {
    let mut v = value.clone();
    match &mut v {
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                compress_single(item, config, now);
            }
        }
        Value::Object(_) => {
            compress_single(&mut v, config, now);
        }
        _ => {}
    }
    v
}

fn compress_single(v: &mut Value, config: &CompressConfig, now: DateTime<Utc>) {
    if let Value::Object(map) = v {
        stage1_strip(map, config);
        stage2_flatten(map);
        if let Some(fields) = &config.fields {
            stage3_project(map, fields);
        }
        stage4_compact(map, config, now);
    }
}

// Stage 1: Remove pure token waste
fn stage1_strip(map: &mut Map<String, Value>, config: &CompressConfig) {
    let waste_keys: &[&str] = &[
        "avatar_url",
        "gravatar_id",
        "node_id",
        "events_url",
        "received_events_url",
        "followers_url",
        "following_url",
        "gists_url",
        "starred_url",
        "subscriptions_url",
        "organizations_url",
        "repos_url",
        "type",
        "site_admin",
        "permissions",
        "plan",
        "url",
        "reactions",
        "sub_issues_summary",
        "issue_dependencies_summary",
        "author_association",
        "locked",
        "performed_via_github_app",
        "active_lock_reason",
        "draft",
        "timeline_url",
        "state_reason",
        "_links",
        "verification",
        "tree",
        "git_url",
        "download_url",
    ];

    for key in waste_keys {
        map.remove(*key);
    }

    if config.strip_urls {
        let url_keys: Vec<String> = map
            .keys()
            .filter(|k| k.ends_with("_url") && *k != "html_url")
            .cloned()
            .collect();
        for key in url_keys {
            map.remove(&key);
        }
    }

    // Recurse into nested objects
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        if let Some(Value::Object(inner)) = map.get_mut(&key) {
            stage1_strip(inner, config);
        }
        if let Some(Value::Array(arr)) = map.get_mut(&key) {
            for item in arr.iter_mut() {
                if let Value::Object(inner) = item {
                    stage1_strip(inner, config);
                }
            }
        }
    }
}

// Stage 2: Collapse nested objects to scalars
fn stage2_flatten(map: &mut Map<String, Value>) {
    // user: { login: "alice", ... } -> user: "alice"
    flatten_to_field(map, "user", "login");
    flatten_to_field(map, "author", "login");
    flatten_to_field(map, "committer", "login");
    flatten_to_field(map, "creator", "login");
    flatten_to_field(map, "owner", "login");
    flatten_to_field(map, "merged_by", "login");
    flatten_to_field(map, "closed_by", "login");
    flatten_to_field(map, "requested_reviewer", "login");

    // milestone: { title: "v1.0", ... } -> milestone: "v1.0"
    flatten_to_field(map, "milestone", "title");

    // labels: [{ name: "bug", ... }] -> labels: ["bug"]
    flatten_array_to_field(map, "labels", "name");
    flatten_array_to_field(map, "assignees", "login");
    flatten_array_to_field(map, "requested_reviewers", "login");
    flatten_array_to_field(map, "requested_teams", "slug");

    // head/base: { ref: "branch", sha: "abc..." } -> head_ref, head_sha
    flatten_ref_object(map, "head");
    flatten_ref_object(map, "base");

    // commit: { message: "...", author: { name, date }, ... } -> message, commit_author, commit_date
    // Uses prefixed keys to avoid collision with top-level `author` (GitHub user object)
    if let Some(Value::Object(commit)) = map.remove("commit") {
        if let Some(msg) = commit.get("message") {
            map.insert("message".to_string(), msg.clone());
        }
        if let Some(Value::Object(author)) = commit.get("author") {
            if let Some(name) = author.get("name") {
                // Only set if no top-level author already (issue/PR have user objects)
                if !map.contains_key("author") {
                    map.insert("author".to_string(), name.clone());
                }
            }
            if let Some(date) = author.get("date") {
                map.insert("commit_date".to_string(), date.clone());
            }
        }
    }

    // file content: decode base64 content for files API responses
    if let Some(Value::String(encoding)) = map.remove("encoding") {
        if encoding == "base64" {
            if let Some(Value::String(content)) = map.get("content") {
                let cleaned: String = content.chars().filter(|c| !c.is_whitespace()).collect();
                match base64_decode(&cleaned) {
                    Ok(decoded) => match String::from_utf8(decoded) {
                        Ok(text) => {
                            map.insert("content".to_string(), Value::String(text));
                        }
                        Err(_) => {
                            // Binary file — restore encoding indicator
                            map.insert("encoding".to_string(), Value::String("binary".into()));
                        }
                    },
                    Err(_) => {
                        // Decode failed — restore encoding so caller knows content is base64
                        map.insert("encoding".to_string(), Value::String("base64".into()));
                        tracing::warn!("Failed to decode base64 content");
                    }
                }
            }
        }
    }

    // repo nested in head/base already handled by strip
    if let Some(Value::Object(repo)) = map.remove("repository") {
        if let Some(name) = repo.get("full_name") {
            map.insert("repo".to_string(), name.clone());
        }
    }
}

fn flatten_to_field(map: &mut Map<String, Value>, key: &str, field: &str) {
    if let Some(Value::Object(inner)) = map.get(key) {
        if let Some(val) = inner.get(field) {
            let v = val.clone();
            map.insert(key.to_string(), v);
        }
    }
}

fn flatten_array_to_field(map: &mut Map<String, Value>, key: &str, field: &str) {
    if let Some(Value::Array(arr)) = map.get(key) {
        let flattened: Vec<Value> = arr
            .iter()
            .filter_map(|item| {
                if let Value::Object(obj) = item {
                    obj.get(field).cloned()
                } else {
                    Some(item.clone())
                }
            })
            .collect();
        map.insert(key.to_string(), Value::Array(flattened));
    }
}

fn flatten_ref_object(map: &mut Map<String, Value>, key: &str) {
    if let Some(Value::Object(inner)) = map.remove(key) {
        if let Some(ref_val) = inner.get("ref") {
            map.insert(format!("{key}_ref"), ref_val.clone());
        }
        if let Some(sha) = inner.get("sha") {
            map.insert(format!("{key}_sha"), sha.clone());
        }
        // Also grab repo full_name if present
        if let Some(Value::Object(repo)) = inner.get("repo") {
            if let Some(name) = repo.get("full_name") {
                map.insert(format!("{key}_repo"), name.clone());
            }
        }
    }
}

// Stage 3: Project only requested fields
fn stage3_project(map: &mut Map<String, Value>, fields: &[String]) {
    let keep: std::collections::HashSet<&str> = fields.iter().map(|s| s.as_str()).collect();
    map.retain(|k, _| keep.contains(k.as_str()));
}

/// Truncate a string to at most `max` bytes, respecting UTF-8 char boundaries.
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

// Stage 4: Compact values
fn stage4_compact(map: &mut Map<String, Value>, config: &CompressConfig, now: DateTime<Utc>) {
    let keys: Vec<String> = map.keys().cloned().collect();
    for key in keys {
        let val = map.get(&key).unwrap().clone();
        match &val {
            Value::String(s) => {
                // Try timestamp compaction
                if key.ends_with("_at") || key == "date" {
                    if let Some(compact) = compact_timestamp(s, config.time_cutoff_days, now) {
                        map.insert(key.clone(), Value::String(compact));
                        continue;
                    }
                }
                // Body truncation — skip if caller explicitly requested this field
                if (key == "body" || key == "description") && s.len() > config.max_body {
                    let explicitly_requested = config.fields.as_ref().is_some_and(|f| f.iter().any(|fld| fld == &key));
                    if !explicitly_requested {
                        map.insert(key.clone(), Value::String(truncate_str(s, config.max_body)));
                        continue;
                    }
                }
                // URL compaction
                if key == "html_url" {
                    if let Some(compact) = compact_github_url(s) {
                        map.insert(key.clone(), Value::String(compact));
                        continue;
                    }
                }
                // SHA truncation
                if (key.ends_with("_sha") || key == "sha") && s.len() > 7 {
                    map.insert(key.clone(), Value::String(s[..7].to_string()));
                }
            }
            Value::Null => {
                map.remove(&key);
            }
            _ => {}
        }
    }
}

fn compact_timestamp(s: &str, cutoff_days: i64, now: DateTime<Utc>) -> Option<String> {
    let dt = s.parse::<DateTime<Utc>>().ok()?;
    let diff = now.signed_duration_since(dt);
    let days = diff.num_days();

    if days < 0 {
        return None; // future timestamps left as-is
    }
    if days == 0 {
        let hours = diff.num_hours();
        if hours == 0 {
            let mins = diff.num_minutes();
            return Some(format!("{mins}m ago"));
        }
        return Some(format!("{hours}h ago"));
    }
    if days <= cutoff_days {
        return Some(format!("{days}d ago"));
    }
    Some(dt.format("%Y-%m-%d").to_string())
}

fn compact_github_url(url: &str) -> Option<String> {
    let url = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = url.splitn(4, '/').collect();
    match parts.as_slice() {
        [owner, repo, "issues", num] => Some(format!("{owner}/{repo}#{num}")),
        [owner, repo, "pull", num] => Some(format!("{owner}/{repo}!{num}")),
        [owner, repo] => Some(format!("{owner}/{repo}")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        "2026-03-18T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn test_strip_removes_avatar_and_url_fields() {
        let input = json!({
            "id": 1,
            "title": "Bug",
            "avatar_url": "https://avatars.githubusercontent.com/u/123",
            "gravatar_id": "",
            "node_id": "MDU6SXNzdWUx",
            "events_url": "https://api.github.com/repos/o/r/issues/1/events",
            "html_url": "https://github.com/o/r/issues/1"
        });

        let config = CompressConfig::default();
        let result = compress(&input, &config, now());

        assert!(result.get("avatar_url").is_none());
        assert!(result.get("gravatar_id").is_none());
        assert!(result.get("node_id").is_none());
        assert!(result.get("events_url").is_none());
        assert!(result.get("html_url").is_some()); // preserved
    }

    #[test]
    fn test_flatten_user_to_login() {
        let input = json!({
            "title": "Bug",
            "user": {
                "login": "alice",
                "id": 123,
                "avatar_url": "https://..."
            }
        });

        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["user"], "alice");
    }

    #[test]
    fn test_flatten_labels() {
        let input = json!({
            "labels": [
                { "name": "bug", "color": "ff0000", "id": 1 },
                { "name": "urgent", "color": "ff0000", "id": 2 }
            ]
        });

        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["labels"], json!(["bug", "urgent"]));
    }

    #[test]
    fn test_compact_timestamps() {
        let input = json!({
            "created_at": "2026-03-18T10:00:00Z",
            "updated_at": "2026-03-15T12:00:00Z",
            "closed_at": "2025-01-01T00:00:00Z"
        });

        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["created_at"], "2h ago");
        assert_eq!(result["updated_at"], "3d ago");
        assert_eq!(result["closed_at"], "2025-01-01");
    }

    #[test]
    fn test_compact_github_urls() {
        let input = json!({
            "html_url": "https://github.com/owner/repo/issues/42"
        });

        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["html_url"], "owner/repo#42");
    }

    #[test]
    fn test_nulls_removed() {
        let input = json!({
            "title": "Bug",
            "body": null,
            "milestone": null
        });

        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert!(result.get("body").is_none());
        assert!(result.get("milestone").is_none());
        assert_eq!(result["title"], "Bug");
    }

    #[test]
    fn test_field_projection() {
        let input = json!({
            "title": "Bug",
            "state": "open",
            "user": { "login": "alice", "id": 123 },
            "labels": [{ "name": "bug" }],
            "body": "long text..."
        });

        let mut config = CompressConfig::default();
        config.fields = Some(vec!["title".into(), "state".into(), "user".into()]);
        let result = compress(&input, &config, now());
        assert_eq!(result["title"], "Bug");
        assert_eq!(result["state"], "open");
        assert_eq!(result["user"], "alice");
        assert!(result.get("labels").is_none());
        assert!(result.get("body").is_none());
    }

    #[test]
    fn test_body_truncation() {
        let long_body = "x".repeat(1000);
        let input = json!({ "body": long_body });

        let config = CompressConfig { max_body: 100, ..CompressConfig::default() };
        let result = compress(&input, &config, now());
        let body = result["body"].as_str().unwrap();
        assert!(body.len() <= 104); // 100 + "..."
        assert!(body.ends_with("..."));
    }

    #[test]
    fn test_body_not_truncated_when_explicitly_requested() {
        let long_body = "x".repeat(1000);
        let input = json!({ "body": long_body, "title": "test" });

        let config = CompressConfig {
            max_body: 100,
            fields: Some(vec!["body".to_string(), "title".to_string()]),
            ..CompressConfig::default()
        };
        let result = compress(&input, &config, now());
        let body = result["body"].as_str().unwrap();
        assert_eq!(body.len(), 1000, "body should not be truncated when explicitly requested via fields");
        assert!(!body.ends_with("..."));
    }

    #[test]
    fn test_description_not_truncated_when_explicitly_requested() {
        let long_desc = "y".repeat(800);
        let input = json!({ "description": long_desc, "name": "test" });

        let config = CompressConfig {
            max_body: 100,
            fields: Some(vec!["description".to_string(), "name".to_string()]),
            ..CompressConfig::default()
        };
        let result = compress(&input, &config, now());
        let desc = result["description"].as_str().unwrap();
        assert_eq!(desc.len(), 800);
    }

    #[test]
    fn test_body_still_truncated_without_fields() {
        // When no fields specified, truncation should still apply
        let long_body = "x".repeat(1000);
        let input = json!({ "body": long_body });

        let config = CompressConfig { max_body: 100, ..CompressConfig::default() };
        let result = compress(&input, &config, now());
        let body = result["body"].as_str().unwrap();
        assert!(body.ends_with("..."));
    }

    #[test]
    fn test_sha_truncation() {
        let input = json!({ "sha": "abc1234567890def" });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["sha"], "abc1234");
    }

    #[test]
    fn test_full_issue_compression() {
        let input = json!({
            "id": 42,
            "number": 42,
            "title": "Fix the login bug",
            "state": "open",
            "body": "This is a bug report",
            "user": {
                "login": "alice",
                "id": 123,
                "avatar_url": "https://avatars.githubusercontent.com/u/123",
                "gravatar_id": "",
                "node_id": "MDQ6VXNlcjEyMw==",
                "type": "User",
                "site_admin": false,
                "html_url": "https://github.com/alice",
                "followers_url": "https://api.github.com/users/alice/followers"
            },
            "labels": [
                { "id": 1, "name": "bug", "color": "d73a4a", "node_id": "xyz" },
                { "id": 2, "name": "priority", "color": "ff0000", "node_id": "abc" }
            ],
            "assignees": [
                { "login": "bob", "id": 456, "avatar_url": "https://..." }
            ],
            "milestone": { "title": "v2.0", "number": 3, "id": 99 },
            "html_url": "https://github.com/owner/repo/issues/42",
            "created_at": "2026-03-18T10:00:00Z",
            "updated_at": "2026-03-18T11:30:00Z",
            "closed_at": null,
            "comments": 5,
            "node_id": "MDU6SXNzdWU0Mg==",
            "events_url": "https://api.github.com/repos/owner/repo/issues/42/events",
            "comments_url": "https://api.github.com/repos/owner/repo/issues/42/comments",
            "labels_url": "https://api.github.com/repos/owner/repo/issues/{/name}"
        });

        let config = CompressConfig::default();
        let result = compress(&input, &config, now());

        // Verify compression happened
        assert_eq!(result["user"], "alice");
        assert_eq!(result["labels"], json!(["bug", "priority"]));
        assert_eq!(result["assignees"], json!(["bob"]));
        assert_eq!(result["milestone"], "v2.0");
        assert_eq!(result["html_url"], "owner/repo#42");
        assert_eq!(result["created_at"], "2h ago");
        assert!(result.get("avatar_url").is_none());
        assert!(result.get("node_id").is_none());
        assert!(result.get("events_url").is_none());
        assert!(result.get("closed_at").is_none()); // null removed

        // Count remaining keys - should be much smaller
        let keys: Vec<&String> = result.as_object().unwrap().keys().collect();
        assert!(keys.len() <= 15, "Expected <= 15 keys, got {}: {:?}", keys.len(), keys);
    }

    #[test]
    fn test_flatten_assignees() {
        let input = json!({
            "assignees": [
                {"login": "a", "id": 1},
                {"login": "b", "id": 2}
            ]
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["assignees"], json!(["a", "b"]));
    }

    #[test]
    fn test_flatten_milestone() {
        let input = json!({
            "milestone": {"title": "v1.0", "number": 1, "id": 99}
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["milestone"], "v1.0");
    }

    #[test]
    fn test_flatten_head_ref() {
        let input = json!({
            "head": {
                "ref": "main",
                "sha": "abc1234567890def",
                "repo": {"full_name": "owner/repo"}
            }
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["head_ref"], "main");
        assert_eq!(result["head_sha"], "abc1234"); // truncated to 7
        assert!(result.get("head").is_none()); // original removed
    }

    #[test]
    fn test_strip_new_waste_keys() {
        let input = json!({
            "title": "Keep me",
            "url": "https://api.github.com/repos/o/r/issues/1",
            "reactions": {"+1": 5, "-1": 0},
            "locked": false,
            "author_association": "CONTRIBUTOR",
            "sub_issues_summary": {"total": 0}
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["title"], "Keep me");
        assert!(result.get("url").is_none());
        assert!(result.get("reactions").is_none());
        assert!(result.get("locked").is_none());
        assert!(result.get("author_association").is_none());
        assert!(result.get("sub_issues_summary").is_none());
    }

    #[test]
    fn test_compress_array() {
        let input = json!([
            {"title": "A", "node_id": "x", "user": {"login": "alice", "id": 1}},
            {"title": "B", "node_id": "y", "user": {"login": "bob", "id": 2}}
        ]);
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["user"], "alice");
        assert_eq!(arr[1]["user"], "bob");
        assert!(arr[0].get("node_id").is_none());
        assert!(arr[1].get("node_id").is_none());
    }

    #[test]
    fn test_compact_pr_url() {
        let input = json!({
            "html_url": "https://github.com/owner/repo/pull/42"
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["html_url"], "owner/repo!42");
    }

    #[test]
    fn test_compact_minutes_ago() {
        let input = json!({
            "created_at": "2026-03-18T11:45:00Z"
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["created_at"], "15m ago");
    }

    #[test]
    fn test_compact_description_truncation() {
        let long_desc = "d".repeat(1000);
        let input = json!({"description": long_desc});
        let config = CompressConfig { max_body: 100, ..CompressConfig::default() };
        let result = compress(&input, &config, now());
        let desc = result["description"].as_str().unwrap();
        assert!(desc.len() <= 104);
        assert!(desc.ends_with("..."));
    }

    #[test]
    fn test_strip_urls_disabled() {
        let input = json!({
            "title": "test",
            "comments_url": "https://api.github.com/repos/o/r/issues/1/comments",
            "labels_url": "https://api.github.com/repos/o/r/issues/{/name}",
            "html_url": "https://github.com/o/r/issues/1"
        });
        let config = CompressConfig {
            strip_urls: false,
            ..CompressConfig::default()
        };
        let result = compress(&input, &config, now());
        // *_url fields preserved when strip_urls=false
        assert!(result.get("comments_url").is_some());
        assert!(result.get("labels_url").is_some());
        assert!(result.get("html_url").is_some());
    }

    #[test]
    fn test_empty_object() {
        let input = json!({});
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result, json!({}));
    }

    #[test]
    fn test_empty_array() {
        let input = json!([]);
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result, json!([]));
    }

    #[test]
    fn test_body_truncation_multibyte() {
        // Emoji (4 bytes each) and CJK (3 bytes each) — truncating mid-char would panic
        let body = "Hello \u{1F600}\u{1F600}\u{1F600} \u{4F60}\u{597D}\u{4E16}\u{754C} end".to_string();
        let input = json!({ "body": body });
        // max_body=10 lands inside the first emoji (byte 6 is start, bytes 6-9 are the emoji)
        let config = CompressConfig { max_body: 8, ..CompressConfig::default() };
        let result = compress(&input, &config, now());
        let truncated = result["body"].as_str().unwrap();
        assert!(truncated.ends_with("..."));
        // Must not panic and must be valid UTF-8
        assert!(truncated.len() <= 12); // at most 8 + "..."

        // Also test description field with CJK
        let desc = "\u{4F60}\u{597D}\u{4E16}\u{754C}\u{4F60}\u{597D}".to_string(); // 18 bytes of CJK
        let input2 = json!({ "description": desc });
        let config2 = CompressConfig { max_body: 7, ..CompressConfig::default() };
        let result2 = compress(&input2, &config2, now());
        let truncated2 = result2["description"].as_str().unwrap();
        assert!(truncated2.ends_with("..."));
    }

    #[test]
    fn test_truncate_str_helper() {
        // Short string — no truncation
        assert_eq!(truncate_str("hello", 10), "hello");
        // ASCII truncation
        assert_eq!(truncate_str("hello world", 5), "hello...");
        // Multi-byte: "é" is 2 bytes, cutting at byte 1 should back up
        assert_eq!(truncate_str("é", 1), "...");
        // Multi-byte: 3-byte char
        assert_eq!(truncate_str("\u{4F60}\u{597D}", 4), "\u{4F60}...");
    }

    #[test]
    fn test_compact_github_url_repo_only() {
        assert_eq!(compact_github_url("https://github.com/owner/repo"), Some("owner/repo".into()));
    }

    #[test]
    fn test_compact_github_url_not_github() {
        assert_eq!(compact_github_url("https://gitlab.com/owner/repo"), None);
    }

    #[test]
    fn test_compact_github_url_unknown_path() {
        // e.g. /settings or /wiki — falls through
        assert_eq!(compact_github_url("https://github.com/owner/repo/settings/hooks"), None);
    }

    #[test]
    fn test_compact_timestamp_future() {
        // Future timestamp returns None (left as-is)
        let future = "2026-03-19T12:00:00Z";
        assert!(compact_timestamp(future, 30, now()).is_none());
    }

    #[test]
    fn test_compact_timestamp_invalid() {
        assert!(compact_timestamp("not-a-date", 30, now()).is_none());
    }

    #[test]
    fn test_flatten_creator() {
        let input = json!({
            "creator": {"login": "alice", "id": 1}
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["creator"], "alice");
    }

    #[test]
    fn test_flatten_merged_by() {
        let input = json!({
            "merged_by": {"login": "bob", "id": 2}
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["merged_by"], "bob");
    }

    #[test]
    fn test_flatten_closed_by() {
        let input = json!({
            "closed_by": {"login": "charlie", "id": 3}
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["closed_by"], "charlie");
    }

    #[test]
    fn test_flatten_requested_reviewer() {
        let input = json!({
            "requested_reviewer": {"login": "reviewer1", "id": 4}
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["requested_reviewer"], "reviewer1");
    }

    #[test]
    fn test_flatten_requested_reviewers() {
        let input = json!({
            "requested_reviewers": [
                {"login": "r1", "id": 1},
                {"login": "r2", "id": 2}
            ]
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["requested_reviewers"], json!(["r1", "r2"]));
    }

    #[test]
    fn test_flatten_requested_teams() {
        let input = json!({
            "requested_teams": [
                {"slug": "core-team", "id": 1},
                {"slug": "docs-team", "id": 2}
            ]
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["requested_teams"], json!(["core-team", "docs-team"]));
    }

    #[test]
    fn test_flatten_owner() {
        let input = json!({
            "owner": {"login": "org-owner", "id": 1}
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["owner"], "org-owner");
    }

    #[test]
    fn test_flatten_repository() {
        let input = json!({
            "repository": {"full_name": "org/repo", "id": 1}
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["repo"], "org/repo");
    }

    #[test]
    fn test_flatten_base_ref() {
        let input = json!({
            "base": {
                "ref": "main",
                "sha": "def7890123456789",
                "repo": {"full_name": "org/upstream"}
            }
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["base_ref"], "main");
        assert_eq!(result["base_sha"], "def7890"); // truncated
        assert_eq!(result["base_repo"], "org/upstream");
    }

    #[test]
    fn test_compress_scalar_value() {
        // Non-object, non-array value should pass through
        let input = json!("just a string");
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result, json!("just a string"));
    }

    #[test]
    fn test_compress_number_value() {
        let input = json!(42);
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result, json!(42));
    }

    #[test]
    fn test_sha_short_not_truncated() {
        let input = json!({"sha": "abc123"});
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["sha"], "abc123"); // 6 chars, not truncated
    }

    #[test]
    fn test_sha_exactly_7_not_truncated() {
        let input = json!({"sha": "abc1234"});
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["sha"], "abc1234"); // exactly 7, not truncated
    }

    #[test]
    fn test_head_sha_truncated() {
        let input = json!({"head_sha": "abc1234567890"});
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["head_sha"], "abc1234");
    }

    #[test]
    fn test_date_field_timestamp() {
        let input = json!({"date": "2026-03-17T12:00:00Z"});
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        assert_eq!(result["date"], "1d ago");
    }

    #[test]
    fn test_nested_strip_in_array() {
        // Objects in arrays within objects should also be stripped
        let input = json!({
            "items": [
                {"login": "alice", "avatar_url": "http://...", "node_id": "x"},
                {"login": "bob", "avatar_url": "http://...", "node_id": "y"}
            ]
        });
        let config = CompressConfig::default();
        let result = compress(&input, &config, now());
        let items = result["items"].as_array().unwrap();
        assert!(items[0].get("avatar_url").is_none());
        assert!(items[0].get("node_id").is_none());
        assert_eq!(items[0]["login"], "alice");
    }

    #[test]
    fn test_strip_all_waste_keys() {
        // Exhaustive test of all waste keys
        let input = json!({
            "title": "keep",
            "avatar_url": "x",
            "gravatar_id": "x",
            "node_id": "x",
            "events_url": "x",
            "received_events_url": "x",
            "followers_url": "x",
            "following_url": "x",
            "gists_url": "x",
            "starred_url": "x",
            "subscriptions_url": "x",
            "organizations_url": "x",
            "repos_url": "x",
            "type": "x",
            "site_admin": false,
            "permissions": {},
            "plan": {},
            "url": "x",
            "reactions": {},
            "sub_issues_summary": {},
            "issue_dependencies_summary": {},
            "author_association": "x",
            "locked": false,
            "performed_via_github_app": null,
            "active_lock_reason": null,
            "draft": false,
            "timeline_url": "x",
            "state_reason": "x"
        });
        let config = CompressConfig { strip_urls: false, ..CompressConfig::default() };
        let result = compress(&input, &config, now());
        // Only "title" should remain (plus possibly performed_via_github_app and active_lock_reason
        // which are null but also in waste_keys so stripped before null removal)
        assert_eq!(result["title"], "keep");
        let keys: Vec<&String> = result.as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["title"]);
    }

    #[test]
    fn test_body_at_limit_no_truncation() {
        let body = "x".repeat(500); // exactly at default max_body
        let input = json!({"body": body});
        let config = CompressConfig::default(); // max_body=500
        let result = compress(&input, &config, now());
        assert_eq!(result["body"].as_str().unwrap().len(), 500);
    }

    #[test]
    fn test_env_format_variants() {
        // These call the env helpers directly
        assert_eq!(env_format("KP_GITHUB_NONEXISTENT_FORMAT_KEY_12345"), OutputFormat::Auto);
    }

    // ---- property-based tests (proptest) ----

    use proptest::prelude::*;

    // 1. Compression never panics on arbitrary JSON
    proptest! {
        #[test]
        fn compress_never_panics(json_str in "\\PC{0,500}") {
            if let Ok(val) = serde_json::from_str::<Value>(&json_str) {
                let config = CompressConfig::default();
                let _ = compress(&val, &config, now());
            }
        }
    }

    // 2. Field projection never adds keys
    proptest! {
        #[test]
        fn projection_only_removes_keys(
            keys in prop::collection::vec("[a-z]{1,5}", 1..10),
            project in prop::collection::vec("[a-z]{1,5}", 0..5),
        ) {
            let mut map = serde_json::Map::new();
            for k in &keys {
                map.insert(k.clone(), Value::String("v".into()));
            }
            let input = Value::Object(map);
            let mut config = CompressConfig::default();
            config.fields = Some(project.clone());
            let result = compress(&input, &config, now());
            if let Value::Object(result_map) = result {
                for key in result_map.keys() {
                    assert!(project.contains(key), "Projected output has unexpected key: {key}");
                }
            }
        }
    }

    // 3. URL encoding roundtrips (no panics on arbitrary strings)
    proptest! {
        #[test]
        fn urlencode_never_panics(s in "\\PC{0,200}") {
            let _ = crate::util::urlencode(&s);
        }
    }

    // 4. Body truncation never panics on arbitrary UTF-8
    proptest! {
        #[test]
        fn truncation_never_panics(body in "\\PC{0,1000}", max in 0usize..500) {
            let input = json!({"body": body});
            let config = CompressConfig { max_body: max, ..CompressConfig::default() };
            let result = compress(&input, &config, now());
            // Should never panic, result should be valid
            if let Some(Value::String(s)) = result.get("body") {
                assert!(s.is_char_boundary(s.len())); // valid UTF-8
            }
        }
    }

    // 5. Timestamp compaction never panics
    proptest! {
        #[test]
        fn compact_timestamp_never_panics(s in "\\PC{0,50}") {
            let _ = compact_timestamp(&s, 30, now());
        }
    }
}
