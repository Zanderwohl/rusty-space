//! The parking orbit: how far from its star a ship fills fastest while its field stays below where
//! Auto turns Clear. See `lightcone/docs/20-solar-power.md` §Parking.
//!
//! Closer than where arriving starlight meets the engines' rating buys heat and no speed, so that
//! is the nearest worth going; the heat ceiling can only push the orbit out from there.

use crate::field::Mode;
use crate::fitting::{Fitting, Setting};
use crate::navigation::{Course, Plane};
use crate::solar;
use crate::system::LocalSystem;

/// The most the star's luminosity may be in doubt, as a fraction, before a parking orbit is offered.
/// Distance goes as its root, so this is 2.5% on where to park.
pub const CHARACTERIZED: f64 = 0.05;

/// How far the orbit held may be from the one wanted, as a fraction, before a correction is flown.
/// Without a deadband every small change of belief would be flown.
pub const REPARK: f64 = 0.01;

pub fn for_host(fitting: &Fitting, now_s: f64, host: &crate::knowledge::Host) -> Option<Parking> {
    if host.luminosity_fraction()? > CHARACTERIZED {
        return None;
    }
    of(fitting, now_s, host.luminosity_w?.0)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Parking {
    pub distance_m: f64,
    /// W, broadside and Black.
    pub arriving_w: f64,
    /// Where a filling ship's heat settles, J.
    pub heat_j: f64,
    /// Into storage net of the drain, W.
    pub filling_w: f64,
}

/// The arriving power to park at: the rating, or less if the heat there would pass `ceiling_w`, the
/// heat power whose equilibrium is the ceiling. `None` when the drain alone reaches it.
pub fn arriving_w(rating_w: f64, efficiency: f64, drain_w: f64, ceiling_w: f64) -> Option<f64> {
    let cooled_w = (ceiling_w - drain_w) / (1.0 - efficiency);
    (cooled_w > 0.0 && rating_w > 0.0).then(|| cooled_w.min(rating_w))
}

/// The ceiling is the Auto order's `clear_above`, or the balance's default when not in Auto.
pub fn of(fitting: &Fitting, now_s: f64, luminosity_w: f64) -> Option<Parking> {
    let balance = fitting.balance();
    let caps = fitting.capacities_at(now_s);
    let field = fitting.field();
    let clear_above = match fitting.posture().setting {
        Setting::Auto(thresholds) => thresholds.clear_above,
        Setting::Clear | Setting::Black => balance.auto_clear_above,
    };
    let efficiency = balance.conversion_efficiency;
    let ceiling_w = clear_above * field.rated_load_w();
    let arriving_w = arriving_w(caps.aperture_w, efficiency, caps.drain_w, ceiling_w)?;

    let geometry = fitting.geometry();
    let broadside_m2 = solar::shadow_m2(geometry, solar::idle_cos(geometry));
    let at_one_m_w = solar::intake_w(balance, broadside_m2, luminosity_w, 1.0);
    let distance_m = (at_one_m_w / arriving_w).sqrt();
    if !distance_m.is_finite() || distance_m <= 0.0 {
        return None;
    }
    let absorbed_w = arriving_w * Mode::Black.absorptivity(balance.clear_absorptivity);
    let converted_w = absorbed_w.min(caps.aperture_w);
    let heat_w = absorbed_w - efficiency * converted_w + caps.drain_w;
    Some(Parking {
        distance_m,
        arriving_w,
        heat_j: field.equilibrium_j(heat_w),
        filling_w: efficiency * converted_w - caps.drain_w,
    })
}

pub fn course(system: &LocalSystem, distance_m: f64) -> Option<Course> {
    let primary = system.primary();
    let radius_m = system.sim().radius(primary);
    let altitude_radii = distance_m / radius_m - 1.0;
    (radius_m > 0.0 && altitude_radii > 0.0).then(|| Course::Orbit {
        body: system.sim().name(primary).to_string(),
        altitude_radii,
        plane: Plane::Equatorial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitting::{Balance, Posture, Thresholds};
    use crate::form::Form;
    use crate::solar::SOLAR_CONSTANT_W_M2;
    use crate::system::UNIT_M as AU_M;

    fn sol_w() -> f64 {
        SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * AU_M * AU_M
    }

    fn starting(clear_above: f64) -> Fitting {
        let b = Balance::DEFAULT;
        let mut fitting = Fitting::full(Form::starting(), b, 0.0);
        let thresholds = Thresholds { clear_above, ..Thresholds::of(&b) };
        fitting.set_posture(Posture { setting: Setting::Auto(thresholds), ..Posture::new_ship(&b) });
        fitting
    }

    /// At the default ceiling the starting ship is rating-bound, inside the rated-load anchor, and
    /// settles below Auto's switch.
    #[test]
    fn the_starting_ship_parks_where_starlight_meets_its_rating() {
        let fitting = starting(Balance::DEFAULT.auto_clear_above);
        let parking = of(&fitting, 0.0, sol_w()).expect("a parking orbit");
        let rating_w = fitting.capacities_at(0.0).aperture_w;
        assert!((parking.arriving_w / rating_w - 1.0).abs() < 1e-12, "{parking:?}");
        let au = parking.distance_m / AU_M;
        assert!((0.03..crate::fitting::RATED_LOAD_AU).contains(&au), "{au} AU");
        let ceiling_j = Balance::DEFAULT.auto_clear_above * fitting.field().heat_max_j();
        assert!(parking.heat_j < ceiling_j, "{} of {ceiling_j}", parking.heat_j);
        assert!(parking.filling_w > 0.0);
    }

    /// A lower ceiling moves the orbit out until the filling heat settles on it.
    #[test]
    fn a_low_ceiling_moves_the_orbit_out_to_it() {
        let rating_bound = of(&starting(Balance::DEFAULT.auto_clear_above), 0.0, sol_w()).unwrap();
        let fitting = starting(0.2);
        let parking = of(&fitting, 0.0, sol_w()).unwrap();
        assert!(parking.distance_m > rating_bound.distance_m);
        let ceiling_j = 0.2 * fitting.field().heat_max_j();
        assert!((parking.heat_j / ceiling_j - 1.0).abs() < 1e-9, "{} against {ceiling_j}", parking.heat_j);
    }

    #[test]
    fn distance_goes_as_the_root_of_luminosity() {
        let fitting = starting(Balance::DEFAULT.auto_clear_above);
        let one = of(&fitting, 0.0, sol_w()).unwrap();
        let four = of(&fitting, 0.0, 4.0 * sol_w()).unwrap();
        assert!((four.distance_m / one.distance_m - 2.0).abs() < 1e-12);
    }

    #[test]
    fn a_drain_past_the_ceiling_has_nowhere_to_park() {
        assert_eq!(arriving_w(1e20, 0.7, 5e18, 4e18), None);
        let cooled = arriving_w(1e20, 0.7, 1e18, 4e18).unwrap();
        assert!((cooled / 1e19 - 1.0).abs() < 1e-12, "{cooled}");
        assert_eq!(arriving_w(5e18, 0.7, 1e18, 4e18), Some(5e18));
    }

    #[test]
    fn the_course_is_an_orbit_of_the_star_at_that_distance() {
        let stars = crate::sky::AuthoredStars::sample();
        let star = crate::sky::StarProvider::stars(&stars)[2].clone();
        let system = LocalSystem::for_star(&star).expect("a system");
        let radius_m = system.sim().radius(system.primary());
        let Some(Course::Orbit { body, altitude_radii, .. }) = course(&system, 0.04 * AU_M) else {
            panic!("an orbit");
        };
        assert_eq!(system.body_named(&body), Some(system.primary()));
        assert!((radius_m * (1.0 + altitude_radii) / (0.04 * AU_M) - 1.0).abs() < 1e-12);
        assert_eq!(course(&system, 0.5 * radius_m), None, "inside the star");
    }
}
