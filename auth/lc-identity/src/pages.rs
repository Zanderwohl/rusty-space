//! The pages the broker draws.
//!
//! Split from `routes` at the file-size cap, and here because the seam is real: this renders
//! and touches no request; that is the HTTP surface.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use maud::{DOCTYPE, Markup, html};

use crate::providers::Provider;
use crate::routes::{Broker, Destination};
use crate::signin::{self, Refused};

/// The reason is deliberately **not** here: it belongs on a page an administrator reads, and
/// the private notes sit one column from it.
pub(crate) fn banned_page(sanction: &crate::bans::Sanction) -> (StatusCode, Markup) {
    let now = Utc::now();
    let how_long = signin::how_long(sanction, now);
    (
        StatusCode::FORBIDDEN,
        shell(
            "Account suspended",
            html! {
                h1 { "Account suspended" }
                p { "This account cannot sign in." }
                p { (how_long) }
                @if let Some(until) = sanction.until {
                    p class="fine-print" {
                        "Until " time datetime=(until.to_rfc3339()) { (until.format("%e %B %Y, %H:%M UTC")) } "."
                    }
                }
                @if sanction.count > 1 {
                    // Or somebody waits out the one they know about and assumes the service is
                    // broken.
                    p class="fine-print" {
                        (sanction.count) " suspensions are in force. The date above is when the last of them ends."
                    }
                }
            },
        ),
    )
}

/// A ban is the one refusal with something worth saying in the body. The rest keep their bare
/// status, because explaining a failure to an unauthenticated caller is helping them.
pub(crate) fn native_refusal(refused: Refused) -> Response {
    match refused {
        Refused::Banned(sanction) => (
            StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({
                "error": "banned",
                "said": signin::how_long(&sanction, Utc::now()),
                "until": sanction.until.map(|u| u.to_rfc3339()),
                "count": sanction.count,
            })),
        )
            .into_response(),
        Refused::Backend(why) => {
            tracing::error!(%why, "the store failed while admitting an account");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        _ => StatusCode::UNAUTHORIZED.into_response(),
    }
}

pub(crate) fn refusal(refused: Refused) -> Response {
    // The one refusal whose text depends on the request.
    if let Refused::Banned(sanction) = &refused {
        return banned_page(sanction).into_response();
    }
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
        .into_response()
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

pub(crate) fn complaint_for(refused: &Refused) -> String {
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

pub(crate) fn shell(title: &str, body: Markup) -> Markup {
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
    use std::sync::Arc;

    use super::*;
    use crate::attempts::Attempts;
    use crate::config::Config;
    use crate::store::Store;
    use crate::ticket::Keys;

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
