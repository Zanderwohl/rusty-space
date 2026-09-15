//! Game tickets: the one thing the broker hands a client to give to the game server.
//!
//! Sixty seconds, single use, and audience-scoped to one game server. It is carried from a page
//! load or a launch to a socket open, which is one round trip — not a session. See
//! `lightcone/docs/16-identity.md`.
//!
//! **The game server verifies these without calling here**, against the public key published at
//! `/.well-known/jwks.json`. That is what keeps a broker outage from stopping existing players
//! reconnecting, and it is why the claim set below is also written down in the doc: two
//! implementations agree by contract, not by sharing a crate across a workspace boundary.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::SigningKey;
use ed25519_dalek::pkcs8::EncodePrivateKey;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// How long a ticket is worth anything.
///
/// A minute is generous for a page-load-to-socket-open, and generous is the right direction
/// only up to the point where it becomes a window. This *is* the revocation latency: there is
/// no revocation list, because a credential this short-lived does not need one.
pub const LIFETIME_S: i64 = 60;

/// What a ticket says. Nothing else: a claim the game does not need is a claim that leaks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Claims {
    /// The opaque account id, which is the only identifier that crosses a product boundary.
    pub sub: String,
    pub name: String,
    /// The game server this is for, and only it. A ticket is useless anywhere else.
    pub aud: String,
    pub iss: String,
    pub iat: i64,
    pub exp: i64,
    /// Recorded by the game server until it expires, so a ticket in a log or a screenshot is
    /// worth nothing a second time.
    pub jti: String,
}

/// The broker's signing key, and the identifier it publishes it under.
pub struct Keys {
    signing: SigningKey,
    encoding: EncodingKey,
    kid: String,
    issuer: String,
}

impl Keys {
    /// Generate one.
    ///
    /// A broker that could not make its own key could not start without an operator first
    /// finding an `openssl` invocation, and a development instance that cannot start is a
    /// development instance nobody runs.
    pub fn generate(issuer: &str) -> anyhow::Result<Self> {
        let mut seed = [0u8; 32];
        getrandom::getrandom(&mut seed)?;
        Self::from_seed(&seed, issuer)
    }

    /// From a 32-byte seed, which is what configuration carries in production.
    ///
    /// A seed rather than a PEM: it is 43 characters of base64 in an environment variable
    /// instead of a multi-line secret, and there is exactly one way to get it wrong.
    pub fn from_seed(seed: &[u8; 32], issuer: &str) -> anyhow::Result<Self> {
        let signing = SigningKey::from_bytes(seed);
        let der = signing.to_pkcs8_der()?;
        let encoding = EncodingKey::from_ed_der(der.as_bytes());
        // The key identifier is derived from the key, so two brokers holding the same key agree
        // on its name without being told, and rotating the key rotates the `kid` by itself.
        let kid = URL_SAFE_NO_PAD.encode(&signing.verifying_key().to_bytes()[..8]);
        Ok(Self {
            signing,
            encoding,
            kid,
            issuer: issuer.to_owned(),
        })
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The public half, base64url, as the JWK `x` parameter.
    pub fn public_x(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.signing.verifying_key().to_bytes())
    }

    /// What `/.well-known/jwks.json` serves.
    ///
    /// A set, not a key, so rotation is publishing two and retiring one rather than a flag day.
    pub fn jwks(&self) -> serde_json::Value {
        json!({
            "keys": [{
                "kty": "OKP",
                "crv": "Ed25519",
                "use": "sig",
                "alg": "EdDSA",
                "kid": self.kid,
                "x": self.public_x(),
            }]
        })
    }

    /// Mint a ticket for an account, for one game server.
    pub fn mint(
        &self,
        account_id: &str,
        name: &str,
        audience: &str,
        now: i64,
    ) -> anyhow::Result<String> {
        let mut jti = [0u8; 16];
        getrandom::getrandom(&mut jti)?;
        let claims = Claims {
            sub: account_id.to_owned(),
            name: name.to_owned(),
            aud: audience.to_owned(),
            iss: self.issuer.clone(),
            iat: now,
            exp: now + LIFETIME_S,
            jti: URL_SAFE_NO_PAD.encode(jti),
        };
        let mut header = Header::new(Algorithm::EdDSA);
        // Without this the verifier has to guess which key signed it, and "try them all" is how
        // a retired key stays live.
        header.kid = Some(self.kid.clone());
        Ok(jsonwebtoken::encode(&header, &claims, &self.encoding)?)
    }
}

impl std::fmt::Debug for Keys {
    /// The private half is not printed, on the same reasoning as a client secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys")
            .field("kid", &self.kid)
            .field("issuer", &self.issuer)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation};

    const SEED: [u8; 32] = [7u8; 32];
    const AUDIENCE: &str = "lightcone-server-1";

    /// The real clock. A ticket minted at a fixed past timestamp is an *expired* ticket, so a
    /// test that wants to be about the audience or the signature has to mint one that is live —
    /// or it passes for the wrong reason, and goes on passing when the thing it names breaks.
    fn now() -> i64 {
        jsonwebtoken::get_current_timestamp() as i64
    }

    fn keys() -> Keys {
        Keys::from_seed(&SEED, "https://accounts.lightcone.example").unwrap()
    }

    /// The game server's side of the contract, written the way the game server will write it:
    /// from the JWK alone, with the algorithm and audience stated rather than inferred.
    fn verify(token: &str, jwk_x: &str, audience: &str) -> jsonwebtoken::errors::Result<Claims> {
        let key = DecodingKey::from_ed_components(jwk_x)?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[audience]);
        validation.set_required_spec_claims(&["exp", "aud", "sub"]);
        Ok(jsonwebtoken::decode::<Claims>(token, &key, &validation)?.claims)
    }

    #[test]
    fn a_ticket_verifies_against_the_published_key() {
        let keys = keys();
        let now = now();
        let token = keys.mint("acct-1", "Ada", AUDIENCE, now).unwrap();
        let claims = verify(&token, &keys.public_x(), AUDIENCE).expect("it verifies");

        assert_eq!(claims.sub, "acct-1");
        assert_eq!(claims.name, "Ada");
        assert_eq!(claims.aud, AUDIENCE);
        assert_eq!(claims.exp, now + LIFETIME_S);
        assert!(!claims.jti.is_empty());
    }

    /// Scoped to one server. A ticket for the shard you are on is not a ticket for another.
    #[test]
    fn a_ticket_is_useless_at_the_wrong_audience() {
        let keys = keys();
        let token = keys.mint("acct-1", "Ada", AUDIENCE, now()).unwrap();
        // Live at its own audience, so the refusal below is about the audience and nothing else.
        assert!(verify(&token, &keys.public_x(), AUDIENCE).is_ok());
        assert!(verify(&token, &keys.public_x(), "lightcone-server-2").is_err());
    }

    /// The whole revocation story is the expiry, so the expiry has to actually be checked.
    #[test]
    fn a_ticket_stops_working() {
        let keys = keys();
        // Minted far enough in the past that it is expired by any clock skew allowance.
        let long_ago = now() - 3600;
        let token = keys.mint("acct-1", "Ada", AUDIENCE, long_ago).unwrap();
        assert!(verify(&token, &keys.public_x(), AUDIENCE).is_err());
    }

    /// A different broker's key does not sign tickets for this one.
    #[test]
    fn another_key_does_not_sign_for_this_one() {
        let mine = keys();
        let theirs = Keys::from_seed(&[9u8; 32], "https://accounts.lightcone.example").unwrap();
        let token = theirs.mint("acct-1", "Ada", AUDIENCE, now()).unwrap();
        assert!(
            verify(&token, &theirs.public_x(), AUDIENCE).is_ok(),
            "live under its own key"
        );
        assert!(verify(&token, &mine.public_x(), AUDIENCE).is_err());
        assert_ne!(mine.kid(), theirs.kid(), "two keys shared a name");
    }

    /// Editing a claim invalidates the signature, which is the entire reason this is signed
    /// rather than sent as a struct.
    #[test]
    fn a_tampered_ticket_is_refused() {
        let keys = keys();
        let token = keys.mint("acct-1", "Ada", AUDIENCE, now()).unwrap();
        assert!(
            verify(&token, &keys.public_x(), AUDIENCE).is_ok(),
            "live before it is edited"
        );
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&json!({
                "sub": "somebody-else", "name": "Ada", "aud": AUDIENCE,
                "iss": "https://accounts.lightcone.example",
                "iat": now(), "exp": now() + 3600, "jti": "x",
            }))
            .unwrap(),
        );
        parts[1] = &forged;
        assert!(verify(&parts.join("."), &keys.public_x(), AUDIENCE).is_err());
    }

    /// Two tickets for the same account are still two tickets. Without that the game server's
    /// replay check would refuse the second honest connection of a session.
    #[test]
    fn every_ticket_has_its_own_identifier() {
        let keys = keys();
        let one = keys.mint("acct-1", "Ada", AUDIENCE, now()).unwrap();
        let two = keys.mint("acct-1", "Ada", AUDIENCE, now()).unwrap();
        assert_ne!(one, two);
    }

    /// The `kid` comes from the key, so a verifier can pick the right one out of a set and a
    /// rotation renames itself.
    #[test]
    fn the_key_identifier_is_derived_from_the_key() {
        let keys = keys();
        assert_eq!(
            keys.kid(),
            Keys::from_seed(&SEED, "someone else").unwrap().kid()
        );

        let jwks = keys.jwks();
        let published = &jwks["keys"][0];
        assert_eq!(published["kid"], keys.kid());
        assert_eq!(published["x"], keys.public_x());
        assert_eq!(published["crv"], "Ed25519");
        assert_eq!(published["alg"], "EdDSA");
    }

    /// A private key that reaches a log is a private key.
    #[test]
    fn the_signing_key_is_not_printed() {
        let dumped = format!("{:?}", keys());
        assert!(dumped.contains("kid"));
        assert!(
            !dumped.contains(&URL_SAFE_NO_PAD.encode(SEED)),
            "the seed escaped: {dumped}"
        );
    }
}
