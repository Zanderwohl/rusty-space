//! Signing in to the administration site, and the extractor that guards every page of it.
//!
//! This service is an ordinary first-party client of the broker, exactly as the website is: it
//! sends a browser to `/signin`, gets a code back, and spends it server to server for an
//! account id. It is **not** an administrator's separate login. There is one account per
//! person and one password, and what makes somebody an administrator is a column, which is the
//! whole reason the level is read fresh on every request rather than carried in a token.

use axum::extract::{FromRequestParts, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{AppendHeaders, IntoResponse, Redirect, Response};
use lc_identity::ability;
use lc_identity::level::Level;
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;
use crate::session::{self, Session};
use crate::views;

/// Who is making this request, once they have been found to be an administrator.
///
/// The extractor for every page but the sign-in pair. A handler that takes one cannot be
/// reached by anybody else, which is a stronger statement than a check at the top of a
/// function that somebody can forget to write.
#[derive(Clone, Debug)]
pub struct Admin {
    pub id: Uuid,
    pub name: String,
    /// **Read from the database on this request.** Never from the cookie. See
    /// [`crate::session`].
    pub level: Level,
}

impl FromRequestParts<AppState> for Admin {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(session) = state.who(&parts.headers) else {
            return Err(to_sign_in(&parts.headers));
        };
        let Ok(id) = session.sub.parse::<Uuid>() else {
            // A well-signed cookie for something that is not an account id. Ours, but from a
            // build that meant something else by `sub`; sign them out rather than 500.
            return Err(signed_out(state));
        };
        let account = match state.store().account(id).await {
            Ok(Some(account)) => account,
            // Signed in as an account that no longer exists.
            Ok(None) => return Err(signed_out(state)),
            Err(why) => {
                tracing::error!(%why, "could not read the acting account");
                return Err(views::wrong(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong",
                    "The account store did not answer. Nothing was changed.",
                ));
            }
        };

        let level = account.level();
        if !ability::may_administer(level) {
            // Signed in, and not for here. A redirect to the sign-in page would loop, because
            // signing in again is exactly what they just did.
            return Err(views::wrong(
                StatusCode::FORBIDDEN,
                "Not for you",
                "This account does not administer anything.",
            ));
        }
        Ok(Admin {
            id,
            name: account.display_name,
            level,
        })
    }
}

/// Send an unauthenticated request to the sign-in page.
///
/// An htmx request gets `HX-Redirect` instead of a 303. htmx follows a redirect with `fetch`
/// and swaps whatever comes back, so a plain redirect would put the broker's sign-in form
/// inside the table it was refreshing — a page that looks broken rather than one that asks you
/// to sign in.
fn to_sign_in(headers: &HeaderMap) -> Response {
    if headers.contains_key("hx-request") {
        return (
            StatusCode::UNAUTHORIZED,
            [("hx-redirect", crate::routes::SIGNIN)],
        )
            .into_response();
    }
    Redirect::to(crate::routes::SIGNIN).into_response()
}

fn signed_out(state: &AppState) -> Response {
    (
        AppendHeaders([(header::SET_COOKIE, session::clear(state.secure_cookies))]),
        Redirect::to(crate::routes::SIGNIN),
    )
        .into_response()
}

impl AppState {
    /// Who the request is from, if the cookie is ours and current. Says nothing about whether
    /// they may be here.
    pub fn who(&self, headers: &HeaderMap) -> Option<Session> {
        let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
        let value = session::from_header(cookies, session::COOKIE)?;
        session::open(
            self.session_key.as_bytes(),
            &value,
            chrono::Utc::now().timestamp(),
        )
    }
}

/// Send the browser to the broker.
pub async fn signin(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if state.who(&headers).is_some() {
        // Already signed in. Showing the form again would fail: the dance ends by comparing a
        // nonce against a state cookie a completed sign-in has already cleared, so the second
        // attempt reads as a forgery.
        return Redirect::to(crate::routes::USERS).into_response();
    }
    let nonce = session::nonce();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("return_to", &state.return_url)
        .append_pair("state", &nonce)
        .finish();
    (
        AppendHeaders([(
            header::SET_COOKIE,
            session::set_state(&nonce, state.secure_cookies),
        )]),
        Redirect::to(&format!("{}/signin?{query}", state.identity_base)),
    )
        .into_response()
}

#[derive(Deserialize)]
pub struct Returned {
    #[serde(default)]
    code: String,
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
struct Identity {
    account_id: String,
    display_name: String,
}

/// Where the broker sends the browser back to.
pub async fn ret(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(returned): Query<Returned>,
) -> Response {
    let expected = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|c| session::from_header(c, session::STATE_COOKIE));
    // The nonce this browser started with. Without it a link anyone can send completes a
    // sign-in in somebody else's browser.
    let matched = expected
        .as_deref()
        .is_some_and(|nonce| !nonce.is_empty() && nonce == returned.state);
    if !matched || returned.code.is_empty() {
        return views::wrong(
            StatusCode::BAD_REQUEST,
            "That sign-in expired",
            "It was already used, or it sat too long. Start again.",
        );
    }

    let exchanged = state
        .http
        .post(format!("{}/exchange", state.identity_api))
        .bearer_auth(&state.identity_secret)
        .json(&serde_json::json!({
            "code": returned.code,
            "return_to": state.return_url,
        }))
        .send()
        .await;

    let identity = match exchanged {
        Ok(response) if response.status().is_success() => response.json::<Identity>().await,
        Ok(response) => {
            // The broker refusing is the ordinary case for a banned account, and a banned
            // account is not an administrator — but it is also what a mistyped secret looks
            // like, so it is logged with its status rather than swallowed.
            tracing::warn!(status = %response.status(), "the broker refused the exchange");
            return views::wrong(
                StatusCode::FORBIDDEN,
                "Not signed in",
                "The identity service would not complete that sign-in.",
            );
        }
        Err(why) => {
            tracing::error!(%why, "could not reach the broker");
            return views::wrong(
                StatusCode::BAD_GATEWAY,
                "The identity service is not answering",
                "Try again shortly.",
            );
        }
    };
    let Ok(identity) = identity else {
        return views::wrong(
            StatusCode::BAD_GATEWAY,
            "The identity service is not answering",
            "It replied with something this service could not read.",
        );
    };

    let session = Session {
        sub: identity.account_id,
        name: identity.display_name,
        exp: chrono::Utc::now().timestamp() + session::LIFETIME_S,
    };
    // Two cookies, appended. An array of header pairs *inserts*, which would leave only the
    // last of them — the trap `AGENTS.md` records, and a sign-in that sets a session and
    // clears a nonce is exactly the shape that hits it.
    (
        AppendHeaders([
            (
                header::SET_COOKIE,
                session::set(
                    &session::seal(state.session_key.as_bytes(), &session),
                    session::LIFETIME_S,
                    state.secure_cookies,
                ),
            ),
            (header::SET_COOKIE, session::clear_state(state.secure_cookies)),
        ]),
        Redirect::to(crate::routes::USERS),
    )
        .into_response()
}

pub async fn signout(State(state): State<AppState>) -> Response {
    (
        AppendHeaders([(header::SET_COOKIE, session::clear(state.secure_cookies))]),
        Redirect::to(crate::routes::SIGNIN),
    )
        .into_response()
}
