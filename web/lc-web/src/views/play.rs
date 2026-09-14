//! The one page that is not a document.
//!
//! Everything else on this site is HTML a browser renders on arrival. This hands over to a
//! Bevy application, and the only thing the server contributes is **where that application
//! lives** — a CDN base and a build id, which is the whole of the coupling between the site
//! and the game.

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use maud::{Markup, html};

use super::{Head, document};
use crate::AppState;
use crate::assets;

const DESCRIPTION: &str = "Fly a ship across a volume of real stars, where nothing you see is \
                           news and nothing you do is heard until its light arrives.";

pub async fn page(State(state): State<AppState>) -> Response {
    let Some(build) = state.build_id.as_deref() else {
        return unavailable().into_response();
    };
    let base = format!("{}/game/{}", state.cdn_base, build);

    document(
        Head::new("Play", DESCRIPTION),
        html! {
            // The canvas the client draws into. It is sized by CSS, and `fit_canvas_to_parent`
            // on the Bevy side follows that rather than the other way round.
            canvas #lightcone tabindex="0" {}

            // The handover, and the only thing the page tells the client. A data attribute
            // rather than an inline script, so the CSP needs no nonce for it.
            div #boot data-base=(base) data-build=(build) {
                h1 #stage { "Starting" }
                p #detail { "Checking what this browser can do." }
                progress #bar max="100" value="0" hidden {}
                p class="fine-print" { "Build " code { (build) } }
            }

            script type="module" src=(assets::url("scripts/play.js")) {}
        },
    )
    .into_response()
}

/// No build configured. Says so rather than showing a loader that cannot load anything.
fn unavailable() -> Markup {
    super::shell(
        Head::new("Play", DESCRIPTION),
        html! {
            section class="stack" {
                h1 { "Nothing to play yet" }
                p class="lede" {
                    "This site has no game build configured, so there is nothing for this page to "
                    "launch."
                }
                p {
                    "If you are running this yourself, that is "
                    code { "FALLBACK_BUILD_ID" }
                    " — the build id to serve from "
                    code { "CDN_BASE" }
                    "."
                }
                p { a class="cta" href="/" { "Back to the start" } }
            }
        },
    )
}
