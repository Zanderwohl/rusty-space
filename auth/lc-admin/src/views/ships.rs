//! The ships index: every ship the shard has launched, newest first.
//!
//! [`super::systems`]'s shape. A row links to its ship and, separately, to its pilot, so unlike a
//! user row it is not a single link.

use std::collections::HashMap;

use maud::{Markup, html};
use uuid::Uuid;

use crate::fleet::{Flying, Listing, PER_PAGE};
use crate::shard::{Missing, ShipPage, ShipRow};
use crate::users::Pilot;

pub fn page(listing: &Listing, found: &Result<ShipPage, Missing>, pilots: &HashMap<Uuid, Pilot>) -> Markup {
    html! {
        section class="stack" {
            h1 { "Ships" }
            (filters(listing))
            (region(listing, found, pilots))
        }
    }
}

/// The part htmx replaces.
pub fn region(listing: &Listing, found: &Result<ShipPage, Missing>, pilots: &HashMap<Uuid, Pilot>) -> Markup {
    html! {
        section id="ship-index"
            data-canonical=(listing.page_url())
            hx-target:inherited="#ship-index"
            hx-swap:inherited="outerHTML"
        {
            @match found {
                Err(missing) => p class="empty" { (missing.said()) },
                Ok(page) => {
                    (summary(listing, page))
                    @if page.ships.is_empty() {
                        p class="empty" {
                            @if page.total > 0 {
                                "There is nothing on page " (listing.page) ". "
                                a href=(listing.at_page(1).page_url()) hx-get=(listing.at_page(1).partial_url()) { "Back to the first page" }
                            } @else {
                                "No ship matches."
                            }
                        }
                    } @else {
                        (table(&page.ships, pilots))
                        (pager(listing, page.total.max(0) as u64))
                    }
                }
            }
        }
    }
}

/// Also the user page's list of an account's ships.
pub fn table(ships: &[ShipRow], pilots: &HashMap<Uuid, Pilot>) -> Markup {
    html! {
        table class="records" {
            thead {
                tr {
                    th scope="col" { "Ship" }
                    th scope="col" { "Pilot" }
                    th scope="col" { "Launched" }
                    th scope="col" { "Fate" }
                }
            }
            tbody {
                @for ship in ships { (line(ship, pilots)) }
            }
        }
    }
}

fn line(ship: &ShipRow, pilots: &HashMap<Uuid, Pilot>) -> Markup {
    html! {
        tr {
            th scope="row" class="cell-name" {
                a href=(crate::routes::ship_url(ship.ship_id)) { (super::ship::name(ship)) }
                span class="cell-email" { (ship.ship_id) }
            }
            td { (super::ship::pilot(ship.account.as_deref(), pilots)) }
            td { (super::ship::at(ship.launched_at)) }
            td { (super::ship::fate(ship)) }
        }
    }
}

fn filters(listing: &Listing) -> Markup {
    html! {
        form id="filters" class="filters"
            data-listing=(Listing::defaults_json().to_string())
            method="get" action=(crate::routes::SHIPS)
            hx-get=(crate::routes::SHIP_ROWS)
            hx-include="this"
            hx-target="#ship-index"
            hx-swap="outerHTML"
            hx-trigger="change, input target:#q delay:300ms, submit"
            hx-sync="this:replace"
            hx-indicator="#index-working"
        {
            div class="field field-search" {
                label for="q" { "Search" }
                input id="q" name="q" type="search" value=(listing.q)
                    placeholder="name or account" autocomplete="off";
            }
            div class="field" {
                label for="state" { "Fate" }
                select id="state" name="state" {
                    @for state in Flying::ALL {
                        option value=(state.slug()) selected[state == listing.state] { (state.label()) }
                    }
                }
            }
            div class="field" {
                label for="per" { "Per page" }
                select id="per" name="per" {
                    @for per in PER_PAGE {
                        option value=(per) selected[per == listing.per] { (per) }
                    }
                }
            }
            noscript { button type="submit" { "Apply" } }
            span id="index-working" class="htmx-indicator" aria-live="polite" { "Working…" }
        }
    }
}

fn summary(listing: &Listing, page: &ShipPage) -> Markup {
    let first = listing.offset() + 1;
    let last = listing.offset() + page.ships.len() as u64;
    html! {
        p class="index-summary" {
            @if page.total == 0 {
                "No ship matches."
            } @else {
                "Showing " (first) "–" (last) " of " (page.total) "."
            }
            @if !listing.q.is_empty() || listing.state != Flying::Any {
                " "
                a class="clear-filters" href=(crate::routes::SHIPS) hx-get=(crate::routes::SHIP_ROWS) { "Clear filters" }
            }
        }
    }
}

fn pager(listing: &Listing, total: u64) -> Markup {
    let pages = listing.pages(total);
    if pages <= 1 {
        return html! {};
    }
    let here = listing.page.min(pages);
    html! {
        nav class="pager" aria-label="Pages" {
            @if here > 1 {
                a class="step" rel="prev" href=(listing.at_page(here - 1).page_url())
                    hx-get=(listing.at_page(here - 1).partial_url()) { "← Previous" }
            } @else {
                span class="step step-off" { "← Previous" }
            }
            span class="page-here" aria-current="page" { (here) " of " (pages) }
            @if here < pages {
                a class="step" rel="next" href=(listing.at_page(here + 1).page_url())
                    hx-get=(listing.at_page(here + 1).partial_url()) { "Next →" }
            } @else {
                span class="step step-off" { "Next →" }
            }
        }
    }
}
