//! The user index: a filter form, a sortable table, and a pager.
//!
//! The partial is the `<section id="user-index">` element, replacing itself `outerHTML` — so
//! it carries its own htmx attributes and canonical URL back with it.
//!
//! **The filter form sits outside the swapped region**, or every keystroke would replace the
//! input being typed into and take the caret with it.
//!
//! Every control is a real `href` or a `method="get"` form; htmx makes the same URLs swaps.

use chrono::{DateTime, Utc};
use maud::{Markup, html};

use crate::listing::{Dir, Listing, PER_PAGE, Rank, Sort, Standing};
use crate::users::{Page, Row};

pub fn page(page: &Page, now: DateTime<Utc>) -> Markup {
    html! {
        section class="stack" {
            h1 { "Users" }
            (filters(&page.listing))
            (region(page, now))
        }
    }
}

/// The part htmx replaces.
pub fn region(page: &Page, now: DateTime<Utc>) -> Markup {
    let listing = &page.listing;
    html! {
        section id="user-index"
            // The server owns what canonical means; `ts/params.ts` applies it, history being the
            // one thing a server cannot reach.
            data-canonical=(listing.page_url())
            // Explicit in htmx 4: without `:inherited` a pager link replaces itself with a whole
            // table.
            hx-target:inherited="#user-index"
            hx-swap:inherited="outerHTML"
        {
            (summary(page))
            @if page.rows.is_empty() {
                (empty(page))
            } @else {
                table class="index" {
                    thead {
                        tr {
                            @for sort in Sort::ALL { (heading(listing, sort)) }
                        }
                    }
                    tbody {
                        @for row in &page.rows { (line(row, now)) }
                    }
                }
                (pager(page))
            }
        }
    }
}

fn filters(listing: &Listing) -> Markup {
    html! {
        form id="filters" class="filters"
            // What the browser needs to canonicalise the address bar: the paths, the target,
            // and the very defaults `Listing::query_string` drops parameters against. In an
            // attribute rather than in a `<script type="application/json">` block, because a
            // script element's content is raw text — entities are not decoded inside one, so
            // the escaping maud correctly applies would arrive at `JSON.parse` as `&quot;`.
            // An attribute value is decoded by the parser, so the escaping round-trips.
            data-listing=(Listing::defaults_json().to_string())
            // Without JavaScript this submits and reloads: slow, not broken.
            method="get" action=(crate::routes::USERS)
            hx-get=(crate::routes::USER_ROWS)
            // htmx 4 sends no enclosing form values on a GET. This line is what makes the filters
            // work.
            hx-include="this"
            hx-target="#user-index"
            hx-swap="outerHTML"
            // **Two spellings are silently wrong and both were written first.** `input
            // changed` never fires: `changed` compares the value of the element the trigger
            // is on, and a form has none. And `target:(#q)` — the parenthesised form the docs
            // give for selectors with whitespace — matches nothing, the parentheses not being
            // stripped. Both look like a search box that does nothing.
            //
            // `target:` at all, because a `<select>` emits `input` beside `change`.
            hx-trigger="change, input target:#q delay:300ms, submit"
            hx-sync="this:replace"
            hx-indicator="#index-working"
        {
            div class="field field-search" {
                label for="q" { "Search" }
                input id="q" name="q" type="search" value=(listing.q)
                    placeholder="name or address" autocomplete="off";
            }
            div class="field" {
                label for="level" { "Level" }
                select id="level" name="level" {
                    @for rank in Rank::all() {
                        option value=(rank.slug()) selected[rank == listing.rank] {
                            (rank.label())
                        }
                    }
                }
            }
            div class="field" {
                label for="standing" { "Standing" }
                select id="standing" name="standing" {
                    @for standing in Standing::ALL {
                        option value=(standing.slug()) selected[standing == listing.standing] {
                            (standing.label())
                        }
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
            // The ordering rides along; the page number does not, because a new filter starts at
            // the first page and the absent parameter is what says so.
            input type="hidden" name="sort" value=(listing.sort.slug());
            input type="hidden" name="dir" value=(listing.dir.slug());
            noscript { button type="submit" { "Apply" } }
            span id="index-working" class="htmx-indicator" aria-live="polite" { "Working…" }
        }
    }
}

fn summary(page: &Page) -> Markup {
    let listing = &page.listing;
    html! {
        p class="index-summary" {
            @if page.total == 0 {
                "No accounts match."
            } @else {
                "Showing " (page.first()) "–" (page.last()) " of " (page.total) "."
            }
            @if !listing.q.is_empty() || listing.rank != Rank::Any || listing.standing != Standing::Any {
                " "
                a class="clear-filters" href=(crate::routes::USERS)
                    hx-get=(crate::routes::USER_ROWS) { "Clear filters" }
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
    // Without `aria-sort` the arrow is decoration only sighted people can read.
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

/// **The whole row is the link** — one anchor in the name cell, stretched over the row by
/// CSS. Three cells each wrapping their own would be three tab stops and three announcements
/// naming one account.
fn line(row: &Row, now: DateTime<Utc>) -> Markup {
    html! {
        tr class={ "row-link" @if row.is_banned() { " is-banned" } } {
            th scope="row" class="cell-name" {
                a href=(crate::routes::user_url(row.id)) { (row.display_name) }
                @if let Some(email) = &row.email {
                    span class="cell-email" { (email) }
                }
            }
            td { (super::level_badge(row.level)) }
            td class="cell-standing" { (standing_of(row, now)) }
        }
    }
}

fn standing_of(row: &Row, now: DateTime<Utc>) -> Markup {
    html! {
        @if !row.is_banned() {
            span class="badge badge-clear" { "Clear" }
        } @else if row.permanent {
            span class="badge badge-banned" { "Banned" }
            span class="standing-until" { "permanent" }
        } @else if let Some(until) = row.until {
            span class="badge badge-banned" { "Banned" }
            span class="standing-until" { (super::how_long(until, now)) " left" }
        }
        @if row.in_force > 1 {
            // Or somebody waits out the one expiry shown and is refused again.
            span class="standing-count" { "×" (row.in_force) }
        }
    }
}

fn empty(page: &Page) -> Markup {
    let listing = &page.listing;
    html! {
        p class="empty" {
            // "Nothing matches" and "you paged past the end" are different answers.
            @if page.total == 0 && listing.page > 1 {
                "There is nothing on page " (listing.page) "."
                " "
                a href=(listing.at_page(1).page_url())
                    hx-get=(listing.at_page(1).partial_url()) { "Back to the first page" }
            } @else {
                "No account matches those filters."
            }
        }
    }
}

/// Numbered links stop a few either side: one per page is four hundred of them at ten
/// thousand accounts.
fn pager(page: &Page) -> Markup {
    let listing = &page.listing;
    let pages = page.pages();
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
    use lc_identity::level::Level;
    use uuid::Uuid;

    fn rows(n: usize) -> Vec<Row> {
        (0..n)
            .map(|i| Row {
                id: Uuid::new_v4(),
                display_name: format!("Account {i}"),
                level: Level::PLAYER,
                email: Some(format!("a{i}@example.test")),
                in_force: 0,
                permanent: false,
                until: None,
            })
            .collect()
    }

    fn a_page(listing: Listing, total: i64, shown: usize) -> Page {
        Page {
            rows: rows(shown),
            total,
            listing,
        }
    }

    /// The region carries everything a swap needs to keep working: its own id, its own
    /// canonical URL, and the inheritance the links below it rely on. Losing any one of them
    /// breaks the *second* interaction rather than the first, which is the kind of bug that
    /// reaches production.
    #[test]
    fn the_swapped_region_is_self_contained() {
        let markup = region(&a_page(Listing::default(), 100, 25), Utc::now()).into_string();
        assert!(markup.contains(r#"id="user-index""#), "{markup}");
        assert!(markup.contains(r#"data-canonical="/users""#), "{markup}");
        assert!(
            markup.contains(r##"hx-target:inherited="#user-index""##),
            "htmx 4 needs the modifier or nothing inherits: {markup}",
        );
        assert!(
            markup.contains(r#"hx-swap:inherited="outerHTML""#),
            "{markup}"
        );
    }

    /// Three columns, and every one of them sorts. A column with no heading would be an
    /// ordering reachable only by editing the URL.
    #[test]
    fn the_table_has_a_heading_for_every_ordering_and_no_others() {
        let markup = region(&a_page(Listing::default(), 100, 3), Utc::now()).into_string();
        assert_eq!(
            markup.matches("<th scope=\"col\"").count(),
            Sort::ALL.len(),
            "{markup}",
        );
        for sort in Sort::ALL {
            assert!(
                markup.contains(sort.heading()),
                "no {} column",
                sort.heading()
            );
        }
        // The two that left. Neither is lost: the addresses sit under each name, and the
        // providers and the joined date are on the user page.
        assert!(!markup.contains("Sign-in"), "{markup}");
        assert!(!markup.contains("Joined"), "{markup}");
    }

    /// The whole row is the link, and it is **one** link: three cells each wrapping their own
    /// anchor would be three tab stops and three announcements naming one account.
    #[test]
    fn a_row_is_one_link_over_the_whole_row() {
        let markup = region(&a_page(Listing::default(), 1, 1), Utc::now()).into_string();
        let row = markup
            .split("<tbody>")
            .nth(1)
            .and_then(|t| t.split("</tbody>").next())
            .expect("a body");
        assert_eq!(row.matches("<a ").count(), 1, "not one link per row: {row}");
        assert!(row.contains(r#"class="row-link"#), "{row}");
        // And the link's text is the account's name, not "view" or an identifier.
        assert!(row.contains(">Account 0</a>"), "{row}");
    }

    /// Every control is a real link or a real form as well as an htmx one. With scripting off
    /// the console is slower and entirely usable, and that is a property worth asserting
    /// because it is invisible while scripting is on.
    #[test]
    fn every_control_works_without_javascript() {
        let listing = Listing {
            page: 2,
            ..Listing::default()
        };
        let markup = region(&a_page(listing, 100, 25), Utc::now()).into_string();
        // Every anchor that swaps also navigates. Not every anchor swaps — a row's name is a
        // link to another page, and a link to another page has nothing to swap.
        let mut swapping = 0;
        for anchor in markup.split("<a ").skip(1) {
            let tag = anchor.split('>').next().expect("a closing bracket");
            if !tag.contains("hx-get=") {
                continue;
            }
            swapping += 1;
            assert!(tag.contains("href="), "an htmx-only control: <a {tag}>");
        }
        assert!(
            swapping > 0,
            "nothing in the region swaps at all:\n{markup}"
        );

        let form = filters(&Listing::default()).into_string();
        assert!(form.contains(r#"method="get""#), "{form}");
        assert!(form.contains(r#"action="/users""#), "{form}");
        assert!(form.contains("<noscript>"), "{form}");
    }

    /// The configuration the browser reads has to survive being written into an attribute and
    /// parsed back out. In a `<script>` block it would not: a script element is raw text, so
    /// the escaped quotes would reach `JSON.parse` as `&quot;`.
    #[test]
    fn the_browser_can_read_the_listing_configuration_back() {
        let form = filters(&Listing::default()).into_string();
        let start = form.find("data-listing=\"").expect("the attribute") + 14;
        let end = start + form[start..].find('"').expect("the closing quote");
        let escaped = &form[start..end];
        assert!(
            escaped.contains("&quot;"),
            "maud stopped escaping: {escaped}"
        );

        // What a browser's HTML parser hands to `dataset.listing`.
        let decoded = escaped
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&lt;", "<")
            .replace("&gt;", ">");
        let parsed: serde_json::Value = serde_json::from_str(&decoded).expect("valid JSON");
        assert_eq!(parsed, Listing::defaults_json());
    }

    /// Two things about the trigger, both of which fail silently and look right.
    ///
    /// `changed` on a form is never satisfied, because a form has no value to have changed —
    /// so nothing typed into the search box would ever be sent. And an `input` trigger that is
    /// not confined to the search box fires for every menu as well, because a `<select>` emits
    /// `input` beside `change`.
    #[test]
    fn the_search_box_debounces_and_the_menus_do_not_fire_twice() {
        let form = filters(&Listing::default()).into_string();
        assert!(form.contains("input target:#q delay:300ms"), "{form}");
        assert!(
            !form.contains("input changed"),
            "`changed` on a form is never true: {form}",
        );
        assert!(
            !form.contains("target:("),
            "a parenthesised trigger selector matches nothing: {form}",
        );
        // And the selector has something to find.
        assert!(form.contains(r#"id="q""#), "{form}");
    }

    /// The filter form has to tell htmx 4 to send its own values. Without it every filter
    /// silently does nothing, and the table refreshes to show exactly what it already showed.
    #[test]
    fn the_filter_form_sends_its_values() {
        let form = filters(&Listing::default()).into_string();
        assert!(form.contains(r#"hx-include="this""#), "{form}");
        // And it carries the ordering, so changing a filter does not reset the sort.
        assert!(form.contains(r#"name="sort""#), "{form}");
        assert!(form.contains(r#"name="dir""#), "{form}");
        // But not the page, because a new filter starts at the first one.
        assert!(!form.contains(r#"name="page""#), "{form}");
    }

    #[test]
    fn a_heading_flips_its_own_column_and_says_which_way() {
        let listing = Listing::default();
        let name = heading(&listing, Sort::Name).into_string();
        assert!(name.contains(r#"aria-sort="ascending""#), "{name}");
        assert!(name.contains("dir=desc"), "clicking did not flip: {name}");

        let other = heading(&listing, Sort::Level).into_string();
        assert!(!other.contains("aria-sort"), "{other}");
        assert!(other.contains("sort=level"), "{other}");
        assert!(
            !other.contains("dir="),
            "a fresh column asked for a direction: {other}"
        );
    }

    /// A pager that renders one link per page renders four hundred of them eventually.
    #[test]
    fn the_pager_is_bounded_however_many_pages_there_are() {
        let listing = Listing {
            page: 200,
            ..Listing::default()
        };
        let page = a_page(listing, 10_000, 25);
        assert_eq!(page.pages(), 400);
        let markup = pager(&page).into_string();
        assert!(markup.matches("<a ").count() <= 10, "{markup}");
        assert!(markup.contains(r#"aria-current="page""#), "{markup}");
        assert!(markup.contains("page=199"), "no previous page: {markup}");
        assert!(markup.contains("page=201"), "no next page: {markup}");
        assert!(markup.contains("page=400"), "no last page: {markup}");

        // One page is no pager at all rather than a pager with nothing in it.
        assert_eq!(pager(&a_page(Listing::default(), 5, 5)).into_string(), "");
    }

    /// "No accounts match" and "you have paged past the end" are different answers.
    #[test]
    fn an_empty_page_says_which_kind_of_empty_it_is() {
        let past_the_end = a_page(
            Listing {
                page: 9,
                ..Listing::default()
            },
            0,
            0,
        );
        let markup = empty(&past_the_end).into_string();
        assert!(markup.contains("nothing on page 9"), "{markup}");
        assert!(markup.contains("Back to the first page"), "{markup}");

        let no_matches = a_page(Listing::default(), 0, 0);
        assert!(
            empty(&no_matches)
                .into_string()
                .contains("No account matches")
        );
    }

    /// Several bans at once must be visible in the list, or an administrator lifts one and is
    /// surprised the account is still out.
    #[test]
    fn a_row_shows_when_more_than_one_ban_is_in_force() {
        let now = Utc::now();
        let mut row = rows(1).pop().unwrap();
        row.in_force = 3;
        row.until = Some(now + chrono::Duration::days(4));
        let markup = standing_of(&row, now).into_string();
        assert!(markup.contains("Banned"), "{markup}");
        assert!(markup.contains("4 d left"), "{markup}");
        assert!(markup.contains("×3"), "{markup}");

        row.permanent = true;
        assert!(standing_of(&row, now).into_string().contains("permanent"));

        row.in_force = 0;
        row.permanent = false;
        row.until = None;
        assert!(standing_of(&row, now).into_string().contains("Clear"));
    }
}
