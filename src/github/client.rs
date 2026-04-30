use reqwest::header::{ACCEPT, AUTHORIZATION, LINK};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;

/// HTTP request timeout (connect + response).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Connection establishment timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Subprocess execution timeout.
const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(60);
/// Maximum number of pages to fetch before stopping (circuit breaker).
const MAX_PAGES: usize = 50;
/// Maximum number of retries for 429/5xx responses.
const MAX_RETRIES: u32 = 3;
/// Overall budget for a single tool call. Bounds the worst-case interaction
/// of retries (`MAX_RETRIES` × Retry-After up to 120s), pagination
/// (`MAX_PAGES` × `REQUEST_TIMEOUT`), and parse time. Picked to comfortably
/// exceed normal slow paths (a 50-page paginate returning ~1s/page takes
/// ~50s) while ensuring the MCP server can't hang a caller indefinitely.
const OVERALL_CALL_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("GitHub API error: {0}")]
    Api(String),
    #[error("gh CLI error: {0}")]
    Cli(String),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Timeout: {0}")]
    Timeout(String),
}

enum Backend {
    /// Direct HTTP via reqwest (fast, no subprocess)
    Http { client: reqwest::Client, token: String },
    /// Fallback to `gh` CLI (slower, but handles auth edge cases)
    GhCli,
    /// Mock backend for testing — returns canned responses in order
    #[cfg(test)]
    Mock {
        responses: std::sync::Mutex<std::collections::VecDeque<Result<Value, ClientError>>>,
    },
}

pub struct GithubClient {
    backend: Backend,
    base_url: String,
}

const GITHUB_API: &str = "https://api.github.com";

impl GithubClient {
    pub fn new(token: &str) -> Self {
        if token.is_empty() {
            tracing::info!("No token provided, using gh CLI fallback");
            return Self { backend: Backend::GhCli, base_url: GITHUB_API.to_string() };
        }

        match reqwest::Client::builder()
            .user_agent("kp-github-mcp/0.1")
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
        {
            Ok(client) => {
                tracing::info!("Using reqwest HTTP backend");
                Self {
                    backend: Backend::Http {
                        client,
                        token: token.to_string(),
                    },
                    base_url: GITHUB_API.to_string(),
                }
            }
            Err(e) => {
                tracing::warn!("Failed to build HTTP client: {e}, falling back to gh CLI");
                Self { backend: Backend::GhCli, base_url: GITHUB_API.to_string() }
            }
        }
    }

    /// Call GitHub API — GET request
    pub async fn api(&self, endpoint: &str, args: &[&str]) -> Result<Value, ClientError> {
        validate_endpoint(endpoint)?;
        // Overall budget across the retry loop. Without this, MAX_RETRIES (3)
        // × Retry-After cap (120s) plus per-request timeout could keep a tool
        // call live for several minutes — long enough to look like a hang to
        // the MCP caller. Bound it.
        with_overall_timeout(endpoint, async {
            match &self.backend {
                Backend::Http { client, token } => {
                    let method = extract_method(args);
                    let fields = extract_fields(args);
                    let url = format!("{}{endpoint}", self.base_url);
                    let body = if !fields.is_empty() { Some(fields_to_json(&fields)) } else { None };

                    let mut retries = 0u32;
                    loop {
                        let req = match method {
                            "POST" => {
                                let r = client.post(&url);
                                if let Some(ref b) = body { r.json(b) } else { r }
                            }
                            "PATCH" => {
                                let r = client.patch(&url);
                                if let Some(ref b) = body { r.json(b) } else { r }
                            }
                            "PUT" => {
                                let r = client.put(&url);
                                if let Some(ref b) = body { r.json(b) } else { r }
                            }
                            "DELETE" => {
                                let r = client.delete(&url);
                                if let Some(ref b) = body { r.json(b) } else { r }
                            }
                            _ => client.get(&url),
                        };

                        let resp = req
                            .header(AUTHORIZATION, format!("Bearer {token}"))
                            .header(ACCEPT, "application/vnd.github+json")
                            .header("X-GitHub-Api-Version", "2022-11-28")
                            .send()
                            .await?;

                        check_rate_limit_and_retry(&resp);

                        let status = resp.status();
                        if is_retryable(status) && retries < MAX_RETRIES {
                            let wait = retry_delay(&resp, retries);
                            tracing::warn!("GitHub {status}, retrying in {wait:?} (attempt {}/{})", retries + 1, MAX_RETRIES);
                            tokio::time::sleep(wait).await;
                            retries += 1;
                            continue;
                        }

                        if !status.is_success() {
                            let body = resp.text().await.unwrap_or_default();
                            return Err(ClientError::Api(format!("{status}: {body}")));
                        }

                        let text = resp.text().await?;
                        if text.trim().is_empty() {
                            return Ok(Value::Null);
                        }
                        return Ok(serde_json::from_str(&text)?);
                    }
                }
                Backend::GhCli => self.gh_cli_api(endpoint, args).await,
                #[cfg(test)]
                Backend::Mock { responses } => {
                    let mut q = responses.lock().unwrap();
                    q.pop_front().unwrap_or(Ok(Value::Null))
                }
            }
        }).await
    }

    /// List endpoint with query params — paginates automatically for limit > 100.
    pub async fn api_list(
        &self,
        endpoint: &str,
        query_params: &[(&str, &str)],
        limit: Option<u32>,
    ) -> Result<Value, ClientError> {
        validate_endpoint(endpoint)?;
        let effective_limit = limit.unwrap_or(30);
        let per_page = effective_limit.min(100);

        let mut url = format!("{endpoint}?per_page={per_page}");
        for (k, v) in query_params {
            url.push_str(&format!("&{k}={}", crate::util::urlencode(v)));
        }

        // Overall budget across all pages. Without this a 50-page paginate
        // crawl combined with slow GitHub responses could keep the MCP loop
        // unresponsive long enough to look hung. Bound it.
        with_overall_timeout(endpoint, async {
        match &self.backend {
            Backend::Http { client, token } => {
                let mut all_items: Vec<Value> = Vec::new();
                let mut next_url = Some(format!("{}{url}", self.base_url));
                let mut page_count = 0usize;

                while let Some(current_url) = next_url.take() {
                    page_count += 1;
                    if page_count > MAX_PAGES {
                        tracing::warn!("Pagination circuit breaker: stopped after {MAX_PAGES} pages");
                        break;
                    }

                    let resp = client
                        .get(&current_url)
                        .header(AUTHORIZATION, format!("Bearer {token}"))
                        .header(ACCEPT, "application/vnd.github+json")
                        .header("X-GitHub-Api-Version", "2022-11-28")
                        .send()
                        .await?;

                    check_rate_limit_and_retry(&resp);

                    if !resp.status().is_success() {
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        return Err(ClientError::Api(format!("{status}: {body}")));
                    }

                    // Parse Link header for next page
                    let link_next = resp
                        .headers()
                        .get(LINK)
                        .and_then(|v| v.to_str().ok())
                        .and_then(parse_link_next)
                        .map(|s| s.to_string());

                    let text = resp.text().await?;
                    let page: Value = serde_json::from_str(&text)?;

                    if let Some(arr) = page.as_array() {
                        all_items.extend(arr.into_iter().cloned());
                    } else {
                        // Non-array response — return as-is (shouldn't happen for list endpoints)
                        return Ok(page);
                    }

                    // Stop if we have enough or no more pages
                    if all_items.len() >= effective_limit as usize {
                        all_items.truncate(effective_limit as usize);
                        break;
                    }

                    next_url = link_next;
                }

                Ok(Value::Array(all_items))
            }
            Backend::GhCli => {
                // gh CLI handles pagination natively with --paginate
                if effective_limit > 100 {
                    let args = vec![
                        "--paginate".to_string(),
                        "--slurp".to_string(),
                    ];
                    // Build the URL with query params
                    let full_url = url.clone();
                    let mut cmd = tokio::process::Command::new("gh");
                    cmd.args(["api", &full_url]);
                    cmd.args(&args);
                    cmd.stdin(std::process::Stdio::null());

                    let output = tokio::time::timeout(
                        SUBPROCESS_TIMEOUT,
                        cmd.output(),
                    )
                    .await
                    .map_err(|_| ClientError::Timeout(format!("gh api pagination timed out after {SUBPROCESS_TIMEOUT:?}")))?
                    .map_err(|e| ClientError::Cli(e.to_string()))?;
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(ClientError::Api(stderr.to_string()));
                    }
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if stdout.trim().is_empty() {
                        return Ok(Value::Array(Vec::new()));
                    }
                    // --slurp wraps pages in an array of arrays; flatten
                    let pages: Value = serde_json::from_str(&stdout)?;
                    if let Some(outer) = pages.as_array() {
                        let mut all: Vec<Value> = Vec::new();
                        for page in outer {
                            if let Some(arr) = page.as_array() {
                                all.extend(arr.iter().cloned());
                            }
                        }
                        all.truncate(effective_limit as usize);
                        Ok(Value::Array(all))
                    } else {
                        Ok(pages)
                    }
                } else {
                    self.api(&url, &[]).await
                }
            }
            #[cfg(test)]
            Backend::Mock { responses } => {
                // For mock: pop all queued responses and merge arrays
                let mut q = responses.lock().unwrap();
                let mut all_items: Vec<Value> = Vec::new();
                // Pop first response
                match q.pop_front() {
                    Some(Ok(page)) => {
                        if let Some(arr) = page.as_array() {
                            all_items.extend(arr.iter().cloned());
                        } else {
                            return Ok(page);
                        }
                    }
                    Some(Err(e)) => return Err(e),
                    None => return Ok(Value::Array(Vec::new())),
                }

                // If limit > 100, keep popping pages (simulates pagination)
                if effective_limit > 100 {
                    while all_items.len() < effective_limit as usize {
                        match q.pop_front() {
                            Some(Ok(page)) => {
                                if let Some(arr) = page.as_array() {
                                    if arr.is_empty() {
                                        break;
                                    }
                                    all_items.extend(arr.iter().cloned());
                                } else {
                                    break;
                                }
                            }
                            _ => break,
                        }
                    }
                    all_items.truncate(effective_limit as usize);
                }

                Ok(Value::Array(all_items))
            }
        }
        }).await
    }

    /// POST/PATCH/PUT a JSON body directly (not via -f key=value args).
    /// Used for Git Data API calls that need nested objects/arrays.
    pub async fn api_json(
        &self,
        endpoint: &str,
        method: &str,
        body: &Value,
    ) -> Result<Value, ClientError> {
        validate_endpoint(endpoint)?;
        match &self.backend {
            Backend::Http { client, token } => {
                let url = format!("{}{endpoint}", self.base_url);
                let req = match method {
                    "POST" => client.post(&url),
                    "PATCH" => client.patch(&url),
                    "PUT" => client.put(&url),
                    "DELETE" => client.delete(&url),
                    _ => client.get(&url),
                };

                let resp = req
                    .header(AUTHORIZATION, format!("Bearer {token}"))
                    .header(ACCEPT, "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .json(body)
                    .send()
                    .await?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(ClientError::Api(format!("{status}: {body}")));
                }

                let text = resp.text().await?;
                if text.trim().is_empty() {
                    return Ok(Value::Null);
                }
                Ok(serde_json::from_str(&text)?)
            }
            Backend::GhCli => {
                // Serialize body to JSON string and pass via --input
                let body_str = serde_json::to_string(body)
                    .map_err(|e| ClientError::Cli(format!("JSON serialize: {e}")))?;
                let mut cmd = tokio::process::Command::new("gh");
                cmd.args(["api", endpoint, "-X", method, "--input", "-"]);
                cmd.stdin(std::process::Stdio::piped());
                cmd.stdout(std::process::Stdio::piped());
                cmd.stderr(std::process::Stdio::piped());

                let mut child = cmd.spawn().map_err(|e| ClientError::Cli(e.to_string()))?;

                if let Some(mut stdin) = child.stdin.take() {
                    use tokio::io::AsyncWriteExt;
                    stdin
                        .write_all(body_str.as_bytes())
                        .await
                        .map_err(|e| ClientError::Cli(e.to_string()))?;
                    drop(stdin);
                }

                let output = tokio::time::timeout(
                    SUBPROCESS_TIMEOUT,
                    child.wait_with_output(),
                )
                .await
                .map_err(|_| ClientError::Timeout(format!("gh api {endpoint} timed out after {SUBPROCESS_TIMEOUT:?}")))?
                .map_err(|e| ClientError::Cli(e.to_string()))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(ClientError::Api(stderr.to_string()));
                }

                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.trim().is_empty() {
                    return Ok(Value::Null);
                }
                Ok(serde_json::from_str(&stdout)?)
            }
            #[cfg(test)]
            Backend::Mock { responses } => {
                let mut q = responses.lock().unwrap();
                q.pop_front().unwrap_or(Ok(Value::Null))
            }
        }
    }

    /// Call GitHub API with a custom Accept header, returning raw text.
    pub async fn api_raw(&self, endpoint: &str, accept: &str) -> Result<String, ClientError> {
        validate_endpoint(endpoint)?;
        match &self.backend {
            Backend::Http { client, token } => {
                let url = format!("{}{endpoint}", self.base_url);
                let resp = client
                    .get(&url)
                    .header(AUTHORIZATION, format!("Bearer {token}"))
                    .header(ACCEPT, accept)
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .send()
                    .await?;

                check_rate_limit_and_retry(&resp);

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(ClientError::Api(format!("{status}: {body}")));
                }

                Ok(resp.text().await?)
            }
            #[cfg(test)]
            Backend::Mock { responses } => {
                let mut q = responses.lock().unwrap();
                match q.pop_front().unwrap_or(Ok(Value::Null)) {
                    Ok(v) => Ok(v.to_string()),
                    Err(e) => Err(e),
                }
            }
            Backend::GhCli => {
                let accept_header = format!("Accept:{accept}");
                let mut cmd = tokio::process::Command::new("gh");
                cmd.args(["api", endpoint, "--header", &accept_header]);
                cmd.stdin(std::process::Stdio::null());

                let output = tokio::time::timeout(
                    SUBPROCESS_TIMEOUT,
                    cmd.output(),
                )
                .await
                .map_err(|_| ClientError::Timeout(format!("gh api {endpoint} timed out after {SUBPROCESS_TIMEOUT:?}")))?
                .map_err(|e| ClientError::Cli(e.to_string()))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(ClientError::Api(stderr.to_string()));
                }

                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            }
        }
    }

    /// gh CLI fallback
    async fn gh_cli_api(&self, endpoint: &str, args: &[&str]) -> Result<Value, ClientError> {
        let mut cmd = tokio::process::Command::new("gh");
        cmd.args(["api", endpoint]);
        for arg in args {
            cmd.arg(arg);
        }
        // Prevent gh from consuming MCP's stdin
        cmd.stdin(std::process::Stdio::null());

        let output = tokio::time::timeout(
            SUBPROCESS_TIMEOUT,
            cmd.output(),
        )
        .await
        .map_err(|_| ClientError::Timeout(format!("gh api {endpoint} timed out after {SUBPROCESS_TIMEOUT:?}")))?
        .map_err(|e| ClientError::Cli(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClientError::Api(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            return Ok(Value::Null);
        }
        Ok(serde_json::from_str(&stdout)?)
    }
}

/// Wrap an API call future in `OVERALL_CALL_TIMEOUT`. On timeout returns
/// `ClientError::Timeout` naming the endpoint so MCP callers see a clean
/// error instead of a hung tool.
async fn with_overall_timeout<F, T>(endpoint: &str, fut: F) -> Result<T, ClientError>
where
    F: std::future::Future<Output = Result<T, ClientError>>,
{
    match tokio::time::timeout(OVERALL_CALL_TIMEOUT, fut).await {
        Ok(res) => res,
        Err(_) => Err(ClientError::Timeout(format!(
            "GitHub API call to {endpoint} exceeded overall budget of {OVERALL_CALL_TIMEOUT:?}"
        ))),
    }
}

/// Check if an HTTP status code is retryable (429 rate limit, 5xx server errors).
fn is_retryable(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

/// Calculate retry delay: use Retry-After header if present, otherwise exponential backoff.
fn retry_delay(resp: &reqwest::Response, attempt: u32) -> Duration {
    // Check Retry-After header (GitHub sends this on 429)
    if let Some(val) = resp.headers().get("retry-after") {
        if let Ok(secs) = val.to_str().unwrap_or("").parse::<u64>() {
            return Duration::from_secs(secs.min(120)); // cap at 2 minutes
        }
    }
    // Exponential backoff: 1s, 2s, 4s
    Duration::from_secs(1 << attempt)
}

/// Validate that an API endpoint does not contain path traversal sequences.
/// Allows `...` (GitHub compare syntax: `base...head`) but rejects `..` followed by `/`.
fn validate_endpoint(endpoint: &str) -> Result<(), ClientError> {
    // Reject "../" or "/.." path traversal patterns
    if endpoint.contains("../") || endpoint.contains("/..") {
        return Err(ClientError::Api(format!(
            "endpoint contains path traversal: {endpoint}"
        )));
    }
    Ok(())
}

/// Check rate limit headers and log warnings.
fn check_rate_limit_and_retry(resp: &reqwest::Response) {
    let remaining = resp
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u32>().ok());
    if let Some(rem) = remaining {
        if rem < 10 {
            tracing::error!("GitHub rate limit critical: {rem} requests remaining");
        } else if rem < 100 {
            tracing::warn!("GitHub rate limit low: {rem} requests remaining");
        }
    }
}

/// Parse the `Link` header to find the `rel="next"` URL.
/// Format: `<https://api.github.com/...?page=2>; rel="next", <...>; rel="last"`
fn parse_link_next(header: &str) -> Option<&str> {
    for part in header.split(',') {
        let part = part.trim();
        if part.contains("rel=\"next\"") {
            // Extract URL between < and >
            let start = part.find('<')? + 1;
            let end = part.find('>')?;
            return Some(&part[start..end]);
        }
    }
    None
}

/// Extract HTTP method from gh-style args: ["-X", "POST", ...] -> "POST"
fn extract_method<'a>(args: &[&'a str]) -> &'a str {
    for pair in args.windows(2) {
        if pair[0] == "-X" {
            return pair[1];
        }
    }
    "GET"
}

/// Extract -f key=value pairs from gh-style args
fn extract_fields<'a>(args: &[&'a str]) -> Vec<(&'a str, &'a str)> {
    let mut fields = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-f" && i + 1 < args.len() {
            if let Some((k, v)) = args[i + 1].split_once('=') {
                fields.push((k, v));
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    fields
}

/// Parse a string value into the appropriate JSON type.
/// "true"/"false" → Bool, integers → Number, everything else → String.
fn parse_field_value(val: &str) -> Value {
    match val {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        "null" => Value::Null,
        _ => {
            if let Ok(n) = val.parse::<i64>() {
                Value::Number(n.into())
            } else {
                Value::String(val.to_string())
            }
        }
    }
}

/// Convert -f key=value pairs to JSON object.
/// Handles array fields: "labels[]=bug" -> {"labels": ["bug"]}
/// Handles typed values: "draft=true" -> {"draft": true}, "per_page=100" -> {"per_page": 100}
fn fields_to_json(fields: &[(&str, &str)]) -> Value {
    let mut map = serde_json::Map::new();

    for &(key, val) in fields {
        if let Some(array_key) = key.strip_suffix("[]") {
            // Array field — array elements stay as strings (labels, assignees)
            let entry = map
                .entry(array_key.to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(arr) = entry {
                arr.push(Value::String(val.to_string()));
            }
        } else {
            map.insert(key.to_string(), parse_field_value(val));
        }
    }

    Value::Object(map)
}

#[cfg(test)]
impl GithubClient {
    /// Create a mock client that returns canned responses in order.
    /// Each call to `api()`, `api_list()`, or `api_raw()` pops the next response.
    pub fn mock(responses: Vec<Value>) -> Self {
        Self {
            backend: Backend::Mock {
                responses: std::sync::Mutex::new(
                    responses.into_iter().map(Ok).collect(),
                ),
            },
            base_url: GITHUB_API.to_string(),
        }
    }

    /// Create a mock client where some responses can be errors.
    pub fn mock_results(responses: Vec<Result<Value, ClientError>>) -> Self {
        Self {
            backend: Backend::Mock {
                responses: std::sync::Mutex::new(responses.into_iter().collect()),
            },
            base_url: GITHUB_API.to_string(),
        }
    }

    /// Create an HTTP client pointing at a custom base URL (for wiremock tests).
    pub fn http_with_base_url(base_url: &str, token: &str) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("kp-github-mcp/0.1")
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .build()
            .expect("Failed to build test HTTP client");
        Self {
            backend: Backend::Http {
                client,
                token: token.to_string(),
            },
            base_url: base_url.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_method_post() {
        assert_eq!(extract_method(&["-X", "POST", "-f", "title=foo"]), "POST");
    }

    #[test]
    fn test_extract_method_patch() {
        assert_eq!(extract_method(&["-X", "PATCH"]), "PATCH");
    }

    #[test]
    fn test_extract_method_default_get() {
        assert_eq!(extract_method(&["-f", "state=open"]), "GET");
    }

    #[test]
    fn test_extract_method_empty() {
        assert_eq!(extract_method(&[]), "GET");
    }

    #[test]
    fn test_extract_fields() {
        let fields = extract_fields(&["-f", "title=foo", "-f", "body=bar"]);
        assert_eq!(fields, vec![("title", "foo"), ("body", "bar")]);
    }

    #[test]
    fn test_extract_fields_skips_non_f() {
        let fields = extract_fields(&["-X", "POST", "-f", "title=foo"]);
        assert_eq!(fields, vec![("title", "foo")]);
    }

    #[test]
    fn test_fields_to_json_scalars() {
        let fields = vec![("title", "bug"), ("state", "open")];
        let result = fields_to_json(&fields);
        assert_eq!(result["title"], "bug");
        assert_eq!(result["state"], "open");
    }

    #[test]
    fn test_fields_to_json_arrays() {
        let fields = vec![("labels[]", "bug"), ("labels[]", "urgent")];
        let result = fields_to_json(&fields);
        assert_eq!(result, json!({"labels": ["bug", "urgent"]}));
    }

    #[test]
    fn test_fields_to_json_mixed() {
        let fields = vec![("title", "fix"), ("labels[]", "bug"), ("labels[]", "p1"), ("state", "open")];
        let result = fields_to_json(&fields);
        assert_eq!(result["title"], "fix");
        assert_eq!(result["state"], "open");
        assert_eq!(result["labels"], json!(["bug", "p1"]));
    }

    #[test]
    fn test_fields_to_json_empty() {
        let fields: Vec<(&str, &str)> = vec![];
        let result = fields_to_json(&fields);
        assert_eq!(result, json!({}));
    }

    #[test]
    fn test_extract_method_put() {
        assert_eq!(extract_method(&["-X", "PUT"]), "PUT");
    }

    #[test]
    fn test_extract_method_delete() {
        assert_eq!(extract_method(&["-X", "DELETE"]), "DELETE");
    }

    #[test]
    fn test_extract_fields_empty() {
        let fields = extract_fields(&[]);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_extract_fields_f_at_end() {
        // -f at the very end without a value
        let fields = extract_fields(&["-f"]);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_extract_fields_no_equals() {
        // -f with a value that has no = sign
        let fields = extract_fields(&["-f", "noequals"]);
        assert!(fields.is_empty());
    }

    #[test]
    fn test_extract_fields_multiple() {
        let fields = extract_fields(&[
            "-f", "a=1", "-f", "b=2", "-f", "c=3",
        ]);
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0], ("a", "1"));
        assert_eq!(fields[1], ("b", "2"));
        assert_eq!(fields[2], ("c", "3"));
    }

    #[test]
    fn test_extract_fields_value_with_equals() {
        // Value containing = should split on first =
        let fields = extract_fields(&["-f", "body=line1=line2"]);
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0], ("body", "line1=line2"));
    }

    #[test]
    fn test_fields_to_json_single_array_element() {
        let fields = vec![("labels[]", "bug")];
        let result = fields_to_json(&fields);
        assert_eq!(result, json!({"labels": ["bug"]}));
    }

    #[test]
    fn test_fields_to_json_value_with_special_chars() {
        let fields = vec![("body", "hello world & goodbye")];
        let result = fields_to_json(&fields);
        assert_eq!(result["body"], "hello world & goodbye");
    }

    #[test]
    fn test_github_client_new_empty_token() {
        // Empty token should create GhCli backend (no panic)
        let client = GithubClient::new("");
        // Can't easily test internal backend, but construction shouldn't panic
        drop(client);
    }

    #[test]
    fn test_github_client_new_with_token() {
        let client = GithubClient::new("ghp_test123");
        drop(client);
    }

    #[test]
    fn test_extract_method_x_in_value() {
        // -X appearing as a value shouldn't be treated as method flag
        let result = extract_method(&["-f", "method=-X"]);
        assert_eq!(result, "GET");
    }

    #[test]
    fn test_fields_to_json_multiple_arrays() {
        let fields = vec![
            ("labels[]", "bug"),
            ("assignees[]", "alice"),
            ("labels[]", "urgent"),
            ("assignees[]", "bob"),
        ];
        let result = fields_to_json(&fields);
        assert_eq!(result["labels"], json!(["bug", "urgent"]));
        assert_eq!(result["assignees"], json!(["alice", "bob"]));
    }

    #[test]
    fn test_client_error_display() {
        let e = ClientError::Api("404 Not Found".into());
        assert!(e.to_string().contains("404 Not Found"));

        let e = ClientError::Cli("gh not found".into());
        assert!(e.to_string().contains("gh not found"));
    }

    // ---- Mock client tests ----

    #[tokio::test]
    async fn test_mock_client_api() {
        let client = GithubClient::mock(vec![json!({"id": 1, "title": "Test"})]);
        let result = client.api("/repos/o/r/issues/1", &[]).await.unwrap();
        assert_eq!(result["id"], 1);
        assert_eq!(result["title"], "Test");
    }

    #[tokio::test]
    async fn test_mock_client_api_multiple_responses() {
        let client = GithubClient::mock(vec![
            json!({"first": true}),
            json!({"second": true}),
        ]);
        let r1 = client.api("/a", &[]).await.unwrap();
        let r2 = client.api("/b", &[]).await.unwrap();
        assert_eq!(r1["first"], true);
        assert_eq!(r2["second"], true);
    }

    #[tokio::test]
    async fn test_mock_client_api_list() {
        let client = GithubClient::mock(vec![json!([{"id": 1}, {"id": 2}])]);
        let result = client.api_list("/repos/o/r/issues", &[], Some(10)).await.unwrap();
        assert_eq!(result.as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn test_mock_client_api_raw() {
        let client = GithubClient::mock(vec![json!("diff --git a/file")]);
        let result = client.api_raw("/repos/o/r/pulls/1", "application/vnd.github.v3.diff").await.unwrap();
        assert!(result.contains("diff"));
    }

    #[tokio::test]
    async fn test_mock_client_api_with_args() {
        let client = GithubClient::mock(vec![json!({"created": true})]);
        let result = client.api("/repos/o/r/issues", &["-X", "POST", "-f", "title=Bug"]).await.unwrap();
        assert_eq!(result["created"], true);
    }

    #[tokio::test]
    async fn test_mock_client_exhausted() {
        let client = GithubClient::mock(vec![]);
        let result = client.api("/anything", &[]).await.unwrap();
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn test_mock_client_error() {
        let client = GithubClient::mock_results(vec![
            Err(ClientError::Api("404 Not Found".into())),
        ]);
        let result = client.api("/repos/o/r/issues/999", &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("404"));
    }

    // ---- HTTP backend tests via wiremock ----

    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path};

    #[tokio::test]
    async fn test_http_get() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"number": 1, "title": "Bug"})))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r/issues/1", &[]).await.unwrap();
        assert_eq!(result["number"], 1);
    }

    #[tokio::test]
    async fn test_http_post() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/issues"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"number": 99})))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r/issues", &["-X", "POST", "-f", "title=Bug"]).await.unwrap();
        assert_eq!(result["number"], 99);
    }

    #[tokio::test]
    async fn test_http_patch() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/issues/1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"state": "closed"})))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r/issues/1", &["-X", "PATCH", "-f", "state=closed"]).await.unwrap();
        assert_eq!(result["state"], "closed");
    }

    #[tokio::test]
    async fn test_http_put() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/o/r/pulls/1/merge"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"merged": true})))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r/pulls/1/merge", &["-X", "PUT", "-f", "merge_method=squash"]).await.unwrap();
        assert_eq!(result["merged"], true);
    }

    #[tokio::test]
    async fn test_http_delete() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/repos/o/r/labels/bug"))
            .respond_with(ResponseTemplate::new(204).set_body_string(""))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r/labels/bug", &["-X", "DELETE"]).await.unwrap();
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn test_http_delete_with_body() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/repos/o/r/contents/file.txt"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"commit": {"sha": "abc"}})))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r/contents/file.txt",
            &["-X", "DELETE", "-f", "sha=abc123", "-f", "message=rm", "-f", "branch=main"]).await.unwrap();
        assert!(result["commit"]["sha"].is_string());
    }

    #[tokio::test]
    async fn test_http_error_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r/issues/999", &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("404"));
    }

    #[tokio::test]
    async fn test_http_empty_response() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/repos/o/r/pulls/1/merge"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r/pulls/1/merge", &["-X", "PUT"]).await.unwrap();
        assert_eq!(result, Value::Null);
    }

    #[tokio::test]
    async fn test_http_rate_limit_warning() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"full_name": "o/r"}))
                    .insert_header("x-ratelimit-remaining", "50")
            )
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r", &[]).await.unwrap();
        assert_eq!(result["full_name"], "o/r");
    }

    #[tokio::test]
    async fn test_http_rate_limit_plenty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"full_name": "o/r"}))
                    .insert_header("x-ratelimit-remaining", "4500")
            )
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r", &[]).await.unwrap();
        assert_eq!(result["full_name"], "o/r");
    }

    #[tokio::test]
    async fn test_http_api_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id": 1}, {"id": 2}])))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api_list("/repos/o/r/issues", &[("state", "open")], Some(10)).await.unwrap();
        assert!(result.is_array());
    }

    #[tokio::test]
    async fn test_http_api_raw() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/pulls/1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("diff --git a/file b/file\n+hello"))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api_raw("/repos/o/r/pulls/1", "application/vnd.github.v3.diff").await.unwrap();
        assert!(result.contains("diff"));
    }

    #[tokio::test]
    async fn test_http_api_raw_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/pulls/999"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api_raw("/repos/o/r/pulls/999", "application/vnd.github.v3.diff").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_http_api_raw_rate_limit() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/pulls/1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("diff content")
                    .insert_header("x-ratelimit-remaining", "10")
            )
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api_raw("/repos/o/r/pulls/1", "application/vnd.github.v3.diff").await.unwrap();
        assert!(result.contains("diff"));
    }

    // ---- Pagination tests ----

    #[test]
    fn test_parse_link_next() {
        let header = r#"<https://api.github.com/repos/o/r/issues?page=2&per_page=100>; rel="next", <https://api.github.com/repos/o/r/issues?page=5&per_page=100>; rel="last""#;
        let next = parse_link_next(header);
        assert_eq!(
            next,
            Some("https://api.github.com/repos/o/r/issues?page=2&per_page=100")
        );
    }

    #[test]
    fn test_parse_link_next_no_next() {
        let header = r#"<https://api.github.com/repos/o/r/issues?page=1>; rel="prev""#;
        assert_eq!(parse_link_next(header), None);
    }

    #[test]
    fn test_parse_link_next_empty() {
        assert_eq!(parse_link_next(""), None);
    }

    #[tokio::test]
    async fn test_mock_pagination_over_100() {
        // Simulate limit=150 with two pages of 100 and 50
        let page1: Vec<Value> = (1..=100).map(|i| json!({"id": i})).collect();
        let page2: Vec<Value> = (101..=150).map(|i| json!({"id": i})).collect();

        let client = GithubClient::mock(vec![
            Value::Array(page1),
            Value::Array(page2),
        ]);
        let result = client
            .api_list("/repos/o/r/issues", &[], Some(150))
            .await
            .unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 150);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[99]["id"], 100);
        assert_eq!(arr[100]["id"], 101);
        assert_eq!(arr[149]["id"], 150);
    }

    #[tokio::test]
    async fn test_mock_pagination_truncates_to_limit() {
        // 2 full pages of 100, but limit is 150
        let page1: Vec<Value> = (1..=100).map(|i| json!({"id": i})).collect();
        let page2: Vec<Value> = (101..=200).map(|i| json!({"id": i})).collect();

        let client = GithubClient::mock(vec![
            Value::Array(page1),
            Value::Array(page2),
        ]);
        let result = client
            .api_list("/repos/o/r/issues", &[], Some(150))
            .await
            .unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 150);
    }

    #[tokio::test]
    async fn test_mock_pagination_small_limit() {
        // limit <= 100 should NOT consume extra pages
        let client = GithubClient::mock(vec![
            json!([{"id": 1}, {"id": 2}]),
            json!([{"id": 3}]),  // should NOT be consumed
        ]);
        let result = client
            .api_list("/repos/o/r/issues", &[], Some(10))
            .await
            .unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }

    #[tokio::test]
    async fn test_http_pagination_with_link_header() {
        use wiremock::matchers::query_param;

        let server = MockServer::start().await;

        let page1: Vec<Value> = (1..=100).map(|i| json!({"id": i})).collect();
        let page2: Vec<Value> = (101..=150).map(|i| json!({"id": i})).collect();

        // Page 1: return Link header pointing to page 2
        let page2_url = format!(
            "{}/repos/o/r/issues?per_page=100&state=open&page=2",
            server.uri()
        );
        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues"))
            .and(query_param("page", "2"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(&page2),
            )
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/repos/o/r/issues"))
            .and(query_param("per_page", "100"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&page1)
                    .insert_header(
                        "link",
                        format!("<{page2_url}>; rel=\"next\""),
                    ),
            )
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client
            .api_list("/repos/o/r/issues", &[("state", "open")], Some(150))
            .await
            .unwrap();
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 150);
        assert_eq!(arr[0]["id"], 1);
        assert_eq!(arr[149]["id"], 150);
    }

    // ---- api_json tests ----

    #[tokio::test]
    async fn test_mock_api_json() {
        let client = GithubClient::mock(vec![json!({"sha": "abc123"})]);
        let body = json!({"content": "SGVsbG8=", "encoding": "base64"});
        let result = client
            .api_json("/repos/o/r/git/blobs", "POST", &body)
            .await
            .unwrap();
        assert_eq!(result["sha"], "abc123");
    }

    #[tokio::test]
    async fn test_http_api_json_post() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/repos/o/r/git/blobs"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(json!({"sha": "blob123"})),
            )
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let body = json!({"content": "SGVsbG8=", "encoding": "base64"});
        let result = client
            .api_json("/repos/o/r/git/blobs", "POST", &body)
            .await
            .unwrap();
        assert_eq!(result["sha"], "blob123");
    }

    #[tokio::test]
    async fn test_http_api_json_patch() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/repos/o/r/git/refs/heads/main"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ref": "refs/heads/main"})),
            )
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let body = json!({"sha": "newcommit123"});
        let result = client
            .api_json("/repos/o/r/git/refs/heads/main", "PATCH", &body)
            .await
            .unwrap();
        assert_eq!(result["ref"], "refs/heads/main");
    }

    // ---- Regression tests for audit findings ----

    #[test]
    fn test_fields_to_json_boolean_values() {
        // Finding #4: booleans must be JSON booleans, not strings
        let fields = vec![("draft", "true"), ("private", "false")];
        let result = fields_to_json(&fields);
        assert_eq!(result["draft"], Value::Bool(true));
        assert_eq!(result["private"], Value::Bool(false));
    }

    #[test]
    fn test_fields_to_json_numeric_values() {
        // Finding #4: numbers must be JSON numbers, not strings
        let fields = vec![("per_page", "100"), ("number", "42")];
        let result = fields_to_json(&fields);
        assert_eq!(result["per_page"], json!(100));
        assert_eq!(result["number"], json!(42));
    }

    #[test]
    fn test_fields_to_json_null_value() {
        let fields = vec![("field", "null")];
        let result = fields_to_json(&fields);
        assert!(result["field"].is_null());
    }

    #[test]
    fn test_fields_to_json_string_not_coerced() {
        // Strings that look like numbers but aren't should stay strings
        let fields = vec![("title", "123abc"), ("body", "not-a-number")];
        let result = fields_to_json(&fields);
        assert_eq!(result["title"], "123abc");
        assert_eq!(result["body"], "not-a-number");
    }

    #[test]
    fn test_fields_to_json_array_elements_stay_strings() {
        // Array elements should remain strings (labels, assignees are always strings)
        let fields = vec![("labels[]", "true"), ("labels[]", "42")];
        let result = fields_to_json(&fields);
        let arr = result["labels"].as_array().unwrap();
        assert!(arr[0].is_string());
        assert!(arr[1].is_string());
    }

    #[test]
    fn test_parse_field_value() {
        assert_eq!(parse_field_value("true"), Value::Bool(true));
        assert_eq!(parse_field_value("false"), Value::Bool(false));
        assert_eq!(parse_field_value("null"), Value::Null);
        assert_eq!(parse_field_value("42"), json!(42));
        assert_eq!(parse_field_value("-1"), json!(-1));
        assert_eq!(parse_field_value("hello"), json!("hello"));
        assert_eq!(parse_field_value("3.14"), json!("3.14")); // floats stay strings (GitHub API uses ints)
    }

    #[tokio::test]
    async fn test_http_timeout_on_slow_server() {
        // Finding #1: HTTP requests must timeout, not hang forever
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"name": "r"}))
                    .set_delay(Duration::from_millis(100))
            )
            .mount(&server)
            .await;

        // Fast response should succeed (within our 30s timeout)
        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r", &[]).await;
        assert!(result.is_ok());
    }

    #[tokio::test(start_paused = true)]
    async fn test_with_overall_timeout_aborts_long_call() {
        // Regression: prs_list-style hangs reported in the field. The overall
        // budget must abort a slow operation well before the legacy worst case
        // (retries × Retry-After cap = several minutes). With paused time,
        // tokio advances virtual time to fire timers without real wall sleep.
        let result: Result<i32, ClientError> = with_overall_timeout(
            "/repos/o/r/pulls",
            async {
                tokio::time::sleep(Duration::from_secs(600)).await;
                Ok(0)
            },
        ).await;
        match result {
            Err(ClientError::Timeout(msg)) => {
                assert!(msg.contains("/repos/o/r/pulls"), "msg should name endpoint: {msg}");
            }
            other => panic!("expected Timeout, got {other:?}"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn test_with_overall_timeout_passes_through_success() {
        // The wrapper must not interfere with fast paths.
        let result: Result<i32, ClientError> =
            with_overall_timeout("/x", async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_http_429_retries_then_fails() {
        // Finding #7: 429 should retry with backoff, then return error
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_string("rate limited")
                    .insert_header("retry-after", "1") // Short for test speed
            )
            .expect(1 + MAX_RETRIES as u64) // Initial + retries
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r", &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("429"));
    }

    #[tokio::test]
    async fn test_http_429_recovers_on_retry() {
        // Server returns 429 once, then succeeds
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r"))
            .respond_with(
                ResponseTemplate::new(429)
                    .set_body_string("rate limited")
                    .insert_header("retry-after", "1")
            )
            .up_to_n_times(1) // Only fail once
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"name": "r"}))
            )
            .mount(&server)
            .await;

        let client = GithubClient::http_with_base_url(&server.uri(), "test-token");
        let result = client.api("/repos/o/r", &[]).await.unwrap();
        assert_eq!(result["name"], "r");
    }

    #[tokio::test]
    async fn test_endpoint_validation_rejects_traversal() {
        let client = GithubClient::mock(vec![]);
        let result = client.api("/repos/../../admin/secrets", &[]).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("path traversal"));
    }

    #[tokio::test]
    async fn test_endpoint_validation_allows_compare() {
        // GitHub compare syntax uses "..." which must not be rejected
        let client = GithubClient::mock(vec![json!({"status": "ahead"})]);
        let result = client.api("/repos/o/r/compare/main...feat", &[]).await;
        assert!(result.is_ok());
    }
}
