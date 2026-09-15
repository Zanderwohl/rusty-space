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
        Self { base: base.trim_end_matches('/').to_owned(), audience: audience.to_owned() }
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
            Err(ureq::Error::Status(401 | 404, _)) => Err(BrokerError::Refused),
            Err(why) => Err(BrokerError::Unreachable(why.to_string())),
        }
    }
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
        assert_eq!(Broker::new("https://accounts.example/", "shard-1").base, "https://accounts.example");
        assert_eq!(Broker::new("https://accounts.example", "shard-1").base, "https://accounts.example");
    }

    #[test]
    fn a_missing_field_is_a_fault_and_not_a_refusal() {
        let answer = ureq::json!({ "grant": "g" });
        assert!(matches!(field(&answer, "grant"), Ok(value) if value == "g"));
        assert!(matches!(field(&answer, "account_id"), Err(BrokerError::Unreachable(_))));
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
}
