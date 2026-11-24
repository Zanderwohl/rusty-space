//! One ship, as an administrator sees it.
//!
//! Bans are not issued here: they are an account's, and stop every ship it has and any it
//! would launch. The page links to the pilot, where they are.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use maud::{Markup, html};
use uuid::Uuid;

use crate::shard::{Missing, ShipDetail, ShipRow};
use crate::users::Pilot;

pub fn title(found: &Result<ShipDetail, Missing>) -> String {
    match found {
        Ok(detail) => detail.ship.name.clone().unwrap_or_else(|| format!("Ship {}", detail.ship.ship_id)),
        Err(_) => "Ship".to_owned(),
    }
}

pub fn page(found: &Result<ShipDetail, Missing>, pilots: &HashMap<Uuid, Pilot>) -> Markup {
    html! {
        section class="stack" {
            p class="crumbs" { a href=(crate::routes::SHIPS) { "← All ships" } }
            @match found {
                Err(missing) => p class="empty" { (missing.said()) },
                Ok(detail) => {
                    header class="person" {
                        h1 { (name(&detail.ship)) }
                        p class="person-badges" { (fate_badge(&detail.ship)) }
                        dl class="person-facts" {
                            dt { "Ship" }
                            dd { code { (detail.ship.ship_id) } }
                            dt { "Pilot" }
                            dd { (pilot(detail.ship.account.as_deref(), pilots)) }
                            dt { "Launched" }
                            dd { (at(detail.ship.launched_at)) }
                            @if let Some(ended) = detail.ship.ended_at {
                                dt { "Collapsed" }
                                dd { (at(ended)) }
                            }
                        }
                    }
                    section class="panel" {
                        h2 { "Status" }
                        @match &detail.status {
                            None => p class="nothing" { "The shard no longer keeps this ship's state." },
                            Some(status) => div class="cards" {
                                (super::user::where_card(status))
                                (super::user::fit_card(status))
                            },
                        }
                    }
                }
            }
        }
    }
}

pub fn name(ship: &ShipRow) -> Markup {
    html! {
        @match &ship.name {
            Some(name) => (name),
            None => span class="nothing" { "Unnamed" },
        }
    }
}

/// The account that launched it, linked, with its standing when a ban is in force.
pub fn pilot(account: Option<&str>, pilots: &HashMap<Uuid, Pilot>) -> Markup {
    let Some(account) = account else {
        return html! { span class="nothing" { "None" } };
    };
    // A shard's anonymous connections are not the broker's accounts, and a link built from one
    // could only 404.
    let Ok(id) = Uuid::parse_str(account) else {
        return html! { code { (account) } };
    };
    let found = pilots.get(&id);
    let label = super::account_label(found.and_then(|p| p.display_name.as_deref()), found.and_then(|p| p.email.as_deref()), id);
    html! {
        a href=(crate::routes::user_url(id)) { (label) }
        @if found.is_some_and(|p| p.banned) {
            " " span class="badge badge-banned" { "Banned" }
        }
    }
}

pub fn fate(ship: &ShipRow) -> Markup {
    html! {
        @match ship.ended_at {
            None => span class="badge badge-clear" { "Flying" },
            Some(ended) => { "Collapsed " (at(ended)) },
        }
    }
}

fn fate_badge(ship: &ShipRow) -> Markup {
    html! {
        @match ship.ended_at {
            None => span class="badge badge-clear" { "Flying" },
            Some(_) => span class="badge badge-banned" { "Collapsed" },
        }
    }
}

/// Unix seconds, as a date.
pub fn at(unix_s: i64) -> Markup {
    match DateTime::<Utc>::from_timestamp(unix_s, 0) {
        Some(at) => super::when_exact(at),
        None => html! { span class="nothing" { "—" } },
    }
}

/// Every account a page of ships names, for [`crate::users::pilots`].
pub fn accounts_of<'a>(ships: impl IntoIterator<Item = &'a ShipRow>) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = ships.into_iter().filter_map(|s| Uuid::parse_str(s.account.as_deref()?).ok()).collect();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(account: Option<&str>, ended_at: Option<i64>) -> ShipRow {
        ShipRow { ship_id: 9, account: account.map(Into::into), name: Some("Canterbury".into()), launched_at: 1_794_000_000, ended_at }
    }

    #[test]
    fn a_ship_links_to_its_pilot_and_says_when_the_pilot_is_banned() {
        let id = Uuid::new_v4();
        let pilots = HashMap::from([(id, Pilot { display_name: None, email: Some("holden@example.com".into()), banned: true })]);
        let detail = ShipDetail { ship: row(Some(&id.to_string()), Some(1_794_500_000)), status: None };
        let page = page(&Ok(detail), &pilots).into_string();
        assert!(page.contains(&crate::routes::user_url(id)), "{page}");
        assert!(page.contains("holden@example.com") && page.contains("Banned"), "{page}");
        assert!(page.contains("Collapsed") && page.contains("no longer keeps"), "{page}");
    }

    #[test]
    fn an_anonymous_pilot_is_shown_and_not_linked() {
        let markup = pilot(Some("anonymous:3"), &HashMap::new()).into_string();
        assert!(markup.contains("anonymous:3") && !markup.contains("href"), "{markup}");
        assert_eq!(accounts_of(&[row(Some("anonymous:3"), None)]), Vec::<Uuid>::new());
    }
}
