//! The console, driven through its router against a real database.
//!
//! **These skip when PostgreSQL cannot be reached**, the way `lc-store`'s do, so the suite
//! passes on a machine without one. `createdb lc_admin_test` is enough; `LC_ADMIN_TEST_URL`
//! overrides where it looks. The schema comes from `lc_identity::schema::apply`, which is the
//! same function the broker runs at boot — a hand-written approximation of it here would be a
//! second definition, and tests that pass against a shape nothing in production has.
//!
//! Each test works on its own accounts, created fresh, so they can run in any order against
//! one database.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use chrono::Utc;
use http_body_util::BodyExt;
use lc_admin::AppState;
use lc_admin::assets::Assets;
use lc_admin::routes::router;
use lc_admin::session::{self, Session};
use lc_identity::level::Level;
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const KEY: &str = "a test key of at least thirty-two characters";

/// A pool, or `None` when there is no database to reach.
async fn pool() -> Option<PgPool> {
    let url = std::env::var("LC_ADMIN_TEST_URL")
        .unwrap_or_else(|_| "postgres://localhost/lc_admin_test".into());
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(std::time::Duration::from_secs(2))
        .connect(&url)
        .await
        .ok()?;
    lc_identity::schema::apply(&pool)
        .await
        .expect("the broker's migrations apply");
    Some(pool)
}

/// The skip, said out loud. A test that silently does nothing is a test that goes on doing
/// nothing after the thing it covers breaks.
macro_rules! with_pool {
    ($name:ident) => {
        match pool().await {
            Some(pool) => pool,
            None => {
                eprintln!(
                    "skipping {}: no PostgreSQL at LC_ADMIN_TEST_URL (createdb lc_admin_test)",
                    stringify!($name),
                );
                return;
            }
        }
    };
}

fn state(pool: PgPool) -> AppState {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
    AppState {
        assets: Assets::load(&root).expect("the assets load; run `npm run build`"),
        pool,
        session_key: KEY.into(),
        identity_base: "https://accounts.example".into(),
        identity_api: "https://accounts.example".into(),
        identity_secret: "shared".into(),
        return_url: "https://admin.example/auth/return".into(),
        secure_cookies: true,
        http: lc_admin::http_client(),
    }
}

async fn account(pool: &PgPool, name: &str, level: Level) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("insert into accounts (id, display_name, permission) values ($1, $2, $3)")
        .bind(id)
        .bind(name)
        .bind(level.as_i32())
        .execute(pool)
        .await
        .expect("an account");
    id
}

/// A signed cookie for `id`, as a completed sign-in would leave.
fn cookie(id: Uuid) -> String {
    let sealed = session::seal(
        KEY.as_bytes(),
        &Session {
            sub: id.to_string(),
            name: "Tester".into(),
            exp: Utc::now().timestamp() + session::LIFETIME_S,
        },
    );
    format!("{}={sealed}", session::COOKIE)
}

fn get(uri: &str, as_who: Option<Uuid>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(id) = as_who {
        builder = builder.header("cookie", cookie(id));
    }
    builder.body(Body::empty()).unwrap()
}

fn post(uri: &str, as_who: Uuid, fields: &[(&str, &str)], htmx: bool) -> Request<Body> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("cookie", cookie(as_who))
        .header("content-type", "application/x-www-form-urlencoded");
    if htmx {
        builder = builder.header("hx-request", "true");
    }
    builder.body(Body::from(body)).unwrap()
}

async fn send(app: &axum::Router, request: Request<Body>) -> Response {
    app.clone().oneshot(request).await.unwrap()
}

async fn text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn level_of(pool: &PgPool, id: Uuid) -> Level {
    let (permission,): (i32,) = sqlx::query_as("select permission from accounts where id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("the account");
    Level::from_stored(permission)
}

/// Nobody reaches anything without a session, and a player with one reaches nothing either.
#[tokio::test]
async fn the_console_is_shut_to_everybody_but_administrators() {
    let pool = with_pool!(the_console_is_shut_to_everybody_but_administrators);
    let app = router(state(pool.clone()));
    let player = account(&pool, "Player", Level::PLAYER).await;

    for path in ["/users", "/users/rows"] {
        // No cookie at all.
        let response = send(&app, get(path, None)).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "{path}");
        assert_eq!(
            response.headers()[axum::http::header::LOCATION],
            "/signin",
            "{path}",
        );

        // Signed in, and not an administrator.
        let response = send(&app, get(path, Some(player))).await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }

    // A user page is administration too. A player cannot read their own.
    let response = send(&app, get(&format!("/users/{player}"), Some(player))).await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

/// An htmx request gets a header it can act on rather than a redirect it would swap into the
/// page. A session expiring mid-session is the ordinary way to arrive here.
#[tokio::test]
async fn an_expired_htmx_request_is_told_to_navigate() {
    let pool = with_pool!(an_expired_htmx_request_is_told_to_navigate);
    let app = router(state(pool));
    let response = send(
        &app,
        Request::builder()
            .method("GET")
            .uri("/users/rows")
            .header("hx-request", "true")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["hx-redirect"], "/signin");
}

/// The partial and the page render the same table, and the partial is the swappable region on
/// its own — not a whole document.
#[tokio::test]
async fn the_partial_is_the_region_and_the_page_contains_it() {
    let pool = with_pool!(the_partial_is_the_region_and_the_page_contains_it);
    let app = router(state(pool.clone()));
    let admin = account(&pool, "Owner", Level::OWNER).await;
    let needle = format!("Findable {}", Uuid::new_v4());
    account(&pool, &needle, Level::PLAYER).await;

    let query = format!(
        "/users/rows?q={}",
        url::form_urlencoded::byte_serialize(needle.as_bytes()).collect::<String>(),
    );
    let partial = text(send(&app, get(&query, Some(admin))).await).await;
    assert!(partial.contains(&needle), "{partial}");
    assert!(
        partial.starts_with("<section id=\"user-index\""),
        "{partial}"
    );
    assert!(
        !partial.contains("<!DOCTYPE"),
        "the partial is a whole page"
    );

    let page = text(send(&app, get(&query.replace("/rows", ""), Some(admin))).await).await;
    assert!(page.contains("<!DOCTYPE"), "the page is not a document");
    assert!(
        page.contains("data-listing="),
        "the browser has no configuration"
    );
    // The same region, rendered by the same function.
    assert!(page.contains(&partial), "the page and the partial disagree");
}

/// Paging happens in the database, and the pages fit together: no row is on two of them and
/// none is on neither. The tie-break in `Listing::order_by` is what makes that true.
#[tokio::test]
async fn the_pages_partition_the_matches() {
    let pool = with_pool!(the_pages_partition_the_matches);
    let app = router(state(pool.clone()));
    let admin = account(&pool, "Owner", Level::OWNER).await;

    // A batch that shares a name prefix and a creation instant, which is the case a
    // tie-break-free ordering gets wrong.
    let tag = Uuid::new_v4().to_string();
    let mut made = Vec::new();
    for n in 0..30 {
        made.push(account(&pool, &format!("Crowd {tag} {n:02}"), Level::PLAYER).await);
    }

    let mut seen = Vec::new();
    for page in 1..=2 {
        let uri = format!("/users/rows?q={tag}&sort=created&per=25&page={page}");
        let markup = text(send(&app, get(&uri, Some(admin))).await).await;
        for id in &made {
            if markup.contains(&id.to_string()) {
                seen.push(*id);
            }
        }
    }
    seen.sort();
    let mut expected = made.clone();
    expected.sort();
    assert_eq!(seen.len(), 30, "a row was on two pages or on none");
    assert_eq!(seen, expected);
}

/// The whole promotion table, exercised through the router rather than against the rules
/// directly: what is asserted here is that the check is actually *wired in*.
#[tokio::test]
async fn the_level_rules_hold_through_the_router() {
    let pool = with_pool!(the_level_rules_hold_through_the_router);
    let app = router(state(pool.clone()));
    let admin = account(&pool, "Admin", Level::ADMIN).await;
    let subject = account(&pool, "Subject", Level::PLAYER).await;

    // Up to their own level: allowed, and it takes.
    let response = send(
        &app,
        post(
            &format!("/users/{subject}/level"),
            admin,
            &[("level", "admin")],
            false,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(level_of(&pool, subject).await, Level::ADMIN);

    // Above their own level: refused, and nothing changed.
    let response = send(
        &app,
        post(
            &format!("/users/{subject}/level"),
            admin,
            &[("level", "owner")],
            true,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = text(response).await;
    assert!(body.contains("above your own"), "{body}");
    assert_eq!(level_of(&pool, subject).await, Level::ADMIN);

    // An equal: refused. This is the one a careless `>=` gets wrong.
    let response = send(
        &app,
        post(
            &format!("/users/{subject}/level"),
            admin,
            &[("level", "moderator")],
            true,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(response).await.contains("below your own level"));
    assert_eq!(level_of(&pool, subject).await, Level::ADMIN);

    // Their own account: refused.
    let response = send(
        &app,
        post(
            &format!("/users/{admin}/level"),
            admin,
            &[("level", "moderator")],
            true,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(response).await.contains("your own level"));
    assert_eq!(level_of(&pool, admin).await, Level::ADMIN);
}

/// A ban issued here reaches the broker's sign-in path, because both read one table through
/// one type. This is the join between the two services, and it is the thing most worth an
/// assertion that crosses them.
#[tokio::test]
async fn a_ban_issued_here_is_a_ban_the_broker_enforces() {
    let pool = with_pool!(a_ban_issued_here_is_a_ban_the_broker_enforces);
    let app = router(state(pool.clone()));
    let admin = account(&pool, "Admin", Level::ADMIN).await;
    // A name nothing else shares, so the index assertions below can narrow to this account.
    // Without that they ask page one of every banned account in the database and pass only
    // while that page is short — which is to say they pass on a fresh database and start
    // failing, for no reason connected to what they are about, once the suite has run enough
    // times to fill a page.
    let name = format!("Subject {}", Uuid::new_v4());
    let subject = account(&pool, &name, Level::PLAYER).await;
    let store = lc_identity::store::Store::Postgres(pool.clone());

    assert_eq!(store.sanction(subject, Utc::now()).await.unwrap(), None);

    let response = send(
        &app,
        post(
            &format!("/users/{subject}/ban"),
            admin,
            &[
                ("reason", "cheating"),
                ("term", "2mo"),
                ("notes", "the private case notes"),
            ],
            true,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let sanction = store
        .sanction(subject, Utc::now())
        .await
        .unwrap()
        .expect("the broker sees the ban");
    assert_eq!(sanction.count, 1);
    assert!(
        sanction.until.is_some(),
        "a two-month ban reads as permanent"
    );

    // A second, concurrent. The furthest expiry is what the broker will report.
    send(
        &app,
        post(
            &format!("/users/{subject}/ban"),
            admin,
            &[("reason", "spam"), ("term", "3h"), ("notes", "")],
            true,
        ),
    )
    .await;
    let both = store.sanction(subject, Utc::now()).await.unwrap().unwrap();
    assert_eq!(both.count, 2);
    assert_eq!(
        both.until, sanction.until,
        "the shorter ban shortened the longer"
    );

    // And the index knows. `standing=clear` must not find them.
    let only_this = |standing: &str| {
        format!(
            "/users/rows?standing={standing}&q={}",
            url::form_urlencoded::byte_serialize(name.as_bytes()).collect::<String>(),
        )
    };
    let banned = text(send(&app, get(&only_this("banned"), Some(admin))).await).await;
    assert!(
        banned.contains(&subject.to_string()),
        "not in the banned list"
    );
    let clear = text(send(&app, get(&only_this("clear"), Some(admin))).await).await;
    assert!(
        !clear.contains(&subject.to_string()),
        "banned and also clear"
    );
}

/// An administrator cannot be banned, and the refusal happens at the act rather than only at
/// the control that offers it — a form can be submitted without ever being rendered.
#[tokio::test]
async fn an_administrator_cannot_be_banned_even_by_posting_the_form() {
    let pool = with_pool!(an_administrator_cannot_be_banned_even_by_posting_the_form);
    let app = router(state(pool.clone()));
    let owner = account(&pool, "Owner", Level::OWNER).await;
    let moderator = account(&pool, "Moderator", Level::MODERATOR).await;

    let response = send(
        &app,
        post(
            &format!("/users/{moderator}/ban"),
            owner,
            &[("reason", "spam"), ("term", "1w"), ("notes", "")],
            true,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text(response).await.contains("Remove their level first"));

    let store = lc_identity::store::Store::Postgres(pool.clone());
    assert_eq!(store.sanction(moderator, Utc::now()).await.unwrap(), None);

    // De-admin first, and then it works. The rule is an ordering, not a prohibition.
    send(
        &app,
        post(
            &format!("/users/{moderator}/level"),
            owner,
            &[("level", "player")],
            true,
        ),
    )
    .await;
    let response = send(
        &app,
        post(
            &format!("/users/{moderator}/ban"),
            owner,
            &[("reason", "spam"), ("term", "1w"), ("notes", "")],
            true,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        store
            .sanction(moderator, Utc::now())
            .await
            .unwrap()
            .is_some()
    );
}

/// Lifting takes a ban id, and a ban id belonging to another account must not be liftable
/// through this one's page.
#[tokio::test]
async fn a_ban_cannot_be_lifted_through_somebody_elses_page() {
    let pool = with_pool!(a_ban_cannot_be_lifted_through_somebody_elses_page);
    let app = router(state(pool.clone()));
    let admin = account(&pool, "Admin", Level::ADMIN).await;
    let theirs = account(&pool, "Theirs", Level::PLAYER).await;
    let other = account(&pool, "Other", Level::PLAYER).await;
    let store = lc_identity::store::Store::Postgres(pool.clone());

    send(
        &app,
        post(
            &format!("/users/{theirs}/ban"),
            admin,
            &[("reason", "spam"), ("term", "forever"), ("notes", "")],
            true,
        ),
    )
    .await;
    let ban = store.bans_for(theirs).await.unwrap()[0].id;

    // Through the wrong account: refused, and the ban stands.
    let response = send(
        &app,
        post(
            &format!("/users/{other}/lift"),
            admin,
            &[("ban", &ban.to_string()), ("notes", "not mine to lift")],
            true,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(store.sanction(theirs, Utc::now()).await.unwrap().is_some());

    // Through the right one: lifted, and the account is admitted again.
    let response = send(
        &app,
        post(
            &format!("/users/{theirs}/lift"),
            admin,
            &[("ban", &ban.to_string()), ("notes", "appealed")],
            true,
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(store.sanction(theirs, Utc::now()).await.unwrap(), None);
}

/// Every act leaves a line, and the user page shows it. A promotion nothing recorded is a
/// power that appeared from nowhere.
#[tokio::test]
async fn every_act_is_logged_on_the_user_page() {
    let pool = with_pool!(every_act_is_logged_on_the_user_page);
    let app = router(state(pool.clone()));
    let owner = account(&pool, "Owner", Level::OWNER).await;
    let subject = account(&pool, "Subject", Level::PLAYER).await;

    for fields in [vec![("level", "moderator")], vec![("level", "player")]] {
        send(
            &app,
            post(&format!("/users/{subject}/level"), owner, &fields, true),
        )
        .await;
    }
    send(
        &app,
        post(
            &format!("/users/{subject}/ban"),
            owner,
            &[("reason", "evasion"), ("term", "1d"), ("notes", "")],
            true,
        ),
    )
    .await;

    let page = text(send(&app, get(&format!("/users/{subject}"), Some(owner))).await).await;
    assert!(page.contains("Promoted"), "{page}");
    assert!(page.contains("Player to Moderator"), "{page}");
    assert!(page.contains("Demoted"), "{page}");
    assert!(page.contains("Banned"), "{page}");
    assert!(page.contains("1 day"), "{page}");
    assert!(page.contains("by Owner"), "the actor is missing: {page}");
}

/// The private notes are on the console and nowhere else. The one place they could leak is a
/// page the account itself can reach, and there is no such page — asserted by there being no
/// route that renders a ban without the `Admin` extractor in front of it.
#[tokio::test]
async fn ban_notes_reach_the_console_and_no_unauthenticated_caller() {
    let pool = with_pool!(ban_notes_reach_the_console_and_no_unauthenticated_caller);
    let app = router(state(pool.clone()));
    let admin = account(&pool, "Admin", Level::ADMIN).await;
    let subject = account(&pool, "Subject", Level::PLAYER).await;
    let secret = format!("case {}", Uuid::new_v4());

    send(
        &app,
        post(
            &format!("/users/{subject}/ban"),
            admin,
            &[("reason", "fraud"), ("term", "1y"), ("notes", &secret)],
            true,
        ),
    )
    .await;

    let page = text(send(&app, get(&format!("/users/{subject}"), Some(admin))).await).await;
    assert!(
        page.contains(&secret),
        "an administrator cannot read the notes"
    );

    // Signed out, and signed in as the account itself.
    let out = send(&app, get(&format!("/users/{subject}"), None)).await;
    assert_eq!(out.status(), StatusCode::SEE_OTHER);
    assert!(!text(out).await.contains(&secret));

    let theirs = send(&app, get(&format!("/users/{subject}"), Some(subject))).await;
    assert_eq!(theirs.status(), StatusCode::FORBIDDEN);
    assert!(!text(theirs).await.contains(&secret));
}

/// A URL somebody edited by hand is answered rather than refused, and the answer is the
/// default view. Nothing in the query string is trusted further than that.
#[tokio::test]
async fn a_hand_edited_query_string_is_answered() {
    let pool = with_pool!(a_hand_edited_query_string_is_answered);
    let app = router(state(pool.clone()));
    let admin = account(&pool, "Owner", Level::OWNER).await;

    // Encoded as a browser would encode them, so what is being tested is what the handler
    // does with the values rather than what `http::Uri` does with the characters.
    for (key, value) in [
        ("sort", "name'; drop table accounts; --"),
        ("dir", "sideways"),
        ("per", "100000"),
        ("page", "-4"),
        ("level", "god"),
        ("standing", "maybe"),
        // The two `like` wildcards, which must be searched for rather than obeyed.
        ("q", "%"),
        ("q", "_"),
        ("q", "' or 1=1 --"),
    ] {
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair(key, value)
            .finish();
        let response = send(&app, get(&format!("/users/rows?{query}"), Some(admin))).await;
        assert_eq!(response.status(), StatusCode::OK, "{key}={value}");
    }

    // And the table is still there, which is the part the first of those was about.
    let (count,): (i64,) = sqlx::query_as("select count(*) from accounts")
        .fetch_one(&pool)
        .await
        .expect("the accounts table survived");
    assert!(count > 0);
}

/// The assets are served, once, under a URL that changes when they do.
#[tokio::test]
async fn the_assets_are_served_and_cacheable_forever() {
    let pool = with_pool!(the_assets_are_served_and_cacheable_forever);
    let state = state(pool);
    let url = state.assets.url(lc_admin::assets::STYLESHEET);
    let app = router(state);

    let response = send(&app, get(&url, None)).await;
    assert_eq!(response.status(), StatusCode::OK, "{url}");
    assert_eq!(
        response.headers()[axum::http::header::CACHE_CONTROL],
        "public, max-age=31536000, immutable",
    );
    // No session needed: a stylesheet is not administration.
    assert!(text(response).await.contains("--accent"));

    // Anything else under the version prefix is not there.
    let response = send(&app, get("/v/anything/styles/../../etc/passwd", None)).await;
    assert_ne!(response.status(), StatusCode::OK);
}

/// A type-level note, not a runtime one: `Arc<str>` is what the state holds, and this fails to
/// compile if that changes under a refactor that also changes what the tests build.
#[test]
fn the_state_holds_shared_strings() {
    let _: Arc<str> = KEY.into();
}
