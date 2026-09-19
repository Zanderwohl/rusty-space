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

/// Where the console lists systems. Paged, filtered and ordered **here** rather than in the
/// console, for the reason the user index pages in SQL: handing over eight thousand systems
/// so twenty-five can be shown works today and stops working at a size nobody is watching.
pub const SYSTEMS: &str = "/admin/systems";

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
        .route(SYSTEMS, get(systems))
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

/// What the console asked for, exactly as it arrives.
///
/// **Every field is a string**, including the two numbers. A typed `u64` would make
/// `?limit=-1` a deserialisation failure, and axum answers one of those with a bare 400
/// before any code here runs — so an edited URL would be refused by the extractor rather than
/// falling back to something sensible. The console's own `Params` carries the same note for
/// the same reason.
#[derive(serde::Deserialize, Default)]
struct Asked {
    q: Option<String>,
    sort: Option<String>,
    dir: Option<String>,
    offset: Option<String>,
    limit: Option<String>,
}

/// Most systems a page may hold.
///
/// A ceiling rather than a suggestion: the limit arrives in a URL, and an unbounded one is a
/// request to serialise the whole catalogue into one response.
const MOST_PER_PAGE: u64 = 200;

async fn systems(
    State(api): State<Api>,
    headers: HeaderMap,
    axum::extract::Query(asked): axum::extract::Query<Asked>,
) -> Response {
    if admitted(&api, &headers).await.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // Every craft, to count them by system. The whole table because a tally is over all of
    // them — the first thing to change when a shard holds thousands, and the shape that
    // changes is this read, not the tally.
    let ships = match lc_store::ships::load_ships(&api.db).await {
        Ok(ships) => ships,
        Err(why) => {
            eprintln!("admin systems: the store did not answer: {why}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let at: Vec<glam::DVec3> = ships
        .iter()
        // A craft whose checkpoint this build cannot read is left out of the tally rather
        // than counted somewhere arbitrary. It is logged where it is decoded, not here.
        .filter_map(|ship| crate::persist::decode(ship).ok())
        .map(|saved| saved.motion.at_ly.into())
        .collect();

    let tally = crate::systems::tally(&at, &api.stars);
    let query = crate::systems::Query {
        q: asked.q.unwrap_or_default(),
        sort: crate::systems::Sort::from_slug(asked.sort.as_deref().unwrap_or("")),
        descending: asked.dir.as_deref() == Some("desc"),
        offset: asked.offset.and_then(|n| n.parse().ok()).unwrap_or(0),
        limit: asked
            .limit
            .and_then(|n| n.parse().ok())
            .unwrap_or(25)
            .clamp(1, MOST_PER_PAGE),
    };
    let page = crate::systems::page(&api.stars, &tally, &query);

    match ron::to_string(&page) {
        Ok(body) => (
            [(
                header::CONTENT_TYPE,
                "application/ron; charset=utf-8".to_owned(),
            )],
            body,
        )
            .into_response(),
        Err(why) => {
            eprintln!("admin systems: could not serialise: {why}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
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
