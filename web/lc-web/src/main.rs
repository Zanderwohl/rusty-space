//! The Lightcone website.
//!
//! Every page is a document a browser renders on arrival. Anything that wants real
//! interactivity belongs in the client, which is a Bevy application with a WebGPU context.
//! See `lightcone/docs/14-hosting.md`.

mod assets;
mod config;
mod content;
mod feed;
mod views;

use std::time::Duration;

use std::sync::{Arc, RwLock};

use axum::Router;
use axum::extract::FromRef;
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use tower_http::compression::CompressionLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;

use crate::assets::Assets;
use crate::config::Config;
use crate::content::Content;

/// Everything a handler can reach. All of it is built at startup, which is what makes a
/// request an index lookup and a template render.
///
/// The content is behind a lock only so the development watcher can replace it. A reader
/// clones the `Arc` and drops the lock immediately, so a rebuild never blocks a render
/// half-finished, and in production nothing ever takes the write side.
#[derive(Clone)]
pub struct AppState {
    pub assets: Assets,
    content: Arc<RwLock<Arc<Content>>>,
    pub base_url: Arc<str>,
    /// Where game builds live. The site knows this and a build id, and nothing else about
    /// the game.
    pub cdn_base: Arc<str>,
    /// Which build `/play` launches, if any.
    pub build_id: Option<Arc<str>>,
}

impl AppState {
    pub fn content(&self) -> Arc<Content> {
        Arc::clone(&self.content.read().expect("content lock poisoned"))
    }

    #[cfg(feature = "watch")]
    fn replace_content(&self, next: Content) {
        *self.content.write().expect("content lock poisoned") = Arc::new(next);
    }
}

impl FromRef<AppState> for Assets {
    fn from_ref(state: &AppState) -> Assets {
        state.assets.clone()
    }
}

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
        Assets::load(&config.static_dir, true)?;
        // Content is checked here too: a malformed post fails the boot, and the same argument
        // applies -- that is a failed deploy where it could have been a failed build.
        Content::load(&config.content_dir, true)?;
        println!("stylesheet and content ok");
        return Ok(());
    }

    tracing::info!(
        build = assets::BUILD,
        env = ?config.env,
        game = config.fallback_build_id.as_deref().unwrap_or("none"),
        "starting"
    );
    let assets = Assets::load(&config.static_dir, config.env.is_production())?;
    // Drafts are loaded outside production and not loaded at all inside it. Excluding them at
    // load time rather than at render time means no handler can leak one by forgetting.
    let content = Content::load(&config.content_dir, !config.env.is_production())?;
    let state = AppState {
        assets: assets.clone(),
        content: Arc::new(RwLock::new(Arc::new(content))),
        base_url: config.base_url.clone().into(),
        cdn_base: config.cdn_base.clone().into(),
        build_id: config.fallback_build_id.clone().map(Into::into),
    };

    // Spawned before the router takes ownership of the state. Both hold the same Arc, so a
    // reload the watcher performs is visible to every handler.
    #[cfg(feature = "watch")]
    if !config.env.is_production() {
        watch::spawn(&config, assets.clone(), state.clone());
    }

    let app = Router::new()
        .route("/", get(views::home::page))
        .route("/about", get(views::page::about))
        .route("/play", get(views::play::page))
        .route("/blog", get(views::blog::index))
        .route("/blog/{slug}", get(views::blog::post))
        .route("/blog/tag/{tag}", get(views::blog::tag))
        .route("/feed.xml", get(feed::rss))
        .route("/feed.json", get(feed::json))
        .route("/sitemap.xml", get(feed::sitemap))
        .route("/robots.txt", get(feed::robots))
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v/{build}/{*path}", get(assets::serve))
        .fallback(not_found)
        .with_state(state)
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

pub async fn not_found() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        views::shell(
            views::Head::new("Not found", "No page at that address."),
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
    use std::sync::mpsc;
    use std::time::Duration;

    use notify::{RecursiveMode, Watcher};

    use crate::AppState;
    use crate::assets::Assets;
    use crate::config::Config;
    use crate::content::Content;

    /// Recompiles the stylesheet and reloads the content when either tree changes.
    ///
    /// A failure keeps what is already loaded: losing the whole site to a half-saved file is
    /// worse than serving something one keystroke out of date.
    pub fn spawn(config: &Config, assets: Assets, state: AppState) {
        let styles = config.static_dir.join("styles");
        let content_dir = config.content_dir.clone();
        let drafts = !config.env.is_production();
        std::thread::spawn(move || {
            let (tx, rx) = mpsc::channel();
            let mut watcher = match notify::recommended_watcher(tx) {
                Ok(w) => w,
                Err(e) => return tracing::error!(error = %e, "no asset watcher"),
            };
            for dir in [styles.as_path(), content_dir.as_path()] {
                if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                    tracing::error!(error = %e, path = %dir.display(), "cannot watch");
                }
            }
            tracing::info!(
                styles = %styles.display(),
                content = %content_dir.display(),
                "watching for changes"
            );

            while let Ok(event) = rx.recv() {
                let Ok(event) = event else { continue };
                let touched =
                    |ext: &str| event.paths.iter().any(|p| p.extension().is_some_and(|e| e == ext));
                let (scss, md) = (touched("scss"), touched("md"));
                if !scss && !md {
                    continue;
                }
                // Editors write a file in several steps; let the burst settle.
                std::thread::sleep(Duration::from_millis(50));
                while rx.recv_timeout(Duration::from_millis(50)).is_ok() {}

                if scss {
                    match assets.recompile() {
                        Ok(()) => tracing::info!("stylesheet reloaded"),
                        Err(e) => {
                            tracing::error!(error = %e, "stylesheet failed; keeping previous")
                        }
                    }
                }
                if md {
                    match Content::load(&content_dir, drafts) {
                        Ok(next) => {
                            state.replace_content(next);
                            tracing::info!("content reloaded");
                        }
                        Err(e) => tracing::error!(error = %e, "content failed; keeping previous"),
                    }
                }
            }
        });
    }
}
