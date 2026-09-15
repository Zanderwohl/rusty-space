//! The HTTP surface: a form, a redirect, and one server-to-server exchange.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Form, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{Duration, Utc};
use maud::{DOCTYPE, Markup, html};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::attempts::{Attempts, PER_ACCOUNT, PER_ADDRESS};
use crate::config::{Config, is_allowed_loopback, is_allowed_return};
use crate::providers::Provider;
use crate::signin::{self, Refused};
use crate::store::Store;
use crate::ticket::Keys;

#[derive(Clone)]
pub struct Broker {
    pub config: Arc<Config>,
    pub store: Store,
    pub attempts: Arc<Attempts>,
    pub keys: Arc<Keys>,
}

pub fn router(broker: Broker) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/signin", get(page))
        .route("/signin/password", post(sign_in))
        .route("/signin/register", post(register))
        // The native pair. Same credentials, same budgets, no redirect — so no code either:
        // a code exists to survive a browser round trip and there is not one here.
        .route("/signin/password/native", post(native_signin))
        .route("/signin/register/native", post(native_register))
        .route("/exchange", post(exchange))
        .route("/ticket", post(ticket))
        .route("/grant", post(grant))
        .route("/.well-known/jwks.json", get(jwks))
        .with_state(broker)
}

/// Where a sign-in came from and where it goes back to. Carried through the form as hidden
/// fields and **re-checked on the way out**: a form is a thing the browser sends, not a thing
/// the broker remembers.
#[derive(Clone, Debug, Deserialize)]
pub struct Destination {
    pub return_to: String,
    pub state: String,
}

impl Destination {
    fn check(&self, config: &Config) -> Result<(), Refused> {
        // Either an exact allowlist entry, or a loopback on any port — which is the native
        // client, whose port the operating system picks. See `config::is_allowed_loopback`.
        let allowed = is_allowed_return(&config.return_to, &self.return_to)
            || is_allowed_loopback(&config.loopback_paths, &self.return_to);
        if !allowed {
            return Err(Refused::BadReturn);
        }
        if !signin::is_nonce(&self.state) {
            return Err(Refused::BadState);
        }
        Ok(())
    }
}

async fn page(State(broker): State<Broker>, Query(to): Query<Destination>) -> Response {
    match to.check(&broker.config) {
        Ok(()) => sign_in_page(&broker, &to, None).into_response(),
        Err(refused) => refusal(refused).into_response(),
    }
}

#[derive(Deserialize)]
struct Credentials {
    #[serde(flatten)]
    to: Destination,
    email: String,
    password: String,
    display_name: Option<String>,
}

async fn sign_in(
    State(broker): State<Broker>,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Form(form): Form<Credentials>,
) -> Response {
    complete(&broker, from, &form, false).await
}

async fn register(
    State(broker): State<Broker>,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    Form(form): Form<Credentials>,
) -> Response {
    complete(&broker, from, &form, true).await
}

async fn complete(
    broker: &Broker,
    from: SocketAddr,
    form: &Credentials,
    new_account: bool,
) -> Response {
    if let Err(refused) = form.to.check(&broker.config) {
        return refusal(refused).into_response();
    }
    if !broker.config.providers.allows(Provider::Password) {
        return refusal(Refused::NoSuchProvider).into_response();
    }

    // Two budgets. The per-account one stops a single password being guessed; the per-address
    // one stops a leaked credential list being walked, which spends one attempt per account and
    // so never troubles the first.
    let by_account = format!("account:{}", crate::store::normalise_email(&form.email));
    let by_address = format!("addr:{}", from.ip());
    if !broker.attempts.allows(&by_account, PER_ACCOUNT)
        || !broker.attempts.allows(&by_address, PER_ADDRESS)
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            shell(
                "Too many attempts",
                html! {
                    p { "Too many failed attempts. Try again later." }
                },
            ),
        )
            .into_response();
    }

    let outcome = if new_account {
        signin::register_password(
            &broker.store,
            &form.email,
            &form.password,
            form.display_name.as_deref().unwrap_or("Traveller"),
        )
        .await
    } else {
        signin::with_password(&broker.store, &form.email, &form.password).await
    };

    let account_id = match outcome {
        Ok(id) => id,
        Err(refused) => {
            broker.attempts.failed(&by_account);
            broker.attempts.failed(&by_address);
            return sign_in_page(broker, &form.to, Some(&refused)).into_response();
        }
    };
    broker.attempts.cleared(&by_account);

    let (code, digest) = signin::mint_code();
    if let Err(why) = broker
        .store
        .put_code(
            &digest,
            account_id,
            &form.to.return_to,
            Utc::now() + Duration::seconds(signin::CODE_LIFETIME_S),
        )
        .await
    {
        tracing::error!(%why, "could not record a sign-in code");
        return refusal(Refused::Backend(why.to_string())).into_response();
    }
    Redirect::to(&signin::return_url(
        &form.to.return_to,
        &code,
        &form.to.state,
    ))
    .into_response()
}

#[derive(Deserialize)]
struct NativeCredentials {
    email: String,
    password: String,
    /// What a revocation list shows a person. The machine, not an identifier.
    label: String,
    /// Only read when registering.
    display_name: Option<String>,
}

async fn native_signin(
    State(broker): State<Broker>,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    axum::Json(form): axum::Json<NativeCredentials>,
) -> Response {
    native(&broker, from, &form, false).await
}

async fn native_register(
    State(broker): State<Broker>,
    ConnectInfo(from): ConnectInfo<SocketAddr>,
    axum::Json(form): axum::Json<NativeCredentials>,
) -> Response {
    native(&broker, from, &form, true).await
}

/// The desktop password path: credentials in, a device grant out.
///
/// It exists because the desktop client cannot use the form above — that one redirects a
/// browser, and there is no browser here. `lightcone/docs/16-identity.md` records that putting
/// a password form in a game window is an argument *against* shipping this provider rather than
/// for embedding the others, and this endpoint is the shape that argument is about.
async fn native(
    broker: &Broker,
    from: SocketAddr,
    form: &NativeCredentials,
    new_account: bool,
) -> Response {
    // Not enabled is **404**, so a client can tell "this server has no password sign-in" from
    // "your password is wrong" and stop offering it.
    if !broker.config.providers.allows(Provider::Password) {
        return StatusCode::NOT_FOUND.into_response();
    }

    // The same two budgets the form path gets. A JSON endpoint is a better target for stuffing
    // than a form, not a worse one.
    let by_account = format!("account:{}", crate::store::normalise_email(&form.email));
    let by_address = format!("addr:{}", from.ip());
    if !broker.attempts.allows(&by_account, PER_ACCOUNT)
        || !broker.attempts.allows(&by_address, PER_ADDRESS)
    {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    let outcome = if new_account {
        signin::register_password(
            &broker.store,
            &form.email,
            &form.password,
            form.display_name.as_deref().unwrap_or("Traveller"),
        )
        .await
    } else {
        signin::with_password(&broker.store, &form.email, &form.password).await
    };
    let account_id = match outcome {
        Ok(id) => id,
        Err(refused) => {
            broker.attempts.failed(&by_account);
            broker.attempts.failed(&by_address);
            // One status for every refusal, as the form gives one message. A client that could
            // tell "no such account" from "wrong password" is an account enumerator.
            let status = match refused {
                Refused::Taken => StatusCode::CONFLICT,
                Refused::Unacceptable(_) => StatusCode::UNPROCESSABLE_ENTITY,
                _ => StatusCode::UNAUTHORIZED,
            };
            return status.into_response();
        }
    };
    broker.attempts.cleared(&by_account);

    let Ok(Some(account)) = broker.store.account(account_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (grant, digest) = signin::mint_code();
    let label: String = form.label.chars().take(120).collect();
    if let Err(why) = broker
        .store
        .put_grant(
            &digest,
            account_id,
            &label,
            Utc::now() + Duration::days(GRANT_DAYS),
        )
        .await
    {
        tracing::error!(%why, "could not record a device grant");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    axum::Json(Granted {
        grant,
        account_id: account.id.to_string(),
        display_name: account.display_name,
    })
    .into_response()
}

#[derive(Deserialize)]
struct Exchange {
    code: String,
    return_to: String,
}

#[derive(Serialize)]
struct Identity {
    account_id: String,
    display_name: String,
}

/// Swap a spent-once code for who it belonged to. Server to server, never a browser.
async fn exchange(
    State(broker): State<Broker>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<Exchange>,
) -> Response {
    if !presented_secret(&headers, &broker.config.exchange_secret) {
        // No body. An endpoint that explains itself to an unauthenticated caller is an
        // endpoint that helps them.
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let digest = signin::digest_of(&request.code);
    let found = broker
        .store
        .take_code(&digest, &request.return_to, Utc::now())
        .await;
    let account = match found {
        Ok(Some(id)) => broker.store.account(id).await,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(why) => {
            tracing::error!(%why, "exchange failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    match account {
        Ok(Some(account)) => axum::Json(Identity {
            account_id: account.id.to_string(),
            display_name: account.display_name,
        })
        .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The public keys a game server verifies tickets against.
///
/// Public by design and cached by whoever reads it: the game verifies locally, so an outage
/// here does not stop anyone reconnecting.
async fn jwks(State(broker): State<Broker>) -> Response {
    axum::Json(broker.keys.jwks()).into_response()
}

/// How a caller says who it is when asking for a ticket.
#[derive(Deserialize)]
#[serde(untagged)]
enum Asking {
    /// The site, which has its own session for the player and the shared secret for us.
    AsSite {
        account_id: String,
        audience: String,
    },
    /// A native client, holding what it was given the day it signed in.
    WithGrant { grant: String, audience: String },
}

#[derive(Serialize)]
struct Minted {
    ticket: String,
}

/// Mint a game ticket. **Both callers get the identical object**, which is what leaves the game
/// server with one code path and no notion of which build it is talking to.
async fn ticket(
    State(broker): State<Broker>,
    headers: HeaderMap,
    axum::Json(asking): axum::Json<Asking>,
) -> Response {
    let (account_id, audience) = match asking {
        Asking::AsSite {
            account_id,
            audience,
        } => {
            // The site speaks for a player because it holds the secret, not because it says so.
            if !presented_secret(&headers, &broker.config.exchange_secret) {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            let Ok(id) = account_id.parse::<uuid::Uuid>() else {
                return StatusCode::BAD_REQUEST.into_response();
            };
            (id, audience)
        }
        Asking::WithGrant { grant, audience } => {
            let digest = signin::digest_of(&grant);
            match broker.store.grant_holder(&digest, Utc::now()).await {
                Ok(Some(id)) => (id, audience),
                Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
                Err(why) => {
                    tracing::error!(%why, "grant lookup failed");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            }
        }
    };

    if !broker.config.audiences.contains(&audience) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Ok(Some(account)) = broker.store.account(account_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match broker.keys.mint(
        &account.id.to_string(),
        &account.display_name,
        &audience,
        chrono::Utc::now().timestamp(),
    ) {
        Ok(ticket) => axum::Json(Minted { ticket }).into_response(),
        Err(why) => {
            tracing::error!(%why, "could not mint a ticket");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
struct GrantRequest {
    code: String,
    return_to: String,
    label: String,
}

#[derive(Serialize)]
struct Granted {
    grant: String,
    account_id: String,
    display_name: String,
}

/// Trade a sign-in code for a device grant: what a native client keeps instead of a session.
///
/// The same code `/exchange` takes, spent the same way — a client gets one or the other, never
/// both, because the code is gone either way.
async fn grant(
    State(broker): State<Broker>,
    axum::Json(request): axum::Json<GrantRequest>,
) -> Response {
    // No shared secret: a native client cannot keep one. What authorises this is the code,
    // which is single use, sixty seconds old, and bound to the loopback it was issued for.
    let digest = signin::digest_of(&request.code);
    let account_id = match broker
        .store
        .take_code(&digest, &request.return_to, Utc::now())
        .await
    {
        Ok(Some(id)) => id,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(why) => {
            tracing::error!(%why, "grant exchange failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Ok(Some(account)) = broker.store.account(account_id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let (grant, digest) = signin::mint_code();
    let label: String = request.label.chars().take(120).collect();
    if let Err(why) = broker
        .store
        .put_grant(
            &digest,
            account_id,
            &label,
            Utc::now() + Duration::days(GRANT_DAYS),
        )
        .await
    {
        tracing::error!(%why, "could not record a device grant");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    axum::Json(Granted {
        grant,
        account_id: account.id.to_string(),
        display_name: account.display_name,
    })
    .into_response()
}

/// How long a device grant lasts.
///
/// Long, because the point is that a desktop player signs in once. Not forever, because a
/// credential with no expiry is one that outlives the machine it was issued to.
pub const GRANT_DAYS: i64 = 90;

/// Constant-time, because a byte-at-a-time comparison of a shared secret is guessable one byte
/// at a time.
fn presented_secret(headers: &HeaderMap, expected: &str) -> bool {
    let Some(offered) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    offered.as_bytes().ct_eq(expected.as_bytes()).into()
}

fn refusal(refused: Refused) -> impl IntoResponse {
    let (status, said): (StatusCode, &str) = match refused {
        // Never name what *would* be allowed: that is a list of targets.
        Refused::BadReturn | Refused::BadState => (StatusCode::BAD_REQUEST, "Bad sign-in request."),
        Refused::NoSuchProvider => (
            StatusCode::NOT_FOUND,
            "That way of signing in is not enabled here.",
        ),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong."),
    };
    (status, shell("Sign in", html! { p { (said) } }))
}

fn sign_in_page(broker: &Broker, to: &Destination, refused: Option<&Refused>) -> Markup {
    let complaint = refused.map(|refused| match refused {
        // One message for both, the same as the refusal itself.
        Refused::BadCredentials => "That address and password do not match.".to_string(),
        Refused::Taken => "That address already has an account.".to_string(),
        Refused::Unacceptable(_) => {
            format!(
                "A password needs at least {} characters.",
                crate::password::MIN_LENGTH
            )
        }
        _ => "Something went wrong.".to_string(),
    });

    shell(
        "Sign in",
        html! {
            h1 { "Sign in" }
            @if let Some(complaint) = &complaint {
                p role="alert" class="complaint" { (complaint) }
            }
            @if broker.config.providers.allows(Provider::Password) {
                form method="post" action="/signin/password" class="signin-form" {
                    input type="hidden" name="return_to" value=(to.return_to);
                    input type="hidden" name="state" value=(to.state);
                    label for="email" { "Email" }
                    input id="email" name="email" type="email" autocomplete="username" required;
                    label for="password" { "Password" }
                    input id="password" name="password" type="password"
                        autocomplete="current-password" required;
                    button type="submit" { "Sign in" }
                }
            }
            @for provider in broker.config.providers.live() {
                @if provider.is_upstream() {
                    p class="provider-pending" {
                        "Signing in with " (provider.name()) " is configured but not yet built."
                    }
                }
            }
        },
    )
}

fn shell(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="referrer" content="no-referrer";
                title { (title) }
            }
            body { main { (body) } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn destination(return_to: &str, state: &str) -> Destination {
        Destination {
            return_to: return_to.into(),
            state: state.into(),
        }
    }

    fn config() -> Config {
        Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
            providers: crate::providers::resolve("password", |_| None).unwrap(),
            return_to: vec!["https://lightcone.example/auth/return".into()],
            loopback_paths: vec!["/return".into()],
            exchange_secret: "shared".into(),
            audiences: vec!["shard-1".into()],
            issuer: "https://accounts.lightcone.example".into(),
            signing_seed: None,
        }
    }

    fn broker(config: Config) -> Broker {
        Broker {
            config: Arc::new(config),
            store: Store::memory(),
            attempts: Arc::new(Attempts::default()),
            keys: Arc::new(
                Keys::from_seed(&[4u8; 32], "https://accounts.lightcone.example").unwrap(),
            ),
        }
    }

    /// A form is a thing the browser sends. Everything in it is checked again on the way out,
    /// or a hidden field is an open redirect with extra steps.
    #[test]
    fn a_destination_from_a_form_is_re_checked() {
        let config = config();
        assert!(
            destination("https://lightcone.example/auth/return", "nonce1")
                .check(&config)
                .is_ok()
        );
        assert_eq!(
            destination("https://evil.test/return", "nonce1").check(&config),
            Err(Refused::BadReturn),
        );
        assert_eq!(
            destination("https://lightcone.example/auth/return", "a&b=c").check(&config),
            Err(Refused::BadState),
        );
    }

    /// A secret compared byte by byte is a secret guessed byte by byte.
    #[test]
    fn the_exchange_secret_is_compared_in_constant_time() {
        let mut headers = HeaderMap::new();
        assert!(
            !presented_secret(&headers, "shared"),
            "no header is not a match"
        );

        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer shared".parse().unwrap(),
        );
        assert!(presented_secret(&headers, "shared"));

        for wrong in [
            "Bearer sharee",
            "Bearer share",
            "Bearer sharedd",
            "shared",
            "Basic shared",
        ] {
            headers.insert(axum::http::header::AUTHORIZATION, wrong.parse().unwrap());
            assert!(
                !presented_secret(&headers, "shared"),
                "{wrong:?} was accepted"
            );
        }
    }

    /// The refusal a browser sees says nothing about what would have worked.
    #[test]
    fn a_bad_return_url_is_not_told_what_a_good_one_is() {
        let rendered = refusal(Refused::BadReturn).into_response();
        assert_eq!(rendered.status(), StatusCode::BAD_REQUEST);
        let page = shell("Sign in", html! { p { "Bad sign-in request." } }).into_string();
        assert!(
            !page.contains("lightcone.example"),
            "the allowlist leaked into the page"
        );
    }

    /// Wrong address and wrong password are one message, the same as they are one refusal.
    #[test]
    fn the_form_does_not_say_which_half_was_wrong() {
        let broker = broker(config());
        let to = destination("https://lightcone.example/auth/return", "nonce1");
        let page = sign_in_page(&broker, &to, Some(&Refused::BadCredentials)).into_string();
        assert!(page.contains("do not match"));
        assert!(
            !page.to_lowercase().contains("no such"),
            "it named the missing half"
        );
        // And the destination rides through the form so the POST knows where to go back to.
        assert!(page.contains("https://lightcone.example/auth/return"));
        assert!(page.contains("nonce1"));
    }

    /// A configured provider with no route is said so rather than rendered as a dead button.
    #[test]
    fn a_provider_that_is_not_built_yet_is_not_a_button() {
        let mut config = config();
        config.providers = crate::providers::resolve("password,google", |name| {
            matches!(name, "GOOGLE_CLIENT_ID" | "GOOGLE_CLIENT_SECRET").then(|| "x".to_string())
        })
        .unwrap();
        let broker = broker(config);
        let to = destination("https://lightcone.example/auth/return", "n");
        let page = sign_in_page(&broker, &to, None).into_string();
        assert!(page.contains("not yet built"));
        assert!(
            !page.contains("action=\"/signin/google\""),
            "a dead button was rendered"
        );
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    const SHARD: &str = "shard-1";

    fn a_broker() -> Broker {
        Broker {
            config: Arc::new(Config {
                bind: "127.0.0.1:0".parse().unwrap(),
                database_url: String::new(),
                providers: crate::providers::resolve("password", |_| None).unwrap(),
                return_to: vec!["https://lightcone.example/auth/return".into()],
                loopback_paths: vec!["/return".into()],
                exchange_secret: "shared".into(),
                audiences: vec![SHARD.into()],
                issuer: "https://accounts.lightcone.example".into(),
                signing_seed: None,
            }),
            store: Store::memory(),
            attempts: Arc::new(Attempts::default()),
            keys: Arc::new(
                Keys::from_seed(&[4u8; 32], "https://accounts.lightcone.example").unwrap(),
            ),
        }
    }

    async fn an_account(broker: &Broker) -> uuid::Uuid {
        crate::signin::register_password(
            &broker.store,
            "ada@example.test",
            "a good password",
            "Ada",
        )
        .await
        .unwrap()
    }

    async fn call(broker: &Broker, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = router(broker.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    fn post(path: &str, body: serde_json::Value, secret: Option<&str>) -> Request<Body> {
        let mut builder = Request::post(path).header("content-type", "application/json");
        if let Some(secret) = secret {
            builder = builder.header("authorization", format!("Bearer {secret}"));
        }
        let mut request = builder.body(Body::from(body.to_string())).unwrap();
        // Driving the router directly leaves no peer address, and the routes that meter by
        // address extract one. Without this they answer 500 and a test reads it as the
        // endpoint being broken.
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        request
    }

    /// The site speaks for a player because it holds the secret, not because it says so.
    #[tokio::test]
    async fn the_site_cannot_mint_without_the_secret() {
        let broker = a_broker();
        let id = an_account(&broker).await;
        let asking = serde_json::json!({"account_id": id.to_string(), "audience": SHARD});

        let (status, _) = call(&broker, post("/ticket", asking.clone(), None)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = call(&broker, post("/ticket", asking.clone(), Some("wrong"))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let (status, _) = call(&broker, post("/ticket", asking, Some("shared"))).await;
        assert_eq!(status, StatusCode::OK);
    }

    /// An audience allowlist for the same reason `return_to` is one. "The audience is ours
    /// anyway" stops being true the first time it is not.
    #[tokio::test]
    async fn a_ticket_cannot_be_minted_for_an_unknown_shard() {
        let broker = a_broker();
        let id = an_account(&broker).await;
        let (status, _) = call(
            &broker,
            post(
                "/ticket",
                serde_json::json!({"account_id": id.to_string(), "audience": "somebody-elses"}),
                Some("shared"),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// **Both callers get the identical object.** That is what leaves the game server with one
    /// code path and no notion of which build it is talking to.
    #[tokio::test]
    async fn a_grant_and_the_site_mint_the_same_kind_of_ticket() {
        let broker = a_broker();
        let id = an_account(&broker).await;

        // The native path: sign in, take a code, trade it for a grant.
        let (code, digest) = crate::signin::mint_code();
        let here = "https://lightcone.example/auth/return";
        broker
            .store
            .put_code(&digest, id, here, Utc::now() + Duration::seconds(60))
            .await
            .unwrap();
        let (status, granted) = call(
            &broker,
            post(
                "/grant",
                serde_json::json!({"code": code, "return_to": here, "label": "Ada's laptop"}),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(granted["account_id"], id.to_string());
        let grant = granted["grant"].as_str().expect("a grant").to_string();

        let (status, from_grant) = call(
            &broker,
            post(
                "/ticket",
                serde_json::json!({"grant": grant, "audience": SHARD}),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (_, from_site) = call(
            &broker,
            post(
                "/ticket",
                serde_json::json!({"account_id": id.to_string(), "audience": SHARD}),
                Some("shared"),
            ),
        )
        .await;

        // Different tokens — each has its own identifier — but the same claims about who.
        let claims = |value: &serde_json::Value| {
            use base64::Engine;
            let token = value["ticket"].as_str().expect("a ticket").to_string();
            let payload = token.split('.').nth(1).expect("a payload").to_string();
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(payload)
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        };
        let (one, two) = (claims(&from_grant), claims(&from_site));
        assert_eq!(one["sub"], two["sub"]);
        assert_eq!(one["aud"], two["aud"]);
        assert_eq!(one["name"], two["name"]);
        assert_ne!(one["jti"], two["jti"], "two tickets shared an identifier");
    }

    /// A grant nobody issued is nobody, and the same for one that has been revoked.
    #[tokio::test]
    async fn an_unknown_grant_mints_nothing() {
        let broker = a_broker();
        an_account(&broker).await;
        let (status, _) = call(
            &broker,
            post(
                "/ticket",
                serde_json::json!({"grant": "invented", "audience": SHARD}),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// A code buys a session *or* a grant, never both: it is gone either way.
    #[tokio::test]
    async fn a_code_spent_on_a_grant_cannot_also_be_exchanged() {
        let broker = a_broker();
        let id = an_account(&broker).await;
        let (code, digest) = crate::signin::mint_code();
        let here = "https://lightcone.example/auth/return";
        broker
            .store
            .put_code(&digest, id, here, Utc::now() + Duration::seconds(60))
            .await
            .unwrap();

        let body = serde_json::json!({"code": code, "return_to": here, "label": "laptop"});
        assert_eq!(
            call(&broker, post("/grant", body.clone(), None)).await.0,
            StatusCode::OK
        );
        assert_eq!(
            call(&broker, post("/grant", body.clone(), None)).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            call(&broker, post("/exchange", body, Some("shared")))
                .await
                .0,
            StatusCode::NOT_FOUND,
        );
    }

    /// The desktop password path, end to end: credentials in, a grant out, and that grant
    /// mints tickets like any other.
    #[tokio::test]
    async fn the_native_path_registers_and_then_signs_in() {
        let broker = a_broker();
        let make = serde_json::json!({
            "email": "ada@example.test", "password": "a good password",
            "label": "Ada's laptop", "display_name": "Ada",
        });
        let (status, granted) = call(&broker, post("/signin/register/native", make, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(granted["display_name"], "Ada");

        // The grant it handed back is a real one.
        let (status, minted) = call(
            &broker,
            post(
                "/ticket",
                serde_json::json!({"grant": granted["grant"], "audience": SHARD}),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            minted["ticket"]
                .as_str()
                .is_some_and(|t| t.split('.').count() == 3)
        );

        // And signing in again works without registering again.
        let again = serde_json::json!({
            "email": "Ada@Example.TEST", "password": "a good password", "label": "the same laptop",
        });
        let (status, second) = call(&broker, post("/signin/password/native", again, None)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            second["account_id"], granted["account_id"],
            "it made a second account"
        );
        assert_ne!(
            second["grant"], granted["grant"],
            "two devices shared a grant"
        );
    }

    /// One status for every refusal. A client that could tell "no such account" from "wrong
    /// password" is an account enumerator with a nicer interface.
    #[tokio::test]
    async fn the_native_path_does_not_say_which_half_was_wrong() {
        let broker = a_broker();
        let make = serde_json::json!({
            "email": "ada@example.test", "password": "a good password", "label": "laptop",
            "display_name": "Ada",
        });
        assert_eq!(
            call(&broker, post("/signin/register/native", make, None))
                .await
                .0,
            StatusCode::OK
        );

        for attempt in [
            serde_json::json!({"email": "ada@example.test", "password": "wrong", "label": "l"}),
            serde_json::json!({"email": "nobody@example.test", "password": "a good password", "label": "l"}),
        ] {
            let (status, body) =
                call(&broker, post("/signin/password/native", attempt, None)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(
                body,
                serde_json::Value::Null,
                "the refusal explained itself"
            );
        }
    }

    /// Not enabled is **404**, distinct from a refusal, so a client can stop offering it rather
    /// than showing a form that can never work.
    #[tokio::test]
    async fn a_server_without_the_password_provider_says_so() {
        let mut config = Config {
            bind: "127.0.0.1:0".parse().unwrap(),
            database_url: String::new(),
            providers: crate::providers::resolve("password", |_| None).unwrap(),
            return_to: vec!["https://lightcone.example/auth/return".into()],
            loopback_paths: vec!["/return".into()],
            exchange_secret: "shared".into(),
            audiences: vec![SHARD.into()],
            issuer: "https://accounts.lightcone.example".into(),
            signing_seed: None,
        };
        config.providers = crate::providers::resolve("google", |name| {
            matches!(name, "GOOGLE_CLIENT_ID" | "GOOGLE_CLIENT_SECRET").then(|| "x".to_string())
        })
        .unwrap();
        let broker = Broker {
            config: Arc::new(config),
            ..a_broker()
        };

        let (status, _) = call(
            &broker,
            post(
                "/signin/password/native",
                serde_json::json!({"email": "a@b.test", "password": "whatever", "label": "l"}),
                None,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// The key set is public, and it is what a game server verifies against.
    #[tokio::test]
    async fn the_key_set_is_published() {
        let broker = a_broker();
        let (status, jwks) = call(
            &broker,
            Request::get("/.well-known/jwks.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(jwks["keys"][0]["kid"], broker.keys.kid());
        assert_eq!(jwks["keys"][0]["crv"], "Ed25519");
        // And nothing private is in it. Checked as a *field*, not a substring: `"kid"`
        // contains `d"`, which is how the first version of this failed against correct code.
        assert!(
            jwks["keys"][0].get("d").is_none(),
            "the private parameter was published"
        );
        assert_eq!(jwks["keys"][0]["x"], broker.keys.public_x());
    }
}
