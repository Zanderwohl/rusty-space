//! The one page that is not a document.
//!
//! Everything else on this site is HTML a browser renders on arrival. This hands over to a
//! Bevy application, and the only thing the server contributes is **where that application
//! lives** — a CDN base and a build id, which is the whole of the coupling between the site
//! and the game.

use axum::extract::{Query as AxumQuery, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use maud::{Markup, html};

use super::{Head, document};
use crate::AppState;
use crate::assets;
use crate::releases;

const DESCRIPTION: &str = "Fly a ship across a volume of real stars, where nothing you see is \
                           news and nothing you do is heard until its light arrives.";

/// Which build to launch, and where it lives.
///
/// In order: an explicit `?build=`, a named `?channel=`, the default channel, then
/// `FALLBACK_BUILD_ID`. The fallback is last and is why this page still works with the
/// database stopped.
async fn resolve(state: &AppState, query: &Query) -> Option<(String, String)> {
    if let Some(pool) = &state.pool {
        // A build id names a row. It is never a URL and never a path fragment: letting a query
        // string choose the origin of the script the page is about to execute is the
        // vulnerability this lookup exists to prevent.
        if let Some(build) = query.build.as_deref() {
            return match releases::get(pool, build).await {
                Ok(Some(r)) if !r.yanked => Some((r.build_id.clone(), r.base_url())),
                Ok(_) => None,
                Err(e) => {
                    tracing::error!(error = %e, "looking up a build");
                    None
                }
            };
        }
        let channel = query.channel.as_deref().unwrap_or(releases::DEFAULT_CHANNEL);
        match releases::for_channel(pool, channel).await {
            Ok(Some(r)) => return Some((r.build_id.clone(), r.base_url())),
            Ok(None) => tracing::warn!(channel, "channel names no usable build"),
            Err(e) => tracing::error!(error = %e, "resolving a channel"),
        }
    }

    // An explicit ?build= is never satisfied from the fallback: it would silently serve
    // something other than what was asked for, which is worse than saying no.
    if query.build.is_some() {
        return None;
    }
    let build = state.build_id.as_deref()?;
    Some((build.to_owned(), format!("{}/game/{}", state.cdn_base, build)))
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Query {
    pub build: Option<String>,
    pub channel: Option<String>,
}

pub async fn page(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumQuery(query): AxumQuery<Query>,
) -> Response {
    let Some((build, base)) = resolve(&state, &query).await else {
        return unavailable(&query).into_response();
    };

    // A deployment with no broker launches the game for anyone, which is the development and
    // single-player case and the state this site shipped in. One with a broker needs to know
    // who is asking, because the ticket is minted for an account.
    let ticket = match (state.identity().is_some(), state.who(&headers)) {
        (false, _) => None,
        (true, Some(session)) => match crate::auth::ticket(&state, &session).await {
            Some(ticket) => Some(ticket),
            // Signed in, and the broker would not mint. Not a sign-in problem, so do not send
            // them round the loop again: they would arrive back here and fail the same way.
            None => return unreachable_broker().into_response(),
        },
        (true, None) => return sign_in_first().into_response(),
    };

    document(
        Head::new("Play", DESCRIPTION),
        html! {
            // The canvas the client draws into. It is sized by CSS, and `fit_canvas_to_parent`
            // on the Bevy side follows that rather than the other way round.
            canvas #lightcone tabindex="0" {}

            // The handover, and the only thing the page tells the client. A data attribute
            // rather than an inline script, so the CSP needs no nonce for it.
            //
            // The ticket rides here too. It is worth sixty seconds and one socket, so it may
            // be in a page the browser will forget — and it is **not** in the URL, where it
            // would be in history, in an access log, and in a `Referer`.
            //
            // So does the shard's address. Not for secrecy — it is public — but because which
            // shard a build talks to is part of the handover rather than something a player
            // types, and a deployment moving it should be a configuration change here.
            div #boot data-base=(base) data-build=(build) data-ticket=[ticket.as_deref()]
                data-server=[state.shard_url.as_deref()] {
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

/// Signed out, on a deployment that needs an account.
fn sign_in_first() -> Markup {
    super::shell(
        Head::new("Play", DESCRIPTION),
        html! {
            section class="stack" {
                h1 { "Sign in to play" }
                p class="lede" {
                    "This server keeps a world per account, so it needs to know who you are "
                    "before it can hand you a ship."
                }
                p { a class="cta" href="/signin" { "Sign in" } }
            }
        },
    )
}

/// Signed in, and the broker would not answer.
///
/// Distinct from being signed out, and deliberately not a redirect to the sign-in: a player
/// sent round that loop arrives back here and fails the same way, having learned nothing.
fn unreachable_broker() -> Markup {
    super::shell(
        Head::new("Play", DESCRIPTION),
        html! {
            section class="stack" {
                h1 { "Cannot start a session" }
                p class="lede" {
                    "You are signed in, but the sign-in service did not answer. This is our "
                    "problem rather than yours; try again shortly."
                }
                p { a class="cta" href="/" { "Back to the start" } }
            }
        },
    )
}

/// Nothing to launch. Says so rather than showing a loader with nothing to load.
fn unavailable(query: &Query) -> Markup {
    super::shell(
        Head::new("Play", DESCRIPTION),
        html! {
            section class="stack" {
                @if let Some(build) = &query.build {
                    h1 { "No such build" }
                    p class="lede" {
                        "Nothing is published as " code { (build) } ", or it has been withdrawn."
                    }
                } @else {
                    h1 { "Nothing to play yet" }
                    p class="lede" {
                        "This site has no game build to launch — either nothing has been "
                        "promoted, or the build it pointed at has been withdrawn."
                    }
                }
                p {
                    "If you are running this yourself: promote a build onto a channel, or set "
                    code { "FALLBACK_BUILD_ID" }
                    " to a build id published under "
                    code { "CDN_BASE" }
                    "."
                }
                p { a class="cta" href="/" { "Back to the start" } }
            }
        },
    )
}
