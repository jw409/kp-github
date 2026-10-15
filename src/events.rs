//! Optional client for the proposed developer event-consumer API.
//!
//! The consumer endpoint is separate from GitHub's webhook ingress. Authentication
//! and ownership checks belong to the receiver; a local org allowlist is only a
//! convenience. Event JSON is returned intact, including delivery/ack tokens and
//! head provenance. Reading never acknowledges an event.

use base64::Engine;
use reqwest::{Method, StatusCode, Url};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_TOKEN_BYTES: u64 = 32 * 1024;
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const TOKEN_EXPIRY_MARGIN_SECONDS: u64 = 15;

/// Errors deliberately omit bearer tokens, subprocess output, and response
/// bodies (a failed remote service could echo any of those).
#[derive(Debug, Error)]
pub enum EventsError {
    #[error("Invalid developer events configuration: {0}")]
    Config(&'static str),
    #[error("Cannot read developer events configuration ({0})")]
    ConfigIo(std::io::ErrorKind),
    #[error("Invalid developer events request: {0}")]
    InvalidRequest(&'static str),
    #[error("Developer events authentication failed: {0}")]
    Auth(&'static str),
    #[error("Developer events request failed (HTTP {status}, Retry-After seconds: {retry_after_seconds:?}); no automatic retry")]
    Api {
        status: u16,
        retry_after_seconds: Option<u64>,
    },
    #[error("Developer events request timed out; mutations may have succeeded; reuse the same idempotency key")]
    Timeout,
    #[error("Developer events transport failed; mutations may have succeeded; reuse the same idempotency key")]
    Transport,
    #[error("Developer events response exceeds the 2 MiB limit")]
    ResponseTooLarge,
    #[error("Developer events response is not valid JSON")]
    InvalidResponse,
}

#[derive(Clone, Deserialize, Serialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct EventsConfig {
    pub version: u32,
    pub endpoint: String,
    /// Actual Google token audience accepted by the receiver. Human gcloud ID
    /// tokens retain Google's OAuth client audience; this is not automatically
    /// the endpoint URL. See docs/listener-api.md before configuring ingress.
    pub audience: String,
    pub allowed_orgs: Vec<String>,
    pub github_account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub google_account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_slug: Option<String>,
}

impl EventsConfig {
    /// An absent default profile disables the feature. An explicitly named or
    /// existing invalid profile is an error, never a reason to switch identities.
    pub fn from_environment() -> Result<Option<Self>, EventsError> {
        if let Some(path) = std::env::var_os("KP_GITHUB_EVENTS_CONFIG") {
            if path.is_empty() {
                return Err(EventsError::Config("KP_GITHUB_EVENTS_CONFIG is empty"));
            }
            return Self::from_path(&PathBuf::from(path)).map(Some);
        }
        let Some(home) = std::env::var_os("HOME") else {
            return Ok(None);
        };
        let path = PathBuf::from(home).join(".config/kp-github/events.json");
        match Self::from_path(&path) {
            Err(EventsError::ConfigIo(std::io::ErrorKind::NotFound)) => Ok(None),
            other => other.map(Some),
        }
    }

    pub fn from_path(path: &Path) -> Result<Self, EventsError> {
        let metadata =
            std::fs::symlink_metadata(path).map_err(|e| EventsError::ConfigIo(e.kind()))?;
        if !metadata.is_file() {
            return Err(EventsError::Config(
                "profile must be a regular file, not a symlink",
            ));
        }
        let file = std::fs::File::open(path).map_err(|e| EventsError::ConfigIo(e.kind()))?;
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| EventsError::ConfigIo(e.kind()))?;
        if bytes.len() as u64 > MAX_CONFIG_BYTES {
            return Err(EventsError::Config("profile exceeds 64 KiB"));
        }
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|_| EventsError::Config("expected a version 1 JSON profile"))?;
        config.validate(false)?;
        Ok(config)
    }

    fn validate(&self, allow_test_loopback: bool) -> Result<Url, EventsError> {
        if self.version != 1 {
            return Err(EventsError::Config("unsupported profile version"));
        }
        if self.endpoint.len() > 2048
            || self.endpoint.contains('\\')
            || self
                .endpoint
                .chars()
                .any(|c| c.is_whitespace() || c.is_control())
        {
            return Err(EventsError::Config(
                "endpoint contains invalid characters or exceeds 2048 bytes",
            ));
        }
        let mut endpoint = Url::parse(&self.endpoint)
            .map_err(|_| EventsError::Config("endpoint must be an absolute HTTPS URL"))?;
        let loopback = endpoint.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if endpoint.scheme() != "https"
            && !(allow_test_loopback && endpoint.scheme() == "http" && loopback)
        {
            return Err(EventsError::Config("endpoint must use HTTPS"));
        }
        if endpoint.host_str().is_none()
            || endpoint.port() == Some(0)
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.query().is_some()
            || endpoint.fragment().is_some()
        {
            return Err(EventsError::Config(
                "endpoint cannot contain credentials, a query, or a fragment",
            ));
        }
        if self.audience.is_empty()
            || self.audience.len() > 2048
            || self.audience.chars().any(char::is_whitespace)
            || self.audience.chars().any(char::is_control)
        {
            return Err(EventsError::Config(
                "audience must be a nonempty identifier",
            ));
        }
        if self.allowed_orgs.is_empty() || self.allowed_orgs.len() > 100 {
            return Err(EventsError::Config(
                "allowed_orgs must contain 1 to 100 organizations",
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for org in &self.allowed_orgs {
            if !valid_account(org) || !seen.insert(org.to_ascii_lowercase()) {
                return Err(EventsError::Config(
                    "allowed_orgs contains an invalid or duplicate organization",
                ));
            }
        }
        if !valid_account(&self.github_account) {
            return Err(EventsError::Config("github_account must be a GitHub login"));
        }
        if self.google_account.as_ref().is_some_and(|account| {
            account.len() > 254
                || !account.contains('@')
                || !account
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"@._-+".contains(&c))
        }) {
            return Err(EventsError::Config(
                "google_account must be an email address",
            ));
        }
        if self.app_slug.as_ref().is_some_and(|slug| {
            slug.is_empty()
                || slug.len() > 100
                || !slug.as_bytes()[0].is_ascii_alphanumeric()
                || !slug.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
        }) {
            return Err(EventsError::Config("app_slug must be a GitHub App slug"));
        }
        // Preserve an explicitly configured base path. Later URL components are
        // validated and appended, never joined from caller-supplied URLs.
        let path = endpoint.path().trim_end_matches('/').to_string();
        endpoint.set_path(&path);
        Ok(endpoint)
    }
}

#[derive(Clone, Copy, Deserialize, Serialize, Debug, PartialEq, Eq, Hash, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Approved,
    Merged,
    CiFailed,
}

#[derive(Clone, Deserialize, Serialize, Debug, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
// An internally tagged enum otherwise emits only oneOf at the schema root.
// MCP requires inputSchema to explicitly declare the object type as well.
#[schemars(extend("type" = "object"))]
pub enum EventsRequest {
    Register {
        owner: String,
        repo: Option<String>,
        events: Vec<EventKind>,
        /// Reuse this key if the result is uncertain or the request is retried.
        idempotency_key: String,
    },
    List,
    Remove {
        listener_id: String,
    },
    Read {
        listener_id: String,
        /// 1..100, default 30.
        limit: Option<u16>,
        /// 0..30 seconds, default 0. Reading never acknowledges deliveries.
        wait_seconds: Option<u8>,
    },
    Ack {
        listener_id: String,
        /// Opaque receiver-issued tokens for this listener and consumer only.
        ack_tokens: Vec<String>,
    },
}

struct CachedToken {
    value: String,
    expires_at: u64,
}

struct TokenMetadata {
    expires_at: u64,
    principal: (String, String),
    audiences: Vec<String>,
    email: Option<String>,
}

#[derive(Default)]
struct TokenState {
    cached: Option<CachedToken>,
    // A local consistency guard across refreshes, not authentication proof.
    // Kept after eviction so changing gcloud's active account cannot silently
    // move this client's existing listeners to another principal.
    principal: Option<(String, String)>,
}

enum TokenProvider {
    Gcloud,
    #[cfg(test)]
    Test {
        tokens: Mutex<std::collections::VecDeque<String>>,
        calls: std::sync::atomic::AtomicUsize,
    },
}

pub struct EventsClient {
    config: EventsConfig,
    endpoint: Url,
    http: reqwest::Client,
    provider: TokenProvider,
    token: Mutex<TokenState>,
}

impl EventsClient {
    pub fn from_environment() -> Result<Option<Self>, EventsError> {
        EventsConfig::from_environment()?
            .map(Self::from_config)
            .transpose()
    }

    /// Construction performs no identity or network calls.
    pub fn from_config(config: EventsConfig) -> Result<Self, EventsError> {
        Self::build(config, TokenProvider::Gcloud, false)
    }

    fn build(
        config: EventsConfig,
        provider: TokenProvider,
        test_loopback: bool,
    ) -> Result<Self, EventsError> {
        let endpoint = config.validate(test_loopback)?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(45))
            .user_agent("kp-github-mcp-events/1")
            .build()
            .map_err(|_| EventsError::Transport)?;
        Ok(Self {
            config,
            endpoint,
            http,
            provider,
            token: Mutex::new(TokenState::default()),
        })
    }

    pub fn github_account(&self) -> &str {
        &self.config.github_account
    }

    /// Fake-service constructor, unavailable in production builds.
    #[cfg(test)]
    pub(crate) fn with_test_token(endpoint: &str, token: &str) -> Self {
        let config = EventsConfig {
            version: 1,
            endpoint: endpoint.into(),
            audience: "test-audience".into(),
            allowed_orgs: vec!["Meshlyai".into(), "Meshly-Open-Source".into()],
            github_account: "jw409".into(),
            google_account: None,
            app_slug: None,
        };
        let mut client = Self::build(
            config,
            TokenProvider::Test {
                tokens: Mutex::new(std::collections::VecDeque::new()),
                calls: std::sync::atomic::AtomicUsize::new(0),
            },
            true,
        )
        .unwrap();
        client.token = Mutex::new(TokenState {
            cached: Some(CachedToken {
                value: token.into(),
                expires_at: now_seconds() + 3600,
            }),
            principal: None,
        });
        client
    }

    pub async fn execute(&self, request: EventsRequest) -> Result<Value, EventsError> {
        let (method, path, body, query) = self.prepare(request)?;
        let mut url = self.endpoint.clone();
        url.set_path(&format!(
            "{}{}",
            self.endpoint.path().trim_end_matches('/'),
            path
        ));
        if let Some((limit, wait)) = query {
            url.query_pairs_mut()
                .append_pair("limit", &limit.to_string())
                .append_pair("wait_seconds", &wait.to_string());
        }

        let mut token = self.token().await?;
        for attempt in 0..=1 {
            let mut req = self
                .http
                .request(method.clone(), url.clone())
                .bearer_auth(&token)
                .header(reqwest::header::ACCEPT, "application/json");
            if let Some(body) = &body {
                req = req.json(body);
            }
            let response = req.send().await.map_err(transport_error)?;
            if response.status() == StatusCode::UNAUTHORIZED {
                self.invalidate_if_current(&token).await;
                if attempt == 0 {
                    // Bodies (including registration's idempotency key) stay
                    // identical across the sole permitted authentication retry.
                    token = self.token().await?;
                    continue;
                }
            }
            if !response.status().is_success() {
                return Err(EventsError::Api {
                    status: response.status().as_u16(),
                    retry_after_seconds: response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.parse().ok()),
                });
            }
            return read_json(response).await;
        }
        unreachable!("authentication loop always returns on its second response")
    }

    #[allow(clippy::type_complexity)]
    fn prepare(
        &self,
        request: EventsRequest,
    ) -> Result<(Method, String, Option<Value>, Option<(u16, u8)>), EventsError> {
        match request {
            EventsRequest::Register {
                owner,
                repo,
                events,
                idempotency_key,
            } => {
                if !valid_account(&owner) {
                    return Err(EventsError::InvalidRequest(
                        "owner must be a GitHub organization login",
                    ));
                }
                if !self
                    .config
                    .allowed_orgs
                    .iter()
                    .any(|org| org.eq_ignore_ascii_case(&owner))
                {
                    return Err(EventsError::InvalidRequest(
                        "owner is outside this profile's allowed_orgs",
                    ));
                }
                if repo.as_ref().is_some_and(|r| {
                    r.is_empty()
                        || r.len() > 100
                        || r == "."
                        || r == ".."
                        || !r
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
                }) {
                    return Err(EventsError::InvalidRequest(
                        "repo must be a repository name without an owner or path",
                    ));
                }
                if events.is_empty()
                    || events.len() > 3
                    || events
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        != events.len()
                {
                    return Err(EventsError::InvalidRequest(
                        "events must contain 1 to 3 distinct event kinds",
                    ));
                }
                if idempotency_key.is_empty()
                    || idempotency_key.len() > 128
                    || !idempotency_key
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
                {
                    return Err(EventsError::InvalidRequest(
                        "idempotency_key must contain 1 to 128 ASCII identifier characters",
                    ));
                }
                let mut body =
                    json!({"owner":owner,"events":events,"idempotency_key":idempotency_key});
                if let Some(repo) = repo {
                    body["repo"] = Value::String(repo);
                }
                Ok((Method::POST, "/v1/listeners".into(), Some(body), None))
            }
            EventsRequest::List => Ok((Method::GET, "/v1/listeners".into(), None, None)),
            EventsRequest::Remove { listener_id } => {
                validate_listener_id(&listener_id)?;
                Ok((
                    Method::DELETE,
                    format!("/v1/listeners/{listener_id}"),
                    None,
                    None,
                ))
            }
            EventsRequest::Read {
                listener_id,
                limit,
                wait_seconds,
            } => {
                validate_listener_id(&listener_id)?;
                let limit = limit.unwrap_or(30);
                let wait = wait_seconds.unwrap_or(0);
                if !(1..=100).contains(&limit) || wait > 30 {
                    return Err(EventsError::InvalidRequest(
                        "limit must be 1..100 and wait_seconds must be 0..30",
                    ));
                }
                Ok((
                    Method::GET,
                    format!("/v1/listeners/{listener_id}/events"),
                    None,
                    Some((limit, wait)),
                ))
            }
            EventsRequest::Ack {
                listener_id,
                ack_tokens,
            } => {
                validate_listener_id(&listener_id)?;
                if ack_tokens.is_empty()
                    || ack_tokens.len() > 100
                    || ack_tokens
                        .iter()
                        .any(|v| v.is_empty() || v.len() > 4096 || v.chars().any(char::is_control))
                    || ack_tokens.iter().map(String::len).sum::<usize>() > 128 * 1024
                {
                    return Err(EventsError::InvalidRequest("ack_tokens must contain 1..100 opaque tokens, each at most 4096 bytes and at most 128 KiB in total"));
                }
                Ok((
                    Method::POST,
                    format!("/v1/listeners/{listener_id}/ack"),
                    Some(json!({"ack_tokens":ack_tokens})),
                    None,
                ))
            }
        }
    }

    async fn token(&self) -> Result<String, EventsError> {
        // Serialize only credential acquisition. HTTP requests remain concurrent.
        let mut cache = self.token.lock().await;
        if let Some(token) = cache.cached.as_ref() {
            if token.expires_at > now_seconds() + TOKEN_EXPIRY_MARGIN_SECONDS {
                return Ok(token.value.clone());
            }
        }
        let value = match &self.provider {
            TokenProvider::Gcloud => gcloud_token(self.config.google_account.as_deref()).await?,
            #[cfg(test)]
            TokenProvider::Test { tokens, calls } => {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokens
                    .lock()
                    .await
                    .pop_front()
                    .ok_or(EventsError::Auth("test provider exhausted"))?
            }
        };
        let metadata = token_metadata(&value)?;
        let expires_at = metadata.expires_at;
        let principal = metadata.principal;
        if expires_at <= now_seconds() + TOKEN_EXPIRY_MARGIN_SECONDS {
            return Err(EventsError::Auth(
                "gcloud returned an expired or nearly expired identity token",
            ));
        }
        if !metadata
            .audiences
            .iter()
            .any(|aud| aud == &self.config.audience)
        {
            return Err(EventsError::Auth("identity token audience does not match the profile; the receiver operator must supply an accepted gcloud-compatible audience"));
        }
        if self.config.google_account.as_ref().is_some_and(|expected| {
            !metadata
                .email
                .as_ref()
                .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
        }) {
            return Err(EventsError::Auth("gcloud identity token email does not match google_account; check that account's stored login"));
        }
        if cache
            .principal
            .as_ref()
            .is_some_and(|previous| previous != &principal)
        {
            return Err(EventsError::Auth("gcloud identity changed during this session; pin google_account in the profile and restart"));
        }
        cache.principal = Some(principal);
        cache.cached = Some(CachedToken {
            value: value.clone(),
            expires_at,
        });
        Ok(value)
    }

    async fn invalidate_if_current(&self, rejected: &str) {
        let mut cache = self.token.lock().await;
        if cache
            .cached
            .as_ref()
            .is_some_and(|token| token.value == rejected)
        {
            cache.cached = None;
        }
    }
}

fn valid_account(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 39
        && !s.contains("--")
        && s.as_bytes()[0].is_ascii_alphanumeric()
        && s.as_bytes()[s.len() - 1].is_ascii_alphanumeric()
        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

fn validate_listener_id(s: &str) -> Result<(), EventsError> {
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
    {
        return Err(EventsError::InvalidRequest(
            "listener_id must contain 1 to 128 letters, digits, underscores, or hyphens",
        ));
    }
    Ok(())
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Unsigned claims schedule refresh and catch local identity/config drift. They
/// are not proof of identity or permission: the receiver MUST verify signatures,
/// issuer, expiry, audience, and authorization independently.
fn token_metadata(token: &str) -> Result<TokenMetadata, EventsError> {
    if token.len() as u64 > MAX_TOKEN_BYTES || token.bytes().any(|c| c.is_ascii_whitespace()) {
        return Err(EventsError::Auth(
            "gcloud returned an invalid identity token",
        ));
    }
    let parts: Vec<_> = token.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty()) {
        return Err(EventsError::Auth(
            "gcloud did not return a JWT identity token",
        ));
    }
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| EventsError::Auth("identity token payload is invalid"))?;
    let claims: Value = serde_json::from_slice(&claims)
        .map_err(|_| EventsError::Auth("identity token payload is invalid"))?;
    let exp = claims
        .get("exp")
        .and_then(Value::as_u64)
        .ok_or(EventsError::Auth("identity token has no valid expiry"))?;
    let issuer = claims
        .get("iss")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(EventsError::Auth("identity token has no issuer"))?;
    let subject = claims
        .get("sub")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or(EventsError::Auth("identity token has no subject"))?;
    let audiences = match claims.get("aud") {
        Some(Value::String(aud)) if !aud.is_empty() => vec![aud.clone()],
        Some(Value::Array(values)) if !values.is_empty() => values
            .iter()
            .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or(EventsError::Auth("identity token has no valid audience"))?,
        _ => return Err(EventsError::Auth("identity token has no valid audience")),
    };
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(TokenMetadata {
        expires_at: exp,
        principal: (issuer.into(), subject.into()),
        audiences,
        email,
    })
}

fn gcloud_command(account: Option<&str>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new("gcloud");
    command.args(["auth", "print-identity-token", "--quiet"]);
    if let Some(account) = account {
        command.args(["--account", account]);
    }
    // No --audiences for human accounts: the CLI cannot mint arbitrary
    // audiences for those. Never change the active account or gcloud config.
    // Empty property values intentionally override BOTH inherited environment
    // variables and persisted gcloud properties. Removing only the environment
    // overrides would still allow credential_file_override in the active config
    // to bypass --account. These changes affect this child process only.
    command
        .env("CLOUDSDK_CORE_LOG_HTTP", "false")
        .env("CLOUDSDK_AUTH_ACCESS_TOKEN", "")
        .env("CLOUDSDK_AUTH_ACCESS_TOKEN_FILE", "")
        .env("CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE", "")
        .env("CLOUDSDK_AUTH_IMPERSONATE_SERVICE_ACCOUNT", "")
        .env("CLOUDSDK_AUTH_AUTHORIZATION_TOKEN_FILE", "")
        .env_remove("GOOGLE_APPLICATION_CREDENTIALS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

async fn gcloud_token(account: Option<&str>) -> Result<String, EventsError> {
    let mut child = gcloud_command(account).spawn().map_err(|_| {
        EventsError::Auth("cannot start gcloud; sign in using gcloud auth login first")
    })?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or(EventsError::Auth("cannot read gcloud token output"))?;
    let operation = async {
        let mut bytes = Vec::new();
        (&mut stdout)
            .take(MAX_TOKEN_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| EventsError::Auth("cannot read gcloud token output"))?;
        if bytes.len() as u64 > MAX_TOKEN_BYTES {
            let _ = child.kill().await;
            return Err(EventsError::Auth("gcloud token output is too large"));
        }
        let status = child
            .wait()
            .await
            .map_err(|_| EventsError::Auth("cannot wait for gcloud"))?;
        if !status.success() {
            return Err(EventsError::Auth(
                "gcloud could not produce an identity token; check the configured account's login",
            ));
        }
        String::from_utf8(bytes)
            .map(|s| s.trim().to_string())
            .map_err(|_| EventsError::Auth("gcloud returned invalid token bytes"))
    };
    tokio::time::timeout(Duration::from_secs(15), operation)
        .await
        .map_err(|_| EventsError::Auth("gcloud identity token acquisition timed out"))?
}

fn transport_error(error: reqwest::Error) -> EventsError {
    if error.is_timeout() {
        EventsError::Timeout
    } else {
        EventsError::Transport
    }
}

async fn read_json(mut response: reqwest::Response) -> Result<Value, EventsError> {
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(EventsError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(EventsError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| EventsError::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use wiremock::matchers::{body_json, header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn config(endpoint: &str) -> EventsConfig {
        EventsConfig {
            version: 1,
            endpoint: endpoint.into(),
            audience: "gcloud-client-id".into(),
            allowed_orgs: vec!["Meshlyai".into(), "Meshly-Open-Source".into()],
            github_account: "jw409".into(),
            google_account: Some("developer@example.com".into()),
            app_slug: Some("meshly-open-source-dev-events".into()),
        }
    }

    fn jwt(subject: &str, suffix: &str, expiry: u64) -> String {
        let claims = json!({"iss":"https://accounts.google.com","sub":subject,"exp":expiry,"aud":"gcloud-client-id","email":"developer@example.com"});
        format!(
            "e30.{}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string()),
            suffix
        )
    }

    fn fresh_token(suffix: &str) -> String {
        jwt("human-123", suffix, now_seconds() + 3600)
    }

    fn client(endpoint: &str, tokens: Vec<String>) -> EventsClient {
        EventsClient::build(
            config(endpoint),
            TokenProvider::Test {
                tokens: Mutex::new(tokens.into()),
                calls: std::sync::atomic::AtomicUsize::new(0),
            },
            true,
        )
        .unwrap()
    }

    fn token_calls(client: &EventsClient) -> usize {
        match &client.provider {
            TokenProvider::Test { calls, .. } => calls.load(Ordering::SeqCst),
            _ => unreachable!(),
        }
    }

    fn register() -> EventsRequest {
        EventsRequest::Register {
            owner: "Meshlyai".into(),
            repo: Some("example".into()),
            events: vec![EventKind::Approved, EventKind::Merged, EventKind::CiFailed],
            idempotency_key: "request-0001".into(),
        }
    }

    #[tokio::test]
    async fn registration_reuses_identical_body_on_only_auth_retry_and_caches_success() {
        let server = MockServer::start().await;
        let old = fresh_token("old");
        let new = fresh_token("new");
        let body = json!({"owner":"Meshlyai","repo":"example","events":["approved","merged","ci_failed"],"idempotency_key":"request-0001"});
        Mock::given(method("POST"))
            .and(path("/v1/listeners"))
            .and(header("authorization", format!("Bearer {old}")))
            .and(body_json(&body))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/listeners"))
            .and(header("authorization", format!("Bearer {new}")))
            .and(body_json(&body))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id":"l-1"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/listeners"))
            .and(header("authorization", format!("Bearer {new}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"listeners":[]})))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri(), vec![old, new]);
        assert_eq!(
            client.execute(register()).await.unwrap(),
            json!({"id":"l-1"})
        );
        client.execute(EventsRequest::List).await.unwrap();
        assert_eq!(token_calls(&client), 2);
    }

    #[tokio::test]
    async fn second_401_stops_and_discards_only_rejected_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/listeners"))
            .respond_with(ResponseTemplate::new(401))
            .expect(2)
            .mount(&server)
            .await;
        let client = client(&server.uri(), vec![fresh_token("one"), fresh_token("two")]);
        assert!(matches!(
            client.execute(register()).await,
            Err(EventsError::Api { status: 401, .. })
        ));
        assert_eq!(token_calls(&client), 2);
        assert!(client.token.lock().await.cached.is_none());
    }

    #[tokio::test]
    async fn forbidden_rate_limit_and_server_failure_keep_cache_and_do_not_retry_mutation() {
        for status in [403, 429, 500, 503] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/listeners"))
                .respond_with(ResponseTemplate::new(status).insert_header("Retry-After", "13"))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/v1/listeners"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"listeners":[]})))
                .expect(1)
                .mount(&server)
                .await;
            let client = client(&server.uri(), vec![fresh_token("one")]);
            assert!(matches!(client.execute(register()).await,
                Err(EventsError::Api { status: actual, retry_after_seconds: Some(13) }) if actual == status));
            client.execute(EventsRequest::List).await.unwrap();
            assert_eq!(token_calls(&client), 1);
        }
    }

    #[tokio::test]
    async fn read_preserves_raw_stale_head_metadata_and_does_not_acknowledge() {
        let server = MockServer::start().await;
        let raw = json!({"events":[{"delivery_id":"delivery-1","ack_token":"opaque-token-with-a-long-tail-1234567890",
            "kind":"approved","head_sha":"1234567890abcdef1234567890abcdef12345678",
            "current_head_sha":"abcdef1234567890abcdef1234567890abcdef12","stale_head":true,
            "payload":{"review":{"state":"approved","body":"verbatim review"}},"unknown_extension":{"a":null}}],
            "more_available":true,"cursor":"opaque-cursor"});
        Mock::given(method("GET"))
            .and(path("/v1/listeners/listener_123/events"))
            .and(query_param("limit", "17"))
            .and(query_param("wait_seconds", "30"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&raw))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri(), vec![fresh_token("one")]);
        let value = client
            .execute(EventsRequest::Read {
                listener_id: "listener_123".into(),
                limit: Some(17),
                wait_seconds: Some(30),
            })
            .await
            .unwrap();
        assert_eq!(value, raw);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn ack_sends_only_explicit_tokens_to_selected_listener_and_preserves_rejection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/listeners/listener-B/ack"))
            .and(body_json(json!({"ack_tokens":["token-owned-by-A"]})))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri(), vec![fresh_token("one")]);
        assert!(matches!(
            client
                .execute(EventsRequest::Ack {
                    listener_id: "listener-B".into(),
                    ack_tokens: vec!["token-owned-by-A".into()]
                })
                .await,
            Err(EventsError::Api { status: 403, .. })
        ));
        // Only the receiver can authenticate token ownership; the client neither
        // claims authorization nor retries against a different listener.
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(token_calls(&client), 1);
    }

    #[tokio::test]
    async fn redirects_are_never_followed_or_given_credentials() {
        let server = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/listeners"))
            .respond_with(
                ResponseTemplate::new(307)
                    .insert_header("Location", format!("{}/stolen", other.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri(), vec![fresh_token("one")]);
        assert!(matches!(
            client.execute(EventsRequest::List).await,
            Err(EventsError::Api { status: 307, .. })
        ));
        assert!(other.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unsafe_or_out_of_scope_requests_fail_before_token_acquisition() {
        let client = client("http://127.0.0.1:12345", vec![]);
        let invalid = vec![
            EventsRequest::Remove {
                listener_id: "../another".into(),
            },
            EventsRequest::Remove {
                listener_id: "https://elsewhere".into(),
            },
            EventsRequest::Read {
                listener_id: "valid".into(),
                limit: Some(0),
                wait_seconds: None,
            },
            EventsRequest::Read {
                listener_id: "valid".into(),
                limit: Some(101),
                wait_seconds: None,
            },
            EventsRequest::Read {
                listener_id: "valid".into(),
                limit: None,
                wait_seconds: Some(31),
            },
            EventsRequest::Ack {
                listener_id: "valid".into(),
                ack_tokens: vec![],
            },
            EventsRequest::Ack {
                listener_id: "valid".into(),
                ack_tokens: vec!["bad\ntoken".into()],
            },
            EventsRequest::Register {
                owner: "unconfigured".into(),
                repo: None,
                events: vec![EventKind::Approved],
                idempotency_key: "good".into(),
            },
            EventsRequest::Register {
                owner: "Meshlyai".into(),
                repo: Some("other/repo".into()),
                events: vec![EventKind::Approved],
                idempotency_key: "good".into(),
            },
            EventsRequest::Register {
                owner: "Meshlyai".into(),
                repo: None,
                events: vec![EventKind::Approved, EventKind::Approved],
                idempotency_key: "good".into(),
            },
            EventsRequest::Register {
                owner: "Meshlyai".into(),
                repo: None,
                events: vec![EventKind::Approved],
                idempotency_key: "".into(),
            },
        ];
        for request in invalid {
            assert!(matches!(
                client.execute(request).await,
                Err(EventsError::InvalidRequest(_))
            ));
        }
        assert_eq!(token_calls(&client), 0);
    }

    #[test]
    fn profile_validation_rejects_unsafe_endpoints_and_ambiguous_identity() {
        for endpoint in [
            "http://example.com",
            "http://localhost",
            "https://user:secret@example.com",
            "https://example.com/?query=1",
            "https://example.com/#fragment",
            "https://example.com:0",
            " https://example.com",
            "https://example.com/\npath",
            "https://example.com\\elsewhere",
        ] {
            assert!(
                EventsClient::from_config(config(endpoint)).is_err(),
                "accepted {endpoint:?}"
            );
        }
        let mut profile = config("https://events.example.com/api");
        assert!(profile.validate(false).is_ok());
        profile.allowed_orgs.push("meshlyai".into());
        assert!(profile.validate(false).is_err());
        profile.allowed_orgs = vec!["bad--org".into()];
        assert!(profile.validate(false).is_err());
        profile.allowed_orgs = vec!["Meshlyai".into()];
        profile.github_account = "".into();
        assert!(profile.validate(false).is_err());
        assert!(serde_json::from_value::<EventsConfig>(json!({"version":1,"endpoint":"https://example.com",
            "audience":"aud","allowed_orgs":["Meshlyai"],"github_account":"jw409","bearer_token":"not-supported"})).is_err());
    }

    #[tokio::test]
    async fn expired_tokens_refresh_lazily_without_revalidating_valid_cache() {
        let client = client(
            "http://127.0.0.1:12345",
            vec![fresh_token("one"), fresh_token("two")],
        );
        let first = client.token().await.unwrap();
        assert_eq!(client.token().await.unwrap(), first);
        assert_eq!(token_calls(&client), 1);
        client
            .token
            .lock()
            .await
            .cached
            .as_mut()
            .unwrap()
            .expires_at = now_seconds() - 1;
        assert_ne!(client.token().await.unwrap(), first);
        assert_eq!(token_calls(&client), 2);
    }

    #[tokio::test]
    async fn stale_401_does_not_evict_newer_token() {
        let client = client(
            "http://127.0.0.1:12345",
            vec![fresh_token("old"), fresh_token("new")],
        );
        let old = client.token().await.unwrap();
        client.invalidate_if_current(&old).await;
        let new = client.token().await.unwrap();
        client.invalidate_if_current(&old).await;
        assert_eq!(client.token().await.unwrap(), new);
        assert_eq!(token_calls(&client), 2);
    }

    #[tokio::test]
    async fn account_drift_after_401_is_rejected_before_second_http_request() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/listeners"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(
            &server.uri(),
            vec![
                fresh_token("first"),
                jwt("different-human", "second", now_seconds() + 3600),
            ],
        );
        assert!(
            matches!(client.execute(EventsRequest::List).await, Err(EventsError::Auth(message)) if message.contains("identity changed"))
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn token_acquisition_is_serialized_for_concurrent_requests() {
        let client = client("http://127.0.0.1:12345", vec![fresh_token("one")]);
        let (one, two, three) = tokio::join!(client.token(), client.token(), client.token());
        assert_eq!(one.unwrap(), two.unwrap());
        assert!(three.is_ok());
        assert_eq!(token_calls(&client), 1);
    }

    #[test]
    fn gcloud_uses_per_process_account_and_no_custom_human_audience() {
        let command = gcloud_command(Some("developer@example.com"));
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|a| a.to_str().unwrap())
            .collect();
        assert_eq!(
            args,
            vec![
                "auth",
                "print-identity-token",
                "--quiet",
                "--account",
                "developer@example.com"
            ]
        );
        assert!(!args
            .iter()
            .any(|s| s.contains("audience") || *s == "set" || *s == "login"));
        let env: std::collections::HashMap<_, _> = command.as_std().get_envs().collect();
        for key in [
            "CLOUDSDK_AUTH_ACCESS_TOKEN",
            "CLOUDSDK_AUTH_ACCESS_TOKEN_FILE",
            "CLOUDSDK_AUTH_CREDENTIAL_FILE_OVERRIDE",
            "CLOUDSDK_AUTH_IMPERSONATE_SERVICE_ACCOUNT",
            "CLOUDSDK_AUTH_AUTHORIZATION_TOKEN_FILE",
        ] {
            assert_eq!(
                env.get(std::ffi::OsStr::new(key)),
                Some(&Some(std::ffi::OsStr::new("")))
            );
        }
        assert_eq!(
            env.get(std::ffi::OsStr::new("GOOGLE_APPLICATION_CREDENTIALS")),
            Some(&None)
        );
    }

    #[tokio::test]
    async fn audience_and_explicit_google_account_mismatch_fail_before_http() {
        let server = MockServer::start().await;
        let mut wrong_audience = client(&server.uri(), vec![fresh_token("one")]);
        wrong_audience.config.audience = "different-client-id".into();
        assert!(matches!(wrong_audience.execute(EventsRequest::List).await,
            Err(EventsError::Auth(message)) if message.contains("audience")));
        let mut wrong_account = client(&server.uri(), vec![fresh_token("two")]);
        wrong_account.config.google_account = Some("another@example.com".into());
        assert!(matches!(wrong_account.execute(EventsRequest::List).await,
            Err(EventsError::Auth(message)) if message.contains("email")));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn audience_array_is_preserved_for_local_configuration_check() {
        let claims = json!({"iss":"https://accounts.google.com","sub":"user","exp":123,
            "aud":["one-client-id","two-client-id"]});
        let token = format!(
            "e30.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims.to_string())
        );
        assert_eq!(
            token_metadata(&token).unwrap().audiences,
            vec!["one-client-id", "two-client-id"]
        );
    }

    #[tokio::test]
    async fn response_body_is_bounded_and_errors_do_not_echo_remote_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/listeners"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("x".repeat(MAX_RESPONSE_BYTES + 1)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server.uri(), vec![fresh_token("one")]);
        assert!(matches!(
            client.execute(EventsRequest::List).await,
            Err(EventsError::ResponseTooLarge)
        ));
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/v1/listeners"))
            .respond_with(
                ResponseTemplate::new(500).set_body_string("this-body-could-contain-credentials"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let message = client
            .execute(EventsRequest::List)
            .await
            .unwrap_err()
            .to_string();
        assert!(!message.contains("credentials"));
        assert!(message.contains("500"));
    }

    #[tokio::test]
    async fn remove_handles_empty_response_and_preserves_endpoint_base_path() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/consumer/v1/listeners/listener-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(
            &format!("{}/consumer/", server.uri()),
            vec![fresh_token("one")],
        );
        assert_eq!(
            client
                .execute(EventsRequest::Remove {
                    listener_id: "listener-1".into()
                })
                .await
                .unwrap(),
            Value::Null
        );
    }

    #[test]
    fn untrusted_jwt_claims_are_used_only_for_expiry_and_consistency() {
        assert!(token_metadata("not-a-jwt").is_err());
        assert!(token_metadata(&format!("{}.tail", fresh_token("one"))).is_err());
        let token = jwt("human-123", "not-a-signature", 123);
        let metadata = token_metadata(&token).unwrap();
        assert_eq!(metadata.expires_at, 123);
        assert_eq!(
            metadata.principal,
            ("https://accounts.google.com".into(), "human-123".into())
        );
        assert_eq!(metadata.audiences, vec!["gcloud-client-id"]);
    }
}
