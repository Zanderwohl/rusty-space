//! Who is signed in to the administration site.
//!
//! A signed cookie, the same shape the site uses, with two differences that both come from
//! what this service is for.
//!
//! **It is short.** Eight hours rather than a fortnight: a session here is a working day at a
//! desk, and the revocation window of a signed cookie is its lifetime.
//!
//! **It does not carry a level.** Only the account id and a name — the level is read from the
//! database on every request. A level in the cookie would mean a demotion took effect whenever
//! the demoted person next signed in, which is to say at a time of their choosing, and the
//! whole point of being able to demote somebody is that it happens now.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use subtle::ConstantTimeEq;

pub const COOKIE: &str = "lc_admin";
/// And the one holding a sign-in's nonce while it is in flight.
pub const STATE_COOKIE: &str = "lc_admin_state";

/// Eight hours. See the module note.
pub const LIFETIME_S: i64 = 8 * 60 * 60;
/// How long a sign-in has to complete.
pub const SIGNIN_WINDOW_S: i64 = 10 * 60;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    /// The opaque account id.
    pub sub: String,
    pub name: String,
    pub exp: i64,
}

pub fn seal(key: &[u8], session: &Session) -> String {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(session).expect("a session encodes"));
    let mac = sign(key, payload.as_bytes());
    format!("{payload}.{mac}")
}

/// Read one back, if it is ours and still current.
///
/// `None` for anything wrong, without saying which. The signature is checked **before** the
/// payload is parsed: deserialising something unauthenticated is running a parser on input an
/// attacker chose.
pub fn open(key: &[u8], value: &str, now: i64) -> Option<Session> {
    let (payload, mac) = value.split_once('.')?;
    let expected = sign(key, payload.as_bytes());
    if !bool::from(mac.as_bytes().ct_eq(expected.as_bytes())) {
        return None;
    }
    let session: Session = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    (session.exp > now).then_some(session)
}

/// `SameSite=Lax` rather than `Strict`, because the sign-in comes back as a top-level
/// navigation from the broker and `Strict` withholds the cookie on exactly that request.
pub fn set(value: &str, max_age_s: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_s}{secure}")
}

pub fn clear(secure: bool) -> String {
    set("", 0, secure)
}

pub fn set_state(nonce: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{STATE_COOKIE}={nonce}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SIGNIN_WINDOW_S}{secure}"
    )
}

pub fn clear_state(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{STATE_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
}

/// One cookie out of a `Cookie:` header.
pub fn from_header(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    })
}

pub fn nonce() -> String {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).expect("the system random source");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sign(key: &[u8], payload: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("hmac takes any key length");
    mac.update(payload);
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"a key of at least thirty-two characters";
    const NOW: i64 = 1_700_000_000;

    fn a_session() -> Session {
        Session {
            sub: "acct-1".into(),
            name: "Ada".into(),
            exp: NOW + LIFETIME_S,
        }
    }

    #[test]
    fn a_sealed_session_opens_again() {
        assert_eq!(open(KEY, &seal(KEY, &a_session()), NOW), Some(a_session()));
    }

    /// The whole point of signing it. A cookie the holder can edit is a cookie that says
    /// whichever account it likes — and here that account administers the game.
    #[test]
    fn an_edited_cookie_is_refused() {
        let forged = Session {
            sub: "somebody-else".into(),
            ..a_session()
        };
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());
        let (_, mac) = seal(KEY, &a_session())
            .split_once('.')
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .unwrap();
        assert_eq!(open(KEY, &format!("{payload}.{mac}"), NOW), None);
        assert_eq!(open(KEY, &seal(b"a key they made up", &forged), NOW), None);
    }

    #[test]
    fn an_expired_session_is_nobody() {
        let stale = Session {
            exp: NOW - 1,
            ..a_session()
        };
        let sealed = seal(KEY, &stale);
        assert_eq!(open(KEY, &sealed, NOW), None);
        // And it was valid before it was not, so the refusal is the expiry and not the shape.
        assert!(open(KEY, &sealed, NOW - 2).is_some());
    }

    #[test]
    fn nonsense_is_nobody() {
        for value in ["", ".", "a.b", "....", "!!!.???", &"x".repeat(10_000)] {
            assert_eq!(open(KEY, value, NOW), None, "{value:?} was accepted");
        }
        // Correctly signed, but not JSON: the MAC is checked first and the parser never runs.
        let payload = URL_SAFE_NO_PAD.encode(b"not json");
        let sealed = format!("{payload}.{}", sign(KEY, payload.as_bytes()));
        assert_eq!(open(KEY, &sealed, NOW), None);
    }

    /// An administration session is a working day, not a fortnight.
    #[test]
    fn the_session_is_short_and_locked_down() {
        assert_eq!(LIFETIME_S, 8 * 60 * 60);
        let header = set("value", LIFETIME_S, true);
        assert!(header.contains("HttpOnly"), "{header}");
        assert!(header.contains("Secure"), "{header}");
        assert!(header.contains("SameSite=Lax"), "{header}");
        assert!(!set("value", LIFETIME_S, false).contains("Secure"));
        assert!(clear(true).contains("Max-Age=0"));
    }

    /// The level is read from the database, never from the cookie. If a level ever appears in
    /// this struct, a demotion stops taking effect until the demoted person signs in again.
    #[test]
    fn the_cookie_says_nothing_about_what_the_holder_may_do() {
        let sealed = seal(KEY, &a_session());
        let payload = sealed.split('.').next().unwrap();
        let decoded = String::from_utf8(URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        for forbidden in ["level", "permission", "perm", "admin"] {
            assert!(
                !decoded.contains(forbidden),
                "the session cookie carries {forbidden}: {decoded}",
            );
        }
    }

    #[test]
    fn one_cookie_is_found_among_others() {
        let header = "theme=dark; lc_admin=abc.def; other=1";
        assert_eq!(from_header(header, COOKIE).as_deref(), Some("abc.def"));
        assert_eq!(from_header("lc_admin_other=x", COOKIE), None);
        assert_eq!(from_header("", COOKIE), None);
    }

    #[test]
    fn a_nonce_is_long_and_never_the_same_twice() {
        let one = nonce();
        assert_eq!(one.len(), 32);
        assert!(one.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(one, nonce());
    }
}
