//! A stylesheet, a script, and htmx.
//!
//! Read once at boot and served from memory, so the container runs read-only. One digest over
//! all three makes the one-year `immutable` header unconditionally true.
//!
//! Compiled here and not in `build.rs` as the broker does it, because `admin.js` is built by a
//! stage of the container and does not exist when `cargo build` runs — and one answer to
//! "where do assets come from" beats two.

use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// `{version}` is not checked: this can only answer with the bytes it loaded, so an older
/// digest is correctly given the current ones.
pub const ROUTE: &str = "/v/{version}/{kind}/{name}";

pub const STYLESHEET: &str = "styles/admin.css";
pub const SCRIPT: &str = "scripts/admin.js";
pub const HTMX: &str = "scripts/htmx.js";

const SCSS_ENTRY: &str = "styles/application.scss";
const TS_BUNDLE: &str = "js/admin.js";
const HTMX_BUNDLE: &str = "vendor/htmx.min.js";

#[derive(Clone)]
pub struct Assets {
    css: Arc<str>,
    script: Arc<str>,
    htmx: Arc<str>,
    version: Arc<str>,
}

impl Assets {
    /// A failure fails the boot: a console that came up without its stylesheet is an outage
    /// that returns 200.
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let entry = root.join(SCSS_ENTRY);
        let css = grass::from_path(&entry, &grass::Options::default())
            .map_err(|e| anyhow::anyhow!("compiling {}: {e}", entry.display()))?;

        let script = read(root, TS_BUNDLE).map_err(|e| {
            anyhow::anyhow!(
                "{e}\n\nThe TypeScript has not been compiled. Run `npm ci && npm run build` in \
                 auth/lc-admin, or build the container, which does it in its own stage.",
            )
        })?;
        let htmx = read(root, HTMX_BUNDLE)?;

        // Per-file digests would leave a window serving last deploy's script with this
        // deploy's stylesheet.
        let version = digest(&[css.as_bytes(), script.as_bytes(), htmx.as_bytes()]);
        tracing::info!(
            css = css.len(),
            script = script.len(),
            htmx = htmx.len(),
            %version,
            "assets loaded",
        );
        Ok(Assets {
            css: css.into(),
            script: script.into(),
            htmx: htmx.into(),
            version: version.into(),
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!("/v/{}/{path}", self.version)
    }
}

fn read(root: &Path, path: &str) -> anyhow::Result<String> {
    let full = root.join(path);
    std::fs::read_to_string(&full).map_err(|e| anyhow::anyhow!("reading {}: {e}", full.display()))
}

/// FNV-1a. A cache key, not a security boundary.
fn digest(parts: &[&[u8]]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for bytes in parts {
        for byte in *bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // Or moving a byte across a file boundary is the same digest.
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub async fn serve(
    State(assets): State<Assets>,
    axum::extract::Path((_version, kind, name)): axum::extract::Path<(String, String, String)>,
) -> Response {
    let asked = format!("{kind}/{name}");
    let (mime, body) = match asked.as_str() {
        STYLESHEET => ("text/css; charset=utf-8", &assets.css),
        SCRIPT => ("text/javascript; charset=utf-8", &assets.script),
        HTMX => ("text/javascript; charset=utf-8", &assets.htmx),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(mime)),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_route_and_the_names_agree() {
        // Every served name is two segments, which is what `ROUTE` matches. A one-segment or
        // three-segment name would 404 with nothing to say why.
        for name in [STYLESHEET, SCRIPT, HTMX] {
            assert_eq!(name.split('/').count(), 2, "{name} does not fit {ROUTE}");
        }
    }

    #[test]
    fn the_digest_notices_a_byte_moving_between_files() {
        assert_ne!(digest(&[b"ab", b"c"]), digest(&[b"a", b"bc"]));
        assert_eq!(digest(&[b"ab", b"c"]), digest(&[b"ab", b"c"]));
        assert_ne!(digest(&[b"a"]), digest(&[b"b"]));
        assert_eq!(digest(&[b"a"]).len(), 16);
    }

    /// The stylesheet compiles and the scripts are where the build puts them. A failure here
    /// is the same failure the boot would give, moved to somewhere it costs nothing.
    #[test]
    fn the_assets_in_this_tree_load() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
        match Assets::load(&root) {
            Ok(assets) => {
                assert!(assets.css.contains("--accent"), "the tokens are missing");
                assert!(!assets.css.contains("@use"), "this is SCSS, not CSS");
                // htmx is **vendored**, not fetched: the bytes are in the repository, in the
                // image, and served by this process under a digest of its own. A page that
                // pulled its script from somewhere else would put an administration console's
                // availability in a third party's hands, and its integrity too.
                assert!(assets.htmx.contains("htmx"), "that is not htmx");
                assert!(
                    assets.htmx.starts_with("var htmx=(()=>{"),
                    "that is not the minified htmx bundle",
                );
            }
            Err(why) => {
                let why = why.to_string();
                // The one acceptable failure: a checkout where `npm run build` has not run.
                // Anything else is a broken stylesheet and has to fail the test.
                assert!(why.contains("TypeScript has not been compiled"), "{why}",);
            }
        }
    }

    /// **The palette is the site's, byte for byte**, on the same reasoning as
    /// `lc_identity::assets::tests::the_tokens_are_the_sites_tokens`: three services now carry
    /// a copy of this file because they are three build contexts, and a copy with nothing
    /// watching it drifts. If this fails, copy the site's over this one and put whatever is
    /// genuinely administration-only in `_admin.scss`.
    #[test]
    fn the_tokens_are_the_sites_tokens() {
        let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let ours = here.join("static/styles/_tokens.scss");
        let theirs = here.join("../../web/static/styles/_tokens.scss");
        assert!(
            theirs.exists(),
            "{} is missing; has the site moved?",
            theirs.display(),
        );
        assert_eq!(
            std::fs::read_to_string(&ours).expect("our tokens"),
            std::fs::read_to_string(&theirs).expect("the site's tokens"),
            "the administration site's palette has drifted from the site's",
        );
    }
}
