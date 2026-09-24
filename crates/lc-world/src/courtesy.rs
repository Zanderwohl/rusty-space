//! How much a lit drive may put on its neighbors. See `lightcone/docs/31-directed-energy.md`
//! §Maneuvering near others.
//!
//! Every limit here is a flux, so it holds for a receiver of any size: shadow and rated load both
//! go as size squared. The receiver is assumed Black unless an absorptivity is given, and the
//! emitter a cone as [`crate::emit`] models it.

use crate::emit::{distance_at_flux_m, thrust_power_w};
use crate::fitting::Balance;
use crate::flight::G0;
use crate::signal::cone_solid_angle_sr;
use crate::solar::SOLAR_CONSTANT_W_M2;

/// A full starting ship broadside to a Sun-like star is at its rated load this far out, AU. It is
/// what sets the field's time constant (`lightcone/docs/30-the-field.md`), and so the cooking flux.
pub const RATED_LOAD_AU: f64 = 0.05;

/// The flux a full, Black receiver of any size sits at its rated load in, W/m².
///
/// The gained starlight at [`RATED_LOAD_AU`]. The living drain also counts toward the anchor, but
/// it grows with volume rather than area, so it has no per-area share; on the starting ship it is
/// a part in 10⁴.
pub fn cooking_flux_w_m2(balance: &Balance) -> f64 {
    balance.solar_gain * SOLAR_CONSTANT_W_M2 / (RATED_LOAD_AU * RATED_LOAD_AU)
}

/// The most a courteous maneuver puts on anyone it can see, W/m².
pub fn courtesy_flux_w_m2(balance: &Balance) -> f64 {
    balance.courtesy_fraction * cooking_flux_w_m2(balance)
}

/// Inside this distance, a full receiver of absorptivity `absorptivity` in the cone is past its
/// rated load, meters.
pub fn cooking_distance_m(balance: &Balance, power_w: f64, half_angle_rad: f64, absorptivity: f64) -> f64 {
    distance_at_flux_m(power_w * absorptivity, half_angle_rad, cooking_flux_w_m2(balance))
}

/// Inside this distance, an emission of `power_w` at `half_angle_rad` is discourteous to anyone in
/// its cone, meters.
pub fn courtesy_radius_m(balance: &Balance, power_w: f64, half_angle_rad: f64) -> f64 {
    distance_at_flux_m(power_w, half_angle_rad, courtesy_flux_w_m2(balance))
}

pub fn drive_courtesy_radius_m(balance: &Balance, power_w: f64) -> f64 {
    courtesy_radius_m(balance, power_w, balance.drive_spread_rad)
}

/// What station-keeping thrusters put out at full throttle on a craft of `mass_kg`, watts.
pub fn thrusters_power_w(balance: &Balance, mass_kg: f64) -> f64 {
    thrust_power_w(mass_kg, balance.rcs_accel_g * G0)
}

pub fn thrusters_courtesy_radius_m(balance: &Balance, mass_kg: f64) -> f64 {
    courtesy_radius_m(balance, thrusters_power_w(balance, mass_kg), balance.rcs_spread_rad)
}

/// The most an emission at `half_angle_rad` may carry and stay courteous to someone `distance_m`
/// away, watts: how far thrusters are throttled when holding station close.
pub fn courteous_power_w(balance: &Balance, half_angle_rad: f64, distance_m: f64) -> f64 {
    courtesy_flux_w_m2(balance) * cone_solid_angle_sr(half_angle_rad) * distance_m * distance_m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::{flux_w_m2, rating_w, received_fraction};
    use crate::fitting::Loadout;
    use crate::solar::broadside_m2;

    const LENGTHS_M: [f64; 3] = [500.0, 5_000.0, 50_000.0];

    /// The starting ship scaled to `length_m`: its drive section's volume and its full mass both
    /// go as length cubed.
    fn scaled(b: &Balance, length_m: f64) -> (f64, f64) {
        let k = (length_m / 500.0).powi(3);
        let drive_m3 = Loadout::STARTING.engines as f64 * b.slot_volume_m3;
        (rating_w(b, drive_m3 * k), b.engine_thrust_n / G0 * k)
    }

    /// Two significant figures, as 31 writes them.
    fn sig2(x: f64) -> String {
        format!("{x:.1e}")
    }

    /// The flux 30's τ anchor fixes: 30's rated load over the starting ship's broadside.
    #[test]
    fn the_cooking_flux_is_the_rated_load_over_the_shadow() {
        let flux = cooking_flux_w_m2(&Balance::DEFAULT);
        assert_eq!(sig2(flux * broadside_m2(500.0)), "7.6e19");
    }

    /// 31 §Exhaust lands on whatever is behind.
    #[test]
    fn exhaust_cooking_distance_table() {
        let b = Balance::DEFAULT;
        let rows = [("1.1e20", "2.6e3"), ("1.1e23", "8.4e4"), ("1.1e26", "2.6e6")];
        for (length, (power, distance)) in LENGTHS_M.into_iter().zip(rows) {
            let (p, _) = scaled(&b, length);
            assert_eq!(sig2(p), power, "{length} m");
            let black = cooking_distance_m(&b, p, b.drive_spread_rad, 1.0);
            assert_eq!(sig2(black), distance, "{length} m");
            // "A Clear receiver's distance is a little over half of these."
            let clear = cooking_distance_m(&b, p, b.drive_spread_rad, b.clear_absorptivity) / black;
            assert!((0.5..0.6).contains(&clear), "{clear}");
        }
    }

    /// 31 §Station-keeping thrusters. At 60° the solid angle and `pi theta^2` part by 10%, and
    /// the doc's first figures, from a flat disk of radius `d tan theta`, were 40% short.
    #[test]
    fn thrusters_cooking_distance_table() {
        let b = Balance::DEFAULT;
        let rows = [("2.1e17", "1.0e1"), ("2.1e20", "3.3e2"), ("2.1e23", "1.0e4")];
        for (length, (power, distance)) in LENGTHS_M.into_iter().zip(rows) {
            let (_, kg) = scaled(&b, length);
            let p = thrusters_power_w(&b, kg);
            assert_eq!(sig2(p), power, "{length} m");
            assert_eq!(sig2(cooking_distance_m(&b, p, b.rcs_spread_rad, 1.0)), distance, "{length} m");
        }
    }

    /// 31 §Courtesy.
    #[test]
    fn drive_courtesy_radius_table() {
        let b = Balance::DEFAULT;
        for (length, radius) in LENGTHS_M.into_iter().zip(["2.6e4", "8.4e5", "2.6e7"]) {
            let (p, _) = scaled(&b, length);
            assert_eq!(sig2(drive_courtesy_radius_m(&b, p)), radius, "{length} m");
        }
    }

    /// The radius is where the flux is the limit, whoever sits there: a receiver of any size at it
    /// absorbs exactly the courtesy flux over its shadow.
    #[test]
    fn at_the_courtesy_radius_any_receiver_takes_the_limit() {
        let b = Balance::DEFAULT;
        let (p, _) = scaled(&b, 5_000.0);
        let r = drive_courtesy_radius_m(&b, p);
        let limit = courtesy_flux_w_m2(&b);
        assert!((flux_w_m2(p, b.drive_spread_rad, r) / limit - 1.0).abs() < 1.0e-12);
        for length in LENGTHS_M {
            let shadow = broadside_m2(length);
            let per_m2 = p * received_fraction(b.drive_spread_rad, shadow, r) / shadow;
            assert!((per_m2 / limit - 1.0).abs() < 1.0e-12, "{length} m: {per_m2}");
        }
        // 31 §Two ways to approach: past the braking cone's reach, eleven radii out, the flux is
        // under a hundredth of the limit.
        assert!(flux_w_m2(p, b.drive_spread_rad, 11.0 * r) < 0.01 * limit);
    }

    /// Throttled to the courteous power at a distance, the thrusters' courtesy radius is that
    /// distance.
    #[test]
    fn courteous_power_is_the_inverse_of_the_courtesy_radius() {
        let b = Balance::DEFAULT;
        let d = 3_000.0;
        let p = courteous_power_w(&b, b.rcs_spread_rad, d);
        let r = courtesy_radius_m(&b, p, b.rcs_spread_rad);
        assert!((r - d).abs() < 1.0e-9, "{r}");
        let (_, kg) = scaled(&b, 50_000.0);
        assert!(p < thrusters_power_w(&b, kg), "a GSV three kilometers off must throttle");
    }
}
