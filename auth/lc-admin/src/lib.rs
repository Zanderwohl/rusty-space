//! The Lightcone administration site: accounts, levels and bans.
//!
//! A second service over the broker's database, separate so an administration console shares
//! no origin, process or dependency tree with the sign-in every player reaches. See
//! `lightcone/docs/16-identity.md`.
//!
//! **It owns no schema.** `lc_identity::schema` does.

#![forbid(unsafe_code)]

pub mod assets;
pub mod auth;
pub mod catalog;
pub mod config;
pub mod detail;
pub mod listing;
pub mod routes;
pub mod session;
pub mod shard;
pub mod users;
pub mod views;

use std::sync::Arc;

use axum::extract::FromRef;

use crate::assets::Assets;

#[derive(Clone)]
pub struct AppState {
    pub assets: Assets,
    pub pool: sqlx::PgPool,
    pub session_key: Arc<str>,
    /// Where a **browser** is sent; `identity_api` is where this process calls. See
    /// [`config::Config`] for why they differ.
    pub identity_base: Arc<str>,
    pub identity_api: Arc<str>,
    pub identity_secret: Arc<str>,
    /// Must be on the broker's allowlist verbatim, or a sign-in fails at its last step.
    pub return_url: Arc<str>,
    pub secure_cookies: bool,
    /// `None` runs the console without a Status section rather than refusing to start.
    pub shard_api: Option<Arc<str>>,
    pub shard_audience: Arc<str>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn shard(&self) -> Option<crate::shard::Shard<'_>> {
        Some(crate::shard::Shard {
            api: self.shard_api.as_deref()?,
            audience: &self.shard_audience,
            identity_api: &self.identity_api,
            identity_secret: &self.identity_secret,
            http: &self.http,
        })
    }
}

impl AppState {
    /// Through `lc_identity`'s types rather than SQL written again here, so the sign-in path
    /// and this console cannot disagree about what a ban is.
    pub fn store(&self) -> lc_identity::store::Store {
        lc_identity::store::Store::Postgres(self.pool.clone())
    }
}

impl FromRef<AppState> for Assets {
    fn from_ref(state: &AppState) -> Assets {
        state.assets.clone()
    }
}

/// Timed out, or a broker that accepts a connection and never answers holds a sign-in open
/// until the browser gives up.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("a default https client")
}
