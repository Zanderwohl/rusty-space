//! What an observer sees of a craft: the starlight its field reflects, and the field's own heat. See
//! `lightcone/docs/30-the-field.md` §What an observer sees.
//!
//! Both are physical, with no gain. `Q/τ` moves energy at game scale; σT⁴A is what an instrument
//! counts. Fluxes are W/m² at the observer, per band.

use em_spectra::{Band, PerBand, blackbody};

use crate::field::Mode;
use crate::fitting::Balance;
use crate::instrument::Instrument;

/// A field as its light left it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub temperature_k: f64,
    pub shade: Mode,
    pub envelope_m2: f64,
}

impl Glow {
    /// What a craft with no fitting is taken to wear: the starting field at rest.
    pub fn unfitted(balance: &Balance) -> Glow {
        Glow { temperature_k: balance.field_idle_k, shade: Mode::Clear, envelope_m2: crate::fitting::STARTING_ENVELOPE_M2 }
    }
}

impl From<Glow> for lc_proto::Glow {
    fn from(g: Glow) -> Self {
        lc_proto::Glow { temperature_k: g.temperature_k, shade: g.shade.into(), envelope_m2: g.envelope_m2 }
    }
}

/// The star lighting a craft, as the craft sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Starlit {
    pub teff_k: f64,
    pub radius_m: f64,
    pub distance_m: f64,
    /// Between the directions to the star and to the observer: 1 sees the lit face whole.
    pub cos_phase: f64,
}

/// A craft's light at the observer, term by term.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub reflected: PerBand<f64>,
    pub thermal: PerBand<f64>,
}

impl Light {
    pub fn total(&self) -> PerBand<f64> {
        self.reflected.map(|band, r| r + self.thermal[band])
    }
}

/// Of the starlight arriving, what a field in `shade` sends back: `1 − α`.
pub fn reflectance(shade: Mode, balance: &Balance) -> f64 {
    1.0 - shade.absorptivity(balance.clear_absorptivity)
}

/// Of a lit shadow seen at `cos_phase`, the share that is lit.
pub fn lit_share(cos_phase: f64) -> f64 {
    0.5 * (1.0 + cos_phase.clamp(-1.0, 1.0))
}

/// W, over every wavelength: σT⁴A.
pub fn thermal_w(glow: &Glow) -> f64 {
    blackbody::radiant_exitance(glow.temperature_k) * glow.envelope_m2
}

/// What `glow` sends an observer `distance_m` away, whose line of sight meets a shadow of
/// `shadow_m2`.
///
/// The envelope is taken as convex, so it shows a quarter of its area on average and its heat is
/// isotropic. The shadow reflects as a Lambertian face.
pub fn light(glow: &Glow, balance: &Balance, shadow_m2: f64, star: Option<Starlit>, distance_m: f64) -> Light {
    let per_sr_to_flux = 1.0 / (distance_m * distance_m).max(f64::MIN_POSITIVE);
    let thermal = thermal(glow, distance_m);
    let reflected = match star {
        Some(star) if star.distance_m > 0.0 => {
            let dilution = (star.radius_m / star.distance_m).powi(2);
            let scale = reflectance(glow.shade, balance) * dilution * shadow_m2 * lit_share(star.cos_phase) * per_sr_to_flux;
            PerBand::new(std::array::from_fn(|i| blackbody::band_radiance(Band::ALL[i], star.teff_k) * scale))
        }
        _ => PerBand::splat(0.0),
    };
    Light { reflected, thermal }
}

/// The heat term of [`light`] alone: all a field in the dark, or an afterglow, sends.
pub fn thermal(glow: &Glow, distance_m: f64) -> PerBand<f64> {
    let per_sr_to_flux = 1.0 / (distance_m * distance_m).max(f64::MIN_POSITIVE);
    PerBand::new(std::array::from_fn(|i| {
        blackbody::band_radiance(Band::ALL[i], glow.temperature_k) * glow.envelope_m2 * 0.25 * per_sr_to_flux
    }))
}

/// Signal over noise for `flux_w_m2` in `band`, against the instrument's own glow: whether it sees
/// a craft there.
pub fn snr(instrument: &Instrument, band: Band, flux_w_m2: f64, exposure_s: f64) -> f64 {
    if !instrument.sees(band) {
        return 0.0;
    }
    let source = instrument.counts_from_flux(band, flux_w_m2, exposure_s);
    if source <= 0.0 {
        return 0.0;
    }
    source / (source + instrument.self_emission_counts(band, exposure_s)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Balance = Balance::DEFAULT;
    const AU: f64 = crate::navigation::AU;
    const SUN_K: f64 = 5772.0;

    fn sunlit() -> Option<Starlit> {
        Some(Starlit { teff_k: SUN_K, radius_m: em_spectra::stellar::SOLAR_RADIUS, distance_m: AU, cos_phase: 1.0 })
    }

    fn glow(temperature_k: f64, shade: Mode) -> Glow {
        Glow { temperature_k, shade, envelope_m2: crate::fitting::STARTING_ENVELOPE_M2 }
    }

    /// 30's anchor: an idle field, a thousand kilometers off, broadside in sunlight at 1 AU.
    fn seen(shade: Mode) -> Light {
        light(&glow(B.field_idle_k, shade), &B, 1.0e6, sunlit(), 1.0e6)
    }

    /// **The whole point.** Black reflects nothing, so in V there is nothing; its heat is the same as
    /// Clear's, and at ten microns it is as bright.
    #[test]
    fn a_black_ship_vanishes_in_v_and_not_at_ten_microns() {
        let (clear, black) = (seen(Mode::Clear), seen(Mode::Black));
        assert_eq!(black.reflected[Band::V], 0.0);
        assert!(black.total()[Band::V] < 1.0e-20 * clear.total()[Band::V], "{:e}", black.total()[Band::V]);
        assert!(black.thermal[Band::ThermalIr] > 0.0);
        assert_eq!(black.total()[Band::ThermalIr], black.thermal[Band::ThermalIr]);
        assert!(black.total()[Band::ThermalIr] > 0.9 * clear.total()[Band::ThermalIr]);

        let (hour, ship) = (3600.0, Instrument::SHIP);
        assert!(snr(&ship, Band::V, black.total()[Band::V], hour) < 1.0);
        assert!(snr(&ship, Band::V, clear.total()[Band::V], hour) > 5.0);
        assert!(snr(&ship, Band::ThermalIr, black.total()[Band::ThermalIr], hour) > 5.0);
    }

    /// Clear gives back `1 − 0.3` of what a white face would.
    #[test]
    fn clear_reflects_what_it_does_not_absorb() {
        let white = |star: Starlit| {
            let irradiance_w_m2 = blackbody::band_radiance(Band::V, star.teff_k) * std::f64::consts::PI * (star.radius_m / star.distance_m).powi(2);
            irradiance_w_m2 * 1.0e6 / std::f64::consts::PI / 1.0e12
        };
        let got = seen(Mode::Clear).reflected[Band::V] / white(sunlit().unwrap());
        assert!((got - 0.7).abs() < 1.0e-12, "{got}");
        assert_eq!(B.clear_absorptivity, 0.3);
    }

    #[test]
    fn a_face_turned_away_reflects_nothing() {
        let away = Starlit { cos_phase: -1.0, ..sunlit().unwrap() };
        assert_eq!(light(&glow(400.0, Mode::Clear), &B, 1.0e6, Some(away), 1.0e6).reflected[Band::V], 0.0);
    }

    /// σT⁴A, with no gain: the envelope's heat integrated over wavelength and direction, against
    /// the account's `Q/τ`, which is energy moving at game scale.
    #[test]
    fn the_thermal_flux_is_sigma_t4_a_with_no_gain() {
        let g = glow(1000.0, Mode::Black);
        let distance_m = 1.0e5;
        // Over every wavelength, by a quadrature of Planck's law rather than through the bands.
        let (lo, hi, n) = (1.0e-7_f64, 1.0e-3_f64, 200_000);
        let step = (hi / lo).ln() / f64::from(n);
        let radiance: f64 = (0..n)
            .map(|k| {
                let l = lo * ((f64::from(k) + 0.5) * step).exp();
                blackbody::spectral_radiance(l, g.temperature_k) * l * step
            })
            .sum();
        let over_sphere_w = radiance * g.envelope_m2 * 0.25 / (distance_m * distance_m) * 4.0 * std::f64::consts::PI * distance_m * distance_m;
        assert!((over_sphere_w / thermal_w(&g) - 1.0).abs() < 1.0e-4, "{over_sphere_w:e} against {:e}", thermal_w(&g));
        let expected = blackbody::SIGMA * 1.0e12 * crate::fitting::STARTING_ENVELOPE_M2;
        assert!((thermal_w(&g) / expected - 1.0).abs() < 1.0e-12);

        let each = light(&g, &B, 0.0, None, distance_m).thermal;
        for band in Band::ALL {
            let want = blackbody::band_radiance(band, g.temperature_k) * g.envelope_m2 / (4.0 * distance_m * distance_m);
            assert!((each[band] / want - 1.0).abs() < 1.0e-12, "{band:?}");
        }
        let field = crate::field::Field::of(g.envelope_m2, &B);
        let q_over_tau = field.idle_j_m2 * g.envelope_m2 / field.tau_s;
        let ratio = thermal_w(&glow(B.field_idle_k, Mode::Black)) / q_over_tau;
        assert!(ratio.log10().abs() > 3.0, "the account's rate is not the light: {ratio:e}");
    }

    /// 30's table: where the field's light is brightest for its width, as it heats.
    #[test]
    fn a_hot_field_moves_through_the_bands_as_30_says() {
        let brightest = |temperature_k: f64| {
            let each = light(&glow(temperature_k, Mode::Black), &B, 0.0, None, 1.0).thermal;
            let (lo, _) = Band::Radio.limits_m();
            Band::ALL
                .into_iter()
                .filter(|band| band.limits_m().1 < lo)
                .max_by(|a, b| {
                    let per_m = |band: Band| each[band] / (band.limits_m().1 - band.limits_m().0);
                    per_m(*a).total_cmp(&per_m(*b))
                })
                .unwrap()
        };
        assert_eq!(brightest(400.0), Band::ThermalIr, "idle: ten microns");
        assert_eq!(brightest(1000.0), Band::K, "full at 1 AU: K");
        assert_eq!(brightest(2400.0), Band::I, "diving: the near infrared");
        assert_eq!(brightest(4600.0), Band::R, "failing: the visible");
        assert!(light(&glow(4600.0, Mode::Black), &B, 0.0, None, 1.0).thermal[Band::V] > 1.0e9 * light(&glow(400.0, Mode::Black), &B, 0.0, None, 1.0).thermal[Band::V]);
    }
}
