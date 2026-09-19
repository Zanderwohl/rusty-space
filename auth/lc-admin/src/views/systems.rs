//! The systems index: every system this shard holds.
//!
//! [`super::index`]'s shape, and its reasons. What differs is that the rows arrive from the
//! shard already paged, so this renders what it was handed and counts nothing.
//!
//! **There is no view for one system**: a row that links somewhere empty is worse than one
//! that does not link.

use maud::{Markup, html};

use crate::catalogue::{Dir, Listing, PER_PAGE, Sort};
use crate::shard::{Missing, System, Systems};

pub fn page(listing: &Listing, found: &Result<Systems, Missing>) -> Markup {
    html! {
        section class="stack" {
            h1 { "Systems" }
            (filters(listing))
            (region(listing, found))
        }
    }
}

/// The part htmx replaces.
pub fn region(listing: &Listing, found: &Result<Systems, Missing>) -> Markup {
    html! {
        section id="system-index"
            data-canonical=(listing.page_url())
            hx-target:inherited="#system-index"
            hx-swap:inherited="outerHTML"
        {
            @match found {
                // A shard that is down says so here: the console administers accounts without one.
                Err(missing) => p class="empty" { (missing.said()) },
                Ok(systems) => {
                    (summary(listing, systems))
                    @if systems.systems.is_empty() {
                        (empty(listing, systems))
                    } @else {
                        table class="index" {
                            thead {
                                tr {
                                    @for sort in Sort::ALL { (heading(listing, sort)) }
                                }
                            }
                            tbody {
                                @for system in &systems.systems { (line(system)) }
                            }
                        }
                        (pager(listing, systems.total))
                    }
                }
            }
        }
    }
}

fn filters(listing: &Listing) -> Markup {
    html! {
        form id="filters" class="filters"
            data-listing=(Listing::defaults_json().to_string())
            method="get" action=(crate::routes::SYSTEMS)
            hx-get=(crate::routes::SYSTEM_ROWS)
            hx-include="this"
            hx-target="#system-index"
            hx-swap="outerHTML"
            hx-trigger="change, input target:#q delay:300ms, submit"
            hx-sync="this:replace"
            hx-indicator="#index-working"
        {
            div class="field field-search" {
                label for="q" { "Search" }
                input id="q" name="q" type="search" value=(listing.q)
                    placeholder="name or identifier" autocomplete="off";
            }
            div class="field" {
                label for="per" { "Per page" }
                select id="per" name="per" {
                    @for per in PER_PAGE {
                        option value=(per) selected[per == listing.per] { (per) }
                    }
                }
            }
            input type="hidden" name="sort" value=(listing.sort.slug());
            input type="hidden" name="dir" value=(listing.dir.slug());
            noscript { button type="submit" { "Apply" } }
            span id="index-working" class="htmx-indicator" aria-live="polite" { "Working…" }
        }
    }
}

fn summary(listing: &Listing, systems: &Systems) -> Markup {
    let first = listing.offset() + 1;
    let last = listing.offset() + systems.systems.len() as u64;
    html! {
        p class="index-summary" {
            @if systems.total == 0 {
                "No system matches."
            } @else {
                "Showing " (first) "–" (last) " of " (systems.total) "."
            }
            @if !listing.q.is_empty() {
                " "
                a class="clear-filters" href=(crate::routes::SYSTEMS)
                    hx-get=(crate::routes::SYSTEM_ROWS) { "Clear filters" }
            }
        }
    }
}

fn heading(listing: &Listing, sort: Sort) -> Markup {
    let next = listing.sorted_by(sort);
    let current = listing.sort == sort;
    let arrow = match (current, listing.dir) {
        (true, Dir::Asc) => "↑",
        (true, Dir::Desc) => "↓",
        (false, _) => "",
    };
    let sorted = match (current, listing.dir) {
        (true, Dir::Asc) => Some("ascending"),
        (true, Dir::Desc) => Some("descending"),
        (false, _) => None,
    };
    html! {
        th scope="col" aria-sort=[sorted] {
            a class={ "sort" @if current { " sort-on" } }
                href=(next.page_url()) hx-get=(next.partial_url()) {
                (sort.heading())
                @if !arrow.is_empty() { span class="arrow" aria-hidden="true" { " " (arrow) } }
            }
        }
    }
}

/// **Not a link**: there is nowhere to go yet.
fn line(system: &System) -> Markup {
    html! {
        tr {
            th scope="row" class="cell-name" {
                @match &system.name {
                    // Its identifier is its name. A blank cell would not be scannable.
                    Some(name) => (name),
                    None => span class="nothing" { "Unnamed" },
                }
                span class="cell-email" { (system.id) }
            }
            td class="cell-count" {
                @if system.ships == 0 {
                    span class="nothing" { "—" }
                } @else {
                    (system.ships)
                }
            }
            td class="cell-count" {
                @if system.objects == 0 {
                    span class="nothing" { "—" }
                } @else {
                    (system.objects)
                }
            }
        }
    }
}

fn empty(listing: &Listing, systems: &Systems) -> Markup {
    html! {
        p class="empty" {
            @if systems.total > 0 {
                "There is nothing on page " (listing.page) "."
                " "
                a href=(listing.at_page(1).page_url())
                    hx-get=(listing.at_page(1).partial_url()) { "Back to the first page" }
            } @else {
                "No system matches that search."
            }
        }
    }
}

/// Bounded, as the user index's is: a catalogue is thousands of systems.
fn pager(listing: &Listing, total: u64) -> Markup {
    let pages = listing.pages(total);
    if pages <= 1 {
        return html! {};
    }
    let here = listing.page.min(pages);
    let window = 2;
    let first_shown = here.saturating_sub(window).max(1);
    let last_shown = (here + window).min(pages);

    html! {
        nav class="pager" aria-label="Pages" {
            @if here > 1 {
                a class="step" rel="prev" href=(listing.at_page(here - 1).page_url())
                    hx-get=(listing.at_page(here - 1).partial_url()) { "← Previous" }
            } @else {
                span class="step step-off" { "← Previous" }
            }

            @if first_shown > 1 {
                a href=(listing.at_page(1).page_url())
                    hx-get=(listing.at_page(1).partial_url()) { "1" }
                @if first_shown > 2 { span class="gap" aria-hidden="true" { "…" } }
            }
            @for n in first_shown..=last_shown {
                @if n == here {
                    span class="page-here" aria-current="page" { (n) }
                } @else {
                    a href=(listing.at_page(n).page_url())
                        hx-get=(listing.at_page(n).partial_url()) { (n) }
                }
            }
            @if last_shown < pages {
                @if last_shown + 1 < pages { span class="gap" aria-hidden="true" { "…" } }
                a href=(listing.at_page(pages).page_url())
                    hx-get=(listing.at_page(pages).partial_url()) { (pages) }
            }

            @if here < pages {
                a class="step" rel="next" href=(listing.at_page(here + 1).page_url())
                    hx-get=(listing.at_page(here + 1).partial_url()) { "Next →" }
            } @else {
                span class="step step-off" { "Next →" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(total: u64, shown: usize) -> Result<Systems, Missing> {
        Ok(Systems {
            total,
            systems: (0..shown)
                .map(|n| System {
                    id: 1000 + n as u64,
                    name: (n == 0).then(|| "Sol".to_owned()),
                    ships: if n == 0 { 3 } else { 0 },
                    objects: 0,
                })
                .collect(),
        })
    }

    /// The region carries everything a swap needs, and targets **its own** id — not the user
    /// index's, which would make one index replace the other.
    #[test]
    fn the_swapped_region_is_self_contained_and_its_own() {
        let markup = region(&Listing::default(), &found(7973, 25)).into_string();
        assert!(markup.contains(r#"id="system-index""#), "{markup}");
        assert!(markup.contains(r#"data-canonical="/systems""#), "{markup}");
        assert!(
            markup.contains(r##"hx-target:inherited="#system-index""##),
            "{markup}"
        );
        assert!(
            !markup.contains("user-index"),
            "it targets the other index: {markup}"
        );
    }

    /// Three columns, each one sortable, and no link to a system: there is nowhere to go.
    #[test]
    fn a_row_names_the_system_and_links_nowhere() {
        let markup = region(&Listing::default(), &found(2, 2)).into_string();
        let body = markup
            .split("<tbody>")
            .nth(1)
            .and_then(|t| t.split("</tbody>").next())
            .expect("a body");
        assert_eq!(
            body.matches("<a ").count(),
            0,
            "a row linked somewhere: {body}"
        );
        assert!(body.contains("Sol"), "{body}");
        assert!(body.contains("1000"), "the identifier is missing: {body}");
        // An unnamed system still shows as something rather than as a blank cell.
        assert!(body.contains("Unnamed"), "{body}");
        // A count of zero reads as none rather than as a nought.
        assert!(body.contains(r#"<span class="nothing">—</span>"#), "{body}");
    }

    /// The summary counts from the shard's total, not from the page in hand — a pager built
    /// from the latter has one page in it whatever the catalogue holds.
    #[test]
    fn the_summary_and_pager_count_the_whole_catalogue() {
        let listing = Listing::default().at_page(2);
        let markup = region(&listing, &found(7973, 25)).into_string();
        assert!(markup.contains("Showing 26–50 of 7973."), "{markup}");
        assert!(markup.contains("page=319"), "no last page: {markup}");
        assert!(
            markup.matches("<a ").count() <= 12,
            "an unbounded pager: {markup}"
        );
    }

    /// A shard that is down says so here, and says nothing alarming.
    #[test]
    fn an_unreachable_shard_is_a_sentence_not_an_error() {
        let markup = region(
            &Listing::default(),
            &Err(Missing::Unreachable("connection refused".into())),
        )
        .into_string();
        assert!(markup.contains("connection refused"), "{markup}");
        assert!(!markup.contains("<table"), "{markup}");
        // And the region is still swappable, so a retry lands somewhere.
        assert!(markup.contains(r#"id="system-index""#), "{markup}");
    }

    /// Every control works without JavaScript, as on the user index.
    #[test]
    fn every_control_is_a_real_link_or_form() {
        let markup = region(&Listing::default().at_page(2), &found(7973, 25)).into_string();
        for anchor in markup.split("<a ").skip(1) {
            let tag = anchor.split('>').next().expect("a closing bracket");
            if tag.contains("hx-get=") {
                assert!(tag.contains("href="), "an htmx-only control: <a {tag}>");
            }
        }
        let form = filters(&Listing::default()).into_string();
        assert!(form.contains(r#"method="get""#), "{form}");
        assert!(form.contains(r#"action="/systems""#), "{form}");
        assert!(form.contains("input target:#q delay:300ms"), "{form}");
        assert!(form.contains(r#"hx-include="this""#), "{form}");
    }
}
