//! The HTTP surface: two pages, one partial, and three acts.
//!
//! Every act is a POST that changes one thing and then re-renders the region it changed. It
//! answers two kinds of caller from one handler: an htmx request gets the region back and
//! swaps it in place, and a plain form submission gets a redirect — the post/redirect/get that
//! stops a refresh from banning somebody twice.
//!
//! A **refusal** does not redirect. Nothing was changed, so there is nothing a refresh could
//! do twice, and rendering the page with the refusal on it keeps the words next to the control
//! that produced them.

use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::Utc;
use lc_identity::ability;
use lc_identity::actions::Action;
use lc_identity::bans::{Issue, Reason, Term};
use lc_identity::level::Level;
use maud::Markup;
use serde::Deserialize;
use uuid::Uuid;

use crate::AppState;
use crate::auth::Admin;
use crate::detail;
use crate::listing::{Listing, Params};
use crate::users;
use crate::views::user::{Detail, Notice};
use crate::views::{self, Head};

pub const SIGNIN: &str = "/signin";
pub const RETURN: &str = "/auth/return";
pub const SIGNOUT: &str = "/signout";
pub const USERS: &str = "/users";
pub const USER_ROWS: &str = "/users/rows";
const USER: &str = "/users/{id}";
const USER_LEVEL: &str = "/users/{id}/level";
const USER_BAN: &str = "/users/{id}/ban";
const USER_LIFT: &str = "/users/{id}/lift";

pub fn user_url(id: Uuid) -> String {
    format!("/users/{id}")
}

pub fn user_level_url(id: Uuid) -> String {
    format!("/users/{id}/level")
}

pub fn user_ban_url(id: Uuid) -> String {
    format!("/users/{id}/ban")
}

pub fn user_lift_url(id: Uuid) -> String {
    format!("/users/{id}/lift")
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::to(USERS) }))
        .route(SIGNIN, get(crate::auth::signin))
        .route(RETURN, get(crate::auth::ret))
        // **POST, not GET.** The session cookie is `SameSite=Lax`, which sends it on a
        // cross-site *top-level navigation* when the method is safe — so as a GET this was a
        // link on any page anywhere that signed you out of the console. Lax never sends a
        // cookie on a cross-site POST, so the method is the whole of the defence and no token
        // is needed. It also puts the route out of reach of anything that follows links on
        // its own: a prefetcher, a crawler, a scanner, a chat client unfurling a pasted URL.
        .route(SIGNOUT, post(crate::auth::signout))
        .route(USERS, get(index))
        // A static path beats `{id}` in the router, so this and `/users/{id}` coexist. They
        // are only reachable by the same method, which makes the ordering rule load-bearing
        // rather than incidental — see the note in `AGENTS.md`.
        .route(USER_ROWS, get(rows))
        .route(USER, get(person))
        .route(USER_LEVEL, post(set_level))
        .route(USER_BAN, post(issue_ban))
        .route(USER_LIFT, post(lift_ban))
        .route(crate::assets::ROUTE, get(crate::assets::serve))
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .fallback(not_found)
        .with_state(state)
}

/// What a completed act tells the page it redirected to.
///
/// A closed set carried in a query parameter, because the alternative to post/redirect/get is
/// a refresh that repeats the act. Unrecognised values render nothing, so an edited URL is
/// merely ineffective.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Done {
    LevelSet,
    Banned,
    Lifted,
}

impl Done {
    const ALL: [Done; 3] = [Done::LevelSet, Done::Banned, Done::Lifted];

    fn slug(self) -> &'static str {
        match self {
            Done::LevelSet => "level-set",
            Done::Banned => "banned",
            Done::Lifted => "lifted",
        }
    }

    fn said(self) -> &'static str {
        match self {
            Done::LevelSet => "Level changed.",
            Done::Banned => "Ban issued.",
            Done::Lifted => "Ban lifted.",
        }
    }

    fn from_slug(slug: &str) -> Option<Done> {
        Done::ALL.into_iter().find(|d| d.slug() == slug)
    }
}

#[derive(Deserialize, Default)]
struct PageQuery {
    #[serde(default)]
    done: Option<String>,
}

async fn index(
    State(state): State<AppState>,
    admin: Admin,
    Query(params): Query<Params>,
) -> Response {
    let listing = Listing::from_params(params);
    match load_page(&state, &listing).await {
        Ok(page) => views::shell(
            &state.assets,
            Head { title: "Users" },
            Some(&admin),
            views::index::page(&page, Utc::now()),
        )
        .into_response(),
        Err(response) => response,
    }
}

/// The index's table alone, for an htmx swap.
///
/// The same function the page renders, so the two cannot drift apart. It is reachable
/// directly, which is deliberate: a partial that only works as part of a swap is a partial
/// that cannot be looked at when it goes wrong.
async fn rows(
    State(state): State<AppState>,
    admin: Admin,
    Query(params): Query<Params>,
) -> Response {
    let _ = &admin;
    let listing = Listing::from_params(params);
    match load_page(&state, &listing).await {
        Ok(page) => views::index::region(&page, Utc::now()).into_response(),
        Err(response) => response,
    }
}

// A rendered `Response` as the error is the axum convention and what every caller here wants
// to do with a failure: return it. Boxing it to satisfy a size heuristic would put a
// dereference at four call sites and buy nothing.
#[allow(clippy::result_large_err)]
async fn load_page(state: &AppState, listing: &Listing) -> Result<users::Page, Response> {
    users::page(&state.pool, listing, Utc::now())
        .await
        .map_err(|why| {
            tracing::error!(%why, "the user index query failed");
            views::wrong(
                &state.assets,
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "The account store did not answer.",
            )
        })
}

async fn person(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Response {
    let detail = match load_detail(&state, id).await {
        Ok(detail) => detail,
        Err(response) => return response,
    };
    let notice = query
        .done
        .as_deref()
        .and_then(Done::from_slug)
        .map(|done| Notice::done(done.said()));
    page_for(&state, &admin, &detail, notice.as_ref())
}

#[allow(clippy::result_large_err)]
async fn load_detail(state: &AppState, id: Uuid) -> Result<Detail, Response> {
    let store = state.store();
    let failed = |why: String| {
        tracing::error!(%why, "could not assemble a user page");
        views::wrong(
            &state.assets,
            StatusCode::INTERNAL_SERVER_ERROR,
            "Something went wrong",
            "The account store did not answer.",
        )
    };

    let account = match store.account(id).await {
        Ok(Some(account)) => account,
        Ok(None) => {
            return Err(views::refusal(
                &state.assets,
                StatusCode::NOT_FOUND,
                "No such account",
                "There is no account with that identifier.",
                (USERS, "← All users"),
            ));
        }
        Err(why) => return Err(failed(why.to_string())),
    };
    let now = Utc::now();
    Ok(Detail {
        sanction: store
            .sanction(id, now)
            .await
            .map_err(|e| failed(e.to_string()))?,
        bans: store
            .bans_for(id)
            .await
            .map_err(|e| failed(e.to_string()))?,
        log: store
            .actions_for(id, LOG_DEPTH)
            .await
            .map_err(|e| failed(e.to_string()))?,
        links: detail::links_for(&state.pool, id)
            .await
            .map_err(|e| failed(e.to_string()))?,
        grants: detail::grants_for(&state.pool, id)
            .await
            .map_err(|e| failed(e.to_string()))?,
        account,
    })
}

/// How much history a user page shows.
///
/// Enough to read the shape of an account's dealings with the administration, bounded so a
/// long-running argument does not render a page megabytes long.
const LOG_DEPTH: i64 = 50;

fn page_for(state: &AppState, admin: &Admin, detail: &Detail, notice: Option<&Notice>) -> Response {
    views::shell(
        &state.assets,
        Head {
            title: &detail.account.display_name,
        },
        Some(admin),
        views::user::page(admin, detail, notice, Utc::now()),
    )
    .into_response()
}

/// How an act answers.
///
/// htmx gets the region; a browser gets a redirect, so a refresh does not repeat the act. A
/// refusal is neither — see the module note — and comes back as the page it was refused on,
/// with the status that says so.
async fn answered(
    state: &AppState,
    admin: &Admin,
    headers: &HeaderMap,
    id: Uuid,
    outcome: Result<Done, Notice>,
) -> Response {
    let htmx = headers.contains_key("hx-request");
    match outcome {
        Ok(done) if !htmx => {
            Redirect::to(&format!("{}?done={}", user_url(id), done.slug())).into_response()
        }
        outcome => {
            let detail = match load_detail(state, id).await {
                Ok(detail) => detail,
                Err(response) => return response,
            };
            let (status, notice) = match outcome {
                Ok(done) => (StatusCode::OK, Notice::done(done.said())),
                Err(notice) => (StatusCode::UNPROCESSABLE_ENTITY, notice),
            };
            if htmx {
                // htmx 4 swaps every status but 204 and 304, so a refusal's body lands where
                // the success would have. That is the behaviour wanted here and it is a change
                // from htmx 2, where a 422 would have been dropped and the page would have sat
                // there saying nothing.
                return (
                    status,
                    render(views::user::region(
                        admin,
                        &detail,
                        Some(&notice),
                        Utc::now(),
                    )),
                )
                    .into_response();
            }
            (status, page_for(state, admin, &detail, Some(&notice))).into_response()
        }
    }
}

fn render(markup: Markup) -> Response {
    markup.into_response()
}

#[derive(Deserialize)]
struct LevelForm {
    level: String,
}

async fn set_level(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Form(form): Form<LevelForm>,
) -> Response {
    let store = state.store();
    let Ok(Some(account)) = store.account(id).await else {
        return views::refusal(
            &state.assets,
            StatusCode::NOT_FOUND,
            "No such account",
            "There is no account with that identifier.",
            (USERS, "← All users"),
        );
    };
    let Some(proposed) = Level::from_slug(&form.level) else {
        return answered(
            &state,
            &admin,
            &headers,
            id,
            Err(Notice::refused("That is not a level.")),
        )
        .await;
    };

    let was = account.level();
    // **The check, and the only one.** Re-read here rather than trusted from the form that
    // offered it: the page was rendered at some earlier moment, and the acting administrator
    // may have been demoted since.
    let outcome = match ability::may_set_level(admin.level, admin.id, was, id, proposed) {
        Err(denied) => Err(Notice::refused(denied.said())),
        Ok(()) => match store.set_permission(id, proposed.as_i32()).await {
            Err(why) => {
                tracing::error!(%why, "could not set a level");
                Err(Notice::refused(
                    "The account store did not answer. Nothing changed.",
                ))
            }
            Ok(()) => {
                let action = if proposed.outranks(was) {
                    Action::Promoted
                } else {
                    Action::Demoted
                };
                // Best effort. The change happened; reporting it as failed because the log
                // write did would leave an administrator retrying an act that already took.
                if let Err(why) = store
                    .record(
                        admin.id,
                        id,
                        action,
                        &format!("{was} to {proposed}"),
                        Utc::now(),
                    )
                    .await
                {
                    tracing::error!(%why, subject = %id, "a level change went unlogged");
                }
                tracing::info!(actor = %admin.id, subject = %id, %was, %proposed, "level changed");
                Ok(Done::LevelSet)
            }
        },
    };
    answered(&state, &admin, &headers, id, outcome).await
}

#[derive(Deserialize)]
struct BanForm {
    reason: String,
    term: String,
    #[serde(default)]
    notes: String,
}

/// Longest private note kept. Long enough for a case, bounded because it is a text column
/// somebody can paste a log file into.
const MAX_NOTES: usize = 4000;

async fn issue_ban(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Form(form): Form<BanForm>,
) -> Response {
    let store = state.store();
    let Ok(Some(account)) = store.account(id).await else {
        return views::refusal(
            &state.assets,
            StatusCode::NOT_FOUND,
            "No such account",
            "There is no account with that identifier.",
            (USERS, "← All users"),
        );
    };
    let (Some(reason), Some(term)) = (Reason::from_slug(&form.reason), Term::from_slug(&form.term))
    else {
        return answered(
            &state,
            &admin,
            &headers,
            id,
            Err(Notice::refused("That is not a reason and a term.")),
        )
        .await;
    };

    let now = Utc::now();
    let outcome = match ability::may_ban(admin.level, account.level()) {
        Err(denied) => Err(Notice::refused(denied.said())),
        Ok(()) => {
            let issue = Issue {
                account_id: id,
                reason,
                notes: form.notes.chars().take(MAX_NOTES).collect(),
                by: admin.id,
                until: term.until(now),
            };
            match store.issue_ban(&issue, now).await {
                Err(why) => {
                    tracing::error!(%why, "could not issue a ban");
                    Err(Notice::refused(
                        "The account store did not answer. Nothing changed.",
                    ))
                }
                Ok(ban) => {
                    if let Err(why) = store
                        .record(
                            admin.id,
                            id,
                            Action::Banned,
                            &format!("{} — {}", term.label(), reason.said()),
                            now,
                        )
                        .await
                    {
                        tracing::error!(%why, subject = %id, "a ban went unlogged");
                    }
                    tracing::info!(
                        actor = %admin.id, subject = %id, %ban,
                        reason = reason.slug(), term = term.slug(), "ban issued",
                    );
                    Ok(Done::Banned)
                }
            }
        }
    };
    answered(&state, &admin, &headers, id, outcome).await
}

#[derive(Deserialize)]
struct LiftForm {
    ban: Uuid,
    #[serde(default)]
    notes: String,
}

async fn lift_ban(
    State(state): State<AppState>,
    admin: Admin,
    Path(id): Path<Uuid>,
    headers: HeaderMap,
    Form(form): Form<LiftForm>,
) -> Response {
    let store = state.store();
    let now = Utc::now();
    let outcome = match ability::may_lift(admin.level) {
        Err(denied) => Err(Notice::refused(denied.said())),
        // The account id goes to the store with the ban id, so a ban belonging to somebody
        // else cannot be lifted by putting its identifier in this form.
        Ok(()) => match store
            .lift_ban(
                form.ban,
                id,
                admin.id,
                &form.notes.chars().take(MAX_NOTES).collect::<String>(),
                now,
            )
            .await
        {
            Err(why) => {
                tracing::error!(%why, "could not lift a ban");
                Err(Notice::refused(
                    "The account store did not answer. Nothing changed.",
                ))
            }
            Ok(false) => Err(Notice::refused(
                "That ban is not this account's, or was lifted already.",
            )),
            Ok(true) => {
                if let Err(why) = store
                    .record(admin.id, id, Action::Lifted, &form.notes, now)
                    .await
                {
                    tracing::error!(%why, subject = %id, "a lift went unlogged");
                }
                tracing::info!(actor = %admin.id, subject = %id, ban = %form.ban, "ban lifted");
                Ok(Done::Lifted)
            }
        },
    };
    answered(&state, &admin, &headers, id, outcome).await
}

/// Readiness. This service has no degraded mode: without the database there is nothing to
/// administer and nothing to sign in against.
async fn readyz(State(state): State<AppState>) -> Response {
    match sqlx::query("select 1").execute(&state.pool).await {
        Ok(_) => "ready".into_response(),
        Err(why) => {
            tracing::warn!(%why, "readiness probe failed");
            (StatusCode::SERVICE_UNAVAILABLE, "database unreachable").into_response()
        }
    }
}

async fn not_found(State(state): State<AppState>) -> Response {
    views::refusal(
        &state.assets,
        StatusCode::NOT_FOUND,
        "No page here",
        "The address is wrong, or the page has moved.",
        (USERS, "← All users"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_identity::ability::Denied;

    /// A slug is a promise to every URL that already carries it.
    #[test]
    fn the_outcome_names_round_trip_and_are_distinct() {
        let mut slugs: Vec<&str> = Done::ALL.iter().map(|d| d.slug()).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), Done::ALL.len());
        for done in Done::ALL {
            assert_eq!(Done::from_slug(done.slug()), Some(done));
        }
        // Anything else renders nothing rather than anything alarming.
        assert_eq!(Done::from_slug("deleted-everything"), None);
    }

    /// The URL builders and the route patterns have to describe the same paths, or a link
    /// renders to a 404 that nothing catches until somebody clicks it.
    #[test]
    fn every_built_url_matches_its_route() {
        let id = Uuid::parse_str("11111111-1111-4111-8111-111111111111").unwrap();
        for (built, pattern) in [
            (user_url(id), USER),
            (user_level_url(id), USER_LEVEL),
            (user_ban_url(id), USER_BAN),
            (user_lift_url(id), USER_LIFT),
        ] {
            assert_eq!(built, pattern.replace("{id}", &id.to_string()), "{pattern}");
        }
        // And the partial is below the index, so the static path wins over `{id}` in the
        // router. If these ever diverge, `/users/rows` starts being read as an account id.
        assert!(USER_ROWS.starts_with(USERS));
        assert_eq!(USER.replace("{id}", "rows"), USER_ROWS);
    }

    /// Every refusal the rules can produce has to be something this layer can show. A
    /// `Denied` with no sentence would reach a page as an empty notice.
    #[test]
    fn every_refusal_has_something_to_say() {
        for denied in Denied::ALL {
            let notice = Notice::refused(denied.said());
            assert!(!notice.said.is_empty(), "{denied:?}");
            assert_eq!(notice.kind, "refused");
        }
    }
}
