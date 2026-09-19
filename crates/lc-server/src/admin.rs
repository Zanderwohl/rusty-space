//! A read-only HTTP surface for the administration console.
//!
//! **No contact with the tick loop**: it reads the checkpoint and an `Arc` of the catalogue,
//! takes no lock the simulation wants, and cannot make a page load cost the world a frame.
//! Keep that if this grows a second route.
//!
//! On **its own port**, which no proxy route reaches — see `lightcone/docs/15-runbook.md`.
//!
//! Authorised by a **game ticket**: no shared secret to invent, no second key to rotate, and
//! the gate is `crate::ability`. The console mints one per request on behalf of the
//! administrator reading the page, so the ticket says which one is asking. Its own [`Spent`]
//! map rather than the tick loop's, which would be a lock for no gain.

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

/// `{account}` is the opaque account id.
pub const STATUS: &str = "/admin/status/{account}";

/// Paged and ordered on this side; see [`crate::systems`].
pub const SYSTEMS: &str = "/admin/systems";

#[derive(Clone)]
pub struct Api {
    pub db: Arc<tokio_postgres::Client>,
    /// Shared, not copied: the largest thing in the process.
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

/// **One refusal for every way of failing** — signature, audience, expiry, replay, level —
/// because a caller that could tell them apart could tell how close it got.
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

/// This crate has no chrono and one call does not earn it.
fn chrono_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// **Every field a string**, including the numbers: a typed `u64` makes `?limit=-1` an axum
/// rejection — a bare 400 — before any fallback here can run.
#[derive(serde::Deserialize, Default)]
struct Asked {
    q: Option<String>,
    sort: Option<String>,
    dir: Option<String>,
    offset: Option<String>,
    limit: Option<String>,
}

/// The limit arrives in a URL, and an unbounded one serialises the whole catalogue.
const MOST_PER_PAGE: u64 = 200;

async fn systems(
    State(api): State<Api>,
    headers: HeaderMap,
    axum::extract::Query(asked): axum::extract::Query<Asked>,
) -> Response {
    if admitted(&api, &headers).await.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    // The whole table, because a tally is over all of it. First thing to change at scale.
    let ships = match lc_store::ships::load_ships(&api.db).await {
        Ok(ships) => ships,
        Err(why) => {
            eprintln!("admin systems: the store did not answer: {why}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let at: Vec<glam::DVec3> = ships
        .iter()
        // Left out rather than counted somewhere arbitrary.
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
        // An answer, not an error: an account that never connected has none.
        Ok(None) => return (StatusCode::NOT_FOUND, "None").into_response(),
        Err(why) => {
            eprintln!("admin status: the store did not answer: {why}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let saved = match crate::persist::decode(&ship) {
        Ok(saved) => saved,
        // Not rendered as an empty ship, which reads as a player who owns nothing.
        Err(why) => {
            eprintln!("admin status: ship {} did not decode: {why}", ship.ship_id);
            return StatusCode::CONFLICT.into_response();
        }
    };

    let status = Status::of(ship.ship_id, ship.saved_t, &saved, &api.stars);
    // "Who asked about whom" is a thing an administration wants to answer about itself.
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
