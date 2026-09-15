//! Signing in, from the site's side.
//!
//! Three routes and one call out. The whole flow is in
//! [16-identity.md](../../../lightcone/docs/16-identity.md); what matters here is that the site
//! is an ordinary first-party client of the broker, not an OAuth2 one, and that everything it
//! learns about a person is an opaque id and a name.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Redirect, Response};
use serde::Deserialize;

use crate::AppState;
use crate::session::{self, Session};

/// Where the broker sends a browser back to. On the broker's exact allowlist, so it carries no
/// query of its own — the one it arrives with is the broker's.
pub const RETURN_PATH: &str = "/auth/return";

/// What a handler needs to talk to the broker, when there is one to talk to.
pub struct Identity<'a> {
    pub base: &'a str,
    pub secret: &'a str,
    pub key: &'a [u8],
}

impl AppState {
    /// The broker, if this deployment has one wired up.
    ///
    /// All three or none. A broker with no session key would sign people in and then hand them
    /// a cookie anybody could forge, which is worse than no sign-in.
    pub fn identity(&self) -> Option<Identity<'_>> {
        Some(Identity {
            base: self.identity_base.as_deref()?,
            secret: self.identity_secret.as_deref()?,
            key: self.session_key.as_deref()?.as_bytes(),
        })
    }

    /// Who the request is from, if anyone.
    pub fn who(&self, headers: &HeaderMap) -> Option<Session> {
        let identity = self.identity()?;
        let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
        let value = session::from_header(cookies, session::COOKIE)?;
        session::open(identity.key, &value, chrono::Utc::now().timestamp())
    }
}

/// Send the player to the broker.
pub async fn signin(State(state): State<AppState>) -> Response {
    let Some(identity) = state.identity() else {
        return (StatusCode::NOT_FOUND, "this site has no sign-in").into_response();
    };
    let nonce = session::nonce();
    let url = format!(
        "{}/signin?return_to={}&state={nonce}",
        identity.base,
        urlencode(&format!("{}{RETURN_PATH}", state.base_url)),
    );
    // The nonce goes out in a cookie and comes back in the query. Without comparing the two, an
    // attacker can complete a sign-in *as themselves* in someone else's browser, and that
    // someone goes on using the attacker's account.
    ([(header::SET_COOKIE, session::set_state(&nonce, state.secure_cookies))], Redirect::to(&url))
        .into_response()
}

#[derive(Deserialize)]
pub struct Returned {
    code: Option<String>,
    state: Option<String>,
}

/// Take the code, find out who it was, and set a session.
pub async fn ret(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(returned): Query<Returned>,
) -> Response {
    let Some(identity) = state.identity() else {
        return (StatusCode::NOT_FOUND, "this site has no sign-in").into_response();
    };
    let expected = headers
        .get(header::COOKIE)
        .and_then(|c| c.to_str().ok())
        .and_then(|c| session::from_header(c, session::STATE_COOKIE));

    let (Some(code), Some(nonce), Some(expected)) = (returned.code, returned.state, expected)
    else {
        return refuse(&state);
    };
    if nonce != expected {
        return refuse(&state);
    }

    let asked = exchange(&identity, &format!("{}{RETURN_PATH}", state.base_url), &code).await;
    let Ok((sub, name)) = asked else {
        tracing::warn!("a sign-in code did not exchange");
        return refuse(&state);
    };

    let session = Session { sub, name, exp: chrono::Utc::now().timestamp() + session::LIFETIME_S };
    let sealed = session::seal(identity.key, &session);
    // `AppendHeaders`, not an array. An array of header pairs *inserts*, which replaces any
    // header of the same name — so two `Set-Cookie` entries leave one, and the one that
    // survives is the last. That silently dropped the session and left only the cleared state,
    // which reads on the wire as a sign-in that worked and did nothing.
    (
        AppendHeaders([
            (header::SET_COOKIE, session::set(&sealed, session::LIFETIME_S, state.secure_cookies)),
            (header::SET_COOKIE, session::clear_state(state.secure_cookies)),
        ]),
        Redirect::to("/play"),
    )
        .into_response()
}

pub async fn signout(State(state): State<AppState>) -> Response {
    ([(header::SET_COOKIE, session::clear(state.secure_cookies))], Redirect::to("/"))
        .into_response()
}

/// Swap a spent-once code for who it belonged to. Server to server.
async fn exchange(
    identity: &Identity<'_>,
    return_to: &str,
    code: &str,
) -> Result<(String, String), ()> {
    let answer = reqwest::Client::new()
        .post(format!("{}/exchange", identity.base))
        .bearer_auth(identity.secret)
        .json(&serde_json::json!({ "code": code, "return_to": return_to }))
        .send()
        .await
        .map_err(|_| ())?;
    if !answer.status().is_success() {
        return Err(());
    }
    let body: serde_json::Value = answer.json().await.map_err(|_| ())?;
    let field = |name: &str| body.get(name).and_then(|v| v.as_str()).map(str::to_owned);
    Ok((field("account_id").ok_or(())?, field("display_name").ok_or(())?))
}

/// Mint a game ticket for a signed-in player.
pub async fn ticket(state: &AppState, session: &Session) -> Option<String> {
    let identity = state.identity()?;
    let answer = reqwest::Client::new()
        .post(format!("{}/ticket", identity.base))
        .bearer_auth(identity.secret)
        .json(&serde_json::json!({ "account_id": session.sub, "audience": state.shard }))
        .send()
        .await
        .ok()?;
    if !answer.status().is_success() {
        tracing::warn!(status = %answer.status(), "the broker would not mint a ticket");
        return None;
    }
    let body: serde_json::Value = answer.json().await.ok()?;
    body.get("ticket").and_then(|v| v.as_str()).map(str::to_owned)
}

/// A sign-in that did not complete.
///
/// It says nothing about which step failed. A browser that learned whether its `state` was
/// wrong, or its code expired, or the broker refused it, would be telling whoever sent it.
fn refuse(state: &AppState) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::SET_COOKIE, session::clear_state(state.secure_cookies))],
        "that sign-in did not complete; try again",
    )
        .into_response()
}

/// Percent-encode a URL for use as a query value.
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_return_url_is_encoded_whole() {
        assert_eq!(
            urlencode("https://lightcone.example/auth/return"),
            "https%3A%2F%2Flightcone.example%2Fauth%2Freturn",
        );
        // The separators a query cannot carry raw are exactly the ones that must be encoded.
        assert!(!urlencode("a&b=c#d").contains(['&', '=', '#']));
    }

    /// The path the broker allowlists carries no query of its own, or the join is ambiguous
    /// and the broker refuses it at boot.
    #[test]
    fn the_return_path_is_joinable() {
        assert!(!RETURN_PATH.contains('?') && !RETURN_PATH.contains('#'));
        assert!(RETURN_PATH.starts_with('/'));
    }
}
