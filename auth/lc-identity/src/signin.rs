//! Signing in, and handing the result back to the site.
//!
//! A redirect and one server-to-server exchange, not an OAuth2 dance — see
//! `lightcone/docs/16-identity.md` for why two first-party services do not need one between
//! them, and what is given up by saying so.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::password;
use crate::providers::Provider;
use crate::store::{Link, Store, StoreError, normalise_email};

/// How long a sign-in code is worth anything.
///
/// It is carried from a redirect to one server-to-server call, which is milliseconds. A minute
/// is already generous, and generous is the right direction only until it is a window.
pub const CODE_LIFETIME_S: i64 = 60;

/// Longest `state` accepted, and it must be a nonce rather than a payload.
pub const MAX_STATE: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// `return_to` is not on the allowlist. Never say more than that: an error naming what
    /// *would* be allowed is a list of targets.
    BadReturn,
    /// `state` is missing or is not a nonce.
    BadState,
    /// The provider is not enabled here.
    NoSuchProvider,
    /// Wrong address **or** wrong password. Deliberately one variant: two would let a caller
    /// ask whether an account exists, which is the same leak the timing decoy closes.
    BadCredentials,
    Unacceptable(password::Unacceptable),
    Taken,
    /// The player said no at the provider's consent screen. Not a failure — the one refusal
    /// here that is somebody exercising a choice.
    Declined,
    /// The `state` matched no dance in flight: expired, already spent, or never issued.
    NoFlow,
    /// The provider could not be reached, or would not answer usefully.
    Upstream(String),
    Backend(String),
}

impl From<StoreError> for Refused {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::EmailTaken => Refused::Taken,
            StoreError::Backend(why) => Refused::Backend(why),
        }
    }
}

/// Whether `state` is a nonce this will echo back into a URL.
///
/// Restricted rather than escaped. The site generates it, so it can generate something that
/// needs no encoding, and a character set that cannot carry a `&` or a `#` cannot smuggle a
/// parameter into the redirect.
pub fn is_nonce(state: &str) -> bool {
    !state.is_empty()
        && state.len() <= MAX_STATE
        && state
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Whether an allowlist entry is one this can safely append to.
///
/// Checked at startup, not at redirect time. A configured URL that already carries a query
/// would make the join ambiguous, and ambiguity in building a redirect is how a parameter gets
/// overridden.
pub fn is_appendable(url: &str) -> bool {
    !url.is_empty() && !url.contains('?') && !url.contains('#')
}

/// The URL a completed sign-in redirects to.
pub fn return_url(return_to: &str, code: &str, state: &str) -> String {
    format!("{return_to}?code={code}&state={state}")
}

/// A fresh code, and the digest to store for it.
///
/// The **digest** is what is written down. A database dump then contains nothing that can be
/// exchanged, which matters more here than usual because a code is a bearer credential for an
/// account.
pub fn mint_code() -> (String, Vec<u8>) {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes).expect("the system random source");
    let code = URL_SAFE_NO_PAD.encode(bytes);
    let digest = digest_of(&code);
    (code, digest)
}

pub fn digest_of(code: &str) -> Vec<u8> {
    Sha256::digest(code.as_bytes()).to_vec()
}

/// Sign in with a password.
///
/// One refusal for "no such account" and for "wrong password", and the same work done either
/// way. Either half of that missing turns the form into an account enumerator.
pub async fn with_password(
    store: &Store,
    email: &str,
    password_input: &str,
) -> Result<Uuid, Refused> {
    let subject = normalise_email(email);
    let link = store.link(Provider::Password, &subject).await?;
    let stored = match &link {
        Some(link) => store.secret(Provider::Password, &link.subject).await?,
        None => None,
    };

    let Some(phc) = stored else {
        password::waste_time();
        return Err(Refused::BadCredentials);
    };
    if !password::verify(password_input, &phc) {
        return Err(Refused::BadCredentials);
    }

    // The upgrade path, taken at the only moment the plaintext exists. Best effort: a failure
    // here must not fail a sign-in that has already succeeded.
    if password::needs_rehash(&phc)
        && let Ok(fresh) = password::hash(password_input)
    {
        let _ = store.set_secret(Provider::Password, &subject, &fresh).await;
    }

    Ok(link.expect("a secret implies a link").account_id)
}

/// Make a password account.
///
/// The address is recorded and **not** verified, because delivery is deferred. That is why it
/// cannot align with anything: see `Store::account_for_verified_email`.
pub async fn register_password(
    store: &Store,
    email: &str,
    password_input: &str,
    display_name: &str,
) -> Result<Uuid, Refused> {
    let phc = password::hash(password_input).map_err(Refused::Unacceptable)?;
    let subject = normalise_email(email);
    if store.link(Provider::Password, &subject).await?.is_some() {
        return Err(Refused::Taken);
    }
    let account = store
        .add_link(
            None,
            display_name,
            Link {
                provider: Provider::Password,
                subject: subject.clone(),
                account_id: Uuid::nil(),
                email: Some(subject.clone()),
                email_verified: false,
            },
        )
        .await?;
    store.set_secret(Provider::Password, &subject, &phc).await?;
    Ok(account.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn with_account(email: &str, password_input: &str) -> Store {
        let store = Store::memory();
        register_password(&store, email, password_input, "Ada")
            .await
            .expect("registered");
        store
    }

    #[tokio::test]
    async fn a_registered_account_signs_in() {
        let store = with_account("ada@example.test", "a good enough password").await;
        let id = with_password(&store, "ada@example.test", "a good enough password")
            .await
            .unwrap();
        // And the address is matched however it is typed.
        let again = with_password(&store, "  Ada@Example.TEST ", "a good enough password")
            .await
            .unwrap();
        assert_eq!(id, again);
    }

    /// The two failures a login form must not distinguish.
    #[tokio::test]
    async fn a_missing_account_and_a_wrong_password_are_the_same_refusal() {
        let store = with_account("ada@example.test", "a good enough password").await;
        assert_eq!(
            with_password(&store, "ada@example.test", "the wrong password").await,
            Err(Refused::BadCredentials),
        );
        assert_eq!(
            with_password(&store, "nobody@example.test", "a good enough password").await,
            Err(Refused::BadCredentials),
        );
    }

    #[tokio::test]
    async fn an_address_cannot_be_registered_twice() {
        let store = with_account("ada@example.test", "a good enough password").await;
        assert_eq!(
            register_password(
                &store,
                "ADA@example.test",
                "another password entirely",
                "Imposter"
            )
            .await,
            Err(Refused::Taken),
        );
    }

    #[tokio::test]
    async fn a_password_that_is_refused_creates_nothing() {
        let store = Store::memory();
        assert_eq!(
            register_password(&store, "ada@example.test", "short", "Ada").await,
            Err(Refused::Unacceptable(password::Unacceptable::TooShort)),
        );
        assert!(
            store
                .link(Provider::Password, "ada@example.test")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// A `state` is echoed into a URL, so it is restricted to something that cannot carry a
    /// parameter of its own.
    #[test]
    fn state_must_be_a_nonce() {
        assert!(is_nonce("abc123"));
        assert!(is_nonce("A-nonce_with-dashes"));
        for hostile in [
            "",
            "a&b=c",
            "a#frag",
            "a b",
            "a/b",
            "a%26b",
            &"x".repeat(MAX_STATE + 1),
        ] {
            assert!(!is_nonce(hostile), "{hostile:?} was accepted as a nonce");
        }
    }

    /// Checked when the allowlist is read, so an ambiguous join is a failed boot rather than a
    /// redirect someone can steer.
    #[test]
    fn an_allowlist_entry_must_be_joinable() {
        assert!(is_appendable("https://lightcone.example/auth/return"));
        for bad in [
            "",
            "https://lightcone.example/r?next=x",
            "https://lightcone.example/r#x",
        ] {
            assert!(
                !is_appendable(bad),
                "{bad:?} would make an ambiguous redirect"
            );
        }
    }

    #[test]
    fn the_redirect_carries_the_code_and_the_state_back() {
        let url = return_url("https://lightcone.example/auth/return", "CODE", "NONCE");
        assert_eq!(
            url,
            "https://lightcone.example/auth/return?code=CODE&state=NONCE"
        );
    }

    /// What is written down is a digest. A dump of the codes table is worth nothing.
    #[test]
    fn a_code_is_stored_as_its_digest_and_never_repeats() {
        let (one, one_digest) = mint_code();
        let (two, two_digest) = mint_code();
        assert_ne!(one, two, "two codes were the same");
        assert_ne!(one_digest, two_digest);
        assert_eq!(
            digest_of(&one),
            one_digest,
            "a presented code must find its own row"
        );
        assert_ne!(digest_of(&two), one_digest);
        assert_eq!(one_digest.len(), 32);
        // The digest is not the code, which is the whole point.
        assert!(!one.as_bytes().starts_with(&one_digest[..4]));
    }
}
