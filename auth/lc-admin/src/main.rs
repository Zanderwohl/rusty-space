//! The administration site as a process: read the environment, connect, serve.

use std::net::SocketAddr;

use lc_admin::AppState;
use lc_admin::assets::Assets;
use lc_admin::config::Config;
use lc_admin::routes::router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lc_admin=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;
    // The return URL is here because a return URL missing from the broker's allowlist is the one
    // misconfiguration that fails only at the end of a sign-in.
    tracing::info!(
        identity = %config.identity_base,
        return_to = %config.return_url(),
        secure_cookies = config.secure_cookies,
        shard = config.shard_api.as_deref().unwrap_or("none"),
        "administration site starting",
    );

    let assets = Assets::load(&config.static_dir)?;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&config.database_url)
        .await?;
    // **No migrations**: `lc_identity::schema` owns them.
    sqlx::query("select 1").execute(&pool).await?;
    tracing::info!("database ready");

    let state = AppState {
        assets,
        pool,
        session_key: config.session_key.clone().into(),
        identity_base: config.identity_base.clone().into(),
        identity_api: config.identity_api.clone().into(),
        identity_secret: config.identity_secret.clone().into(),
        return_url: config.return_url().into(),
        secure_cookies: config.secure_cookies,
        shard_api: config.shard_api.clone().map(Into::into),
        shard_audience: config.shard_audience.clone().into(),
        http: lc_admin::http_client(),
    };

    let bind: SocketAddr = config.bind;
    let app = router(state)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            std::time::Duration::from_secs(15),
        ))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("x-content-type-options"),
            axum::http::HeaderValue::from_static("nosniff"),
        ))
        // A user page's URL carries an account identifier.
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("x-frame-options"),
            axum::http::HeaderValue::from_static("DENY"),
        ))
        .layer(tower_http::set_header::SetResponseHeaderLayer::overriding(
            axum::http::HeaderName::from_static("referrer-policy"),
            axum::http::HeaderValue::from_static("same-origin"),
        ));

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    tracing::info!("stopped");
    Ok(())
}

async fn shutdown() {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => tracing::info!("SIGTERM, draining"),
        _ = int.recv() => tracing::info!("SIGINT, draining"),
    }
}
