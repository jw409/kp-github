use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};
use crate::tools::search_util::extract_search_items;

/// List issues for a repository.
pub async fn list(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    state: Option<&str>,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues");
    let mut params: Vec<(&str, &str)> = Vec::new();
    if let Some(s) = state {
        params.push(("state", s));
    }
    client.api_list(&endpoint, &params, limit).await
}

/// Get a single issue by number.
pub async fn get(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues/{number}");
    client.api(&endpoint, &[]).await
}

/// Search issues using GitHub search syntax.
/// Appends `type:issue` unless the query already contains a `type:` qualifier.
pub async fn search(
    client: &GithubClient,
    query: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let full_query = if query.contains("type:") {
        query.to_string()
    } else {
        format!("{query} type:issue")
    };
    let per_page = limit.unwrap_or(30).min(100);
    let url = format!("/search/issues?q={}&per_page={per_page}", crate::util::urlencode(&full_query));
    let result = client.api(&url, &[]).await?;

    // Extract .items from search response, preserving total_count
    extract_search_items(&result, limit)
}

/// Create a new issue.
pub async fn create(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    title: &str,
    body: Option<&str>,
    labels: Option<Vec<String>>,
    assignees: Option<Vec<String>>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues");

    let mut args: Vec<String> = vec!["-X".into(), "POST".into()];
    args.push("-f".into());
    args.push(format!("title={title}"));

    if let Some(b) = body {
        args.push("-f".into());
        args.push(format!("body={b}"));
    }

    if let Some(ref lbls) = labels {
        for label in lbls {
            args.push("-f".into());
            args.push(format!("labels[]={label}"));
        }
    }

    if let Some(ref asn) = assignees {
        for assignee in asn {
            args.push("-f".into());
            args.push(format!("assignees[]={assignee}"));
        }
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    client.api(&endpoint, &arg_refs).await
}

/// Update an existing issue.
pub async fn update(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    title: Option<String>,
    body: Option<String>,
    state: Option<String>,
    labels: Option<Vec<String>>,
    assignees: Option<Vec<String>>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues/{number}");

    let mut args: Vec<String> = vec!["-X".into(), "PATCH".into()];

    if let Some(ref t) = title {
        args.push("-f".into());
        args.push(format!("title={t}"));
    }
    if let Some(ref b) = body {
        args.push("-f".into());
        args.push(format!("body={b}"));
    }
    if let Some(ref s) = state {
        args.push("-f".into());
        args.push(format!("state={s}"));
    }
    if let Some(ref lbls) = labels {
        for label in lbls {
            args.push("-f".into());
            args.push(format!("labels[]={label}"));
        }
    }
    if let Some(ref asn) = assignees {
        for assignee in asn {
            args.push("-f".into());
            args.push(format!("assignees[]={assignee}"));
        }
    }

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    client.api(&endpoint, &arg_refs).await
}

/// Add a comment to an issue.
pub async fn comment(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    body: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues/{number}/comments");
    let body_field = format!("body={body}");
    client.api(&endpoint, &["-X", "POST", "-f", &body_field]).await
}

/// List comments on an issue.
pub async fn comments(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues/{number}/comments");
    client.api_list(&endpoint, &[], limit).await
}

/// Get labels on an issue.
pub async fn labels(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues/{number}/labels");
    client.api(&endpoint, &[]).await
}

/// Get sub-issues of an issue.
pub async fn sub_issues(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    number: u32,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/issues/{number}/sub_issues");
    let result = client.api_list(&endpoint, &[], limit).await?;
    // Extract items if the response wraps them
    if let Value::Object(ref map) = result {
        if let Some(items) = map.get("items") {
            return Ok(items.clone());
        }
    }
    Ok(result)
}

/// List issue types for an organization.
pub async fn list_issue_types(
    client: &GithubClient,
    owner: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/orgs/{owner}/issue-types");
    client.api(&endpoint, &[]).await
}

// --- Testable helpers (only used in tests) ---

#[cfg(test)]
fn list_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/issues")
}

#[cfg(test)]
fn get_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/issues/{number}")
}

#[cfg(test)]
fn search_url(query: &str, per_page: u32) -> String {
    let full_query = if query.contains("type:") {
        query.to_string()
    } else {
        format!("{query} type:issue")
    };
    format!("/search/issues?q={}&per_page={per_page}", crate::util::urlencode(&full_query))
}

#[cfg(test)]
fn comments_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/issues/{number}/comments")
}

#[cfg(test)]
fn labels_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/issues/{number}/labels")
}

#[cfg(test)]
fn sub_issues_endpoint(owner: &str, repo: &str, number: u32) -> String {
    format!("/repos/{owner}/{repo}/issues/{number}/sub_issues")
}

#[cfg(test)]
fn issue_types_endpoint(owner: &str) -> String {
    format!("/orgs/{owner}/issue-types")
}

#[cfg(test)]
fn create_args(title: &str, body: Option<&str>, labels: &Option<Vec<String>>, assignees: &Option<Vec<String>>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-X".into(), "POST".into()];
    args.push("-f".into());
    args.push(format!("title={title}"));
    if let Some(b) = body {
        args.push("-f".into());
        args.push(format!("body={b}"));
    }
    if let Some(ref lbls) = labels {
        for label in lbls {
            args.push("-f".into());
            args.push(format!("labels[]={label}"));
        }
    }
    if let Some(ref asn) = assignees {
        for assignee in asn {
            args.push("-f".into());
            args.push(format!("assignees[]={assignee}"));
        }
    }
    args
}

#[cfg(test)]
fn update_args(
    title: &Option<String>,
    body: &Option<String>,
    state: &Option<String>,
    labels: &Option<Vec<String>>,
    assignees: &Option<Vec<String>>,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-X".into(), "PATCH".into()];
    if let Some(ref t) = title {
        args.push("-f".into());
        args.push(format!("title={t}"));
    }
    if let Some(ref b) = body {
        args.push("-f".into());
        args.push(format!("body={b}"));
    }
    if let Some(ref s) = state {
        args.push("-f".into());
        args.push(format!("state={s}"));
    }
    if let Some(ref lbls) = labels {
        for label in lbls {
            args.push("-f".into());
            args.push(format!("labels[]={label}"));
        }
    }
    if let Some(ref asn) = assignees {
        for assignee in asn {
            args.push("-f".into());
            args.push(format!("assignees[]={assignee}"));
        }
    }
    args
}

#[cfg(test)]
fn extract_search_items_old(result: &Value) -> Option<Value> {
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
        assert_eq!(list_endpoint("owner", "repo"), "/repos/owner/repo/issues");
    }

    #[test]
    fn test_get_endpoint() {
        assert_eq!(get_endpoint("owner", "repo", 42), "/repos/owner/repo/issues/42");
    }

    #[test]
    fn test_search_url() {
        let url = search_url("is:open label:bug", 10);
        assert!(url.starts_with("/search/issues?q="));
        // type:issue is URL-encoded
        assert!(url.contains("type%3Aissue"));
        assert!(url.contains("per_page=10"));
    }

    #[test]
    fn test_comments_endpoint() {
        assert_eq!(comments_endpoint("o", "r", 5), "/repos/o/r/issues/5/comments");
    }

    #[test]
    fn test_labels_endpoint() {
        assert_eq!(labels_endpoint("o", "r", 3), "/repos/o/r/issues/3/labels");
    }

    #[test]
    fn test_sub_issues_endpoint() {
        assert_eq!(sub_issues_endpoint("o", "r", 7), "/repos/o/r/issues/7/sub_issues");
    }

    #[test]
    fn test_issue_types_endpoint() {
        assert_eq!(issue_types_endpoint("my-org"), "/orgs/my-org/issue-types");
    }

    #[test]
    fn test_create_args_minimal() {
        let args = create_args("Bug report", None, &None, &None);
        assert_eq!(args, vec!["-X", "POST", "-f", "title=Bug report"]);
    }

    #[test]
    fn test_create_args_full() {
        let labels = Some(vec!["bug".into(), "p1".into()]);
        let assignees = Some(vec!["alice".into()]);
        let args = create_args("Bug", Some("desc"), &labels, &assignees);
        assert!(args.contains(&"-X".to_string()));
        assert!(args.contains(&"POST".to_string()));
        assert!(args.contains(&"title=Bug".to_string()));
        assert!(args.contains(&"body=desc".to_string()));
        assert!(args.contains(&"labels[]=bug".to_string()));
        assert!(args.contains(&"labels[]=p1".to_string()));
        assert!(args.contains(&"assignees[]=alice".to_string()));
    }

    #[test]
    fn test_update_args_minimal() {
        let args = update_args(&None, &None, &None, &None, &None);
        assert_eq!(args, vec!["-X", "PATCH"]);
    }

    #[test]
    fn test_update_args_state_only() {
        let args = update_args(&None, &None, &Some("closed".into()), &None, &None);
        assert!(args.contains(&"state=closed".to_string()));
    }

    #[test]
    fn test_update_args_full() {
        let labels = Some(vec!["bug".into()]);
        let assignees = Some(vec!["bob".into()]);
        let args = update_args(
            &Some("New Title".into()),
            &Some("New Body".into()),
            &Some("open".into()),
            &labels,
            &assignees,
        );
        assert!(args.contains(&"title=New Title".to_string()));
        assert!(args.contains(&"body=New Body".to_string()));
        assert!(args.contains(&"state=open".to_string()));
        assert!(args.contains(&"labels[]=bug".to_string()));
        assert!(args.contains(&"assignees[]=bob".to_string()));
    }

    #[test]
    fn test_extract_search_items_with_items() {
        let result = json!({"total_count": 2, "items": [{"id": 1}, {"id": 2}]});
        let extracted = extract_search_items(&result, None).unwrap();
        assert_eq!(extracted["items"].as_array().unwrap().len(), 2);
        assert_eq!(extracted["total_count"], 2);
        assert_eq!(extracted["truncated"], false);
    }

    #[test]
    fn test_extract_search_items_without_items() {
        let result = json!({"data": "something"});
        let extracted = extract_search_items(&result, None).unwrap();
        assert_eq!(extracted["data"], "something");
    }

    #[test]
    fn test_extract_search_items_non_object() {
        let result = json!([1, 2, 3]);
        let extracted = extract_search_items(&result, None).unwrap();
        assert!(extracted.is_array());
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_list_issues() {
        let client = GithubClient::mock(vec![json!([{"number": 1, "title": "Bug"}])]);
        let result = list(&client, "o", "r", Some("open"), Some(10)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_list_issues_no_state() {
        let client = GithubClient::mock(vec![json!([])]);
        let result = list(&client, "o", "r", None, None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_get_issue() {
        let client = GithubClient::mock(vec![json!({"number": 42, "title": "Bug"})]);
        let result = get(&client, "o", "r", 42).await.unwrap();
        assert_eq!(result["number"], 42);
    }

    #[tokio::test]
    async fn test_search_issues_with_items() {
        let client = GithubClient::mock(vec![json!({"items": [{"id": 1}], "total_count": 1})]);
        let result = search(&client, "is:open", Some(10)).await.unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 1);
        assert_eq!(result["total_count"], 1);
    }

    #[tokio::test]
    async fn test_search_issues_respects_existing_type_qualifier() {
        // Finding #13: should not append type:issue when query already has type:
        let client = GithubClient::mock(vec![json!({"items": [], "total_count": 0})]);
        let result = search(&client, "type:pr author:jw", Some(10)).await.unwrap();
        assert!(result["items"].is_array());
    }

    #[tokio::test]
    async fn test_search_issues_no_items() {
        let client = GithubClient::mock(vec![json!({"data": "other"})]);
        let result = search(&client, "label:bug", None).await.unwrap();
        assert!(result.is_object());
    }

    #[tokio::test]
    async fn test_create_issue() {
        let client = GithubClient::mock(vec![json!({"number": 99, "title": "New"})]);
        let result = create(&client, "o", "r", "New", Some("body"),
            Some(vec!["bug".into()]), Some(vec!["alice".into()])).await.unwrap();
        assert_eq!(result["number"], 99);
    }

    #[tokio::test]
    async fn test_create_issue_minimal() {
        let client = GithubClient::mock(vec![json!({"number": 1})]);
        let result = create(&client, "o", "r", "Title", None, None, None).await.unwrap();
        assert_eq!(result["number"], 1);
    }

    #[tokio::test]
    async fn test_update_issue() {
        let client = GithubClient::mock(vec![json!({"number": 5, "state": "closed"})]);
        let result = update(&client, "o", "r", 5,
            Some("New Title".into()), Some("Body".into()),
            Some("closed".into()), Some(vec!["bug".into()]),
            Some(vec!["bob".into()])).await.unwrap();
        assert_eq!(result["state"], "closed");
    }

    #[tokio::test]
    async fn test_update_issue_minimal() {
        let client = GithubClient::mock(vec![json!({"number": 5})]);
        let result = update(&client, "o", "r", 5, None, None, None, None, None).await.unwrap();
        assert_eq!(result["number"], 5);
    }

    #[tokio::test]
    async fn test_comment_issue() {
        let client = GithubClient::mock(vec![json!({"id": 1, "body": "LGTM"})]);
        let result = comment(&client, "o", "r", 1, "LGTM").await.unwrap();
        assert_eq!(result["body"], "LGTM");
    }

    #[tokio::test]
    async fn test_comments_issue() {
        let client = GithubClient::mock(vec![json!([{"id": 1}, {"id": 2}])]);
        let result = comments(&client, "o", "r", 1, Some(10)).await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_labels_issue() {
        let client = GithubClient::mock(vec![json!([{"name": "bug"}])]);
        let result = labels(&client, "o", "r", 1).await.unwrap();
        assert_eq!(result[0]["name"], "bug");
    }

    #[tokio::test]
    async fn test_sub_issues_with_items() {
        let client = GithubClient::mock(vec![json!({"items": [{"number": 2}]})]);
        let result = sub_issues(&client, "o", "r", 1, None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_sub_issues_plain() {
        let client = GithubClient::mock(vec![json!([{"number": 2}])]);
        let result = sub_issues(&client, "o", "r", 1, Some(5)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_list_issue_types_fn() {
        let client = GithubClient::mock(vec![json!([{"name": "Bug"}])]);
        let result = list_issue_types(&client, "my-org").await.unwrap();
        assert!(result.is_array());
    }
}

