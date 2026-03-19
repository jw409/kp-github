use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// Search repositories using GitHub search syntax.
pub async fn search(
    client: &GithubClient,
    query: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let per_page = limit.unwrap_or(30).min(100);
    let url = format!("/search/repositories?q={}&per_page={per_page}", crate::util::urlencode(query));
    let result = client.api(&url, &[]).await?;

    // Extract .items from search response
    match result {
        Value::Object(ref map) => {
            if let Some(items) = map.get("items") {
                if let Some(limit) = limit {
                    if let Some(arr) = items.as_array() {
                        let limited: Vec<Value> =
                            arr.iter().take(limit as usize).cloned().collect();
                        return Ok(Value::Array(limited));
                    }
                }
                return Ok(items.clone());
            }
            Ok(result)
        }
        _ => Ok(result),
    }
}

/// Get a single repository.
pub async fn get(
    client: &GithubClient,
    owner: &str,
    repo: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}");
    client.api(&endpoint, &[]).await
}

/// Create a new repository.
///
/// If `org` is provided, creates under that organization; otherwise under the
/// authenticated user's account.
pub async fn create(
    client: &GithubClient,
    name: &str,
    description: Option<&str>,
    private: Option<bool>,
    org: Option<&str>,
) -> Result<Value, ClientError> {
    let endpoint = match org {
        Some(o) => format!("/orgs/{o}/repos"),
        None => "/user/repos".to_string(),
    };

    let name_field = format!("name={name}");
    let mut args = vec!["-X", "POST", "-f", &name_field];

    let desc_field;
    if let Some(d) = description {
        desc_field = format!("description={d}");
        args.push("-f");
        args.push(&desc_field);
    }

    let private_field;
    if let Some(p) = private {
        private_field = format!("private={p}");
        args.push("-f");
        args.push(&private_field);
    }

    client.api(&endpoint, &args).await
}

/// Compare two commits, branches, or tags.
pub async fn compare(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    base: &str,
    head: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/compare/{base}...{head}");
    client.api(&endpoint, &[]).await
}

/// Fork a repository.
///
/// If `org` is provided, forks into that organization.
pub async fn fork(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    org: Option<&str>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/forks");

    let mut args = vec!["-X", "POST"];

    let org_field;
    if let Some(o) = org {
        org_field = format!("organization={o}");
        args.push("-f");
        args.push(&org_field);
    }

    client.api(&endpoint, &args).await
}

fn get_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}")
}

fn create_endpoint(org: Option<&str>) -> String {
    match org {
        Some(o) => format!("/orgs/{o}/repos"),
        None => "/user/repos".to_string(),
    }
}

fn compare_endpoint(owner: &str, repo: &str, base: &str, head: &str) -> String {
    format!("/repos/{owner}/{repo}/compare/{base}...{head}")
}

fn fork_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/forks")
}

fn search_url(query: &str, per_page: u32) -> String {
    format!("/search/repositories?q={}&per_page={per_page}", crate::util::urlencode(query))
}

fn extract_search_items(result: &Value, limit: Option<u32>) -> Value {
    match result {
        Value::Object(ref map) => {
            if let Some(items) = map.get("items") {
                if let Some(limit) = limit {
                    if let Some(arr) = items.as_array() {
                        let limited: Vec<Value> =
                            arr.iter().take(limit as usize).cloned().collect();
                        return Value::Array(limited);
                    }
                }
                return items.clone();
            }
            result.clone()
        }
        _ => result.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_endpoint() {
        assert_eq!(get_endpoint("owner", "repo"), "/repos/owner/repo");
    }

    #[test]
    fn test_create_endpoint_user() {
        assert_eq!(create_endpoint(None), "/user/repos");
    }

    #[test]
    fn test_create_endpoint_org() {
        assert_eq!(create_endpoint(Some("my-org")), "/orgs/my-org/repos");
    }

    #[test]
    fn test_compare_endpoint() {
        assert_eq!(compare_endpoint("o", "r", "main", "feat"), "/repos/o/r/compare/main...feat");
    }

    #[test]
    fn test_fork_endpoint() {
        assert_eq!(fork_endpoint("upstream", "project"), "/repos/upstream/project/forks");
    }

    #[test]
    fn test_search_url() {
        let url = search_url("language:rust", 10);
        assert!(url.starts_with("/search/repositories?q="));
        assert!(url.contains("per_page=10"));
    }

    #[test]
    fn test_extract_search_items_with_items() {
        let result = json!({"total_count": 2, "items": [{"id": 1}, {"id": 2}]});
        let items = extract_search_items(&result, None);
        assert_eq!(items.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_extract_search_items_with_limit() {
        let result = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});
        let items = extract_search_items(&result, Some(2));
        assert_eq!(items.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_extract_search_items_no_items() {
        let result = json!({"data": []});
        let items = extract_search_items(&result, None);
        assert!(items.is_object()); // returns original
    }

    #[test]
    fn test_extract_search_items_non_object() {
        let result = json!([1, 2]);
        let items = extract_search_items(&result, None);
        assert!(items.is_array());
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_search_repos() {
        let client = GithubClient::mock(vec![json!({"items": [{"full_name": "a/b"}]})]);
        let result = search(&client, "language:rust", Some(5)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_search_repos_no_limit() {
        let client = GithubClient::mock(vec![json!({"items": [{"id": 1}, {"id": 2}]})]);
        let result = search(&client, "stars:>100", None).await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_search_repos_no_items() {
        let client = GithubClient::mock(vec![json!({"data": "other"})]);
        let result = search(&client, "foo", None).await.unwrap();
        assert!(result.is_object());
    }

    #[tokio::test]
    async fn test_search_repos_non_object() {
        let client = GithubClient::mock(vec![json!([1, 2])]);
        let result = search(&client, "foo", None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_get_repo() {
        let client = GithubClient::mock(vec![json!({"full_name": "o/r"})]);
        let result = get(&client, "o", "r").await.unwrap();
        assert_eq!(result["full_name"], "o/r");
    }

    #[tokio::test]
    async fn test_create_repo() {
        let client = GithubClient::mock(vec![json!({"full_name": "alice/new"})]);
        let result = create(&client, "new", Some("desc"), Some(true), None).await.unwrap();
        assert_eq!(result["full_name"], "alice/new");
    }

    #[tokio::test]
    async fn test_create_repo_in_org() {
        let client = GithubClient::mock(vec![json!({"full_name": "org/repo"})]);
        let result = create(&client, "repo", None, None, Some("org")).await.unwrap();
        assert_eq!(result["full_name"], "org/repo");
    }

    #[tokio::test]
    async fn test_compare_repos() {
        let client = GithubClient::mock(vec![json!({"status": "ahead"})]);
        let result = compare(&client, "o", "r", "main", "feat").await.unwrap();
        assert_eq!(result["status"], "ahead");
    }

    #[tokio::test]
    async fn test_fork_repo() {
        let client = GithubClient::mock(vec![json!({"full_name": "alice/project"})]);
        let result = fork(&client, "upstream", "project", None).await.unwrap();
        assert_eq!(result["full_name"], "alice/project");
    }

    #[tokio::test]
    async fn test_fork_repo_to_org() {
        let client = GithubClient::mock(vec![json!({"full_name": "org/project"})]);
        let result = fork(&client, "upstream", "project", Some("org")).await.unwrap();
        assert_eq!(result["full_name"], "org/project");
    }
}
