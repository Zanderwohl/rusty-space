//! An orbit from ranged positions by way of the body's own motion: its position, velocity and
//! pull toward its primary, read at the middle of the arc.
//!
//! [`super::arc`]'s conic through the positions assumes a circle over the few degrees a close
//! pass covers. Positions good to meters give the velocity and acceleration as well, and with
//! them the eccentricity.

use glam::{DMat4, DVec3, DVec4};

use super::arc::{self, Fitted, Look};
use em_foundations::kepler;

/// A cubic in time per axis: the jerk takes up the change in pull across the arc, which a
/// quadratic would fold into the acceleration and report as the wrong mass.
const TERMS: usize = 4;

/// The orbit whose state at the middle of `places` is the one the positions trace, scored
/// against every look. `None` for fewer than [`TERMS`] positions, an arc that shows no pull
/// toward the primary, or a state that is not a bound ellipse.
pub(super) fn from_motion(places: &[(DVec3, f64)], looks: &[Look], reach_m: f64) -> Option<Fitted> {
    if places.len() < TERMS {
        return None;
    }
    let (first, last) = places
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), (_, t)| (lo.min(*t), hi.max(*t)));
    let half_s = 0.5 * (last - first);
    if !arc::sound(half_s) {
        return None;
    }
    let mid_s = first + half_s;

    // Least squares in time scaled to [-1, 1], which keeps the normal matrix's entries within a
    // factor of a few of each other however long the arc.
    let mut normal = DMat4::ZERO;
    let mut rhs = [DVec4::ZERO; 3];
    for (at, t) in places {
        let tau = (t - mid_s) / half_s;
        let row = DVec4::new(1.0, tau, tau * tau, tau * tau * tau);
        normal += DMat4::from_cols(row * row.x, row * row.y, row * row.z, row * row.w);
        for (axis, sum) in rhs.iter_mut().enumerate() {
            *sum += row * at[axis];
        }
    }
    if !arc::sound(normal.determinant()) {
        return None;
    }
    let inverse = normal.inverse();
    let [x, y, z] = rhs.map(|sum| inverse * sum);
    let r = DVec3::new(x.x, y.x, z.x);
    let v = DVec3::new(x.y, y.y, z.y) / half_s;
    let a = DVec3::new(x.z, y.z, z.z) * 2.0 / (half_s * half_s);

    // The primary's pull is whatever of the acceleration points at it.
    let radius = r.length();
    let mu = -a.dot(r / radius) * radius * radius;
    elements(r, v, mu, mid_s, looks, reach_m)
}

/// The orbit with this state at `at_s`, in the terms [`Fitted`] holds it.
fn elements(r: DVec3, v: DVec3, mu: f64, at_s: f64, looks: &[Look], reach_m: f64) -> Option<Fitted> {
    if !arc::sound(mu) {
        return None;
    }
    let h = r.cross(v);
    if !arc::sound(h.length()) {
        return None;
    }
    let pole = h.normalize();
    let radius = r.length();
    let semi_major_m = 1.0 / (2.0 / radius - v.length_squared() / mu);
    let e_vec = v.cross(h) / mu - r / radius;
    let eccentricity = e_vec.length();
    if !arc::sound(semi_major_m) || !(eccentricity < 0.95) {
        return None;
    }
    let (u, w) = arc::basis(pole);
    // With no eccentricity to point anywhere, periapsis is taken where the body is.
    let toward = if eccentricity > 1.0e-9 { e_vec } else { r };
    let periapsis_rad = toward.dot(w).atan2(toward.dot(u));
    let true_anomaly = kepler::anomaly::wrap_pi(r.dot(w).atan2(r.dot(u)) - periapsis_rad);
    let mean = kepler::anomaly::mean_from_eccentric(
        kepler::anomaly::eccentric_from_true(true_anomaly, eccentricity),
        eccentricity,
    );
    let n = (mu / semi_major_m.powi(3)).sqrt();
    let mut fitted = Fitted {
        semi_major_m,
        eccentricity,
        period_s: std::f64::consts::TAU / n,
        pole,
        periapsis_rad,
        epoch_s: at_s - mean / n,
        mu,
        reach_m,
        assumed_circular: false,
        residual_rad: 0.0,
        looks: looks.len(),
    };
    fitted.residual_rad = arc::residual(&fitted, looks, f64::INFINITY)?;
    Some(fitted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AU_M: f64 = 1.495_978_707e11;
    const MU_SUN: f64 = 1.327_124_4e20;

    /// Earth-like and near perihelion, where a circle through the positions reads 0.983 AU.
    fn truth() -> Fitted {
        let semi_major_m = AU_M;
        let n = (MU_SUN / semi_major_m.powi(3)).sqrt();
        Fitted {
            semi_major_m,
            eccentricity: 0.0167,
            period_s: std::f64::consts::TAU / n,
            pole: DVec3::new(0.0, 0.05, 1.0).normalize(),
            periapsis_rad: 1.0,
            epoch_s: -2.0 * 86_400.0,
            mu: MU_SUN,
            reach_m: 200.0 * AU_M,
            assumed_circular: false,
            residual_rad: 0.0,
            looks: 0,
        }
    }

    /// Ranged looks from a ship in low orbit about the body, every `every_s`.
    fn close(truth: &Fitted, count: usize, every_s: f64) -> Vec<Look> {
        (0..count)
            .map(|i| {
                let at_s = i as f64 * every_s;
                let body = truth.at(at_s);
                let phase = at_s / 5820.0 * std::f64::consts::TAU;
                let ship = body + DVec3::new(phase.cos(), phase.sin(), 0.0) * 7.0e6;
                let offset = body - ship;
                Look {
                    from_m: ship,
                    toward: offset.normalize(),
                    at_s,
                    sigma_rad: 1.0e-7,
                    range_m: Some((offset.length(), 5.0)),
                }
            })
            .collect()
    }

    fn places(looks: &[Look]) -> Vec<(DVec3, f64)> {
        looks.iter().map(|l| (l.from_m + l.toward * l.range_m.unwrap().0, l.at_s)).collect()
    }

    /// Two days of a close pass: a hundredth of the path round, which a conic cannot shape.
    #[test]
    fn two_days_of_a_close_pass_read_the_whole_orbit() {
        let truth = truth();
        let looks = close(&truth, 13, 13_000.0);
        let fitted = from_motion(&places(&looks), &looks, truth.reach_m).expect("an orbit");
        assert!((fitted.semi_major_m / truth.semi_major_m - 1.0).abs() < 1.0e-3, "{} AU", fitted.semi_major_m / AU_M);
        assert!((fitted.eccentricity - truth.eccentricity).abs() < 2.0e-3, "e {}", fitted.eccentricity);
        assert!((fitted.mu / MU_SUN - 1.0).abs() < 1.0e-3, "mu {}", fitted.mu / MU_SUN);
        assert!(fitted.pole.angle_between(truth.pole) < 1.0e-4);
        // And it goes on to be right a month later, which a circle at the perihelion distance
        // is not.
        let later = 30.0 * 86_400.0;
        assert!(fitted.at(later).distance(truth.at(later)) < 1.0e-4 * AU_M);
    }

    /// Positions that show no pull -- a straight line -- are no orbit, rather than a mass of zero
    /// and an axis of infinity.
    #[test]
    fn a_path_that_does_not_bend_is_no_orbit() {
        let places: Vec<(DVec3, f64)> =
            (0..6).map(|i| (DVec3::new(AU_M, 3.0e4 * i as f64 * 1.0e4, 0.0), i as f64 * 1.0e4)).collect();
        assert!(from_motion(&places, &[], 200.0 * AU_M).is_none());
    }
}
