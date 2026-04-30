use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// List branches for a repository.
pub async fn list(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/branches");
    client.api_list(&endpoint, &[], limit).await
}

/// Create a new branch from a given SHA.
pub async fn create(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
    from_sha: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/git/refs");

    let ref_field = format!("ref=refs/heads/{branch}");
    let sha_field = format!("sha={from_sha}");

    let args = vec!["-X", "POST", "-f", &ref_field, "-f", &sha_field];

    client.api(&endpoint, &args).await
}

#[cfg(test)]
fn list_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/branches")
}

#[cfg(test)]
fn create_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/git/refs")
}

#[cfg(test)]
fn create_ref_field(branch: &str) -> String {
    format!("ref=refs/heads/{branch}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_list_endpoint() {
        assert_eq!(list_endpoint("o", "r"), "/repos/o/r/branches");
    }

    #[test]
    fn test_create_endpoint() {
        assert_eq!(create_endpoint("o", "r"), "/repos/o/r/git/refs");
    }

    #[test]
    fn test_create_ref_field() {
        assert_eq!(create_ref_field("feature-x"), "ref=refs/heads/feature-x");
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_list_branches() {
        let client = GithubClient::mock(vec![json!([{"name": "main"}, {"name": "dev"}])]);
        let result = list(&client, "o", "r", None).await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_create_branch() {
        let client = GithubClient::mock(vec![json!({"ref": "refs/heads/feature"})]);
        let result = create(&client, "o", "r", "feature", "abc123").await.unwrap();
        assert!(result["ref"].as_str().unwrap().contains("feature"));
    }
}
