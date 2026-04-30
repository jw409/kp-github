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

/// Get a single commit, including its `files[]` array.
///
/// `include_patches` retains the inline `patch` per file. Default false:
/// patches dominate response size on commits touching many files (GitHub
/// returns up to 300 files per commit) and routinely blow MCP token
/// budgets. Use the `prs_diff` / `repos_compare` tools — or set
/// `include_patches=true` — when diffs are needed.
pub async fn get(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    sha: &str,
    include_patches: bool,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/commits/{sha}");
    let mut result = client.api(&endpoint, &[]).await?;
    if !include_patches {
        strip_patches_from_files(&mut result);
    }
    Ok(result)
}

fn strip_patches_from_files(result: &mut Value) {
    if let Some(files) = result.get_mut("files").and_then(|v| v.as_array_mut()) {
        for f in files.iter_mut() {
            if let Some(obj) = f.as_object_mut() {
                obj.remove("patch");
            }
        }
    }
}

pub(crate) fn strip_patches(result: &mut Value) {
    strip_patches_from_files(result);
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
        let result = get(&client, "o", "r", "abc", false).await.unwrap();
        assert_eq!(result["sha"], "abc");
    }

    #[tokio::test]
    async fn test_get_commit_strips_patches_by_default() {
        let client = GithubClient::mock(vec![json!({
            "sha": "abc",
            "files": [
                {"filename": "a.rs", "patch": "@@ huge diff @@"},
                {"filename": "b.rs", "patch": "@@ another @@"},
            ],
        })]);
        let result = get(&client, "o", "r", "abc", false).await.unwrap();
        for f in result["files"].as_array().unwrap() {
            assert!(f.get("patch").is_none(), "patch must be stripped by default");
            assert!(f.get("filename").is_some());
        }
    }

    #[tokio::test]
    async fn test_get_commit_keeps_patches_when_requested() {
        let client = GithubClient::mock(vec![json!({
            "sha": "abc",
            "files": [{"filename": "a.rs", "patch": "@@ keep @@"}],
        })]);
        let result = get(&client, "o", "r", "abc", true).await.unwrap();
        assert_eq!(result["files"][0]["patch"], "@@ keep @@");
    }
}
