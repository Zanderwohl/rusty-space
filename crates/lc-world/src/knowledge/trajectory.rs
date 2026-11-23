//! The path a craft was seen on, fitted from where it was seen, so later sightings that keep to
//! it need hold only their times. See `lightcone/docs/22-provenance.md` §Where craft are.
//!
//! Fitted from positions and nothing about how the craft is flown: a conic about the body whose
//! influence holds it, of any eccentricity, or a straight line between systems. Five places pin
//! a conic in its plane, and the body's `G m` is public, so the times then fix where on it the
//! craft is. A fit is accepted only if every place it was made from lies on it.

use em_foundations::kepler::anomaly;
use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};

use super::BodyId;
use super::arc::basis;
use crate::flight::JULIAN_YEAR_S;
use crate::system::{LocalSystem, M_PER_LY};

/// Places a fit needs: five pin a conic in its plane.
pub const FIT_PLACES: usize = 5;

/// Closer to parabolic than this is nudged off it: neither anomaly solver converges at `e = 1`.
const PARABOLIC_TOLERANCE: f64 = 1e-6;

/// Below this the places do not bend enough to say which plane they are in, relative to the
/// largest radius squared.
const FLAT: f64 = 1e-12;

/// Where a craft was seen: coordinate seconds the light left it, and light-years.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub at_s: f64,
    pub position_ly: DVec3,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Trajectory {
    /// A conic about `about`, in its frame: `pole` is the plane's normal, the craft going round
    /// it counterclockwise; `periapsis_rad` is measured in [`basis`] of the pole. Meters and
    /// seconds; `mu` is the body's `G m`. Hyperbolic when `eccentricity > 1`.
    Conic {
        about: BodyId,
        mu: f64,
        pole: DVec3,
        semi_latus_m: f64,
        eccentricity: f64,
        periapsis_rad: f64,
        periapsis_s: f64,
    },
    /// A straight line, light-years and a fraction of c.
    Line { position_ly: DVec3, beta: DVec3, epoch_s: f64 },
}

impl Trajectory {
    /// The path through `places`, oldest first, if they share one: a conic about the body holding
    /// the latest of them where there is a system, a line where there is none. `None` with fewer
    /// than [`FIT_PLACES`], or when any place is further than `tolerance_ly` off the fit, which
    /// is what a craft under thrust gives.
    pub fn fit(system: Option<&LocalSystem>, places: &[Place], tolerance_ly: f64) -> Option<Trajectory> {
        if places.len() < FIT_PLACES {
            return None;
        }
        let fitted = match system {
            Some(system) => conic(system, places)?,
            None => line(places)?,
        };
        places
            .iter()
            .all(|p| fitted.place_at(system, p.at_s).is_some_and(|at| at.distance(p.position_ly) <= tolerance_ly))
            .then_some(fitted)
    }

    /// Where the path puts the craft at coordinate time `at_s`, light-years. A conic needs the
    /// system it is in; `None` without it, or when its body is not there.
    pub fn place_at(&self, system: Option<&LocalSystem>, at_s: f64) -> Option<DVec3> {
        match *self {
            Trajectory::Line { position_ly, beta, epoch_s } => Some(position_ly + beta * ((at_s - epoch_s) / JULIAN_YEAR_S)),
            Trajectory::Conic { about, mu, pole, semi_latus_m, eccentricity, periapsis_rad, periapsis_s } => {
                let system = system?;
                let index = system.sim().indices().find(|&k| BodyId::of(system.star, system.sim().name(k)) == about)?;
                let (center_m, _) = system.body_state_at(index, at_s)?;
                let mean = rate(mu, semi_latus_m, eccentricity)? * (at_s - periapsis_s);
                let true_anomaly = anomaly::true_from_mean(mean, eccentricity)?;
                let r = semi_latus_m / (1.0 + eccentricity * true_anomaly.cos());
                let (u, v) = basis(pole);
                let angle = periapsis_rad + true_anomaly;
                let local = (u * angle.cos() + v * angle.sin()) * r;
                Some(system.origin_ly + (center_m + local) / M_PER_LY)
            }
        }
    }
}

/// Mean motion, radians a second, of the conic with this shape about `mu`.
fn rate(mu: f64, semi_latus_m: f64, eccentricity: f64) -> Option<f64> {
    let a = semi_latus_m / (1.0 - eccentricity * eccentricity);
    let n = (mu / a.abs().powi(3)).sqrt();
    n.is_finite().then_some(n)
}

/// Mean anomaly at a true anomaly, on an ellipse or a hyperbola.
fn mean_at(true_anomaly: f64, eccentricity: f64) -> Option<f64> {
    if eccentricity < 1.0 {
        Some(anomaly::mean_from_eccentric(anomaly::eccentric_from_true(true_anomaly, eccentricity), eccentricity))
    } else {
        Some(anomaly::mean_from_hyperbolic(anomaly::hyperbolic_from_true(true_anomaly, eccentricity)?, eccentricity))
    }
}

/// The plane from the area the places sweep, the shape from `1/r = A + B cos θ + C sin θ` in it,
/// and the time of periapsis from each place's mean anomaly. Only the epoch is fitted in time:
/// the body's `G m` gives the rate.
fn conic(system: &LocalSystem, places: &[Place]) -> Option<Trajectory> {
    let last = places.last()?;
    let index = system.holding(last.position_ly, last.at_s);
    let mu = system.sim().gravitational_constant() * system.sim().mass(index);
    if mu <= 0.0 || !mu.is_finite() {
        return None;
    }
    let mut local = Vec::with_capacity(places.len());
    for p in places {
        let (center_m, _) = system.body_state_at(index, p.at_s)?;
        local.push(((p.position_ly - system.origin_ly) * M_PER_LY - center_m, p.at_s));
    }
    // Twice the area each step sweeps, so it points along the motion's own normal; a chord's
    // cross product would be the arc's curvature, far smaller on a short arc.
    let mut pole = DVec3::ZERO;
    let mut scale = 0.0f64;
    for pair in local.windows(2) {
        pole += pair[0].0.cross(pair[1].0);
        scale = scale.max(pair[0].0.length_squared()).max(pair[1].0.length_squared());
    }
    if !(pole.length() > FLAT * scale) {
        return None;
    }
    let pole = pole.normalize();
    let (u, v) = basis(pole);

    let mut normal = DMat3::ZERO;
    let mut rhs = DVec3::ZERO;
    let polar: Vec<(f64, f64, f64)> = local.iter().map(|(r, t)| (r.dot(v).atan2(r.dot(u)), r.length(), *t)).collect();
    for &(theta, r, _) in &polar {
        let row = DVec3::new(1.0, theta.cos(), theta.sin());
        normal += DMat3::from_cols(row * row.x, row * row.y, row * row.z);
        rhs += row / r;
    }
    let solved = normal.inverse() * rhs;
    if !solved.is_finite() || solved.x <= 0.0 {
        return None;
    }
    let semi_latus_m = 1.0 / solved.x;
    let mut eccentricity = solved.y.hypot(solved.z) / solved.x;
    if (eccentricity - 1.0).abs() < PARABOLIC_TOLERANCE {
        eccentricity = 1.0 + PARABOLIC_TOLERANCE.copysign(eccentricity - 1.0);
    }
    let periapsis_rad = solved.z.atan2(solved.y);
    let n = rate(mu, semi_latus_m, eccentricity)?;

    // Each place says when periapsis was; on an ellipse only up to whole periods, so each is
    // carried to the turn nearest the first before they are averaged.
    let period_s = (eccentricity < 1.0).then(|| std::f64::consts::TAU / n);
    let mut sum = 0.0;
    let mut first = None;
    for &(theta, _, t) in &polar {
        let mean = mean_at(anomaly::wrap_pi(theta - periapsis_rad), eccentricity)?;
        let mut passed = t - mean / n;
        let anchor = *first.get_or_insert(passed);
        if let Some(period) = period_s {
            passed -= ((passed - anchor) / period).round() * period;
        }
        sum += passed;
    }
    Some(Trajectory::Conic {
        about: BodyId::of(system.star, system.sim().name(index)),
        mu,
        pole,
        semi_latus_m,
        eccentricity,
        periapsis_rad,
        periapsis_s: sum / polar.len() as f64,
    })
}

/// Least squares of position on time.
fn line(places: &[Place]) -> Option<Trajectory> {
    let count = places.len() as f64;
    let epoch_s = places.iter().map(|p| p.at_s).sum::<f64>() / count;
    let mean = places.iter().map(|p| p.position_ly).sum::<DVec3>() / count;
    let spread: f64 = places.iter().map(|p| (p.at_s - epoch_s).powi(2)).sum();
    if !(spread > 0.0) {
        return None;
    }
    let per_s = places.iter().map(|p| (p.position_ly - mean) * (p.at_s - epoch_s)).sum::<DVec3>() / spread;
    Some(Trajectory::Line { position_ly: mean, beta: per_s * JULIAN_YEAR_S, epoch_s })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coast::Coast;
    use crate::navigation::{Course, Plane};

    fn sol() -> LocalSystem {
        let provider = crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv").expect("the catalog");
        let sun = crate::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))
            .expect("the Sun")
            .clone();
        let mut system = LocalSystem::for_star(&sun).expect("the solar system");
        system.advance_to(0.0);
        system
    }

    fn m(a: DVec3, b: DVec3) -> f64 {
        a.distance(b) * M_PER_LY
    }

    const KM_LY: f64 = 1.0e3 / M_PER_LY;

    /// A held orbit of Jupiter, five places twenty minutes apart, followed for three days.
    #[test]
    fn five_places_on_a_held_orbit_give_a_conic_that_keeps_to_it() {
        let system = sol();
        let station = Course::Orbit { body: "Jupiter".into(), altitude_radii: 0.5, plane: Plane::Equatorial }
            .resolve(&system, DVec3::ZERO, 0.0)
            .expect("an orbit");
        let places: Vec<Place> = (0..5)
            .map(|k| {
                let at_s = 1.0e5 + 1200.0 * k as f64;
                Place { at_s, position_ly: station.place_at(&system, at_s).unwrap() }
            })
            .collect();
        let fitted = Trajectory::fit(Some(&system), &places, KM_LY).expect("a conic");
        assert!(matches!(fitted, Trajectory::Conic { .. }), "{fitted:?}");
        let later = 1.0e5 + 3.0 * 86_400.0;
        let off = m(fitted.place_at(Some(&system), later).unwrap(), station.place_at(&system, later).unwrap());
        assert!(off < 1.0e4, "off by {off} m after three days");
    }

    /// An eccentric arc and an escape: nothing here assumes the path is a circle.
    #[test]
    fn an_ellipse_and_a_hyperbola_are_fitted_as_what_they_are() {
        let system = sol();
        let earth = system.body_named("Earth").unwrap();
        let (center, carried) = system.body_state_at(earth, 0.0).unwrap();
        let mu = system.sim().gravitational_constant() * system.sim().mass(earth);
        let r0 = 7.0e6;
        for (speed, hyperbolic) in [(1.25, false), (1.6, true)] {
            let circular = (mu / r0).sqrt();
            let at_ly = system.origin_ly + (center + DVec3::X * r0) / M_PER_LY;
            let velocity = carried + DVec3::Y * circular * speed;
            let coast = Coast::from_state(&system, at_ly, velocity, 0.0).expect("a coast");
            let places: Vec<Place> = (0..6)
                .map(|k| {
                    let at_s = 300.0 * k as f64;
                    Place { at_s, position_ly: coast.at(&system, at_s).unwrap().0 }
                })
                .collect();
            let fitted = Trajectory::fit(Some(&system), &places, KM_LY).expect("a conic");
            let Trajectory::Conic { eccentricity, .. } = fitted else { panic!("{fitted:?}") };
            assert_eq!(eccentricity > 1.0, hyperbolic, "e = {eccentricity}");
            let later = 4.0 * 3600.0;
            let off = m(fitted.place_at(Some(&system), later).unwrap(), coast.at(&system, later).unwrap().0);
            assert!(off < 1.0e4, "speed {speed}: off by {off} m four hours on");
        }
    }

    /// Places under thrust lie on no conic, and are refused rather than fitted loosely.
    #[test]
    fn places_under_thrust_fit_nothing() {
        let system = sol();
        let earth = system.body_named("Earth").unwrap();
        let places: Vec<Place> = (0..6)
            .map(|k| {
                let at_s = 300.0 * k as f64;
                let (center, _) = system.body_state_at(earth, at_s).unwrap();
                let pushed = DVec3::X * (7.0e6 + 0.5 * 10.0 * at_s * at_s);
                Place { at_s, position_ly: system.origin_ly + (center + pushed) / M_PER_LY }
            })
            .collect();
        assert_eq!(Trajectory::fit(Some(&system), &places, KM_LY), None);
    }

    #[test]
    fn between_systems_the_path_is_a_line() {
        let places: Vec<Place> =
            (0..5).map(|k| Place { at_s: 1.0e6 * k as f64, position_ly: DVec3::new(3.0, 0.0, 0.0) + DVec3::Y * 1e-4 * k as f64 }).collect();
        let fitted = Trajectory::fit(None, &places, 1e-12).expect("a line");
        let at = fitted.place_at(None, 1.0e7).unwrap();
        assert!(at.distance(DVec3::new(3.0, 1e-3, 0.0)) < 1e-12, "{at}");
        assert!(Trajectory::fit(None, &places[..4], 1e-12).is_none(), "four places are not enough");
    }
}
