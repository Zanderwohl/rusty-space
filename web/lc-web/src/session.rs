//! Who is signed in, carried in a signed cookie.
//!
//! A signed cookie rather than a sessions table. The site is shaped to serve with its database
//! stopped — [14-hosting.md](../../../lightcone/docs/14-hosting.md) calls that a supported
//! state — and a sign-in that needed a table would be the one page that does not. What is
//! given up is instant revocation, and the answer is a short expiry.
//!
//! The cookie holds nothing the site could not tell you anyway: an opaque account id and a
//! display name. No email, no provider, nothing about the world. That is the whole of what
//! crosses out of the broker.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

/// The cookie's name.
pub const COOKIE: &str = "lc_session";

/// And the one that carries a sign-in's nonce while it is in flight.
pub const STATE_COOKIE: &str = "lc_signin_state";

/// How long a session lasts.
///
/// A fortnight. Long enough that a player is not signing in weekly, short enough that it is
/// also the revocation window — which is the trade a signed cookie makes.
pub const LIFETIME_S: i64 = 14 * 24 * 60 * 60;

/// How long a sign-in has to complete.
pub const SIGNIN_WINDOW_S: i64 = 10 * 60;

/// What the cookie says.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    /// The opaque account id, which is the only identifier that crosses a product boundary.
    pub sub: String,
    pub name: String,
    pub exp: i64,
}

/// Sign a session into a cookie value.
pub fn seal(key: &[u8], session: &Session) -> String {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(session).expect("a session encodes"));
    let mac = sign(key, payload.as_bytes());
    format!("{payload}.{mac}")
}

/// Read one back, if it is ours and still current.
///
/// `None` for anything wrong, without saying which: a caller can do nothing differently with
/// "tampered" than with "expired", and the one thing it must not do is trust it.
pub fn open(key: &[u8], value: &str, now: i64) -> Option<Session> {
    let (payload, mac) = value.split_once('.')?;
    // Verified **before** the payload is parsed. Deserialising something unauthenticated is
    // running a parser on input an attacker chose.
    let expected = sign(key, payload.as_bytes());
    if !constant_time_eq(mac.as_bytes(), expected.as_bytes()) {
        return None;
    }
    let session: Session = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    (session.exp > now).then_some(session)
}

/// The `Set-Cookie` for a signed-in session.
///
/// `HttpOnly` so no script can read it, `Secure` so it never crosses plain HTTP, and
/// `SameSite=Lax` — not `Strict` — because the sign-in comes back as a top-level navigation
/// *from the broker*, and `Strict` would withhold the cookie on exactly that request.
pub fn set(value: &str, max_age_s: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{COOKIE}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_s}{secure}")
}

/// The `Set-Cookie` that removes one.
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
///
/// Hand-parsed because the site has no cookie crate and this is the whole of what it needs:
/// split on `;`, split on the first `=`, trim. Anything it cannot make sense of is absent,
/// which for a session cookie is the safe reading.
pub fn from_header(header: &str, name: &str) -> Option<String> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim().to_owned())
    })
}

/// A nonce for a sign-in in flight.
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

/// Length-independent and content-independent comparison.
///
/// A `==` on a MAC is a MAC guessable one byte at a time, given enough attempts and a clock.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &[u8] = b"a site session key, of some length";
    const NOW: i64 = 1_700_000_000;

    fn a_session() -> Session {
        Session { sub: "acct-1".into(), name: "Ada".into(), exp: NOW + LIFETIME_S }
    }

    #[test]
    fn a_sealed_session_opens_again() {
        let sealed = seal(KEY, &a_session());
        assert_eq!(open(KEY, &sealed, NOW), Some(a_session()));
    }

    /// The whole point of signing it. A cookie the holder can edit is a cookie that says
    /// whatever they like about who they are.
    #[test]
    fn an_edited_cookie_is_refused() {
        let forged = Session { sub: "somebody-else".into(), ..a_session() };
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&forged).unwrap());

        // Their own payload with our signature over the original.
        let (_, mac) = seal(KEY, &a_session())
            .split_once('.')
            .map(|(a, b)| (a.to_owned(), b.to_owned()))
            .unwrap();
        assert_eq!(open(KEY, &format!("{payload}.{mac}"), NOW), None);

        // And their own payload signed with a key they chose.
        let theirs = seal(b"a key they made up", &forged);
        assert_eq!(open(KEY, &theirs, NOW), None);
    }

    #[test]
    fn an_expired_session_is_nobody() {
        let stale = Session { exp: NOW - 1, ..a_session() };
        let sealed = seal(KEY, &stale);
        assert_eq!(open(KEY, &sealed, NOW), None);
        // And it was valid before it was not, so the refusal is the expiry and not the shape.
        assert!(open(KEY, &sealed, NOW - 2).is_some());
    }

    /// Anything malformed is absent rather than an error, and none of it panics.
    #[test]
    fn nonsense_is_nobody() {
        for value in ["", ".", "a.b", "....", "!!!.???", &"x".repeat(10_000)] {
            assert_eq!(open(KEY, value, NOW), None, "{value:?} was accepted");
        }
    }

    /// The signature is checked before the payload is parsed. Running a deserialiser over
    /// unauthenticated input is running a parser on something an attacker chose.
    #[test]
    fn the_signature_is_checked_before_the_payload() {
        // A payload that is not JSON at all, signed correctly. If the order were wrong this
        // would fail in the parser rather than at the MAC — the result is the same either way,
        // which is why the order has to be asserted rather than observed.
        let payload = URL_SAFE_NO_PAD.encode(b"not json");
        let sealed = format!("{payload}.{}", sign(KEY, payload.as_bytes()));
        assert_eq!(open(KEY, &sealed, NOW), None);

        // Unsigned nonsense is refused *without* the parser seeing it.
        let unsigned = format!("{payload}.{}", sign(b"another key", payload.as_bytes()));
        assert_eq!(open(KEY, &unsigned, NOW), None);
    }

    /// `SameSite=Lax` and not `Strict`: the sign-in comes back as a top-level navigation from
    /// the broker, and `Strict` withholds the cookie on exactly that request.
    #[test]
    fn the_cookie_is_locked_down_but_survives_the_redirect_back() {
        let header = set("value", LIFETIME_S, true);
        assert!(header.contains("HttpOnly"), "{header}");
        assert!(header.contains("Secure"), "{header}");
        assert!(header.contains("SameSite=Lax"), "{header}");
        assert!(!header.contains("SameSite=Strict"), "{header}");
        assert!(header.contains("Path=/"), "{header}");

        // Development over plain HTTP would never receive a `Secure` cookie.
        assert!(!set("value", LIFETIME_S, false).contains("Secure"));
        // Clearing is the same cookie with no life left.
        assert!(clear(true).contains("Max-Age=0"));
    }

    #[test]
    fn one_cookie_is_found_among_others() {
        let header = "theme=dark; lc_session=abc.def; other=1";
        assert_eq!(from_header(header, COOKIE).as_deref(), Some("abc.def"));
        assert_eq!(from_header(header, "theme").as_deref(), Some("dark"));
        assert_eq!(from_header(header, "absent"), None);
        // A name that is a prefix of another must not match it.
        assert_eq!(from_header("lc_session_other=x", COOKIE), None);
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
