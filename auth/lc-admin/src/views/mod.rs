//! The chrome, and the small things every page renders the same way.
//!
//! Server-rendered HTML throughout. htmx swaps fragments of it; nothing here builds a page in
//! the browser, and the two handlers that answer a swap render the same function the full page
//! does. A partial that drifts from the page it lives in is the failure mode of this whole
//! approach, and rendering both from one function is what prevents it rather than a convention
//! about remembering to.

pub mod index;
pub mod user;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use lc_identity::level::Level;
use maud::{DOCTYPE, Markup, html};

use crate::assets::Assets;
use crate::auth::Admin;

pub struct Head<'a> {
    pub title: &'a str,
}

/// The document. Every page goes through this and nothing else emits a `<!DOCTYPE>`.
pub fn shell(assets: &Assets, head: Head<'_>, admin: Option<&Admin>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (head.title) " — Lightcone Frontier administration" }
                // Nothing here is for anyone but an administrator, and an administration
                // console in somebody's search results is an administration console being
                // probed.
                meta name="robots" content="noindex, nofollow";
                // htmx 4 stopped inheriting attributes implicitly, and this service leans on
                // that: `:inherited` is written where inheritance is wanted, so nothing
                // inherits a target by accident. Stated rather than left to the default in
                // case the default ever moves.
                meta name="htmx-config" content="implicitInheritance:false, defaultSwap:innerHTML";
                link rel="stylesheet" href=(assets.url(crate::assets::STYLESHEET));
                // Deferred, both of them. htmx processes the document on load and the module
                // listens for its events; neither needs to run before the body exists, and a
                // console that renders only once two scripts have parsed is a console that
                // looks broken on a slow connection.
                script src=(assets.url(crate::assets::HTMX)) defer {}
                script src=(assets.url(crate::assets::SCRIPT)) type="module" {}
            }
            body {
                header class="masthead" {
                    a class="wordmark" href=(crate::routes::USERS) { "Lightcone" }
                    span class="wordmark-tag" { "administration" }
                    nav {
                        a href=(crate::routes::USERS) { "Users" }
                    }
                    @if let Some(admin) = admin {
                        div class="whoami" {
                            span class="whoami-name" { (admin.name) }
                            (level_badge(admin.level))
                            a class="signout" href=(crate::routes::SIGNOUT) { "Sign out" }
                        }
                    }
                }
                main class="page" { (body) }
            }
        }
    }
}

/// A page that is only a message: a refusal, or something that went wrong.
///
/// Not behind the [`Admin`] extractor, so it renders without a stylesheet URL that needs
/// state. The chrome is the plain one on purpose — somebody seeing this is not somebody the
/// console should be offering navigation to.
pub fn wrong(assets: &Assets, status: StatusCode, heading: &str, said: &str) -> Response {
    message(assets, status, heading, said, None)
}

/// The same, with a link.
///
/// **Any page a person can land on and stay on needs one of these.** The console's only
/// navigation is a masthead that renders when you are an administrator, so a refusal without
/// a link is a dead end: no menu, no back button that helps, and nothing on screen naming the
/// address that would get you out. That is not a theoretical failure — "Not for you" shipped
/// without one and stranded the first account that met it.
pub fn refusal(
    assets: &Assets,
    status: StatusCode,
    heading: &str,
    said: &str,
    way_out: (&str, &str),
) -> Response {
    message(assets, status, heading, said, Some(way_out))
}

/// The chrome a message page gets: the stylesheet and the wordmark, and **no navigation**.
///
/// Not [`shell`], which renders a nav bar — half of these pages are seen by somebody who may
/// not have the pages that bar links to, and a link that leads to another refusal is worse
/// than no link. The way out, where there is one, is named by the caller, because only the
/// caller knows what it is.
fn message(
    assets: &Assets,
    status: StatusCode,
    heading: &str,
    said: &str,
    way_out: Option<(&str, &str)>,
) -> Response {
    (
        status,
        maud::html! {
            (DOCTYPE)
            html lang="en" {
                head {
                    meta charset="utf-8";
                    meta name="viewport" content="width=device-width, initial-scale=1";
                    meta name="robots" content="noindex, nofollow";
                    title { (heading) " — Lightcone Frontier administration" }
                    link rel="stylesheet" href=(assets.url(crate::assets::STYLESHEET));
                }
                body {
                    header class="masthead" {
                        span class="wordmark" { "Lightcone" }
                        span class="wordmark-tag" { "administration" }
                    }
                    main class="page" {
                        section class="stack" {
                            h1 { (heading) }
                            p { (said) }
                            @if let Some((href, label)) = way_out {
                                p { a class="way-out" href=(href) { (label) } }
                            }
                        }
                    }
                }
            }
        },
    )
        .into_response()
}

/// A level, as a badge. One rendering, so the index and the user page cannot disagree about
/// what an administrator looks like.
pub fn level_badge(level: Level) -> Markup {
    html! {
        span class={ "badge badge-level badge-" (level.slug()) } { (level.name()) }
    }
}

/// A timestamp a person reads, with the machine-readable one alongside.
pub fn when(at: DateTime<Utc>) -> Markup {
    html! {
        time datetime=(at.to_rfc3339()) { (at.format("%Y-%m-%d")) }
    }
}

/// The same, to the minute, for things where the minute matters.
pub fn when_exact(at: DateTime<Utc>) -> Markup {
    html! {
        time datetime=(at.to_rfc3339()) { (at.format("%Y-%m-%d %H:%M")) " UTC" }
    }
}

/// How long until `at`, in the coarsest unit that is still honest.
///
/// The counterpart of `lc_identity::signin::how_long`, which says the same thing to the person
/// being refused. Two renderings because the audiences differ: this one is terse and sits in a
/// table cell, that one is a sentence on a page somebody is reading in dismay.
pub fn how_long(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let left = at - now;
    if left.num_seconds() <= 0 {
        "over".to_owned()
    } else if left.num_hours() < 1 {
        format!("{} min", left.num_minutes().max(1))
    } else if left.num_days() < 1 {
        format!("{} h", left.num_hours())
    } else if left.num_days() < 60 {
        format!("{} d", left.num_days())
    } else {
        format!("{} mo", left.num_days() / 30)
    }
}

/// A message at the top of a region, after something was done or refused.
pub fn note(kind: &str, said: &str) -> Markup {
    html! {
        p class={ "note note-" (kind) } role="status" { (said) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal is somewhere a person stops. Without a link they stop there permanently:
    /// this console has no navigation except a masthead that only administrators are shown.
    #[test]
    fn a_refusal_names_a_way_out_and_a_plain_error_does_not_have_to() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("static");
        let Ok(assets) = Assets::load(&root) else {
            // A checkout where `npm run build` has not run. `assets::tests` is the test that
            // is about that; this one is about the markup.
            return;
        };

        let refused = refusal(
            &assets,
            StatusCode::FORBIDDEN,
            "Not for you",
            "This account administers nothing.",
            (crate::routes::SIGNIN, "Sign in as another account"),
        );
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);

        let plain = wrong(
            &assets,
            StatusCode::NOT_FOUND,
            "No page here",
            "The address is wrong.",
        );
        assert_eq!(plain.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_time_that_has_passed_does_not_read_as_time_remaining() {
        let now = Utc::now();
        assert_eq!(how_long(now - chrono::Duration::days(1), now), "over");
        assert_eq!(how_long(now, now), "over");
        assert_eq!(how_long(now + chrono::Duration::hours(5), now), "5 h");
        assert_eq!(how_long(now + chrono::Duration::days(3), now), "3 d");
        assert_eq!(how_long(now + chrono::Duration::days(200), now), "6 mo");
        // Never zero of anything: something still in force reads as still in force.
        assert_eq!(how_long(now + chrono::Duration::seconds(10), now), "1 min");
    }

    #[test]
    fn every_level_has_its_own_badge_class() {
        let mut classes: Vec<String> = Level::ALL
            .iter()
            .map(|l| level_badge(*l).into_string())
            .collect();
        classes.sort();
        classes.dedup();
        assert_eq!(classes.len(), Level::ALL.len(), "two levels render alike");
    }
}
