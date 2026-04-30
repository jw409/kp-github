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

    crate::tools::search_util::extract_search_items(&result, limit)
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
    include_patches: bool,
) -> Result<Value, ClientError> {
    // base/head can be SHAs, branches (`feature/foo`), or refspecs —
    // preserve `/` but encode anything else that would break the URL.
    let encoded_base = crate::util::urlencode_path_multi(base);
    let encoded_head = crate::util::urlencode_path_multi(head);
    let endpoint = format!("/repos/{owner}/{repo}/compare/{encoded_base}...{encoded_head}");
    let mut result = client.api(&endpoint, &[]).await?;
    if !include_patches {
        // Same shape as commit detail: response.files[*].patch can be huge.
        crate::tools::commits::strip_patches(&mut result);
    }
    Ok(result)
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

#[cfg(test)]
fn get_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}")
}

#[cfg(test)]
fn create_endpoint(org: Option<&str>) -> String {
    match org {
        Some(o) => format!("/orgs/{o}/repos"),
        None => "/user/repos".to_string(),
    }
}

#[cfg(test)]
fn compare_endpoint(owner: &str, repo: &str, base: &str, head: &str) -> String {
    let encoded_base = crate::util::urlencode_path_multi(base);
    let encoded_head = crate::util::urlencode_path_multi(head);
    format!("/repos/{owner}/{repo}/compare/{encoded_base}...{encoded_head}")
}

#[cfg(test)]
fn fork_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/forks")
}

#[cfg(test)]
fn search_url(query: &str, per_page: u32) -> String {
    format!("/search/repositories?q={}&per_page={per_page}", crate::util::urlencode(query))
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

    // search_util tests are in search_util.rs — only async integration tests here

    #[tokio::test]
    async fn test_search_repos() {
        let client = GithubClient::mock(vec![json!({"total_count": 1, "items": [{"full_name": "a/b"}]})]);
        let result = search(&client, "language:rust", Some(5)).await.unwrap();
        assert!(result["items"].is_array());
        assert_eq!(result["total_count"], 1);
    }

    #[tokio::test]
    async fn test_search_repos_no_limit() {
        let client = GithubClient::mock(vec![json!({"total_count": 2, "items": [{"id": 1}, {"id": 2}]})]);
        let result = search(&client, "stars:>100", None).await.unwrap();
        assert_eq!(result["items"].as_array().unwrap().len(), 2);
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
        let result = compare(&client, "o", "r", "main", "feat", false).await.unwrap();
        assert_eq!(result["status"], "ahead");
    }

    #[tokio::test]
    async fn test_compare_strips_patches_by_default() {
        let client = GithubClient::mock(vec![json!({
            "status": "ahead",
            "files": [
                {"filename": "a.rs", "patch": "@@ huge @@"},
                {"filename": "b.rs", "patch": "@@ another @@"},
            ],
        })]);
        let result = compare(&client, "o", "r", "main", "feat", false).await.unwrap();
        for f in result["files"].as_array().unwrap() {
            assert!(f.get("patch").is_none());
            assert!(f.get("filename").is_some());
        }
    }

    #[tokio::test]
    async fn test_compare_keeps_patches_when_requested() {
        let client = GithubClient::mock(vec![json!({
            "status": "ahead",
            "files": [{"filename": "a.rs", "patch": "@@ keep @@"}],
        })]);
        let result = compare(&client, "o", "r", "main", "feat", true).await.unwrap();
        assert_eq!(result["files"][0]["patch"], "@@ keep @@");
    }

    #[test]
    fn test_compare_endpoint_with_slash_in_ref() {
        // Branch refs like `feature/foo` must keep their `/` literal,
        // otherwise GitHub can't resolve them.
        let ep = compare_endpoint("o", "r", "main", "feature/my-fix");
        assert_eq!(ep, "/repos/o/r/compare/main...feature/my-fix");
    }

    #[test]
    fn test_compare_endpoint_with_caret_and_tilde() {
        // git-style ancestry ops (`HEAD^`, `main~3`): `~` is unreserved (passes
        // through), `^` is reserved (must be percent-encoded).
        let ep = compare_endpoint("o", "r", "main~3", "HEAD^");
        assert_eq!(ep, "/repos/o/r/compare/main~3...HEAD%5E");
    }

    #[tokio::test]
    async fn test_compare_wire_path_with_slash_in_branch() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        use wiremock::matchers::{method, path};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/compare/main...feature/my-fix"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ahead"})))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = compare(&client, "o", "r", "main", "feature/my-fix", false).await;
        assert!(result.is_ok(), "{result:?}");
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
