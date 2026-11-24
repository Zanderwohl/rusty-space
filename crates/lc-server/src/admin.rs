//! The HTTP surface for the administration console. Read-only but for [`STANDING`].
//!
//! **No lock shared with the tick loop**: it reads the checkpoint and an `Arc` of the catalog,
//! and [`STANDING`] only queues a message the loop drains, so a page load cannot cost the world
//! a frame.
//!
//! On **its own port**, which no proxy route reaches — see `lightcone/docs/15-runbook.md`.
//!
//! Authorized by a **game ticket**: no shared secret to invent, no second key to rotate, and
//! the gate is `crate::ability`. The console mints one per request on behalf of the
//! administrator reading the page, so the ticket says which one is asking. Its own [`Spent`]
//! map rather than the tick loop's, which would be a lock for no gain.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use lc_world::sky::CatalogStar;
use tokio::sync::Mutex;

use crate::ability::Level;
use crate::status::Status;
use crate::ticket::{Spent, Trusted};

/// `{account}` is the opaque account id.
pub const STATUS: &str = "/admin/status/{account}";

/// Paged and ordered on this side; see [`crate::systems`].
pub const SYSTEMS: &str = "/admin/systems";

/// `{id}` is a system's id as [`SYSTEMS`] lists it.
pub const SYSTEM: &str = "/admin/systems/{id}";

/// `POST`, with `banned=true` and an optional `until` in Unix seconds, or `banned=false`: the
/// account's standing after a ban or a lift. See [`crate::standing`].
pub const STANDING: &str = "/admin/standing/{account}";

/// Every ship ever launched, newest first, from `ship_registry`.
pub const SHIPS: &str = "/admin/ships";

/// `{id}` is a ship id.
pub const SHIP: &str = "/admin/ships/{id}";

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShipRow {
    pub ship_id: i64,
    pub account: Option<String>,
    pub name: Option<String>,
    /// Unix seconds.
    pub launched_at: i64,
    /// Unix seconds; `None` while it flies.
    pub ended_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShipPage {
    pub ships: Vec<ShipRow>,
    pub total: i64,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ShipDetail {
    pub ship: ShipRow,
    /// From the checkpoint, which keeps a wreck only until its light has faded.
    pub status: Option<Status>,
}

impl From<lc_store::registry::Record> for ShipRow {
    fn from(r: lc_store::registry::Record) -> Self {
        ShipRow { ship_id: r.ship_id, account: r.account, name: r.name, launched_at: r.launched_at, ended_at: r.ended_at }
    }
}

#[derive(Clone)]
pub struct Api {
    pub db: Arc<tokio_postgres::Client>,
    /// Shared, not copied: the largest thing in the process.
    pub stars: Arc<Vec<CatalogStar>>,
    pub trusted: Arc<Trusted>,
    spent: Arc<Mutex<Spent>>,
    standings: tokio::sync::mpsc::UnboundedSender<crate::standing::Stated>,
}

impl Api {
    pub fn new(
        db: Arc<tokio_postgres::Client>,
        stars: Arc<Vec<CatalogStar>>,
        trusted: Arc<Trusted>,
        standings: tokio::sync::mpsc::UnboundedSender<crate::standing::Stated>,
    ) -> Api {
        Api {
            db,
            stars,
            trusted,
            spent: Arc::new(Mutex::new(Spent::default())),
            standings,
        }
    }
}

pub fn router(api: Api) -> Router {
    Router::new()
        .route(STATUS, get(status))
        .route(SYSTEMS, get(systems))
        .route(SYSTEM, get(system))
        .route(SHIPS, get(ships))
        .route(SHIP, get(ship))
        .route(STANDING, post(standing))
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
    account: Option<String>,
    /// `flying` or `ended`.
    state: Option<String>,
}

/// The limit arrives in a URL, and an unbounded one serializes the whole catalog.
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

    ron_response(&page, "systems")
}

async fn system(State(api): State<Api>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if admitted(&api, &headers).await.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let found = id.parse::<u64>().ok().and_then(|id| api.stars.iter().find(|s| s.id.get() == id));
    let Some(star) = found else {
        return (StatusCode::NOT_FOUND, "None").into_response();
    };

    let ships = match lc_store::ships::load_ships(&api.db).await {
        Ok(ships) => ships,
        Err(why) => {
            eprintln!("admin system: the store did not answer: {why}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let craft: Vec<_> = ships
        .iter()
        .filter_map(|ship| {
            let saved = crate::persist::decode(ship).ok()?;
            Some(crate::systems::Aboard {
                ship_id: ship.ship_id,
                name: saved.name,
                account: ship.account.clone(),
                saved_t: ship.saved_t,
                motion: saved.motion,
            })
        })
        .collect();

    let subject = lc_proto::encode(&lc_world::knowledge::Subject::Star(star.id));
    let rows = match lc_store::knowledge::files_on(&api.db, &subject).await {
        Ok(rows) => rows,
        Err(why) => {
            eprintln!("admin system: the store did not answer: {why}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let files: Vec<_> = rows
        .iter()
        .filter_map(|row| match crate::archive::read_file(row) {
            Ok((_, file)) => Some((row.ship_id, file)),
            // Left out, as an undecodable ship is from the tally: one bad row is not a 500.
            Err(why) => {
                eprintln!("admin system: craft {}'s file did not decode: {why}", row.ship_id);
                None
            }
        })
        .collect();
    let files: Vec<_> = files.iter().map(|(ship, file)| (*ship, file)).collect();

    ron_response(&crate::systems::detail(star, &api.stars, &craft, &files), "system")
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

    ron_response(&status, "status")
}

async fn ships(
    State(api): State<Api>,
    headers: HeaderMap,
    axum::extract::Query(asked): axum::extract::Query<Asked>,
) -> Response {
    if admitted(&api, &headers).await.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let query = lc_store::registry::Query {
        search: asked.q.filter(|q| !q.trim().is_empty()).map(|q| q.trim().to_owned()),
        account: asked.account.filter(|a| !a.is_empty()),
        flying: match asked.state.as_deref() {
            Some("flying") => Some(true),
            Some("ended") => Some(false),
            _ => None,
        },
        offset: asked.offset.and_then(|n| n.parse().ok()).unwrap_or(0).max(0),
        limit: asked.limit.and_then(|n| n.parse().ok()).unwrap_or(25).clamp(1, MOST_PER_PAGE as i64),
    };
    match lc_store::registry::list(&api.db, &query).await {
        Ok((rows, total)) => ron_response(&ShipPage { ships: rows.into_iter().map(Into::into).collect(), total }, "ships"),
        Err(why) => {
            eprintln!("admin ships: the store did not answer: {why}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn ship(State(api): State<Api>, headers: HeaderMap, Path(id): Path<String>) -> Response {
    if admitted(&api, &headers).await.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(id) = id.parse::<i64>() else {
        return (StatusCode::NOT_FOUND, "None").into_response();
    };
    let (record, saved) = match (lc_store::registry::get(&api.db, id).await, lc_store::ships::ship_by_id(&api.db, id).await) {
        (Ok(record), Ok(saved)) => (record, saved),
        (Err(why), _) | (_, Err(why)) => {
            eprintln!("admin ship: the store did not answer: {why}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Some(record) = record else {
        return (StatusCode::NOT_FOUND, "None").into_response();
    };
    let status = saved.and_then(|ship| {
        let decoded = crate::persist::decode(&ship).map_err(|why| eprintln!("admin ship: ship {id} did not decode: {why}")).ok()?;
        Some(Status::of(ship.ship_id, ship.saved_t, &decoded, &api.stars))
    });
    ron_response(&ShipDetail { ship: record.into(), status }, "ship")
}

#[derive(serde::Deserialize, Default)]
struct Stand {
    banned: Option<String>,
    until: Option<String>,
}

async fn standing(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(account): Path<String>,
    axum::extract::Query(asked): axum::extract::Query<Stand>,
) -> Response {
    if admitted(&api, &headers).await.is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(standing) = stand(&asked) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match api.standings.send(crate::standing::Stated { account, standing }) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// `None` for an unknown `banned`, or an `until` that is not a number, which would otherwise
/// read as a permanent ban.
fn stand(asked: &Stand) -> Option<crate::standing::Standing> {
    use crate::standing::Standing;
    match asked.banned.as_deref()? {
        "false" => Some(Standing::Clear),
        "true" => {
            let until_s = match &asked.until {
                None => None,
                Some(until) => Some(until.parse().ok()?),
            };
            Some(Standing::Banned { until_s })
        }
        _ => None,
    }
}

fn ron_response(body: &impl serde::Serialize, route: &str) -> Response {
    match ron::to_string(body) {
        Ok(body) => (
            [(
                header::CONTENT_TYPE,
                "application/ron; charset=utf-8".to_owned(),
            )],
            body,
        )
            .into_response(),
        Err(why) => {
            eprintln!("admin {route}: could not serialize: {why}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the console's `shard.rs` pins as captured. A change here is a change there.
    pub const SHIPS_PAYLOAD: &str = r#"(ships:[(ship_id:12,account:Some("6f1c2a4e-0d5b-4c43-9a57-3b1e2f9d8c70"),name:Some("Tachi"),launched_at:1795000000,ended_at:None),(ship_id:9,account:Some("6f1c2a4e-0d5b-4c43-9a57-3b1e2f9d8c70"),name:Some("Canterbury"),launched_at:1794000000,ended_at:Some(1794500000))],total:2)"#;

    #[test]
    fn a_standing_is_read_strictly() {
        use crate::standing::Standing;
        let asked = |banned: Option<&str>, until: Option<&str>| stand(&Stand { banned: banned.map(Into::into), until: until.map(Into::into) });
        assert_eq!(asked(Some("true"), None), Some(Standing::Banned { until_s: None }));
        assert_eq!(asked(Some("true"), Some("1795000000")), Some(Standing::Banned { until_s: Some(1_795_000_000) }));
        assert_eq!(asked(Some("true"), Some("soon")), None);
        assert_eq!(asked(Some("false"), None), Some(Standing::Clear));
        assert_eq!(asked(None, None), None);
    }

    #[test]
    fn a_ship_page_serializes_as_the_console_reads_it() {
        let account = Some("6f1c2a4e-0d5b-4c43-9a57-3b1e2f9d8c70".to_owned());
        let page = ShipPage {
            ships: vec![
                ShipRow { ship_id: 12, account: account.clone(), name: Some("Tachi".into()), launched_at: 1_795_000_000, ended_at: None },
                ShipRow { ship_id: 9, account, name: Some("Canterbury".into()), launched_at: 1_794_000_000, ended_at: Some(1_794_500_000) },
            ],
            total: 2,
        };
        assert_eq!(ron::to_string(&page).unwrap(), SHIPS_PAYLOAD);
        let detail = ShipDetail { ship: page.ships[1].clone(), status: None };
        assert_eq!(
            ron::to_string(&detail).unwrap(),
            r#"(ship:(ship_id:9,account:Some("6f1c2a4e-0d5b-4c43-9a57-3b1e2f9d8c70"),name:Some("Canterbury"),launched_at:1794000000,ended_at:Some(1794500000)),status:None)"#,
        );
    }
}
