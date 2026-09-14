//! Site resources: the compiled stylesheet, and everything else under `static/`.
//!
//! Served from `/v/<build>/…` with a one-year immutable header. The build segment is not
//! checked against the running build — it exists only to make the URL change when the image
//! does, and this handler can only ever serve files this image ships, so a request carrying
//! an older build id is correctly answered with the current bytes.
//!
//! Note this is the *site's* build id. The game's, which versions the wasm on the CDN, is a
//! different string on a different schedule and the two are never compared.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::ServeDir;

pub const BUILD: &str = env!("SITE_BUILD");

/// Where `application.scss` lives below the static root, and the URL path the compiled sheet
/// is served at. One constant so the template, the watcher and the handler cannot disagree.
pub const STYLESHEET: &str = "styles/application.css";
const SCSS_ENTRY: &str = "styles/application.scss";

const IMMUTABLE: &str = "public, max-age=31536000, immutable";

#[derive(Clone)]
pub struct Assets {
    /// Compiled CSS held in memory rather than written beside its source: the container runs
    /// on a read-only root filesystem, and a server whose first act is to write into its own
    /// image is a server that cannot start.
    css: Arc<RwLock<Arc<str>>>,
    files: ServeDir,
    #[cfg_attr(not(feature = "watch"), allow(dead_code))]
    root: PathBuf,
}

impl Assets {
    /// Compiles the stylesheet. A failure here fails the boot: a site that came up without a
    /// stylesheet is an outage that returns 200, which is worse than an outage.
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let css = compile(root)?;
        Ok(Assets {
            css: Arc::new(RwLock::new(css)),
            files: ServeDir::new(root).precompressed_br().precompressed_gzip(),
            root: root.to_path_buf(),
        })
    }

    #[cfg(feature = "watch")]
    /// Recompiles in place. Used by the dev watcher; an error is logged and the previous
    /// sheet is kept, because losing the stylesheet mid-session is worse than staleness.
    pub fn recompile(&self) -> anyhow::Result<()> {
        let next = compile(&self.root)?;
        *self.css.write().expect("assets lock poisoned") = next;
        Ok(())
    }

    fn css(&self) -> Arc<str> {
        Arc::clone(&self.css.read().expect("assets lock poisoned"))
    }
}

fn compile(root: &Path) -> anyhow::Result<Arc<str>> {
    let entry = root.join(SCSS_ENTRY);
    let css = grass::from_path(&entry, &grass::Options::default())
        .map_err(|e| anyhow::anyhow!("compiling {}: {e}", entry.display()))?;
    tracing::info!(bytes = css.len(), "stylesheet compiled");
    Ok(css.into())
}

/// The URL for a site resource, e.g. `asset_url("styles/application.css")`.
pub fn url(path: &str) -> String {
    format!("/v/{BUILD}/{path}")
}

pub async fn serve(State(assets): State<Assets>, request: Request) -> Response {
    let Some(path) = strip_version(request.uri().path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    if path == STYLESHEET {
        let css = assets.css();
        return (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8")),
                (header::CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE)),
            ],
            Body::from(css.to_string()),
        )
            .into_response();
    }

    // Rewrite to the unversioned path and let ServeDir answer. It is infallible, and answers
    // a missing or traversing path with 404.
    let Ok(uri) = format!("/{path}").parse::<Uri>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut request = request;
    *request.uri_mut() = uri;

    let mut response = match assets.files.clone().oneshot(request).await {
        Ok(r) => r.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "static file service failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if response.status().is_success() {
        response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(IMMUTABLE));
    }
    response
}

/// `/v/<build>/styles/application.css` becomes `styles/application.css`.
///
/// Deliberately textual: the path handed to `ServeDir` is byte-for-byte the one the browser
/// asked for, with no percent-decode and re-encode round trip in between.
fn strip_version(path: &str) -> Option<&str> {
    let (_build, rest) = path.strip_prefix("/v/")?.split_once('/')?;
    (!rest.is_empty()).then_some(rest)
}

#[cfg(test)]
mod tests {
    use super::strip_version;

    #[test]
    fn strips_the_build_segment() {
        assert_eq!(strip_version("/v/a1b2c3d/styles/application.css"), Some("styles/application.css"));
        assert_eq!(strip_version("/v/anything/images/a.png"), Some("images/a.png"));
    }

    #[test]
    fn rejects_paths_without_one() {
        assert_eq!(strip_version("/styles/application.css"), None);
        assert_eq!(strip_version("/v/"), None);
        assert_eq!(strip_version("/v/a1b2c3d"), None);
        assert_eq!(strip_version("/v/a1b2c3d/"), None);
    }
}
