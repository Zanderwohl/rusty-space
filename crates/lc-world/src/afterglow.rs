//! The light a collapse lets go of: the spike at once, then the afterglow. See
//! `lightcone/docs/30-the-field.md` §Collapse.
//!
//! 30 says only that the afterglow's temperature falls from the field's limit. The law here is the
//! simplest that spends exactly its share of `E` in `collapse_afterglow_s`: a blackbody of fixed
//! area whose luminosity falls linearly to nothing, so `T = T_limit (1 − τ/D)^¼` and
//! `A = 2 E_a / (σ T_limit⁴ D)`.
//!
//! It is light and nothing else. Held in a neighbor's field it peaks at about 6% of the spike's
//! dose there, weeks after the spike, which moves the lethal radius out by 3%
//! ([`tests::the_afterglow_barely_moves_the_lethal_radius`]).

use em_spectra::{Band, PerBand, blackbody};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::field::{Field, Mode};
use crate::fitting::{Balance, STARTING_ENVELOPE_M2};
use crate::glow::{self, Glow};

/// Midpoints an exposure's afterglow is averaged over.
const SUBEXPOSURES: usize = 8;

/// A field's temperature at `Q_max`, which does not depend on its size: the afterglow's first color.
pub fn limit_k(balance: &Balance) -> f64 {
    let field = Field::of(STARTING_ENVELOPE_M2, balance);
    field.temperature_k(field.heat_max_j())
}

/// `since_s` into an afterglow of `duration_s` from `limit_k`; `None` before it and once it is over.
/// For anything that draws one.
pub fn temperature_k(limit_k: f64, duration_s: f64, since_s: f64) -> Option<f64> {
    (0.0..duration_s).contains(&since_s).then(|| limit_k * (1.0 - since_s / duration_s).sqrt().sqrt())
}

/// One collapse's light, from where the ship was.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Afterglow {
    /// Light-microseconds.
    pub from: [f64; 3],
    /// The collapse, coordinate seconds.
    pub at_s: f64,
    pub spike_j: f64,
    pub spike_k: f64,
    /// What leaves after the spike.
    pub energy_j: f64,
    pub duration_s: f64,
    pub limit_k: f64,
}

impl Afterglow {
    pub fn of(from: DVec3, at_s: f64, released_j: f64, balance: &Balance) -> Self {
        let spike_j = balance.collapse_spike_fraction * released_j;
        Self {
            from: from.to_array(),
            at_s,
            spike_j,
            spike_k: balance.collapse_spike_k,
            energy_j: released_j - spike_j,
            duration_s: balance.collapse_afterglow_s,
            limit_k: limit_k(balance),
        }
    }

    pub fn position(&self) -> DVec3 {
        DVec3::from_array(self.from)
    }

    pub fn area_m2(&self) -> f64 {
        2.0 * self.energy_j / (blackbody::radiant_exitance(self.limit_k) * self.duration_s)
    }

    /// Of the sphere whose area is [`Afterglow::area_m2`]: an equivalent, not the debris's size.
    pub fn radius_m(&self) -> f64 {
        (self.area_m2() / (4.0 * std::f64::consts::PI)).sqrt()
    }

    /// When its last light leaves, coordinate seconds.
    pub fn ends_s(&self) -> f64 {
        self.at_s + self.duration_s
    }

    /// As its light left at `t_s`.
    pub fn glow_at(&self, t_s: f64) -> Option<Glow> {
        let temperature_k = temperature_k(self.limit_k, self.duration_s, t_s - self.at_s)?;
        Some(Glow { temperature_k, shade: Mode::Black, envelope_m2: self.area_m2() })
    }

    /// W, as its light left at `t_s`.
    pub fn power_w(&self, t_s: f64) -> f64 {
        self.glow_at(t_s).map_or(0.0, |glow| glow::thermal_w(&glow))
    }

    /// Mean flux in each band, W/m², `distance_m` off, over an exposure to light that left from
    /// `from_s` to `to_s`. The spike is far shorter than any exposure, so it is its fluence spread
    /// over the exposure it falls in.
    pub fn mean_flux(&self, from_s: f64, to_s: f64, distance_m: f64) -> PerBand<f64> {
        let mut flux = PerBand::splat(0.0);
        let exposure_s = to_s - from_s;
        if exposure_s <= 0.0 {
            return flux;
        }
        if self.at_s > from_s && self.at_s <= to_s {
            let sphere_m2 = 4.0 * std::f64::consts::PI * (distance_m * distance_m).max(f64::MIN_POSITIVE);
            let exitance = blackbody::radiant_exitance(self.spike_k);
            for band in Band::ALL {
                let share = std::f64::consts::PI * blackbody::band_radiance(band, self.spike_k) / exitance;
                flux[band] += self.spike_j * share / sphere_m2 / exposure_s;
            }
        }
        let (lo, hi) = (from_s.max(self.at_s), to_s.min(self.ends_s()));
        if hi > lo {
            let step = (hi - lo) / SUBEXPOSURES as f64;
            for k in 0..SUBEXPOSURES {
                let Some(glow) = self.glow_at(lo + (k as f64 + 0.5) * step) else { continue };
                let each = glow::thermal(&glow, distance_m);
                for band in Band::ALL {
                    flux[band] += each[band] * step / exposure_s;
                }
            }
        }
        flux
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Balance = Balance::DEFAULT;
    const E_J: f64 = 1.0e21;

    fn afterglow() -> Afterglow {
        Afterglow::of(DVec3::ZERO, 100.0, E_J, &B)
    }

    #[test]
    fn it_starts_at_the_fields_limit_and_cools() {
        let a = afterglow();
        let limit = limit_k(&B);
        assert!((limit - 4600.0).abs() < 100.0, "30's failing field: {limit}");
        let field = Field::of(3.0 * STARTING_ENVELOPE_M2, &B);
        assert!((field.temperature_k(field.heat_max_j()) / limit - 1.0).abs() < 1.0e-12, "not every size's limit");
        assert_eq!(a.glow_at(a.at_s).unwrap().temperature_k, limit);
        assert_eq!(a.glow_at(a.at_s - 1.0), None);
        assert_eq!(a.glow_at(a.ends_s()), None);
        let mut last = f64::INFINITY;
        for k in 0..100 {
            let t = a.at_s + a.duration_s * k as f64 / 100.0;
            let now = a.glow_at(t).unwrap().temperature_k;
            assert!(now < last || k == 0, "at {k}%: {now} after {last}");
            last = now;
        }
        assert_eq!(a.duration_s, 30.0 * 86_400.0);
    }

    /// Everything the spike did not take, and no more, in `collapse_afterglow_s`.
    #[test]
    fn the_afterglow_spends_the_rest_of_e() {
        let a = afterglow();
        assert!((a.spike_j / E_J - B.collapse_spike_fraction).abs() < 1.0e-15);
        let n = 100_000;
        let dt = a.duration_s / n as f64;
        let spent_j: f64 = (0..n).map(|k| a.power_w(a.at_s + (k as f64 + 0.5) * dt) * dt).sum();
        let want_j = (1.0 - B.collapse_spike_fraction) * E_J;
        assert!((spent_j / want_j - 1.0).abs() < 1.0e-6, "{spent_j:e} of {want_j:e}");
    }

    /// The spike is a blackbody at `collapse_spike_k`: X-rays, with a Rayleigh–Jeans tail in the
    /// bands rising as `ν²`.
    #[test]
    fn the_spikes_color_is_collapse_spike_k() {
        let a = afterglow();
        assert_eq!(a.spike_k, B.collapse_spike_k);
        let flux = a.mean_flux(a.at_s - 1.0, a.at_s, 1.0e9);
        let fluence_j_m2 = a.spike_j / (4.0 * std::f64::consts::PI * 1.0e18);
        for band in Band::ALL {
            let want = fluence_j_m2 * std::f64::consts::PI * blackbody::band_radiance(band, B.collapse_spike_k)
                / blackbody::radiant_exitance(B.collapse_spike_k);
            assert!((flux[band] / want - 1.0).abs() < 1.0e-9, "{band:?}");
        }
        let at_the_limit = a.mean_flux(a.at_s + 1.0, a.at_s + 2.0, 1.0e9);
        assert!(flux[Band::V] / flux[Band::K] > 10.0 * at_the_limit[Band::V] / at_the_limit[Band::K], "not bluer than the afterglow");
    }

    #[test]
    fn an_exposure_sees_only_the_light_inside_it() {
        let a = afterglow();
        let d = 1.0e12;
        assert_eq!(a.mean_flux(0.0, a.at_s - 1.0, d)[Band::V], 0.0);
        assert_eq!(a.mean_flux(a.ends_s(), a.ends_s() + 1.0e4, d)[Band::V], 0.0);
        let whole = a.mean_flux(a.at_s + 10.0, a.at_s + 1.0e4 + 10.0, d)[Band::V];
        let straddling = a.mean_flux(a.at_s + 10.0 - 5.0e3, a.at_s + 5.0e3 + 10.0, d)[Band::V];
        assert!((straddling / whole - 0.5).abs() < 0.01, "{straddling} {whole}");
    }

    /// Why the afterglow is not an emission: a neighbor holds at most this much of it, against the
    /// spike's dose at the same place, relaxing as its own field does.
    #[test]
    fn the_afterglow_barely_moves_the_lethal_radius() {
        let a = afterglow();
        let tau_s = B.field_tau_s;
        let dt = 600.0;
        let (mut held_j, mut peak_j, mut t) = (0.0_f64, 0.0_f64, a.at_s);
        while t < a.ends_s() + 3.0 * tau_s {
            held_j += (a.power_w(t + 0.5 * dt) - held_j / tau_s) * dt;
            peak_j = peak_j.max(held_j);
            t += dt;
        }
        let of_spike = peak_j / a.spike_j;
        assert!(of_spike < 0.07, "{of_spike}");
        let farther = (1.0 + of_spike).sqrt() - 1.0;
        assert!(farther < 0.03, "{farther}");
    }
}
