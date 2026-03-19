mod auth;
mod compress;
mod format;
mod github;
mod query;
mod server;
mod tools;
pub mod util;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing to stderr (MCP uses stdout for JSON-RPC)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("KP_GITHUB_LOG_LEVEL")
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("kp-github-mcp starting");

    // Resolve auth
    let token = auth::resolve_token()?;
    tracing::info!("GitHub auth resolved");

    // Start MCP server on stdio
    server::run(token).await
}
