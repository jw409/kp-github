use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// Get a specific label from a repository.
pub async fn get(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    name: &str,
) -> Result<Value, ClientError> {
    let encoded = crate::util::urlencode(name);
    let endpoint = format!("/repos/{owner}/{repo}/labels/{encoded}");
    client.api(&endpoint, &[]).await
}

/// List labels in a repository.
pub async fn list(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/labels");
    client.api_list(&endpoint, &[], limit).await
}

/// Create a label in a repository.
pub async fn create(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    name: &str,
    color: Option<&str>,
    description: Option<&str>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/labels");
    let name_field = format!("name={name}");
    let mut args: Vec<&str> = vec!["-X", "POST", "-f", &name_field];

    let color_field;
    if let Some(c) = color {
        color_field = format!("color={c}");
        args.push("-f");
        args.push(&color_field);
    }
    let desc_field;
    if let Some(d) = description {
        desc_field = format!("description={d}");
        args.push("-f");
        args.push(&desc_field);
    }
    client.api(&endpoint, &args).await
}

/// Update a label in a repository.
pub async fn update(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    name: &str,
    new_name: Option<&str>,
    color: Option<&str>,
    description: Option<&str>,
) -> Result<Value, ClientError> {
    let encoded = crate::util::urlencode(name);
    let endpoint = format!("/repos/{owner}/{repo}/labels/{encoded}");
    let mut args: Vec<&str> = vec!["-X", "PATCH"];

    let name_field;
    if let Some(n) = new_name {
        name_field = format!("new_name={n}");
        args.push("-f");
        args.push(&name_field);
    }
    let color_field;
    if let Some(c) = color {
        color_field = format!("color={c}");
        args.push("-f");
        args.push(&color_field);
    }
    let desc_field;
    if let Some(d) = description {
        desc_field = format!("description={d}");
        args.push("-f");
        args.push(&desc_field);
    }
    client.api(&endpoint, &args).await
}

/// Delete a label from a repository.
pub async fn delete(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    name: &str,
) -> Result<Value, ClientError> {
    let encoded = crate::util::urlencode(name);
    let endpoint = format!("/repos/{owner}/{repo}/labels/{encoded}");
    client.api(&endpoint, &["-X", "DELETE"]).await
}

#[cfg(test)]
fn get_endpoint(owner: &str, repo: &str, name: &str) -> String {
    let encoded = crate::util::urlencode(name);
    format!("/repos/{owner}/{repo}/labels/{encoded}")
}

#[cfg(test)]
fn list_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/labels")
}

#[cfg(test)]
fn create_args(name: &str, color: Option<&str>, description: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "-X".into(), "POST".into(),
        "-f".into(), format!("name={name}"),
    ];
    if let Some(c) = color {
        args.push("-f".into());
        args.push(format!("color={c}"));
    }
    if let Some(d) = description {
        args.push("-f".into());
        args.push(format!("description={d}"));
    }
    args
}

#[cfg(test)]
fn update_args(new_name: Option<&str>, color: Option<&str>, description: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = vec!["-X".into(), "PATCH".into()];
    if let Some(n) = new_name {
        args.push("-f".into());
        args.push(format!("new_name={n}"));
    }
    if let Some(c) = color {
        args.push("-f".into());
        args.push(format!("color={c}"));
    }
    if let Some(d) = description {
        args.push("-f".into());
        args.push(format!("description={d}"));
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_endpoint() {
        assert_eq!(get_endpoint("o", "r", "bug"), "/repos/o/r/labels/bug");
    }

    #[test]
    fn test_get_endpoint_with_spaces() {
        let ep = get_endpoint("o", "r", "help wanted");
        assert!(ep.contains("help+wanted"));
    }

    #[test]
    fn test_list_endpoint() {
        assert_eq!(list_endpoint("o", "r"), "/repos/o/r/labels");
    }

    #[test]
    fn test_create_args_minimal() {
        let args = create_args("bug", None, None);
        assert_eq!(args, vec!["-X", "POST", "-f", "name=bug"]);
    }

    #[test]
    fn test_create_args_full() {
        let args = create_args("bug", Some("ff0000"), Some("Bug report"));
        assert!(args.contains(&"color=ff0000".to_string()));
        assert!(args.contains(&"description=Bug report".to_string()));
    }

    #[test]
    fn test_update_args_minimal() {
        let args = update_args(None, None, None);
        assert_eq!(args, vec!["-X", "PATCH"]);
    }

    #[test]
    fn test_update_args_full() {
        let args = update_args(Some("defect"), Some("00ff00"), Some("Updated"));
        assert!(args.contains(&"new_name=defect".to_string()));
        assert!(args.contains(&"color=00ff00".to_string()));
        assert!(args.contains(&"description=Updated".to_string()));
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_get_label() {
        let client = GithubClient::mock(vec![json!({"name": "bug", "color": "ff0000"})]);
        let result = get(&client, "o", "r", "bug").await.unwrap();
        assert_eq!(result["name"], "bug");
    }

    #[tokio::test]
    async fn test_list_labels() {
        let client = GithubClient::mock(vec![json!([{"name": "bug"}, {"name": "enhancement"}])]);
        let result = list(&client, "o", "r", Some(10)).await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_create_label() {
        let client = GithubClient::mock(vec![json!({"name": "priority"})]);
        let result = create(&client, "o", "r", "priority", Some("ff0000"), Some("High priority")).await.unwrap();
        assert_eq!(result["name"], "priority");
    }

    #[tokio::test]
    async fn test_create_label_minimal() {
        let client = GithubClient::mock(vec![json!({"name": "bug"})]);
        let result = create(&client, "o", "r", "bug", None, None).await.unwrap();
        assert_eq!(result["name"], "bug");
    }

    #[tokio::test]
    async fn test_update_label() {
        let client = GithubClient::mock(vec![json!({"name": "defect"})]);
        let result = update(&client, "o", "r", "bug", Some("defect"), Some("ff0000"), Some("desc")).await.unwrap();
        assert_eq!(result["name"], "defect");
    }

    #[tokio::test]
    async fn test_update_label_minimal() {
        let client = GithubClient::mock(vec![json!({"name": "bug"})]);
        let result = update(&client, "o", "r", "bug", None, None, None).await.unwrap();
        assert_eq!(result["name"], "bug");
    }

    #[tokio::test]
    async fn test_delete_label() {
        let client = GithubClient::mock(vec![json!(null)]);
        let result = delete(&client, "o", "r", "stale").await.unwrap();
        assert!(result.is_null());
    }
}
