//! The Lightcone website.
//!
//! Every page is a document a browser renders on arrival. Anything that wants real
//! interactivity belongs in the client, which is a Bevy application with a WebGPU context.
//! See `lightcone/docs/14-hosting.md`.

mod assets;
mod config;
mod views;

use std::time::Duration;

use axum::Router;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::assets::Assets;
use crate::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "lc_web=info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_env()?;

    // The stylesheet is compiled at boot, so a syntax error in it is a failed deploy rather
    // than a failed build. This flag lets CI move that leftward.
    if std::env::args().any(|a| a == "--check-styles") {
        Assets::load(&config.static_dir)?;
        println!("stylesheet ok");
        return Ok(());
    }

    tracing::info!(build = assets::BUILD, env = ?config.env, "starting");
    let assets = Assets::load(&config.static_dir)?;

    #[cfg(feature = "watch")]
    if !config.env.is_production() {
        watch::spawn(config.static_dir.clone(), assets.clone());
    }

    let app = Router::new()
        .route("/", get(views::home::page))
        .route("/about", get(views::about::page))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v/{build}/{*path}", get(assets::serve))
        .fallback(not_found)
        .with_state(assets)
        .layer(TraceLayer::new_for_http())
        .layer(CompressionLayer::new())
        .layer(TimeoutLayer::with_status_code(StatusCode::GATEWAY_TIMEOUT, Duration::from_secs(15)))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-lightcone-build"),
            HeaderValue::from_static(assets::BUILD),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ));

    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    axum::serve(listener, app).with_graceful_shutdown(shutdown()).await?;
    tracing::info!("stopped");
    Ok(())
}

/// Liveness. Touches nothing, so a dependency outage does not get the container restarted.
async fn healthz() -> &'static str {
    "ok"
}

/// Readiness. Nothing to check yet — the database arrives in W4, and the site is designed to
/// serve without it even then.
async fn readyz() -> &'static str {
    "ready"
}

async fn not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        views::shell(
            "Not found",
            "No page at that address.",
            maud::html! {
                section class="stack" {
                    h1 { "No page here" }
                    p class="lede" { "The address is wrong, or the page has moved." }
                    p { a class="cta" href="/" { "Back to the start" } }
                }
            },
        ),
    )
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

#[cfg(feature = "watch")]
mod watch {
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;

    use notify::{RecursiveMode, Watcher};

    use crate::assets::Assets;

    /// Recompiles the stylesheet when a `.scss` under `static/styles` changes.
    ///
    /// A failed recompile keeps the previous sheet: losing styling mid-session for a typo in
    /// progress is worse than serving something one save out of date.
    pub fn spawn(root: PathBuf, assets: Assets) {
        std::thread::spawn(move || {
            let (tx, rx) = mpsc::channel();
            let mut watcher = match notify::recommended_watcher(tx) {
                Ok(w) => w,
                Err(e) => return tracing::error!(error = %e, "no stylesheet watcher"),
            };
            let styles = root.join("styles");
            if let Err(e) = watcher.watch(&styles, RecursiveMode::Recursive) {
                return tracing::error!(error = %e, path = %styles.display(), "cannot watch");
            }
            tracing::info!(path = %styles.display(), "watching stylesheets");
            while let Ok(event) = rx.recv() {
                let Ok(event) = event else { continue };
                if !event.paths.iter().any(|p| p.extension().is_some_and(|e| e == "scss")) {
                    continue;
                }
                // Editors write a file in several steps; let the burst settle.
                std::thread::sleep(Duration::from_millis(50));
                while rx.recv_timeout(Duration::from_millis(50)).is_ok() {}
                if let Err(e) = assets.recompile() {
                    tracing::error!(error = %e, "stylesheet recompile failed; keeping previous");
                }
            }
        });
    }
}
