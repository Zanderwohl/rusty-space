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
    /// Where the **browser** is sent. Public, and the origin a person sees in the address bar.
    pub base: &'a str,
    /// Where **this process** calls the broker's server-to-server endpoints.
    ///
    /// Usually the same, and deliberately separable. In a container deployment they are not:
    /// the public name resolves to the host's own address, and a container reaching the host's
    /// published port by that address hairpins through the NAT and hangs. The symptom is a 504
    /// on `/auth/return` with nothing in either log, because nothing arrived anywhere. The
    /// answer is to call the broker by its name on the shared network instead.
    pub api: &'a str,
    pub secret: &'a str,
    pub key: &'a [u8],
}

impl AppState {
    /// The broker, if this deployment has one wired up.
    ///
    /// All three or none. A broker with no session key would sign people in and then hand them
    /// a cookie anybody could forge, which is worse than no sign-in.
    pub fn identity(&self) -> Option<Identity<'_>> {
        let base = self.identity_base.as_deref()?;
        Some(Identity {
            base,
            // Falls back to the public name, which is right for every deployment where the two
            // are reachable the same way — including every test and every local run.
            api: self.identity_api.as_deref().unwrap_or(base),
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

/// Keep an active player signed in.
///
/// A session is a fortnight from when it was issued, so without this somebody who uses the
/// site every day is still signed out a fortnight after signing in. Re-issuing it once it is
/// past halfway makes the fortnight count from the last visit instead.
///
/// **It must not undo a response that is already setting the cookie.** `/signout` clears it and
/// `/auth/return` replaces it, and both read as a valid session on the way in — appending a
/// refreshed cookie afterwards would make signing out a no-op.
pub async fn refresh(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let now = chrono::Utc::now().timestamp();
    let renewed = state
        .identity()
        .zip(state.who(request.headers()))
        .filter(|(_, session)| session::worth_refreshing(session, now))
        .map(|(identity, session)| {
            session::set(
                &session::seal(identity.key, &session::renewed(&session, now)),
                session::LIFETIME_S,
                state.secure_cookies,
            )
        });

    let mut response = next.run(request).await;
    let Some(cookie) = renewed else { return response };
    if sets_the_session(&response) {
        return response;
    }
    if let Ok(value) = cookie.parse() {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
    response
}

/// Whether a response already has something to say about the session cookie.
fn sets_the_session(response: &Response) -> bool {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .any(|value| value.starts_with(&format!("{}=", session::COOKIE)))
}

/// Send the player to the broker.
///
/// Unless they are already signed in, in which case there is nothing to send them for. Showing
/// a signed-in person a sign-in form is wrong on its own, and it also breaks: the dance ends at
/// `/auth/return`, which compares a nonce against a cookie that a completed sign-in has already
/// cleared, so the second attempt is refused as though it had failed.
pub async fn signin(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(identity) = state.identity() else {
        return (StatusCode::NOT_FOUND, "this site has no sign-in").into_response();
    };
    if let Some(session) = state.who(&headers) {
        return already(&session).into_response();
    }
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
        return refuse(&state, &headers);
    };
    if nonce != expected {
        return refuse(&state, &headers);
    }

    let asked = exchange(&identity, &format!("{}{RETURN_PATH}", state.base_url), &code).await;
    let Ok((sub, name)) = asked else {
        tracing::warn!("a sign-in code did not exchange");
        return refuse(&state, &headers);
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
        .post(format!("{}/exchange", identity.api))
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
        .post(format!("{}/ticket", identity.api))
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
///
/// **Already signed in is not a failure.** A stale sign-in finishing in a browser that already
/// holds a session is the back button, not an attack, and the code is simply dropped — which is
/// strictly safer than honoring it, since honoring a sign-in nobody here started is the login
/// CSRF the nonce exists to stop.
fn refuse(state: &AppState, headers: &HeaderMap) -> Response {
    if let Some(session) = state.who(headers) {
        return (
            [(header::SET_COOKIE, session::clear_state(state.secure_cookies))],
            already(&session),
        )
            .into_response();
    }
    (
        StatusCode::BAD_REQUEST,
        [(header::SET_COOKIE, session::clear_state(state.secure_cookies))],
        "that sign-in did not complete; try again",
    )
        .into_response()
}

/// What a signed-in person sees where a sign-in form would be.
///
/// It is also the only place this site offers a way *out*: nothing in the masthead says who is
/// signed in, so without this a player has a sign-in they cannot repeat and a sign-out they
/// cannot find.
fn already(session: &Session) -> maud::Markup {
    crate::views::shell(
        crate::views::Head::new("Signed in", "You are already signed in."),
        maud::html! {
            section class="stack" {
                h1 { "Signed in as " (session.name) }
                p class="lede" {
                    "There is nothing to sign in to — this browser already holds a session."
                }
                p { a class="cta" href="/play" { "Play" } }
                p { a href="/signout" { "Sign out" } }
            }
        },
    )
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

    fn with_cookies(values: &[String]) -> Response {
        let mut response = Response::new(axum::body::Body::empty());
        for value in values {
            response.headers_mut().append(header::SET_COOKIE, value.parse().unwrap());
        }
        response
    }

    /// The trap the refresh exists around. `/signout` clears the cookie, but the *request* it
    /// answers still carries a valid session — so a refresh that did not look at the response
    /// would append a fresh cookie behind it and make signing out do nothing at all.
    #[test]
    fn a_response_that_clears_the_session_is_left_alone() {
        assert!(sets_the_session(&with_cookies(&[session::clear(true)])));
        assert!(sets_the_session(&with_cookies(&[session::set("v", 60, true)])));
    }

    /// And an ordinary page is refreshed, or the whole thing does nothing.
    #[test]
    fn an_ordinary_response_is_refreshed() {
        assert!(!sets_the_session(&with_cookies(&[])));
    }

    /// The sign-in nonce is a different cookie and must not be mistaken for the session, or
    /// `/signin` would stop extending a session that is about to lapse mid-sign-in.
    #[test]
    fn the_nonce_cookie_is_not_the_session_cookie() {
        assert!(!sets_the_session(&with_cookies(&[session::set_state("n", true)])));
        assert!(!sets_the_session(&with_cookies(&[session::clear_state(true)])));
        // And a response carrying both is still recognized by the one that matters.
        assert!(sets_the_session(&with_cookies(&[
            session::clear_state(true),
            session::set("v", 60, true),
        ])));
    }

    fn identity<'a>(base: &'a str, api: Option<&'a str>) -> Identity<'a> {
        Identity { base, api: api.unwrap_or(base), secret: "s", key: b"k" }
    }

    /// The two addresses are the same thing until a deployment makes them different, and the
    /// fallback is what keeps every local run and every test on one value.
    #[test]
    fn the_api_address_falls_back_to_the_public_one() {
        let one = identity("https://accounts.example", None);
        assert_eq!(one.api, one.base);
    }

    /// And when they differ, the browser gets the public name while the server-to-server call
    /// gets the internal one. Crossing them is a 504 on `/auth/return` with nothing in any log.
    #[test]
    fn a_split_deployment_sends_the_browser_and_the_server_to_different_places() {
        let split = identity("https://accounts.example", Some("http://lightcone-identity:3200"));
        assert_eq!(split.base, "https://accounts.example");
        assert_eq!(split.api, "http://lightcone-identity:3200");
        // The one a person sees is the public one; the one a socket opens is not.
        assert!(split.base.starts_with("https://"));
        assert!(!split.api.contains("accounts.example"));
    }

    /// Where a sign-in form would be, a signed-in person gets their name and the two things
    /// they might actually want. The sign-out link matters more than it looks: nothing in the
    /// masthead says who is signed in, so this is the only way off the account.
    #[test]
    fn a_signed_in_person_is_told_so_and_offered_the_way_out() {
        let page = already(&Session { sub: "acct-1".into(), name: "Ada Lovelace".into(), exp: 0 })
            .into_string();

        assert!(page.contains("Ada Lovelace"), "{page}");
        assert!(page.contains("href=\"/signout\""), "no way to sign out: {page}");
        assert!(page.contains("href=\"/play\""), "{page}");
        // And emphatically not a form to sign in again, which is the thing that was broken.
        assert!(!page.contains("<form"), "it still offered a sign-in: {page}");
        assert!(!page.contains("did not complete"), "{page}");
    }

    /// The path the broker allowlists carries no query of its own, or the join is ambiguous
    /// and the broker refuses it at boot.
    #[test]
    fn the_return_path_is_joinable() {
        assert!(!RETURN_PATH.contains('?') && !RETURN_PATH.contains('#'));
        assert!(RETURN_PATH.starts_with('/'));
    }
}
