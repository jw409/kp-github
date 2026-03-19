use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// List tags in a repository.
pub async fn list(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/tags");
    client.api_list(&endpoint, &[], limit).await
}

/// Get a specific tag.
pub async fn get(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    tag: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/git/ref/tags/{tag}");
    client.api(&endpoint, &[]).await
}

fn list_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/tags")
}

fn get_endpoint(owner: &str, repo: &str, tag: &str) -> String {
    format!("/repos/{owner}/{repo}/git/ref/tags/{tag}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_list_endpoint() {
        assert_eq!(list_endpoint("o", "r"), "/repos/o/r/tags");
    }

    #[test]
    fn test_get_endpoint() {
        assert_eq!(get_endpoint("o", "r", "v1.0.0"), "/repos/o/r/git/ref/tags/v1.0.0");
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_list_tags() {
        let client = GithubClient::mock(vec![json!([{"name": "v1.0"}])]);
        let result = list(&client, "o", "r", Some(10)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_get_tag() {
        let client = GithubClient::mock(vec![json!({"ref": "refs/tags/v1.0"})]);
        let result = get(&client, "o", "r", "v1.0").await.unwrap();
        assert!(result["ref"].as_str().unwrap().contains("v1.0"));
    }
}
