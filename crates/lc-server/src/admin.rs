//! A read-only HTTP surface for the administration console.
//!
//! One route, one answer, and **no contact with the tick loop**. It reads the checkpoint out
//! of the shard's own database and the star catalogue out of an `Arc`; it takes no lock the
//! simulation wants, sends nothing down a channel the tick reads, and cannot make a page load
//! cost the world a frame. That is the property to preserve if this ever grows a second route.
//!
//! It listens on **its own port**, which is not the one players reach. The game socket is
//! proxied to the internet; this is not, and the deployment keeps it that way by simply not
//! publishing a route to it — see `lightcone/docs/15-runbook.md`.
//!
//! ## What authorises a request
//!
//! A **game ticket**, the same object a client presents to open a socket: signed by the
//! broker, audience-scoped to this shard, sixty seconds long, carrying the account's level.
//! Nothing new was invented for this — no shared secret between the console and the shard, no
//! second key to rotate — and the gate is `crate::ability`, which already knows what a level
//! means.
//!
//! The console mints one per request through the broker's `/ticket`, on behalf of the
//! administrator who is looking at the page. So the ticket says *which* administrator is
//! asking, and a shard log line can say so too.
//!
//! Tickets are single use here as everywhere: this holds its own [`Spent`] map rather than
//! sharing the tick loop's, because sharing it would be a lock between an HTTP handler and
//! the simulation for no gain — the two surfaces are spending different tickets.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use lc_world::sky::CatalogueStar;
use tokio::sync::Mutex;

use crate::ability::Level;
use crate::status::Status;
use crate::ticket::{Spent, Trusted};

/// Where the console asks. `{account}` is the opaque account id, the only identifier that
/// crosses a product boundary.
pub const STATUS: &str = "/admin/status/{account}";

#[derive(Clone)]
pub struct Api {
    pub db: Arc<tokio_postgres::Client>,
    /// The catalogue this shard was started with. Shared, not copied: it is the largest thing
    /// in the process and there is no reason for a second one.
    pub stars: Arc<Vec<CatalogueStar>>,
    pub trusted: Arc<Trusted>,
    spent: Arc<Mutex<Spent>>,
}

impl Api {
    pub fn new(
        db: Arc<tokio_postgres::Client>,
        stars: Arc<Vec<CatalogueStar>>,
        trusted: Arc<Trusted>,
    ) -> Api {
        Api {
            db,
            stars,
            trusted,
            spent: Arc::new(Mutex::new(Spent::default())),
        }
    }
}

pub fn router(api: Api) -> Router {
    Router::new()
        .route(STATUS, get(status))
        .route("/admin/health", get(|| async { "ok" }))
        .with_state(api)
}

/// The ticket's claims, if the request carries one this shard will accept from an
/// administrator.
///
/// **One refusal for every way of failing.** Wrong signature, wrong audience, expired, spent,
/// or an account that administers nothing all answer 401 with no body — a caller that could
/// tell them apart could tell how close it got.
async fn admitted(api: &Api, headers: &HeaderMap) -> Option<crate::ticket::Claims> {
    let token = headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")?;
    let claims = api.trusted.check(token).ok()?;
    if !Level::from_claim(claims.perm).is_admin() {
        return None;
    }
    let now = chrono_now();
    api.spent.lock().await.claim(&claims, now).ok()?;
    Some(claims)
}

/// Seconds since the epoch. Its own function because this crate has no chrono and one call
/// does not earn it.
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

async fn status(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(account): Path<String>,
) -> Response {
    let Some(claims) = admitted(&api, &headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let ship = match lc_store::ships::ship_for_account(&api.db, &account).await {
        Ok(Some(ship)) => ship,
        // No ship is an answer, not an error: an account that has never connected has none.
        Ok(None) => return (StatusCode::NOT_FOUND, "None").into_response(),
        Err(why) => {
            eprintln!("admin status: the store did not answer: {why}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let saved = match crate::persist::decode(&ship) {
        Ok(saved) => saved,
        // A checkpoint this build cannot read. Said plainly rather than rendered as an empty
        // ship, which would look like a player who owns nothing.
        Err(why) => {
            eprintln!("admin status: ship {} did not decode: {why}", ship.ship_id);
            return StatusCode::CONFLICT.into_response();
        }
    };

    let status = Status::of(ship.ship_id, ship.saved_t, &saved, &api.stars);
    // Logged, because "who asked about whom" is the kind of thing an administration wants to
    // be able to answer about itself.
    eprintln!("admin status: {} asked about {account}", claims.sub);

    match ron::to_string(&status) {
        Ok(body) => (
            [(
                header::CONTENT_TYPE,
                "application/ron; charset=utf-8".to_owned(),
            )],
            body,
        )
            .into_response(),
        Err(why) => {
            eprintln!("admin status: could not serialise: {why}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}
