//! Which game build the site serves, and how that changes.
//!
//! Registering a build and promoting one are separate acts. A build lands in `releases` when
//! CI has put it on the CDN, which says only that it exists; a `channels` row is what decides
//! that players get it. Rollback is the same statement with an older id, and touches no
//! deploy, no container and no CDN.
//!
//! **Everything here is optional.** The pool may be absent or unreachable, and the site is
//! expected to keep serving — `/play` falls back to `FALLBACK_BUILD_ID`. Losing this costs
//! release management, not the site.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, PgPool};

/// The channel served when a request does not name one.
pub const DEFAULT_CHANNEL: &str = "stable";

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Release {
    pub build_id: String,
    pub cdn_base: String,
    pub wasm_bytes: Option<i64>,
    pub notes: Option<String>,
    pub yanked: bool,
    pub published_at: DateTime<Utc>,
}

impl Release {
    /// Where this build's files are. The trailing segment is the build id, so two builds are
    /// two directories and neither can overwrite the other.
    pub fn base_url(&self) -> String {
        format!("{}/game/{}", self.cdn_base.trim_end_matches('/'), self.build_id)
    }
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct Channel {
    pub name: String,
    pub build_id: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewRelease {
    pub build_id: String,
    pub cdn_base: String,
    pub wasm_bytes: Option<i64>,
    pub notes: Option<String>,
}

/// Registers a build, or updates what is known about one already registered.
///
/// Does **not** touch `channels`. Publishing and promoting are different decisions, and a
/// deploy pipeline that conflates them will eventually promote something nobody looked at.
pub async fn register(pool: &PgPool, new: &NewRelease) -> sqlx::Result<Release> {
    sqlx::query_as::<_, Release>(
        "insert into releases (build_id, cdn_base, wasm_bytes, notes)
         values ($1, $2, $3, $4)
         on conflict (build_id) do update
             set cdn_base = excluded.cdn_base,
                 wasm_bytes = excluded.wasm_bytes,
                 notes = coalesce(excluded.notes, releases.notes)
         returning *",
    )
    .bind(&new.build_id)
    .bind(new.cdn_base.trim_end_matches('/'))
    .bind(new.wasm_bytes)
    .bind(&new.notes)
    .fetch_one(pool)
    .await
}

/// Points a channel at a build.
///
/// The foreign key refuses a build that was never registered, and the explicit check refuses
/// a yanked one — so the two ways of promoting something nobody should be running are both
/// closed here rather than in whatever called this.
pub async fn promote(
    pool: &PgPool,
    channel: &str,
    build_id: &str,
) -> Result<Channel, PromoteError> {
    let release = get(pool, build_id).await?.ok_or(PromoteError::Unknown)?;
    if release.yanked {
        return Err(PromoteError::Yanked);
    }
    sqlx::query_as::<_, Channel>(
        "insert into channels (name, build_id) values ($1, $2)
         on conflict (name) do update set build_id = excluded.build_id, updated_at = now()
         returning *",
    )
    .bind(channel)
    .bind(build_id)
    .fetch_one(pool)
    .await
    .map_err(PromoteError::Db)
}

#[derive(Debug)]
pub enum PromoteError {
    Unknown,
    Yanked,
    Db(sqlx::Error),
}

impl std::fmt::Display for PromoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown => write!(f, "no such build; register it first"),
            Self::Yanked => write!(f, "that build is yanked"),
            Self::Db(e) => write!(f, "{e}"),
        }
    }
}

impl From<sqlx::Error> for PromoteError {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e)
    }
}

pub async fn set_yanked(
    pool: &PgPool,
    build_id: &str,
    yanked: bool,
) -> sqlx::Result<Option<Release>> {
    sqlx::query_as::<_, Release>("update releases set yanked = $2 where build_id = $1 returning *")
        .bind(build_id)
        .bind(yanked)
        .fetch_optional(pool)
        .await
}

pub async fn get(pool: &PgPool, build_id: &str) -> sqlx::Result<Option<Release>> {
    sqlx::query_as::<_, Release>("select * from releases where build_id = $1")
        .bind(build_id)
        .fetch_optional(pool)
        .await
}

pub async fn list(pool: &PgPool) -> sqlx::Result<Vec<Release>> {
    sqlx::query_as::<_, Release>("select * from releases order by published_at desc limit 50")
        .fetch_all(pool)
        .await
}

pub async fn channels(pool: &PgPool) -> sqlx::Result<Vec<Channel>> {
    sqlx::query_as::<_, Channel>("select * from channels order by name").fetch_all(pool).await
}

/// The release a channel points at, if the channel exists and its build is not yanked.
pub async fn for_channel(pool: &PgPool, channel: &str) -> sqlx::Result<Option<Release>> {
    sqlx::query_as::<_, Release>(
        "select r.* from channels c join releases r on r.build_id = c.build_id
         where c.name = $1 and not r.yanked",
    )
    .bind(channel)
    .fetch_optional(pool)
    .await
}
