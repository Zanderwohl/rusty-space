//! A stand-in for the identity broker, for tests.
//!
//! This crate must not depend on the broker — it is another workspace, and
//! `lightcone/docs/16-identity.md` says the game server depends on a public key and a JWT
//! library and nothing else. So the minting half is written out here, from the same claim set,
//! and the two agree by that document rather than by sharing a crate.

use base64::Engine as _;
use jsonwebtoken::Algorithm;
use serde_json::json;

/// A ticket the broker would have minted. The key is a fixed seed, so the bytes are fixed too.
pub struct Broker {
    pub signing: ed25519_dalek::SigningKey,
    pub kid: String,
}

impl Broker {
    pub fn new(seed: [u8; 32]) -> Self {
        let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
        let kid = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(&signing.verifying_key().to_bytes()[..8]);
        Self { signing, kid }
    }

    pub fn jwks(&self) -> serde_json::Value {
        json!({"keys": [{
            "kty": "OKP", "crv": "Ed25519", "use": "sig", "alg": "EdDSA",
            "kid": self.kid,
            "x": base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(self.signing.verifying_key().to_bytes()),
        }]})
    }

    /// A ticket with no `perm`, as a broker from before permissions minted one.
    pub fn mint(&self, sub: &str, aud: &str, exp_in: i64, jti: &str) -> String {
        self.mint_claims(sub, aud, exp_in, jti, None)
    }

    /// A ticket for an account at a permission level.
    pub fn mint_with(&self, sub: &str, aud: &str, jti: &str, perm: i32) -> String {
        self.mint_claims(sub, aud, 60, jti, Some(perm))
    }

    fn mint_claims(&self, sub: &str, aud: &str, exp_in: i64, jti: &str, perm: Option<i32>) -> String {
        use ed25519_dalek::pkcs8::EncodePrivateKey;
        let now = jsonwebtoken::get_current_timestamp() as i64;
        let der = self.signing.to_pkcs8_der().unwrap();
        let key = jsonwebtoken::EncodingKey::from_ed_der(der.as_bytes());
        let mut header = jsonwebtoken::Header::new(Algorithm::EdDSA);
        header.kid = Some(self.kid.clone());
        let mut claims = json!({
            "sub": sub, "name": "Ada", "aud": aud,
            "iss": "https://accounts.lightcone.example",
            "iat": now, "exp": now + exp_in, "jti": jti,
        });
        if let Some(perm) = perm {
            claims["perm"] = json!(perm);
        }
        jsonwebtoken::encode(&header, &claims, &key).unwrap()
    }
}

