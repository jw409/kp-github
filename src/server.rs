use chrono::Utc;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, ListResourceTemplatesResult, ReadResourceRequestParams,
    ReadResourceResult, ResourceContents, ServerInfo,
};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;

use crate::compress::{compress, CompressConfig, OutputFormat};
use crate::format::format_output;
use crate::github::client::GithubClient;
use crate::tools;

pub struct KpGithubServer {
    client: Arc<GithubClient>,
    tool_router: ToolRouter<Self>,
}

impl KpGithubServer {
    fn new(token: &str) -> Self {
        Self {
            client: Arc::new(GithubClient::new(token)),
            tool_router: Self::tool_router(),
        }
    }

    #[cfg(test)]
    fn with_client(client: GithubClient) -> Self {
        Self {
            client: Arc::new(client),
            tool_router: Self::tool_router(),
        }
    }

    /// Compress search results: compress the items array but preserve total_count/truncated metadata.
    fn compress_and_format_search(&self, value: Value, fields: Option<Vec<String>>, format: Option<String>) -> String {
        // If this is a search wrapper {items, total_count, truncated}, extract items for compression
        if let Value::Object(ref map) = value {
            if let Some(items) = map.get("items") {
                let compressed_items = self.compress_and_format(items.clone(), fields, format);
                let total = map.get("total_count").and_then(|v| v.as_u64()).unwrap_or(0);
                let truncated = map.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
                if truncated {
                    return format!("{compressed_items}\n\n[{total} total results, showing first batch]");
                }
                return compressed_items;
            }
        }
        // Fallback: not a search wrapper
        self.compress_and_format(value, fields, format)
    }

    fn compress_and_format(&self, value: Value, fields: Option<Vec<String>>, format: Option<String>) -> String {
        let fmt = format
            .as_deref()
            .map(|s| match s {
                "json" => OutputFormat::Json,
                "table" => OutputFormat::Table,
                "text" => OutputFormat::Text,
                _ => OutputFormat::Auto,
            })
            .unwrap_or(OutputFormat::Auto);

        let config = CompressConfig {
            fields,
            format: fmt,
            ..CompressConfig::default()
        };

        let compressed = compress(&value, &config, Utc::now());
        format_output(&compressed, config.format)
    }
}

// --- Parameter structs ---

#[derive(Debug, Deserialize, JsonSchema)]
#[allow(dead_code)] // Used in tests
pub struct RepoParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssuesListParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Filter by state: open, closed, all
    #[serde(default)]
    pub state: Option<String>,
    /// Maximum number of results
    #[serde(default)]
    pub limit: Option<u32>,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueGetParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Issue number
    pub number: u32,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchParams {
    /// Search query (GitHub search syntax)
    pub query: String,
    /// Maximum number of results
    #[serde(default)]
    pub limit: Option<u32>,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueCreateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Issue title
    pub title: String,
    /// Issue body
    #[serde(default)]
    pub body: Option<String>,
    /// Labels to apply
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Assignees
    #[serde(default)]
    pub assignees: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueUpdateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Issue number
    pub number: u32,
    /// New title
    #[serde(default)]
    pub title: Option<String>,
    /// New body
    #[serde(default)]
    pub body: Option<String>,
    /// New state: open, closed
    #[serde(default)]
    pub state: Option<String>,
    /// Labels to set
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Assignees to set
    #[serde(default)]
    pub assignees: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueCommentParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Issue number
    pub number: u32,
    /// Comment body
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrGetParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrDiffParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FieldsOnlyParams {
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommitsListParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Branch name or SHA
    #[serde(default)]
    pub sha: Option<String>,
    /// Maximum number of results
    #[serde(default)]
    pub limit: Option<u32>,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RepoFieldsParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RepoLimitParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Maximum number of results
    #[serde(default)]
    pub limit: Option<u32>,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

// --- New parameter structs for missing tools ---

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueCommentsParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Issue number
    pub number: u32,
    /// Maximum number of comments to return
    #[serde(default)]
    pub limit: Option<u32>,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueNumberFieldsParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Issue number
    pub number: u32,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrCreateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR title
    pub title: String,
    /// Branch containing changes
    pub head: String,
    /// Branch to merge into
    pub base: String,
    /// PR body/description
    #[serde(default)]
    pub body: Option<String>,
    /// Create as draft PR
    #[serde(default)]
    pub draft: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrUpdateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
    /// New title
    #[serde(default)]
    pub title: Option<String>,
    /// New body
    #[serde(default)]
    pub body: Option<String>,
    /// New state: open, closed
    #[serde(default)]
    pub state: Option<String>,
    /// New base branch
    #[serde(default)]
    pub base: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrMergeParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
    /// Merge method: merge, squash, rebase
    #[serde(default)]
    pub merge_method: Option<String>,
    /// Title for the merge commit
    #[serde(default)]
    pub commit_title: Option<String>,
    /// Message for the merge commit
    #[serde(default)]
    pub commit_message: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrReviewCreateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
    /// Review event: APPROVE, REQUEST_CHANGES, COMMENT
    pub event: String,
    /// Review body/comment text
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileGetParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Path to file or directory
    pub path: String,
    /// Git ref (branch, tag, or SHA)
    #[serde(default)]
    pub git_ref: Option<String>,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileCreateOrUpdateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Path to file
    pub path: String,
    /// File content (must be base64-encoded)
    pub content: String,
    /// Commit message
    pub message: String,
    /// Branch to create/update the file in
    pub branch: String,
    /// Blob SHA of existing file (required for updates, omit for new files)
    #[serde(default)]
    pub sha: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileDeleteParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Path to the file to delete
    pub path: String,
    /// Commit message
    pub message: String,
    /// Branch to delete from
    pub branch: String,
    /// Blob SHA of the file being deleted
    pub sha: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RepoCreateParams {
    /// Repository name
    pub name: String,
    /// Repository description
    #[serde(default)]
    pub description: Option<String>,
    /// Whether the repo should be private
    #[serde(default)]
    pub private: Option<bool>,
    /// Organization to create the repo in (omit for personal account)
    #[serde(default)]
    pub org: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RepoForkParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Organization to fork into (omit for personal account)
    #[serde(default)]
    pub org: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BranchCreateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Name for the new branch
    pub branch: String,
    /// SHA to create the branch from
    pub from_sha: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommitGetParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Commit SHA
    pub sha: String,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReleaseByTagParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Tag name (e.g. 'v1.0.0')
    pub tag: String,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TagGetParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Tag name
    pub tag: String,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TeamMembersParams {
    /// Organization name
    pub org: String,
    /// Team slug
    pub team_slug: String,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

// --- New param structs for added tools ---

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IssueSubIssuesParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Issue number
    pub number: u32,
    /// Maximum number of results
    #[serde(default)]
    pub limit: Option<u32>,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OrgOwnerParams {
    /// Organization owner
    pub owner: String,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrReviewCommentParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
    /// Relative file path
    pub path: String,
    /// Comment body
    pub body: String,
    /// Line number in the diff to comment on
    #[serde(default)]
    pub line: Option<u32>,
    /// Side of the diff: LEFT or RIGHT
    #[serde(default)]
    pub side: Option<String>,
    /// Subject type: FILE or LINE
    #[serde(default)]
    pub subject_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrReplyCommentParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
    /// ID of the comment to reply to
    pub comment_id: u64,
    /// Reply body
    pub body: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrSubmitReviewParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
    /// Review ID to submit
    pub review_id: u64,
    /// Review event: APPROVE, REQUEST_CHANGES, COMMENT
    pub event: String,
    /// Optional body text
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PrDeleteReviewParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// PR number
    pub number: u32,
    /// Review ID to delete
    pub review_id: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FilePushParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Branch to push to
    pub branch: String,
    /// Commit message
    pub message: String,
    /// JSON string: array of {path, content} objects. Content should be base64-encoded.
    pub files_json: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelGetParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Label name
    pub name: String,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelCreateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Label name
    pub name: String,
    /// Label color (hex without #)
    #[serde(default)]
    pub color: Option<String>,
    /// Label description
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelUpdateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Current label name
    pub name: String,
    /// New label name
    #[serde(default)]
    pub new_name: Option<String>,
    /// New color (hex without #)
    #[serde(default)]
    pub color: Option<String>,
    /// New description
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelDeleteParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Label name to delete
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActionRunIdParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Workflow run ID
    pub run_id: u64,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActionRunIdOnlyParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Workflow run ID
    pub run_id: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompareParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Base branch/tag/SHA
    pub base: String,
    /// Head branch/tag/SHA
    pub head: String,
    /// Fields to include in output
    #[serde(default)]
    pub fields: Option<Vec<String>>,
    /// Output format: json, table, text
    #[serde(default)]
    pub format: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReleaseCreateParams {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Tag name for the release
    pub tag_name: String,
    /// Release name/title
    #[serde(default)]
    pub name: Option<String>,
    /// Release body/description
    #[serde(default)]
    pub body: Option<String>,
    /// Create as draft
    #[serde(default)]
    pub draft: Option<bool>,
    /// Mark as prerelease
    #[serde(default)]
    pub prerelease: Option<bool>,
}

// --- Tool implementations ---

#[rmcp::tool_router]
impl KpGithubServer {
    // ==================== Issues ====================

    /// List issues in a GitHub repository with token-compressed output
    #[rmcp::tool(name = "github_issues_list")]
    async fn github_issues_list(&self, Parameters(p): Parameters<IssuesListParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::list(&self.client, &p.owner, &p.repo, p.state.as_deref(), p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get a specific issue by number
    #[rmcp::tool(name = "github_issues_get")]
    async fn github_issues_get(&self, Parameters(p): Parameters<IssueGetParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::get(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Search issues across repositories
    #[rmcp::tool(name = "github_issues_search")]
    async fn github_issues_search(&self, Parameters(p): Parameters<SearchParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::search(&self.client, &p.query, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format_search(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Create a new issue
    #[rmcp::tool(name = "github_issues_create")]
    async fn github_issues_create(&self, Parameters(p): Parameters<IssueCreateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::create(&self.client, &p.owner, &p.repo, &p.title, p.body.as_deref(), p.labels, p.assignees).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Update an existing issue
    #[rmcp::tool(name = "github_issues_update")]
    async fn github_issues_update(&self, Parameters(p): Parameters<IssueUpdateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::update(&self.client, &p.owner, &p.repo, p.number, p.title, p.body, p.state, p.labels, p.assignees).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Add a comment to an issue
    #[rmcp::tool(name = "github_issues_comment")]
    async fn github_issues_comment(&self, Parameters(p): Parameters<IssueCommentParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::comment(&self.client, &p.owner, &p.repo, p.number, &p.body).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// List comments on an issue
    #[rmcp::tool(name = "github_issues_comments")]
    async fn github_issues_comments(&self, Parameters(p): Parameters<IssueCommentsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::comments(&self.client, &p.owner, &p.repo, p.number, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get labels on an issue
    #[rmcp::tool(name = "github_issues_labels")]
    async fn github_issues_labels(&self, Parameters(p): Parameters<IssueNumberFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::labels(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Pull Requests ====================

    /// List pull requests in a repository
    #[rmcp::tool(name = "github_prs_list")]
    async fn github_prs_list(&self, Parameters(p): Parameters<IssuesListParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::list(&self.client, &p.owner, &p.repo, p.state.as_deref(), p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get a specific pull request
    #[rmcp::tool(name = "github_prs_get")]
    async fn github_prs_get(&self, Parameters(p): Parameters<PrGetParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::get(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get the diff of a pull request
    #[rmcp::tool(name = "github_prs_diff")]
    async fn github_prs_diff(&self, Parameters(p): Parameters<PrDiffParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::diff(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    /// Get files changed in a pull request
    #[rmcp::tool(name = "github_prs_files")]
    async fn github_prs_files(&self, Parameters(p): Parameters<PrGetParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::files(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Create a new pull request
    #[rmcp::tool(name = "github_prs_create")]
    async fn github_prs_create(&self, Parameters(p): Parameters<PrCreateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::create(&self.client, &p.owner, &p.repo, &p.title, &p.head, &p.base, p.body.as_deref(), p.draft).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Update an existing pull request
    #[rmcp::tool(name = "github_prs_update")]
    async fn github_prs_update(&self, Parameters(p): Parameters<PrUpdateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::update(&self.client, &p.owner, &p.repo, p.number, p.title.as_deref(), p.body.as_deref(), p.state.as_deref(), p.base.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Merge a pull request
    #[rmcp::tool(name = "github_prs_merge")]
    async fn github_prs_merge(&self, Parameters(p): Parameters<PrMergeParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::merge(&self.client, &p.owner, &p.repo, p.number, p.merge_method.as_deref(), p.commit_title.as_deref(), p.commit_message.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get reviews on a pull request
    #[rmcp::tool(name = "github_prs_reviews")]
    async fn github_prs_reviews(&self, Parameters(p): Parameters<IssueNumberFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::reviews(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get review comments (threaded) on a pull request
    #[rmcp::tool(name = "github_prs_review_comments")]
    async fn github_prs_review_comments(&self, Parameters(p): Parameters<IssueNumberFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::review_comments(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get comments on a pull request (non-review comments)
    #[rmcp::tool(name = "github_prs_comments")]
    async fn github_prs_comments(&self, Parameters(p): Parameters<IssueNumberFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::comments(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get check runs for a pull request's head commit
    #[rmcp::tool(name = "github_prs_checks")]
    async fn github_prs_checks(&self, Parameters(p): Parameters<IssueNumberFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::check_runs(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get combined commit status for a pull request
    #[rmcp::tool(name = "github_prs_status")]
    async fn github_prs_status(&self, Parameters(p): Parameters<IssueNumberFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::status(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Create a review on a pull request (APPROVE, REQUEST_CHANGES, or COMMENT)
    #[rmcp::tool(name = "github_prs_review_create")]
    async fn github_prs_review_create(&self, Parameters(p): Parameters<PrReviewCreateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::create_review(&self.client, &p.owner, &p.repo, p.number, &p.event, p.body.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Update a PR branch with latest changes from the base branch
    #[rmcp::tool(name = "github_prs_update_branch")]
    async fn github_prs_update_branch(&self, Parameters(p): Parameters<PrDiffParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::update_branch(&self.client, &p.owner, &p.repo, p.number).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Files ====================

    /// Get file or directory contents from a repository
    #[rmcp::tool(name = "github_files_get")]
    async fn github_files_get(&self, Parameters(p): Parameters<FileGetParams>) -> Result<CallToolResult, McpError> {
        let result = tools::files::get_contents(&self.client, &p.owner, &p.repo, &p.path, p.git_ref.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Create or update a file in a repository (content must be base64-encoded)
    #[rmcp::tool(name = "github_files_create_or_update")]
    async fn github_files_create_or_update(&self, Parameters(p): Parameters<FileCreateOrUpdateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::files::create_or_update(&self.client, &p.owner, &p.repo, &p.path, &p.content, &p.message, &p.branch, p.sha.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Delete a file from a repository
    #[rmcp::tool(name = "github_files_delete")]
    async fn github_files_delete(&self, Parameters(p): Parameters<FileDeleteParams>) -> Result<CallToolResult, McpError> {
        let result = tools::files::delete(&self.client, &p.owner, &p.repo, &p.path, &p.message, &p.branch, &p.sha).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Repos ====================

    /// Get a single repository's details
    #[rmcp::tool(name = "github_repos_get")]
    async fn github_repos_get(&self, Parameters(p): Parameters<RepoFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::repos::get(&self.client, &p.owner, &p.repo).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Create a new repository
    #[rmcp::tool(name = "github_repos_create")]
    async fn github_repos_create(&self, Parameters(p): Parameters<RepoCreateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::repos::create(&self.client, &p.name, p.description.as_deref(), p.private, p.org.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Fork a repository
    #[rmcp::tool(name = "github_repos_fork")]
    async fn github_repos_fork(&self, Parameters(p): Parameters<RepoForkParams>) -> Result<CallToolResult, McpError> {
        let result = tools::repos::fork(&self.client, &p.owner, &p.repo, p.org.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Search repositories on GitHub
    #[rmcp::tool(name = "github_repos_search")]
    async fn github_repos_search(&self, Parameters(p): Parameters<SearchParams>) -> Result<CallToolResult, McpError> {
        let result = tools::repos::search(&self.client, &p.query, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format_search(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Branches ====================

    /// List branches in a repository
    #[rmcp::tool(name = "github_branches_list")]
    async fn github_branches_list(&self, Parameters(p): Parameters<RepoFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::branches::list(&self.client, &p.owner, &p.repo).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Create a new branch from a SHA
    #[rmcp::tool(name = "github_branches_create")]
    async fn github_branches_create(&self, Parameters(p): Parameters<BranchCreateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::branches::create(&self.client, &p.owner, &p.repo, &p.branch, &p.from_sha).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Commits ====================

    /// List commits in a repository
    #[rmcp::tool(name = "github_commits_list")]
    async fn github_commits_list(&self, Parameters(p): Parameters<CommitsListParams>) -> Result<CallToolResult, McpError> {
        let result = tools::commits::list(&self.client, &p.owner, &p.repo, p.sha.as_deref(), p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get a single commit with diff
    #[rmcp::tool(name = "github_commits_get")]
    async fn github_commits_get(&self, Parameters(p): Parameters<CommitGetParams>) -> Result<CallToolResult, McpError> {
        let result = tools::commits::get(&self.client, &p.owner, &p.repo, &p.sha).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Releases ====================

    /// List releases in a repository
    #[rmcp::tool(name = "github_releases_list")]
    async fn github_releases_list(&self, Parameters(p): Parameters<RepoLimitParams>) -> Result<CallToolResult, McpError> {
        let result = tools::releases::list(&self.client, &p.owner, &p.repo, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get a release by tag name
    #[rmcp::tool(name = "github_releases_get_by_tag")]
    async fn github_releases_get_by_tag(&self, Parameters(p): Parameters<ReleaseByTagParams>) -> Result<CallToolResult, McpError> {
        let result = tools::releases::get_by_tag(&self.client, &p.owner, &p.repo, &p.tag).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get the latest release in a repository
    #[rmcp::tool(name = "github_releases_latest")]
    async fn github_releases_latest(&self, Parameters(p): Parameters<RepoFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::releases::get_latest(&self.client, &p.owner, &p.repo).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Tags ====================

    /// List tags in a repository
    #[rmcp::tool(name = "github_tags_list")]
    async fn github_tags_list(&self, Parameters(p): Parameters<RepoLimitParams>) -> Result<CallToolResult, McpError> {
        let result = tools::tags::list(&self.client, &p.owner, &p.repo, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get a specific tag
    #[rmcp::tool(name = "github_tags_get")]
    async fn github_tags_get(&self, Parameters(p): Parameters<TagGetParams>) -> Result<CallToolResult, McpError> {
        let result = tools::tags::get(&self.client, &p.owner, &p.repo, &p.tag).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Teams ====================

    /// List teams for the authenticated user
    #[rmcp::tool(name = "github_teams_list")]
    async fn github_teams_list(&self, Parameters(p): Parameters<FieldsOnlyParams>) -> Result<CallToolResult, McpError> {
        let result = tools::teams::list(&self.client).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get members of a team
    #[rmcp::tool(name = "github_teams_members")]
    async fn github_teams_members(&self, Parameters(p): Parameters<TeamMembersParams>) -> Result<CallToolResult, McpError> {
        let result = tools::teams::members(&self.client, &p.org, &p.team_slug).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Users ====================

    /// Get the authenticated user's info
    #[rmcp::tool(name = "github_user_me")]
    async fn github_user_me(&self, Parameters(p): Parameters<FieldsOnlyParams>) -> Result<CallToolResult, McpError> {
        let result = tools::user::me(&self.client).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Search GitHub users
    #[rmcp::tool(name = "github_users_search")]
    async fn github_users_search(&self, Parameters(p): Parameters<SearchParams>) -> Result<CallToolResult, McpError> {
        let result = tools::user::search(&self.client, &p.query, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format_search(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Code Search ====================

    /// Search code across repositories
    #[rmcp::tool(name = "github_code_search")]
    async fn github_code_search(&self, Parameters(p): Parameters<SearchParams>) -> Result<CallToolResult, McpError> {
        let result = tools::code_search::search(&self.client, &p.query, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format_search(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Issues (additional) ====================

    /// Get sub-issues of an issue
    #[rmcp::tool(name = "github_issues_sub_issues")]
    async fn github_issues_sub_issues(&self, Parameters(p): Parameters<IssueSubIssuesParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::sub_issues(&self.client, &p.owner, &p.repo, p.number, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// List issue types for an organization
    #[rmcp::tool(name = "github_issues_list_types")]
    async fn github_issues_list_types(&self, Parameters(p): Parameters<OrgOwnerParams>) -> Result<CallToolResult, McpError> {
        let result = tools::issues::list_issue_types(&self.client, &p.owner).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== PRs (additional) ====================

    /// Search pull requests across repositories
    #[rmcp::tool(name = "github_prs_search")]
    async fn github_prs_search(&self, Parameters(p): Parameters<SearchParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::search(&self.client, &p.query, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format_search(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Add a review comment to a pull request
    #[rmcp::tool(name = "github_prs_add_review_comment")]
    async fn github_prs_add_review_comment(&self, Parameters(p): Parameters<PrReviewCommentParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::add_review_comment(&self.client, &p.owner, &p.repo, p.number, &p.path, &p.body, p.line, p.side.as_deref(), p.subject_type.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Reply to a pull request review comment
    #[rmcp::tool(name = "github_prs_reply_to_comment")]
    async fn github_prs_reply_to_comment(&self, Parameters(p): Parameters<PrReplyCommentParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::reply_to_comment(&self.client, &p.owner, &p.repo, p.number, p.comment_id, &p.body).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Submit a pending review on a pull request
    #[rmcp::tool(name = "github_prs_submit_review")]
    async fn github_prs_submit_review(&self, Parameters(p): Parameters<PrSubmitReviewParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::submit_review(&self.client, &p.owner, &p.repo, p.number, p.review_id, &p.event, p.body.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Delete a pending review on a pull request
    #[rmcp::tool(name = "github_prs_delete_review")]
    async fn github_prs_delete_review(&self, Parameters(p): Parameters<PrDeleteReviewParams>) -> Result<CallToolResult, McpError> {
        let result = tools::prs::delete_review(&self.client, &p.owner, &p.repo, p.number, p.review_id).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Files (additional) ====================

    /// Push multiple files to a repository (sequential create_or_update per file)
    #[rmcp::tool(name = "github_files_push")]
    async fn github_files_push(&self, Parameters(p): Parameters<FilePushParams>) -> Result<CallToolResult, McpError> {
        let result = tools::files::push_files(&self.client, &p.owner, &p.repo, &p.branch, &p.message, &p.files_json).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Labels ====================

    /// Get a specific label from a repository
    #[rmcp::tool(name = "github_labels_get")]
    async fn github_labels_get(&self, Parameters(p): Parameters<LabelGetParams>) -> Result<CallToolResult, McpError> {
        let result = tools::labels::get(&self.client, &p.owner, &p.repo, &p.name).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// List labels in a repository
    #[rmcp::tool(name = "github_labels_list")]
    async fn github_labels_list(&self, Parameters(p): Parameters<RepoLimitParams>) -> Result<CallToolResult, McpError> {
        let result = tools::labels::list(&self.client, &p.owner, &p.repo, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Create a label in a repository
    #[rmcp::tool(name = "github_labels_create")]
    async fn github_labels_create(&self, Parameters(p): Parameters<LabelCreateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::labels::create(&self.client, &p.owner, &p.repo, &p.name, p.color.as_deref(), p.description.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Update a label in a repository
    #[rmcp::tool(name = "github_labels_update")]
    async fn github_labels_update(&self, Parameters(p): Parameters<LabelUpdateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::labels::update(&self.client, &p.owner, &p.repo, &p.name, p.new_name.as_deref(), p.color.as_deref(), p.description.as_deref()).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Delete a label from a repository
    #[rmcp::tool(name = "github_labels_delete")]
    async fn github_labels_delete(&self, Parameters(p): Parameters<LabelDeleteParams>) -> Result<CallToolResult, McpError> {
        let result = tools::labels::delete(&self.client, &p.owner, &p.repo, &p.name).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Actions ====================

    /// List workflow runs for a repository
    #[rmcp::tool(name = "github_actions_list_runs")]
    async fn github_actions_list_runs(&self, Parameters(p): Parameters<RepoLimitParams>) -> Result<CallToolResult, McpError> {
        let result = tools::actions::list_runs(&self.client, &p.owner, &p.repo, p.limit).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get a specific workflow run
    #[rmcp::tool(name = "github_actions_get_run")]
    async fn github_actions_get_run(&self, Parameters(p): Parameters<ActionRunIdParams>) -> Result<CallToolResult, McpError> {
        let result = tools::actions::get_run(&self.client, &p.owner, &p.repo, p.run_id).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Re-run a workflow run
    #[rmcp::tool(name = "github_actions_rerun")]
    async fn github_actions_rerun(&self, Parameters(p): Parameters<ActionRunIdOnlyParams>) -> Result<CallToolResult, McpError> {
        let result = tools::actions::rerun(&self.client, &p.owner, &p.repo, p.run_id).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// List workflows for a repository
    #[rmcp::tool(name = "github_actions_list_workflows")]
    async fn github_actions_list_workflows(&self, Parameters(p): Parameters<RepoFieldsParams>) -> Result<CallToolResult, McpError> {
        let result = tools::actions::list_workflows(&self.client, &p.owner, &p.repo).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    /// Get logs URL for a workflow run
    #[rmcp::tool(name = "github_actions_run_logs")]
    async fn github_actions_run_logs(&self, Parameters(p): Parameters<ActionRunIdOnlyParams>) -> Result<CallToolResult, McpError> {
        let result = tools::actions::run_logs(&self.client, &p.owner, &p.repo, p.run_id).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }

    // ==================== Repos (additional) ====================

    /// Compare two commits, branches, or tags
    #[rmcp::tool(name = "github_repos_compare")]
    async fn github_repos_compare(&self, Parameters(p): Parameters<CompareParams>) -> Result<CallToolResult, McpError> {
        let result = tools::repos::compare(&self.client, &p.owner, &p.repo, &p.base, &p.head).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, p.fields, p.format);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }

    // ==================== Releases (additional) ====================

    /// Create a new release
    #[rmcp::tool(name = "github_releases_create")]
    async fn github_releases_create(&self, Parameters(p): Parameters<ReleaseCreateParams>) -> Result<CallToolResult, McpError> {
        let result = tools::releases::create(&self.client, &p.owner, &p.repo, &p.tag_name, p.name.as_deref(), p.body.as_deref(), p.draft, p.prerelease).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        let output = self.compress_and_format(result, None, None);
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

#[rmcp::tool_handler]
impl ServerHandler for KpGithubServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            server_info: rmcp::model::Implementation {
                name: "kp-github-mcp".into(),
                title: Some("Token-Compressed GitHub MCP Server".into()),
                version: env!("CARGO_PKG_VERSION").into(),
                description: Some("63-tool GitHub MCP server with 10-40x token compression via field projection, smart formatting, and 5-stage compression pipeline. Reqwest HTTP primary, gh CLI fallback.".into()),
                icons: None,
                website_url: None,
            },
            capabilities: rmcp::model::ServerCapabilities {
                tools: Some(rmcp::model::ToolsCapability { list_changed: Some(true) }),
                resources: Some(rmcp::model::ResourcesCapability::default()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn list_resource_templates(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        use rmcp::model::RawResourceTemplate;
        use rmcp::model::Annotated;

        let templates = vec![
            Annotated::new(
                RawResourceTemplate {
                    uri_template: "github://repos/{owner}/{repo}/issues".into(),
                    name: "repo_issues".into(),
                    title: Some("Repository Issues".into()),
                    description: Some("List open issues for a repository (compressed)".into()),
                    mime_type: Some("text/plain".into()),
                    icons: None,
                },
                None,
            ),
            Annotated::new(
                RawResourceTemplate {
                    uri_template: "github://repos/{owner}/{repo}/pulls".into(),
                    name: "repo_pulls".into(),
                    title: Some("Repository Pull Requests".into()),
                    description: Some("List open pull requests for a repository (compressed)".into()),
                    mime_type: Some("text/plain".into()),
                    icons: None,
                },
                None,
            ),
            Annotated::new(
                RawResourceTemplate {
                    uri_template: "github://repos/{owner}/{repo}/readme".into(),
                    name: "repo_readme".into(),
                    title: Some("Repository README".into()),
                    description: Some("Get the README content for a repository".into()),
                    mime_type: Some("text/markdown".into()),
                    icons: None,
                },
                None,
            ),
            Annotated::new(
                RawResourceTemplate {
                    uri_template: "github://repos/{owner}/{repo}".into(),
                    name: "repo_info".into(),
                    title: Some("Repository Info".into()),
                    description: Some("Get repository metadata (compressed)".into()),
                    mime_type: Some("text/plain".into()),
                    icons: None,
                },
                None,
            ),
        ];

        std::future::ready(Ok(ListResourceTemplatesResult::with_all_items(templates)))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        async move {
            let uri = &request.uri;

            // Parse github://repos/{owner}/{repo}[/suffix]
            let path = uri
                .strip_prefix("github://repos/")
                .ok_or_else(|| McpError::invalid_params(format!("unsupported URI scheme: {uri}"), None))?;

            let parts: Vec<&str> = path.splitn(3, '/').collect();
            if parts.len() < 2 {
                return Err(McpError::invalid_params(
                    format!("URI must include owner and repo: {uri}"),
                    None,
                ));
            }

            let owner = parts[0];
            let repo = parts[1];
            let suffix = parts.get(2).copied().unwrap_or("");

            let (text, mime) = match suffix {
                "issues" => {
                    let result = tools::issues::list(&self.client, owner, repo, Some("open"), Some(30))
                        .await
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    let output = self.compress_and_format(result, None, None);
                    (output, "text/plain")
                }
                "pulls" => {
                    let result = tools::prs::list(&self.client, owner, repo, Some("open"), Some(30))
                        .await
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    let output = self.compress_and_format(result, None, None);
                    (output, "text/plain")
                }
                "readme" => {
                    let result = tools::files::get_contents(&self.client, owner, repo, "README.md", None)
                        .await
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    // The GitHub contents API returns base64-encoded content;
                    // decode it so the resource is human-readable.
                    let text = if let Some(encoded) = result.get("content").and_then(|v| v.as_str()) {
                        let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
                        use base64::Engine;
                        match base64::engine::general_purpose::STANDARD.decode(&cleaned) {
                            Ok(bytes) => match String::from_utf8(bytes) {
                                Ok(s) => s,
                                Err(_) => "[binary content — cannot display as text]".to_string(),
                            },
                            Err(e) => format!("[base64 decode error: {e}]"),
                        }
                    } else {
                        serde_json::to_string_pretty(&result).unwrap_or_default()
                    };
                    (text, "text/markdown")
                }
                "" => {
                    let result = tools::repos::get(&self.client, owner, repo)
                        .await
                        .map_err(|e| McpError::internal_error(e.to_string(), None))?;
                    let output = self.compress_and_format(result, None, None);
                    (output, "text/plain")
                }
                other => {
                    return Err(McpError::invalid_params(
                        format!("unknown resource path suffix: {other}"),
                        None,
                    ));
                }
            };

            Ok(ReadResourceResult {
                contents: vec![ResourceContents::TextResourceContents {
                    uri: request.uri,
                    mime_type: Some(mime.into()),
                    text,
                    meta: None,
                }],
            })
        }
    }
}

pub async fn run(token: String) -> anyhow::Result<()> {
    let server = KpGithubServer::new(&token);
    let service = server.serve(rmcp::transport::io::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ---- Server construction ----

    #[test]
    fn test_server_new() {
        let server = KpGithubServer::new("ghp_test_token");
        // Just verify construction doesn't panic
        assert!(!server.compress_and_format(json!({"ok": true}), None, None).is_empty());
    }

    // ---- compress_and_format ----

    #[test]
    fn test_compress_and_format_default_json() {
        let server = KpGithubServer::new("test");
        let input = json!({"title": "Bug", "state": "open"});
        let output = server.compress_and_format(input, None, None);
        assert!(output.contains("Bug"));
        assert!(output.contains("open"));
    }

    #[test]
    fn test_compress_and_format_json_explicit() {
        let server = KpGithubServer::new("test");
        let input = json!({"title": "Bug"});
        let output = server.compress_and_format(input, None, Some("json".into()));
        assert!(output.contains("Bug"));
    }

    #[test]
    fn test_compress_and_format_text() {
        let server = KpGithubServer::new("test");
        let input = json!([
            {"number": 1, "title": "First"},
            {"number": 2, "title": "Second"}
        ]);
        let output = server.compress_and_format(input, None, Some("text".into()));
        assert!(output.contains("#1"));
        assert!(output.contains("First"));
    }

    #[test]
    fn test_compress_and_format_table() {
        let server = KpGithubServer::new("test");
        let input = json!([
            {"id": 1, "name": "a"},
            {"id": 2, "name": "b"},
            {"id": 3, "name": "c"}
        ]);
        let output = server.compress_and_format(input, None, Some("table".into()));
        assert!(output.contains('|'));
        assert!(output.contains("id"));
        assert!(output.contains("name"));
    }

    #[test]
    fn test_compress_and_format_auto_fallback() {
        let server = KpGithubServer::new("test");
        let input = json!({"title": "test"});
        let output = server.compress_and_format(input, None, Some("unknown_format".into()));
        // Unknown format falls through to Auto
        assert!(output.contains("test"));
    }

    #[test]
    fn test_compress_and_format_with_fields() {
        let server = KpGithubServer::new("test");
        let input = json!({
            "title": "Bug",
            "state": "open",
            "body": "some body",
            "user": {"login": "alice", "id": 1}
        });
        let output = server.compress_and_format(
            input,
            Some(vec!["title".into(), "state".into()]),
            Some("json".into()),
        );
        assert!(output.contains("Bug"));
        assert!(output.contains("open"));
        assert!(!output.contains("alice"));
        assert!(!output.contains("body"));
    }

    #[test]
    fn test_compress_and_format_with_compression() {
        let server = KpGithubServer::new("test");
        let input = json!({
            "title": "Test",
            "user": {"login": "alice", "id": 1, "avatar_url": "http://..."},
            "node_id": "MDU6SXNzdWUx",
            "html_url": "https://github.com/owner/repo/issues/42"
        });
        let output = server.compress_and_format(input, None, Some("json".into()));
        // avatar_url and node_id stripped
        assert!(!output.contains("avatar_url"));
        assert!(!output.contains("MDU6SXNzdWUx"));
        // user flattened to login
        assert!(output.contains("alice"));
        // html_url compacted
        assert!(output.contains("owner/repo#42"));
    }

    #[test]
    fn test_compress_and_format_empty_object() {
        let server = KpGithubServer::new("test");
        let output = server.compress_and_format(json!({}), None, None);
        assert_eq!(output.trim(), "{}");
    }

    #[test]
    fn test_compress_and_format_empty_array() {
        let server = KpGithubServer::new("test");
        let output = server.compress_and_format(json!([]), None, None);
        assert!(output.is_empty() || output.trim() == "[]" || output.trim().is_empty());
    }

    // ---- Parameter struct deserialization ----

    #[test]
    fn test_repo_params_deser() {
        let p: RepoParams = serde_json::from_value(json!({
            "owner": "alice",
            "repo": "project"
        }))
        .unwrap();
        assert_eq!(p.owner, "alice");
        assert_eq!(p.repo, "project");
    }

    #[test]
    fn test_issues_list_params_full() {
        let p: IssuesListParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "state": "open",
            "limit": 10,
            "fields": ["title", "state"],
            "format": "table"
        }))
        .unwrap();
        assert_eq!(p.owner, "o");
        assert_eq!(p.repo, "r");
        assert_eq!(p.state.as_deref(), Some("open"));
        assert_eq!(p.limit, Some(10));
        assert_eq!(p.fields.as_ref().unwrap().len(), 2);
        assert_eq!(p.format.as_deref(), Some("table"));
    }

    #[test]
    fn test_issues_list_params_minimal() {
        let p: IssuesListParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r"
        }))
        .unwrap();
        assert!(p.state.is_none());
        assert!(p.limit.is_none());
        assert!(p.fields.is_none());
        assert!(p.format.is_none());
    }

    #[test]
    fn test_issue_get_params_deser() {
        let p: IssueGetParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 42
        }))
        .unwrap();
        assert_eq!(p.number, 42);
    }

    #[test]
    fn test_search_params_deser() {
        let p: SearchParams = serde_json::from_value(json!({
            "query": "is:open label:bug",
            "limit": 5,
            "fields": ["title"],
            "format": "json"
        }))
        .unwrap();
        assert_eq!(p.query, "is:open label:bug");
        assert_eq!(p.limit, Some(5));
    }

    #[test]
    fn test_issue_create_params_deser() {
        let p: IssueCreateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "title": "New Bug",
            "body": "Description here",
            "labels": ["bug", "p1"],
            "assignees": ["alice"]
        }))
        .unwrap();
        assert_eq!(p.title, "New Bug");
        assert_eq!(p.body.as_deref(), Some("Description here"));
        assert_eq!(p.labels.as_ref().unwrap(), &["bug", "p1"]);
        assert_eq!(p.assignees.as_ref().unwrap(), &["alice"]);
    }

    #[test]
    fn test_issue_update_params_deser() {
        let p: IssueUpdateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 5,
            "state": "closed"
        }))
        .unwrap();
        assert_eq!(p.number, 5);
        assert_eq!(p.state.as_deref(), Some("closed"));
        assert!(p.title.is_none());
    }

    #[test]
    fn test_issue_comment_params_deser() {
        let p: IssueCommentParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 1,
            "body": "LGTM"
        }))
        .unwrap();
        assert_eq!(p.body, "LGTM");
    }

    #[test]
    fn test_pr_get_params_deser() {
        let p: PrGetParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 99,
            "fields": ["title", "state"],
            "format": "text"
        }))
        .unwrap();
        assert_eq!(p.number, 99);
        assert_eq!(p.format.as_deref(), Some("text"));
    }

    #[test]
    fn test_pr_diff_params_deser() {
        let p: PrDiffParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 1
        }))
        .unwrap();
        assert_eq!(p.number, 1);
    }

    #[test]
    fn test_fields_only_params_deser() {
        let p: FieldsOnlyParams = serde_json::from_value(json!({
            "fields": ["login"],
            "format": "json"
        }))
        .unwrap();
        assert_eq!(p.fields.as_ref().unwrap(), &["login"]);
    }

    #[test]
    fn test_fields_only_params_empty() {
        let p: FieldsOnlyParams = serde_json::from_value(json!({})).unwrap();
        assert!(p.fields.is_none());
        assert!(p.format.is_none());
    }

    #[test]
    fn test_commits_list_params_deser() {
        let p: CommitsListParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "sha": "main",
            "limit": 20
        }))
        .unwrap();
        assert_eq!(p.sha.as_deref(), Some("main"));
        assert_eq!(p.limit, Some(20));
    }

    #[test]
    fn test_repo_fields_params_deser() {
        let p: RepoFieldsParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r"
        }))
        .unwrap();
        assert_eq!(p.owner, "o");
    }

    #[test]
    fn test_repo_limit_params_deser() {
        let p: RepoLimitParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "limit": 50
        }))
        .unwrap();
        assert_eq!(p.limit, Some(50));
    }

    #[test]
    fn test_issue_comments_params_deser() {
        let p: IssueCommentsParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 7,
            "limit": 10
        }))
        .unwrap();
        assert_eq!(p.number, 7);
    }

    #[test]
    fn test_issue_number_fields_params_deser() {
        let p: IssueNumberFieldsParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 3
        }))
        .unwrap();
        assert_eq!(p.number, 3);
    }

    #[test]
    fn test_pr_create_params_deser() {
        let p: PrCreateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "title": "Fix bug",
            "head": "fix-branch",
            "base": "main",
            "body": "Fixes #42",
            "draft": true
        }))
        .unwrap();
        assert_eq!(p.title, "Fix bug");
        assert_eq!(p.head, "fix-branch");
        assert_eq!(p.base, "main");
        assert_eq!(p.draft, Some(true));
    }

    #[test]
    fn test_pr_update_params_deser() {
        let p: PrUpdateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 10,
            "state": "closed"
        }))
        .unwrap();
        assert_eq!(p.number, 10);
        assert_eq!(p.state.as_deref(), Some("closed"));
        assert!(p.title.is_none());
    }

    #[test]
    fn test_pr_merge_params_deser() {
        let p: PrMergeParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 5,
            "merge_method": "squash",
            "commit_title": "feat: merge PR",
            "commit_message": "Details"
        }))
        .unwrap();
        assert_eq!(p.merge_method.as_deref(), Some("squash"));
    }

    #[test]
    fn test_pr_review_create_params_deser() {
        let p: PrReviewCreateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 1,
            "event": "APPROVE",
            "body": "Looks good"
        }))
        .unwrap();
        assert_eq!(p.event, "APPROVE");
    }

    #[test]
    fn test_file_get_params_deser() {
        let p: FileGetParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "path": "src/main.rs",
            "git_ref": "v1.0"
        }))
        .unwrap();
        assert_eq!(p.path, "src/main.rs");
        assert_eq!(p.git_ref.as_deref(), Some("v1.0"));
    }

    #[test]
    fn test_file_create_or_update_params_deser() {
        let p: FileCreateOrUpdateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "path": "README.md",
            "content": "SGVsbG8=",
            "message": "add readme",
            "branch": "main"
        }))
        .unwrap();
        assert_eq!(p.content, "SGVsbG8=");
        assert!(p.sha.is_none());
    }

    #[test]
    fn test_file_delete_params_deser() {
        let p: FileDeleteParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "path": "old.txt",
            "message": "remove old",
            "branch": "main",
            "sha": "abc123"
        }))
        .unwrap();
        assert_eq!(p.sha, "abc123");
    }

    #[test]
    fn test_repo_create_params_deser() {
        let p: RepoCreateParams = serde_json::from_value(json!({
            "name": "new-project",
            "description": "A new project",
            "private": true,
            "org": "my-org"
        }))
        .unwrap();
        assert_eq!(p.name, "new-project");
        assert_eq!(p.private, Some(true));
        assert_eq!(p.org.as_deref(), Some("my-org"));
    }

    #[test]
    fn test_repo_fork_params_deser() {
        let p: RepoForkParams = serde_json::from_value(json!({
            "owner": "upstream",
            "repo": "project"
        }))
        .unwrap();
        assert_eq!(p.owner, "upstream");
        assert!(p.org.is_none());
    }

    #[test]
    fn test_branch_create_params_deser() {
        let p: BranchCreateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "branch": "feature-x",
            "from_sha": "abc1234"
        }))
        .unwrap();
        assert_eq!(p.branch, "feature-x");
        assert_eq!(p.from_sha, "abc1234");
    }

    #[test]
    fn test_commit_get_params_deser() {
        let p: CommitGetParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "sha": "deadbeef"
        }))
        .unwrap();
        assert_eq!(p.sha, "deadbeef");
    }

    #[test]
    fn test_release_by_tag_params_deser() {
        let p: ReleaseByTagParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "tag": "v1.0.0"
        }))
        .unwrap();
        assert_eq!(p.tag, "v1.0.0");
    }

    #[test]
    fn test_tag_get_params_deser() {
        let p: TagGetParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "tag": "v2.0"
        }))
        .unwrap();
        assert_eq!(p.tag, "v2.0");
    }

    #[test]
    fn test_team_members_params_deser() {
        let p: TeamMembersParams = serde_json::from_value(json!({
            "org": "my-org",
            "team_slug": "core-team"
        }))
        .unwrap();
        assert_eq!(p.org, "my-org");
        assert_eq!(p.team_slug, "core-team");
    }

    #[test]
    fn test_issue_sub_issues_params_deser() {
        let p: IssueSubIssuesParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 10,
            "limit": 5
        }))
        .unwrap();
        assert_eq!(p.number, 10);
        assert_eq!(p.limit, Some(5));
    }

    #[test]
    fn test_org_owner_params_deser() {
        let p: OrgOwnerParams = serde_json::from_value(json!({
            "owner": "my-org"
        }))
        .unwrap();
        assert_eq!(p.owner, "my-org");
    }

    #[test]
    fn test_pr_review_comment_params_deser() {
        let p: PrReviewCommentParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 1,
            "path": "src/lib.rs",
            "body": "Nitpick here",
            "line": 42,
            "side": "RIGHT",
            "subject_type": "LINE"
        }))
        .unwrap();
        assert_eq!(p.path, "src/lib.rs");
        assert_eq!(p.line, Some(42));
        assert_eq!(p.side.as_deref(), Some("RIGHT"));
        assert_eq!(p.subject_type.as_deref(), Some("LINE"));
    }

    #[test]
    fn test_pr_reply_comment_params_deser() {
        let p: PrReplyCommentParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 1,
            "comment_id": 12345,
            "body": "Good point"
        }))
        .unwrap();
        assert_eq!(p.comment_id, 12345);
    }

    #[test]
    fn test_pr_submit_review_params_deser() {
        let p: PrSubmitReviewParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 1,
            "review_id": 999,
            "event": "APPROVE"
        }))
        .unwrap();
        assert_eq!(p.review_id, 999);
        assert_eq!(p.event, "APPROVE");
    }

    #[test]
    fn test_pr_delete_review_params_deser() {
        let p: PrDeleteReviewParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "number": 1,
            "review_id": 888
        }))
        .unwrap();
        assert_eq!(p.review_id, 888);
    }

    #[test]
    fn test_file_push_params_deser() {
        let p: FilePushParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "branch": "main",
            "message": "push files",
            "files_json": "[{\"path\":\"a.txt\",\"content\":\"aGk=\"}]"
        }))
        .unwrap();
        assert_eq!(p.branch, "main");
        assert!(p.files_json.contains("a.txt"));
    }

    #[test]
    fn test_label_get_params_deser() {
        let p: LabelGetParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "name": "bug"
        }))
        .unwrap();
        assert_eq!(p.name, "bug");
    }

    #[test]
    fn test_label_create_params_deser() {
        let p: LabelCreateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "name": "enhancement",
            "color": "00ff00",
            "description": "Feature request"
        }))
        .unwrap();
        assert_eq!(p.name, "enhancement");
        assert_eq!(p.color.as_deref(), Some("00ff00"));
    }

    #[test]
    fn test_label_update_params_deser() {
        let p: LabelUpdateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "name": "bug",
            "new_name": "defect",
            "color": "ff0000"
        }))
        .unwrap();
        assert_eq!(p.new_name.as_deref(), Some("defect"));
    }

    #[test]
    fn test_label_delete_params_deser() {
        let p: LabelDeleteParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "name": "stale"
        }))
        .unwrap();
        assert_eq!(p.name, "stale");
    }

    #[test]
    fn test_action_run_id_params_deser() {
        let p: ActionRunIdParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "run_id": 123456789
        }))
        .unwrap();
        assert_eq!(p.run_id, 123456789);
    }

    #[test]
    fn test_action_run_id_only_params_deser() {
        let p: ActionRunIdOnlyParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "run_id": 987654321
        }))
        .unwrap();
        assert_eq!(p.run_id, 987654321);
    }

    #[test]
    fn test_compare_params_deser() {
        let p: CompareParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "base": "main",
            "head": "feature"
        }))
        .unwrap();
        assert_eq!(p.base, "main");
        assert_eq!(p.head, "feature");
    }

    #[test]
    fn test_release_create_params_deser() {
        let p: ReleaseCreateParams = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "tag_name": "v1.0.0",
            "name": "Release 1.0",
            "body": "First stable",
            "draft": false,
            "prerelease": false
        }))
        .unwrap();
        assert_eq!(p.tag_name, "v1.0.0");
        assert_eq!(p.draft, Some(false));
    }

    // ---- Missing required fields should fail ----

    #[test]
    fn test_repo_params_missing_owner() {
        let result: Result<RepoParams, _> = serde_json::from_value(json!({"repo": "r"}));
        assert!(result.is_err());
    }

    #[test]
    fn test_issue_create_missing_title() {
        let result: Result<IssueCreateParams, _> = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r"
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_pr_create_missing_head() {
        let result: Result<PrCreateParams, _> = serde_json::from_value(json!({
            "owner": "o",
            "repo": "r",
            "title": "PR",
            "base": "main"
        }));
        assert!(result.is_err());
    }

    // ---- compress_and_format with realistic fixtures ----

    #[test]
    fn test_compress_and_format_realistic_issue_list() {
        let server = KpGithubServer::new("test");
        let input = json!([
            {
                "id": 1, "number": 10, "title": "Bug A", "state": "open",
                "user": {"login": "alice", "id": 1, "avatar_url": "https://..."},
                "labels": [{"name": "bug", "id": 1}],
                "html_url": "https://github.com/o/r/issues/10",
                "node_id": "xyz", "events_url": "https://..."
            },
            {
                "id": 2, "number": 11, "title": "Bug B", "state": "closed",
                "user": {"login": "bob", "id": 2, "avatar_url": "https://..."},
                "labels": [],
                "html_url": "https://github.com/o/r/issues/11",
                "node_id": "abc", "events_url": "https://..."
            }
        ]);
        let output = server.compress_and_format(input, None, Some("json".into()));
        // Compressed output should not contain waste
        assert!(!output.contains("node_id"));
        assert!(!output.contains("events_url"));
        assert!(!output.contains("avatar_url"));
        // Should contain core data
        assert!(output.contains("Bug A"));
        assert!(output.contains("Bug B"));
        assert!(output.contains("alice"));
        assert!(output.contains("bob"));
    }

    #[test]
    fn test_compress_and_format_realistic_pr() {
        let server = KpGithubServer::new("test");
        let input = json!({
            "number": 42,
            "title": "Add feature",
            "state": "open",
            "user": {"login": "dev", "id": 1, "avatar_url": "http://..."},
            "head": {"ref": "feature", "sha": "abc1234567890", "repo": {"full_name": "dev/project"}},
            "base": {"ref": "main", "sha": "def7890123456", "repo": {"full_name": "org/project"}},
            "html_url": "https://github.com/org/project/pull/42",
            "node_id": "xyz",
            "merged_by": null,
            "milestone": null
        });
        let output = server.compress_and_format(input, None, Some("json".into()));
        assert!(output.contains("feature")); // head_ref
        assert!(output.contains("abc1234")); // truncated sha
        assert!(output.contains("org/project!42")); // compacted URL
        assert!(!output.contains("node_id"));
    }

    // ---- ServerInfo ----

    #[test]
    fn test_server_info() {
        let server = KpGithubServer::new("test");
        let info = server.get_info();
        assert_eq!(info.server_info.name, "kp-github-mcp");
        assert!(info.server_info.title.as_deref().unwrap().contains("GitHub"));
    }

    // ---- Additional compress_and_format edge cases ----

    #[test]
    fn test_compress_and_format_null_value() {
        let server = KpGithubServer::new("test");
        let output = server.compress_and_format(Value::Null, None, None);
        assert_eq!(output.trim(), "null");
    }

    #[test]
    fn test_compress_and_format_string_value() {
        let server = KpGithubServer::new("test");
        let output = server.compress_and_format(json!("hello"), None, None);
        assert!(output.contains("hello"));
    }

    #[test]
    fn test_compress_and_format_number_value() {
        let server = KpGithubServer::new("test");
        let output = server.compress_and_format(json!(42), None, None);
        assert!(output.contains("42"));
    }

    #[test]
    fn test_compress_and_format_fields_empty_vec() {
        let server = KpGithubServer::new("test");
        let input = json!({"title": "Bug", "state": "open"});
        // Empty fields vec = project to nothing
        let output = server.compress_and_format(input, Some(vec![]), Some("json".into()));
        assert_eq!(output.trim(), "{}");
    }

    #[test]
    fn test_compress_and_format_nested_compression() {
        let server = KpGithubServer::new("test");
        let input = json!([
            {
                "number": 1,
                "title": "PR One",
                "head": {"ref": "feat-1", "sha": "aabbccdd11223344", "repo": {"full_name": "dev/repo"}},
                "base": {"ref": "main", "sha": "eeff00112233aabb", "repo": {"full_name": "org/repo"}},
                "user": {"login": "dev1", "id": 1, "avatar_url": "http://..."},
                "labels": [{"name": "ready", "id": 1}],
                "assignees": [{"login": "reviewer", "id": 2}],
                "html_url": "https://github.com/org/repo/pull/1",
                "node_id": "xyz"
            },
            {
                "number": 2,
                "title": "PR Two",
                "head": {"ref": "feat-2", "sha": "1122334455667788", "repo": {"full_name": "dev/repo"}},
                "base": {"ref": "main", "sha": "aabbccddee001122", "repo": {"full_name": "org/repo"}},
                "user": {"login": "dev2", "id": 3, "avatar_url": "http://..."},
                "labels": [],
                "assignees": [],
                "html_url": "https://github.com/org/repo/pull/2",
                "node_id": "abc"
            },
            {
                "number": 3,
                "title": "PR Three",
                "head": {"ref": "feat-3", "sha": "9988776655443322", "repo": {"full_name": "dev/repo"}},
                "base": {"ref": "main", "sha": "aabb11223344eeff", "repo": {"full_name": "org/repo"}},
                "user": {"login": "dev3", "id": 5, "avatar_url": "http://..."},
                "labels": [{"name": "wip", "id": 2}],
                "assignees": [{"login": "lead", "id": 4}],
                "html_url": "https://github.com/org/repo/pull/3",
                "node_id": "def"
            }
        ]);
        let output = server.compress_and_format(input, None, Some("table".into()));
        // Should produce table format with flattened data
        assert!(output.contains('|'));
        assert!(output.contains("PR One"));
        assert!(output.contains("dev1"));
        assert!(!output.contains("node_id"));
        assert!(!output.contains("avatar_url"));
    }

    #[test]
    fn test_compress_and_format_with_timestamps() {
        let server = KpGithubServer::new("test");
        let input = json!({
            "title": "Old Issue",
            "created_at": "2020-01-01T00:00:00Z",
            "updated_at": "2020-06-15T12:00:00Z"
        });
        let output = server.compress_and_format(input, None, Some("json".into()));
        // Very old timestamps get formatted as dates
        assert!(output.contains("2020-01-01"));
        assert!(output.contains("2020-06-15"));
    }

    // ---- JSON Schema generation (via schemars) ----

    #[test]
    fn test_repo_params_schema() {
        let schema = schemars::schema_for!(RepoParams);
        let json = serde_json::to_value(&schema).unwrap();
        let props = &json["properties"];
        assert!(props.get("owner").is_some());
        assert!(props.get("repo").is_some());
    }

    #[test]
    fn test_issues_list_params_schema() {
        let schema = schemars::schema_for!(IssuesListParams);
        let json = serde_json::to_value(&schema).unwrap();
        let props = &json["properties"];
        assert!(props.get("owner").is_some());
        assert!(props.get("state").is_some());
        assert!(props.get("limit").is_some());
        assert!(props.get("fields").is_some());
        assert!(props.get("format").is_some());
    }

    #[test]
    fn test_search_params_schema() {
        let schema = schemars::schema_for!(SearchParams);
        let json = serde_json::to_value(&schema).unwrap();
        let props = &json["properties"];
        assert!(props.get("query").is_some());
    }

    #[test]
    fn test_pr_create_params_schema() {
        let schema = schemars::schema_for!(PrCreateParams);
        let json = serde_json::to_value(&schema).unwrap();
        let props = &json["properties"];
        assert!(props.get("title").is_some());
        assert!(props.get("head").is_some());
        assert!(props.get("base").is_some());
        assert!(props.get("draft").is_some());
    }

    #[test]
    fn test_file_get_params_schema() {
        let schema = schemars::schema_for!(FileGetParams);
        let json = serde_json::to_value(&schema).unwrap();
        let props = &json["properties"];
        assert!(props.get("path").is_some());
        assert!(props.get("git_ref").is_some());
    }

    #[test]
    fn test_release_create_params_schema() {
        let schema = schemars::schema_for!(ReleaseCreateParams);
        let json = serde_json::to_value(&schema).unwrap();
        let props = &json["properties"];
        assert!(props.get("tag_name").is_some());
        assert!(props.get("prerelease").is_some());
    }

    #[test]
    fn test_all_param_structs_are_deserializable() {
        // Verify that all param structs can round-trip through JSON schema
        // This exercises the JsonSchema derive for all structs
        let _ = schemars::schema_for!(RepoParams);
        let _ = schemars::schema_for!(IssuesListParams);
        let _ = schemars::schema_for!(IssueGetParams);
        let _ = schemars::schema_for!(SearchParams);
        let _ = schemars::schema_for!(IssueCreateParams);
        let _ = schemars::schema_for!(IssueUpdateParams);
        let _ = schemars::schema_for!(IssueCommentParams);
        let _ = schemars::schema_for!(PrGetParams);
        let _ = schemars::schema_for!(PrDiffParams);
        let _ = schemars::schema_for!(FieldsOnlyParams);
        let _ = schemars::schema_for!(CommitsListParams);
        let _ = schemars::schema_for!(RepoFieldsParams);
        let _ = schemars::schema_for!(RepoLimitParams);
        let _ = schemars::schema_for!(IssueCommentsParams);
        let _ = schemars::schema_for!(IssueNumberFieldsParams);
        let _ = schemars::schema_for!(PrCreateParams);
        let _ = schemars::schema_for!(PrUpdateParams);
        let _ = schemars::schema_for!(PrMergeParams);
        let _ = schemars::schema_for!(PrReviewCreateParams);
        let _ = schemars::schema_for!(FileGetParams);
        let _ = schemars::schema_for!(FileCreateOrUpdateParams);
        let _ = schemars::schema_for!(FileDeleteParams);
        let _ = schemars::schema_for!(RepoCreateParams);
        let _ = schemars::schema_for!(RepoForkParams);
        let _ = schemars::schema_for!(BranchCreateParams);
        let _ = schemars::schema_for!(CommitGetParams);
        let _ = schemars::schema_for!(ReleaseByTagParams);
        let _ = schemars::schema_for!(TagGetParams);
        let _ = schemars::schema_for!(TeamMembersParams);
        let _ = schemars::schema_for!(IssueSubIssuesParams);
        let _ = schemars::schema_for!(OrgOwnerParams);
        let _ = schemars::schema_for!(PrReviewCommentParams);
        let _ = schemars::schema_for!(PrReplyCommentParams);
        let _ = schemars::schema_for!(PrSubmitReviewParams);
        let _ = schemars::schema_for!(PrDeleteReviewParams);
        let _ = schemars::schema_for!(FilePushParams);
        let _ = schemars::schema_for!(LabelGetParams);
        let _ = schemars::schema_for!(LabelCreateParams);
        let _ = schemars::schema_for!(LabelUpdateParams);
        let _ = schemars::schema_for!(LabelDeleteParams);
        let _ = schemars::schema_for!(ActionRunIdParams);
        let _ = schemars::schema_for!(ActionRunIdOnlyParams);
        let _ = schemars::schema_for!(CompareParams);
        let _ = schemars::schema_for!(ReleaseCreateParams);
    }

    // ==== Async tool method tests via mock client ====
    // Each test exercises the full tool method path through server.rs

    fn mock_server(responses: Vec<Value>) -> KpGithubServer {
        KpGithubServer::with_client(GithubClient::mock(responses))
    }

    fn ok_text(result: &CallToolResult) -> String {
        result.content.iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("")
    }

    // --- Issues ---

    #[tokio::test]
    async fn test_tool_issues_list() {
        let server = mock_server(vec![json!([{"number": 1, "title": "Bug"}])]);
        let result = server.github_issues_list(Parameters(IssuesListParams {
            owner: "o".into(), repo: "r".into(), state: Some("open".into()),
            limit: Some(10), fields: None, format: None,
        })).await.unwrap();
        let text = ok_text(&result);
        assert!(text.contains("Bug"));
    }

    #[tokio::test]
    async fn test_tool_issues_get() {
        let server = mock_server(vec![json!({"number": 42, "title": "Found it"})]);
        let result = server.github_issues_get(Parameters(IssueGetParams {
            owner: "o".into(), repo: "r".into(), number: 42,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("Found it"));
    }

    #[tokio::test]
    async fn test_tool_issues_search() {
        let server = mock_server(vec![json!({"items": [{"title": "Match"}]})]);
        let result = server.github_issues_search(Parameters(SearchParams {
            query: "is:open".into(), limit: None, fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("Match"));
    }

    #[tokio::test]
    async fn test_tool_issues_create() {
        let server = mock_server(vec![json!({"number": 99, "title": "New"})]);
        let result = server.github_issues_create(Parameters(IssueCreateParams {
            owner: "o".into(), repo: "r".into(), title: "New".into(),
            body: Some("desc".into()), labels: Some(vec!["bug".into()]),
            assignees: Some(vec!["alice".into()]),
        })).await.unwrap();
        assert!(ok_text(&result).contains("New"));
    }

    #[tokio::test]
    async fn test_tool_issues_update() {
        let server = mock_server(vec![json!({"number": 5, "state": "closed"})]);
        let result = server.github_issues_update(Parameters(IssueUpdateParams {
            owner: "o".into(), repo: "r".into(), number: 5,
            title: None, body: None, state: Some("closed".into()),
            labels: None, assignees: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("closed"));
    }

    #[tokio::test]
    async fn test_tool_issues_comment() {
        let server = mock_server(vec![json!({"id": 1, "body": "LGTM"})]);
        let result = server.github_issues_comment(Parameters(IssueCommentParams {
            owner: "o".into(), repo: "r".into(), number: 1, body: "LGTM".into(),
        })).await.unwrap();
        assert!(ok_text(&result).contains("LGTM"));
    }

    #[tokio::test]
    async fn test_tool_issues_comments() {
        let server = mock_server(vec![json!([{"id": 1, "body": "Comment"}])]);
        let result = server.github_issues_comments(Parameters(IssueCommentsParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            limit: None, fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("Comment"));
    }

    #[tokio::test]
    async fn test_tool_issues_labels() {
        let server = mock_server(vec![json!([{"name": "bug"}])]);
        let result = server.github_issues_labels(Parameters(IssueNumberFieldsParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("bug"));
    }

    #[tokio::test]
    async fn test_tool_issues_sub_issues() {
        let server = mock_server(vec![json!([{"number": 2, "title": "Sub"}])]);
        let result = server.github_issues_sub_issues(Parameters(IssueSubIssuesParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            limit: None, fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("Sub"));
    }

    #[tokio::test]
    async fn test_tool_issues_list_types() {
        let server = mock_server(vec![json!([{"name": "Bug"}, {"name": "Feature"}])]);
        let result = server.github_issues_list_types(Parameters(OrgOwnerParams {
            owner: "my-org".into(), fields: None, format: None,
        })).await.unwrap();
        let text = ok_text(&result);
        assert!(text.contains("Bug"));
        assert!(text.contains("Feature"));
    }

    // --- Pull Requests ---

    #[tokio::test]
    async fn test_tool_prs_list() {
        let server = mock_server(vec![json!([{"number": 1, "title": "PR"}])]);
        let result = server.github_prs_list(Parameters(IssuesListParams {
            owner: "o".into(), repo: "r".into(), state: None,
            limit: None, fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("PR"));
    }

    #[tokio::test]
    async fn test_tool_prs_get() {
        let server = mock_server(vec![json!({"number": 10, "title": "My PR"})]);
        let result = server.github_prs_get(Parameters(PrGetParams {
            owner: "o".into(), repo: "r".into(), number: 10,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("My PR"));
    }

    #[tokio::test]
    async fn test_tool_prs_diff() {
        let server = mock_server(vec![json!("diff --git a/file b/file")]);
        let result = server.github_prs_diff(Parameters(PrDiffParams {
            owner: "o".into(), repo: "r".into(), number: 1,
        })).await.unwrap();
        assert!(ok_text(&result).contains("diff"));
    }

    #[tokio::test]
    async fn test_tool_prs_files() {
        let server = mock_server(vec![json!([{"filename": "src/lib.rs", "changes": 5}])]);
        let result = server.github_prs_files(Parameters(PrGetParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("lib.rs"));
    }

    #[tokio::test]
    async fn test_tool_prs_create() {
        let server = mock_server(vec![json!({"number": 50, "title": "New PR"})]);
        let result = server.github_prs_create(Parameters(PrCreateParams {
            owner: "o".into(), repo: "r".into(), title: "New PR".into(),
            head: "feature".into(), base: "main".into(),
            body: Some("description".into()), draft: Some(false),
        })).await.unwrap();
        assert!(ok_text(&result).contains("New PR"));
    }

    #[tokio::test]
    async fn test_tool_prs_update() {
        let server = mock_server(vec![json!({"number": 10, "state": "closed"})]);
        let result = server.github_prs_update(Parameters(PrUpdateParams {
            owner: "o".into(), repo: "r".into(), number: 10,
            title: Some("Updated".into()), body: None,
            state: Some("closed".into()), base: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("closed"));
    }

    #[tokio::test]
    async fn test_tool_prs_merge() {
        let server = mock_server(vec![json!({"merged": true})]);
        let result = server.github_prs_merge(Parameters(PrMergeParams {
            owner: "o".into(), repo: "r".into(), number: 5,
            merge_method: Some("squash".into()),
            commit_title: Some("feat: merge".into()),
            commit_message: Some("details".into()),
        })).await.unwrap();
        assert!(ok_text(&result).contains("merged"));
    }

    #[tokio::test]
    async fn test_tool_prs_reviews() {
        let server = mock_server(vec![json!([{"id": 1, "state": "APPROVED"}])]);
        let result = server.github_prs_reviews(Parameters(IssueNumberFieldsParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("APPROVED"));
    }

    #[tokio::test]
    async fn test_tool_prs_review_comments() {
        let server = mock_server(vec![json!([{"id": 1, "body": "nitpick"}])]);
        let result = server.github_prs_review_comments(Parameters(IssueNumberFieldsParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("nitpick"));
    }

    #[tokio::test]
    async fn test_tool_prs_comments() {
        let server = mock_server(vec![json!([{"id": 1, "body": "general comment"}])]);
        let result = server.github_prs_comments(Parameters(IssueNumberFieldsParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("general comment"));
    }

    #[tokio::test]
    async fn test_tool_prs_checks() {
        // check_runs calls get() first (for SHA), then the check-runs endpoint
        let server = mock_server(vec![
            json!({"head": {"sha": "abc123"}}),  // PR get
            json!({"check_runs": [{"name": "ci", "conclusion": "success"}]}),  // check-runs
        ]);
        let result = server.github_prs_checks(Parameters(IssueNumberFieldsParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            fields: None, format: None,
        })).await.unwrap();
        let text = ok_text(&result);
        assert!(text.contains("ci") || text.contains("success"));
    }

    #[tokio::test]
    async fn test_tool_prs_status() {
        // status calls get() first (for SHA), then the status endpoint
        let server = mock_server(vec![
            json!({"head": {"sha": "abc123"}}),  // PR get
            json!({"state": "success", "total_count": 2}),  // status
        ]);
        let result = server.github_prs_status(Parameters(IssueNumberFieldsParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("success"));
    }

    #[tokio::test]
    async fn test_tool_prs_review_create() {
        let server = mock_server(vec![json!({"id": 1, "state": "APPROVED"})]);
        let result = server.github_prs_review_create(Parameters(PrReviewCreateParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            event: "APPROVE".into(), body: Some("LGTM".into()),
        })).await.unwrap();
        assert!(ok_text(&result).contains("APPROVED"));
    }

    #[tokio::test]
    async fn test_tool_prs_update_branch() {
        let server = mock_server(vec![json!({"message": "Updating pull request branch"})]);
        let result = server.github_prs_update_branch(Parameters(PrDiffParams {
            owner: "o".into(), repo: "r".into(), number: 1,
        })).await.unwrap();
        assert!(ok_text(&result).contains("Updating"));
    }

    #[tokio::test]
    async fn test_tool_prs_search() {
        let server = mock_server(vec![json!({"items": [{"title": "Found PR"}]})]);
        let result = server.github_prs_search(Parameters(SearchParams {
            query: "author:alice".into(), limit: None, fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("Found PR"));
    }

    #[tokio::test]
    async fn test_tool_prs_add_review_comment() {
        // add_review_comment calls get() first for commit SHA
        let server = mock_server(vec![
            json!({"head": {"sha": "commit123"}}),  // PR get
            json!({"id": 1, "body": "inline comment"}),  // comment creation
        ]);
        let result = server.github_prs_add_review_comment(Parameters(PrReviewCommentParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            path: "src/lib.rs".into(), body: "inline comment".into(),
            line: Some(42), side: Some("RIGHT".into()),
            subject_type: Some("LINE".into()),
        })).await.unwrap();
        assert!(ok_text(&result).contains("inline comment"));
    }

    #[tokio::test]
    async fn test_tool_prs_reply_to_comment() {
        let server = mock_server(vec![json!({"id": 2, "body": "Thanks"})]);
        let result = server.github_prs_reply_to_comment(Parameters(PrReplyCommentParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            comment_id: 999, body: "Thanks".into(),
        })).await.unwrap();
        assert!(ok_text(&result).contains("Thanks"));
    }

    #[tokio::test]
    async fn test_tool_prs_submit_review() {
        let server = mock_server(vec![json!({"id": 42, "state": "APPROVED"})]);
        let result = server.github_prs_submit_review(Parameters(PrSubmitReviewParams {
            owner: "o".into(), repo: "r".into(), number: 1,
            review_id: 42, event: "APPROVE".into(), body: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("APPROVED"));
    }

    #[tokio::test]
    async fn test_tool_prs_delete_review() {
        let server = mock_server(vec![json!({"id": 42, "state": "PENDING"})]);
        let result = server.github_prs_delete_review(Parameters(PrDeleteReviewParams {
            owner: "o".into(), repo: "r".into(), number: 1, review_id: 42,
        })).await.unwrap();
        assert!(ok_text(&result).contains("PENDING"));
    }

    // --- Files ---

    #[tokio::test]
    async fn test_tool_files_get() {
        let server = mock_server(vec![json!({"name": "lib.rs", "content": "fn main(){}"})]);
        let result = server.github_files_get(Parameters(FileGetParams {
            owner: "o".into(), repo: "r".into(), path: "src/lib.rs".into(),
            git_ref: Some("main".into()), fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("lib.rs"));
    }

    #[tokio::test]
    async fn test_tool_files_create_or_update() {
        let server = mock_server(vec![json!({"content": {"path": "README.md"}})]);
        let result = server.github_files_create_or_update(Parameters(FileCreateOrUpdateParams {
            owner: "o".into(), repo: "r".into(), path: "README.md".into(),
            content: "SGVsbG8=".into(), message: "add readme".into(),
            branch: "main".into(), sha: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("README.md"));
    }

    #[tokio::test]
    async fn test_tool_files_delete() {
        let server = mock_server(vec![json!({"commit": {"sha": "abc123", "message": "deleted", "author": {"name": "jw", "date": "2026-03-18T12:00:00Z"}}})]);
        let result = server.github_files_delete(Parameters(FileDeleteParams {
            owner: "o".into(), repo: "r".into(), path: "old.txt".into(),
            message: "remove".into(), branch: "main".into(), sha: "def456".into(),
        })).await.unwrap();
        assert!(ok_text(&result).contains("deleted"));
    }

    #[tokio::test]
    async fn test_tool_files_push() {
        // push_files uses Git Data API: ref → commit → blob → tree → commit → update ref
        let server = mock_server(vec![
            json!({"object": {"sha": "commit_abc"}}),   // GET ref
            json!({"tree": {"sha": "tree_abc"}}),        // GET commit
            json!({"sha": "blob_abc"}),                   // POST blob
            json!({"sha": "newtree_abc"}),                // POST tree
            json!({"sha": "newcommit_abc"}),              // POST commit
            json!({"ref": "refs/heads/main"}),             // PATCH ref
        ]);
        let result = server.github_files_push(Parameters(FilePushParams {
            owner: "o".into(), repo: "r".into(), branch: "main".into(),
            message: "push files".into(),
            files_json: r#"[{"path":"a.txt","content":"aGk="}]"#.into(),
        })).await.unwrap();
        assert!(ok_text(&result).contains("files_pushed"));
    }

    // --- Repos ---

    #[tokio::test]
    async fn test_tool_repos_get() {
        let server = mock_server(vec![json!({"full_name": "o/r", "private": false})]);
        let result = server.github_repos_get(Parameters(RepoFieldsParams {
            owner: "o".into(), repo: "r".into(), fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("o/r"));
    }

    #[tokio::test]
    async fn test_tool_repos_create() {
        let server = mock_server(vec![json!({"full_name": "alice/new-project"})]);
        let result = server.github_repos_create(Parameters(RepoCreateParams {
            name: "new-project".into(), description: Some("test".into()),
            private: Some(true), org: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("new-project"));
    }

    #[tokio::test]
    async fn test_tool_repos_fork() {
        let server = mock_server(vec![json!({"full_name": "alice/project"})]);
        let result = server.github_repos_fork(Parameters(RepoForkParams {
            owner: "upstream".into(), repo: "project".into(), org: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("project"));
    }

    #[tokio::test]
    async fn test_tool_repos_search() {
        let server = mock_server(vec![json!({"items": [{"full_name": "rust-lang/rust"}]})]);
        let result = server.github_repos_search(Parameters(SearchParams {
            query: "language:rust".into(), limit: None, fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("rust"));
    }

    #[tokio::test]
    async fn test_tool_repos_compare() {
        let server = mock_server(vec![json!({"status": "ahead", "ahead_by": 3})]);
        let result = server.github_repos_compare(Parameters(CompareParams {
            owner: "o".into(), repo: "r".into(),
            base: "main".into(), head: "feature".into(),
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("ahead"));
    }

    // --- Branches ---

    #[tokio::test]
    async fn test_tool_branches_list() {
        let server = mock_server(vec![json!([{"name": "main"}, {"name": "dev"}])]);
        let result = server.github_branches_list(Parameters(RepoFieldsParams {
            owner: "o".into(), repo: "r".into(), fields: None, format: None,
        })).await.unwrap();
        let text = ok_text(&result);
        assert!(text.contains("main"));
        assert!(text.contains("dev"));
    }

    #[tokio::test]
    async fn test_tool_branches_create() {
        let server = mock_server(vec![json!({"ref": "refs/heads/feature-x"})]);
        let result = server.github_branches_create(Parameters(BranchCreateParams {
            owner: "o".into(), repo: "r".into(),
            branch: "feature-x".into(), from_sha: "abc123".into(),
        })).await.unwrap();
        assert!(ok_text(&result).contains("feature-x"));
    }

    // --- Commits ---

    #[tokio::test]
    async fn test_tool_commits_list() {
        let server = mock_server(vec![json!([{"sha": "abc", "commit": {"message": "init"}}])]);
        let result = server.github_commits_list(Parameters(CommitsListParams {
            owner: "o".into(), repo: "r".into(), sha: Some("main".into()),
            limit: Some(5), fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("init"));
    }

    #[tokio::test]
    async fn test_tool_commits_get() {
        let server = mock_server(vec![json!({"sha": "deadbeef", "commit": {"message": "fix bug"}})]);
        let result = server.github_commits_get(Parameters(CommitGetParams {
            owner: "o".into(), repo: "r".into(), sha: "deadbeef".into(),
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("fix bug"));
    }

    // --- Releases ---

    #[tokio::test]
    async fn test_tool_releases_list() {
        let server = mock_server(vec![json!([{"tag_name": "v1.0"}, {"tag_name": "v0.9"}])]);
        let result = server.github_releases_list(Parameters(RepoLimitParams {
            owner: "o".into(), repo: "r".into(), limit: None,
            fields: None, format: None,
        })).await.unwrap();
        let text = ok_text(&result);
        assert!(text.contains("v1.0"));
    }

    #[tokio::test]
    async fn test_tool_releases_get_by_tag() {
        let server = mock_server(vec![json!({"tag_name": "v1.0", "name": "Release 1.0"})]);
        let result = server.github_releases_get_by_tag(Parameters(ReleaseByTagParams {
            owner: "o".into(), repo: "r".into(), tag: "v1.0".into(),
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("Release 1.0"));
    }

    #[tokio::test]
    async fn test_tool_releases_latest() {
        let server = mock_server(vec![json!({"tag_name": "v2.0", "name": "Latest"})]);
        let result = server.github_releases_latest(Parameters(RepoFieldsParams {
            owner: "o".into(), repo: "r".into(), fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("Latest"));
    }

    #[tokio::test]
    async fn test_tool_releases_create() {
        let server = mock_server(vec![json!({"tag_name": "v3.0", "draft": false})]);
        let result = server.github_releases_create(Parameters(ReleaseCreateParams {
            owner: "o".into(), repo: "r".into(), tag_name: "v3.0".into(),
            name: Some("Release 3.0".into()), body: Some("Notes".into()),
            draft: Some(false), prerelease: Some(false),
        })).await.unwrap();
        assert!(ok_text(&result).contains("v3.0"));
    }

    // --- Tags ---

    #[tokio::test]
    async fn test_tool_tags_list() {
        let server = mock_server(vec![json!([{"name": "v1.0"}, {"name": "v2.0"}])]);
        let result = server.github_tags_list(Parameters(RepoLimitParams {
            owner: "o".into(), repo: "r".into(), limit: None,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("v1.0"));
    }

    #[tokio::test]
    async fn test_tool_tags_get() {
        let server = mock_server(vec![json!({"ref": "refs/tags/v1.0", "object": {"sha": "abc"}})]);
        let result = server.github_tags_get(Parameters(TagGetParams {
            owner: "o".into(), repo: "r".into(), tag: "v1.0".into(),
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("v1.0"));
    }

    // --- Teams ---

    #[tokio::test]
    async fn test_tool_teams_list() {
        let server = mock_server(vec![json!([{"name": "core", "slug": "core"}])]);
        let result = server.github_teams_list(Parameters(FieldsOnlyParams {
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("core"));
    }

    #[tokio::test]
    async fn test_tool_teams_members() {
        let server = mock_server(vec![json!([{"login": "alice"}, {"login": "bob"}])]);
        let result = server.github_teams_members(Parameters(TeamMembersParams {
            org: "my-org".into(), team_slug: "core".into(),
            fields: None, format: None,
        })).await.unwrap();
        let text = ok_text(&result);
        assert!(text.contains("alice"));
        assert!(text.contains("bob"));
    }

    // --- Users ---

    #[tokio::test]
    async fn test_tool_user_me() {
        let server = mock_server(vec![json!({"login": "jw409", "id": 1})]);
        let result = server.github_user_me(Parameters(FieldsOnlyParams {
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("jw409"));
    }

    #[tokio::test]
    async fn test_tool_users_search() {
        let server = mock_server(vec![json!({"items": [{"login": "alice"}]})]);
        let result = server.github_users_search(Parameters(SearchParams {
            query: "location:sf".into(), limit: None, fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("alice"));
    }

    // --- Code Search ---

    #[tokio::test]
    async fn test_tool_code_search() {
        let server = mock_server(vec![json!({"items": [{"path": "src/main.rs"}]})]);
        let result = server.github_code_search(Parameters(SearchParams {
            query: "fn main".into(), limit: None, fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("main.rs"));
    }

    // --- Labels ---

    #[tokio::test]
    async fn test_tool_labels_get() {
        let server = mock_server(vec![json!({"name": "bug", "color": "ff0000"})]);
        let result = server.github_labels_get(Parameters(LabelGetParams {
            owner: "o".into(), repo: "r".into(), name: "bug".into(),
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("bug"));
    }

    #[tokio::test]
    async fn test_tool_labels_list() {
        let server = mock_server(vec![json!([{"name": "bug"}, {"name": "enhancement"}])]);
        let result = server.github_labels_list(Parameters(RepoLimitParams {
            owner: "o".into(), repo: "r".into(), limit: None,
            fields: None, format: None,
        })).await.unwrap();
        let text = ok_text(&result);
        assert!(text.contains("bug"));
        assert!(text.contains("enhancement"));
    }

    #[tokio::test]
    async fn test_tool_labels_create() {
        let server = mock_server(vec![json!({"name": "priority", "color": "00ff00"})]);
        let result = server.github_labels_create(Parameters(LabelCreateParams {
            owner: "o".into(), repo: "r".into(), name: "priority".into(),
            color: Some("00ff00".into()), description: Some("Priority label".into()),
        })).await.unwrap();
        assert!(ok_text(&result).contains("priority"));
    }

    #[tokio::test]
    async fn test_tool_labels_update() {
        let server = mock_server(vec![json!({"name": "defect", "color": "ff0000"})]);
        let result = server.github_labels_update(Parameters(LabelUpdateParams {
            owner: "o".into(), repo: "r".into(), name: "bug".into(),
            new_name: Some("defect".into()), color: Some("ff0000".into()),
            description: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("defect"));
    }

    #[tokio::test]
    async fn test_tool_labels_delete() {
        let server = mock_server(vec![json!(null)]);
        let result = server.github_labels_delete(Parameters(LabelDeleteParams {
            owner: "o".into(), repo: "r".into(), name: "stale".into(),
        })).await.unwrap();
        // Delete returns null, should not error
        let _ = ok_text(&result);
    }

    // --- Actions ---

    #[tokio::test]
    async fn test_tool_actions_list_runs() {
        let server = mock_server(vec![json!({"workflow_runs": [{"id": 1, "status": "completed"}]})]);
        let result = server.github_actions_list_runs(Parameters(RepoLimitParams {
            owner: "o".into(), repo: "r".into(), limit: None,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("completed"));
    }

    #[tokio::test]
    async fn test_tool_actions_get_run() {
        let server = mock_server(vec![json!({"id": 123, "status": "completed", "conclusion": "success"})]);
        let result = server.github_actions_get_run(Parameters(ActionRunIdParams {
            owner: "o".into(), repo: "r".into(), run_id: 123,
            fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("success"));
    }

    #[tokio::test]
    async fn test_tool_actions_rerun() {
        let server = mock_server(vec![json!(null)]);
        let result = server.github_actions_rerun(Parameters(ActionRunIdOnlyParams {
            owner: "o".into(), repo: "r".into(), run_id: 456,
        })).await.unwrap();
        let _ = ok_text(&result);
    }

    #[tokio::test]
    async fn test_tool_actions_list_workflows() {
        let server = mock_server(vec![json!({"workflows": [{"id": 1, "name": "CI"}]})]);
        let result = server.github_actions_list_workflows(Parameters(RepoFieldsParams {
            owner: "o".into(), repo: "r".into(), fields: None, format: None,
        })).await.unwrap();
        assert!(ok_text(&result).contains("CI"));
    }

    #[tokio::test]
    async fn test_tool_actions_run_logs() {
        let server = mock_server(vec![json!("https://example.com/logs.zip")]);
        let result = server.github_actions_run_logs(Parameters(ActionRunIdOnlyParams {
            owner: "o".into(), repo: "r".into(), run_id: 789,
        })).await.unwrap();
        assert!(ok_text(&result).contains("logs"));
    }
}
