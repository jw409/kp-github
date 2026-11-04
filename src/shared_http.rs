//! One local HTTP process; one MCP session per client; one pinned account/profile.
use std::{
    collections::HashMap,
    io::Read,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{bail, Context};
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{header, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Router,
};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;

use crate::server::KpGithubServer;

pub const HELP: &str = "kp-github-mcp [--http LOOPBACK_IP:PORT --http-token-file PATH]\n\nNo arguments: stdio. HTTP: one shared account/profile, separate MCP sessions.\nToken file: current-user-owned regular file, mode 0600, 64 random hex characters.\nHTTP binds only to loopback. GitHub and profile selectors are read at startup.";
const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_SESSIONS: usize = 256;
const IDLE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

pub enum Mode {
    Stdio,
    Help,
    Http {
        address: SocketAddr,
        token_file: PathBuf,
    },
}

impl Mode {
    pub fn parse(args: impl Iterator<Item = String>) -> anyhow::Result<Self> {
        let mut args = args.peekable();
        if args.peek().is_none() {
            return Ok(Self::Stdio);
        }
        let mut address = None;
        let mut token_file = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h"
                    if address.is_none() && token_file.is_none() && args.peek().is_none() =>
                {
                    return Ok(Self::Help)
                }
                "--http" if address.is_none() => {
                    address = Some(
                        args.next()
                            .context("--http requires an address")?
                            .parse::<SocketAddr>()
                            .context("--http requires a literal loopback IP:port")?,
                    )
                }
                "--http-token-file" if token_file.is_none() => {
                    token_file = Some(PathBuf::from(
                        args.next().context("--http-token-file requires a path")?,
                    ))
                }
                _ => bail!("Unknown, repeated, or misplaced option; use --help"),
            }
        }
        let address = address.context("--http is required with --http-token-file")?;
        if !address.ip().is_loopback() {
            bail!("Shared HTTP must bind to a loopback address");
        }
        Ok(Self::Http {
            address,
            token_file: token_file.context("--http requires --http-token-file")?,
        })
    }

    pub fn prepare(self) -> anyhow::Result<Option<HttpConfig>> {
        match self {
            Self::Stdio | Self::Help => Ok(None),
            Self::Http {
                address,
                token_file,
            } => Ok(Some(HttpConfig {
                address,
                authorization: read_key(&token_file)?,
            })),
        }
    }
}

pub struct HttpConfig {
    address: SocketAddr,
    authorization: String,
}

#[cfg(unix)]
fn read_key(path: &std::path::Path) -> anyhow::Result<String> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    // Reject links in every ancestor, including a replaced runtime directory.
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut directory = PathBuf::new();
    for component in absolute.components() {
        directory.push(component);
        if std::fs::symlink_metadata(&directory)
            .context("Cannot inspect local HTTP key path")?
            .file_type()
            .is_symlink()
        {
            bail!("Local HTTP key path must not traverse symlinks");
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .context("Cannot open local HTTP key file")?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.mode() & 0o777 != 0o600 || meta.uid() != unsafe { libc::geteuid() } {
        bail!("Local HTTP key must be a current-user-owned regular file with mode 0600");
    }
    let mut text = String::new();
    file.by_ref()
        .take(66)
        .read_to_string(&mut text)
        .context("Cannot read local HTTP key")?;
    let key = text.strip_suffix('\n').unwrap_or(&text);
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!(
            "Local HTTP key must contain exactly 64 random hex characters and an optional newline"
        );
    }
    // Do not log the key or place it in the process arguments.
    Ok(format!("Bearer {key}"))
}

#[cfg(not(unix))]
fn read_key(_: &std::path::Path) -> anyhow::Result<String> {
    bail!("Shared HTTP currently supports Unix user permissions only")
}

struct Boundary {
    authorization: String,
    authority: String,
    sessions: Arc<LocalSessionManager>,
    activity: Mutex<HashMap<String, Instant>>,
    initialize: tokio::sync::Mutex<()>,
}

async fn guard(State(state): State<Arc<Boundary>>, request: Request, next: Next) -> Response {
    let headers = request.headers();
    if headers.get_all(header::HOST).iter().count() != 1
        || headers.get(header::HOST).and_then(|v| v.to_str().ok()) != Some(state.authority.as_str())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if headers.get_all(header::ORIGIN).iter().count() > 1
        || headers
            .get(header::ORIGIN)
            .is_some_and(|v| v.as_bytes() != format!("http://{}", state.authority).as_bytes())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if headers.get_all(header::AUTHORIZATION).iter().count() != 1
        || !headers
            .get(header::AUTHORIZATION)
            .is_some_and(|value| bool::from(value.as_bytes().ct_eq(state.authorization.as_bytes())))
    {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
        )
            .into_response();
    }
    let id = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let delete = request.method() == axum::http::Method::DELETE;
    // MCP requires 404 for an expired session. rmcp 0.16 returns 401 here,
    // which would incorrectly trigger credential refresh on a healthy key.
    if let Some(id) = &id {
        if !state
            .sessions
            .sessions
            .read()
            .await
            .contains_key(id.as_str())
        {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    // Bound chunked and declared bodies alike, before creating a session.
    let (parts, body) = request.into_parts();
    let bytes = match tokio::time::timeout(Duration::from_secs(15), to_bytes(body, MAX_BODY)).await
    {
        Ok(Ok(bytes)) => bytes,
        Ok(Err(_)) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
        Err(_) => return StatusCode::REQUEST_TIMEOUT.into_response(),
    };
    let _initialize_guard = if id.is_none() {
        let lock = state.initialize.lock().await;
        if state.sessions.sessions.read().await.len() >= MAX_SESSIONS {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
        Some(lock)
    } else {
        None
    };
    if let Some(id) = &id {
        if let Some(last_seen) = state.activity.lock().unwrap().get_mut(id) {
            *last_seen = Instant::now();
        }
    }
    let mut response = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    if id.is_some() && response.status() == StatusCode::UNAUTHORIZED {
        // A DELETE or idle expiry can race the check above. Authentication
        // already succeeded, so this remains a transport-session failure.
        *response.status_mut() = StatusCode::NOT_FOUND;
        response.headers_mut().remove(header::WWW_AUTHENTICATE);
    }
    if let Some(id) = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
    {
        state
            .activity
            .lock()
            .unwrap()
            .insert(id.to_owned(), Instant::now());
    }
    if delete && response.status().is_success() {
        if let Some(id) = id {
            state.activity.lock().unwrap().remove(&id);
        }
    }
    response
}

// Reconcile with the transport's registry as well as request timestamps: a
// disconnected initialize request may create a session before its HTTP response
// gets recorded. Those sessions must also age out instead of leaking forever.
async fn expire_idle(state: &Boundary) {
    let handles = {
        let mut sessions = state.sessions.sessions.write().await;
        let mut activity = state.activity.lock().unwrap();
        activity.retain(|id, _| sessions.contains_key(id.as_str()));
        for id in sessions.keys() {
            activity.entry(id.to_string()).or_insert_with(Instant::now);
        }
        let expired = activity
            .iter()
            .filter(|(_, time)| time.elapsed() >= IDLE_TTL)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|id| {
                activity.remove(&id);
                sessions.remove(id.as_str())
            })
            .collect::<Vec<_>>()
    };
    for handle in handles {
        let _ = tokio::time::timeout(Duration::from_secs(1), handle.close()).await;
    }
}

async fn reap_idle(state: Arc<Boundary>, stop: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            _ = interval.tick() => expire_idle(&state).await,
        }
    }
}

pub async fn run(server: KpGithubServer, config: HttpConfig) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(config.address)
        .await
        .context("Cannot bind shared HTTP listener (is it already running?)")?;
    let address = listener.local_addr()?;
    let stop = CancellationToken::new();
    let sessions = Arc::new(LocalSessionManager::default());
    let state = Arc::new(Boundary {
        authorization: config.authorization,
        authority: address.to_string(),
        sessions: sessions.clone(),
        activity: Mutex::new(HashMap::new()),
        initialize: tokio::sync::Mutex::new(()),
    });
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        sessions,
        StreamableHttpServerConfig {
            cancellation_token: stop.child_token(),
            ..Default::default()
        },
    );
    let router = Router::new()
        .nest_service("/mcp", service)
        .layer(middleware::from_fn_with_state(state.clone(), guard));
    let reaper = tokio::spawn(reap_idle(state, stop.child_token()));
    eprintln!("kp-github-mcp shared HTTP ready at http://{address}/mcp");
    let shutdown = stop.clone();
    let result = axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            #[cfg(unix)]
            {
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(mut term) => {
                        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
                    }
                    Err(_) => {
                        let _ = tokio::signal::ctrl_c().await;
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = tokio::signal::ctrl_c().await;
            }
            shutdown.cancel();
        })
        .await;
    stop.cancel();
    let _ = reaper.await;
    result.context("Shared HTTP server failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::transport::streamable_http_server::session::SessionManager;

    #[tokio::test]
    async fn idle_cleanup_preserves_active_sessions_and_tracks_interrupted_initialization() {
        let manager = Arc::new(LocalSessionManager::default());
        let (old, _old_transport) = manager.create_session().await.unwrap();
        let (active, _active_transport) = manager.create_session().await.unwrap();
        let (orphan, _orphan_transport) = manager.create_session().await.unwrap();
        let state = Boundary {
            authorization: "synthetic".into(),
            authority: "127.0.0.1:1".into(),
            sessions: manager.clone(),
            initialize: tokio::sync::Mutex::new(()),
            activity: Mutex::new(HashMap::from([
                (
                    old.to_string(),
                    Instant::now() - IDLE_TTL - Duration::from_secs(1),
                ),
                (active.to_string(), Instant::now()),
                ("already-deleted".into(), Instant::now()),
            ])),
        };
        expire_idle(&state).await;
        assert!(!manager.has_session(&old).await.unwrap());
        assert!(manager.has_session(&active).await.unwrap());
        assert!(manager.has_session(&orphan).await.unwrap());
        assert!(state.activity.lock().unwrap().contains_key(orphan.as_ref()));
        assert!(!state
            .activity
            .lock()
            .unwrap()
            .contains_key("already-deleted"));
        state.activity.lock().unwrap().insert(
            orphan.to_string(),
            Instant::now() - IDLE_TTL - Duration::from_secs(1),
        );
        expire_idle(&state).await;
        assert!(!manager.has_session(&orphan).await.unwrap());
        assert!(manager.has_session(&active).await.unwrap());
    }
}
