//! The chrome, and the small things every page renders the same way.
//!
//! A page and the partial that replaces part of it render from **one function**. A partial
//! that drifts from its page is the failure mode of this approach, and sharing the function
//! prevents it where a convention about remembering would not.

pub mod index;
pub mod system;
pub mod systems;
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

pub fn shell(assets: &Assets, head: Head<'_>, admin: Option<&Admin>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (head.title) " — Lightcone Frontier administration" }
                // A console in somebody's search results is a console being probed.
                meta name="robots" content="noindex, nofollow";
                // Stated rather than left to htmx 4's default, in case it moves.
                meta name="htmx-config" content="implicitInheritance:false, defaultSwap:innerHTML";
                link rel="stylesheet" href=(assets.url(crate::assets::STYLESHEET));
                // Neither needs to run before the body exists.
                script src=(assets.url(crate::assets::HTMX)) defer {}
                script src=(assets.url(crate::assets::SCRIPT)) type="module" {}
            }
            body {
                header class="masthead" {
                    a class="wordmark" href=(crate::routes::USERS) { "Lightcone" }
                    span class="wordmark-tag" { "administration" }
                    nav {
                        a href=(crate::routes::USERS) { "Users" }
                        a href=(crate::routes::SYSTEMS) { "Systems" }
                    }
                    @if let Some(admin) = admin {
                        div class="whoami" {
                            span class="whoami-name" { (admin.name) }
                            (level_badge(admin.level))
                            // A form rather than a link, because the route is a POST. It
                            // is styled to read as the link it replaces: this is navigation
                            // to the person using it, whatever the method underneath.
                            form class="signout" method="post" action=(crate::routes::SIGNOUT) {
                                button type="submit" { "Sign out" }
                            }
                        }
                    }
                }
                main class="page" { (body) }
            }
        }
    }
}

/// A page that is only a message.
pub fn wrong(assets: &Assets, status: StatusCode, heading: &str, said: &str) -> Response {
    message(assets, status, heading, said, None)
}

/// **Any page a person can land on and stay on needs one of these.** The only navigation is
/// a masthead administrators see, so a refusal without a link is a dead end — which is what
/// "Not for you" shipped as, stranding the first account that met it.
pub fn refusal(
    assets: &Assets,
    status: StatusCode,
    heading: &str,
    said: &str,
    way_out: (&str, &str),
) -> Response {
    message(assets, status, heading, said, Some(way_out))
}

/// **No navigation**: half of these are seen by somebody who cannot open the pages a nav bar
/// links to, and a link to another refusal is worse than none.
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

/// One rendering, so the index and the user page cannot disagree.
pub fn level_badge(level: Level) -> Markup {
    html! {
        span class={ "badge badge-level badge-" (level.slug()) } { (level.name()) }
    }
}

pub fn when(at: DateTime<Utc>) -> Markup {
    html! {
        time datetime=(at.to_rfc3339()) { (at.format("%Y-%m-%d")) }
    }
}

pub fn when_exact(at: DateTime<Utc>) -> Markup {
    html! {
        time datetime=(at.to_rfc3339()) { (at.format("%Y-%m-%d %H:%M")) " UTC" }
    }
}

/// Terse, for a table cell. `lc_identity::signin::how_long` says the same as a sentence to
/// the person being refused.
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
