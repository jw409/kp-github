//! Live integration tests for kp-github-mcp.
//! These make real GitHub API calls - require GITHUB_TOKEN or gh auth.
//! Run with: cargo test --test live_integration -- --ignored

use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

struct McpClient {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
    next_id: u32,
}

impl McpClient {
    async fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kp-github-mcp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start server");

        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        let mut client = Self {
            child,
            stdin,
            reader,
            next_id: 1,
        };

        // Initialize
        let _ = client
            .call(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0.1"}
                }),
            )
            .await;

        // Send initialized notification (no response expected)
        client.send_notification("notifications/initialized").await;

        client
    }

    async fn send_notification(&mut self, method: &str) {
        let msg = json!({"jsonrpc": "2.0", "method": method});
        let line = format!("{}\n", serde_json::to_string(&msg).unwrap());
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let line = format!("{}\n", serde_json::to_string(&msg).unwrap());
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();

        let mut buf = String::new();
        self.reader.read_line(&mut buf).await.unwrap();
        serde_json::from_str(&buf).unwrap()
    }

    async fn tool_call(&mut self, name: &str, args: Value) -> Value {
        self.call("tools/call", json!({"name": name, "arguments": args}))
            .await
    }

    fn get_text(resp: &Value) -> &str {
        resp["result"]["content"][0]["text"].as_str().unwrap_or("")
    }

    fn is_error(resp: &Value) -> bool {
        resp.get("error").is_some()
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

// ALL tests are #[ignore] by default - run with `cargo test --test live_integration -- --ignored`

#[tokio::test]
#[ignore]
async fn test_tools_list() {
    let mut client = McpClient::new().await;
    let resp = client.call("tools/list", json!({})).await;
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert!(
        tools.len() >= 40,
        "Expected 40+ tools, got {}",
        tools.len()
    );

    // Verify key tools exist
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"github_issues_list"));
    assert!(names.contains(&"github_prs_create"));
    assert!(names.contains(&"github_files_get"));
    assert!(names.contains(&"github_code_search"));
    assert!(names.contains(&"github_actions_list_runs"));
}

#[tokio::test]
#[ignore]
async fn test_issues_list() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_issues_list",
            json!({
                "owner": "anthropics", "repo": "claude-code", "state": "open", "limit": 3
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp), "Error: {:?}", resp["error"]);
    let text = McpClient::get_text(&resp);
    assert!(!text.is_empty());
    // Should be compressed - no avatar_url, no node_id
    assert!(
        !text.contains("avatar_url"),
        "avatar_url should be stripped"
    );
    assert!(!text.contains("node_id"), "node_id should be stripped");
    // Should contain issue data
    assert!(text.contains("title"), "Should have title field");
    assert!(text.contains("state"), "Should have state field");
}

#[tokio::test]
#[ignore]
async fn test_issues_list_with_field_projection() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_issues_list",
            json!({
                "owner": "anthropics", "repo": "claude-code", "state": "open", "limit": 3,
                "fields": ["number", "title", "state"],
                "format": "table"
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
    let text = McpClient::get_text(&resp);
    // Table format should have | separators
    assert!(text.contains("|"), "Table format should have pipes");
    assert!(text.contains("number"), "Should have number header");
    assert!(text.contains("title"), "Should have title header");
    // Should NOT have unprojected fields
    assert!(!text.contains("\"body\""), "body should be projected out");
}

#[tokio::test]
#[ignore]
async fn test_issues_get() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_issues_get",
            json!({
                "owner": "anthropics", "repo": "claude-code", "number": 1
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
    let text = McpClient::get_text(&resp);
    assert!(text.contains("\"number\""), "Should have number");
}

#[tokio::test]
#[ignore]
async fn test_issues_search() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_issues_search",
            json!({
                "query": "repo:anthropics/claude-code is:open", "limit": 3
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
    let text = McpClient::get_text(&resp);
    assert!(!text.is_empty());
}

#[tokio::test]
#[ignore]
async fn test_prs_list() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_prs_list",
            json!({
                "owner": "anthropics", "repo": "claude-code", "state": "closed", "limit": 2
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

#[tokio::test]
#[ignore]
async fn test_commits_list() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_commits_list",
            json!({
                "owner": "jw409", "repo": "kinderpowers", "limit": 5
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
    let text = McpClient::get_text(&resp);
    assert!(text.contains("sha"), "Commits should have sha");
}

#[tokio::test]
#[ignore]
async fn test_repos_get() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_repos_get",
            json!({
                "owner": "jw409", "repo": "kinderpowers"
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
    let text = McpClient::get_text(&resp);
    assert!(text.contains("kinderpowers"));
}

#[tokio::test]
#[ignore]
async fn test_repos_search() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_repos_search",
            json!({
                "query": "claude-code language:typescript", "limit": 3
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

#[tokio::test]
#[ignore]
async fn test_branches_list() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_branches_list",
            json!({
                "owner": "jw409", "repo": "kinderpowers"
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

#[tokio::test]
#[ignore]
async fn test_releases_list() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_releases_list",
            json!({
                "owner": "anthropics", "repo": "claude-code", "limit": 3
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

#[tokio::test]
#[ignore]
async fn test_tags_list() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_tags_list",
            json!({
                "owner": "anthropics", "repo": "claude-code", "limit": 3
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

#[tokio::test]
#[ignore]
async fn test_code_search() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_code_search",
            json!({
                "query": "repo:jw409/kinderpowers sequential", "limit": 3
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

#[tokio::test]
#[ignore]
async fn test_user_me() {
    let mut client = McpClient::new().await;
    let resp = client.tool_call("github_user_me", json!({})).await;
    assert!(!McpClient::is_error(&resp));
    let text = McpClient::get_text(&resp);
    assert!(text.contains("login"));
}

#[tokio::test]
#[ignore]
async fn test_files_get() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_files_get",
            json!({
                "owner": "jw409", "repo": "kinderpowers", "path": "README.md"
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

#[tokio::test]
#[ignore]
async fn test_users_search() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_users_search",
            json!({
                "query": "jw409", "limit": 3
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

/// Per-endpoint compression floors, measured against `gh api` raw responses on
/// a public repo (cli/cli). Byte-based: the byte ratio tracks the token ratio
/// ~1:1 because the compression is structural (dropped fields), not
/// tokenizer-dependent — see `scripts/bench_tokens.py` for the true-token table.
/// Fails if a change regresses compression on any hot read path.
///
///   cargo test --test live_integration test_compression_floors -- --ignored --nocapture
#[tokio::test]
#[ignore]
async fn test_compression_floors() {
    let mut client = McpClient::new().await;
    // (label, tool, args, gh api path, min byte ratio)
    let cases: Vec<(&str, &str, Value, &str, f64)> = vec![
        (
            "issues_list(5)",
            "github_issues_list",
            json!({"owner": "cli", "repo": "cli", "state": "open", "limit": 5}),
            "/repos/cli/cli/issues?state=open&per_page=5",
            4.0,
        ),
        (
            "prs_list(5)",
            "github_prs_list",
            json!({"owner": "cli", "repo": "cli", "state": "open", "limit": 5}),
            "/repos/cli/cli/pulls?state=open&per_page=5",
            8.0,
        ),
        (
            "compare(default)",
            "github_repos_compare",
            json!({"owner": "cli", "repo": "cli", "base": "trunk~30", "head": "trunk"}),
            "/repos/cli/cli/compare/trunk~30...trunk",
            3.0,
        ),
    ];

    eprintln!("\n{:<18}{:>10}{:>10}{:>9}", "endpoint", "raw_B", "comp_B", "ratio");
    for (label, tool, args, path, floor) in cases {
        let resp = client.tool_call(tool, args).await;
        assert!(!McpClient::is_error(&resp), "{label} errored: {resp:?}");
        let comp = McpClient::get_text(&resp).len().max(1);
        let raw = tokio::process::Command::new("gh")
            .args(["api", path])
            .output()
            .await
            .unwrap()
            .stdout
            .len();
        let ratio = raw as f64 / comp as f64;
        eprintln!("{label:<18}{raw:>10}{comp:>10}{ratio:>8.1}x");
        assert!(ratio >= floor, "{label}: compression {ratio:.1}x < floor {floor:.1}x");
    }
}

/// The branch-tracking half: `github_branch_status` returns a compressed
/// merged/ahead/behind verdict (with a derived `merged_into_base`) instead of a
/// full compare's commit + file arrays. Its output is bounded to a handful of
/// scalars regardless of how far the branch has diverged — that boundedness is
/// the point, so the win over a raw compare grows with divergence.
#[tokio::test]
#[ignore]
async fn test_branch_status_is_compact_and_correct() {
    let mut client = McpClient::new().await;

    // Merged direction: trunk~30 is fully contained in trunk -> merged_into_base.
    let merged = client
        .tool_call(
            "github_branch_status",
            json!({"owner": "cli", "repo": "cli", "branch": "trunk~30", "base": "trunk", "format": "json"}),
        )
        .await;
    assert!(!McpClient::is_error(&merged), "branch_status errored: {merged:?}");
    let mtext = McpClient::get_text(&merged);
    let mv: Value = serde_json::from_str(mtext).expect("branch_status must be valid json");
    assert_eq!(mv["merged_into_base"], json!(true), "trunk~30 should be contained in trunk: {mv}");
    // Bounded output: a fixed set of scalars, never the commit/file arrays.
    assert!(mtext.len() < 600, "branch_status should be bounded, got {}B: {mtext}", mtext.len());

    // Ahead direction: trunk has 30 commits trunk~30 lacks -> not merged, and the
    // full compare balloons while branch_status stays flat (the real win).
    let ahead = client
        .tool_call(
            "github_branch_status",
            json!({"owner": "cli", "repo": "cli", "branch": "trunk", "base": "trunk~30", "format": "json"}),
        )
        .await;
    let av: Value = serde_json::from_str(McpClient::get_text(&ahead)).expect("valid json");
    assert_eq!(av["merged_into_base"], json!(false), "trunk is ahead of trunk~30: {av}");
    assert!(av["ahead_by"].as_u64().unwrap_or(0) >= 1, "expected commits ahead: {av}");
    // Ref names must survive compression — regression guard for the pipeline
    // eating "base"/"head" as ref-objects (hence base_ref/head_ref).
    assert_eq!(av["base_ref"], json!("trunk~30"), "base_ref dropped by compressor: {av}");
    assert_eq!(av["head_ref"], json!("trunk"), "head_ref dropped by compressor: {av}");

    let bs_len = McpClient::get_text(&ahead).len();
    let cmp_len = McpClient::get_text(
        &client
            .tool_call(
                "github_repos_compare",
                json!({"owner": "cli", "repo": "cli", "base": "trunk~30", "head": "trunk"}),
            )
            .await,
    )
    .len();
    eprintln!("branch_status={bs_len}B  full_compare={cmp_len}B  ratio={:.0}x", cmp_len as f64 / bs_len.max(1) as f64);
    assert!(bs_len * 20 < cmp_len, "branch_status not compact vs a diverged compare: {bs_len}B vs {cmp_len}B");
}

#[tokio::test]
#[ignore]
async fn test_field_projection_reduces_size() {
    let mut client = McpClient::new().await;

    // Full output
    let full = client
        .tool_call(
            "github_issues_list",
            json!({
                "owner": "anthropics", "repo": "claude-code", "state": "open", "limit": 5
            }),
        )
        .await;
    let full_len = McpClient::get_text(&full).len();

    // Projected output
    let proj = client
        .tool_call(
            "github_issues_list",
            json!({
                "owner": "anthropics", "repo": "claude-code", "state": "open", "limit": 5,
                "fields": ["number", "title", "state"]
            }),
        )
        .await;
    let proj_len = McpClient::get_text(&proj).len();

    assert!(
        proj_len < full_len / 2,
        "Projected should be <50% of full ({} vs {})",
        proj_len,
        full_len
    );
}

#[tokio::test]
#[ignore]
async fn test_actions_list_runs() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_actions_list_runs",
            json!({
                "owner": "anthropics", "repo": "claude-code", "limit": 3
            }),
        )
        .await;
    // May error if no actions access, but should not crash
    assert!(
        !McpClient::is_error(&resp) || resp["error"]["code"].as_i64().is_some(),
        "Should return structured response or structured error"
    );
}

#[tokio::test]
#[ignore]
async fn test_labels_list() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_labels_list",
            json!({
                "owner": "anthropics", "repo": "claude-code"
            }),
        )
        .await;
    assert!(!McpClient::is_error(&resp));
}

#[tokio::test]
#[ignore]
async fn test_repos_compare() {
    let mut client = McpClient::new().await;
    let resp = client
        .tool_call(
            "github_repos_compare",
            json!({
                "owner": "jw409", "repo": "kinderpowers", "base": "main~3", "head": "main"
            }),
        )
        .await;
    // May fail if repo has <3 commits, but should not crash the server
    let text = McpClient::get_text(&resp);
    let has_result = !text.is_empty() || resp.get("error").is_some();
    assert!(has_result, "Should return either content or structured error");
}

#[tokio::test]
#[ignore]
async fn test_invalid_tool_returns_error() {
    let mut client = McpClient::new().await;
    let resp = client.tool_call("nonexistent_tool", json!({})).await;
    assert!(
        McpClient::is_error(&resp),
        "Nonexistent tool should return error"
    );
}

#[tokio::test]
#[ignore]
async fn test_missing_required_params() {
    let mut client = McpClient::new().await;
    // github_issues_list requires owner and repo
    let resp = client.tool_call("github_issues_list", json!({})).await;
    // Should return an error or isError content, not crash
    let is_err = McpClient::is_error(&resp)
        || resp["result"]["isError"].as_bool().unwrap_or(false);
    assert!(is_err, "Missing required params should error: {:?}", resp);
}
