//! Ocean-first systems: the authored skew that builds an ocean world before anything else, where
//! a star could have one. See `lightcone/docs/35-system-contents.md`.
//!
//! A star that loses the draw, or cannot have one, is untouched: the draw uses a salt nothing else
//! does, so the rest of the generator sees exactly the hashes it always did.

use super::disc::{self, Disc};
use super::planet::{WATER, exosphere_k, retention_margin};
use super::tuning::Tuning;
use crate::rng;
use crate::sky::CatalogStar;

/// Earth's water as a share of its mass: the line between open seas and damp ground.
pub const SEAS_WATER_FRACTION: f64 = 2.3e-4;

const SALT_DRAW: u64 = 0x0cea_4f17;
const SALT_SITE: u64 = 0x0cea_5173;
pub(crate) const SALT_LADDER: u64 = 0x0cea_1add;

/// Water retention margin an ocean world's floor mass must clear. Above the retention rule's
/// 1.0 by enough to survive the radius spread (a twentieth of a dex) the planet stage draws.
const MARGIN_WANTED: f64 = 1.3;

/// Habitable-zone orbits tried, log-spaced; a site is drawn among those that work.
const SITES: usize = 32;

/// One orbit where an ocean world could form, and the masses it could have there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
    pub semi_major_m: f64,
    /// Smallest mass that holds its water at this orbit's temperature, Earth masses.
    pub floor_earths: f64,
    /// Largest it can assemble from the narrowest feeding zone the ladder allows, and still be
    /// under the core mass that binds a hydrogen envelope, Earth masses.
    pub ceiling_earths: f64,
}

/// Whether a star is on the main sequence, by its radius against a main-sequence one of its mass:
/// a giant has swollen past its planets' habitable zone and a white dwarf has none.
fn on_main_sequence(star: &CatalogStar) -> bool {
    let r = star.star.radius_m / em_spectra::stellar::SOLAR_RADIUS;
    let m = star.mass_solar;
    let main_sequence = if m < 1.0 { m.powf(0.8) } else { m.powf(0.57) };
    star.luminosity_solar > 0.0 && r > 0.5 * main_sequence && r < 2.5 * main_sequence
}

/// Smallest mass whose water retention margin clears [`MARGIN_WANTED`] at `exosphere_k`, by
/// bisection; `None` if even `upper` does not.
fn floor_earths(exosphere_k: f64, upper: f64, tuning: &Tuning) -> Option<f64> {
    let margin = |m: f64| retention_margin(WATER, m, disc::radius_earths(m, false), exosphere_k, tuning);
    if margin(upper) < MARGIN_WANTED {
        return None;
    }
    let (mut lo, mut hi) = (1e-3, upper);
    for _ in 0..40 {
        let mid = (lo * hi).sqrt();
        if margin(mid) >= MARGIN_WANTED { hi = mid } else { lo = mid }
    }
    Some(hi)
}

/// Every habitable-zone orbit, of [`SITES`] tried, where the disc can build an ocean world.
pub fn sites(disc: &Disc, star: &CatalogStar, tuning: &Tuning) -> Vec<Site> {
    if !on_main_sequence(star) {
        return Vec::new();
    }
    let t = &tuning.ladder;
    let (lo, hi) = disc.habitable_m;
    let narrowest = t.spacing.0.sqrt();
    let growth_m = t.growth_over_snow * disc.snow_m;
    (0..SITES)
        .filter_map(|i| {
            let a = lo * (hi / lo).powf((i as f64 + 0.5) / SITES as f64);
            if disc.icy(a) || a < disc.inner_m * narrowest {
                return None;
            }
            let assembled = (growth_m / a).powi(3).min(1.0) * t.efficiency.1;
            let ceiling = (disc.solids_between(a / narrowest, a * narrowest) * assembled).min(0.95 * t.ice_giant_core_earths);
            let exosphere = exosphere_k(disc::temperature_at(&star.star, a), tuning);
            let floor = floor_earths(exosphere, ceiling, tuning)?;
            Some(Site { semi_major_m: a, floor_earths: floor, ceiling_earths: ceiling })
        })
        .collect()
}

/// Whether `star` could have an ocean world at all, whatever its profile says.
pub fn eligible(star: &CatalogStar, tuning: &Tuning) -> bool {
    !sites(&Disc::of(star, tuning), star, tuning).is_empty()
}

/// The site this star builds its ocean world at, if it is ocean-first under `tuning`.
pub fn first(disc: &Disc, star: &CatalogStar, tuning: &Tuning, seed: u64) -> Option<Site> {
    let chance = tuning.world.ocean_first;
    if chance <= 0.0 || rng::uniform(rng::hash(&[seed, SALT_DRAW])) >= chance {
        return None;
    }
    let sites = sites(disc, star, tuning);
    if sites.is_empty() {
        return None;
    }
    let pick = (rng::uniform(rng::hash(&[seed, SALT_SITE])) * sites.len() as f64) as usize;
    Some(sites[pick.min(sites.len() - 1)])
}

/// The ocean world's mass: log-uniform between its site's floor and ceiling.
pub fn mass_earths(site: &Site, seed: u64) -> f64 {
    rng::uniform_in(rng::hash(&[seed, SALT_SITE, 1]), site.floor_earths.ln(), site.ceiling_earths.ln()).exp()
}
