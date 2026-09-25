//! Environment only, like the broker's. No config file, and no secrets in the image.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// **The broker's.** Read and written, never migrated — see `lc_identity::schema`.
    pub database_url: String,
    /// Where a **browser** is sent to sign in.
    pub identity_base: String,
    /// Where **this process** calls. Separable because a container reaching the host's
    /// published port by its public name hairpins through the NAT and hangs.
    pub identity_api: String,
    /// The broker's `LC_IDENTITY_EXCHANGE_SECRET`.
    pub identity_secret: String,
    /// Builds the return URL, which must be on the broker's `LC_IDENTITY_RETURN_TO` verbatim.
    pub public_url: String,
    /// What the session cookie is signed with.
    pub session_key: String,
    /// The container name, never the public one. Absent runs without a Status section rather
    /// than refusing to start.
    pub shard_api: Option<String>,
    /// Must also be on the broker's `LC_IDENTITY_AUDIENCES`, or no ticket can be minted.
    pub shard_audience: String,
    /// The CDN's internal listing port, and its password. Both, or no CDN page.
    pub cdn_list: Option<(String, String)>,
    /// The site's internal address and its **read** token, which cannot promote or yank.
    pub site: Option<(String, String)>,
    /// The shelf's `books.toml`, mounted read-only.
    pub book_catalog: Option<PathBuf>,
    pub static_dir: PathBuf,
    /// Off only where the console is reached over plain HTTP.
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
        // A forgeable cookie here is an administrator account.
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
            shard_api: var("LC_ADMIN_SHARD_API").map(|u| u.trim_end_matches('/').to_owned()),
            shard_audience: var("LC_ADMIN_SHARD_AUDIENCE").unwrap_or_else(|| "shard-1".into()),
            cdn_list: both("LC_ADMIN_CDN_LIST", "LC_ADMIN_CDN_LIST_PASSWORD"),
            site: both("LC_ADMIN_SITE_API", "LC_ADMIN_RELEASE_READ_TOKEN"),
            book_catalog: var("LC_ADMIN_BOOK_CATALOG").map(Into::into),
            static_dir: var("LC_ADMIN_STATIC_DIR")
                .unwrap_or_else(|| "static".into())
                .into(),
            // Opt *out*: a deployment that forgets gets the safe answer.
            secure_cookies: var("LC_ADMIN_INSECURE_COOKIES").is_none(),
        })
    }

    pub fn return_url(&self) -> String {
        format!("{}{}", self.public_url, crate::routes::RETURN)
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// An address and its secret. Either alone is treated as neither, since neither works alone.
fn both(address: &str, secret: &str) -> Option<(String, String)> {
    let address = var(address)?.trim_end_matches('/').to_owned();
    Some((address, var(secret)?))
}

fn required(name: &str) -> anyhow::Result<String> {
    var(name).ok_or_else(|| anyhow::anyhow!("{name} is required"))
}
