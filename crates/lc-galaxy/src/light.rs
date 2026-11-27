//! What a population's stars shine with on average, so pictures of the galaxy integrate light
//! without drawing a star. See `34-galaxy-authoring.md`, "Signed distances and ray marching".
//!
//! Light is in the B, V and R bands, in solar luminosities, which [`em_spectra::presets::natural`]
//! shows as blue, green and red: the client's starfield draws a star through the same mapping.

use std::f64::consts::PI;
use std::sync::OnceLock;

use em_spectra::{Band, blackbody};
use lc_world::rng;
use lc_world::sky::galaxy::FEH_STEP;

use crate::attributes::{mass, tracks};
use crate::compiled::{Age, Population};
use crate::kinds::MAX_AGE_GYR;

/// Stars drawn per population for its mean. The mean light of a young population is dominated
/// by its rare massive stars, so this is large.
const DRAWS: u64 = 20_000;

/// R, V and B light, L☉, in that order so it reads as display red, green, blue.
pub type Rgb = [f64; 3];

/// Mean light per star of `population`. Ignores the `[Fe/H]` gradient and offsets, which move a
/// star's luminosity by far less than its mass and age do.
pub fn mean_light(population: &Population, salt: u64) -> Rgb {
    let mut sum = [0.0; 3];
    for i in 0..DRAWS {
        let h = |n: u64| rng::uniform(rng::hash(&[salt, i, n]));
        let initial = mass(&population.imf, h(1));
        let age = match population.age {
            Age::Uniform { min_gyr, max_gyr } => min_gyr + h(2) * (max_gyr - min_gyr),
            Age::Burst { gyr, spread_gyr } => {
                // Box-Muller, as the bake draws a burst's ages.
                let g = libm::sqrt(-2.0 * libm::log(h(3).max(f64::MIN_POSITIVE))) * libm::cos(std::f64::consts::TAU * h(4));
                (gyr + spread_gyr * g).clamp(0.0, MAX_AGE_GYR)
            }
        };
        let feh = population.feh.zero_age + population.feh.per_gyr * age;
        let step = (feh / FEH_STEP).round().clamp(-128.0, 127.0) as i8;
        let e = tracks::evolve_step(initial, age, step);
        let share = band_shares(e.teff_k);
        for (s, f) in sum.iter_mut().zip(share) {
            *s += e.luminosity_solar * f;
        }
    }
    sum.map(|s| s / DRAWS as f64)
}

/// Log2 kelvin that [`band_shares`] tabulates; outside it clamps.
const LOG_T: (f64, f64) = (9.0, 17.0);
const SAMPLES: usize = 512;

/// The share of a blackbody's output in R, V and B: band radiance over `σT⁴ / π`.
pub fn band_shares(teff_k: f64) -> Rgb {
    static TABLE: OnceLock<Vec<Rgb>> = OnceLock::new();
    let (lo, hi) = LOG_T;
    let table = TABLE.get_or_init(|| {
        (0..SAMPLES)
            .map(|i| {
                let t = libm::exp2(lo + (hi - lo) * i as f64 / (SAMPLES - 1) as f64);
                let total = blackbody::radiant_exitance(t) / PI;
                [Band::R, Band::V, Band::B].map(|b| blackbody::band_radiance(b, t) / total)
            })
            .collect()
    });
    if teff_k <= 0.0 {
        return [0.0; 3];
    }
    let at = ((libm::log2(teff_k) - lo) / (hi - lo) * (SAMPLES - 1) as f64).clamp(0.0, (SAMPLES - 1) as f64);
    let (i, f) = (at.floor() as usize, at.fract());
    let (a, b) = (table[i], table[(i + 1).min(SAMPLES - 1)]);
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiled::{Dispersion, Feh, Imf};
    use crate::feature::FeatureId;

    fn population(age: Age) -> Population {
        Population {
            frame: FeatureId(0),
            imf: Imf::Kroupa,
            age,
            feh: Feh { zero_age: 0.0, per_gyr: 0.0, per_kly: 0.0, scatter: 0.0 },
            dispersion: Dispersion { zero_age_km_s: 1.0, exponent: 0.0 },
        }
    }

    #[test]
    fn a_young_population_is_brighter_and_bluer_than_an_old_one() {
        let young = mean_light(&population(Age::Burst { gyr: 0.05, spread_gyr: 0.01 }), 1);
        let old = mean_light(&population(Age::Burst { gyr: 10.0, spread_gyr: 0.5 }), 1);
        let total = |c: Rgb| c.iter().sum::<f64>();
        assert!(total(young) > 5.0 * total(old), "young {young:?}, old {old:?}");
        assert!(young[2] / young[0] > old[2] / old[0], "young {young:?} is not bluer than old {old:?}");
    }

    #[test]
    fn the_sun_puts_a_few_tenths_of_its_light_in_each_band() {
        let s = band_shares(5772.0);
        assert!(s.iter().all(|&f| (0.05..0.4).contains(&f)), "{s:?}");
    }
}
