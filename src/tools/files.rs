use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// Get file or directory contents from a repository.
///
/// Returns file content (base64-encoded) and metadata, or directory listing.
pub async fn get_contents(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    path: &str,
    git_ref: Option<&str>,
) -> Result<Value, ClientError> {
    let encoded_path = crate::util::urlencode_path_multi(path);
    let mut endpoint = format!("/repos/{owner}/{repo}/contents/{encoded_path}");
    if let Some(r) = git_ref {
        endpoint.push_str(&format!("?ref={}", crate::util::urlencode(r)));
    }
    client.api(&endpoint, &[]).await
}

/// Create or update a file in a repository.
///
/// Uses PUT /repos/{owner}/{repo}/contents/{path}.
/// The `content` parameter **must** be base64-encoded by the caller,
/// matching the GitHub API contract.
/// If `sha` is provided, the file is updated (overwritten); otherwise it is created.
pub async fn create_or_update(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    path: &str,
    content: &str,
    message: &str,
    branch: &str,
    sha: Option<&str>,
) -> Result<Value, ClientError> {
    let encoded_path = crate::util::urlencode_path_multi(path);
    let endpoint = format!("/repos/{owner}/{repo}/contents/{encoded_path}");

    let content_field = format!("content={content}");
    let message_field = format!("message={message}");
    let branch_field = format!("branch={branch}");

    let mut args = vec!["-X", "PUT", "-f", &content_field, "-f", &message_field, "-f", &branch_field];

    let sha_field;
    if let Some(s) = sha {
        sha_field = format!("sha={s}");
        args.push("-f");
        args.push(&sha_field);
    }

    client.api(&endpoint, &args).await
}

/// Delete a file from a repository.
///
/// Requires the blob SHA of the file being deleted.
pub async fn delete(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    path: &str,
    message: &str,
    branch: &str,
    sha: &str,
) -> Result<Value, ClientError> {
    let encoded_path = crate::util::urlencode_path_multi(path);
    let endpoint = format!("/repos/{owner}/{repo}/contents/{encoded_path}");

    let message_field = format!("message={message}");
    let branch_field = format!("branch={branch}");
    let sha_field = format!("sha={sha}");

    let args = vec![
        "-X", "DELETE",
        "-f", &message_field,
        "-f", &branch_field,
        "-f", &sha_field,
    ];

    client.api(&endpoint, &args).await
}

/// Push multiple files to a repository in a single commit via the Git Data API.
///
/// **Not atomic**: uses a 6-step sequence. If any step after blob creation
/// fails, orphaned Git objects remain (GitHub GCs them after ~90 days).
/// The branch ref is only updated in the final step, so visible repo state
/// stays consistent on failure — but the operation cannot be rolled back.
///
/// Steps:
/// 1. GET ref → current commit SHA
/// 2. GET commit → current tree SHA
/// 3. POST blobs → blob SHAs for each file
/// 4. POST tree → new tree SHA
/// 5. POST commit → new commit SHA
/// 6. PATCH ref → update branch
///
/// `files_json` must be a JSON string containing an array of objects with
/// `path` and `content` fields. Content should be base64-encoded.
pub async fn push_files(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    branch: &str,
    message: &str,
    files_json: &str,
) -> Result<Value, ClientError> {
    let files: Vec<serde_json::Value> = serde_json::from_str(files_json)
        .map_err(|e| ClientError::Api(format!("invalid files_json: {e}")))?;

    if files.is_empty() {
        return Err(ClientError::Api("files_json array is empty".into()));
    }

    // Validate all files upfront
    for file in &files {
        file["path"]
            .as_str()
            .ok_or_else(|| ClientError::Api("each file must have a 'path' string".into()))?;
        file["content"]
            .as_str()
            .ok_or_else(|| ClientError::Api("each file must have a 'content' string".into()))?;
    }

    // Step 1: Get current commit SHA from branch ref.
    // Branch names can legitimately contain `/` (`feature/foo`) — preserve those
    // but percent-encode anything else that would break the URL.
    let encoded_branch = crate::util::urlencode_path_multi(branch);
    let ref_endpoint = format!("/repos/{owner}/{repo}/git/ref/heads/{encoded_branch}");
    let ref_data = client.api(&ref_endpoint, &[]).await
        .map_err(|e| ClientError::Api(format!("push_files step 1/6 (get ref): {e}")))?;
    let commit_sha = ref_data["object"]["sha"]
        .as_str()
        .ok_or_else(|| ClientError::Api("push_files step 1/6: could not get commit SHA from ref".into()))?;

    // Step 2: Get tree SHA from current commit
    let commit_endpoint = format!("/repos/{owner}/{repo}/git/commits/{commit_sha}");
    let commit_data = client.api(&commit_endpoint, &[]).await
        .map_err(|e| ClientError::Api(format!("push_files step 2/6 (get commit): {e}")))?;
    let base_tree_sha = commit_data["tree"]["sha"]
        .as_str()
        .ok_or_else(|| ClientError::Api("push_files step 2/6: could not get tree SHA from commit".into()))?;

    // Step 3: Create blobs for each file
    // NOTE: if a later step fails, these blobs become orphaned objects.
    // GitHub will GC them after ~90 days. No rollback is possible via the API.
    let mut tree_entries = Vec::new();
    let mut created_blob_count: usize = 0;
    for file in &files {
        let file_path = file["path"].as_str().unwrap(); // validated above
        let content = file["content"].as_str().unwrap(); // validated above

        let blob_endpoint = format!("/repos/{owner}/{repo}/git/blobs");
        let blob_body = serde_json::json!({
            "content": content,
            "encoding": "base64"
        });
        let blob_result = client.api_json(&blob_endpoint, "POST", &blob_body).await
            .map_err(|e| ClientError::Api(format!(
                "push_files step 3/6 (create blob for '{file_path}', {created_blob_count} prior blobs orphaned): {e}"
            )))?;
        let blob_sha = blob_result["sha"]
            .as_str()
            .ok_or_else(|| ClientError::Api(format!(
                "push_files step 3/6: could not get blob SHA for '{file_path}'"
            )))?;

        created_blob_count += 1;
        tree_entries.push(serde_json::json!({
            "path": file_path,
            "mode": "100644",
            "type": "blob",
            "sha": blob_sha
        }));
    }

    // Step 4: Create new tree
    let tree_endpoint = format!("/repos/{owner}/{repo}/git/trees");
    let tree_body = serde_json::json!({
        "base_tree": base_tree_sha,
        "tree": tree_entries
    });
    let tree_result = client.api_json(&tree_endpoint, "POST", &tree_body).await
        .map_err(|e| ClientError::Api(format!(
            "push_files step 4/6 (create tree, {} blobs orphaned): {e}",
            created_blob_count
        )))?;
    let new_tree_sha = tree_result["sha"]
        .as_str()
        .ok_or_else(|| ClientError::Api("push_files step 4/6: could not get new tree SHA".into()))?;

    // Step 5: Create new commit
    let new_commit_endpoint = format!("/repos/{owner}/{repo}/git/commits");
    let new_commit_body = serde_json::json!({
        "message": message,
        "tree": new_tree_sha,
        "parents": [commit_sha]
    });
    let new_commit_result = client
        .api_json(&new_commit_endpoint, "POST", &new_commit_body)
        .await
        .map_err(|e| ClientError::Api(format!(
            "push_files step 5/6 (create commit, tree+{} blobs orphaned): {e}",
            created_blob_count
        )))?;
    let new_commit_sha = new_commit_result["sha"]
        .as_str()
        .ok_or_else(|| ClientError::Api("push_files step 5/6: could not get new commit SHA".into()))?;

    // Step 6: Update branch ref to point to new commit
    let update_ref_endpoint = format!("/repos/{owner}/{repo}/git/refs/heads/{encoded_branch}");
    let update_ref_body = serde_json::json!({
        "sha": new_commit_sha
    });
    client
        .api_json(&update_ref_endpoint, "PATCH", &update_ref_body)
        .await
        .map_err(|e| ClientError::Api(format!(
            "push_files step 6/6 (update ref, commit+tree+{} blobs orphaned): {e}",
            created_blob_count
        )))?;

    Ok(serde_json::json!({
        "files_pushed": files.len(),
        "commit_sha": new_commit_sha,
    }))
}

#[cfg(test)]
fn get_contents_endpoint(owner: &str, repo: &str, path: &str, git_ref: Option<&str>) -> String {
    let encoded_path = crate::util::urlencode_path_multi(path);
    let mut endpoint = format!("/repos/{owner}/{repo}/contents/{encoded_path}");
    if let Some(r) = git_ref {
        endpoint.push_str(&format!("?ref={}", crate::util::urlencode(r)));
    }
    endpoint
}

#[cfg(test)]
fn contents_endpoint(owner: &str, repo: &str, path: &str) -> String {
    let encoded_path = crate::util::urlencode_path_multi(path);
    format!("/repos/{owner}/{repo}/contents/{encoded_path}")
}

#[cfg(test)]
fn create_or_update_args(content: &str, message: &str, branch: &str, sha: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "-X".into(), "PUT".into(),
        "-f".into(), format!("content={content}"),
        "-f".into(), format!("message={message}"),
        "-f".into(), format!("branch={branch}"),
    ];
    if let Some(s) = sha {
        args.push("-f".into());
        args.push(format!("sha={s}"));
    }
    args
}

#[cfg(test)]
fn delete_args(message: &str, branch: &str, sha: &str) -> Vec<String> {
    vec![
        "-X".into(), "DELETE".into(),
        "-f".into(), format!("message={message}"),
        "-f".into(), format!("branch={branch}"),
        "-f".into(), format!("sha={sha}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_get_contents_endpoint_no_ref() {
        assert_eq!(
            get_contents_endpoint("o", "r", "src/main.rs", None),
            "/repos/o/r/contents/src/main.rs"
        );
    }

    #[test]
    fn test_get_contents_endpoint_with_ref() {
        let ep = get_contents_endpoint("o", "r", "README.md", Some("v1.0"));
        assert_eq!(ep, "/repos/o/r/contents/README.md?ref=v1.0");
    }

    #[test]
    fn test_contents_endpoint() {
        assert_eq!(contents_endpoint("o", "r", "a/b.txt"), "/repos/o/r/contents/a/b.txt");
    }

    #[test]
    fn test_create_or_update_args_new_file() {
        let args = create_or_update_args("SGVsbG8=", "add file", "main", None);
        assert!(args.contains(&"-X".to_string()));
        assert!(args.contains(&"PUT".to_string()));
        assert!(args.contains(&"content=SGVsbG8=".to_string()));
        assert!(args.contains(&"message=add file".to_string()));
        assert!(args.contains(&"branch=main".to_string()));
        assert!(!args.iter().any(|a| a.starts_with("sha=")));
    }

    #[test]
    fn test_create_or_update_args_update_file() {
        let args = create_or_update_args("data", "update", "dev", Some("abc123"));
        assert!(args.contains(&"sha=abc123".to_string()));
    }

    #[test]
    fn test_delete_args() {
        let args = delete_args("remove old", "main", "def456");
        assert!(args.contains(&"-X".to_string()));
        assert!(args.contains(&"DELETE".to_string()));
        assert!(args.contains(&"message=remove old".to_string()));
        assert!(args.contains(&"branch=main".to_string()));
        assert!(args.contains(&"sha=def456".to_string()));
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_get_contents_fn() {
        let client = GithubClient::mock(vec![json!({"name": "lib.rs", "type": "file"})]);
        let result = get_contents(&client, "o", "r", "src/lib.rs", None).await.unwrap();
        assert_eq!(result["name"], "lib.rs");
    }

    #[tokio::test]
    async fn test_get_contents_with_ref() {
        let client = GithubClient::mock(vec![json!({"name": "lib.rs"})]);
        let result = get_contents(&client, "o", "r", "src/lib.rs", Some("v1.0")).await.unwrap();
        assert_eq!(result["name"], "lib.rs");
    }

    #[tokio::test]
    async fn test_create_or_update_fn() {
        let client = GithubClient::mock(vec![json!({"content": {"path": "f.txt"}})]);
        let result = create_or_update(&client, "o", "r", "f.txt", "SGVsbG8=", "add", "main", None).await.unwrap();
        assert!(result["content"].is_object());
    }

    #[tokio::test]
    async fn test_create_or_update_with_sha() {
        let client = GithubClient::mock(vec![json!({"content": {"path": "f.txt"}})]);
        let result = create_or_update(&client, "o", "r", "f.txt", "data", "update", "main", Some("abc123")).await.unwrap();
        assert!(result["content"].is_object());
    }

    #[tokio::test]
    async fn test_delete_fn() {
        let client = GithubClient::mock(vec![json!({"commit": {"sha": "abc"}})]);
        let result = delete(&client, "o", "r", "old.txt", "remove", "main", "sha123").await.unwrap();
        assert!(result["commit"].is_object());
    }

    #[tokio::test]
    async fn test_push_files_single_file() {
        // Git Data API flow: ref → commit → blob → tree → commit → update ref
        let client = GithubClient::mock(vec![
            json!({"object": {"sha": "commit111"}}),           // GET ref
            json!({"tree": {"sha": "tree111"}}),               // GET commit
            json!({"sha": "blob111"}),                          // POST blob
            json!({"sha": "newtree111"}),                       // POST tree
            json!({"sha": "newcommit111"}),                     // POST commit
            json!({"ref": "refs/heads/main"}),                  // PATCH ref
        ]);
        let files_json = r#"[{"path":"a.txt","content":"aGk="}]"#;
        let result = push_files(&client, "o", "r", "main", "push", files_json).await.unwrap();
        assert_eq!(result["files_pushed"], 1);
        assert_eq!(result["commit_sha"], "newcommit111");
    }

    #[tokio::test]
    async fn test_push_files_multiple_files() {
        // 2 files: ref → commit → blob1 → blob2 → tree → commit → update ref
        let client = GithubClient::mock(vec![
            json!({"object": {"sha": "commit222"}}),
            json!({"tree": {"sha": "tree222"}}),
            json!({"sha": "blobA"}),                            // blob for file 1
            json!({"sha": "blobB"}),                            // blob for file 2
            json!({"sha": "newtree222"}),
            json!({"sha": "newcommit222"}),
            json!({"ref": "refs/heads/main"}),
        ]);
        let files_json = r#"[{"path":"a.txt","content":"aGk="},{"path":"b.txt","content":"d29ybGQ="}]"#;
        let result = push_files(&client, "o", "r", "main", "add files", files_json).await.unwrap();
        assert_eq!(result["files_pushed"], 2);
        assert_eq!(result["commit_sha"], "newcommit222");
    }

    #[tokio::test]
    async fn test_push_files_invalid_json() {
        let client = GithubClient::mock(vec![]);
        let result = push_files(&client, "o", "r", "main", "push", "not json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_push_files_missing_path() {
        let client = GithubClient::mock(vec![]);
        let result = push_files(&client, "o", "r", "main", "push", r#"[{"content":"aGk="}]"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_push_files_missing_content() {
        let client = GithubClient::mock(vec![]);
        let result = push_files(&client, "o", "r", "main", "push", r#"[{"path":"a.txt"}]"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_push_files_empty_array() {
        let client = GithubClient::mock(vec![]);
        let result = push_files(&client, "o", "r", "main", "push", "[]").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }

    #[tokio::test]
    async fn test_push_files_ref_not_found() {
        use crate::github::client::ClientError;
        let client = GithubClient::mock_results(vec![
            Err(ClientError::Api("404: branch not found".into())),
        ]);
        let result = push_files(&client, "o", "r", "nope", "push",
            r#"[{"path":"a.txt","content":"aGk="}]"#).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("step 1/6"), "error should include step context: {err_msg}");
    }

    #[tokio::test]
    async fn test_push_files_tree_creation_fails_reports_orphaned_blobs() {
        use crate::github::client::ClientError;
        // Steps 1-3 succeed, step 4 (tree creation) fails
        let client = GithubClient::mock_results(vec![
            Ok(json!({"object": {"sha": "commit_abc"}})),   // step 1: GET ref
            Ok(json!({"tree": {"sha": "tree_abc"}})),        // step 2: GET commit
            Ok(json!({"sha": "blob_abc"})),                  // step 3: POST blob
            Err(ClientError::Api("422: tree creation failed".into())), // step 4: fails
        ]);
        let result = push_files(&client, "o", "r", "main", "push",
            r#"[{"path":"a.txt","content":"aGk="}]"#).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("step 4/6"), "error should include step: {err_msg}");
        assert!(err_msg.contains("orphaned"), "error should mention orphaned objects: {err_msg}");
    }

    #[tokio::test]
    async fn test_push_files_commit_creation_fails_reports_tree_plus_blobs() {
        use crate::github::client::ClientError;
        // Steps 1-4 succeed, step 5 (commit creation) fails
        let client = GithubClient::mock_results(vec![
            Ok(json!({"object": {"sha": "commit_abc"}})),    // step 1: GET ref
            Ok(json!({"tree": {"sha": "tree_abc"}})),         // step 2: GET commit
            Ok(json!({"sha": "blob_abc"})),                   // step 3: POST blob
            Ok(json!({"sha": "newtree_abc"})),                // step 4: POST tree
            Err(ClientError::Api("500: commit creation failed".into())), // step 5: fails
        ]);
        let result = push_files(&client, "o", "r", "main", "push",
            r#"[{"path":"a.txt","content":"aGk="}]"#).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("step 5/6"), "error should include step: {err_msg}");
        assert!(err_msg.contains("tree+"), "error should mention tree orphaned: {err_msg}");
    }

    // --- Wire-path encoding regression tests ---

    use wiremock::{Mock, MockServer, ResponseTemplate};
    use wiremock::matchers::{method, path};

    #[tokio::test]
    async fn test_get_contents_path_with_space_is_percent_encoded() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/contents/src/foo%20bar.rs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "foo bar.rs"})))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = get_contents(&client, "o", "r", "src/foo bar.rs", None).await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn test_get_contents_preserves_path_separators() {
        // Slashes inside `path` must NOT be encoded — they're real path segments.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/contents/a/b/c.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "c.txt"})))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = get_contents(&client, "o", "r", "a/b/c.txt", None).await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn test_push_files_branch_with_space_in_ref_endpoint() {
        // Step 1 hits /git/ref/heads/{branch} — branch with space must be %20.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/ref/heads/feature/my%20fix"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object": {"sha": "abc"}})))
            .expect(1)
            .mount(&server)
            .await;
        // We only test step 1 reaches the right path; subsequent steps would
        // need more mocks but are not what we're verifying here.
        Mock::given(method("GET"))
            .and(path("/repos/o/r/git/commits/abc"))
            .respond_with(ResponseTemplate::new(404).set_body_string(""))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let _ = push_files(
            &client,
            "o",
            "r",
            "feature/my fix",
            "msg",
            r#"[{"path":"a.txt","content":"aGk="}]"#,
        )
        .await;
        // Server.drop() asserts the step-1 mock was hit exactly once.
    }

    #[tokio::test]
    async fn test_push_files_ref_update_fails_reports_full_orphan_chain() {
        use crate::github::client::ClientError;
        // Steps 1-5 succeed, step 6 (ref update) fails
        let client = GithubClient::mock_results(vec![
            Ok(json!({"object": {"sha": "commit_abc"}})),    // step 1
            Ok(json!({"tree": {"sha": "tree_abc"}})),         // step 2
            Ok(json!({"sha": "blob_abc"})),                   // step 3
            Ok(json!({"sha": "newtree_abc"})),                // step 4
            Ok(json!({"sha": "newcommit_abc"})),              // step 5
            Err(ClientError::Api("409: ref update conflict".into())), // step 6 fails
        ]);
        let result = push_files(&client, "o", "r", "main", "push",
            r#"[{"path":"a.txt","content":"aGk="}]"#).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("step 6/6"), "error should include step: {err_msg}");
        assert!(err_msg.contains("commit+tree"), "error should mention commit+tree orphaned: {err_msg}");
    }
}
