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
    /// Paths a **loopback** redirect may use, on any port. See [`is_allowed_return`].
    pub loopback_paths: Vec<String>,
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
            loopback_paths: var("LC_IDENTITY_LOOPBACK_PATHS")
                .unwrap_or_else(|| "/return".into())
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
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

/// The one exception: a **loopback** redirect, on any port.
///
/// A native client listens on a port the operating system gives it, so its redirect cannot be
/// known in advance and cannot be on an exact list. RFC 8252 §7.3 says exactly this — an
/// authorization server must not require a fixed port for loopback — and it is safe for the
/// reason the rest of the allowlist is not: `127.0.0.1` is not reachable from anywhere else, so
/// a redirect there cannot deliver a code to anyone but the person at the machine.
///
/// Everything *except* the port is still pinned, and this is the one place a URL is parsed
/// rather than compared. The conditions are deliberately all of: plain `http`, a literal
/// loopback address and not a name that resolves to one, an allowlisted path, no query, no
/// fragment, and no credentials.
pub fn is_allowed_loopback(paths: &[String], candidate: &str) -> bool {
    let Ok(url) = url::Url::parse(candidate) else { return false };
    let loopback = matches!(url.host_str(), Some("127.0.0.1") | Some("[::1]") | Some("::1"));
    url.scheme() == "http"
        && loopback
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && paths.iter().any(|p| p == url.path())
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

    /// The port is the *only* thing a loopback redirect may vary, because it is the only thing
    /// a native client cannot know in advance.
    #[test]
    fn a_loopback_redirect_may_use_any_port_and_nothing_else() {
        let paths = vec!["/return".to_string()];
        for port in ["1024", "49152", "65535"] {
            assert!(is_allowed_loopback(&paths, &format!("http://127.0.0.1:{port}/return")));
        }
        assert!(is_allowed_loopback(&paths, "http://[::1]:7635/return"));

        for hostile in [
            // Not loopback, however much it looks like it.
            "http://127.0.0.1.attacker.test:7635/return",
            "http://localhost:7635/return",
            "http://evil.test:7635/return",
            // A path nobody allowlisted.
            "http://127.0.0.1:7635/anything-else",
            "http://127.0.0.1:7635/return/../evil",
            // Carrying something of its own.
            "http://127.0.0.1:7635/return?next=x",
            "http://127.0.0.1:7635/return#x",
            "http://user:pass@127.0.0.1:7635/return",
            // Not plain loopback http.
            "https://127.0.0.1:7635/return",
            "file:///return",
            "",
        ] {
            assert!(!is_allowed_loopback(&paths, hostile), "{hostile} was allowed");
        }
    }

    /// `localhost` is a name, and a name is something someone else can be made to resolve. The
    /// literal address is the whole point of the exception.
    #[test]
    fn loopback_means_the_address_and_not_the_name() {
        let paths = vec!["/return".to_string()];
        assert!(is_allowed_loopback(&paths, "http://127.0.0.1:7635/return"));
        assert!(!is_allowed_loopback(&paths, "http://localhost:7635/return"));
    }

    #[test]
    fn an_empty_allowlist_allows_nothing() {
        assert!(!is_allowed_return(
            &[],
            "https://lightcone.example/auth/return"
        ));
    }
}
