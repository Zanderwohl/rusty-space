//! Proving who is on the other end of a socket.
//!
//! A ticket is a short-lived signed assertion from the identity broker, audience-scoped to this
//! server. It is verified **here**, against a key fetched from the broker's published set and
//! cached — never by asking the broker per connection, which would make an auth outage stop
//! existing players from reconnecting.
//!
//! The claim set is the contract in `lightcone/docs/16-identity.md`. The broker's half of it is
//! its own crate in its own workspace, and deliberately not shared: this crate must not depend
//! on the broker. They agree by the document and by a test on each side.

use std::collections::HashMap;

use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;

/// What this server accepts a ticket as saying.
///
/// A subset of what the broker mints is fine — serde ignores what it is not asked for — but
/// every field here has to be there, and `Validation` below states which are required rather
/// than trusting the shape.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Claims {
    /// The opaque account id. The only identifier that crosses a product boundary.
    pub sub: String,
    pub name: String,
    pub exp: i64,
    /// Recorded until it expires, so a ticket is worth one connection.
    pub jti: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// Signature, audience, expiry, or an unknown key. **One variant on purpose** — telling a
    /// client which is telling it how close it got.
    NotYou,
    /// A ticket that already opened a socket.
    Spent,
}

/// The keys this server will accept a ticket under, by `kid`.
///
/// A set rather than a key, so a broker rotating its signing key is two published keys and a
/// retirement rather than a flag day where every connection fails at once.
#[derive(Clone, Default)]
pub struct Trusted {
    keys: HashMap<String, Vec<u8>>,
    audience: String,
}

impl Trusted {
    pub fn new(audience: &str) -> Self {
        Self { keys: HashMap::new(), audience: audience.to_owned() }
    }

    /// Learn a key from a JWKS document.
    ///
    /// Only Ed25519 signing keys are taken. A key set that has grown an algorithm this server
    /// was not written for is a key set to ignore that entry of, not to guess at: algorithm
    /// confusion is the JWT vulnerability, and the way it happens is a verifier being
    /// accommodating.
    pub fn learn(&mut self, jwks: &serde_json::Value) -> usize {
        let Some(keys) = jwks.get("keys").and_then(|k| k.as_array()) else { return 0 };
        let mut learned = 0;
        for key in keys {
            let (Some(kid), Some(x)) = (
                key.get("kid").and_then(|v| v.as_str()),
                key.get("x").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            let ed25519 = key.get("kty").and_then(|v| v.as_str()) == Some("OKP")
                && key.get("crv").and_then(|v| v.as_str()) == Some("Ed25519");
            if !ed25519 {
                continue;
            }
            self.keys.insert(kid.to_owned(), x.as_bytes().to_vec());
            learned += 1;
        }
        learned
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Check a ticket, without spending it.
    ///
    /// The algorithm is **stated**, never read from the token's own header. A verifier that
    /// takes the header's word for which algorithm to use is the algorithm-confusion bug, and
    /// it is the one that actually gets exploited.
    pub fn check(&self, token: &str) -> Result<Claims, Rejected> {
        let header = jsonwebtoken::decode_header(token).map_err(|_| Rejected::NotYou)?;
        let kid = header.kid.ok_or(Rejected::NotYou)?;
        let x = self.keys.get(&kid).ok_or(Rejected::NotYou)?;
        let x = std::str::from_utf8(x).map_err(|_| Rejected::NotYou)?;
        let key = DecodingKey::from_ed_components(x).map_err(|_| Rejected::NotYou)?;

        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[&self.audience]);
        validation.set_required_spec_claims(&["exp", "aud", "sub"]);
        jsonwebtoken::decode::<Claims>(token, &key, &validation)
            .map(|data| data.claims)
            .map_err(|_| Rejected::NotYou)
    }
}

/// Tickets already spent, kept until they would have expired anyway.
///
/// Bounded by the ticket lifetime rather than by a policy: after sixty seconds a replayed
/// ticket fails on expiry, so remembering it longer buys nothing.
#[derive(Default)]
pub struct Spent {
    seen: HashMap<String, i64>,
}

impl Spent {
    /// Record a ticket, or say it has already been used.
    pub fn claim(&mut self, claims: &Claims, now_s: i64) -> Result<(), Rejected> {
        // Dropped on write. The map is only as large as the tickets spent in the last minute,
        // and every one of them is touched again to grow it.
        self.seen.retain(|_, exp| *exp > now_s);
        if self.seen.contains_key(&claims.jti) {
            return Err(Rejected::Spent);
        }
        self.seen.insert(claims.jti.clone(), claims.exp);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use crate::testing::Broker;
    use serde_json::json;

    const AUDIENCE: &str = "lightcone-server-1";

    fn trusting(broker: &Broker) -> Trusted {
        let mut trusted = Trusted::new(AUDIENCE);
        assert_eq!(trusted.learn(&broker.jwks()), 1);
        trusted
    }

    #[test]
    fn a_ticket_from_the_broker_names_its_account() {
        let broker = Broker::new([7u8; 32]);
        let claims = trusting(&broker)
            .check(&broker.mint("acct-1", AUDIENCE, 60, "j1"))
            .expect("it verifies");
        assert_eq!(claims.sub, "acct-1");
        assert_eq!(claims.name, "Ada");
    }

    /// Scoped to one server. A ticket for another shard is not a ticket here.
    #[test]
    fn a_ticket_for_another_server_is_refused() {
        let broker = Broker::new([7u8; 32]);
        let trusted = trusting(&broker);
        assert!(trusted.check(&broker.mint("acct-1", AUDIENCE, 60, "j1")).is_ok());
        assert_eq!(
            trusted.check(&broker.mint("acct-1", "lightcone-server-2", 60, "j1")),
            Err(Rejected::NotYou),
        );
    }

    #[test]
    fn an_expired_ticket_is_refused() {
        let broker = Broker::new([7u8; 32]);
        assert_eq!(
            trusting(&broker).check(&broker.mint("acct-1", AUDIENCE, -3600, "j1")),
            Err(Rejected::NotYou),
        );
    }

    /// A broker this server has never heard of does not get to say who anyone is.
    #[test]
    fn an_unknown_key_is_refused() {
        let ours = Broker::new([7u8; 32]);
        let stranger = Broker::new([9u8; 32]);
        assert_ne!(ours.kid, stranger.kid);
        assert_eq!(
            trusting(&ours).check(&stranger.mint("acct-1", AUDIENCE, 60, "j1")),
            Err(Rejected::NotYou),
        );
    }

    /// **The algorithm-confusion guard.** A token asking to be verified with a symmetric
    /// algorithm, whose key would be the public key everyone has, must not be.
    #[test]
    fn a_token_does_not_get_to_choose_its_own_algorithm() {
        let broker = Broker::new([7u8; 32]);
        let trusted = trusting(&broker);
        let public = broker.signing.verifying_key().to_bytes();

        let mut header = jsonwebtoken::Header::new(Algorithm::HS256);
        header.kid = Some(broker.kid.clone());
        let now = jsonwebtoken::get_current_timestamp() as i64;
        let forged = jsonwebtoken::encode(
            &header,
            &json!({
                "sub": "somebody-else", "name": "Mallory", "aud": AUDIENCE,
                "iss": "https://accounts.lightcone.example",
                "iat": now, "exp": now + 3600, "jti": "j1",
            }),
            // Signed with the *public* key as an HMAC secret, which is the whole attack.
            &jsonwebtoken::EncodingKey::from_secret(&public),
        )
        .unwrap();
        assert_eq!(trusted.check(&forged), Err(Rejected::NotYou));

        // And the same for `alg: none`, hand-built because no library will make one.
        let unsigned = format!(
            "{}.{}.",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(json!({"alg": "none", "kid": broker.kid}).to_string()),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                json!({"sub": "somebody-else", "aud": AUDIENCE, "exp": now + 3600, "jti": "j"})
                    .to_string()
            ),
        );
        assert_eq!(trusted.check(&unsigned), Err(Rejected::NotYou));
    }

    /// Anything that is not an Ed25519 signing key is ignored rather than guessed at.
    #[test]
    fn a_key_set_with_something_else_in_it_is_read_selectively() {
        let broker = Broker::new([7u8; 32]);
        let mut mixed = broker.jwks();
        mixed["keys"].as_array_mut().unwrap().push(json!({
            "kty": "RSA", "kid": "rsa-1", "n": "…", "e": "AQAB",
        }));
        mixed["keys"].as_array_mut().unwrap().push(json!({
            "kty": "OKP", "crv": "X25519", "kid": "x25519-1", "x": "…",
        }));

        let mut trusted = Trusted::new(AUDIENCE);
        assert_eq!(trusted.learn(&mixed), 1, "only the Ed25519 signing key");
        assert!(trusted.check(&broker.mint("acct-1", AUDIENCE, 60, "j1")).is_ok());

        // And nothing at all is an empty set, not a set that trusts everything.
        let mut empty = Trusted::new(AUDIENCE);
        assert_eq!(empty.learn(&json!({})), 0);
        assert!(empty.is_empty());
        assert_eq!(empty.check(&broker.mint("acct-1", AUDIENCE, 60, "j1")), Err(Rejected::NotYou));
    }

    /// One ticket, one connection.
    #[test]
    fn a_ticket_opens_exactly_one_socket() {
        let broker = Broker::new([7u8; 32]);
        let claims = trusting(&broker).check(&broker.mint("acct-1", AUDIENCE, 60, "j1")).unwrap();
        let now = jsonwebtoken::get_current_timestamp() as i64;

        let mut spent = Spent::default();
        assert_eq!(spent.claim(&claims, now), Ok(()));
        assert_eq!(spent.claim(&claims, now), Err(Rejected::Spent), "it was replayed");
    }

    /// The record is bounded by the ticket lifetime: past the expiry a replay fails anyway, so
    /// remembering it costs memory and buys nothing.
    #[test]
    fn spent_tickets_are_forgotten_once_they_would_have_expired() {
        let broker = Broker::new([7u8; 32]);
        let trusted = trusting(&broker);
        let now = jsonwebtoken::get_current_timestamp() as i64;

        let mut spent = Spent::default();
        for i in 0..50 {
            let claims = trusted.check(&broker.mint("acct-1", AUDIENCE, 60, &format!("j{i}"))).unwrap();
            spent.claim(&claims, now).unwrap();
        }
        assert_eq!(spent.len(), 50);

        let later = trusted.check(&broker.mint("acct-1", AUDIENCE, 60, "much-later")).unwrap();
        spent.claim(&later, now + 3600).unwrap();
        assert_eq!(spent.len(), 1, "the old ones were kept");
    }
}
