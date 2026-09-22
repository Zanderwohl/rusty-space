//! Where a star is, from which way it was seen and from where.
//!
//! A single bearing says nothing about distance. Two bearings from places a baseline apart
//! differ by the parallax, `B / d` radians, and that difference is the whole of the distance
//! measurement. See `lightcone/docs/22-provenance.md`.

use glam::DVec3;
use serde::{Deserialize, Serialize};

/// Centroiding beats the diffraction limit by the signal-to-noise ratio, down to this fraction
/// of it. Below that, systematics in the optics and the pointing win; Gaia sits near here.
pub const CENTROID_FLOOR: f64 = 1e-3;

/// Parallax signal-to-noise below which a distance is only a lower bound.
pub const PARALLAX_SNR: f64 = 2.0;

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
    /// Unit vector from the observer toward the star, with the observer's own aberration
    /// already removed: a craft knows its own velocity exactly.
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
    /// Triangulated. `sigma_ly` is along the line of sight; across it the error is the bearing
    /// error times the distance, which is far smaller.
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

/// Least squares over every bearing, in a frame whose `z` is the mean bearing.
///
/// Solved as a regression of transverse position on slope, both centered, rather than as
/// the 3x3 system `sum (I - u u^T) x = sum (I - u u^T) p`. That matrix's smallest eigenvalue
/// is the parallax squared — 1e-12 of the others at 25 ly over an AU — so inverting it in f64
/// returns noise. Centered slopes carry the same information as small numbers.
/// The angle the widest pair of observing positions subtends at `star_ly`, radians: how much
/// parallax the bearings had to work with. Two craft a light-year apart beat one orbit, and this
/// is the number that says so.
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
    if s_var <= 0.0 {
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

    /// Two craft a light-day apart see what an orbit takes months to show.
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
