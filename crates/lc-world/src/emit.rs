//! What an emission delivers: an aperture's rating, the flux along its cone, the share that lands
//! on a receiver, and how sure the aim behind it can be. See
//! `lightcone/docs/31-directed-energy.md`.
//!
//! An emission is a top-hat cone from a point: uniform intensity inside the half-angle and none
//! outside, the same cone [`Beam::covers`](crate::signal::Beam::covers) tests. So flux is inverse
//! square and **infinite at zero distance**, and [`flux_w_m2`] says so rather than inventing a
//! finite number. What lands on a receiver is bounded all the same: [`received_fraction`] is one
//! wherever the spot is no larger than the receiver's shadow, so no receiver ever takes more than
//! was sent. Distances are meters and powers watts; the caller supplies both.

use crate::fitting::Balance;
use crate::flight::C_M_S;
use crate::signal::cone_solid_angle_sr;

/// What `engine_m3` of engine can send, watts: its exhaust, its deliberate emission and its
/// conversion into storage are all bounded by this.
pub fn rating_w(balance: &Balance, engine_m3: f64) -> f64 {
    balance.engine_density_w * engine_m3.max(0.0)
}

/// What a photon thruster puts out to push `mass_kg` at `accel_m_s2`, watts: thrust times `c`.
pub fn thrust_power_w(mass_kg: f64, accel_m_s2: f64) -> f64 {
    mass_kg * accel_m_s2 * C_M_S
}

/// The temperature of an aperture's open face, kelvin: a blackbody radiating all of `power_w`
/// through `face_m2`. For a photon drive `power_w` is `F c`.
pub fn aperture_temperature_k(power_w: f64, face_m2: f64) -> f64 {
    if power_w <= 0.0 || face_m2 <= 0.0 {
        return 0.0;
    }
    (power_w / (em_spectra::blackbody::SIGMA * face_m2)).powf(0.25)
}

/// W/m² anywhere inside a cone of `half_angle_rad` carrying `power_w`, at `distance_m`.
pub fn flux_w_m2(power_w: f64, half_angle_rad: f64, distance_m: f64) -> f64 {
    if power_w <= 0.0 {
        return 0.0;
    }
    power_w / (cone_solid_angle_sr(half_angle_rad) * distance_m * distance_m)
}

/// Where [`flux_w_m2`] falls to `flux_w_m2`, meters. Zero for no power, infinite for no flux.
pub fn distance_at_flux_m(power_w: f64, half_angle_rad: f64, flux_w_m2: f64) -> f64 {
    if power_w <= 0.0 {
        return 0.0;
    }
    (power_w / (cone_solid_angle_sr(half_angle_rad) * flux_w_m2)).sqrt()
}

/// The share of an emission a receiver in its cone intercepts: its shadow toward the emitter over
/// the spot, at most one.
///
/// The spot is the cap the cone cuts from a sphere of `distance_m`, not the disk `pi (theta d)^2`
/// that 31's formula writes, which is the same thing only for a narrow cone.
pub fn received_fraction(half_angle_rad: f64, shadow_m2: f64, distance_m: f64) -> f64 {
    if shadow_m2 <= 0.0 {
        return 0.0;
    }
    let spot_m2 = cone_solid_angle_sr(half_angle_rad) * distance_m * distance_m;
    if spot_m2 <= shadow_m2 { 1.0 } else { shadow_m2 / spot_m2 }
}

/// How far from its predicted place a target free to thrust at `accel_m_s2` can be, meters, when
/// the prediction is `blind_s` old at the beam's arrival.
///
/// Hyperbolic motion from rest, `(c²/a)(√(1 + x²) − 1)` with `x = a t / c`, so it never passes
/// `c t`: `½ a t²` does after 71 days at 5 g, and at four light-years says twenty times what light
/// could cover. Written as `a t² / (√(1 + x²) + 1)` because the first form cancels to nothing at
/// the light-seconds 31 tabulates, where `x²` is 10⁻¹³.
pub fn lead_uncertainty_m(accel_m_s2: f64, blind_s: f64) -> f64 {
    let a = accel_m_s2.abs();
    let x = a * blind_s / C_M_S;
    a * blind_s * blind_s / ((1.0 + x * x).sqrt() + 1.0)
}

/// How long a target is unwatched when the freshest sighting of it is aimed at: its light's
/// flight here plus the beam's flight back, seconds.
pub fn blind_s(distance_m: f64) -> f64 {
    2.0 * distance_m / C_M_S
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitting::Fitting;
    use crate::form::presets::SLOT_M3;
    use crate::form::Form;
    use crate::flight::{G0, JULIAN_YEAR_S};
    use crate::signal::Transmitter;
    use crate::solar::broadside_m2;

    const AU_M: f64 = 1.495_978_707e11;
    const LY_M: f64 = C_M_S * JULIAN_YEAR_S;

    #[test]
    fn the_starting_drive_section_is_rated_at_its_five_g() {
        let b = Balance::DEFAULT;
        let drive_m3 = 5.0 * SLOT_M3;
        assert_eq!(format!("{:.2e}", drive_m3), "1.96e6");
        let rated = rating_w(&b, drive_m3);
        assert_eq!(format!("{rated:.1e}"), "1.1e20");
        // Full is the anchored dry mass and 30 ME, so the form weighs this to the anchor's 1e-9.
        let full_kg = Fitting::full(Form::starting(), b, 0.0).mass_kg_at(&crate::motion::ShipState::at(glam::DVec3::ZERO), 0.0);
        let five_g = thrust_power_w(full_kg, 5.0 * G0);
        assert!((rated - five_g).abs() / rated < 1.0e-9, "{rated} against {five_g}");
    }

    /// 32 §The exhaust cone: the starting drive, all of it through a 100 m face, is white-hot.
    #[test]
    fn the_starting_drive_s_face_is_white_hot() {
        let b = Balance::DEFAULT;
        let rated = rating_w(&b, 5.0 * SLOT_M3);
        let face_m2 = std::f64::consts::PI * 50.0 * 50.0;
        assert_eq!(format!("{:.1e}", aperture_temperature_k(rated, face_m2)), "7.0e5");
        assert_eq!(aperture_temperature_k(0.0, face_m2), 0.0, "an unlit drive has no face");
    }

    /// 31 §What arrives, every cell: a 100 m aperture at its diffraction floor, the spot across,
    /// and the fraction a 500 m, 5 km and 50 km hull collects broadside.
    #[test]
    fn what_arrives_table() {
        let rows: [(f64, f64, &str, [&str; 3]); 5] = [
            (AU_M, 1.0e-6, "1.5e3", ["7e-2", "1e0", "1e0"]),
            (AU_M, 1.0e-9, "1.5e0", ["1e0", "1e0", "1e0"]),
            (100.0 * AU_M, 1.0e-6, "1.5e5", ["7e-6", "7e-4", "7e-2"]),
            (100.0 * AU_M, 1.0e-9, "1.5e2", ["1e0", "1e0", "1e0"]),
            (4.0 * LY_M, 1.0e-9, "3.8e5", ["1e-6", "1e-4", "1e-2"]),
        ];
        for (d, lambda, spot, fractions) in rows {
            let t = Transmitter::new(lambda, 100.0);
            assert_eq!(format!("{:.1e}", t.spot_at(d)), spot, "spot at {d:e} m, {lambda:e} m");
            for (length, want) in [500.0, 5_000.0, 50_000.0].into_iter().zip(fractions) {
                let got = received_fraction(t.half_angle_rad(), broadside_m2(length), d);
                assert_eq!(format!("{got:.0e}"), want, "{length} m hull at {d:e} m, {lambda:e} m");
            }
        }
        // 31 §Dumping heat: 10⁻¹¹ radians wide.
        assert_eq!(format!("{:.0e}", 2.0 * Transmitter::new(1.0e-9, 100.0).half_angle_rad()), "1e-11");
    }

    /// 31 §Spread: a 5 g target, aimed at from its freshest sighting.
    #[test]
    fn lead_uncertainty_table() {
        let a = 5.0 * G0;
        let rows = [(1.0, "1e2"), (10.0, "1e4"), (60.0, "3.5e5")];
        for (light_s, want) in rows {
            let got = lead_uncertainty_m(a, blind_s(light_s * C_M_S));
            let digits = if want.contains('.') { 1 } else { 0 };
            assert_eq!(format!("{got:.digits$e}"), want, "{light_s} light-seconds");
        }
    }

    /// Past a few light-days, a target can be anywhere light could have reached, less the time it
    /// spends getting up to speed: `c t − c²/a` in the limit.
    #[test]
    fn lead_uncertainty_is_bounded_by_light() {
        let a = 5.0 * G0;
        let t = blind_s(4.0 * LY_M);
        let got = lead_uncertainty_m(a, t);
        assert!(got < C_M_S * t, "{got:e} past light's {:e}", C_M_S * t);
        let limit = C_M_S * t - C_M_S * C_M_S / a;
        assert!((got - limit).abs() / limit < 1.0e-3, "{got:e} against {limit:e}");
        // And still ½ a t² where the table lives, to a part in 10⁶.
        let t = blind_s(C_M_S);
        assert!((lead_uncertainty_m(a, t) / (0.5 * a * t * t) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn flux_is_inverse_square_and_power_over_the_spot() {
        let (p, half) = (1.0e20, 5.0_f64.to_radians());
        let near = flux_w_m2(p, half, 1_000.0);
        assert!((near / flux_w_m2(p, half, 2_000.0) - 4.0).abs() < 1.0e-12);
        // Everything sent crosses the spot: flux times the spot's area is the power.
        assert!((near * cone_solid_angle_sr(half) * 1.0e6 - p).abs() / p < 1.0e-12);
        let d = distance_at_flux_m(p, half, near);
        assert!((d - 1_000.0).abs() < 1.0e-9, "{d}");
    }

    /// The decision at zero distance: the flux is honestly infinite, and what a receiver takes is
    /// still bounded by what was sent.
    #[test]
    fn at_zero_distance_flux_is_infinite_and_the_fraction_is_one() {
        assert_eq!(flux_w_m2(1.0e20, 0.1, 0.0), f64::INFINITY);
        assert_eq!(flux_w_m2(0.0, 0.1, 0.0), 0.0);
        assert_eq!(received_fraction(0.1, 1.0, 0.0), 1.0);
        assert_eq!(received_fraction(0.1, 0.0, 0.0), 0.0);
        assert_eq!(distance_at_flux_m(0.0, 0.1, 1.0), 0.0);
    }

    /// A receiver whose shadow covers the spot takes everything; past that it takes its share.
    #[test]
    fn the_fraction_saturates_where_the_spot_shrinks_to_the_shadow() {
        let half = 0.01;
        let shadow = 100.0;
        let edge = (shadow / cone_solid_angle_sr(half)).sqrt();
        assert_eq!(received_fraction(half, shadow, 0.999 * edge), 1.0);
        let past = received_fraction(half, shadow, 2.0 * edge);
        assert!((past - 0.25).abs() < 1.0e-12, "{past}");
    }
}
