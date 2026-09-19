//! The three files this service serves that are not pages: a stylesheet, a script of its own,
//! and htmx.
//!
//! All three are read once at boot and served from memory, so the container runs with a
//! read-only root filesystem and a request touches no disk. The URL carries a digest of all
//! three together, which is what makes the one-year `immutable` header unconditionally true:
//! changing any of them changes every asset URL, and there is no deploy-time coordination to
//! get wrong.
//!
//! The stylesheet is compiled here rather than in `build.rs`, which is where the broker
//! compiles its. The reason is the script beside it: `admin.js` is TypeScript compiled by a
//! stage of the container build, so it does not exist when `cargo build` runs. A service that
//! baked one asset into the binary and read the other off disk would have two answers to
//! "where do assets come from", and the one it gave you would depend on which asset you asked
//! about.

use std::path::Path;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

/// Where the compiled sheet, the script and htmx are served. The `{version}` segment is not
/// checked: this handler can only answer with the bytes this process loaded, so a request
/// carrying an older digest is correctly given the current ones.
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
    /// Compile the sheet and read the scripts.
    ///
    /// A failure here fails the boot. A site that came up without its stylesheet is an outage
    /// that returns 200, and one that came up without its script is an administration console
    /// whose filters silently stop writing the address bar.
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

        // One digest over all three, so any change to any of them changes every URL. A
        // per-file digest would be three cache keys and a window in which a page is served
        // with last deploy's script and this deploy's stylesheet.
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

/// FNV-1a, as hex. A cache key and not a security boundary: it only has to change when the
/// bytes do, which a 64-bit non-cryptographic hash does without a dependency.
fn digest(parts: &[&[u8]]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for bytes in parts {
        for byte in *bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // A separator, so moving a byte from the end of one file to the start of the next is
        // not the same digest.
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
                assert!(assets.htmx.contains("htmx"), "that is not htmx");
            }
            Err(why) => {
                let why = why.to_string();
                // The one acceptable failure: a checkout where `npm run build` has not run.
                // Anything else is a broken stylesheet and has to fail the test.
                assert!(
                    why.contains("TypeScript has not been compiled"),
                    "{why}",
                );
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
