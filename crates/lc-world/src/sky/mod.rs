//! Where stars come from.
//!
//! The shipped game is set in a fictional galaxy; real catalog data is how the physics gets
//! validated, not what ships. So star data arrives through a provider, and **a catalog's own
//! numbering never becomes a [`StarId`]** — it survives only as provenance. Both rules are
//! free now and are data migrations later.
//!
//! Its **names** are provenance too, for the same reason and one more: nothing in this game has
//! a name of its own. A name is something an observer gave a star and may have passed on, and
//! it lives in [`crate::knowledge`] with a witness on it. That is why [`CatalogStar`] has no
//! `name` field to reach for — see `lightcone/docs/22-provenance.md`.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::rng;
use crate::star::Star;

pub mod chunk;
pub mod generate;
pub mod metallicity;
pub mod record;

#[cfg(feature = "hyg")]
pub mod hyg;

/// A synthetic, stable star identifier.
///
/// Hashed from the provider's name and its own key, so it is order-independent: importing the
/// same catalog twice, or in a different order, gives the same ids.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StarId(u64);

impl StarId {
    pub fn synthesize(provider: &str, key: u64) -> Self {
        let tag = provider.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
        });
        Self(rng::hash(&[tag, key]))
    }

    #[inline]
    pub fn get(self) -> u64 {
        self.0
    }

    /// The id a wire message carries. The wire holds only what `get` gave it, so this is the
    /// inverse of that and never a way to invent an id.
    #[inline]
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

/// Where a star's data came from. Never an identity, and never what anybody calls it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub source: String,
    /// The source's own row number or designation, for tracing back to it.
    pub key: u64,
    /// What the catalog calls it.
    ///
    /// **Provenance, not a name.** Nothing in the game has a name of its own: a name is
    /// something an observer gave a star and may have told somebody else, and it lives in
    /// `knowledge` with a witness on it. This is here so a charting office has something to
    /// hand a ship, and so generation can call a planet after its primary, and for no other
    /// reason. Never show it to a player. See `lightcone/docs/22-provenance.md`.
    pub name: Option<String>,
}

/// Which member of a multiple system a record describes.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Component {
    /// 1 for a single star or the primary of a multiple.
    pub index: u8,
    /// Shared across the members of one multiple; `None` for a single star.
    pub group: Option<u64>,
}

impl Component {
    pub fn is_primary(&self) -> bool {
        self.index <= 1
    }
}

/// One star as a provider hands it over.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CatalogStar {
    pub id: StarId,
    pub provenance: Provenance,
    /// Ecliptic position in light-years, the frame the simulation uses.
    pub position_ly: DVec3,
    /// Ecliptic velocity in m/s.
    pub velocity: DVec3,
    pub star: Star,
    pub luminosity_solar: f64,
    pub mass_solar: f64,
    /// `[Fe/H]`, synthesized from kinematics; see [`metallicity`].
    pub metallicity: f64,
    pub component: Component,
}

impl CatalogStar {
    /// Seed for everything generated about this star. Derived from the id, so generation is
    /// reproducible and independent of load order.
    pub fn seed(&self) -> u64 {
        rng::hash(&[self.id.get(), 0x5147_4e59])
    }

    /// The normal of the plane this system's planets and belts orbit in.
    ///
    /// `+Z` for Sol, whose bodies are fitted against JPL in the ecliptic of J2000, and the
    /// generated pole for everything else. The one place that difference is decided: reading
    /// `generate::pole_for` directly gives Sol a random plane its planets do not lie in.
    pub fn system_pole(&self) -> DVec3 {
        match self.provenance.name.as_deref() {
            Some(crate::system::SOL) => DVec3::Z,
            _ => generate::pole_for(self.seed()),
        }
    }

    /// Which way the star itself spins: [`CatalogStar::system_pole`] tilted a few degrees.
    pub fn spin_axis(&self) -> DVec3 {
        generate::spin_axis_for(self.system_pole(), self.seed())
    }
}

/// A source of stars.
///
/// World generation never parses a catalog directly. `HygProvider` is one implementation
/// and an authored galaxy will be another; the trait is what keeps the second from being a
/// rewrite.
pub trait StarProvider {
    fn name(&self) -> &str;
    fn stars(&self) -> &[CatalogStar];

    fn get(&self, id: StarId) -> Option<&CatalogStar> {
        self.stars().iter().find(|s| s.id == id)
    }

    fn len(&self) -> usize {
        self.stars().len()
    }

    fn is_empty(&self) -> bool {
        self.stars().is_empty()
    }
}

/// A handful of hand-written stars.
///
/// Exists so the trait has a second implementation: an interface proven by one caller is not
/// proven. Also the seed of an authored galaxy.
pub struct AuthoredStars {
    name: String,
    stars: Vec<CatalogStar>,
}

impl AuthoredStars {
    pub fn new(name: impl Into<String>, stars: Vec<CatalogStar>) -> Self {
        Self { name: name.into(), stars }
    }

    /// Three stars, enough to exercise anything that consumes a provider.
    pub fn sample() -> Self {
        let make = |key: u64, ly: f64, teff: f64, lum: f64| {
            let l_w = lum * em_spectra::stellar::SOLAR_LUMINOSITY;
            let mass = em_spectra::stellar::main_sequence_mass_solar(lum);
            CatalogStar {
                id: StarId::synthesize("authored", key),
                provenance: Provenance {
                    source: "authored".into(),
                    key,
                name: Some(format!("Authored {key}")),
                },
                position_ly: DVec3::new(ly, 0.0, 0.0),
                velocity: DVec3::ZERO,
                star: Star {
                    radius_m: em_spectra::stellar::radius_from_luminosity(l_w, teff),
                    teff_k: teff,
                    mu: em_spectra::stellar::mu_from_mass_solar(mass),
                    limb_darkening: (0.4, 0.26),
                },
                luminosity_solar: lum,
                mass_solar: mass,
                metallicity: 0.0,
                component: Component { index: 1, group: None },
            }
        };
        Self::new("authored", vec![make(1, 4.2, 3000.0, 0.0017), make(2, 11.0, 5772.0, 1.0), make(3, 25.0, 9500.0, 25.0)])
    }
}

impl StarProvider for AuthoredStars {
    fn name(&self) -> &str {
        &self.name
    }
    fn stars(&self) -> &[CatalogStar] {
        &self.stars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_and_provider_scoped() {
        assert_eq!(StarId::synthesize("hyg", 71456), StarId::synthesize("hyg", 71456));
        assert_ne!(StarId::synthesize("hyg", 71456), StarId::synthesize("hyg", 71457));
        // The same catalog key under a different provider is a different star.
        assert_ne!(StarId::synthesize("hyg", 1), StarId::synthesize("authored", 1));
        // And an id is not the key wearing a hat.
        assert_ne!(StarId::synthesize("hyg", 71456).get(), 71456);
    }

    #[test]
    fn a_second_provider_satisfies_the_trait() {
        let p = AuthoredStars::sample();
        assert_eq!(p.len(), 3);
        assert_eq!(p.name(), "authored");
        let first = p.stars()[0].id;
        assert_eq!(p.get(first).unwrap().id, first);
        assert!(p.get(StarId::synthesize("nope", 0)).is_none());
    }

    #[test]
    fn generation_seeds_follow_the_id_not_the_order() {
        let p = AuthoredStars::sample();
        let seeds: Vec<u64> = p.stars().iter().map(|s| s.seed()).collect();
        let mut shuffled = AuthoredStars::sample();
        shuffled.stars.reverse();
        let mut back: Vec<u64> = shuffled.stars().iter().map(|s| s.seed()).collect();
        back.reverse();
        assert_eq!(seeds, back);
    }
}
