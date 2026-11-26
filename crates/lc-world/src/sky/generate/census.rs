//! A star's generated system, counted, for statistics over many stars. See
//! `lightcone/docs/35-system-contents.md`, "In Galaxy Forge".

use super::ocean::{self, SEAS_WATER_FRACTION};
use super::tuning::Tuning;
use super::{GeneratedSystem, system_with};
use crate::population::Kind;
use crate::sky::CatalogStar;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub planets: u32,
    pub giants: u32,
    pub habitable: u32,
    /// Habitable, with at least Earth's share of water.
    pub ocean_worlds: u32,
    pub belt: bool,
    pub oort_cloud: bool,
    /// Could have an ocean world at all, whatever its profile; see [`ocean::eligible`].
    pub eligible: bool,
    pub ocean_first: bool,
    pub moons: u32,
    /// Caught rather than grown in a disc or knocked off by an impact; see [`super::moon`].
    pub captured_moons: u32,
    pub planets_with_moons: u32,
    pub submoons: u32,
}

impl Census {
    pub fn of(star: &CatalogStar, tuning: &Tuning) -> Self {
        Self::of_system(&system_with(star, tuning), ocean::eligible(star, tuning))
    }

    fn of_system(system: &GeneratedSystem, eligible: bool) -> Self {
        let count = |f: &dyn Fn(&super::Planet) -> bool| system.planets.iter().filter(|p| f(p)).count() as u32;
        let has = |kind: Kind| system.populations.iter().any(|p| p.kind == kind);
        Self {
            planets: system.planets.len() as u32,
            giants: count(&|p| p.class.is_giant()),
            habitable: count(&|p| p.habitable),
            ocean_worlds: count(&|p| p.habitable && p.water_fraction >= SEAS_WATER_FRACTION),
            belt: has(Kind::Belt),
            oort_cloud: has(Kind::OortCloud),
            eligible,
            ocean_first: system.architecture.rungs.iter().any(|r| r.ocean),
            moons: system.planets.iter().map(|p| p.moons.len() as u32).sum(),
            captured_moons: system.moons().filter(|m| !m.regular).count() as u32,
            planets_with_moons: count(&|p| !p.moons.is_empty()),
            submoons: system.moons().map(|m| m.moons.len() as u32).sum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::generate::architecture::tests_support::sun_like;

    #[test]
    fn only_an_ocean_first_tuning_builds_ocean_first_and_every_eligible_star_then_has_one() {
        let mut first = Tuning::default();
        first.world.ocean_first = 1.0;
        let mut eligible = 0;
        for k in 0..40 {
            let star = sun_like(k);
            assert!(!Census::of(&star, &Tuning::default()).ocean_first);
            let c = Census::of(&star, &first);
            assert_eq!(c.ocean_first, c.eligible);
            assert!(c.captured_moons <= c.moons && c.planets_with_moons <= c.planets && c.planets_with_moons <= c.moons, "{c:?}");
            if c.eligible {
                eligible += 1;
                assert!(c.ocean_worlds >= 1 && c.habitable >= c.ocean_worlds && c.planets >= c.habitable, "{c:?}");
            }
        }
        assert!(eligible > 30, "only {eligible} of forty Sun-like stars eligible");
    }
}
