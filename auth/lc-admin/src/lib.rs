//! The Lightcone administration site: accounts, levels and bans.
//!
//! A second service over the identity broker's database. It exists separately rather than as
//! `/admin` on the broker for one reason worth stating: the broker is the service every player
//! reaches to sign in, and an administration console is not something that should share an
//! origin, a process or a dependency tree with it. `lightcone/docs/16-identity.md` has the
//! rest of the argument.
//!
//! **This service owns no schema.** `lc-identity` holds every migration and applies them at
//! its own boot; this one reads and writes tables it did not create. Two services migrating
//! one database is two advisory locks and a race nobody wants to debug at four in the morning.
//!
//! A library with a thin binary over it, so the parts worth testing are reachable without a
//! socket.

#![forbid(unsafe_code)]

pub mod assets;
pub mod auth;
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

/// Everything a handler can reach. Built at startup, so a request is a query and a render.
#[derive(Clone)]
pub struct AppState {
    pub assets: Assets,
    /// The broker's database. See the module note: read and written, never migrated.
    pub pool: sqlx::PgPool,
    pub session_key: Arc<str>,
    /// Where a browser is sent to sign in.
    pub identity_base: Arc<str>,
    /// Where this process calls the broker.
    pub identity_api: Arc<str>,
    pub identity_secret: Arc<str>,
    /// This service's own return URL, which must be on the broker's allowlist verbatim.
    pub return_url: Arc<str>,
    pub secure_cookies: bool,
    /// Where the shard answers about ships. `None` runs the console without a Status section.
    pub shard_api: Option<Arc<str>>,
    pub shard_audience: Arc<str>,
    pub http: reqwest::Client,
}

impl AppState {
    /// What it takes to ask the shard, when there is one to ask.
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
    /// The broker's own store, over this service's pool.
    ///
    /// Accounts, bans and the action log are read and written through `lc_identity`'s types
    /// rather than through SQL written again here, so there is one definition of what a ban is
    /// and one place the sign-in path and this console can disagree about it: nowhere.
    pub fn store(&self) -> lc_identity::store::Store {
        lc_identity::store::Store::Postgres(self.pool.clone())
    }
}

impl FromRef<AppState> for Assets {
    fn from_ref(state: &AppState) -> Assets {
        state.assets.clone()
    }
}

/// The HTTP client used for the one server-to-server call this service makes.
///
/// A timeout, because a broker that accepts a connection and never answers would otherwise
/// hold a sign-in open until the browser gives up.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .expect("a default https client")
}
