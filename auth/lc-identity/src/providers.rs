//! Which ways of signing in exist, and whether each is configured to.
//!
//! One list rather than a flag per provider. `GOOGLE_ACCOUNTS=ture` is silently off, and an
//! auth service silently missing a provider locks out everyone who used it; a misspelled name
//! in a list fails to start. See `lightcone/docs/16-identity.md`.

use std::collections::BTreeMap;
use std::fmt;

/// A way of proving who you are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Provider {
    /// Local, opt-in, and off unless named. It exists so a development environment can make a
    /// hundred accounts without talking to anyone.
    Password,
    Google,
    Discord,
}

impl Provider {
    pub const ALL: [Provider; 3] = [Provider::Password, Provider::Google, Provider::Discord];

    /// The name in the configuration list, and the `provider` column.
    pub fn name(self) -> &'static str {
        match self {
            Provider::Password => "password",
            Provider::Google => "google",
            Provider::Discord => "discord",
        }
    }

    /// What a person is offered on a button.
    pub fn label(self) -> &'static str {
        match self {
            Provider::Password => "a password",
            Provider::Google => "Google",
            Provider::Discord => "Discord",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Whether it needs an upstream client id and secret.
    ///
    /// The password provider does not, which is exactly why it has to be named explicitly:
    /// there is no configuration whose presence could imply it.
    pub fn is_upstream(self) -> bool {
        !matches!(self, Provider::Password)
    }

    /// The environment prefix its credentials live under.
    pub fn env_prefix(self) -> String {
        self.name().to_uppercase()
    }

    /// Where its OAuth2 dance happens, for the providers that have one written.
    ///
    /// `None` means upstream but unbuilt, which [`resolve`] treats as a failure to start. A
    /// provider that appears on the sign-in page and cannot complete is worse than one that
    /// is absent, and the absent one is at least noticed.
    pub fn endpoints(self) -> Option<Endpoints> {
        match self {
            Provider::Google => Some(Endpoints {
                // Hardcoded rather than read from the discovery document. Discovery is a
                // network dependency at boot for three strings that have not changed in a
                // decade, and a boot that needs Google up is a boot that fails when it is not.
                authorize: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                token: "https://oauth2.googleapis.com/token".into(),
                // Google issues under both spellings and has always done so.
                issuers: vec![
                    "https://accounts.google.com".into(),
                    "accounts.google.com".into(),
                ],
                scope: "openid email profile".into(),
            }),
            Provider::Password | Provider::Discord => None,
        }
    }
}

/// The three URLs and one scope string an OAuth2 dance needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoints {
    pub authorize: String,
    pub token: String,
    /// What the `iss` claim must say. A list because a provider may spell itself more than one
    /// way, and Google does.
    pub issuers: Vec<String>,
    pub scope: String,
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// An upstream provider's client credentials.
#[derive(Clone, PartialEq, Eq)]
pub struct Upstream {
    pub client_id: String,
    pub client_secret: String,
    pub endpoints: Endpoints,
}

impl fmt::Debug for Upstream {
    /// The secret is not printed. A config dump in a log is the most ordinary way a client
    /// secret escapes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Upstream")
            .field("client_id", &self.client_id)
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

/// The providers this deployment will accept, and their configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Enabled {
    upstream: BTreeMap<Provider, Upstream>,
    password: bool,
}

impl Enabled {
    pub fn allows(&self, provider: Provider) -> bool {
        match provider {
            Provider::Password => self.password,
            other => self.upstream.contains_key(&other),
        }
    }

    pub fn upstream(&self, provider: Provider) -> Option<&Upstream> {
        self.upstream.get(&provider)
    }

    /// Every live provider, in a stable order, for the sign-in page and the startup banner.
    pub fn live(&self) -> Vec<Provider> {
        Provider::ALL
            .into_iter()
            .filter(|p| self.allows(*p))
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        !self.password && self.upstream.is_empty()
    }
}

/// Why a provider list could not be honoured.
///
/// Every one of these stops the process. An auth service that starts with a provider quietly
/// missing is worse than one that does not start.
#[derive(Debug, PartialEq, Eq)]
pub enum Misconfigured {
    /// A name in the list that is not a provider. Almost always a typo, and the reason the
    /// list is one variable rather than a flag each.
    Unknown(String),
    /// Named, but its client id or secret is absent.
    Unconfigured(Provider),
    /// Named, credentials and all, but nobody has written its dance yet.
    Unimplemented(Provider),
    /// No providers at all. Nobody could sign in, which is not a state worth serving.
    Nothing,
}

impl fmt::Display for Misconfigured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Misconfigured::Unknown(name) => write!(
                f,
                "LC_IDENTITY_PROVIDERS names {name:?}, which is not a provider. Known: {}",
                Provider::ALL.map(|p| p.name()).join(", "),
            ),
            Misconfigured::Unconfigured(provider) => {
                let prefix = provider.env_prefix();
                write!(
                    f,
                    "{provider} is enabled but {prefix}_CLIENT_ID or {prefix}_CLIENT_SECRET is missing"
                )
            }
            Misconfigured::Unimplemented(provider) => write!(
                f,
                "{provider} is enabled but signing in with it is not implemented"
            ),
            Misconfigured::Nothing => {
                f.write_str("LC_IDENTITY_PROVIDERS is empty; nobody could sign in")
            }
        }
    }
}

impl std::error::Error for Misconfigured {}

/// Read the list, and refuse anything it cannot honour.
///
/// `lookup` reads one environment variable, so a test can supply a world without touching the
/// process environment — which is shared, and which two tests running at once would fight over.
pub fn resolve(
    list: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Enabled, Misconfigured> {
    let mut enabled = Enabled::default();
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        let provider = Provider::parse(name).ok_or_else(|| Misconfigured::Unknown(name.into()))?;
        if !provider.is_upstream() {
            enabled.password = true;
            continue;
        }
        let mut endpoints = provider
            .endpoints()
            .ok_or(Misconfigured::Unimplemented(provider))?;
        let prefix = provider.env_prefix();
        // Overridable so a test can point the dance at a stub on loopback and exercise the
        // same code path a deployment does, configuration reading included. Anyone able to set
        // these already owns the process.
        if let Some(url) = lookup(&format!("{prefix}_AUTH_URL")) {
            endpoints.authorize = url;
        }
        if let Some(url) = lookup(&format!("{prefix}_TOKEN_URL")) {
            endpoints.token = url;
        }
        if let Some(issuer) = lookup(&format!("{prefix}_ISSUER")) {
            endpoints.issuers = vec![issuer];
        }
        let client_id = lookup(&format!("{prefix}_CLIENT_ID"));
        let client_secret = lookup(&format!("{prefix}_CLIENT_SECRET"));
        match (client_id, client_secret) {
            (Some(client_id), Some(client_secret)) => {
                enabled.upstream.insert(
                    provider,
                    Upstream {
                        client_id,
                        client_secret,
                        endpoints,
                    },
                );
            }
            _ => return Err(Misconfigured::Unconfigured(provider)),
        }
    }
    if enabled.is_empty() {
        return Err(Misconfigured::Nothing);
    }
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configured(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name: &str| map.get(name).cloned()
    }

    #[test]
    fn a_password_provider_needs_nothing_but_naming() {
        let enabled = resolve("password", configured(&[])).expect("no configuration needed");
        assert!(enabled.allows(Provider::Password));
        assert!(!enabled.allows(Provider::Google));
        assert_eq!(enabled.live(), vec![Provider::Password]);
    }

    /// Off unless named. Shipping without having thought about it leaves it off, which is the
    /// direction the failure should point.
    #[test]
    fn a_password_provider_is_off_unless_named() {
        let enabled = resolve(
            "google",
            configured(&[("GOOGLE_CLIENT_ID", "a"), ("GOOGLE_CLIENT_SECRET", "b")]),
        )
        .expect("google alone is a valid deployment");
        assert!(!enabled.allows(Provider::Password));
    }

    /// The whole reason the list is one variable. A flag per provider cannot notice this.
    #[test]
    fn a_misspelled_provider_refuses_to_start() {
        assert_eq!(
            resolve("password,googel", configured(&[])),
            Err(Misconfigured::Unknown("googel".into())),
        );
        // And the message says what the known ones are, because the next thing anyone does is
        // go looking for the spelling.
        let said = Misconfigured::Unknown("googel".into()).to_string();
        assert!(said.contains("googel") && said.contains("google"), "{said}");
    }

    /// Enabled and configured are the same fact for anything upstream. A provider in the list
    /// with no credentials would be a button that always fails.
    #[test]
    fn an_upstream_provider_without_credentials_refuses_to_start() {
        assert_eq!(
            resolve("google", configured(&[("GOOGLE_CLIENT_ID", "a")])),
            Err(Misconfigured::Unconfigured(Provider::Google)),
        );
        assert_eq!(
            resolve("google", configured(&[("GOOGLE_CLIENT_SECRET", "b")])),
            Err(Misconfigured::Unconfigured(Provider::Google)),
        );
    }

    /// Configured but not listed is *not* an error: it is how credentials get staged before a
    /// provider goes live.
    #[test]
    fn configured_but_unlisted_is_simply_off() {
        let enabled = resolve(
            "password",
            configured(&[("GOOGLE_CLIENT_ID", "a"), ("GOOGLE_CLIENT_SECRET", "b")]),
        )
        .expect("staging credentials is not an error");
        assert!(!enabled.allows(Provider::Google));
    }

    #[test]
    fn a_list_with_nothing_in_it_refuses_to_start() {
        assert_eq!(resolve("", configured(&[])), Err(Misconfigured::Nothing));
        assert_eq!(
            resolve("  , ,", configured(&[])),
            Err(Misconfigured::Nothing)
        );
    }

    #[test]
    fn whitespace_and_order_do_not_matter() {
        let creds = [("GOOGLE_CLIENT_ID", "a"), ("GOOGLE_CLIENT_SECRET", "b")];
        let one = resolve(" google , password ", configured(&creds)).unwrap();
        let two = resolve("password,google", configured(&creds)).unwrap();
        assert_eq!(one.live(), two.live());
        assert_eq!(one.live(), vec![Provider::Password, Provider::Google]);
    }

    /// Credentials are not the only thing a provider needs. Discord has a client id shape and
    /// no dance written, and the failure for that belongs at boot rather than on a button.
    #[test]
    fn a_provider_with_no_dance_written_refuses_to_start() {
        assert_eq!(
            resolve(
                "discord",
                configured(&[("DISCORD_CLIENT_ID", "c"), ("DISCORD_CLIENT_SECRET", "d")]),
            ),
            Err(Misconfigured::Unimplemented(Provider::Discord)),
        );
    }

    /// The hook the end-to-end test hangs on: the dance can be pointed somewhere else without
    /// a second code path for tests.
    #[test]
    fn endpoints_can_be_pointed_elsewhere() {
        let enabled = resolve(
            "google",
            configured(&[
                ("GOOGLE_CLIENT_ID", "a"),
                ("GOOGLE_CLIENT_SECRET", "b"),
                ("GOOGLE_TOKEN_URL", "http://127.0.0.1:9/token"),
            ]),
        )
        .unwrap();
        let upstream = enabled.upstream(Provider::Google).unwrap();
        assert_eq!(upstream.endpoints.token, "http://127.0.0.1:9/token");
        // Only what was overridden.
        assert!(
            upstream
                .endpoints
                .authorize
                .starts_with("https://accounts.google.com/")
        );
    }

    /// A secret that reaches a log is a secret. The config dump is the ordinary way out.
    #[test]
    fn a_client_secret_is_not_printed() {
        let enabled = resolve(
            "google",
            configured(&[
                ("GOOGLE_CLIENT_ID", "id-x"),
                ("GOOGLE_CLIENT_SECRET", "hunter2"),
            ]),
        )
        .unwrap();
        let dumped = format!("{enabled:?}");
        assert!(
            dumped.contains("id-x"),
            "the client id is not a secret: {dumped}"
        );
        assert!(!dumped.contains("hunter2"), "the secret escaped: {dumped}");
    }
}
