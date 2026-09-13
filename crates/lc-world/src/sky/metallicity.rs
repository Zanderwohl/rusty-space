//! `[Fe/H]` synthesised from kinematics.
//!
//! The catalogue has no metallicity column, so it is inferred rather than read. Kinematics
//! and abundance are genuinely correlated: the galaxy's oldest stars formed from unenriched
//! gas and were scattered onto fast, inclined orbits, so speed relative to the local standard
//! of rest is a usable proxy. Fast is old is metal-poor.

use crate::rng;

/// Metallicity of a star at rest in the local standard of rest.
pub const THIN_DISC_FEH: f64 = 0.0;
/// Floor, roughly the halo.
pub const FLOOR_FEH: f64 = -2.0;
/// Speed at which the relation has fallen most of the way, m/s.
const SCALE_SPEED: f64 = 120_000.0;
/// Intrinsic spread at a given speed, dex.
const SCATTER: f64 = 0.2;

/// `[Fe/H]` from peculiar speed in m/s, with seeded scatter.
pub fn from_speed(speed_m_s: f64, seed: u64) -> f64 {
    let mean = THIN_DISC_FEH + FLOOR_FEH * (1.0 - (-speed_m_s.max(0.0) / SCALE_SPEED).exp());
    (mean + SCATTER * rng::gaussian(rng::hash(&[seed, 0xfe_04]))).clamp(FLOOR_FEH - 0.5, 0.6)
}

/// Mass of solid material available to a system, relative to a solar-metallicity one.
///
/// Scales as `10^[Fe/H]`: a tenth of the metals is a tenth of the rock, which is a smaller
/// belt, a thinner Kuiper analogue and fewer volatiles. This is what makes metal-rich systems
/// worth travelling to, from real catalogue data rather than a sprinkled bonus.
pub fn solid_mass_factor(feh: f64) -> f64 {
    10f64.powf(feh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_stars_are_metal_rich_and_fast_stars_are_not() {
        let slow = from_speed(10_000.0, 1);
        let fast = from_speed(250_000.0, 1);
        assert!(slow > -0.5, "a 10 km/s star should be near solar: {slow}");
        assert!(fast < -1.2, "a 250 km/s star should be halo-like: {fast}");
        assert!(slow > fast);
    }

    #[test]
    fn the_relation_is_monotonic_on_average() {
        let mean = |v: f64| (0..400).map(|k| from_speed(v, k)).sum::<f64>() / 400.0;
        let mut prev = f64::INFINITY;
        for v in [0.0, 20e3, 50e3, 100e3, 200e3, 400e3] {
            let m = mean(v);
            assert!(m < prev, "not monotonic at {v} m/s");
            prev = m;
        }
    }

    #[test]
    fn scatter_is_seeded_and_bounded() {
        assert_eq!(from_speed(50e3, 7), from_speed(50e3, 7));
        assert_ne!(from_speed(50e3, 7), from_speed(50e3, 8));
        for k in 0..5000u64 {
            let f = from_speed(rng::uniform(rng::hash(&[k])) * 400e3, k);
            assert!((-2.5..=0.6).contains(&f), "{f} left the plausible range");
        }
    }

    #[test]
    fn solid_mass_follows_the_metals() {
        assert!((solid_mass_factor(0.0) - 1.0).abs() < 1e-12);
        assert!((solid_mass_factor(-1.0) - 0.1).abs() < 1e-12);
        assert!(solid_mass_factor(0.3) > 1.9);
    }
}
