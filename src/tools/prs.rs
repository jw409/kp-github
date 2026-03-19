use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// List pull requests for a repository.
pub async fn list(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    state: Option<&str>,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls");
    let mut params: Vec<(&str, &str)> = Vec::new();
    if let Some(s) = state {
        params.push(("state", s));
    }
    client.api_list(&endpoint, &params, limit).await
}

/// Get a single pull request by number.
pub async fn get(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}");
    client.api(&endpoint, &[]).await
}

/// Get the diff for a pull request via the GitHub API.
pub async fn diff(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<String, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}");
    client.api_raw(&endpoint, "application/vnd.github.v3.diff").await
}

/// List files changed in a pull request.
pub async fn files(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/files");
    client.api(&endpoint, &[]).await
}

/// Create a new pull request.
pub async fn create(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    title: &str,
    head: &str,
    base: &str,
    body: Option<&str>,
    draft: Option<bool>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls");
    let title_field = format!("title={title}");
    let head_field = format!("head={head}");
    let base_field = format!("base={base}");
    let mut args = vec![
        "-X", "POST",
        "-f", &title_field as &str,
        "-f", &head_field as &str,
        "-f", &base_field as &str,
    ];
    let body_field;
    if let Some(b) = body {
        body_field = format!("body={b}");
        args.push("-f");
        args.push(&body_field);
    }
    let draft_field;
    if let Some(d) = draft {
        draft_field = format!("draft={d}");
        args.push("-f");
        args.push(&draft_field);
    }
    client.api(&endpoint, &args).await
}

/// Update a pull request.
pub async fn update(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    title: Option<&str>,
    body: Option<&str>,
    state: Option<&str>,
    base: Option<&str>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}");
    let mut args: Vec<&str> = vec!["-X", "PATCH"];
    let title_field;
    if let Some(t) = title {
        title_field = format!("title={t}");
        args.push("-f");
        args.push(&title_field);
    }
    let body_field;
    if let Some(b) = body {
        body_field = format!("body={b}");
        args.push("-f");
        args.push(&body_field);
    }
    let state_field;
    if let Some(s) = state {
        state_field = format!("state={s}");
        args.push("-f");
        args.push(&state_field);
    }
    let base_field;
    if let Some(b) = base {
        base_field = format!("base={b}");
        args.push("-f");
        args.push(&base_field);
    }
    client.api(&endpoint, &args).await
}

/// Merge a pull request.
pub async fn merge(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    merge_method: Option<&str>,
    commit_title: Option<&str>,
    commit_message: Option<&str>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/merge");
    let mut args: Vec<&str> = vec!["-X", "PUT"];
    let method_field;
    if let Some(m) = merge_method {
        method_field = format!("merge_method={m}");
        args.push("-f");
        args.push(&method_field);
    }
    let title_field;
    if let Some(t) = commit_title {
        title_field = format!("commit_title={t}");
        args.push("-f");
        args.push(&title_field);
    }
    let msg_field;
    if let Some(m) = commit_message {
        msg_field = format!("commit_message={m}");
        args.push("-f");
        args.push(&msg_field);
    }
    client.api(&endpoint, &args).await
}

/// Get reviews on a pull request.
pub async fn reviews(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/reviews");
    client.api(&endpoint, &[]).await
}

/// Get review comments (threaded) on a pull request.
pub async fn review_comments(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/comments");
    client.api(&endpoint, &[]).await
}

/// Get comments on a pull request (non-review, via issues API).
pub async fn comments(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues/{number}/comments");
    client.api(&endpoint, &[]).await
}

/// Get check runs for a PR's head commit.
pub async fn check_runs(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    // First fetch the PR to get head SHA
    let pr = get(client, owner, repo, number).await?;
    let sha = pr["head"]["sha"]
        .as_str()
        .ok_or_else(|| ClientError::Api("missing head.sha in PR response".to_string()))?;
    let endpoint = format!("/repos/{owner}/{repo}/commits/{sha}/check-runs");
    let result = client.api(&endpoint, &[]).await?;
    Ok(result["check_runs"].clone())
}

/// Get combined status for a PR's head commit.
pub async fn status(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    // First fetch the PR to get head SHA
    let pr = get(client, owner, repo, number).await?;
    let sha = pr["head"]["sha"]
        .as_str()
        .ok_or_else(|| ClientError::Api("missing head.sha in PR response".to_string()))?;
    let endpoint = format!("/repos/{owner}/{repo}/commits/{sha}/status");
    client.api(&endpoint, &[]).await
}

/// Create a review on a pull request.
pub async fn create_review(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    event: &str,
    body: Option<&str>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/reviews");
    let event_field = format!("event={event}");
    let mut args: Vec<&str> = vec!["-X", "POST", "-f", &event_field];
    let body_field;
    if let Some(b) = body {
        body_field = format!("body={b}");
        args.push("-f");
        args.push(&body_field);
    }
    client.api(&endpoint, &args).await
}

/// Search pull requests using GitHub search syntax.
pub async fn search(
    client: &GithubClient,
    query: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let full_query = format!("{query} type:pr");
    let per_page = limit.unwrap_or(30).min(100);
    let url = format!("/search/issues?q={}&per_page={per_page}", crate::util::urlencode(&full_query));
    let result = client.api(&url, &[]).await?;

    // Extract .items from search response
    if let Value::Object(ref map) = result {
        if let Some(items) = map.get("items") {
            return Ok(items.clone());
        }
    }
    Ok(result)
}

/// Add a single review comment to a pull request.
pub async fn add_review_comment(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    path: &str,
    body: &str,
    line: Option<u32>,
    side: Option<&str>,
    subject_type: Option<&str>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/comments");
    let mut args: Vec<String> = vec![
        "-X".into(), "POST".into(),
        "-f".into(), format!("path={path}"),
        "-f".into(), format!("body={body}"),
    ];
    if let Some(l) = line {
        args.push("-f".into());
        args.push(format!("line={l}"));
    }
    if let Some(s) = side {
        args.push("-f".into());
        args.push(format!("side={s}"));
    }
    if let Some(st) = subject_type {
        args.push("-f".into());
        args.push(format!("subject_type={st}"));
    }
    // We need to include commit_id — fetch it from the PR
    let pr = get(client, owner, repo, number).await?;
    let commit_id = pr["head"]["sha"]
        .as_str()
        .ok_or_else(|| ClientError::Api("missing head.sha in PR response".to_string()))?;
    args.push("-f".into());
    args.push(format!("commit_id={commit_id}"));

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    client.api(&endpoint, &arg_refs).await
}

/// Reply to an existing pull request review comment.
pub async fn reply_to_comment(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    comment_id: u64,
    body: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies");
    let body_field = format!("body={body}");
    client.api(&endpoint, &["-X", "POST", "-f", &body_field]).await
}

/// Submit a pending review on a pull request.
pub async fn submit_review(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    review_id: u64,
    event: &str,
    body: Option<&str>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/reviews/{review_id}/events");
    let event_field = format!("event={event}");
    let mut args: Vec<&str> = vec!["-X", "POST", "-f", &event_field];
    let body_field;
    if let Some(b) = body {
        body_field = format!("body={b}");
        args.push("-f");
        args.push(&body_field);
    }
    client.api(&endpoint, &args).await
}

/// Delete a pending review on a pull request.
pub async fn delete_review(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    review_id: u64,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/reviews/{review_id}");
    client.api(&endpoint, &["-X", "DELETE"]).await
}

/// Update PR branch with latest from base.
pub async fn update_branch(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/pulls/{number}/update-branch");
    client.api(&endpoint, &["-X", "PUT"]).await
}

// --- Testable helpers ---

fn list_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/pulls")
}

fn get_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}")
}

fn diff_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}")
}

fn files_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}/files")
}

fn reviews_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}/reviews")
}

fn review_comments_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}/comments")
}

fn comments_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/issues/{number}/comments")
}

fn check_runs_endpoint(owner: &str, repo: &str, sha: &str) -> String {
    format!("/repos/{owner}/{repo}/commits/{sha}/check-runs")
}

fn status_endpoint(owner: &str, repo: &str, sha: &str) -> String {
    format!("/repos/{owner}/{repo}/commits/{sha}/status")
}

fn merge_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}/merge")
}

fn update_branch_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}/update-branch")
}

fn search_url(query: &str, per_page: u32) -> String {
    let full_query = format!("{query} type:pr");
    format!("/search/issues?q={}&per_page={per_page}", crate::util::urlencode(&full_query))
}

fn reply_endpoint(owner: &str, repo: &str, number: u32, comment_id: u64) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}/comments/{comment_id}/replies")
}

fn submit_review_endpoint(owner: &str, repo: &str, number: u32, review_id: u64) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}/reviews/{review_id}/events")
}

fn delete_review_endpoint(owner: &str, repo: &str, number: u32, review_id: u64) -> String {
    format!("/repos/{owner}/{repo}/pulls/{number}/reviews/{review_id}")
}

fn create_args(title: &str, head: &str, base: &str, body: Option<&str>, draft: Option<bool>) -> Vec<String> {
    let mut args = vec![
        "-X".into(), "POST".into(),
        "-f".into(), format!("title={title}"),
        "-f".into(), format!("head={head}"),
        "-f".into(), format!("base={base}"),
    ];
    if let Some(b) = body {
        args.push("-f".into());
        args.push(format!("body={b}"));
    }
    if let Some(d) = draft {
        args.push("-f".into());
        args.push(format!("draft={d}"));
    }
    args
}

fn update_args(title: Option<&str>, body: Option<&str>, state: Option<&str>, base: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-X".into(), "PATCH".into()];
    if let Some(t) = title {
        args.push("-f".into());
        args.push(format!("title={t}"));
    }
    if let Some(b) = body {
        args.push("-f".into());
        args.push(format!("body={b}"));
    }
    if let Some(s) = state {
        args.push("-f".into());
        args.push(format!("state={s}"));
    }
    if let Some(b) = base {
        args.push("-f".into());
        args.push(format!("base={b}"));
    }
    args
}

fn merge_args(merge_method: Option<&str>, commit_title: Option<&str>, commit_message: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-X".into(), "PUT".into()];
    if let Some(m) = merge_method {
        args.push("-f".into());
        args.push(format!("merge_method={m}"));
    }
    if let Some(t) = commit_title {
        args.push("-f".into());
        args.push(format!("commit_title={t}"));
    }
    if let Some(m) = commit_message {
        args.push("-f".into());
        args.push(format!("commit_message={m}"));
    }
    args
}

fn create_review_args(event: &str, body: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-X".into(), "POST".into(),
        "-f".into(), format!("event={event}"),
    ];
    if let Some(b) = body {
        args.push("-f".into());
        args.push(format!("body={b}"));
    }
    args
}

fn add_review_comment_args(path: &str, body: &str, line: Option<u32>, side: Option<&str>, subject_type: Option<&str>, commit_id: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-X".into(), "POST".into(),
        "-f".into(), format!("path={path}"),
        "-f".into(), format!("body={body}"),
    ];
    if let Some(l) = line {
        args.push("-f".into());
        args.push(format!("line={l}"));
    }
    if let Some(s) = side {
        args.push("-f".into());
        args.push(format!("side={s}"));
    }
    if let Some(st) = subject_type {
        args.push("-f".into());
        args.push(format!("subject_type={st}"));
    }
    args.push("-f".into());
    args.push(format!("commit_id={commit_id}"));
    args
}

fn extract_search_items(result: &Value) -> Option<Value> {
    if let Value::Object(ref map) = result {
        if let Some(items) = map.get("items") {
            return Some(items.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_list_endpoint() {
        assert_eq!(list_endpoint("o", "r"), "/repos/o/r/pulls");
    }

    #[test]
    fn test_get_endpoint() {
        assert_eq!(get_endpoint("o", "r", 42), "/repos/o/r/pulls/42");
    }

    #[test]
    fn test_diff_endpoint() {
        assert_eq!(diff_endpoint("o", "r", 1), "/repos/o/r/pulls/1");
    }

    #[test]
    fn test_files_endpoint() {
        assert_eq!(files_endpoint("o", "r", 5), "/repos/o/r/pulls/5/files");
    }

    #[test]
    fn test_reviews_endpoint() {
        assert_eq!(reviews_endpoint("o", "r", 10), "/repos/o/r/pulls/10/reviews");
    }

    #[test]
    fn test_review_comments_endpoint() {
        assert_eq!(review_comments_endpoint("o", "r", 3), "/repos/o/r/pulls/3/comments");
    }

    #[test]
    fn test_comments_endpoint() {
        assert_eq!(comments_endpoint("o", "r", 7), "/repos/o/r/issues/7/comments");
    }

    #[test]
    fn test_check_runs_endpoint() {
        assert_eq!(check_runs_endpoint("o", "r", "abc123"), "/repos/o/r/commits/abc123/check-runs");
    }

    #[test]
    fn test_status_endpoint() {
        assert_eq!(status_endpoint("o", "r", "def456"), "/repos/o/r/commits/def456/status");
    }

    #[test]
    fn test_merge_endpoint() {
        assert_eq!(merge_endpoint("o", "r", 99), "/repos/o/r/pulls/99/merge");
    }

    #[test]
    fn test_update_branch_endpoint() {
        assert_eq!(update_branch_endpoint("o", "r", 5), "/repos/o/r/pulls/5/update-branch");
    }

    #[test]
    fn test_search_url() {
        let url = search_url("author:alice", 20);
        assert!(url.contains("type:pr"));
        assert!(url.contains("per_page=20"));
    }

    #[test]
    fn test_reply_endpoint() {
        assert_eq!(reply_endpoint("o", "r", 1, 999), "/repos/o/r/pulls/1/comments/999/replies");
    }

    #[test]
    fn test_submit_review_endpoint() {
        assert_eq!(submit_review_endpoint("o", "r", 1, 42), "/repos/o/r/pulls/1/reviews/42/events");
    }

    #[test]
    fn test_delete_review_endpoint() {
        assert_eq!(delete_review_endpoint("o", "r", 1, 42), "/repos/o/r/pulls/1/reviews/42");
    }

    #[test]
    fn test_create_args_minimal() {
        let args = create_args("Fix", "fix-branch", "main", None, None);
        assert!(args.contains(&"title=Fix".to_string()));
        assert!(args.contains(&"head=fix-branch".to_string()));
        assert!(args.contains(&"base=main".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("body=")));
        assert!(!args.iter().any(|a| a.starts_with("draft=")));
    }

    #[test]
    fn test_create_args_full() {
        let args = create_args("Fix", "fix", "main", Some("desc"), Some(true));
        assert!(args.contains(&"body=desc".to_string()));
        assert!(args.contains(&"draft=true".to_string()));
    }

    #[test]
    fn test_update_args_minimal() {
        let args = update_args(None, None, None, None);
        assert_eq!(args, vec!["-X", "PATCH"]);
    }

    #[test]
    fn test_update_args_full() {
        let args = update_args(Some("New"), Some("Body"), Some("closed"), Some("dev"));
        assert!(args.contains(&"title=New".to_string()));
        assert!(args.contains(&"body=Body".to_string()));
        assert!(args.contains(&"state=closed".to_string()));
        assert!(args.contains(&"base=dev".to_string()));
    }

    #[test]
    fn test_merge_args_minimal() {
        let args = merge_args(None, None, None);
        assert_eq!(args, vec!["-X", "PUT"]);
    }

    #[test]
    fn test_merge_args_full() {
        let args = merge_args(Some("squash"), Some("title"), Some("msg"));
        assert!(args.contains(&"merge_method=squash".to_string()));
        assert!(args.contains(&"commit_title=title".to_string()));
        assert!(args.contains(&"commit_message=msg".to_string()));
    }

    #[test]
    fn test_create_review_args_minimal() {
        let args = create_review_args("APPROVE", None);
        assert!(args.contains(&"event=APPROVE".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("body=")));
    }

    #[test]
    fn test_create_review_args_with_body() {
        let args = create_review_args("COMMENT", Some("Looks good"));
        assert!(args.contains(&"event=COMMENT".to_string()));
        assert!(args.contains(&"body=Looks good".to_string()));
    }

    #[test]
    fn test_add_review_comment_args_minimal() {
        let args = add_review_comment_args("src/lib.rs", "nitpick", None, None, None, "sha123");
        assert!(args.contains(&"path=src/lib.rs".to_string()));
        assert!(args.contains(&"body=nitpick".to_string()));
        assert!(args.contains(&"commit_id=sha123".to_string()));
    }

    #[test]
    fn test_add_review_comment_args_full() {
        let args = add_review_comment_args("file.rs", "comment", Some(42), Some("RIGHT"), Some("LINE"), "sha");
        assert!(args.contains(&"line=42".to_string()));
        assert!(args.contains(&"side=RIGHT".to_string()));
        assert!(args.contains(&"subject_type=LINE".to_string()));
    }

    #[test]
    fn test_extract_search_items() {
        let result = json!({"total_count": 1, "items": [{"id": 1}]});
        let items = extract_search_items(&result).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_extract_search_items_missing() {
        let result = json!({"data": []});
        assert!(extract_search_items(&result).is_none());
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_list_prs() {
        let client = GithubClient::mock(vec![json!([{"number": 1}])]);
        let result = list(&client, "o", "r", Some("open"), Some(10)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_list_prs_no_state() {
        let client = GithubClient::mock(vec![json!([])]);
        let result = list(&client, "o", "r", None, None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_get_pr() {
        let client = GithubClient::mock(vec![json!({"number": 10, "title": "PR"})]);
        let result = get(&client, "o", "r", 10).await.unwrap();
        assert_eq!(result["number"], 10);
    }

    #[tokio::test]
    async fn test_diff_pr() {
        let client = GithubClient::mock(vec![json!("diff --git a/f b/f")]);
        let result = diff(&client, "o", "r", 1).await.unwrap();
        assert!(result.contains("diff"));
    }

    #[tokio::test]
    async fn test_files_pr() {
        let client = GithubClient::mock(vec![json!([{"filename": "a.rs"}])]);
        let result = files(&client, "o", "r", 1).await.unwrap();
        assert_eq!(result[0]["filename"], "a.rs");
    }

    #[tokio::test]
    async fn test_create_pr() {
        let client = GithubClient::mock(vec![json!({"number": 50})]);
        let result = create(&client, "o", "r", "Fix", "fix", "main", Some("body"), Some(true)).await.unwrap();
        assert_eq!(result["number"], 50);
    }

    #[tokio::test]
    async fn test_create_pr_minimal() {
        let client = GithubClient::mock(vec![json!({"number": 1})]);
        let result = create(&client, "o", "r", "PR", "head", "base", None, None).await.unwrap();
        assert_eq!(result["number"], 1);
    }

    #[tokio::test]
    async fn test_update_pr() {
        let client = GithubClient::mock(vec![json!({"number": 10, "state": "closed"})]);
        let result = update(&client, "o", "r", 10, Some("T"), Some("B"), Some("closed"), Some("dev")).await.unwrap();
        assert_eq!(result["state"], "closed");
    }

    #[tokio::test]
    async fn test_update_pr_minimal() {
        let client = GithubClient::mock(vec![json!({"number": 10})]);
        let result = update(&client, "o", "r", 10, None, None, None, None).await.unwrap();
        assert_eq!(result["number"], 10);
    }

    #[tokio::test]
    async fn test_merge_pr() {
        let client = GithubClient::mock(vec![json!({"merged": true})]);
        let result = merge(&client, "o", "r", 5, Some("squash"), Some("title"), Some("msg")).await.unwrap();
        assert_eq!(result["merged"], true);
    }

    #[tokio::test]
    async fn test_merge_pr_minimal() {
        let client = GithubClient::mock(vec![json!({"merged": true})]);
        let result = merge(&client, "o", "r", 5, None, None, None).await.unwrap();
        assert_eq!(result["merged"], true);
    }

    #[tokio::test]
    async fn test_reviews_pr() {
        let client = GithubClient::mock(vec![json!([{"id": 1}])]);
        let result = reviews(&client, "o", "r", 1).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_review_comments_pr() {
        let client = GithubClient::mock(vec![json!([{"id": 1, "body": "nit"}])]);
        let result = review_comments(&client, "o", "r", 1).await.unwrap();
        assert_eq!(result[0]["body"], "nit");
    }

    #[tokio::test]
    async fn test_comments_pr() {
        let client = GithubClient::mock(vec![json!([{"id": 1}])]);
        let result = comments(&client, "o", "r", 1).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_check_runs_pr() {
        let client = GithubClient::mock(vec![
            json!({"head": {"sha": "abc123"}}),
            json!({"check_runs": [{"name": "ci"}]}),
        ]);
        let result = check_runs(&client, "o", "r", 1).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_check_runs_pr_missing_sha() {
        let client = GithubClient::mock(vec![json!({"head": {}})]);
        let result = check_runs(&client, "o", "r", 1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_status_pr() {
        let client = GithubClient::mock(vec![
            json!({"head": {"sha": "abc123"}}),
            json!({"state": "success"}),
        ]);
        let result = status(&client, "o", "r", 1).await.unwrap();
        assert_eq!(result["state"], "success");
    }

    #[tokio::test]
    async fn test_status_pr_missing_sha() {
        let client = GithubClient::mock(vec![json!({"head": {}})]);
        let result = status(&client, "o", "r", 1).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_create_review_pr() {
        let client = GithubClient::mock(vec![json!({"id": 1})]);
        let result = create_review(&client, "o", "r", 1, "APPROVE", Some("LGTM")).await.unwrap();
        assert_eq!(result["id"], 1);
    }

    #[tokio::test]
    async fn test_create_review_pr_no_body() {
        let client = GithubClient::mock(vec![json!({"id": 1})]);
        let result = create_review(&client, "o", "r", 1, "COMMENT", None).await.unwrap();
        assert_eq!(result["id"], 1);
    }

    #[tokio::test]
    async fn test_search_prs() {
        let client = GithubClient::mock(vec![json!({"items": [{"id": 1}]})]);
        let result = search(&client, "author:alice", Some(5)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_search_prs_no_items() {
        let client = GithubClient::mock(vec![json!({"data": "other"})]);
        let result = search(&client, "label:fix", None).await.unwrap();
        assert!(result.is_object());
    }

    #[tokio::test]
    async fn test_add_review_comment_pr() {
        let client = GithubClient::mock(vec![
            json!({"head": {"sha": "commit123"}}),
            json!({"id": 1, "body": "comment"}),
        ]);
        let result = add_review_comment(&client, "o", "r", 1, "file.rs", "comment",
            Some(42), Some("RIGHT"), Some("LINE")).await.unwrap();
        assert_eq!(result["body"], "comment");
    }

    #[tokio::test]
    async fn test_add_review_comment_pr_minimal() {
        let client = GithubClient::mock(vec![
            json!({"head": {"sha": "sha"}}),
            json!({"id": 1}),
        ]);
        let result = add_review_comment(&client, "o", "r", 1, "f.rs", "body",
            None, None, None).await.unwrap();
        assert_eq!(result["id"], 1);
    }

    #[tokio::test]
    async fn test_reply_to_comment_pr() {
        let client = GithubClient::mock(vec![json!({"id": 2, "body": "reply"})]);
        let result = reply_to_comment(&client, "o", "r", 1, 999, "reply").await.unwrap();
        assert_eq!(result["body"], "reply");
    }

    #[tokio::test]
    async fn test_submit_review_pr() {
        let client = GithubClient::mock(vec![json!({"id": 42})]);
        let result = submit_review(&client, "o", "r", 1, 42, "APPROVE", Some("ok")).await.unwrap();
        assert_eq!(result["id"], 42);
    }

    #[tokio::test]
    async fn test_submit_review_pr_no_body() {
        let client = GithubClient::mock(vec![json!({"id": 42})]);
        let result = submit_review(&client, "o", "r", 1, 42, "COMMENT", None).await.unwrap();
        assert_eq!(result["id"], 42);
    }

    #[tokio::test]
    async fn test_delete_review_pr() {
        let client = GithubClient::mock(vec![json!({"id": 42})]);
        let result = delete_review(&client, "o", "r", 1, 42).await.unwrap();
        assert_eq!(result["id"], 42);
    }

    #[tokio::test]
    async fn test_update_branch_pr() {
        let client = GithubClient::mock(vec![json!({"message": "ok"})]);
        let result = update_branch(&client, "o", "r", 1).await.unwrap();
        assert_eq!(result["message"], "ok");
    }
}
