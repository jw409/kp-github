mod auth;
mod compress;
mod events;
mod format;
mod github;
mod query;
mod server;
mod shared_http;
mod tools;
pub mod util;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mode = shared_http::Mode::parse(std::env::args().skip(1))?;
    if matches!(mode, shared_http::Mode::Help) {
        println!("{}", shared_http::HELP);
        return Ok(());
    }
    // Read/validate the local HTTP key before looking up any GitHub credential.
    let http = mode.prepare()?;

    // Initialize tracing to stderr (MCP uses stdout for JSON-RPC)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("KP_GITHUB_LOG_LEVEL")
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("kp-github-mcp starting");

    // Configuration is local and cached; creating the events client makes no
    // network requests. Its account selection also applies to GitHub tools.
    let events = events::EventsClient::from_environment()?;
    let token = match events.as_ref() {
        Some(client) => auth::resolve_token_for_account(Some(client.github_account()))?,
        None => auth::resolve_token()?,
    };
    tracing::info!("GitHub auth resolved");

    match http {
        Some(config) => shared_http::run(server::configured(&token, events), config).await,
        None => server::run(token, events).await,
    }
}
