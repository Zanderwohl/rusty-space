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
use crate::config::{Config, is_allowed_return};
use crate::providers::Provider;
use crate::signin::{self, Refused};
use crate::store::Store;

#[derive(Clone)]
pub struct Broker {
    pub config: Arc<Config>,
    pub store: Store,
    pub attempts: Arc<Attempts>,
}

pub fn router(broker: Broker) -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/signin", get(page))
        .route("/signin/password", post(sign_in))
        .route("/signin/register", post(register))
        .route("/exchange", post(exchange))
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
        if !is_allowed_return(&config.return_to, &self.return_to) {
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
            exchange_secret: "shared".into(),
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
        let broker = Broker {
            config: Arc::new(config()),
            store: Store::memory(),
            attempts: Arc::new(Attempts::default()),
        };
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
        let broker = Broker {
            config: Arc::new(config),
            store: Store::memory(),
            attempts: Arc::new(Attempts::default()),
        };
        let to = destination("https://lightcone.example/auth/return", "n");
        let page = sign_in_page(&broker, &to, None).into_string();
        assert!(page.contains("not yet built"));
        assert!(
            !page.contains("action=\"/signin/google\""),
            "a dead button was rendered"
        );
    }
}
