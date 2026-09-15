//! The broker as a process: read the environment, say what is live, migrate, serve.

use std::net::SocketAddr;
use std::sync::Arc;

use lc_identity::attempts::Attempts;
use lc_identity::config::Config;
use lc_identity::routes::{Broker, router};
use lc_identity::store::Store;
use lc_identity::ticket::Keys;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env()?;
    // The line that answers "which providers does production have" without anyone having to
    // remember. A provider absent from it is a provider nobody can use.
    tracing::info!(
        providers = %config.providers.live().iter().map(|p| p.name()).collect::<Vec<_>>().join(","),
        return_to = config.return_to.len(),
        "identity broker starting",
    );

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&config.database_url)
        .await?;
    // Applied at boot, from the copy that shipped in the image: the schema and the code that
    // assumes it are one artifact, which is the only way a rollback rolls back both.
    sqlx::migrate!("./migrations").run(&pool).await?;

    // A seed in the environment in production; a fresh key otherwise. A generated key is said
    // out loud because it means every restart publishes a different one, and anything caching
    // the key set will refuse tickets until it refetches.
    let keys = match &config.signing_seed {
        Some(encoded) => {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded)?;
            let seed: [u8; 32] = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("LC_IDENTITY_SIGNING_SEED must be 32 bytes"))?;
            Keys::from_seed(&seed, &config.issuer)?
        }
        None => {
            tracing::warn!(
                "no LC_IDENTITY_SIGNING_SEED: generating one, which will not survive a restart"
            );
            Keys::generate(&config.issuer)?
        }
    };
    tracing::info!(
        kid = keys.kid(),
        audiences = config.audiences.len(),
        "signing key ready"
    );

    let bind = config.bind;
    let broker = Broker {
        config: Arc::new(config),
        store: Store::Postgres(pool),
        attempts: Arc::new(Attempts::default()),
        keys: Arc::new(keys),
    };

    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!(%bind, "listening");
    // The address is needed per request for the per-address attempt budget.
    axum::serve(
        listener,
        router(broker).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
