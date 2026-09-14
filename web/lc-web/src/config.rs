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
    #[cfg_attr(not(feature = "watch"), allow(dead_code))]
    /// Drafts are listed and assets are watched only outside production.
    pub fn is_production(self) -> bool {
        self == Env::Production
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub env: Env,
    pub static_dir: PathBuf,
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
        })
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}
