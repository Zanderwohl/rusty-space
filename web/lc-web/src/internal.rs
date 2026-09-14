//! The privileged endpoints: registering a build, promoting one, yanking one.
//!
//! Authentication is a shared secret in a header, compared in constant time. **This is the
//! first thing that moves behind the auth service**, along with the eventual account pages;
//! those two are the whole integration surface, which is the point of deferring it.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::releases::{self, NewRelease, PromoteError};

const TOKEN_HEADER: &str = "x-release-token";

/// Compares without leaking where two secrets first differ.
///
/// The difference is not measurable over a network in practice; it is one line, and the
/// alternative is explaining to the next reader why it was fine to skip.
fn authorised(state: &AppState, headers: &HeaderMap) -> bool {
    let Some(expected) = state.release_token.as_deref() else {
        return false;
    };
    let Some(given) = headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let (a, b) = (expected.as_bytes(), given.as_bytes());
    // Lengths are compared first and are not themselves secret.
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Why a privileged request was refused. A small type rather than a `Response`, so the happy
/// path does not carry a whole HTTP response around as its error variant.
pub enum Denied {
    Unauthorised,
    NoDatabase,
}

impl IntoResponse for Denied {
    fn into_response(self) -> Response {
        match self {
            // The same answer whether the token is wrong or unset, so probing learns nothing.
            Self::Unauthorised => (StatusCode::UNAUTHORIZED, "no").into_response(),
            Self::NoDatabase => {
                (StatusCode::SERVICE_UNAVAILABLE, "no database; release management is unavailable")
                    .into_response()
            }
        }
    }
}

fn guard(state: &AppState, headers: &HeaderMap) -> Result<sqlx::PgPool, Denied> {
    if !authorised(state, headers) {
        return Err(Denied::Unauthorised);
    }
    state.pool.clone().ok_or(Denied::NoDatabase)
}

pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(new): Json<NewRelease>,
) -> Response {
    let pool = match guard(&state, &headers) {
        Ok(p) => p,
        Err(denied) => return denied.into_response(),
    };
    match releases::register(&pool, &new).await {
        Ok(r) => (StatusCode::OK, Json(json!({"registered": r}))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "registering a release");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct Promotion {
    pub build_id: String,
}

pub async fn promote(
    State(state): State<AppState>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Promotion>,
) -> Response {
    let pool = match guard(&state, &headers) {
        Ok(p) => p,
        Err(denied) => return denied.into_response(),
    };
    match releases::promote(&pool, &channel, &body.build_id).await {
        Ok(c) => {
            tracing::info!(channel = %c.name, build = %c.build_id, "promoted");
            (StatusCode::OK, Json(json!({"promoted": c}))).into_response()
        }
        Err(e @ (PromoteError::Unknown | PromoteError::Yanked)) => {
            (StatusCode::CONFLICT, e.to_string()).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "promoting");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
        }
    }
}

#[derive(Deserialize)]
pub struct Yank {
    #[serde(default = "yes")]
    pub yanked: bool,
}

fn yes() -> bool {
    true
}

pub async fn yank(
    State(state): State<AppState>,
    Path(build_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Yank>,
) -> Response {
    let pool = match guard(&state, &headers) {
        Ok(p) => p,
        Err(denied) => return denied.into_response(),
    };
    match releases::set_yanked(&pool, &build_id, body.yanked).await {
        Ok(Some(r)) => (StatusCode::OK, Json(json!({"release": r}))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no such build").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// What is registered and what each channel points at.
///
/// Readable with the token, because it is operational rather than public: a list of builds
/// including yanked ones is a list of what went wrong and when.
pub async fn list(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let pool = match guard(&state, &headers) {
        Ok(p) => p,
        Err(denied) => return denied.into_response(),
    };
    let releases = releases::list(&pool).await.unwrap_or_default();
    let channels = releases::channels(&pool).await.unwrap_or_default();
    Json(json!({ "releases": releases, "channels": channels })).into_response()
}
