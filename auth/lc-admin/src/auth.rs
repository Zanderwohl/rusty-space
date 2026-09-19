//! Signing in to the administration site, and the extractor that guards every page of it.
//!
//! An ordinary first-party client of the broker, as the website is. There is no separate
//! administrator login: what makes somebody one is a column, which is why the level is read
//! fresh on every request rather than carried in a token.

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

/// The extractor for every page but the sign-in pair, so a handler that takes one cannot be
/// reached by anybody else — a stronger statement than a check somebody can forget to write.
#[derive(Clone, Debug)]
pub struct Admin {
    pub id: Uuid,
    pub name: String,
    /// **Read from the database on this request**, never from the cookie. See [`crate::session`].
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
            // Ours, but from a build that meant something else by `sub`.
            return Err(signed_out(state));
        };
        let account = match state.store().account(id).await {
            Ok(Some(account)) => account,
            // Signed in as an account that no longer exists.
            Ok(None) => return Err(signed_out(state)),
            Err(why) => {
                tracing::error!(%why, "could not read the acting account");
                return Err(views::wrong(
                    &state.assets,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Something went wrong",
                    "The account store did not answer. Nothing was changed.",
                ));
            }
        };

        let level = account.level();
        if !ability::may_administer(level) {
            // **Only reached by an administrator demoted mid-session.** A player never gets this
            // far: `ret` checks the level before it seals anything. Redirecting to sign-in would
            // loop.
            return Err(not_an_administrator(state));
        }
        Ok(Admin {
            id,
            name: account.display_name,
            level,
        })
    }
}

/// **The session is cleared.** One that only ever produces a refusal is worth nothing to its
/// holder, and leaving it turns this page into a trap — the only navigation is a masthead
/// administrators see, so there would be no link to anything, including to signing out.
fn not_an_administrator(state: &AppState) -> Response {
    (
        AppendHeaders([
            (header::SET_COOKIE, session::clear(state.secure_cookies)),
            (
                header::SET_COOKIE,
                session::clear_state(state.secure_cookies),
            ),
        ]),
        views::refusal(
            &state.assets,
            StatusCode::FORBIDDEN,
            "Not an administrator",
            "That account does not administer anything, so there is nothing here for it. \
             You have been signed out of this console.",
            (crate::routes::SIGNIN, "Sign in as another account"),
        ),
    )
        .into_response()
}

/// htmx gets `HX-Redirect`, not a 303: it follows a redirect with `fetch` and would swap the
/// broker's sign-in form into the table it was refreshing.
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
    /// Says nothing about whether they may be here.
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
        // Showing the form again would fail: the dance compares a nonce against a state cookie a
        // completed sign-in has already cleared.
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
    // Without it, a link anyone can send completes a sign-in in somebody else's browser.
    let matched = expected
        .as_deref()
        .is_some_and(|nonce| !nonce.is_empty() && nonce == returned.state);
    if !matched || returned.code.is_empty() {
        return views::refusal(
            &state.assets,
            StatusCode::BAD_REQUEST,
            "That sign-in expired",
            "It was already used, or it sat too long.",
            (crate::routes::SIGNIN, "Start again"),
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
            // A banned account looks the same as a mistyped secret, hence the status in the log.
            tracing::warn!(status = %response.status(), "the broker refused the exchange");
            return views::refusal(
                &state.assets,
                StatusCode::FORBIDDEN,
                "Not signed in",
                "The identity service would not complete that sign-in.",
                (crate::routes::SIGNIN, "Try again"),
            );
        }
        Err(why) => {
            tracing::error!(%why, "could not reach the broker");
            return views::refusal(
                &state.assets,
                StatusCode::BAD_GATEWAY,
                "The identity service is not answering",
                "Try again shortly.",
                (crate::routes::SIGNIN, "Try again"),
            );
        }
    };
    let Ok(identity) = identity else {
        return views::wrong(
            &state.assets,
            StatusCode::BAD_GATEWAY,
            "The identity service is not answering",
            "It replied with something this service could not read.",
        );
    };

    // **Checked before anything is sealed**: a player is left holding nothing rather than a
    // cookie whose only use is to be rejected.
    let Ok(id) = identity.account_id.parse::<Uuid>() else {
        return views::wrong(
            &state.assets,
            StatusCode::BAD_GATEWAY,
            "The identity service is not answering",
            "It named an account this service could not read.",
        );
    };
    match state.store().account(id).await {
        Ok(Some(account)) if ability::may_administer(account.level()) => {}
        Ok(_) => return not_an_administrator(&state),
        Err(why) => {
            // Cannot tell whether they may be here, so do not let them.
            tracing::error!(%why, "could not read the signing-in account");
            return views::wrong(
                &state.assets,
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "The account store did not answer. You have not been signed in.",
            );
        }
    }

    let session = Session {
        sub: identity.account_id,
        name: identity.display_name,
        exp: chrono::Utc::now().timestamp() + session::LIFETIME_S,
    };
    // Appended: an array of header pairs *inserts*, leaving only the last — the `AGENTS.md`
    // trap, and a sign-in that sets a session and clears a nonce is its exact shape.
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
            (
                header::SET_COOKIE,
                session::clear_state(state.secure_cookies),
            ),
        ]),
        Redirect::to(crate::routes::USERS),
    )
        .into_response()
}

/// **POST only** — see the route. Idempotent, which is what a double submission and a stale
/// tab both look like.
pub async fn signout(State(state): State<AppState>) -> Response {
    (
        AppendHeaders([(header::SET_COOKIE, session::clear(state.secure_cookies))]),
        Redirect::to(crate::routes::SIGNIN),
    )
        .into_response()
}
