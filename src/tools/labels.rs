use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// Get a specific label from a repository.
pub async fn get(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    name: &str,
) -> Result<Value, ClientError> {
    let encoded = crate::util::urlencode_path(name);
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
    let encoded = crate::util::urlencode_path(name);
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
    let encoded = crate::util::urlencode_path(name);
    let endpoint = format!("/repos/{owner}/{repo}/labels/{encoded}");
    client.api(&endpoint, &["-X", "DELETE"]).await
}

#[cfg(test)]
fn get_endpoint(owner: &str, repo: &str, name: &str) -> String {
    let encoded = crate::util::urlencode_path(name);
    format!("/repos/{owner}/{repo}/labels/{encoded}")
}

#[cfg(test)]
fn update_endpoint(owner: &str, repo: &str, name: &str) -> String {
    get_endpoint(owner, repo, name)
}

#[cfg(test)]
fn delete_endpoint(owner: &str, repo: &str, name: &str) -> String {
    get_endpoint(owner, repo, name)
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
        // Path segments must use %20 for space, NOT form-style `+`.
        // GitHub treats `+` as a literal character in a path, so a label
        // named "help wanted" sent as "help+wanted" returns 404.
        let ep = get_endpoint("o", "r", "help wanted");
        assert_eq!(ep, "/repos/o/r/labels/help%20wanted");
        assert!(!ep.contains('+'), "must not use form-style + for space: {ep}");
    }

    #[test]
    fn test_get_endpoint_issue_19_regression() {
        // Exact label from kinderpowers#19: GET/PATCH/DELETE on these all 404'd
        // because the name was form-encoded into the path.
        let ep = get_endpoint("Meshlyai", "meshly-backend", "priority: P0");
        assert_eq!(ep, "/repos/Meshlyai/meshly-backend/labels/priority%3A%20P0");
    }

    #[test]
    fn test_update_and_delete_endpoints_match_get() {
        // All three label-name path tools must encode identically — the bug
        // from issue #19 affected get/update/delete the same way.
        let name = "priority: P0";
        assert_eq!(get_endpoint("o", "r", name), update_endpoint("o", "r", name));
        assert_eq!(get_endpoint("o", "r", name), delete_endpoint("o", "r", name));
    }

    #[test]
    fn test_label_endpoint_unicode() {
        let ep = get_endpoint("o", "r", "重要");
        // Each Chinese codepoint is 3 UTF-8 bytes → 9 chars %XX%XX%XX each
        assert_eq!(ep, "/repos/o/r/labels/%E9%87%8D%E8%A6%81");
    }

    #[test]
    fn test_label_endpoint_slash_does_not_escape_segment() {
        // A label name with `/` must stay inside the labels segment —
        // otherwise it'd target a different endpoint entirely.
        let ep = get_endpoint("o", "r", "kind/bug");
        assert!(ep.contains("kind%2Fbug"), "/ must be encoded: {ep}");
        assert!(!ep.ends_with("kind/bug"));
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

    // --- Wire-path regression tests (issue #19) ---
    //
    // Mock-client tests above only check the response, not the URL.
    // These tests stand up a local HTTP server, run the real client
    // against it, and assert the actual path it requests — which is
    // what GitHub sees and what the bug was about.

    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path};

    #[tokio::test]
    async fn test_get_label_with_space_hits_percent_20_path() {
        // Issue #19: "priority: P0" GET must land on /labels/priority%3A%20P0
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/Meshlyai/meshly-backend/labels/priority%3A%20P0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "priority: P0"})))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = get(&client, "Meshlyai", "meshly-backend", "priority: P0").await;
        assert!(result.is_ok(), "request must reach the percent-encoded path: {result:?}");
    }

    #[tokio::test]
    async fn test_update_label_with_space_hits_percent_20_path() {
        // Issue #19 exact repro: PATCH /labels/priority%3A%20P0 with new_name body.
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/repos/Meshlyai/meshly-backend/labels/priority%3A%20P0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "priority:P0"})))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = update(
            &client,
            "Meshlyai",
            "meshly-backend",
            "priority: P0",
            Some("priority:P0"),
            None,
            None,
        )
        .await;
        assert!(result.is_ok(), "PATCH must reach the percent-encoded path: {result:?}");
    }

    #[tokio::test]
    async fn test_delete_label_with_space_hits_percent_20_path() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/repos/o/r/labels/help%20wanted"))
            .respond_with(ResponseTemplate::new(204).set_body_string(""))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = delete(&client, "o", "r", "help wanted").await;
        assert!(result.is_ok(), "DELETE must reach the percent-encoded path: {result:?}");
    }

    #[tokio::test]
    async fn test_label_with_form_plus_path_does_not_match() {
        // Negative test: prove the OLD (buggy) form-encoded path no longer matches.
        // If we ever regress to `+` for space, this test fails because the
        // wire path will hit the *only* mounted mock (the buggy one) and the
        // `expect(0)` assertion on the correct path will fail.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/labels/help%20wanted"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "help wanted"})))
            .expect(1)
            .mount(&server)
            .await;
        // Mount a mock for the *wrong* path and assert it's never hit.
        Mock::given(method("GET"))
            .and(path("/repos/o/r/labels/help+wanted"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .expect(0)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let _ = get(&client, "o", "r", "help wanted").await.unwrap();
        // Drop server → wiremock asserts expectations.
    }
}
