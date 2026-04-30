use serde_json::Value;

use crate::github::client::{ClientError, GithubClient};

/// List workflow runs for a repository.
pub async fn list_runs(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/actions/runs");
    let result = client.api_list(&endpoint, &[], limit).await?;
    // Extract workflow_runs array if wrapped
    if let Value::Object(ref map) = result {
        if let Some(runs) = map.get("workflow_runs") {
            return Ok(runs.clone());
        }
    }
    Ok(result)
}

/// Get a specific workflow run.
pub async fn get_run(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    run_id: u64,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/actions/runs/{run_id}");
    client.api(&endpoint, &[]).await
}

/// Re-run a workflow run.
pub async fn rerun(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    run_id: u64,
) -> Result<Value, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/actions/runs/{run_id}/rerun");
    client.api(&endpoint, &["-X", "POST"]).await
}

/// List workflows for a repository.
pub async fn list_workflows(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    limit: Option<u32>,
) -> Result<Value, ClientError> {
    // GitHub returns `{total_count, workflows: [...]}`, so we can't use
    // api_list directly. Cap server-side via `per_page` (max 100), then
    // unwrap and trim client-side to the requested limit.
    let per_page = limit.unwrap_or(30).min(100);
    let endpoint = format!("/repos/{owner}/{repo}/actions/workflows?per_page={per_page}");
    let result = client.api(&endpoint, &[]).await?;
    if let Value::Object(map) = &result {
        if let Some(workflows) = map.get("workflows").cloned() {
            if let Some(mut arr) = workflows.as_array().cloned() {
                if let Some(cap) = limit {
                    arr.truncate(cap as usize);
                }
                return Ok(Value::Array(arr));
            }
            return Ok(workflows);
        }
    }
    Ok(result)
}

/// Get logs URL for a workflow run.
///
/// Returns the redirect URL for downloading run logs.
pub async fn run_logs(
    client: &GithubClient,
    owner: &str,
    repo: &str,
    run_id: u64,
) -> Result<String, ClientError> {
    let endpoint = format!("/repos/{owner}/{repo}/actions/runs/{run_id}/logs");
    client.api_raw(&endpoint, "application/vnd.github.v3+json").await
}

#[cfg(test)]
fn list_runs_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/actions/runs")
}

#[cfg(test)]
fn get_run_endpoint(owner: &str, repo: &str, run_id: u64) -> String {
    format!("/repos/{owner}/{repo}/actions/runs/{run_id}")
}

#[cfg(test)]
fn rerun_endpoint(owner: &str, repo: &str, run_id: u64) -> String {
    format!("/repos/{owner}/{repo}/actions/runs/{run_id}/rerun")
}

#[cfg(test)]
fn list_workflows_endpoint(owner: &str, repo: &str) -> String {
    format!("/repos/{owner}/{repo}/actions/workflows")
}

#[cfg(test)]
fn run_logs_endpoint(owner: &str, repo: &str, run_id: u64) -> String {
    format!("/repos/{owner}/{repo}/actions/runs/{run_id}/logs")
}

#[cfg(test)]
fn extract_workflow_runs(result: &Value) -> Value {
    if let Value::Object(ref map) = result {
        if let Some(runs) = map.get("workflow_runs") {
            return runs.clone();
        }
    }
    result.clone()
}

#[cfg(test)]
fn extract_workflows(result: &Value) -> Value {
    if let Value::Object(ref map) = result {
        if let Some(workflows) = map.get("workflows") {
            return workflows.clone();
        }
    }
    result.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_list_runs_endpoint() {
        assert_eq!(list_runs_endpoint("o", "r"), "/repos/o/r/actions/runs");
    }

    #[test]
    fn test_get_run_endpoint() {
        assert_eq!(get_run_endpoint("o", "r", 123), "/repos/o/r/actions/runs/123");
    }

    #[test]
    fn test_rerun_endpoint() {
        assert_eq!(rerun_endpoint("o", "r", 456), "/repos/o/r/actions/runs/456/rerun");
    }

    #[test]
    fn test_list_workflows_endpoint() {
        assert_eq!(list_workflows_endpoint("o", "r"), "/repos/o/r/actions/workflows");
    }

    #[test]
    fn test_run_logs_endpoint() {
        assert_eq!(run_logs_endpoint("o", "r", 789), "/repos/o/r/actions/runs/789/logs");
    }

    #[test]
    fn test_extract_workflow_runs() {
        let result = json!({"total_count": 1, "workflow_runs": [{"id": 1}]});
        let runs = extract_workflow_runs(&result);
        assert_eq!(runs.as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_extract_workflow_runs_fallback() {
        let result = json!([{"id": 1}]);
        let runs = extract_workflow_runs(&result);
        assert!(runs.is_array());
    }

    #[test]
    fn test_extract_workflows() {
        let result = json!({"total_count": 2, "workflows": [{"id": 1}, {"id": 2}]});
        let wf = extract_workflows(&result);
        assert_eq!(wf.as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_extract_workflows_fallback() {
        let result = json!({"data": "other"});
        let wf = extract_workflows(&result);
        assert!(wf.is_object());
    }

    // --- Async tests with mock client ---

    #[tokio::test]
    async fn test_list_runs_fn() {
        let client = GithubClient::mock(vec![json!({"workflow_runs": [{"id": 1}]})]);
        let result = list_runs(&client, "o", "r", Some(5)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_list_runs_no_wrapper() {
        let client = GithubClient::mock(vec![json!([{"id": 1}])]);
        let result = list_runs(&client, "o", "r", None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_get_run_fn() {
        let client = GithubClient::mock(vec![json!({"id": 123, "status": "completed"})]);
        let result = get_run(&client, "o", "r", 123).await.unwrap();
        assert_eq!(result["id"], 123);
    }

    #[tokio::test]
    async fn test_rerun_fn() {
        let client = GithubClient::mock(vec![json!(null)]);
        let result = rerun(&client, "o", "r", 456).await.unwrap();
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn test_list_workflows_fn() {
        let client = GithubClient::mock(vec![json!({"workflows": [{"id": 1, "name": "CI"}]})]);
        let result = list_workflows(&client, "o", "r", None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_list_workflows_no_wrapper() {
        let client = GithubClient::mock(vec![json!([{"id": 1}])]);
        let result = list_workflows(&client, "o", "r", None).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_list_workflows_respects_limit() {
        let client = GithubClient::mock(vec![json!({"workflows": [
            {"id": 1, "name": "a"}, {"id": 2, "name": "b"}, {"id": 3, "name": "c"},
        ]})]);
        let result = list_workflows(&client, "o", "r", Some(2)).await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_run_logs_fn() {
        let client = GithubClient::mock(vec![json!("https://example.com/logs.zip")]);
        let result = run_logs(&client, "o", "r", 789).await.unwrap();
        assert!(result.contains("logs"));
    }
}
