//! The upstream dance, walked end to end against a stub provider.
//!
//! The stub is not a mock that agrees with whatever it is sent. It **checks**: the client
//! credentials, the redirect URI, and — the point of the exercise — that the verifier presented
//! at the token endpoint hashes to the challenge it was given at the authorize endpoint. A PKCE
//! implementation that generated both halves and never compared them would pass a test that
//! only asserted the parameters were present, and would be broken.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use http_body_util::BodyExt;
use sha2::{Digest, Sha256};
use tower::ServiceExt;

use lc_identity::attempts::Attempts;
use lc_identity::config::Config;
use lc_identity::routes::{Broker, router};
use lc_identity::store::Store;
use lc_identity::ticket::Keys;

const ISSUER: &str = "https://stub-provider.test";
const CLIENT_ID: &str = "the-client-id";
const CLIENT_SECRET: &str = "the-client-secret";
const RETURN_TO: &str = "https://lightcone.example/auth/return";
const PUBLIC_URL: &str = "https://accounts.lightcone.test";

/// What the stub provider will say about whoever signs in, and what it has been asked so far.
#[derive(Clone, Default)]
struct Stub {
    /// state -> the PKCE challenge that state was started with.
    challenges: Arc<Mutex<HashMap<String, String>>>,
    /// What the id token should claim. Settable so one test can make it an unverified address.
    claims: Arc<Mutex<serde_json::Value>>,
    /// Every redirect_uri seen, so the two halves can be compared.
    redirect_uris: Arc<Mutex<Vec<String>>>,
}

async fn stub_authorize(
    State(stub): State<Stub>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["code_challenge_method"], "S256");
    let state = q["state"].clone();
    stub.challenges
        .lock()
        .unwrap()
        .insert(state.clone(), q["code_challenge"].clone());
    stub.redirect_uris
        .lock()
        .unwrap()
        .push(q["redirect_uri"].clone());
    Redirect::to(&format!(
        "{}?code=an-authorization-code&state={state}",
        q["redirect_uri"]
    ))
    .into_response()
}

async fn stub_token(State(stub): State<Stub>, Form(f): Form<HashMap<String, String>>) -> Response {
    assert_eq!(f["grant_type"], "authorization_code");
    assert_eq!(f["code"], "an-authorization-code");
    // The client secret goes to the token endpoint and nowhere a browser has been.
    assert_eq!(f["client_id"], CLIENT_ID);
    assert_eq!(f["client_secret"], CLIENT_SECRET);
    stub.redirect_uris
        .lock()
        .unwrap()
        .push(f["redirect_uri"].clone());

    // The check this whole file exists for.
    let offered = URL_SAFE_NO_PAD.encode(Sha256::digest(f["code_verifier"].as_bytes()));
    let issued: Vec<String> = stub.challenges.lock().unwrap().values().cloned().collect();
    assert!(
        issued.contains(&offered),
        "the verifier does not hash to any challenge issued: {offered} not in {issued:?}",
    );

    let claims = stub.claims.lock().unwrap().clone();
    axum::Json(serde_json::json!({
        "access_token": "an-access-token",
        "token_type": "Bearer",
        "id_token": unsigned_jwt(&claims),
    }))
    .into_response()
}

/// Unsigned, because the broker does not check the signature and says why — the token arrives
/// over a TLS channel it opened to the provider itself. See `upstream::exchange_code`.
fn unsigned_jwt(claims: &serde_json::Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("{header}.{payload}.")
}

fn default_claims() -> serde_json::Value {
    serde_json::json!({
        "iss": ISSUER,
        "aud": CLIENT_ID,
        "sub": "google-subject-1029384756",
        "email": "ada@example.test",
        "email_verified": true,
        "name": "Ada Lovelace",
        "exp": (chrono::Utc::now() + chrono::Duration::minutes(5)).timestamp(),
    })
}

/// Start the stub on a port the operating system picks, and return its base URL.
async fn serve_stub(stub: Stub) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new()
        .route("/authorize", get(stub_authorize))
        .route("/token", post(stub_token))
        .with_state(stub);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://127.0.0.1:{}", addr.port())
}

fn broker_for(base: &str, store: Store) -> Broker {
    let env: HashMap<String, String> = [
        ("GOOGLE_CLIENT_ID", CLIENT_ID.to_string()),
        ("GOOGLE_CLIENT_SECRET", CLIENT_SECRET.to_string()),
        ("GOOGLE_AUTH_URL", format!("{base}/authorize")),
        ("GOOGLE_TOKEN_URL", format!("{base}/token")),
        ("GOOGLE_ISSUER", ISSUER.to_string()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();

    // Through `resolve`, so the test exercises the configuration path a deployment takes.
    let providers =
        lc_identity::providers::resolve("password,google", |name| env.get(name).cloned())
            .expect("a configured google");

    Broker {
        config: Arc::new(Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
            providers,
            return_to: vec![RETURN_TO.into()],
            loopback_paths: vec!["/return".into()],
            exchange_secret: "shared".into(),
            audiences: vec!["shard-1".into()],
            issuer: "https://accounts.lightcone.test".into(),
            public_url: Some(PUBLIC_URL.into()),
            signing_seed: None,
        }),
        store,
        attempts: Arc::new(Attempts::default()),
        keys: Arc::new(Keys::generate("https://accounts.lightcone.test").unwrap()),
        http: lc_identity::upstream::http_client(),
    }
}

async fn fetch(app: &axum::Router, uri: &str) -> Response {
    app.clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(uri)
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

fn location(response: &Response) -> String {
    response
        .headers()
        .get(axum::http::header::LOCATION)
        .expect("a redirect")
        .to_str()
        .unwrap()
        .to_owned()
}

fn query_of(url: &str) -> HashMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

/// Play the browser: follow the redirect to the provider and hand back the request it bounces
/// the browser on to, as a path and query the broker's router can be given.
///
/// Going through the provider rather than fabricating a callback is what leaves the stub
/// holding a challenge to check the verifier against.
async fn bounce(at_provider: &str) -> String {
    let bounced = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .get(at_provider)
        .send()
        .await
        .unwrap();
    let back = bounced
        .headers()
        .get("location")
        .expect("the provider should redirect back")
        .to_str()
        .unwrap()
        .to_owned();
    assert!(
        back.starts_with(&format!("{PUBLIC_URL}/signin/google/callback")),
        "the provider was told to come back somewhere unexpected: {back}",
    );
    let parsed = url::Url::parse(&back).unwrap();
    format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default())
}

/// Walk the whole thing: the broker's sign-in page, out to the provider, back, and a sign-in
/// code the site can spend.
async fn walk(app: &axum::Router, _stub: &Stub) -> String {
    let started = fetch(
        app,
        &format!("/signin/google?return_to={RETURN_TO}&state=the-nonce"),
    )
    .await;
    assert!(started.status().is_redirection(), "{:?}", started.status());

    let finished = fetch(app, &bounce(&location(&started)).await).await;
    assert!(
        finished.status().is_redirection(),
        "the callback did not complete: {:?}",
        finished.status(),
    );

    let home = location(&finished);
    assert!(home.starts_with(RETURN_TO), "{home}");
    let params = query_of(&home);
    assert_eq!(
        params["state"], "the-nonce",
        "the caller's nonce did not survive the dance",
    );
    params["code"].clone()
}

async fn exchange(app: &axum::Router, code: &str) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/exchange")
                .header("authorization", "Bearer shared")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(
                    serde_json::json!({ "code": code, "return_to": RETURN_TO }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200, "the code would not exchange");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn a_google_sign_in_ends_in_an_account() {
    let stub = Stub {
        claims: Arc::new(Mutex::new(default_claims())),
        ..Stub::default()
    };
    let base = serve_stub(stub.clone()).await;
    let app = router(broker_for(&base, Store::memory()));

    let code = walk(&app, &stub).await;
    let identity = exchange(&app, &code).await;
    assert_eq!(identity["display_name"], "Ada Lovelace");
    assert!(
        identity["account_id"]
            .as_str()
            .unwrap()
            .parse::<uuid::Uuid>()
            .is_ok()
    );

    // The redirect URI the provider was given and the one the token request presented are the
    // same string. A mismatch is the most common way this flow fails in production.
    let seen = stub.redirect_uris.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "both halves should have named one");
    assert_eq!(seen[0], seen[1]);
    assert_eq!(seen[0], format!("{PUBLIC_URL}/signin/google/callback"));
}

/// Signing in twice is one account, not two. The `sub` claim is what makes that true.
#[tokio::test]
async fn signing_in_twice_is_the_same_account() {
    let stub = Stub {
        claims: Arc::new(Mutex::new(default_claims())),
        ..Stub::default()
    };
    let base = serve_stub(stub.clone()).await;
    let app = router(broker_for(&base, Store::memory()));

    let first = exchange(&app, &walk(&app, &stub).await).await;
    let again = exchange(&app, &walk(&app, &stub).await).await;
    assert_eq!(first["account_id"], again["account_id"]);
}

/// A callback is worth one visit. The second finds the flow spent and cannot complete.
#[tokio::test]
async fn a_callback_cannot_be_replayed() {
    let stub = Stub {
        claims: Arc::new(Mutex::new(default_claims())),
        ..Stub::default()
    };
    let base = serve_stub(stub.clone()).await;
    let app = router(broker_for(&base, Store::memory()));

    let started = fetch(
        &app,
        &format!("/signin/google?return_to={RETURN_TO}&state=the-nonce"),
    )
    .await;
    let callback = bounce(&location(&started)).await;

    assert!(fetch(&app, &callback).await.status().is_redirection());
    let replayed = fetch(&app, &callback).await;
    assert_eq!(
        replayed.status(),
        400,
        "a spent state completed a second time",
    );
}

/// A state nobody issued is not a sign-in, however well-formed it looks.
#[tokio::test]
async fn an_invented_state_completes_nothing() {
    let stub = Stub {
        claims: Arc::new(Mutex::new(default_claims())),
        ..Stub::default()
    };
    let base = serve_stub(stub.clone()).await;
    let app = router(broker_for(&base, Store::memory()));

    let forged = fetch(
        &app,
        "/signin/google/callback?code=an-authorization-code&state=a-state-nobody-issued",
    )
    .await;
    assert_eq!(forged.status(), 400);
}

/// Pressing Cancel at the provider is not an error page.
#[tokio::test]
async fn declining_at_the_provider_returns_to_the_sign_in_page() {
    let stub = Stub {
        claims: Arc::new(Mutex::new(default_claims())),
        ..Stub::default()
    };
    let base = serve_stub(stub.clone()).await;
    let app = router(broker_for(&base, Store::memory()));

    let started = fetch(
        &app,
        &format!("/signin/google?return_to={RETURN_TO}&state=the-nonce"),
    )
    .await;
    let state = query_of(&location(&started))["state"].clone();

    let declined = fetch(
        &app,
        &format!("/signin/google/callback?error=access_denied&state={state}"),
    )
    .await;
    assert_eq!(declined.status(), 200, "canceling should not be an error");
    let body = String::from_utf8(
        declined
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(body.contains("did not finish"), "{body}");
    // And the way back in is still on the page, with the destination intact.
    assert!(body.contains("the-nonce"), "the destination was lost");
}

/// A `return_to` that is not on the allowlist never reaches the provider at all.
#[tokio::test]
async fn an_unlisted_return_to_never_starts_a_dance() {
    let stub = Stub {
        claims: Arc::new(Mutex::new(default_claims())),
        ..Stub::default()
    };
    let base = serve_stub(stub.clone()).await;
    let app = router(broker_for(&base, Store::memory()));

    let refused = fetch(
        &app,
        "/signin/google?return_to=https://attacker.test/collect&state=the-nonce",
    )
    .await;
    assert_eq!(refused.status(), 400);
    assert!(
        stub.challenges.lock().unwrap().is_empty(),
        "a dance was started for somewhere we would not return to",
    );
}

/// The native client's loopback return works through an upstream provider unchanged. It is the
/// same `Destination` check, so this is the one thing that would make Google unusable from the
/// desktop build if it were not true.
#[tokio::test]
async fn a_loopback_return_to_works_through_the_provider() {
    let stub = Stub {
        claims: Arc::new(Mutex::new(default_claims())),
        ..Stub::default()
    };
    let base = serve_stub(stub.clone()).await;
    let app = router(broker_for(&base, Store::memory()));

    let started = fetch(
        &app,
        "/signin/google?return_to=http://127.0.0.1:49731/return&state=the-nonce",
    )
    .await;
    assert!(started.status().is_redirection());

    let finished = fetch(&app, &bounce(&location(&started)).await).await;
    assert!(finished.status().is_redirection());
    assert!(location(&finished).starts_with("http://127.0.0.1:49731/return?code="));
}

/// The alignment rule, through the real routes: an unverified password account does not hand
/// over anything to a Google sign-in on the same address.
#[tokio::test]
async fn a_squatted_address_does_not_become_the_google_account() {
    let stub = Stub {
        claims: Arc::new(Mutex::new(default_claims())),
        ..Stub::default()
    };
    let base = serve_stub(stub.clone()).await;
    let store = Store::memory();
    let squatter = lc_identity::signin::register_password(
        &store,
        "ada@example.test",
        "a good enough password",
        "Imposter",
    )
    .await
    .unwrap();

    let app = router(broker_for(&base, store));
    let identity = exchange(&app, &walk(&app, &stub).await).await;
    assert_ne!(identity["account_id"], squatter.to_string());
    assert_eq!(identity["display_name"], "Ada Lovelace");
}
