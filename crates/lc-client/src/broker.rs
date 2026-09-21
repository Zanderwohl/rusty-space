//! Talking to the identity broker.
//!
//! Two calls and nothing else: trade a sign-in code for a device grant, and trade a grant for a
//! game ticket. Blocking, because they happen on a Bevy IO task and not in a frame.
//!
//! The client never learns anything about an account beyond an opaque id and a name — that is
//! the whole of what crosses, and `lightcone/docs/16-identity.md` says why.

use crate::auth::{BrokerError, Identity};

/// Where the broker is, and which game server tickets are wanted for.
#[derive(Clone, Debug)]
pub struct Broker {
    pub base: String,
    pub audience: String,
}

/// What a sign-in leaves the client holding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Granted {
    pub grant: String,
    pub identity: Identity,
}

impl Broker {
    pub fn new(base: &str, audience: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_owned(),
            audience: audience.to_owned(),
        }
    }

    /// Trade a sign-in code for a device grant.
    ///
    /// `label` is what a revocation list shows a person later, so it is the machine's name and
    /// not an identifier.
    pub fn redeem(&self, code: &str, return_to: &str, label: &str) -> Result<Granted, BrokerError> {
        let body = ureq::json!({ "code": code, "return_to": return_to, "label": label });
        let answer = self.post("/grant", body)?;
        Ok(Granted {
            grant: field(&answer, "grant")?,
            identity: Identity {
                account_id: field(&answer, "account_id")?,
                display_name: field(&answer, "display_name")?,
            },
        })
    }

    /// Sign in with the local password provider, without a browser.
    ///
    /// Straight to a grant: a code exists to survive a browser redirect and there is not one
    /// here. `NoPasswordProvider` is distinct from a refusal so the client can stop offering a
    /// form that can never work on this server.
    pub fn with_password(
        &self,
        email: &str,
        password: &str,
        label: &str,
        register_as: Option<&str>,
    ) -> Result<Granted, BrokerError> {
        let path = if register_as.is_some() {
            "/signin/register/native"
        } else {
            "/signin/password/native"
        };
        let body = ureq::json!({
            "email": email,
            "password": password,
            "label": label,
            "display_name": register_as,
        });
        let answer = self.post(path, body)?;
        Ok(Granted {
            grant: field(&answer, "grant")?,
            identity: Identity {
                account_id: field(&answer, "account_id")?,
                display_name: field(&answer, "display_name")?,
            },
        })
    }

    /// Trade a device grant for a game ticket.
    ///
    /// Done on every connection rather than once, which is what lets a ticket be worth sixty
    /// seconds without anyone signing in again.
    pub fn ticket(&self, grant: &str) -> Result<String, BrokerError> {
        let body = ureq::json!({ "grant": grant, "audience": self.audience });
        field(&self.post("/ticket", body)?, "ticket")
    }

    fn post(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, BrokerError> {
        match ureq::post(&format!("{}{path}", self.base)).send_json(body) {
            Ok(response) => response
                .into_json()
                .map_err(|why| BrokerError::Unreachable(why.to_string())),
            // 401 and 404 both mean "this credential is no good", and both are recoverable by
            // signing in again. Everything else is a fault, not a refusal, and must *not*
            // clear the vault: a broker that is down would otherwise sign everyone out.
            // A path that does not exist means this server has no such provider, which is a
            // thing the client should stop offering rather than retry.
            Err(ureq::Error::Status(404, _)) if path.ends_with("/native") => {
                Err(BrokerError::NoPasswordProvider)
            }
            Err(ureq::Error::Status(401 | 404, _)) => Err(BrokerError::Refused),
            Err(ureq::Error::Status(409, _)) => Err(BrokerError::AlreadyRegistered),
            Err(ureq::Error::Status(422, _)) => Err(BrokerError::WeakPassword),
            Err(ureq::Error::Status(429, _)) => Err(BrokerError::TooManyAttempts),
            Err(why) => Err(BrokerError::Unreachable(why.to_string())),
        }
    }
}

/// Who a ticket says it is for.
///
/// The claims are read **without verifying the signature**, and that is not a shortcut. The
/// ticket arrived over TLS from the broker in answer to a request this process made, and the
/// client is not deciding anything with it — the *game server* verifies it, and a forged name
/// here would only mislead a player about their own account. Reading it is what lets a
/// returning player see their name instead of the word "signed in".
pub fn identity_in(ticket: &str) -> Option<Identity> {
    use base64::Engine;
    let payload = ticket.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    Some(Identity {
        account_id: claims.get("sub")?.as_str()?.to_owned(),
        display_name: claims
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_owned(),
    })
}

fn field(value: &serde_json::Value, name: &str) -> Result<String, BrokerError> {
    value
        .get(name)
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| BrokerError::Unreachable(format!("the broker sent no {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_does_not_become_a_double_one() {
        assert_eq!(
            Broker::new("https://accounts.example/", "shard-1").base,
            "https://accounts.example"
        );
        assert_eq!(
            Broker::new("https://accounts.example", "shard-1").base,
            "https://accounts.example"
        );
    }

    #[test]
    fn a_missing_field_is_a_fault_and_not_a_refusal() {
        let answer = ureq::json!({ "grant": "g" });
        assert!(matches!(field(&answer, "grant"), Ok(value) if value == "g"));
        assert!(matches!(
            field(&answer, "account_id"),
            Err(BrokerError::Unreachable(_))
        ));
    }

    /// The distinction the whole error type exists for. A refused credential should sign the
    /// player out; an unreachable broker must not, or an outage logs everyone out at once and
    /// none of them can sign back in.
    #[test]
    fn only_a_refusal_is_a_refusal() {
        assert!(BrokerError::Refused.to_string().contains("no longer valid"));
        let fault = BrokerError::Unreachable("connection refused".into());
        assert!(fault.to_string().contains("could not reach"));
        assert!(!matches!(fault, BrokerError::Refused));
    }
    /// A returning player should see their own name, not the word "signed in".
    #[test]
    fn a_tickets_claims_say_who_it_is_for() {
        use base64::Engine;
        let b64 = |v: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        let claims = r#"{"sub":"acct-7","name":"Ada Lovelace","aud":"shard-1","exp":9}"#;
        let ticket = format!("{}.{}.{}", b64("{}"), b64(claims), "not-checked");

        let who = identity_in(&ticket).expect("claims");
        assert_eq!(who.account_id, "acct-7");
        assert_eq!(who.display_name, "Ada Lovelace");
    }

    /// Anything that is not a ticket reads as nobody, rather than as a panic in a frame.
    #[test]
    fn something_that_is_not_a_ticket_names_nobody() {
        for junk in ["", "a", "a.b", "a.b.c", "....", "a.!!!!.c"] {
            assert!(identity_in(junk).is_none(), "{junk:?} named somebody");
        }
        // Well-formed base64 of well-formed JSON that is not a ticket: still nobody, because
        // the one claim that matters is missing.
        use base64::Engine;
        let b64 = |v: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        let no_sub = format!("{}.{}.{}", b64("{}"), b64(r#"{"name":"Ada"}"#), "x");
        assert!(identity_in(&no_sub).is_none());
    }

    /// A ticket with no name is still an account. The server falls back to a placeholder
    /// rather than showing an empty label.
    #[test]
    fn a_nameless_ticket_still_names_an_account() {
        use base64::Engine;
        let b64 = |v: &str| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        let ticket = format!("{}.{}.{}", b64("{}"), b64(r#"{"sub":"acct-7"}"#), "x");
        let who = identity_in(&ticket).expect("claims");
        assert_eq!(who.account_id, "acct-7");
        assert!(who.display_name.is_empty());
    }
}
