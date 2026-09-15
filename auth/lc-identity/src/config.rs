//! Environment only. No config file, and no secrets in the image.

use std::net::SocketAddr;

use crate::providers::{Enabled, resolve};

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    pub database_url: String,
    /// Which providers this deployment accepts. Resolved at startup and never re-read: a
    /// provider appearing mid-process would be a provider nobody decided to enable.
    pub providers: Enabled,
    /// Exactly the `return_to` values a sign-in may redirect back to.
    ///
    /// An exact-match list, not prefixes. An open redirect on an auth service is how a sign-in
    /// gets stolen, and prefix matching is how open redirects happen.
    pub return_to: Vec<String>,
    /// Shared secret for `/exchange` and `/ticket`, which are server to server and never
    /// reachable from a browser.
    pub exchange_secret: String,
    /// The game servers this broker will mint tickets for.
    ///
    /// An allowlist for the same reason `return_to` is one: a caller that could name any
    /// audience could mint a ticket for a shard it has no business on, and "the audience is
    /// ours anyway" stops being true the first time it is not.
    pub audiences: Vec<String>,
    /// What the tickets say issued them.
    pub issuer: String,
    /// The Ed25519 seed, base64url. Absent generates one, which is a development convenience
    /// and is logged as such: a generated key means every restart publishes a different one.
    pub signing_seed: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let list = var("LC_IDENTITY_PROVIDERS").unwrap_or_default();
        let providers = resolve(&list, var)?;

        let return_to: Vec<String> = var("LC_IDENTITY_RETURN_TO")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if return_to.is_empty() {
            anyhow::bail!("LC_IDENTITY_RETURN_TO is empty; no sign-in could complete");
        }
        // Checked here rather than at redirect time: an entry carrying its own query would
        // make the join ambiguous, and an ambiguous redirect is one someone can steer.
        if let Some(bad) = return_to
            .iter()
            .find(|url| !crate::signin::is_appendable(url))
        {
            anyhow::bail!("LC_IDENTITY_RETURN_TO entry {bad:?} must carry no query or fragment");
        }

        let audiences: Vec<String> = var("LC_IDENTITY_AUDIENCES")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        if audiences.is_empty() {
            anyhow::bail!("LC_IDENTITY_AUDIENCES is empty; no ticket could be minted");
        }

        Ok(Config {
            bind: var("BIND_ADDR")
                .unwrap_or_else(|| "0.0.0.0:3200".into())
                .parse()?,
            // Required, unlike the site's. A broker with no database cannot serve a degraded
            // version of itself; it can only let the wrong people in or nobody.
            database_url: var("DATABASE_URL")
                .ok_or_else(|| anyhow::anyhow!("DATABASE_URL is required"))?,
            providers,
            return_to,
            exchange_secret: var("LC_IDENTITY_EXCHANGE_SECRET")
                .ok_or_else(|| anyhow::anyhow!("LC_IDENTITY_EXCHANGE_SECRET is required"))?,
            audiences,
            issuer: var("LC_IDENTITY_ISSUER")
                .unwrap_or_else(|| "https://accounts.lightcone.example".into()),
            signing_seed: var("LC_IDENTITY_SIGNING_SEED"),
        })
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Whether `candidate` is somewhere a sign-in may return to.
///
/// Exact string equality against the allowlist. Not `starts_with`, which
/// `https://lightcone.example.attacker.test` satisfies for
/// `https://lightcone.example`; not a parsed-origin comparison, which is only as good as the
/// parser and has been the source of a long line of bypasses.
pub fn is_allowed_return(allowed: &[String], candidate: &str) -> bool {
    allowed.iter().any(|a| a == candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_return_url_must_match_exactly() {
        let allowed = vec!["https://lightcone.example/auth/return".to_string()];
        assert!(is_allowed_return(
            &allowed,
            "https://lightcone.example/auth/return"
        ));

        for hostile in [
            // The prefix attack a `starts_with` check would pass.
            "https://lightcone.example.attacker.test/auth/return",
            "https://lightcone.example/auth/return/../../evil",
            "https://lightcone.example/auth/return?next=//evil",
            "http://lightcone.example/auth/return",
            "https://lightcone.example/auth/return#x",
            "",
        ] {
            assert!(
                !is_allowed_return(&allowed, hostile),
                "{hostile} was allowed"
            );
        }
    }

    #[test]
    fn an_empty_allowlist_allows_nothing() {
        assert!(!is_allowed_return(
            &[],
            "https://lightcone.example/auth/return"
        ));
    }
}
