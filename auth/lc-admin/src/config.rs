//! Environment only, like the broker's. No config file, and no secrets in the image.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// **The broker's database.** This service reads and writes `accounts`, `bans` and
    /// `admin_actions` in the same database `lc-identity` owns, and owns no schema of its own:
    /// every migration is the broker's and is applied at the broker's boot. Two services
    /// migrating one database is two advisory locks and one race nobody wants to debug.
    pub database_url: String,
    /// Where a **browser** is sent to sign in.
    pub identity_base: String,
    /// Where **this process** calls the broker. Usually the same; separable for the reason
    /// `lc_web::auth::Identity::api` gives — a container reaching the host's published port by
    /// its public name hairpins through the NAT and hangs.
    pub identity_api: String,
    /// The broker's `LC_IDENTITY_EXCHANGE_SECRET`. What lets this service turn a sign-in code
    /// into an account id.
    pub identity_secret: String,
    /// Where this service is reachable from a browser, with no trailing slash. The return URL
    /// is built from it, and it must be on the broker's `LC_IDENTITY_RETURN_TO` allowlist.
    pub public_url: String,
    /// What the session cookie is signed with.
    pub session_key: String,
    pub static_dir: PathBuf,
    /// Whether cookies are marked `Secure`. Off only where the site is reached over plain
    /// HTTP, which is development and nowhere else.
    pub secure_cookies: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let public_url = required("LC_ADMIN_PUBLIC_URL")?
            .trim_end_matches('/')
            .to_owned();
        let identity_base = required("LC_ADMIN_IDENTITY_BASE")?
            .trim_end_matches('/')
            .to_owned();
        let session_key = required("LC_ADMIN_SESSION_KEY")?;
        // A short key is a forgeable cookie, and a cookie forged here is an administrator
        // account. Refused at boot rather than warned about: there is no degraded mode of this
        // service worth running.
        anyhow::ensure!(
            session_key.len() >= 32,
            "LC_ADMIN_SESSION_KEY must be at least 32 characters",
        );

        Ok(Config {
            bind: var("BIND_ADDR")
                .unwrap_or_else(|| "0.0.0.0:3300".into())
                .parse()?,
            database_url: required("DATABASE_URL")?,
            identity_api: var("LC_ADMIN_IDENTITY_API")
                .map(|url| url.trim_end_matches('/').to_owned())
                .unwrap_or_else(|| identity_base.clone()),
            identity_base,
            identity_secret: required("LC_ADMIN_IDENTITY_SECRET")?,
            public_url,
            session_key,
            static_dir: var("LC_ADMIN_STATIC_DIR")
                .unwrap_or_else(|| "static".into())
                .into(),
            // Opt *out*, not in. A deployment that forgets to say gets the safe answer, and
            // the one place that has to say is a developer's own machine.
            secure_cookies: var("LC_ADMIN_INSECURE_COOKIES").is_none(),
        })
    }

    /// Where the broker sends a browser back to. Must be on the broker's allowlist verbatim.
    pub fn return_url(&self) -> String {
        format!("{}{}", self.public_url, crate::routes::RETURN)
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn required(name: &str) -> anyhow::Result<String> {
    var(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}
