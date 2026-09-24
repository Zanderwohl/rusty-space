//! Where a star is, from which way it was seen and from where.
//!
//! Two bearings from places a baseline `B` apart differ by the parallax, `B / d` radians,
//! which is the whole distance measurement. See `lightcone/docs/22-provenance.md`.

use glam::{DMat3, DVec3};
use serde::{Deserialize, Serialize};

/// Centroiding beats the diffraction limit by the signal-to-noise ratio, down to this fraction
/// of it. Below that, systematics in the optics and the pointing win; Gaia sits near here.
pub const CENTROID_FLOOR: f64 = 1e-3;

/// Parallax signal-to-noise below which a distance is only a lower bound.
pub const PARALLAX_SNR: f64 = 2.0;

/// Best fractional precision on a resolved disc's angular diameter.
///
/// The counterpart of [`CENTROID_FLOOR`] for a size rather than a position, and a floor for a
/// different reason: the limb of a real body is not a step, and how it darkens toward the edge
/// is a model rather than a measurement. Real interferometric stellar diameters do worse than
/// this; a planet's sharp edge does better, and one number for both is as much as an instrument
/// that cannot tell which it is looking at can claim.
pub const LIMB_FLOOR: f64 = 1.0e-3;

/// Diameter of a filled circular aperture of this area.
pub fn diameter_m(aperture_m2: f64) -> f64 {
    2.0 * (aperture_m2.max(0.0) / std::f64::consts::PI).sqrt()
}

/// Rayleigh criterion. `baseline_m` is the widest separation between elements acting as one
/// instrument; a lone telescope's is its own diameter.
pub fn resolution_rad(wavelength_m: f64, baseline_m: f64) -> f64 {
    1.22 * wavelength_m / baseline_m.max(f64::MIN_POSITIVE)
}

/// One sigma on a bearing, per axis.
pub fn centroid_sigma_rad(resolution_rad: f64, snr: f64) -> f64 {
    resolution_rad / snr.clamp(1.0, 1.0 / CENTROID_FLOOR)
}

/// One bearing to a star: from where, which way, and how well.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Bearing {
    pub observer_ly: DVec3,
    /// Unit vector, with the observer's own aberration already removed.
    pub toward: DVec3,
    pub sigma_rad: f64,
}

/// A distance, as well as the bearings support one.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub enum Distance {
    /// Nothing but directions, all from effectively one place.
    #[default]
    Unknown,
    /// No parallax found, which says the star is at least this far away, light-years.
    AtLeast(f64),
    /// `sigma_ly` is along the line of sight; across it the error is far smaller.
    Measured { position_ly: DVec3, sigma_ly: f64 },
}

impl Distance {
    pub fn position_ly(&self) -> Option<DVec3> {
        match self {
            Self::Measured { position_ly, .. } => Some(*position_ly),
            _ => None,
        }
    }

    /// From `observer_ly`, light-years.
    pub fn from(&self, observer_ly: DVec3) -> Option<f64> {
        self.position_ly().map(|p| p.distance(observer_ly))
    }
}

/// The angle the widest pair of observing positions subtends at `star_ly`, radians.
pub fn baseline_rad(bearings: &[Bearing], star_ly: DVec3) -> f64 {
    let mut widest: f64 = 0.0;
    for (i, a) in bearings.iter().enumerate() {
        for b in bearings.iter().skip(i + 1) {
            let toward = (star_ly - (a.observer_ly + b.observer_ly) * 0.5).normalize_or_zero();
            let apart = b.observer_ly - a.observer_ly;
            let across = (apart - toward * apart.dot(toward)).length();
            let range = (star_ly - a.observer_ly).length();
            if range > 0.0 {
                widest = widest.max(across / range);
            }
        }
    }
    widest
}

/// Bearings spread by more than this about their mean are solved by [`intersect`] instead.
///
/// Radians. The two solvers fail in each other's regime and nowhere else: at this spread the
/// 3x3 system's condition number is about 800, which f64 inverts without complaint, and at the
/// 1e-6 of a star 25 ly off over an AU baseline it is 1e12, which it does not. The regression
/// below runs in a frame that is a projection about the mean bearing, so it needs the bearings
/// to share a direction and has nothing to say about a source seen from all round it.
pub const WIDE_RAD: f64 = 0.05;

/// Transverse spread of the observing positions below which no depth is read from them: a
/// kilometer, far under any baseline that measures anything and far over rounding error.
const MIN_BASELINE_LY: f64 = 1.0e3 / crate::system::M_PER_LY;

/// Smallest determinant, against the cube of the matrix's own largest entry, that
/// [`intersect`] will invert.
const CONDITION: f64 = 1.0e-12;

/// Least squares intersection of the lines of sight themselves:
/// `sum (I - u u^T) x = sum (I - u u^T) p`.
///
/// What makes that matrix invertible is the bearings pointing in genuinely different
/// directions, so this is only for a source near enough that they do. The ship's own sun 5 AU
/// off, seen from around an orbit, is the case it exists for, and the case the regression in
/// [`triangulate`] cannot take at all.
///
/// Two passes, because the measurement is an angle and the residual is a length: the first
/// finds where the source roughly is, the second weights each bearing by `sigma_rad * range`,
/// which is what its miss distance is actually worth.
fn intersect(bearings: &[Bearing]) -> Option<Distance> {
    let across = |u: DVec3| DMat3::IDENTITY - DMat3::from_cols(u * u.x, u * u.y, u * u.z);
    let solve = |weight: &dyn Fn(&Bearing) -> f64| {
        let mut normal = DMat3::ZERO;
        let mut rhs = DVec3::ZERO;
        for b in bearings {
            let w = weight(b);
            let p = across(b.toward);
            normal += p * w;
            rhs += (p * b.observer_ly) * w;
        }
        (normal, rhs)
    };
    // Against the matrix's own scale, not against zero: the entries are `1 / sigma^2` and run
    // to 1e18, so a determinant of 1e40 is singular and `> 0.0` would have accepted it.
    let inverse = |normal: DMat3| {
        let scale = normal.to_cols_array().iter().fold(0.0f64, |m, e| m.max(e.abs()));
        (normal.determinant().abs() > CONDITION * scale * scale * scale)
            .then(|| normal.inverse())
    };
    let angular = |b: &Bearing| 1.0 / (b.sigma_rad * b.sigma_rad).max(f64::MIN_POSITIVE);
    let (normal, rhs) = solve(&angular);
    let rough = inverse(normal)? * rhs;

    let weight = |b: &Bearing| {
        let miss = b.sigma_rad * rough.distance(b.observer_ly);
        1.0 / (miss * miss).max(f64::MIN_POSITIVE)
    };
    let (normal, rhs) = solve(&weight);
    let covariance = inverse(normal)?;
    let position_ly = covariance * rhs;
    // The covariance is the inverse of the normal matrix once the weights are lengths. Report
    // it along the mean line of sight, which is what `Distance::Measured` documents.
    let toward = bearings
        .iter()
        .map(|b| (position_ly - b.observer_ly).normalize_or_zero())
        .sum::<DVec3>()
        .normalize_or(DVec3::Z);
    let variance = toward.dot(covariance * toward);
    if !position_ly.is_finite() || !variance.is_finite() || variance < 0.0 {
        return None;
    }
    Some(Distance::Measured { position_ly, sigma_ly: variance.sqrt() })
}

/// Least squares over every bearing, in a frame whose `z` is the mean bearing.
///
/// A regression of transverse position on slope, not the 3x3 system
/// `sum (I - u u^T) x = sum (I - u u^T) p`: that matrix's smallest eigenvalue is the parallax
/// squared, 1e-12 of the others at 25 ly over an AU, so inverting it in f64 returns noise. For
/// a source near enough that the bearings to it spread by more than [`WIDE_RAD`] the same
/// matrix is well conditioned and this projection is the thing that breaks, so [`intersect`]
/// takes that case.
#[allow(clippy::indexing_slicing)] // axes are 0 and 1 of two-element arrays and a vector
pub fn triangulate(bearings: &[Bearing]) -> Distance {
    let weight = |b: &Bearing| 1.0 / (b.sigma_rad * b.sigma_rad).max(f64::MIN_POSITIVE);
    let total: f64 = bearings.iter().map(weight).sum();
    if bearings.len() < 2 || total <= 0.0 {
        return Distance::Unknown;
    }
    let z = bearings
        .iter()
        .map(|b| b.toward * weight(b))
        .sum::<DVec3>()
        .normalize_or_zero();
    if z == DVec3::ZERO {
        return Distance::Unknown;
    }
    // A source close enough to be seen from around belongs to the other solver, and the
    // widest bearing decides rather than an average: one bearing past the gate below is one
    // the regression would silently drop.
    let widest = bearings
        .iter()
        .map(|b| b.toward.angle_between(z))
        .fold(0.0, f64::max);
    if widest > WIDE_RAD {
        return intersect(bearings).unwrap_or(Distance::Unknown);
    }
    let (x, y) = z.any_orthonormal_pair();
    let origin = bearings
        .iter()
        .map(|b| b.observer_ly * weight(b))
        .sum::<DVec3>()
        / total;

    struct Row {
        w: f64,
        s: [f64; 2],
        q: DVec3,
    }
    let rows: Vec<Row> = bearings
        .iter()
        .filter(|b| b.toward.dot(z) > 0.5)
        .map(|b| {
            let along = b.toward.dot(z);
            let d = b.observer_ly - origin;
            Row {
                w: weight(b),
                s: [b.toward.dot(x) / along, b.toward.dot(y) / along],
                q: DVec3::new(d.dot(x), d.dot(y), d.dot(z)),
            }
        })
        .collect();
    let total: f64 = rows.iter().map(|r| r.w).sum();
    let mean = |f: &dyn Fn(&Row) -> f64| rows.iter().map(|r| r.w * f(r)).sum::<f64>() / total;

    // Per transverse axis the line of sight through observer `i` crosses depth `r` at
    // `q + s (r - q_z)`, so the star's transverse coordinate `a` satisfies `a = y - s r`
    // with `y = q - s q_z`: a regression of `y` on `s` whose slope is `-r`.
    let mut s_var = 0.0;
    let mut sy_cov = 0.0;
    let mut spread = 0.0;
    let mut intercepts = [(0.0, 0.0); 2];
    for (axis, intercept) in intercepts.iter_mut().enumerate() {
        let yv = |r: &Row| r.q[axis] - r.s[axis] * r.q.z;
        let (s_mean, y_mean) = (mean(&|r| r.s[axis]), mean(&|r| yv(r)));
        let q_mean = mean(&|r| r.q[axis]);
        for r in &rows {
            let ds = r.s[axis] - s_mean;
            s_var += r.w * ds * ds;
            sy_cov += r.w * ds * (yv(r) - y_mean);
            spread += r.w * (r.q[axis] - q_mean).powi(2);
        }
        *intercept = (s_mean, y_mean);
    }
    // Twice the distance at which the transverse baseline would show a parallax at the
    // threshold: what "no parallax found" rules out.
    let floor_ly = spread.sqrt() / PARALLAX_SNR;
    // Without a baseline the slope is rounding error and the source's own motion, whose ratio
    // passes the test below.
    let baseline_ly = (spread / total).sqrt();
    if s_var <= 0.0 || !(baseline_ly >= MIN_BASELINE_LY) {
        return if floor_ly > 0.0 {
            Distance::AtLeast(floor_ly)
        } else {
            Distance::Unknown
        };
    }
    let depth = -sy_cov / s_var;
    // Residuals scale with depth because the noise is angular.
    let sigma = depth.abs() / s_var.sqrt();
    if depth <= 0.0 || depth < PARALLAX_SNR * sigma {
        return if floor_ly > 0.0 {
            Distance::AtLeast(floor_ly)
        } else {
            Distance::Unknown
        };
    }
    let transverse = |axis: usize| {
        let (s_mean, y_mean) = intercepts[axis];
        y_mean + s_mean * depth
    };
    let position_ly = origin + x * transverse(0) + y * transverse(1) + z * depth;
    Distance::Measured {
        position_ly,
        sigma_ly: sigma,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng;

    const AU_LY: f64 = 1.581_250_7e-5;

    fn sighted(star: DVec3, at: DVec3, sigma: f64, seed: u64) -> Bearing {
        let toward = (star - at).normalize();
        let (x, y) = toward.any_orthonormal_pair();
        let nudge = x * rng::gaussian(rng::hash(&[seed, 1])) * sigma
            + y * rng::gaussian(rng::hash(&[seed, 2])) * sigma;
        Bearing {
            observer_ly: at,
            toward: (toward + nudge).normalize(),
            sigma_rad: sigma,
        }
    }

    /// Bearings from one place give no distance, even to a source that wobbles about its
    /// barycenter.
    #[test]
    fn bearings_from_one_place_give_no_distance() {
        let at = DVec3::new(5.0, 0.0, 0.0) * AU_LY;
        let bearings: Vec<Bearing> = (0..16)
            .map(|i| {
                let wobble = DVec3::new(0.0, i as f64 * 1.0e-4, 0.0) * AU_LY;
                sighted(wobble, at, 3.0e-10, i)
            })
            .collect();
        assert!(!matches!(triangulate(&bearings), Distance::Measured { .. }), "{:?}", triangulate(&bearings));
    }

    /// An orbit of one AU about a point, sampled over half a year.
    fn orbit(star: DVec3, sigma: f64, n: u64) -> Vec<Bearing> {
        (0..n)
            .map(|i| {
                let phase = std::f64::consts::PI * i as f64 / (n - 1) as f64;
                let at = DVec3::new(phase.cos(), phase.sin(), 0.0) * AU_LY;
                sighted(star, at, sigma, i)
            })
            .collect()
    }

    /// The ship's own sun, 5 AU off, seen from all round a 5 AU orbit: every other number in
    /// its system hangs off this one, and the regression cannot take it at all -- bearings
    /// spread over a circle have no mean direction to project about.
    #[test]
    fn a_ship_measures_the_distance_to_its_own_sun() {
        let star = DVec3::new(2.0, -1.0, 0.5);
        let radius = 5.0 * AU_LY;
        // The centroid floor of a ship's telescope in V: resolution times CENTROID_FLOOR.
        let sigma = 2.979e-7 * CENTROID_FLOOR;
        let bearings: Vec<Bearing> = (0..crate::knowledge::BEARINGS_KEPT as u64)
            .map(|i| {
                let phase = std::f64::consts::TAU * i as f64 / crate::knowledge::BEARINGS_KEPT as f64;
                let at = star + DVec3::new(phase.cos(), phase.sin(), 0.0) * radius;
                sighted(star, at, sigma, i)
            })
            .collect();

        let Distance::Measured { position_ly, sigma_ly } = triangulate(&bearings) else {
            panic!("a sun seen from around its own orbit is measurable: {:?}", triangulate(&bearings))
        };
        let error = position_ly.distance(star);
        assert!(error < 5.0 * sigma_ly, "{error} ly out against a sigma of {sigma_ly}");
        assert!(
            sigma_ly / radius < 1.0e-8,
            "a part in {} of 5 AU is not enough for 1% planet radii",
            radius / sigma_ly
        );
    }

    /// The gate is on the widest bearing and not on an average, because one bearing outside it
    /// is one the regression drops in silence.
    #[test]
    fn the_solver_switches_on_how_far_the_bearings_spread() {
        let star = DVec3::new(10.0, 0.0, 0.0);
        let sigma = 1.0e-9;
        // A baseline wide enough that the bearings to a star 10 ly off stay well inside the
        // gate, so this is the regression's case however many bearings there are.
        let narrow: Vec<Bearing> = (0..8)
            .map(|i| sighted(star, DVec3::Y * AU_LY * i as f64, sigma, i))
            .collect();
        let widest = narrow
            .iter()
            .map(|b| b.toward.angle_between(narrow[0].toward))
            .fold(0.0, f64::max);
        assert!(widest < WIDE_RAD, "{widest} rad should be the regression's");
        assert!(matches!(triangulate(&narrow), Distance::Measured { .. }));

        // The same star, but with one bearing taken from far enough off to spread past the
        // gate. The intersection must take it and must still land on the star.
        let mut wide = narrow.clone();
        wide.push(sighted(star, DVec3::Y * 1.0, sigma, 99));
        let Distance::Measured { position_ly, .. } = triangulate(&wide) else {
            panic!("the intersection takes the wide case")
        };
        assert!(position_ly.distance(star) < 1.0e-3, "{position_ly} against {star}");
    }

    /// A wider orbit is a wider baseline and buys precision on everything out in the sky,
    /// linearly. The host star is the one thing it does not help: it sits at the center of the
    /// orbit, so the baseline and the range to it grow together.
    #[test]
    fn a_wider_orbit_buys_the_sky_and_not_the_sun() {
        let star = DVec3::new(2.0, -1.0, 0.5);
        let far = star + DVec3::X * 10.0;
        let sigma = 2.979e-7 * CENTROID_FLOOR;
        let round = |radius: f64, target: DVec3, salt: u64| -> f64 {
            let bearings: Vec<Bearing> = (0..16u64)
                .map(|i| {
                    let phase = std::f64::consts::TAU * i as f64 / 16.0;
                    let at = star + DVec3::new(phase.cos(), phase.sin(), 0.0) * radius;
                    sighted(target, at, sigma, i + salt)
                })
                .collect();
            match triangulate(&bearings) {
                Distance::Measured { sigma_ly, .. } => sigma_ly,
                other => panic!("{radius} ly orbit gave {other:?}"),
            }
        };

        let (near, wide) = (1.0 * AU_LY, 30.0 * AU_LY);
        let gain = round(near, far, 500) / round(wide, far, 500);
        assert!(
            (gain - 30.0).abs() < 1.0,
            "thirty times the baseline is {gain} times the precision"
        );

        // And the sun's own fractional precision is the same from either orbit.
        let host = |radius: f64| radius / round(radius, star, 0);
        let (close, out) = (host(near), host(wide));
        assert!(
            (close / out - 1.0).abs() < 0.2,
            "a part in {close:e} from 1 AU against {out:e} from 30"
        );
    }

    #[test]
    fn half_an_orbit_measures_a_nearby_star() {
        let star = DVec3::new(3.0, 2.5, 1.0);
        let Distance::Measured {
            position_ly,
            sigma_ly,
        } = triangulate(&orbit(star, 1e-9, 12))
        else {
            panic!("a star 4 ly off at 1 AU is a parallax of an arcsecond");
        };
        assert!(
            position_ly.distance(star) < 4.0 * sigma_ly.max(1e-6),
            "{position_ly} vs {star}"
        );
        assert!(sigma_ly < 0.01, "{sigma_ly}");
    }

    #[test]
    fn precision_falls_as_distance_squared() {
        let dir = DVec3::new(1.0, 1.0, 0.2).normalize();
        let sigma_at = |d: f64| match triangulate(&orbit(dir * d, 1e-9, 12)) {
            Distance::Measured { sigma_ly, .. } => sigma_ly,
            other => panic!("{d} ly: {other:?}"),
        };
        let ratio = sigma_at(200.0) / sigma_at(20.0);
        assert!((ratio / 100.0 - 1.0).abs() < 0.3, "ratio {ratio}");
    }

    #[test]
    fn one_place_gives_no_distance() {
        let star = DVec3::new(5.0, 0.0, 0.0);
        let still: Vec<_> = (0..8)
            .map(|i| sighted(star, DVec3::ZERO, 1e-8, i))
            .collect();
        assert_eq!(triangulate(&still), Distance::Unknown);
        assert_eq!(triangulate(&still[..1]), Distance::Unknown);
    }

    #[test]
    fn a_star_beyond_the_baseline_is_only_bounded() {
        let star = DVec3::new(0.0, 0.0, 5.0e4);
        match triangulate(&orbit(star, 1e-8, 12)) {
            Distance::AtLeast(ly) => assert!(ly > 1.0 && ly < 5.0e4, "{ly}"),
            other => panic!("{other:?}"),
        }
    }

    /// Two craft a light-day apart measure better than half an orbit at 1 AU.
    #[test]
    fn a_wide_pair_beats_a_year_of_orbiting() {
        let star = DVec3::new(0.0, 300.0, 0.0);
        let day_ly = 1.0 / 365.25;
        let pair = vec![
            sighted(star, DVec3::ZERO, 1e-9, 1),
            sighted(star, DVec3::X * day_ly, 1e-9, 2),
        ];
        let (
            Distance::Measured { sigma_ly: wide, .. },
            Distance::Measured {
                sigma_ly: orbit, ..
            },
        ) = (triangulate(&pair), triangulate(&orbit(star, 1e-9, 12)))
        else {
            panic!("both should measure it");
        };
        assert!(wide < orbit / 10.0, "{wide} vs {orbit}");
    }

    #[test]
    fn a_swarm_resolves_what_one_mirror_cannot() {
        let lone = resolution_rad(551e-9, diameter_m(4.0));
        let swarm = resolution_rad(551e-9, 1.0e4);
        assert!(lone / swarm > 4.0e3);
        assert!(centroid_sigma_rad(lone, 1e9) >= lone * CENTROID_FLOOR);
    }
}
