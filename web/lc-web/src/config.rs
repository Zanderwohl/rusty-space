//! Environment only. No config file, and no secrets in the image.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Env {
    Dev,
    Staging,
    Production,
}

impl Env {
    /// Drafts are listed, assets are watched and caching is disabled only outside production.
    pub fn is_production(self) -> bool {
        self == Env::Production
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub env: Env,
    pub static_dir: PathBuf,
    pub content_dir: PathBuf,
    /// Absolute origin, for feeds and the sitemap. Those are the only places a URL has to be
    /// absolute; every link in a page is relative and needs no configuration.
    pub base_url: String,
    /// Where game builds are served from, without a trailing slash.
    pub cdn_base: String,
    /// Which build `/play` launches.
    ///
    /// Named for what it will be once there is a `channels` table: the answer when the
    /// database cannot be reached. Today there is no database, so it is the only answer, and
    /// that is the same code path rather than a temporary one.
    pub fallback_build_id: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let bind = var("BIND_ADDR").unwrap_or_else(|| "0.0.0.0:3100".into()).parse()?;
        let env = match var("SITE_ENV").as_deref() {
            Some("dev") | Some("development") => Env::Dev,
            Some("staging") => Env::Staging,
            // Defaulting to production is the safe direction: the failure is a missing dev
            // convenience, not drafts on the public internet.
            _ => Env::Production,
        };
        Ok(Config {
            bind,
            env,
            static_dir: var("STATIC_DIR").unwrap_or_else(|| "static".into()).into(),
            content_dir: var("CONTENT_DIR").unwrap_or_else(|| "content".into()).into(),
            base_url: var("BASE_URL")
                .unwrap_or_else(|| "https://lightcone.example".into())
                .trim_end_matches('/')
                .to_owned(),
            cdn_base: var("CDN_BASE")
                .unwrap_or_else(|| "https://cdn.lightcone.example".into())
                .trim_end_matches('/')
                .to_owned(),
            fallback_build_id: var("FALLBACK_BUILD_ID"),
        })
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}
