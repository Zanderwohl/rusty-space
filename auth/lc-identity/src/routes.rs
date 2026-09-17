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
    /// For the upstream half only. Shared so the connection pool is, and carrying the timeout
    /// that keeps a slow provider from becoming a slow broker.
    pub http: reqwest::Client,
}

pub fn router(broker: Broker) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route(crate::assets::ROUTE, get(crate::assets::serve))
        .route("/signin", get(page))
        .route("/register", get(register_page))
        .route("/signin/password", post(sign_in))
        .route("/signin/register", post(register))
        // The native pair. Same credentials, same budgets, no redirect — so no code either:
        // a code exists to survive a browser round trip and there is not one here.
        .route("/signin/password/native", post(native_signin))
        .route("/signin/register/native", post(native_register))
        // The upstream pair. A static path wins over `{provider}` in the router, so the
        // password routes above are unaffected by it.
        .route("/signin/{provider}", get(crate::upstream::start))
        .route("/signin/{provider}/callback", get(crate::upstream::finish))
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
    /// `#[serde(default)]` on both, so a URL missing one of them reaches [`check`] and is
    /// refused there. Without it the extractor rejects first, and axum's rejection is a bare
    /// plain-text 400 — an unstyled page for an ordinary mistake, from a service whose entire
    /// job is to look like the site it belongs to.
    ///
    /// The empty string is safe to fall through to: `Config::from_env` drops empty allowlist
    /// entries, so it can never match one.
    ///
    /// [`check`]: Destination::check
    #[serde(default)]
    pub return_to: String,
    #[serde(default)]
    pub state: String,
}

impl Destination {
    pub(crate) fn check(&self, config: &Config) -> Result<(), Refused> {
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

/// The sign-up page. A separate address so a person can be sent straight to it, and so the
/// browser's password manager sees two pages rather than one that changes meaning.
async fn register_page(State(broker): State<Broker>, Query(to): Query<Destination>) -> Response {
    // Not enabled is not found, the same answer the native endpoint gives. A page that renders
    // a form which can only ever be refused is worse than no page.
    if !broker.config.providers.allows(Provider::Password) {
        return refusal(Refused::NoSuchProvider).into_response();
    }
    match to.check(&broker.config) {
        Ok(()) => credentials_page(&broker, &to, None, Which::Register).into_response(),
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
                    h1 { "Too many attempts" }
                    p { "Too many failed sign-ins from here. Try again in a few minutes." }
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
            // Back to the page they were on. Sending a failed sign-up to the sign-in form
            // loses what they typed and reads as though the account was made.
            let which = if new_account {
                Which::Register
            } else {
                Which::SignIn
            };
            return credentials_page(broker, &form.to, Some(&refused), which).into_response();
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

pub(crate) fn refusal(refused: Refused) -> impl IntoResponse {
    let (status, heading, said): (StatusCode, &str, &str) = match refused {
        // Never name what *would* be allowed: that is a list of targets. The heading is as
        // uninformative as the sentence, for the same reason.
        Refused::BadReturn | Refused::BadState => (
            StatusCode::BAD_REQUEST,
            "Bad sign-in request",
            "That sign-in link is not one this service will follow.",
        ),
        // The caller's fault, not ours, and specifically not a 500: an expired or replayed
        // state is an ordinary thing that happens to people who leave a tab open, and paging
        // somebody every time one does is how alarms get ignored.
        Refused::NoFlow => (
            StatusCode::BAD_REQUEST,
            "That sign-in expired",
            "It was already used, or it sat too long. Start again.",
        ),
        Refused::Upstream(_) => (
            StatusCode::BAD_GATEWAY,
            "That provider is not answering",
            "Try again shortly, or use another way in.",
        ),
        Refused::NoSuchProvider => (
            StatusCode::NOT_FOUND,
            "Not enabled here",
            "That way of signing in is not available on this server.",
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Something went wrong",
            "Nothing you did. Try again in a moment.",
        ),
    };
    (
        status,
        shell(
            heading,
            html! {
                h1 { (heading) }
                p { (said) }
            },
        ),
    )
}

/// Which of the two credential pages is being drawn.
///
/// One function rather than two, because the pair differ in four strings and a form action and
/// would otherwise drift apart — which for a sign-in and a sign-up means the two paths into an
/// account stop looking like the same product.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Which {
    SignIn,
    Register,
}

impl Which {
    fn title(self) -> &'static str {
        match self {
            Which::SignIn => "Sign in",
            Which::Register => "Create an account",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Which::SignIn => "/signin/password",
            Which::Register => "/signin/register",
        }
    }

    /// What the browser's password manager should do. Getting this wrong is how a manager
    /// offers to fill a new-account form with an existing password, or offers to save nothing.
    fn autocomplete(self) -> &'static str {
        match self {
            Which::SignIn => "current-password",
            Which::Register => "new-password",
        }
    }

    fn other(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Which::SignIn => ("No account yet?", "/register", "Create one"),
            Which::Register => ("Already have an account?", "/signin", "Sign in"),
        }
    }
}

fn complaint_for(refused: &Refused) -> String {
    match refused {
        // One message for both, the same as the refusal itself.
        Refused::BadCredentials => "That address and password do not match.".to_string(),
        Refused::Taken => "That address already has an account.".to_string(),
        Refused::Unacceptable(_) => {
            format!(
                "A password needs at least {} characters.",
                crate::password::MIN_LENGTH
            )
        }
        // Not an error. Somebody pressed Cancel, and saying "something went wrong" to that is
        // telling a person they made a mistake by changing their mind.
        Refused::Declined => "You did not finish signing in with that provider.".to_string(),
        Refused::Upstream(_) => {
            "That provider could not be reached. Try again, or use another way in.".to_string()
        }
        _ => "Something went wrong.".to_string(),
    }
}

/// A destination carried through a link rather than a form.
pub(crate) fn with_destination(path: &str, to: &Destination) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("return_to", &to.return_to)
        .append_pair("state", &to.state)
        .finish();
    format!("{path}?{query}")
}

pub(crate) fn sign_in_page(broker: &Broker, to: &Destination, refused: Option<&Refused>) -> Markup {
    credentials_page(broker, to, refused, Which::SignIn)
}

pub(crate) fn credentials_page(
    broker: &Broker,
    to: &Destination,
    refused: Option<&Refused>,
    which: Which,
) -> Markup {
    let complaint = refused.map(complaint_for);
    let password = broker.config.providers.allows(Provider::Password);
    let upstream: Vec<Provider> = broker
        .config
        .providers
        .live()
        .into_iter()
        .filter(|p| p.is_upstream())
        .collect();
    let (prompt, path, link) = which.other();

    shell(
        which.title(),
        html! {
            h1 { (which.title()) }
            @if let Some(complaint) = &complaint {
                p role="alert" class="complaint" { (complaint) }
            }
            @if password {
                form method="post" action=(which.action()) class="signin-form" {
                    input type="hidden" name="return_to" value=(to.return_to);
                    input type="hidden" name="state" value=(to.state);
                    @if which == Which::Register {
                        label for="display_name" { "Name" }
                        // What other players see. Optional, because a person who has not
                        // decided should still be able to finish.
                        input id="display_name" name="display_name" type="text"
                            autocomplete="nickname" maxlength="60";
                    }
                    label for="email" { "Email" }
                    input id="email" name="email" type="email" autocomplete="username" required;
                    label for="password" { "Password" }
                    input id="password" name="password" type="password"
                        autocomplete=(which.autocomplete())
                        minlength=(crate::password::MIN_LENGTH) required;
                    @if which == Which::Register {
                        // Said before it is refused, because a rule a person meets on the first
                        // try is not a rule they had to be told off about.
                        p class="fine-print" {
                            "At least " (crate::password::MIN_LENGTH) " characters. "
                            "There is no password reset yet, so keep it somewhere."
                        }
                    }
                    button type="submit" { (which.title()) }
                }
            }
            // Only between two things. On a deployment with one way in it is a separator
            // separating nothing, which reads as a missing form.
            @if password && !upstream.is_empty() {
                p class="or" { "or" }
            }
            @for provider in &upstream {
                // A link and not a form: starting a dance writes a row and redirects, and
                // neither is a state change this page is responsible for.
                a class="provider-link" href=(crate::upstream::start_url(*provider, to)) {
                    "Continue with " (provider.label())
                }
            }
            // Last, under the rule: it is the way off this page, not one of the ways through
            // it, and above the providers it read as the end of the form.
            @if password {
                p class="other-way" {
                    (prompt) " " a href=(with_destination(path, to)) { (link) }
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
                // Every URL here carries a `return_to` and a `state`. A `Referer` would hand
                // both to whatever a page links out to.
                meta name="referrer" content="no-referrer";
                // Nothing here is a destination. A sign-in page in an index is a sign-in page
                // reached without the query that makes it work.
                meta name="robots" content="noindex, nofollow";
                title { (title) " \u{2014} Lightcone Frontier" }
                link rel="stylesheet" href=(crate::assets::url());
            }
            body {
                // Text, not a link. Everywhere a browser goes from these pages is on the
                // allowlist, and a masthead is one more place to be sent that is not.
                p class="wordmark" { "Lightcone Frontier" }
                main { (body) }
                footer class="fine-print" {
                    "A relativistic sandbox in a volume of real stars."
                }
            }
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
            public_url: Some("https://accounts.lightcone.example".into()),
            signing_seed: None,
        }
    }

    fn broker(config: Config) -> Broker {
        Broker {
            config: Arc::new(config),
            store: Store::memory(),
            attempts: Arc::new(Attempts::default()),
            http: reqwest::Client::new(),
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
    ///
    /// Asserted against the **rendered response**, not against a page rebuilt here. The first
    /// version of this built its own copy of the markup and would have passed whatever the
    /// handler actually sent.
    #[tokio::test]
    async fn a_bad_return_url_is_not_told_what_a_good_one_is() {
        use http_body_util::BodyExt;

        let rendered = refusal(Refused::BadReturn).into_response();
        assert_eq!(rendered.status(), StatusCode::BAD_REQUEST);
        let bytes = rendered.into_body().collect().await.unwrap().to_bytes();
        let page = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            !page.contains("lightcone.example"),
            "the allowlist leaked into the page: {page}"
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

    /// An enabled upstream provider is a link that carries the destination with it. Without
    /// that, the dance starts and has nowhere to finish.
    #[test]
    fn an_upstream_provider_is_a_link_that_keeps_the_destination() {
        let mut config = config();
        config.providers = crate::providers::resolve("password,google", |name| {
            matches!(name, "GOOGLE_CLIENT_ID" | "GOOGLE_CLIENT_SECRET").then(|| "x".to_string())
        })
        .unwrap();
        let broker = broker(config);
        let to = destination("https://lightcone.example/auth/return", "nonce1");
        let page = sign_in_page(&broker, &to, None).into_string();

        assert!(page.contains("Continue with Google"), "{page}");
        // Percent-encoded, because it is a URL inside a query string.
        assert!(
            page.contains("/signin/google?return_to=https%3A%2F%2Flightcone.example%2Fauth%2Freturn&amp;state=nonce1"),
            "{page}"
        );
    }

    /// The sign-up page exists and carries the destination, or a person who follows it from
    /// the sign-in page arrives somewhere that cannot finish.
    #[test]
    fn the_sign_up_page_keeps_the_destination_and_offers_the_way_back() {
        let broker = broker(config());
        let to = destination("https://lightcone.example/auth/return", "nonce1");
        let page = credentials_page(&broker, &to, None, Which::Register).into_string();

        assert!(page.contains("action=\"/signin/register\""), "{page}");
        assert!(page.contains("https://lightcone.example/auth/return"));
        assert!(page.contains("nonce1"));
        // A new account needs a name and a new-password field, not a current-password one.
        assert!(page.contains("display_name"), "{page}");
        assert!(page.contains("new-password"), "{page}");
        assert!(!page.contains("current-password"), "{page}");
        // And the way back, with the destination intact.
        assert!(
            page.contains(
                "/signin?return_to=https%3A%2F%2Flightcone.example%2Fauth%2Freturn&amp;state=nonce1"
            ),
            "{page}",
        );
    }

    /// And the sign-in page offers the way to it, or nobody can make an account at all.
    #[test]
    fn the_sign_in_page_offers_a_way_to_make_an_account() {
        let page = sign_in_page(
            &broker(config()),
            &destination("https://lightcone.example/auth/return", "nonce1"),
            None,
        )
        .into_string();
        assert!(
            page.contains("/register?return_to=https%3A%2F%2Flightcone.example%2Fauth%2Freturn&amp;state=nonce1"),
            "{page}",
        );
        assert!(page.contains("current-password"), "{page}");
        assert!(
            !page.contains("display_name"),
            "a sign-in asked for a name: {page}"
        );
    }

    /// A failed sign-up comes back to the sign-up page. Sending it to the sign-in form reads as
    /// though the account was made.
    #[test]
    fn a_refused_sign_up_says_so_on_the_sign_up_page() {
        let page = credentials_page(
            &broker(config()),
            &destination("https://lightcone.example/auth/return", "n"),
            Some(&Refused::Taken),
            Which::Register,
        )
        .into_string();
        assert!(page.contains("already has an account"), "{page}");
        assert!(page.contains("action=\"/signin/register\""), "{page}");
    }

    /// Off unless enabled, like everything else here.
    #[test]
    fn a_provider_this_deployment_does_not_have_is_not_offered() {
        let page = sign_in_page(
            &broker(config()),
            &destination("https://lightcone.example/auth/return", "n"),
            None,
        )
        .into_string();
        assert!(!page.contains("Google"), "{page}");
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
                public_url: Some("https://accounts.lightcone.example".into()),
                signing_seed: None,
            }),
            store: Store::memory(),
            attempts: Arc::new(Attempts::default()),
            http: reqwest::Client::new(),
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
            public_url: Some("https://accounts.lightcone.example".into()),
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

    /// A browser GET, with the peer address every metered route extracts.
    fn get(path: &str) -> Request<Body> {
        let mut request = Request::get(path).body(Body::empty()).unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        request
    }

    /// A form POST, which is how a browser submits credentials.
    fn form(path: &str, body: &str) -> Request<Body> {
        let mut request = Request::post(path)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body.to_owned()))
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 40000))));
        request
    }

    async fn page(broker: &Broker, request: Request<Body>) -> (StatusCode, Option<String>, String) {
        let response = router(broker.clone()).oneshot(request).await.unwrap();
        let status = response.status();
        let location = response
            .headers()
            .get(axum::http::header::LOCATION)
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, location, String::from_utf8(bytes.to_vec()).unwrap())
    }

    /// Every way into this service that names where to go afterwards, refused for the same
    /// `return_to`.
    ///
    /// The unit tests cover `is_allowed_return` itself; this covers the **wiring**, which is
    /// the half that breaks silently. A route that forgot to call `check` would pass every one
    /// of those and still be an open redirect.
    #[tokio::test]
    async fn no_route_will_send_a_browser_somewhere_off_the_allowlist() {
        let mut config = Config {
            // Google too, so `/signin/google` below is a live route rather than a 404 that
            // would pass this test without ever reaching the check.
            providers: crate::providers::resolve("password,google", |name| {
                matches!(name, "GOOGLE_CLIENT_ID" | "GOOGLE_CLIENT_SECRET").then(|| "x".to_string())
            })
            .unwrap(),
            ..Config::clone(&a_broker().config)
        };
        config.loopback_paths = vec!["/return".into()];
        let broker = Broker {
            config: Arc::new(config),
            ..a_broker()
        };
        an_account(&broker).await;

        for hostile in [
            // The prefix and suffix tricks a `starts_with` or `ends_with` check would pass.
            "https://lightcone.example.attacker.test/auth/return",
            "https://attacker.test/https://lightcone.example/auth/return",
            "https://attacker.test#https://lightcone.example/auth/return",
            // Protocol-relative, which a browser reads as another origin entirely.
            "//attacker.test/auth/return",
            // Scheme downgrades and non-http schemes.
            "http://lightcone.example/auth/return",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            // Userinfo, which puts the real host after an @ that a person will not read.
            "https://lightcone.example%2Fauth%2Freturn@attacker.test/",
            // A loopback that is not one, and a loopback path nobody allowlisted.
            "http://127.0.0.1.attacker.test:7635/return",
            "http://localhost:7635/return",
            "http://127.0.0.1:7635/anything-else",
            // Nothing at all.
            "",
        ] {
            let escaped =
                url::form_urlencoded::byte_serialize(hostile.as_bytes()).collect::<String>();

            // The two pages a person lands on, which carry the destination in the query.
            for path in [
                format!("/signin?return_to={escaped}&state=nonce1"),
                format!("/register?return_to={escaped}&state=nonce1"),
                // And starting an upstream dance, which is a redirect out of here.
                format!("/signin/google?return_to={escaped}&state=nonce1"),
            ] {
                let (status, location, body) = page(&broker, get(&path)).await;
                assert!(
                    status.is_client_error(),
                    "{path} answered {status} for {hostile:?}"
                );
                assert_eq!(location, None, "{path} redirected for {hostile:?}");
                assert!(
                    !body.contains("lightcone.example"),
                    "{path} leaked the allowlist for {hostile:?}"
                );
            }

            // And the form POST, where the destination is a hidden field the browser sends
            // back. **This is the one that matters**: a hidden field is whatever it was
            // edited to, so a check done only when the page was drawn is no check at all.
            let credentials = format!(
                "return_to={escaped}&state=nonce1&email=ada%40example.test&password=a+good+password"
            );
            for path in ["/signin/password", "/signin/register"] {
                let (status, location, _) = page(&broker, form(path, &credentials)).await;
                assert_eq!(
                    status,
                    StatusCode::BAD_REQUEST,
                    "{path} accepted {hostile:?}"
                );
                assert_eq!(location, None, "{path} redirected to {hostile:?}");
            }
        }
    }

    /// And the allowlisted destination still works, or the test above passes on a broker that
    /// refuses everyone.
    #[tokio::test]
    async fn the_allowlisted_destination_still_completes_a_sign_in() {
        let broker = a_broker();
        an_account(&broker).await;
        let here = "https%3A%2F%2Flightcone.example%2Fauth%2Freturn";

        let (status, _, body) =
            page(&broker, get(&format!("/signin?return_to={here}&state=n1"))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Sign in"), "{body}");

        let (status, location, _) = page(
            &broker,
            form(
                "/signin/password",
                &format!(
                    "return_to={here}&state=n1&email=ada%40example.test&password=a+good+password"
                ),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::SEE_OTHER);
        let location = location.expect("a redirect");
        assert!(
            location.starts_with("https://lightcone.example/auth/return?code="),
            "{location}"
        );
        assert!(location.ends_with("&state=n1"), "{location}");
    }

    /// A URL with the destination missing is a refusal, not an extractor rejection.
    ///
    /// Same 400 either way; the difference is that this one is a page. Axum's rejection is
    /// bare plain text, which from a service whose whole job is to look like the site it
    /// belongs to is a broken-looking 400 for an ordinary mistake.
    #[tokio::test]
    async fn a_sign_in_url_missing_its_destination_gets_a_page() {
        let broker = a_broker();
        for path in ["/signin", "/signin?state=nonce1", "/signin?return_to="] {
            let (status, _, body) = page(&broker, get(path)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert!(body.starts_with("<!DOCTYPE html>"), "{path} sent {body:?}");
            assert!(body.contains("stylesheet"), "{path} sent an unstyled page");
        }
    }

    /// The stylesheet is served, and cached forever — which is only safe because its URL
    /// carries a digest of its own bytes.
    #[tokio::test]
    async fn the_stylesheet_is_served_and_cacheable() {
        let broker = a_broker();
        let response = router(broker)
            .oneshot(get(&crate::assets::url()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()[axum::http::header::CONTENT_TYPE],
            "text/css; charset=utf-8"
        );
        assert!(
            response.headers()[axum::http::header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("immutable")
        );
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
