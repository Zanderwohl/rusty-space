//! The broker as a process: read the environment, say what is live, serve.

use axum::Router;
use axum::routing::get;
use lc_identity::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env()?;
    // The line that answers "which providers does production have" without anyone having to
    // remember. A provider absent from it is a provider nobody can use, and that should be
    // visible at a glance rather than by trying to sign in.
    tracing::info!(
        providers = %config.providers.live().iter().map(|p| p.name()).collect::<Vec<_>>().join(","),
        return_to = config.return_to.len(),
        "identity broker starting",
    );

    let app = Router::new().route("/health", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(bind = %config.bind, "listening");
    axum::serve(listener, app).await?;
    Ok(())
}
