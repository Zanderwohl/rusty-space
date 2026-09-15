//! The stylesheet, which is the whole of this service's static surface.
//!
//! Compiled by `build.rs` and baked in, so the handler reads no file and the image ships no
//! `static/` tree. The URL carries a digest of the CSS, which is what lets the response be
//! cached forever: a changed stylesheet is a changed URL, with no deploy-time coordination.

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};

const CSS: &str = include_str!(concat!(env!("OUT_DIR"), "/identity.css"));

/// A digest of [`CSS`], from `build.rs`.
pub const DIGEST: &str = env!("STYLE_DIGEST");

/// Where the sheet is served. The `{digest}` segment is not checked against [`DIGEST`]: this
/// handler can only ever answer with the bytes this binary was built from, so a request
/// carrying an older one is correctly given the current sheet.
pub const ROUTE: &str = "/v/{digest}/styles/identity.css";

pub fn url() -> String {
    format!("/v/{DIGEST}/styles/identity.css")
}

pub async fn serve() -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        CSS,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_url_and_the_route_agree() {
        assert_eq!(url(), ROUTE.replace("{digest}", DIGEST));
    }

    /// The sheet compiled, and it is the compiled sheet rather than its source.
    #[test]
    fn the_stylesheet_is_css() {
        assert!(!CSS.is_empty());
        assert!(CSS.contains("--accent"), "the tokens are missing");
        assert!(!CSS.contains("@use"), "this is SCSS, not CSS");
    }

    /// **The palette is the site's, byte for byte.**
    ///
    /// The two services are separate cargo workspaces with separate docker build contexts, so
    /// the file cannot be shared — it is copied, and a copy with nothing watching it drifts.
    /// This is what watches it. If it fails, one of the two was edited: copy the site's over
    /// this one, and put whatever is genuinely broker-only in `_status.scss`.
    #[test]
    fn the_tokens_are_the_sites_tokens() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let ours = here.join("static/styles/_tokens.scss");
        // `auth/lc-identity` -> the repository root.
        let theirs = here.join("../../web/static/styles/_tokens.scss");
        assert!(
            theirs.exists(),
            "{} is missing; has the site moved?",
            theirs.display()
        );
        assert_eq!(
            std::fs::read_to_string(&ours).expect("our tokens"),
            std::fs::read_to_string(&theirs).expect("the site's tokens"),
            "the broker's palette has drifted from the site's",
        );
    }
}
