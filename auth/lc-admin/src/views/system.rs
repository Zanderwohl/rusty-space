//! One system: the star as the catalog has it, the craft inside its shell, and what players
//! have named it.
//!
//! All of it as of the shard's last checkpoint, some seconds stale.

use maud::{Markup, html};
use uuid::Uuid;

use crate::shard::{Craft, Detail, Facts, Missing, Name};

const JULIAN_YEAR_S: f64 = 31_557_600.0;

pub fn page(found: &Result<Detail, Missing>) -> Markup {
    html! {
        section class="stack" {
            p class="crumbs" { a href=(crate::routes::SYSTEMS) { "← All systems" } }
            @match found {
                Err(missing) => p class="nothing" { (missing.said()) },
                Ok(detail) => {
                    header class="page-head" {
                        h1 {
                            @match &detail.star.catalog_name {
                                Some(name) => (name),
                                None => "Unnamed system",
                            }
                        }
                        (facts(detail.id, &detail.star))
                    }
                    (ships(&detail.ships))
                    (names(&detail.names))
                }
            }
        }
    }
}

pub fn title(found: &Result<Detail, Missing>) -> String {
    match found {
        Ok(Detail {
            star: Facts {
                catalog_name: Some(name),
                ..
            },
            ..
        }) => name.clone(),
        Ok(detail) => format!("System {}", detail.id),
        Err(_) => "System".to_owned(),
    }
}

fn facts(id: u64, star: &Facts) -> Markup {
    const SUN_RADIUS_M: f64 = 6.957e8;
    html! {
        dl class="facts" {
            dt { "System" }
            dd { code { (id) } }
            dt { "Catalog" }
            dd { (star.source) " " (star.key) }
            dt { "From Sol" }
            dd { (format!("{:.2} ly", star.from_sol_ly)) }
            dt { "Temperature" }
            dd { (format!("{:.0} K", star.teff_k)) }
            dt { "Radius" }
            dd { (format!("{:.2} R☉", star.radius_m / SUN_RADIUS_M)) }
            dt { "Luminosity" }
            dd { (format!("{:.3} L☉", star.luminosity_solar)) }
            dt { "Mass" }
            dd { (format!("{:.2} M☉", star.mass_solar)) }
            dt { "[Fe/H]" }
            dd { (format!("{:+.2}", star.metallicity)) }
            @if let Some(group) = star.group {
                dt { "Multiple" }
                dd { "component " (star.component) " of group " code { (group) } }
            }
        }
    }
}

fn ships(ships: &[Craft]) -> Markup {
    html! {
        section class="panel" {
            h2 { "Ships" }
            @if ships.is_empty() {
                p class="nothing" { "No craft is inside this system." }
            } @else {
                table class="records" {
                    thead {
                        tr {
                            th scope="col" { "Ship" }
                            th scope="col" { "Hull" }
                            th scope="col" { "Mind" }
                            th scope="col" { "Where" }
                        }
                    }
                    tbody {
                        @for craft in ships {
                            tr {
                                th scope="row" {
                                    @match &craft.name {
                                        Some(name) => (name),
                                        None => span class="nothing" { "Unnamed" },
                                    }
                                }
                                td { (craft.ship_id) }
                                td { (mind(craft.account.as_deref())) }
                                td {
                                    (craft.place)
                                    span class="from-star" { (format!("{:.2} AU from the star", craft.au)) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The shard's account ids are the broker's, as text. One that does not parse is shown rather
/// than linked, since a link built from it could only 404.
fn mind(account: Option<&str>) -> Markup {
    html! {
        @match account {
            None => span class="nothing" { "None" },
            Some(account) => @match Uuid::parse_str(account) {
                Ok(id) => a href=(crate::routes::user_url(id)) { "Account" },
                Err(_) => code { (account) },
            },
        }
    }
}

fn names(names: &[Name]) -> Markup {
    html! {
        section class="panel" {
            h2 { "Names" }
            @if names.is_empty() {
                p class="nothing" { "No craft has named this star." }
            } @else {
                table class="records" {
                    thead {
                        tr {
                            th scope="col" { "Name" }
                            th scope="col" { "Chosen by" }
                            th scope="col" { "When" }
                            th scope="col" { "Held by" }
                        }
                    }
                    tbody {
                        @for name in names {
                            tr {
                                th scope="row" { (name.name) }
                                td { "hull " (name.by) }
                                td { (format!("T + {:.2} years", name.stated_s / JULIAN_YEAR_S)) }
                                td { (held_by(name)) }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Holding a name is not calling the star by it; see `lc_server::systems::Name`.
fn held_by(name: &Name) -> String {
    let others: Vec<String> = name
        .held_by
        .iter()
        .filter(|&&h| h != name.by)
        .map(|h| h.to_string())
        .collect();
    let chooser = name.held_by.contains(&name.by);
    match (chooser, others.is_empty()) {
        (true, true) => "its chooser".to_owned(),
        (true, false) => format!("its chooser and hulls {}", others.join(", ")),
        (false, true) => "nobody".to_owned(),
        (false, false) => format!("hulls {}", others.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(by: i64, held_by: &[i64]) -> Name {
        Name {
            name: "Hearth".into(),
            by,
            stated_s: 0.0,
            held_by: held_by.to_vec(),
        }
    }

    #[test]
    fn a_name_says_who_holds_it_besides_its_chooser() {
        assert_eq!(held_by(&name(7, &[7])), "its chooser");
        assert_eq!(
            held_by(&name(7, &[7, 9, 12])),
            "its chooser and hulls 9, 12"
        );
        // The chooser renamed it since, and only relayed copies remain.
        assert_eq!(held_by(&name(7, &[9])), "hulls 9");
    }

    #[test]
    fn a_mind_links_to_its_account_only_when_the_id_is_one() {
        let id = "11111111-1111-4111-8111-111111111111";
        let linked = mind(Some(id)).into_string();
        assert!(
            linked.contains(&format!("href=\"/users/{id}\"")),
            "{linked}"
        );
        let unlinked = mind(Some("acct-7")).into_string();
        assert!(!unlinked.contains("href"), "{unlinked}");
        assert!(mind(None).into_string().contains("None"));
    }

    #[test]
    fn an_unnamed_system_is_titled_by_its_id() {
        let detail = Detail {
            id: 42,
            star: Facts {
                catalog_name: None,
                source: "hyg".into(),
                key: 1,
                from_sol_ly: 4.2,
                teff_k: 3000.0,
                radius_m: 1e8,
                luminosity_solar: 0.01,
                mass_solar: 0.1,
                metallicity: 0.0,
                component: 1,
                group: None,
            },
            ships: Vec::new(),
            names: Vec::new(),
        };
        assert_eq!(title(&Ok(detail.clone())), "System 42");
        let page = page(&Ok(detail)).into_string();
        assert!(page.contains("Unnamed system"));
        assert!(page.contains("No craft has named this star."));
        assert!(page.contains("No craft is inside this system."));
    }
}
