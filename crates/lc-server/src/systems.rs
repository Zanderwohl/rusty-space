//! The systems this shard holds, as the console lists them.
//!
//! **Where they come from is this module's business.** Today, the star catalog the shard was
//! started with; later, its own `systems` table. The console asks for systems and gets systems,
//! so that move changes this file and nothing it ships — hence `id` and not `star`.
//!
//! Paged, filtered and ordered **here**: handing the console eight thousand systems so it can
//! show twenty-five works today and stops at a size nobody is watching for.

use std::collections::HashMap;

use lc_world::sky::CatalogStar;
use serde::{Deserialize, Serialize};

/// One system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    pub id: u64,
    /// Most of a catalog has none.
    pub name: Option<String>,
    /// As of the last checkpoint.
    pub ships: u64,
    /// **None exist yet**; nothing makes one. Here so the column does not appear later and
    /// move every other one.
    pub objects: u64,
}

/// `total` counts matches, not the page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub total: u64,
    pub systems: Vec<Row>,
}

/// Which column the list is ordered by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Sort {
    #[default]
    Name,
    Ships,
    Objects,
}

impl Sort {
    /// Unrecognized is the default, never an error: a 400 is the wrong answer to a typo in a
    /// URL somebody edited.
    pub fn from_slug(slug: &str) -> Sort {
        match slug {
            "ships" => Sort::Ships,
            "objects" => Sort::Objects,
            _ => Sort::Name,
        }
    }
}

/// What the console asked for.
#[derive(Clone, Debug)]
pub struct Query {
    /// Name, case-insensitively, or the id as text so an identifier can be pasted in.
    pub q: String,
    pub sort: Sort,
    pub descending: bool,
    pub offset: u64,
    pub limit: u64,
}

/// **O(craft × systems)** — a linear nearest-star scan per craft. Nothing at three craft, and
/// the first thing to change at thousands; the fix is an index on the catalog, not a
/// different signature here.
pub fn tally(at: &[glam::DVec3], stars: &[CatalogStar]) -> HashMap<u64, u64> {
    let mut counts = HashMap::new();
    for position in at {
        if let Some(star) = crate::status::nearest_within_shell(*position, stars) {
            *counts.entry(star.id.get()).or_insert(0) += 1;
        }
    }
    counts
}

/// One page of systems.
pub fn page(stars: &[CatalogStar], ships: &HashMap<u64, u64>, query: &Query) -> Page {
    let needle = query.q.trim().to_lowercase();
    let mut rows: Vec<Row> = stars
        .iter()
        .map(|star| {
            let id = star.id.get();
            Row {
                id,
                name: star.provenance.name.clone(),
                ships: ships.get(&id).copied().unwrap_or(0),
                objects: 0,
            }
        })
        .filter(|row| {
            if needle.is_empty() {
                return true;
            }
            row.name
                .as_deref()
                .is_some_and(|name| name.to_lowercase().contains(&needle))
                || row.id.to_string().contains(&needle)
        })
        .collect();

    // **Ends in the id.** Every column here is nearly all ties. A stable sort over the same
    // `Vec` every call hides that; a `SELECT` with no total order, which is where these are
    // going, does not — and then a system lands on two pages and another on none.
    //
    // The direction applies to the primary key only. Reversing the whole list afterwards
    // would reverse the tie-break with it, which is still total and still correct, but it
    // makes two pages of identical rows swap order for no reason a reader could explain.
    rows.sort_by(|a, b| {
        let primary = match query.sort {
            // Unnamed last **whichever way the list runs** — they are the bulk of a
            // catalog — so named-ness is compared outside the direction flip.
            Sort::Name => {
                return match (a.name.is_some(), b.name.is_some()) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => {
                        let key = |r: &Row| r.name.as_deref().unwrap_or("").to_lowercase();
                        let by_name = key(a).cmp(&key(b));
                        let by_name = if query.descending { by_name.reverse() } else { by_name };
                        by_name.then(a.id.cmp(&b.id))
                    }
                };
            }
            Sort::Ships => a.ships.cmp(&b.ships),
            Sort::Objects => a.objects.cmp(&b.objects),
        };
        let primary = if query.descending { primary.reverse() } else { primary };
        primary.then(a.id.cmp(&b.id))
    });
    Page {
        total: rows.len() as u64,
        systems: rows
            .into_iter()
            .skip(query.offset as usize)
            .take(query.limit as usize)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_world::sky::{Component as StarComponent, Provenance, StarId};
    use lc_world::star::Star;

    fn star(key: u64, name: Option<&str>, at: glam::DVec3) -> CatalogStar {
        CatalogStar {
            id: StarId::synthesize("test", key),
            provenance: Provenance {
                source: "test".into(),
                key,
            name: name.map(str::to_owned),
            },
            position_ly: at,
            velocity: glam::DVec3::ZERO,
            star: Star {
                radius_m: 6.957e8,
                teff_k: 5772.0,
                mu: 1.327e20,
                limb_darkening: (0.4, 0.26),
            },
            luminosity_solar: 1.0,
            mass_solar: 1.0,
            metallicity: 0.0,
            component: StarComponent { index: 1, group: None },
        }
    }

    /// Spread far enough apart that no shell overlaps another — `LOCAL_SHELL_LY` is 1.6, so
    /// ten light-years between them leaves no ambiguity about which system a craft is in.
    fn catalog() -> Vec<CatalogStar> {
        (0..10)
            .map(|n| {
                let name = match n {
                    0 => Some("Sol"),
                    1 => Some("Alpha Centauri"),
                    2 => Some("Barnard's Star"),
                    _ => None,
                };
                star(n, name, glam::DVec3::new(n as f64 * 10.0, 0.0, 0.0))
            })
            .collect()
    }

    fn query() -> Query {
        Query { q: String::new(), sort: Sort::Name, descending: false, offset: 0, limit: 25 }
    }

    /// A craft counts toward the system it is **in**, and one between the stars counts toward
    /// nothing — not toward whichever star happens to be nearest.
    #[test]
    fn craft_are_tallied_into_the_system_they_are_inside() {
        let stars = catalog();
        let counts = tally(
            &[
                glam::DVec3::ZERO,                        // in Sol
                glam::DVec3::new(0.5, 0.0, 0.0),          // also in Sol
                glam::DVec3::new(10.0, 0.0, 0.0),         // in the second system
                glam::DVec3::new(5.0, 0.0, 0.0),          // between them, in neither
            ],
            &stars,
        );
        assert_eq!(counts.get(&stars[0].id.get()), Some(&2));
        assert_eq!(counts.get(&stars[1].id.get()), Some(&1));
        assert_eq!(counts.len(), 2, "a craft between the stars was counted: {counts:?}");
    }

    /// **Every page is disjoint and together they are the whole list — whatever order the
    /// systems arrive in.**
    ///
    /// The rotation is the point. `sort_by` is stable, so with the catalog in the same order
    /// every call the pages partition even with no tie-break at all — which is what the first
    /// version of this test proved, which is nothing. Sorting a differently-ordered copy for
    /// each page is what the future actually looks like: these systems are a `Vec` loaded once
    /// today and will be rows from a `SELECT`, and a query with no total order is free to hand
    /// them back differently every time.
    ///
    /// Every column here is nearly all ties — most systems have no name and almost none have
    /// craft — so this is exactly the shape that loses a row between two pages.
    #[test]
    fn the_pages_partition_the_systems_whatever_order_they_arrive_in() {
        let counts = HashMap::new();
        for sort in [Sort::Name, Sort::Ships, Sort::Objects] {
            for descending in [false, true] {
                let mut seen = Vec::new();
                for (turn, offset) in (0..10).step_by(3).enumerate() {
                    // A different arrangement of the same systems for every page.
                    let mut stars = catalog();
                    let by = turn * 3 % stars.len();
                    stars.rotate_left(by);
                    let page = page(&stars, &counts, &Query {
                        sort,
                        descending,
                        offset,
                        limit: 3,
                        ..query()
                    });
                    assert_eq!(page.total, 10);
                    seen.extend(page.systems.into_iter().map(|r| r.id));
                }
                let mut unique = seen.clone();
                unique.sort_unstable();
                unique.dedup();
                assert_eq!(
                    unique.len(),
                    seen.len(),
                    "{sort:?} descending={descending} put a system on two pages",
                );
                assert_eq!(
                    unique.len(),
                    10,
                    "{sort:?} descending={descending} lost a system between pages",
                );
            }
        }
    }

    /// Named systems come first whichever way the list runs. They are a handful out of
    /// thousands, and a page of bare identifiers is a page nobody asked for.
    #[test]
    fn unnamed_systems_sort_last_in_both_directions() {
        let stars = catalog();
        let counts = HashMap::new();
        for descending in [false, true] {
            let page = page(&stars, &counts, &Query { descending, limit: 4, ..query() });
            let named = page.systems.iter().filter(|r| r.name.is_some()).count();
            assert_eq!(named, 3, "descending={descending}: {:?}", page.systems);
            assert!(
                page.systems[..3].iter().all(|r| r.name.is_some()),
                "descending={descending}: the unnamed ones came first",
            );
        }
        // And the direction does reverse the named ones among themselves.
        let up = page(&stars, &counts, &Query { limit: 3, ..query() });
        let down = page(&stars, &counts, &Query { descending: true, limit: 3, ..query() });
        assert_eq!(up.systems[0].name.as_deref(), Some("Alpha Centauri"));
        assert_eq!(down.systems[0].name.as_deref(), Some("Sol"));
    }

    /// Searching finds a name however it is typed, and finds an identifier pasted in whole.
    #[test]
    fn a_search_matches_a_name_or_an_identifier() {
        let stars = catalog();
        let counts = HashMap::new();
        let found = |q: &str| {
            page(
                &stars,
                &counts,
                &Query {
                    q: q.into(),
                    ..query()
                },
            )
            .total
        };

        assert_eq!(found("sol"), 1);
        assert_eq!(found("SOL"), 1);
        assert_eq!(found("  centauri  "), 1);
        assert_eq!(found("barnard"), 1);
        assert_eq!(found(""), 10);
        assert_eq!(found("no such system"), 0);
        // An administrator with an identifier in hand should be able to paste it.
        assert_eq!(found(&stars[7].id.get().to_string()), 1);
    }

    /// Ordering by craft counts actually orders by them, which a sort on a derived column is
    /// easy to get wrong.
    #[test]
    fn ordering_by_ships_puts_the_busiest_first() {
        let stars = catalog();
        let mut counts = HashMap::new();
        counts.insert(stars[5].id.get(), 7);
        counts.insert(stars[2].id.get(), 3);

        let page = page(&stars, &counts, &Query {
            sort: Sort::Ships,
            descending: true,
            limit: 3,
            ..query()
        });
        assert_eq!(page.systems[0].id, stars[5].id.get());
        assert_eq!(page.systems[0].ships, 7);
        assert_eq!(page.systems[1].id, stars[2].id.get());
        assert_eq!(page.systems[1].ships, 3);
        assert_eq!(page.systems[2].ships, 0);
        // Nothing builds objects yet, and the column says zero rather than being absent.
        assert!(page.systems.iter().all(|r| r.objects == 0));
    }

    /// A page past the end is empty and still reports the total, so the console can say which
    /// kind of empty it is.
    #[test]
    fn a_page_past_the_end_is_empty_and_still_counts() {
        let stars = catalog();
        let page = page(&stars, &HashMap::new(), &Query { offset: 500, ..query() });
        assert!(page.systems.is_empty());
        assert_eq!(page.total, 10);
    }

    #[test]
    fn an_unrecognized_ordering_is_the_default() {
        assert_eq!(Sort::from_slug("nonsense"), Sort::Name);
        assert_eq!(Sort::from_slug(""), Sort::Name);
        assert_eq!(Sort::from_slug("ships"), Sort::Ships);
        assert_eq!(Sort::from_slug("objects"), Sort::Objects);
    }
}
