//! A ban, driven through the router rather than the store.
//!
//! The store's own tests say a sanction is computed correctly. These say it is *consulted* —
//! on the form, on the desktop endpoint, at the exchange the site makes, and at the ticket
//! that puts somebody in the game. The last is the one that matters most and the one a unit
//! test cannot reach: a session already issued outlives the ban by a fortnight, so a build
//! that checked only the sign-in form would look correct here and let every banned player who
//! was already signed in keep playing.
//!
//! Each test signs in successfully **first**, so a refusal afterwards is the ban and not a
//! typo in the fixture. A test asserting only "this was refused" passes just as well when
//! everything is refused.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

use lc_identity::attempts::Attempts;
use lc_identity::bans::{Issue, Reason, Term};
use lc_identity::config::Config;
use lc_identity::routes::{Broker, router};
use lc_identity::store::Store;
use lc_identity::ticket::Keys;

const RETURN_TO: &str = "https://lightcone.example/auth/return";
const SECRET: &str = "the-shared-secret";
const SHARD: &str = "shard-1";
const EMAIL: &str = "ada@example.test";
const PASSWORD: &str = "a good enough password";

fn broker(store: Store) -> Broker {
    let providers = lc_identity::providers::resolve("password", |_| None).expect("password only");
    Broker {
        config: Arc::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
            providers,
            return_to: vec![RETURN_TO.into()],
            loopback_paths: vec!["/return".into()],
            exchange_secret: SECRET.into(),
            audiences: vec![SHARD.into()],
            issuer: "https://accounts.lightcone.test".into(),
            public_url: None,
            signing_seed: None,
        }),
        store,
        attempts: Arc::new(Attempts::default()),
        keys: Arc::new(Keys::generate("https://accounts.lightcone.test").unwrap()),
        http: lc_identity::upstream::http_client(),
    }
}

async fn send(app: &axum::Router, request: Request<Body>) -> Response {
    app.clone().oneshot(request).await.unwrap()
}

/// The peer address the attempt budgets are keyed on.
///
/// Inserted by hand because `oneshot` drives the router without a listener, and `ConnectInfo`
/// is an extension a listener adds. Without it every request that carries a budget is a 500,
/// which looks exactly like a broken handler.
fn from_somewhere(mut request: Request<Body>) -> Request<Body> {
    let peer: SocketAddr = "127.0.0.1:49152".parse().unwrap();
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(peer));
    request
}

fn form(path: &str, fields: &[(&str, &str)]) -> Request<Body> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    from_somewhere(
        Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap(),
    )
}

fn json(path: &str, body: serde_json::Value, secret: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(secret) = secret {
        builder = builder.header("authorization", format!("Bearer {secret}"));
    }
    from_somewhere(builder.body(Body::from(body.to_string())).unwrap())
}

async fn text(response: Response) -> String {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A registered account and the code a completed sign-in just handed out.
async fn sign_in(app: &axum::Router) -> String {
    let response = send(
        app,
        form("/signin/password", &[
            ("return_to", RETURN_TO),
            ("state", "a-nonce"),
            ("email", EMAIL),
            ("password", PASSWORD),
        ]),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "the sign-in that was supposed to work did not",
    );
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    url::Url::parse(&location)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect::<HashMap<String, String>>()["code"]
        .clone()
}

async fn account(store: &Store) -> Uuid {
    lc_identity::signin::register_password(store, EMAIL, PASSWORD, "Ada")
        .await
        .expect("registered")
}

async fn ban(store: &Store, id: Uuid, term: Term) {
    let now = chrono::Utc::now();
    store
        .issue_ban(
            &Issue {
                account_id: id,
                reason: Reason::Cheating,
                notes: "case notes nobody but an administrator sees".into(),
                by: Uuid::new_v4(),
                until: term.until(now),
            },
            now,
        )
        .await
        .expect("the ban was written");
}

#[tokio::test]
async fn a_banned_account_is_refused_at_the_form_and_told_how_long() {
    let store = Store::memory();
    let id = account(&store).await;
    let app = router(broker(store.clone()));

    // It works first. Everything below is therefore the ban.
    sign_in(&app).await;

    ban(&store, id, Term::Months2).await;
    let response = send(
        &app,
        form("/signin/password", &[
            ("return_to", RETURN_TO),
            ("state", "a-nonce"),
            ("email", EMAIL),
            ("password", PASSWORD),
        ]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let page = text(response).await;
    assert!(page.contains("Account suspended"), "{page}");
    assert!(page.contains("It ends in"), "no length was given: {page}");
    // And the private notes are nowhere near it.
    assert!(
        !page.contains("case notes"),
        "a ban's private notes reached the sign-in page: {page}",
    );
}

/// The longest ban is the one reported, not the nearest and not the first written.
#[tokio::test]
async fn the_length_given_is_the_longest_ban() {
    let store = Store::memory();
    let id = account(&store).await;
    let app = router(broker(store.clone()));

    ban(&store, id, Term::Hours3).await;
    ban(&store, id, Term::Year).await;
    let response = send(
        &app,
        form("/signin/password", &[
            ("return_to", RETURN_TO),
            ("state", "a-nonce"),
            ("email", EMAIL),
            ("password", PASSWORD),
        ]),
    )
    .await;
    let page = text(response).await;
    assert!(
        page.contains("about 12 months") || page.contains("about 11 months"),
        "the three-hour ban was reported instead of the year: {page}",
    );
    assert!(page.contains("2 suspensions are in force"), "{page}");
}

/// The load-bearing one. A ban issued to somebody already signed in has to reach them, and the
/// only thing between a fortnight-old session and the game is this endpoint.
#[tokio::test]
async fn a_ban_stops_a_ticket_for_a_session_that_already_exists() {
    let store = Store::memory();
    let id = account(&store).await;
    let app = router(broker(store.clone()));

    let asking = serde_json::json!({ "account_id": id.to_string(), "audience": SHARD });
    let before = send(&app, json("/ticket", asking.clone(), Some(SECRET))).await;
    assert_eq!(
        before.status(),
        StatusCode::OK,
        "the ticket that was supposed to be minted was not",
    );

    ban(&store, id, Term::Forever).await;
    let after = send(&app, json("/ticket", asking, Some(SECRET))).await;
    assert_eq!(after.status(), StatusCode::FORBIDDEN);
    let body = text(after).await;
    assert!(body.contains("banned"), "{body}");
    assert!(body.contains("does not expire"), "{body}");
}

/// The site's exchange, which is what a code becomes a session through.
#[tokio::test]
async fn a_ban_landing_mid_flight_stops_the_exchange() {
    let store = Store::memory();
    let id = account(&store).await;
    let app = router(broker(store.clone()));

    // A code minted while clear, spent after the ban — the sixty-second window.
    let code = sign_in(&app).await;
    ban(&store, id, Term::Week).await;

    let response = send(
        &app,
        json(
            "/exchange",
            serde_json::json!({ "code": code, "return_to": RETURN_TO }),
            Some(SECRET),
        ),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_banned_account_gets_no_device_grant() {
    let store = Store::memory();
    let id = account(&store).await;
    let app = router(broker(store.clone()));

    let credentials = serde_json::json!({
        "email": EMAIL, "password": PASSWORD, "label": "Ada's laptop",
    });
    let before = send(&app, json("/signin/password/native", credentials.clone(), None)).await;
    assert_eq!(before.status(), StatusCode::OK, "the desktop sign-in failed");

    ban(&store, id, Term::Day).await;
    let after = send(&app, json("/signin/password/native", credentials, None)).await;
    assert_eq!(after.status(), StatusCode::FORBIDDEN);
    let body = text(after).await;
    assert!(body.contains("\"error\":\"banned\""), "{body}");
    assert!(body.contains("\"until\""), "no expiry was given: {body}");
}

/// Lifting a ban lets somebody back in, which is the half of the mechanism that is easy to
/// leave untested and expensive to have wrong.
#[tokio::test]
async fn lifting_the_only_ban_admits_the_account_again() {
    let store = Store::memory();
    let id = account(&store).await;
    let app = router(broker(store.clone()));

    ban(&store, id, Term::Forever).await;
    let bans = store.bans_for(id).await.unwrap();
    assert_eq!(bans.len(), 1);

    let blocked = send(
        &app,
        form("/signin/password", &[
            ("return_to", RETURN_TO),
            ("state", "a-nonce"),
            ("email", EMAIL),
            ("password", PASSWORD),
        ]),
    )
    .await;
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    store
        .lift_ban(
            bans[0].id,
            id,
            Uuid::new_v4(),
            "appealed",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    sign_in(&app).await;
}

/// A wrong password for a banned account is still a wrong password, and must not answer
/// differently from a wrong password for any other account. Otherwise the sign-in form becomes
/// a way to ask whether somebody is banned.
#[tokio::test]
async fn a_wrong_password_does_not_reveal_a_ban() {
    let store = Store::memory();
    let id = account(&store).await;
    let app = router(broker(store.clone()));
    ban(&store, id, Term::Forever).await;

    let wrong = |email: &str| {
        form("/signin/password", &[
            ("return_to", RETURN_TO),
            ("state", "a-nonce"),
            ("email", email),
            ("password", "not the password"),
        ])
    };
    let banned = send(&app, wrong(EMAIL)).await;
    let nobody = send(&app, wrong("nobody@example.test")).await;
    assert_eq!(banned.status(), nobody.status());

    let banned = text(banned).await;
    assert!(
        !banned.contains("Account suspended"),
        "a wrong password announced the ban: {banned}",
    );
    assert_eq!(banned, text(nobody).await);
}
