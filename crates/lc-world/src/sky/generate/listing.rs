//! A generated system as a flat list of what orbits the star, for tools that show one.

use super::architecture::Class;
use super::disc::EARTH_MASS;
use super::tuning::Tuning;
use super::{AU, GeneratedSystem};
use crate::population::{self, Population};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Planet(Class),
    Population(population::Kind),
}

impl Kind {
    pub const BELT: Kind = Kind::Population(population::Kind::Belt);
    pub const OORT_CLOUD: Kind = Kind::Population(population::Kind::OortCloud);

    pub fn label(self) -> &'static str {
        match self {
            Kind::Planet(Class::Rocky) => "rocky",
            Kind::Planet(Class::Icy) => "icy",
            Kind::Planet(Class::IceGiant) => "ice giant",
            Kind::Planet(Class::GasGiant) => "gas giant",
            Kind::Population(k) => k.label(),
        }
    }
}

/// Planet-only values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct World {
    pub water_fraction: f64,
    pub habitable: bool,
    pub life: f64,
    pub moons: usize,
    /// The world an ocean-first system was built around.
    pub ocean_first: bool,
    pub migrated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    /// Torus radii, AU. A planet's are both its semi-major axis.
    pub inner_au: f64,
    pub outer_au: f64,
    /// Earth masses; `None` where the model has no mass.
    pub mass_earths: Option<f64>,
    pub world: Option<World>,
}

impl Entry {
    pub fn mean_au(&self) -> f64 {
        0.5 * (self.inner_au + self.outer_au)
    }
}

/// The disc's landmarks, AU.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zones {
    pub inner_au: f64,
    pub snow_au: f64,
    pub outer_au: f64,
    pub habitable_au: (f64, f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub zones: Zones,
    /// Inside out by mean radius.
    pub entries: Vec<Entry>,
}

/// `tuning` must be the one `system` was generated under: a population's mass is read back from
/// its element count through the tuning's area per kilogram.
pub fn listing(system: &GeneratedSystem, tuning: &Tuning) -> Listing {
    let disc = &system.architecture.disc;
    let zones = Zones {
        inner_au: disc.inner_m / AU,
        snow_au: disc.snow_m / AU,
        outer_au: disc.outer_m / AU,
        habitable_au: (disc.habitable_m.0 / AU, disc.habitable_m.1 / AU),
    };
    // `planets` is built from `architecture.planets()` in order, one per rung.
    let mut entries: Vec<Entry> = system
        .planets
        .iter()
        .zip(system.architecture.planets())
        .map(|(p, rung)| Entry {
            name: p.name.clone(),
            kind: Kind::Planet(p.class),
            inner_au: p.semi_major_m / AU,
            outer_au: p.semi_major_m / AU,
            mass_earths: Some(p.mass_kg / EARTH_MASS),
            world: Some(World {
                water_fraction: p.water_fraction,
                habitable: p.habitable,
                life: p.life,
                moons: p.moons.len(),
                ocean_first: rung.ocean,
                migrated: p.migrated,
            }),
        })
        .collect();
    entries.extend(system.populations.iter().filter_map(|p| population(p, tuning)));
    entries.sort_by(|a, b| a.mean_au().total_cmp(&b.mean_au()));
    let mut belts = 0;
    for e in entries.iter_mut().filter(|e| e.kind == Kind::BELT) {
        belts += 1;
        e.name = format!("belt {belts}");
    }
    Listing { zones, entries }
}

fn population(p: &Population, tuning: &Tuning) -> Option<Entry> {
    let extent = p.extent()?;
    let t = &tuning.belts;
    // Rings and swarms are not made from a mass, so none can be read back.
    let area_per_kg = match p.kind {
        population::Kind::Belt => Some(t.belt_area_per_kg),
        population::Kind::OuterDisc => Some(t.kuiper_area_per_kg),
        population::Kind::OortCloud => Some(t.oort_area_per_kg),
        population::Kind::Ring | population::Kind::Swarm => None,
    };
    let kind = Kind::Population(p.kind);
    Some(Entry {
        name: kind.label().into(),
        kind,
        inner_au: extent.inner_m / AU,
        outer_au: extent.outer_m / AU,
        mass_earths: area_per_kg.map(|a| p.count * p.cross_section / a / EARTH_MASS),
        world: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::generate::{architecture::tests_support::sun_like, system_with};

    #[test]
    fn every_planet_and_population_is_listed_inside_out() {
        let tuning = Tuning::default();
        let mut belts = 0;
        for k in 0..40 {
            let system = system_with(&sun_like(k), &tuning);
            let l = listing(&system, &tuning);
            let planets = l.entries.iter().filter(|e| e.world.is_some()).count();
            assert_eq!(planets, system.planets.len());
            assert_eq!(l.entries.len() - planets, system.populations.iter().filter(|p| p.extent().is_some()).count());
            assert!(l.entries.windows(2).all(|w| w[0].mean_au() <= w[1].mean_au()));
            for e in l.entries.iter().filter(|e| e.kind == Kind::BELT) {
                belts += 1;
                assert!(e.mass_earths.is_some_and(|m| m > 0.0) && e.inner_au < e.outer_au, "{e:?}");
            }
        }
        assert!(belts > 0, "no belts in forty Sun-like systems");
    }
}
