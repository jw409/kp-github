use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// List commits for a repository, optionally filtered by branch/sha.
pub async fn list(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    sha: Option<&str>,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/commits");
    let mut params: Vec<(&str, &str)> = Vec::new();
    if let Some(s) = sha {
        params.push(("sha", s));
    }
    client.api_list(&endpoint, &params, limit).await
}

/// Get a single commit with diff.
pub async fn get(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    sha: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/commits/{sha}");
    client.api(&endpoint, &[]).await
}

#[cfg(test)]
fn list_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/commits")
}

#[cfg(test)]
fn get_endpoint(owner: &str, repo: &str, sha: &str) -> String {
    format!("/repos/{owner}/{repo}/commits/{sha}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_list_endpoint() {
        assert_eq!(list_endpoint("o", "r"), "/repos/o/r/commits");
    }

    #[test]
    fn test_get_endpoint() {
        assert_eq!(get_endpoint("o", "r", "abc123"), "/repos/o/r/commits/abc123");
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_list_commits() {
        let client = GithubClient::mock(vec![json!([{"sha": "abc"}])]);
        let result = list(&client, "o", "r", Some("main"), Some(5)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_list_commits_no_sha() {
        let client = GithubClient::mock(vec![json!([])]);
        let result = list(&client, "o", "r", None, None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_get_commit() {
        let client = GithubClient::mock(vec![json!({"sha": "abc", "commit": {"message": "fix"}})]);
        let result = get(&client, "o", "r", "abc").await.unwrap();
        assert_eq!(result["sha"], "abc");
    }
}
