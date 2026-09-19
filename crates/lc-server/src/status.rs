//! What an administrator is told about one account's ship.
//!
//! **This is a contract with a service that cannot link this crate.** The administration
//! console lives in `auth/`, a different cargo workspace, and must not depend on a game crate
//! — so it holds its own copy of the types below and the two agree by shape, exactly as the
//! ticket claims do. `lightcone/docs/16-identity.md` has the reasoning; what it means in
//! practice is that **a field renamed here is a field that silently stops arriving there**.
//! Renaming one is a change to both sides or it is a bug.
//!
//! Serialised as **RON**, which is the reason a console that cannot see `lc_proto::Motion` can
//! still read this: RON is a format, not a type, so the far side deserialises into its own
//! mirror structs. It is also what `serde_json` is not — exact about f64 — which matters less
//! here than it does in `persist`, this being a page somebody reads rather than a checkpoint
//! the world is rebuilt from, but there is no reason to use the format that rounds.
//!
//! It is read from the **checkpoint**, not from the tick loop. A shard saves every
//! `SAVE_EVERY_TICKS`, so this is a few seconds stale, and that is the whole reason this
//! module needs nothing from the running world: no channel into the tick, no lock on the
//! fleet, and no way for an administrator refreshing a page to slow the simulation down.

use lc_world::sky::CatalogueStar;
use serde::{Deserialize, Serialize};

use crate::persist::Saved;

/// How near a star a craft has to be before it is *in* that system rather than between them.
///
/// The same shell `crate::world::system_at` uses to decide the same question, taken from the
/// same place so the two cannot drift: a craft the tick loop considers inside a system and
/// this module considers interstellar would be a console disagreeing with the game about
/// where somebody is.
pub use lc_world::system::LOCAL_SHELL_LY;

/// Where a craft is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Whereabouts {
    /// Inside a system's local shell.
    In {
        star: u64,
        name: String,
        /// Distance from the star itself, astronomical units. A craft on a station and one
        /// at the shell's edge are both "in" the system and are not in the same place.
        au: f64,
    },
    /// Between the stars, with the nearest one named — which is what somebody reading this
    /// actually wants to know, "nowhere" being true and useless.
    Interstellar { star: u64, near: String, ly: f64 },
    /// Outside the catalogue entirely. Reachable only by a craft placed by hand.
    Nowhere,
}

/// What is fitted, and what it is doing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fit {
    /// Module counts, by name, in the order the loadout declares them. Names rather than a
    /// struct with a field per module: a module added on this side would otherwise be a field
    /// the console's mirror does not have, and RON would refuse the whole payload.
    pub modules: Vec<(String, u32)>,
    pub hull_slots: u32,
    pub used_slots: u32,
    /// Joules in store as of `saved_t`. Not extrapolated: this is a checkpoint, and a number
    /// grown forward from one would be a guess wearing a measurement's clothes.
    pub stored_j: f64,
    /// Watts of starlight it was collecting.
    pub solar_w: f64,
    /// Joules already promised to a plan in flight.
    pub committed_j: f64,
    /// Set while a refit is running.
    pub refitting: bool,
}

/// One account's ship, as of the last checkpoint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Status {
    pub ship_id: i64,
    pub name: Option<String>,
    /// Hull length, metres.
    pub length_m: f64,
    /// Coordinate microseconds the checkpoint was taken at. **The freshness of everything
    /// here**, and rendered as such rather than hidden: a page that shows a stale number
    /// without saying it is stale is a page that lies once a shard stops saving.
    pub saved_t: i64,
    pub whereabouts: Whereabouts,
    /// Absent for a craft saved before fittings existed, or one with no fitting at all.
    pub fit: Option<Fit>,
}

/// Light-years in an astronomical unit.
const AU_LY: f64 = 1.495_978_707e11 / 9.460_730_472_580_8e15;

impl Status {
    /// Read a checkpoint into the answer.
    ///
    /// `stars` is the catalogue the shard was started with. A linear scan over it, which is
    /// what `world::system_at` does too: this runs once per page view of one account, and a
    /// hundred thousand distance comparisons is nothing beside the database round trip that
    /// fetched the row.
    pub fn of(ship_id: i64, saved_t: i64, saved: &Saved, stars: &[CatalogueStar]) -> Status {
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

fn whereabouts(at: glam::DVec3, stars: &[CatalogueStar]) -> Whereabouts {
    let nearest = stars.iter().min_by(|a, b| {
        let (x, y) = (a.position_ly.distance(at), b.position_ly.distance(at));
        // `total_cmp` rather than `partial_cmp().unwrap()`: a NaN here is a craft at a
        // coordinate nothing should have produced, and panicking on a page load is a worse
        // answer than an arbitrary ordering.
        x.total_cmp(&y)
    });
    let Some(star) = nearest else {
        return Whereabouts::Nowhere;
    };
    let ly = star.position_ly.distance(at);
    let name = star
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
    let loadout = &fitting.loadout;
    let modules = vec![
        ("Engines".to_owned(), loadout.engines),
        ("Storage".to_owned(), loadout.storage),
        ("Drone bays".to_owned(), loadout.drones),
        ("Living".to_owned(), loadout.living),
    ];
    let used: u32 = modules.iter().map(|(_, n)| n).sum();
    Fit {
        modules,
        hull_slots: loadout.slots,
        used_slots: used,
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

    fn star(key: u64, name: Option<&str>, at: glam::DVec3) -> CatalogueStar {
        CatalogueStar {
            id: StarId::synthesise("test", key),
            provenance: Provenance {
                source: "test".into(),
                key,
            },
            name: name.map(str::to_owned),
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
    fn an_empty_catalogue_is_nowhere_rather_than_a_panic() {
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
    /// it. A type that serialises and does not deserialise would fail on the far side, where
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
        let text = ron::to_string(&status).expect("it serialises");
        let back: Status = ron::from_str(&text).expect("it parses");
        assert_eq!(back, status);
        // Exactly, including the f64s — which is half the reason this is RON.
        assert_eq!(back.fit.unwrap().stored_j, 1.5e12);
    }
}
