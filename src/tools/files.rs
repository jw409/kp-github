use serde_json::{json, Map, Value};

use crate::github::client::{ClientError, GithubClient};

/// Override commit identity for create/update/delete/push_files.
///
/// Both `name` and `email` of a side (author or committer) must be set together;
/// passing only one is a caller error and returns [`ClientError::Api`]. If both
/// `author` and `committer` are `None` GitHub uses the OAuth user's identity,
/// which historically caused commits to be attributed to the token owner rather
/// than the intended bot/user.
#[derive(Debug, Default, Clone)]
pub struct CommitIdentity<'a> {
    pub author_name: Option<&'a str>,
    pub author_email: Option<&'a str>,
    pub committer_name: Option<&'a str>,
    pub committer_email: Option<&'a str>,
}

impl<'a> CommitIdentity<'a> {
    /// Returns `(author_obj, committer_obj)` ready to inject into a request body.
    ///
    /// Each side is `Some(obj)` only when both name+email are provided; an
    /// asymmetric pair (name without email or vice versa) is rejected as an
    /// error rather than silently dropped.
    fn resolve(&self) -> Result<(Option<Value>, Option<Value>), ClientError> {
        let author = match (self.author_name, self.author_email) {
            (Some(n), Some(e)) => Some(json!({ "name": n, "email": e })),
            (None, None) => None,
            _ => return Err(ClientError::Api(
                "author requires both author_name and author_email (or omit both)".into(),
            )),
        };
        let committer = match (self.committer_name, self.committer_email) {
            (Some(n), Some(e)) => Some(json!({ "name": n, "email": e })),
            (None, None) => None,
            _ => return Err(ClientError::Api(
                "committer requires both committer_name and committer_email (or omit both)".into(),
            )),
        };
        Ok((author, committer))
    }

    /// Inject the resolved `author` / `committer` JSON objects into a body map
    /// at the top level. No-op if both are unset.
    fn apply(&self, body: &mut Map<String, Value>) -> Result<(), ClientError> {
        let (author, committer) = self.resolve()?;
        if let Some(a) = author {
            body.insert("author".into(), a);
        }
        if let Some(c) = committer {
            body.insert("committer".into(), c);
        }
        Ok(())
    }
}

/// Owned commit identity built by layering caller-supplied args over env vars.
///
/// The MCP adapter constructs this once per commit-creating tool call:
///   * Per-field precedence: caller arg → env var → unset.
///   * Env vars read: `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`,
///     `GIT_COMMITTER_NAME`, `GIT_COMMITTER_EMAIL`.
///   * Each side (author/committer) must be both-set or both-unset; a half-set
///     pair (after the env layer) is rejected as a caller error.
///   * Cascade: if the committer side ends up fully unset *and* the author
///     side is fully set, committer mirrors author. This means a single
///     `GIT_AUTHOR_*` env pair attributes both sides correctly instead of
///     leaving the committer to silently fall back to the OAuth user.
///   * `KP_GITHUB_REQUIRE_AUTHOR=1` (or `true`/`yes`) turns "no author
///     resolved" into a hard error, so a misconfigured deployment fails loud
///     instead of stamping commits with the token owner's identity.
///
/// Borrow as a `CommitIdentity<'_>` via `as_borrowed()` to pass into the
/// commit-creating functions.
#[derive(Debug, Default, Clone)]
pub struct OwnedCommitIdentity {
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub committer_name: Option<String>,
    pub committer_email: Option<String>,
}

impl OwnedCommitIdentity {
    /// Resolve using the process environment. See struct docs for layering rules.
    pub fn resolve(
        author_name: Option<String>,
        author_email: Option<String>,
        committer_name: Option<String>,
        committer_email: Option<String>,
    ) -> Result<Self, ClientError> {
        Self::resolve_with_env(
            author_name,
            author_email,
            committer_name,
            committer_email,
            |k| std::env::var(k).ok(),
        )
    }

    /// Test-friendly resolution: caller supplies the env lookup closure so unit
    /// tests can stub env without touching the real process environment (which
    /// would race against other tests).
    pub fn resolve_with_env<F: Fn(&str) -> Option<String>>(
        author_name: Option<String>,
        author_email: Option<String>,
        committer_name: Option<String>,
        committer_email: Option<String>,
        env: F,
    ) -> Result<Self, ClientError> {
        // Layer 1: caller arg, else env, per individual field.
        // Treats empty strings as unset so an exported-but-empty env var
        // doesn't accidentally produce `{"name": "", "email": ""}`. We also
        // treat empty *caller* strings as unset for the same reason — a JSON
        // `""` is almost certainly a mistake, not a request for an anonymous
        // commit.
        let env_some = |k: &str| env(k).filter(|v| !v.is_empty());
        let arg = |v: Option<String>| v.filter(|s| !s.is_empty());
        let an = arg(author_name).or_else(|| env_some("GIT_AUTHOR_NAME"));
        let ae = arg(author_email).or_else(|| env_some("GIT_AUTHOR_EMAIL"));
        let mut cn = arg(committer_name).or_else(|| env_some("GIT_COMMITTER_NAME"));
        let mut ce = arg(committer_email).or_else(|| env_some("GIT_COMMITTER_EMAIL"));

        // Layer 2: each side must be both-set or both-unset. Validate BEFORE
        // cascading so a half-set author doesn't poison the committer side.
        if an.is_some() != ae.is_some() {
            return Err(ClientError::Api(
                "author requires both author_name and author_email (or omit both, \
                 including any GIT_AUTHOR_NAME / GIT_AUTHOR_EMAIL env vars)"
                    .into(),
            ));
        }
        if cn.is_some() != ce.is_some() {
            return Err(ClientError::Api(
                "committer requires both committer_name and committer_email (or omit both, \
                 including any GIT_COMMITTER_NAME / GIT_COMMITTER_EMAIL env vars)"
                    .into(),
            ));
        }

        // Layer 3: cascade author -> committer when committer is fully unset.
        // Without this, setting only GIT_AUTHOR_* leaves the committer slot
        // unset and GitHub stamps it with the OAuth user — re-introducing the
        // exact "wrong-name in commit metadata" bug this resolver exists to fix.
        if cn.is_none() && ce.is_none() && an.is_some() {
            cn = an.clone();
            ce = ae.clone();
        }

        // Layer 4: hard-refusal flag for deployments that want to guarantee
        // no commit ever falls back to the OAuth user.
        let require = env("KP_GITHUB_REQUIRE_AUTHOR")
            .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES" | "True" | "Yes"))
            .unwrap_or(false);
        if require && an.is_none() {
            return Err(ClientError::Api(
                "KP_GITHUB_REQUIRE_AUTHOR is set but no commit author resolved. \
                 Pass author_name+author_email as tool args, or set GIT_AUTHOR_NAME \
                 and GIT_AUTHOR_EMAIL in the MCP server's env."
                    .into(),
            ));
        }

        Ok(Self {
            author_name: an,
            author_email: ae,
            committer_name: cn,
            committer_email: ce,
        })
    }

    /// Borrow as a `CommitIdentity<'_>` suitable for passing into the commit-
    /// creating functions in this module.
    pub fn as_borrowed(&self) -> CommitIdentity<'_> {
        CommitIdentity {
            author_name: self.author_name.as_deref(),
            author_email: self.author_email.as_deref(),
            committer_name: self.committer_name.as_deref(),
            committer_email: self.committer_email.as_deref(),
        }
    }
}

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
///
/// `identity` overrides the commit's author/committer; omit to fall back to the
/// OAuth user.
pub async fn create_or_update(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    path: &str,
    content: &str,
    message: &str,
    branch: &str,
    sha: Option<&str>,
    identity: &CommitIdentity<'_>,
) -> Result<Value, ClientError> {
    let encoded_path = crate::util::urlencode_path_multi(path);
    let endpoint = format!("/repos/{owner}/{repo}/contents/{encoded_path}");

    let mut body = Map::new();
    body.insert("message".into(), Value::String(message.into()));
    body.insert("content".into(), Value::String(content.into()));
    body.insert("branch".into(), Value::String(branch.into()));
    if let Some(s) = sha {
        body.insert("sha".into(), Value::String(s.into()));
    }
    identity.apply(&mut body)?;

    client.api_json(&endpoint, "PUT", &Value::Object(body)).await
}

/// Delete a file from a repository.
///
/// Requires the blob SHA of the file being deleted.
///
/// `identity` overrides the commit's author/committer; omit to fall back to the
/// OAuth user.
pub async fn delete(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    path: &str,
    message: &str,
    branch: &str,
    sha: &str,
    identity: &CommitIdentity<'_>,
) -> Result<Value, ClientError> {
    let encoded_path = crate::util::urlencode_path_multi(path);
    let endpoint = format!("/repos/{owner}/{repo}/contents/{encoded_path}");

    let mut body = Map::new();
    body.insert("message".into(), Value::String(message.into()));
    body.insert("branch".into(), Value::String(branch.into()));
    body.insert("sha".into(), Value::String(sha.into()));
    identity.apply(&mut body)?;

    client.api_json(&endpoint, "DELETE", &Value::Object(body)).await
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
    identity: &CommitIdentity<'_>,
) -> Result<Value, ClientError> {
    // Resolve identity up-front so a malformed pair fails before we touch GitHub.
    let (author_obj, committer_obj) = identity.resolve()?;

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
    let mut new_commit_body_map = Map::new();
    new_commit_body_map.insert("message".into(), Value::String(message.into()));
    new_commit_body_map.insert("tree".into(), Value::String(new_tree_sha.into()));
    new_commit_body_map.insert("parents".into(), json!([commit_sha]));
    if let Some(ref a) = author_obj {
        new_commit_body_map.insert("author".into(), a.clone());
    }
    if let Some(ref c) = committer_obj {
        new_commit_body_map.insert("committer".into(), c.clone());
    }
    let new_commit_body = Value::Object(new_commit_body_map);
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
        let result = create_or_update(
            &client, "o", "r", "f.txt", "SGVsbG8=", "add", "main", None,
            &CommitIdentity::default(),
        ).await.unwrap();
        assert!(result["content"].is_object());
    }

    #[tokio::test]
    async fn test_create_or_update_with_sha() {
        let client = GithubClient::mock(vec![json!({"content": {"path": "f.txt"}})]);
        let result = create_or_update(
            &client, "o", "r", "f.txt", "data", "update", "main", Some("abc123"),
            &CommitIdentity::default(),
        ).await.unwrap();
        assert!(result["content"].is_object());
    }

    #[tokio::test]
    async fn test_delete_fn() {
        let client = GithubClient::mock(vec![json!({"commit": {"sha": "abc"}})]);
        let result = delete(
            &client, "o", "r", "old.txt", "remove", "main", "sha123",
            &CommitIdentity::default(),
        ).await.unwrap();
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
        let result = push_files(&client, "o", "r", "main", "push", files_json, &CommitIdentity::default()).await.unwrap();
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
        let result = push_files(&client, "o", "r", "main", "add files", files_json, &CommitIdentity::default()).await.unwrap();
        assert_eq!(result["files_pushed"], 2);
        assert_eq!(result["commit_sha"], "newcommit222");
    }

    #[tokio::test]
    async fn test_push_files_invalid_json() {
        let client = GithubClient::mock(vec![]);
        let result = push_files(&client, "o", "r", "main", "push", "not json", &CommitIdentity::default()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_push_files_missing_path() {
        let client = GithubClient::mock(vec![]);
        let result = push_files(&client, "o", "r", "main", "push", r#"[{"content":"aGk="}]"#, &CommitIdentity::default()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_push_files_missing_content() {
        let client = GithubClient::mock(vec![]);
        let result = push_files(&client, "o", "r", "main", "push", r#"[{"path":"a.txt"}]"#, &CommitIdentity::default()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_push_files_empty_array() {
        let client = GithubClient::mock(vec![]);
        let result = push_files(&client, "o", "r", "main", "push", "[]", &CommitIdentity::default()).await;
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
            r#"[{"path":"a.txt","content":"aGk="}]"#, &CommitIdentity::default()).await;
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
            r#"[{"path":"a.txt","content":"aGk="}]"#, &CommitIdentity::default()).await;
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
            r#"[{"path":"a.txt","content":"aGk="}]"#, &CommitIdentity::default()).await;
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
            &CommitIdentity::default(),
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
            r#"[{"path":"a.txt","content":"aGk="}]"#, &CommitIdentity::default()).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("step 6/6"), "error should include step: {err_msg}");
        assert!(err_msg.contains("commit+tree"), "error should mention commit+tree orphaned: {err_msg}");
    }

    // --- CommitIdentity tests ---

    #[test]
    fn test_commit_identity_default_resolves_to_none() {
        let id = CommitIdentity::default();
        let (a, c) = id.resolve().unwrap();
        assert!(a.is_none());
        assert!(c.is_none());
    }

    #[test]
    fn test_commit_identity_full_pair_builds_objects() {
        let id = CommitIdentity {
            author_name: Some("jw"),
            author_email: Some("jw@example.com"),
            committer_name: Some("bot"),
            committer_email: Some("bot@example.com"),
        };
        let (a, c) = id.resolve().unwrap();
        assert_eq!(a, Some(json!({"name": "jw", "email": "jw@example.com"})));
        assert_eq!(c, Some(json!({"name": "bot", "email": "bot@example.com"})));
    }

    #[test]
    fn test_commit_identity_partial_pair_is_rejected() {
        // name without email → error, not silent drop
        let id = CommitIdentity { author_name: Some("jw"), ..Default::default() };
        let err = id.resolve().unwrap_err().to_string();
        assert!(err.contains("author_name") && err.contains("author_email"));

        let id = CommitIdentity { committer_email: Some("bot@x"), ..Default::default() };
        let err = id.resolve().unwrap_err().to_string();
        assert!(err.contains("committer_name") && err.contains("committer_email"));
    }

    #[test]
    fn test_commit_identity_apply_inserts_only_set_sides() {
        // Author set, committer omitted → body has author only
        let id = CommitIdentity {
            author_name: Some("jw"),
            author_email: Some("jw@x"),
            ..Default::default()
        };
        let mut body = Map::new();
        id.apply(&mut body).unwrap();
        assert!(body.contains_key("author"));
        assert!(!body.contains_key("committer"));
    }

    // --- Wire-body tests: prove author/committer reach the request body ---

    #[tokio::test]
    async fn test_create_or_update_wire_body_includes_author_and_committer() {
        use wiremock::matchers::{body_json, method, path};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/o/r/contents/f.txt"))
            .and(body_json(json!({
                "message": "add",
                "content": "SGVsbG8=",
                "branch": "main",
                "author": {"name": "jw", "email": "jw@example.com"},
                "committer": {"name": "bot", "email": "bot@example.com"}
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"content": {"path": "f.txt"}})))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let identity = CommitIdentity {
            author_name: Some("jw"),
            author_email: Some("jw@example.com"),
            committer_name: Some("bot"),
            committer_email: Some("bot@example.com"),
        };
        let result = create_or_update(
            &client, "o", "r", "f.txt", "SGVsbG8=", "add", "main", None, &identity,
        ).await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn test_create_or_update_wire_body_omits_identity_when_unset() {
        // When identity is default (all None), the body must NOT contain author/committer keys —
        // so GitHub falls back to the OAuth user as before.
        use wiremock::matchers::{body_json, method, path};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/o/r/contents/f.txt"))
            .and(body_json(json!({
                "message": "add",
                "content": "SGVsbG8=",
                "branch": "main"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"content": {"path": "f.txt"}})))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = create_or_update(
            &client, "o", "r", "f.txt", "SGVsbG8=", "add", "main", None,
            &CommitIdentity::default(),
        ).await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn test_delete_wire_body_includes_author() {
        use wiremock::matchers::{body_json, method, path};
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/repos/o/r/contents/old.txt"))
            .and(body_json(json!({
                "message": "rm",
                "branch": "main",
                "sha": "abc123",
                "author": {"name": "jw", "email": "jw@x"}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"commit": {"sha": "z"}})))
            .expect(1)
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let identity = CommitIdentity {
            author_name: Some("jw"),
            author_email: Some("jw@x"),
            ..Default::default()
        };
        let result = delete(&client, "o", "r", "old.txt", "rm", "main", "abc123", &identity).await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn test_create_or_update_partial_pair_returns_error_before_request() {
        // No mock mounted: if a request goes out, the test fails (server returns
        // its default 404). The partial-pair check must short-circuit first.
        let server = MockServer::start().await;
        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let identity = CommitIdentity {
            author_name: Some("jw"),  // email missing
            ..Default::default()
        };
        let err = create_or_update(
            &client, "o", "r", "f.txt", "SGVsbG8=", "add", "main", None, &identity,
        ).await.unwrap_err().to_string();
        assert!(err.contains("author_name") && err.contains("author_email"), "{err}");
    }

    // --- OwnedCommitIdentity (env-var resolver) tests ---
    //
    // All tests use `resolve_with_env` and a closure-based stub so they do not
    // mutate the real process environment (which would race against parallel
    // cargo-test threads).

    fn env_map<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        // Capture pairs by ref; closure returns owned String to match env::var.
        move |k: &str| pairs.iter().find(|(kk, _)| *kk == k).map(|(_, v)| (*v).to_string())
    }

    #[test]
    fn owned_identity_all_unset_resolves_to_all_none() {
        let id = OwnedCommitIdentity::resolve_with_env(None, None, None, None, env_map(&[])).unwrap();
        assert!(id.author_name.is_none() && id.author_email.is_none());
        assert!(id.committer_name.is_none() && id.committer_email.is_none());
    }

    #[test]
    fn owned_identity_caller_args_win_over_env() {
        // Caller passed "jw"; env says "bot". Caller must win.
        let id = OwnedCommitIdentity::resolve_with_env(
            Some("jw".into()), Some("jw@x".into()), None, None,
            env_map(&[("GIT_AUTHOR_NAME", "bot"), ("GIT_AUTHOR_EMAIL", "bot@x")]),
        ).unwrap();
        assert_eq!(id.author_name.as_deref(), Some("jw"));
        assert_eq!(id.author_email.as_deref(), Some("jw@x"));
    }

    #[test]
    fn owned_identity_env_fills_when_caller_unset() {
        let id = OwnedCommitIdentity::resolve_with_env(
            None, None, None, None,
            env_map(&[("GIT_AUTHOR_NAME", "jw"), ("GIT_AUTHOR_EMAIL", "jw@example.com")]),
        ).unwrap();
        assert_eq!(id.author_name.as_deref(), Some("jw"));
        assert_eq!(id.author_email.as_deref(), Some("jw@example.com"));
        // committer cascades from author
        assert_eq!(id.committer_name.as_deref(), Some("jw"));
        assert_eq!(id.committer_email.as_deref(), Some("jw@example.com"));
    }

    #[test]
    fn owned_identity_env_fills_individual_missing_field() {
        // Caller passed name; env supplies email. Resolver should complete the pair.
        let id = OwnedCommitIdentity::resolve_with_env(
            Some("jw".into()), None, None, None,
            env_map(&[("GIT_AUTHOR_EMAIL", "jw@example.com")]),
        ).unwrap();
        assert_eq!(id.author_name.as_deref(), Some("jw"));
        assert_eq!(id.author_email.as_deref(), Some("jw@example.com"));
    }

    #[test]
    fn owned_identity_committer_env_overrides_cascade() {
        // When committer env is set, it must NOT be cascaded over by author.
        let id = OwnedCommitIdentity::resolve_with_env(
            None, None, None, None,
            env_map(&[
                ("GIT_AUTHOR_NAME", "jw"), ("GIT_AUTHOR_EMAIL", "jw@x"),
                ("GIT_COMMITTER_NAME", "bot"), ("GIT_COMMITTER_EMAIL", "bot@x"),
            ]),
        ).unwrap();
        assert_eq!(id.author_name.as_deref(), Some("jw"));
        assert_eq!(id.committer_name.as_deref(), Some("bot"));
        assert_eq!(id.committer_email.as_deref(), Some("bot@x"));
    }

    #[test]
    fn owned_identity_half_set_author_errors_even_with_no_env() {
        let err = OwnedCommitIdentity::resolve_with_env(
            Some("jw".into()), None, None, None, env_map(&[]),
        ).unwrap_err().to_string();
        assert!(err.contains("author_name") && err.contains("author_email"), "{err}");
    }

    #[test]
    fn owned_identity_half_set_committer_after_env_errors() {
        // Caller passes committer_email; env provides nothing else.
        let err = OwnedCommitIdentity::resolve_with_env(
            None, None, None, Some("bot@x".into()), env_map(&[]),
        ).unwrap_err().to_string();
        assert!(err.contains("committer_name") && err.contains("committer_email"), "{err}");
    }

    #[test]
    fn owned_identity_empty_env_string_is_treated_as_unset() {
        // An exported-but-empty env var must not produce {"name":"", "email":""}.
        let id = OwnedCommitIdentity::resolve_with_env(
            None, None, None, None,
            env_map(&[("GIT_AUTHOR_NAME", ""), ("GIT_AUTHOR_EMAIL", "")]),
        ).unwrap();
        assert!(id.author_name.is_none());
        assert!(id.author_email.is_none());
    }

    #[test]
    fn owned_identity_require_flag_errors_when_no_author() {
        let err = OwnedCommitIdentity::resolve_with_env(
            None, None, None, None,
            env_map(&[("KP_GITHUB_REQUIRE_AUTHOR", "1")]),
        ).unwrap_err().to_string();
        assert!(err.contains("KP_GITHUB_REQUIRE_AUTHOR"), "{err}");
        assert!(err.contains("GIT_AUTHOR"), "{err}");
    }

    #[test]
    fn owned_identity_require_flag_passes_when_env_set() {
        let id = OwnedCommitIdentity::resolve_with_env(
            None, None, None, None,
            env_map(&[
                ("KP_GITHUB_REQUIRE_AUTHOR", "true"),
                ("GIT_AUTHOR_NAME", "jw"), ("GIT_AUTHOR_EMAIL", "jw@x"),
            ]),
        ).unwrap();
        assert_eq!(id.author_name.as_deref(), Some("jw"));
    }

    #[test]
    fn owned_identity_require_flag_passes_when_caller_set() {
        let id = OwnedCommitIdentity::resolve_with_env(
            Some("jw".into()), Some("jw@x".into()), None, None,
            env_map(&[("KP_GITHUB_REQUIRE_AUTHOR", "yes")]),
        ).unwrap();
        assert_eq!(id.author_name.as_deref(), Some("jw"));
    }

    #[test]
    fn owned_identity_require_flag_ignored_when_falsey() {
        // "0", "false", "no" must NOT enforce the requirement.
        for v in ["0", "false", "no", "off", ""] {
            let id = OwnedCommitIdentity::resolve_with_env(
                None, None, None, None,
                env_map(&[("KP_GITHUB_REQUIRE_AUTHOR", v)]),
            );
            assert!(id.is_ok(), "value {v:?} should not enforce REQUIRE_AUTHOR");
        }
    }

    #[test]
    fn owned_identity_as_borrowed_matches_owned_fields() {
        let id = OwnedCommitIdentity {
            author_name: Some("jw".into()),
            author_email: Some("jw@x".into()),
            committer_name: Some("bot".into()),
            committer_email: Some("bot@x".into()),
        };
        let b = id.as_borrowed();
        assert_eq!(b.author_name, Some("jw"));
        assert_eq!(b.author_email, Some("jw@x"));
        assert_eq!(b.committer_name, Some("bot"));
        assert_eq!(b.committer_email, Some("bot@x"));
    }

    #[tokio::test]
    async fn owned_identity_end_to_end_env_fill_reaches_wire() {
        // Compose the resolver + create_or_update path: caller passes no author
        // args, env supplies them, and the resulting GitHub PUT must carry the
        // env-derived author + cascaded committer in the body.
        use wiremock::matchers::{body_json, method, path};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/o/r/contents/f.txt"))
            .and(body_json(json!({
                "message": "add",
                "content": "SGVsbG8=",
                "branch": "main",
                "author": {"name": "jw", "email": "jw@example.com"},
                "committer": {"name": "jw", "email": "jw@example.com"}
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"content": {"path": "f.txt"}})))
            .expect(1)
            .mount(&server)
            .await;

        let identity = OwnedCommitIdentity::resolve_with_env(
            None, None, None, None,
            env_map(&[("GIT_AUTHOR_NAME", "jw"), ("GIT_AUTHOR_EMAIL", "jw@example.com")]),
        ).unwrap();
        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = create_or_update(
            &client, "o", "r", "f.txt", "SGVsbG8=", "add", "main", None,
            &identity.as_borrowed(),
        ).await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn test_push_files_wire_body_step5_includes_author_and_committer() {
        // Verify step 5 (POST /git/commits) carries the author/committer overrides.
        use wiremock::matchers::{body_json, method, path};
        let server = MockServer::start().await;

        // Steps 1-4 succeed (we only inspect step 5's body).
        Mock::given(method("GET")).and(path("/repos/o/r/git/ref/heads/main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object": {"sha": "C1"}})))
            .mount(&server).await;
        Mock::given(method("GET")).and(path("/repos/o/r/git/commits/C1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tree": {"sha": "T1"}})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/repos/o/r/git/blobs"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "B1"})))
            .mount(&server).await;
        Mock::given(method("POST")).and(path("/repos/o/r/git/trees"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "T2"})))
            .mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/commits"))
            .and(body_json(json!({
                "message": "msg",
                "tree": "T2",
                "parents": ["C1"],
                "author": {"name": "jw", "email": "jw@x"},
                "committer": {"name": "bot", "email": "bot@x"}
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"sha": "C2"})))
            .expect(1)
            .mount(&server).await;
        Mock::given(method("PATCH")).and(path("/repos/o/r/git/refs/heads/main"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ref": "refs/heads/main"})))
            .mount(&server).await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let identity = CommitIdentity {
            author_name: Some("jw"), author_email: Some("jw@x"),
            committer_name: Some("bot"), committer_email: Some("bot@x"),
        };
        let result = push_files(
            &client, "o", "r", "main", "msg",
            r#"[{"path":"a.txt","content":"aGk="}]"#,
            &identity,
        ).await;
        assert!(result.is_ok(), "{result:?}");
    }
}
