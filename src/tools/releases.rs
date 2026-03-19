use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// List releases for a repository.
pub async fn list(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/releases");
    client.api_list(&endpoint, &[], limit).await
}

/// Get a release by tag.
pub async fn get_by_tag(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    tag: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/releases/tags/{tag}");
    client.api(&endpoint, &[]).await
}

/// Create a new release.
pub async fn create(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    tag_name: &str,
    name: Option<&str>,
    body: Option<&str>,
    draft: Option<bool>,
    prerelease: Option<bool>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/releases");
    let tag_field = format!("tag_name={tag_name}");
    let mut args: Vec<&str> = vec!["-X", "POST", "-f", &tag_field];

    let name_field;
    if let Some(n) = name {
        name_field = format!("name={n}");
        args.push("-f");
        args.push(&name_field);
    }
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
    let pre_field;
    if let Some(p) = prerelease {
        pre_field = format!("prerelease={p}");
        args.push("-f");
        args.push(&pre_field);
    }
    client.api(&endpoint, &args).await
}

/// Get the latest release.
pub async fn get_latest(
    client: &GithubClient,
    owner: &str,
    repo: &str,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/releases/latest");
    client.api(&endpoint, &[]).await
}

fn list_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/releases")
}

fn get_by_tag_endpoint(owner: &str, repo: &str, tag: &str) -> String {
    format!("/repos/{owner}/{repo}/releases/tags/{tag}")
}

fn latest_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/releases/latest")
}

fn create_args(tag_name: &str, name: Option<&str>, body: Option<&str>, draft: Option<bool>, prerelease: Option<bool>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-X".into(), "POST".into(),
        "-f".into(), format!("tag_name={tag_name}"),
    ];
    if let Some(n) = name {
        args.push("-f".into());
        args.push(format!("name={n}"));
    }
    if let Some(b) = body {
        args.push("-f".into());
        args.push(format!("body={b}"));
    }
    if let Some(d) = draft {
        args.push("-f".into());
        args.push(format!("draft={d}"));
    }
    if let Some(p) = prerelease {
        args.push("-f".into());
        args.push(format!("prerelease={p}"));
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_list_endpoint() {
        assert_eq!(list_endpoint("o", "r"), "/repos/o/r/releases");
    }

    #[test]
    fn test_get_by_tag_endpoint() {
        assert_eq!(get_by_tag_endpoint("o", "r", "v1.0"), "/repos/o/r/releases/tags/v1.0");
    }

    #[test]
    fn test_latest_endpoint() {
        assert_eq!(latest_endpoint("o", "r"), "/repos/o/r/releases/latest");
    }

    #[test]
    fn test_create_args_minimal() {
        let args = create_args("v1.0", None, None, None, None);
        assert!(args.contains(&"tag_name=v1.0".to_string()));
        assert_eq!(args.len(), 4); // -X, POST, -f, tag_name=v1.0
    }

    #[test]
    fn test_create_args_full() {
        let args = create_args("v1.0", Some("Release"), Some("Notes"), Some(false), Some(true));
        assert!(args.contains(&"name=Release".to_string()));
        assert!(args.contains(&"body=Notes".to_string()));
        assert!(args.contains(&"draft=false".to_string()));
        assert!(args.contains(&"prerelease=true".to_string()));
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_list_releases() {
        let client = GithubClient::mock(vec![json!([{"tag_name": "v1.0"}])]);
        let result = list(&client, "o", "r", Some(5)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_get_by_tag_release() {
        let client = GithubClient::mock(vec![json!({"tag_name": "v1.0"})]);
        let result = get_by_tag(&client, "o", "r", "v1.0").await.unwrap();
        assert_eq!(result["tag_name"], "v1.0");
    }

    #[tokio::test]
    async fn test_get_latest_release() {
        let client = GithubClient::mock(vec![json!({"tag_name": "v2.0"})]);
        let result = get_latest(&client, "o", "r").await.unwrap();
        assert_eq!(result["tag_name"], "v2.0");
    }

    #[tokio::test]
    async fn test_create_release() {
        let client = GithubClient::mock(vec![json!({"tag_name": "v3.0"})]);
        let result = create(&client, "o", "r", "v3.0", Some("R"), Some("Notes"), Some(false), Some(true)).await.unwrap();
        assert_eq!(result["tag_name"], "v3.0");
    }

    #[tokio::test]
    async fn test_create_release_minimal() {
        let client = GithubClient::mock(vec![json!({"tag_name": "v1.0"})]);
        let result = create(&client, "o", "r", "v1.0", None, None, None, None).await.unwrap();
        assert_eq!(result["tag_name"], "v1.0");
    }
}
