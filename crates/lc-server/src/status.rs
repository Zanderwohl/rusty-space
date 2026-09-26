//! What an administrator is told about one account's ship.
//!
//! **A contract with a service that cannot link this crate.** The console is another cargo
//! workspace and holds its own copy of these types, agreeing by shape as the ticket claims do
//! — so **a field renamed here silently stops arriving there**. RON rather than JSON because
//! it is a format and not a type, and because it does not round f64s.
//!
//! Read from the **checkpoint**, so it is seconds stale and needs nothing from the running
//! world: no channel into the tick, no lock on the fleet, no page load that costs a frame.

use lc_world::sky::CatalogStar;
use serde::{Deserialize, Serialize};

use crate::persist::Saved;

/// From the same place `crate::world::system_at` takes it, or the console and the game
/// disagree about where somebody is.
pub use lc_world::system::LOCAL_SHELL_LY;

/// Where a craft is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Whereabouts {
    In {
        star: u64,
        name: String,
        /// From the star, astronomical units.
        au: f64,
    },
    /// The nearest is named because "nowhere" is true and useless.
    Interstellar { star: u64, near: String, ly: f64 },
    /// Outside the catalog; reachable only by a craft placed by hand.
    Nowhere,
}

/// What is fitted, and what it is doing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    /// Parts by kind, the Mind apart: entries in the form, as `MAX_PARTS` counts them, so a
    /// mirrored part is one. By name, not a field per kind: a new kind would otherwise be
    /// a field the console's mirror lacks, and RON would refuse the whole payload. Named for
    /// 19's modules, which the console's mirror still calls them.
    pub modules: Vec<(String, u32)>,
    /// `MAX_PARTS`.
    pub hull_slots: u32,
    /// Parts, the Mind included.
    pub used_slots: u32,
    /// Joules as of `saved_t`, not extrapolated forward from it.
    pub stored_j: f64,
    /// Watts of starlight.
    pub solar_w: f64,
    /// Joules promised to a plan in flight.
    pub committed_j: f64,
    pub refitting: bool,
}

/// One account's ship, as of the last checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub ship_id: i64,
    pub name: Option<String>,
    /// Hull length, meters.
    pub length_m: f64,
    /// Coordinate microseconds; the freshness of everything else here.
    pub saved_t: i64,
    pub whereabouts: Whereabouts,
    /// Absent for a craft saved before fittings existed, or one with none.
    pub fit: Option<Fit>,
}

/// Light-years in an astronomical unit.
pub(crate) const AU_LY: f64 = 1.495_978_707e11 / 9.460_730_472_580_8e15;

impl Status {
    /// A linear scan over `stars`, as `world::system_at` does: once per page view, and
    /// cheaper than the round trip that fetched the row.
    pub fn of(ship_id: i64, saved_t: i64, saved: &Saved, stars: &[CatalogStar]) -> Status {
        Status {
            ship_id,
            name: saved.name.clone(),
            length_m: saved.length_m,
            saved_t,
            whereabouts: whereabouts(saved.motion.at_ly.into(), stars),
            fit: saved.fitting.as_ref().map(fit_of),
        }
    }
}

/// `total_cmp`, not `partial_cmp().unwrap()`: a NaN is a coordinate nothing should have
/// produced, and an arbitrary ordering beats panicking on a page load.
pub(crate) fn nearest(at: glam::DVec3, stars: &[CatalogStar]) -> Option<(&CatalogStar, f64)> {
    stars
        .iter()
        .min_by(|a, b| {
            let (x, y) = (a.position_ly.distance(at), b.position_ly.distance(at));
            x.total_cmp(&y)
        })
        .map(|star| (star, star.position_ly.distance(at)))
}

/// Inside the shell, not merely nearest — otherwise craft count toward systems they are
/// light-years from.
pub(crate) fn nearest_within_shell(
    at: glam::DVec3,
    stars: &[CatalogStar],
) -> Option<&CatalogStar> {
    nearest(at, stars).filter(|(_, ly)| *ly < LOCAL_SHELL_LY).map(|(star, _)| star)
}

fn whereabouts(at: glam::DVec3, stars: &[CatalogStar]) -> Whereabouts {
    let Some((star, ly)) = nearest(at, stars) else {
        return Whereabouts::Nowhere;
    };
    // The catalog's own name: this is the operator's status page, not a player's screen.
    let name = star
        .provenance
        .name
        .clone()
        .unwrap_or_else(|| format!("star {}", star.id.get()));
    if ly < LOCAL_SHELL_LY {
        Whereabouts::In {
            star: star.id.get(),
            name,
            au: ly / AU_LY,
        }
    } else {
        Whereabouts::Interstellar {
            star: star.id.get(),
            near: name,
            ly,
        }
    }
}

fn fit_of(fitting: &lc_proto::Fitting) -> Fit {
    use lc_proto::form::Kind;
    let parts = &fitting.form.parts;
    let count = |of: fn(Kind) -> bool| parts.iter().filter(|p| of(p.kind)).count() as u32;
    let modules = vec![
        ("Engines".to_owned(), count(|k| k == Kind::Engine)),
        ("Storage".to_owned(), count(|k| k == Kind::Storage)),
        ("Drones".to_owned(), count(|k| k == Kind::Drone)),
        ("Living".to_owned(), count(|k| k == Kind::Living)),
        ("Data".to_owned(), count(|k| k == Kind::Data)),
        ("Bays".to_owned(), count(|k| k == Kind::Bay)),
        ("Spars".to_owned(), count(|k| matches!(k, Kind::Spar(_)))),
    ];
    Fit {
        modules,
        hull_slots: lc_world::form::MAX_PARTS as u32,
        used_slots: parts.len() as u32,
        stored_j: fitting.stored_j,
        solar_w: fitting.solar_w,
        committed_j: fitting.committed_j,
        refitting: fitting.refit.is_some(),
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
            component: StarComponent {
                index: 1,
                group: None,
            },
        }
    }

    /// A craft inside the shell is in the system; one outside it is between the stars, and
    /// the nearest is named — "nowhere" is true and useless to whoever is reading the page.
    #[test]
    fn a_craft_is_in_a_system_or_named_near_one() {
        let stars = vec![
            star(1, Some("Sol"), glam::DVec3::ZERO),
            star(2, Some("Alpha Centauri"), glam::DVec3::new(4.37, 0.0, 0.0)),
        ];

        // Just inside Sol's shell.
        let close = whereabouts(glam::DVec3::new(LOCAL_SHELL_LY * 0.5, 0.0, 0.0), &stars);
        match close {
            Whereabouts::In { name, au, star } => {
                assert_eq!(name, "Sol");
                assert_eq!(star, stars[0].id.get());
                assert!(au > 0.0, "a craft at the shell edge read as on the star");
            }
            other => panic!("{other:?}"),
        }

        // Halfway to Alpha Centauri: outside both shells, nearer Sol.
        match whereabouts(glam::DVec3::new(2.0, 0.0, 0.0), &stars) {
            Whereabouts::Interstellar { near, ly, .. } => {
                assert_eq!(near, "Sol");
                assert!((ly - 2.0).abs() < 1e-9, "{ly}");
            }
            other => panic!("{other:?}"),
        }

        // And nearer the other one, past the midpoint — but still outside its shell, which
        // is 1.6 ly and so covers most of the gap between two stars this close. At 3.0 ly a
        // craft is 1.37 from Alpha Centauri and therefore *in* that system, which is what the
        // first version of this test asserted was interstellar.
        match whereabouts(glam::DVec3::new(2.5, 0.0, 0.0), &stars) {
            Whereabouts::Interstellar { near, .. } => assert_eq!(near, "Alpha Centauri"),
            other => panic!("{other:?}"),
        }
        // The shell really does reach that far, so the boundary is where it is said to be.
        assert!(matches!(
            whereabouts(glam::DVec3::new(3.0, 0.0, 0.0), &stars),
            Whereabouts::In { .. }
        ));
    }

    #[test]
    fn an_empty_catalog_is_nowhere_rather_than_a_panic() {
        assert_eq!(whereabouts(glam::DVec3::ZERO, &[]), Whereabouts::Nowhere);
    }

    /// An unnamed star still names itself, because a card reading "Interstellar space near"
    /// and then nothing is worse than one reading "near star 41234".
    #[test]
    fn an_unnamed_star_is_still_named() {
        let stars = vec![star(41234, None, glam::DVec3::ZERO)];
        let expected = format!("star {}", stars[0].id.get());
        match whereabouts(glam::DVec3::new(50.0, 0.0, 0.0), &stars) {
            Whereabouts::Interstellar { near, .. } => assert_eq!(near, expected),
            other => panic!("{other:?}"),
        }
    }

    /// The payload round-trips through RON, which is the only thing the console can do with
    /// it. A type that serializes and does not deserialize would fail on the far side, where
    /// there is no test to catch it.
    #[test]
    fn the_payload_round_trips_through_ron() {
        let status = Status {
            ship_id: 7,
            name: Some("Rocinante".into()),
            length_m: 46.0,
            saved_t: 1_234_567,
            whereabouts: Whereabouts::In {
                star: 1,
                name: "Sol".into(),
                au: 5.0,
            },
            fit: Some(Fit {
                modules: vec![("Drive".into(), 1), ("Reactor".into(), 2)],
                hull_slots: 8,
                used_slots: 3,
                stored_j: 1.5e12,
                solar_w: 4.2e3,
                committed_j: 0.0,
                refitting: false,
            }),
        };
        let text = ron::to_string(&status).expect("it serializes");
        let back: Status = ron::from_str(&text).expect("it parses");
        assert_eq!(back, status);
        // Exactly, including the f64s — which is half the reason this is RON.
        assert_eq!(back.fit.unwrap().stored_j, 1.5e12);
    }
}
