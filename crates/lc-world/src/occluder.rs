//! Discrete occluders: planets, moons, individual structures.
//!
//! Analytic, because their signature is *coherent* — a periodic box whose depth, phase and
//! ingress shape a time-averaged shell would discard. Size and angular scale are not what
//! decides the path; coherence is. See `lightcone/docs/04-stellar-photometry.md`.

use std::f64::consts::PI;

use glam::DVec3;

use crate::star::Star;

/// A body that can cross the stellar disc.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Occluder {
    pub radius_m: f64,
}

impl Occluder {
    pub fn new(radius_m: f64) -> Self {
        Self { radius_m }
    }

    /// Fractional deficit this body causes, seen from `view`, with the body at
    /// `position_m` relative to the star's centre.
    ///
    /// Zero when the body is behind the star, which is an eclipse rather than a transit.
    pub fn deficit(&self, position_m: DVec3, view: DVec3, star: &Star) -> f64 {
        match impact_parameter(position_m, view, star) {
            Some(b) => transit_depth(b, self.radius_m / star.radius_m, star.limb_darkening),
            None => 0.0,
        }
    }
}

/// Impact parameter in stellar radii, or `None` when the body is on the far side.
pub fn impact_parameter(position_m: DVec3, view: DVec3, star: &Star) -> Option<f64> {
    let v = view.normalize();
    let along = position_m.dot(v);
    if along <= 0.0 {
        return None;
    }
    Some((position_m - v * along).length() / star.radius_m)
}

/// Intensity at fractional radius `r` under quadratic limb darkening, relative to the centre.
#[inline]
fn intensity(r: f64, (u1, u2): (f64, f64)) -> f64 {
    let mu = (1.0 - r * r).max(0.0).sqrt();
    1.0 - u1 * (1.0 - mu) - u2 * (1.0 - mu) * (1.0 - mu)
}

/// Total flux of the disc, `pi (1 - u1/3 - u2/6)`.
#[inline]
pub fn disc_flux((u1, u2): (f64, f64)) -> f64 {
    PI * (1.0 - u1 / 3.0 - u2 / 6.0)
}

/// Fraction of the star's flux blocked by a disc of relative radius `ratio` at impact
/// parameter `b`, both in stellar radii.
///
/// Integrated over the **occulter's** area rather than over rings of the star. Ring
/// integration is the obvious formulation and the wrong one: the covered fraction of a ring
/// goes as a square root at the occulter's edges, and so does the limb-darkened intensity at
/// the stellar limb, so Simpson converges at a crawl and a fully-interior transit comes out
/// 0.04% light. Over the occulter the integrand is smooth wherever the occulter lies wholly
/// inside the disc, which is what a transit does for most of its duration.
///
/// This is the Mandel and Agol case without the elliptic integrals. The closed form is an
/// optimisation for later; this version is checkable against the lens area exactly.
pub fn transit_depth(b: f64, ratio: f64, limb: (f64, f64)) -> f64 {
    if ratio <= 0.0 || b >= 1.0 + ratio {
        return 0.0;
    }
    if ratio >= 1.0 + b {
        return 1.0; // the occulter covers the whole disc
    }

    let crosses_limb = b + ratio > 1.0;
    let (n_rho, n_alpha) = if crosses_limb { (48, 64) } else { (16, 24) };
    let rho_max = ratio.min(1.0 + b);

    // For a point at (b + rho cos a, rho sin a), distance from the star centre is
    // sqrt(b^2 + rho^2 + 2 b rho cos a), largest at a = 0. Points inside the disc are those
    // with |a| >= a0, so the angular measure is exactly 2 (pi - a0) and the boundary needs no
    // indicator function.
    let inner = |rho: f64| -> f64 {
        // At rho = 0 the angular factor is 0/0; the rho weight below makes it moot.
        if rho <= 0.0 {
            return 0.0;
        }
        let a0 = if b <= 0.0 {
            if rho <= 1.0 { 0.0 } else { PI }
        } else {
            let k = (1.0 - b * b - rho * rho) / (2.0 * b * rho);
            if k >= 1.0 {
                0.0
            } else if k <= -1.0 {
                PI
            } else {
                k.acos()
            }
        };
        if a0 >= PI {
            return 0.0;
        }
        2.0 * simpson(a0, PI, n_alpha, |a| {
            let r2 = b * b + rho * rho + 2.0 * b * rho * a.cos();
            intensity(r2.max(0.0).sqrt().min(1.0), limb)
        })
    };

    // inner() turns on as a square root at rho = |1 - b|, where the occulter first meets the
    // stellar limb. Simpson across that converges at n^-1.5 and costs four digits, so the
    // segment above it is integrated in w = sqrt(rho - cut), which makes the onset linear.
    let cut = (1.0 - b).abs();
    let blocked = if cut > 0.0 && cut < rho_max {
        let below = simpson(0.0, cut, n_rho, |rho| rho * inner(rho));
        let w_max = (rho_max - cut).sqrt();
        let above = simpson(0.0, w_max, n_rho, |w| {
            let rho = cut + w * w;
            2.0 * w * rho * inner(rho)
        });
        below + above
    } else {
        simpson(0.0, rho_max, n_rho, |rho| rho * inner(rho))
    };
    (blocked / disc_flux(limb)).clamp(0.0, 1.0)
}

fn simpson(lo: f64, hi: f64, intervals: usize, f: impl Fn(f64) -> f64) -> f64 {
    if hi <= lo {
        return 0.0;
    }
    let h = (hi - lo) / intervals as f64;
    let mut sum = f(lo) + f(hi);
    for i in 1..intervals {
        sum += f(lo + i as f64 * h) * if i.is_multiple_of(2) { 2.0 } else { 4.0 };
    }
    sum * h / 3.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Overlap area of two circles, radii 1 and `ratio`, centres `b` apart.
    fn lens_area(b: f64, ratio: f64) -> f64 {
        if b >= 1.0 + ratio {
            return 0.0;
        }
        if b <= (1.0 - ratio).abs() {
            return PI * ratio.min(1.0).powi(2);
        }
        let a1 = ((b * b + 1.0 - ratio * ratio) / (2.0 * b)).clamp(-1.0, 1.0).acos();
        let a2 = ((b * b + ratio * ratio - 1.0) / (2.0 * b * ratio)).clamp(-1.0, 1.0).acos();
        let tri = 0.5
            * ((-b + 1.0 + ratio) * (b + 1.0 - ratio) * (b - 1.0 + ratio) * (b + 1.0 + ratio))
                .max(0.0)
                .sqrt();
        a1 + ratio * ratio * a2 - tri
    }

    #[test]
    fn a_uniform_disc_reproduces_the_lens_area_exactly() {
        for ratio in [0.01, 0.1, 0.3] {
            for b in [0.0, 0.2, 0.5, 0.9, 1.0, 1.05, 1.2] {
                let got = transit_depth(b, ratio, (0.0, 0.0));
                let want = lens_area(b, ratio) / PI;
                // Wholly inside the disc the integrand is smooth and this is near exact;
                // straddling the limb it retains a square-root onset, which costs four
                // digits rather than nine.
                let tol = if b + ratio <= 1.0 { 1e-9 } else { 1e-6 };
                assert!(
                    (got - want).abs() < tol * want.max(1e-6),
                    "b={b} ratio={ratio}: {got} vs {want}, relative {:.2e}",
                    (got - want).abs() / want
                );
            }
        }
    }

    #[test]
    fn a_centred_occulter_without_limb_darkening_blocks_its_area() {
        assert!((transit_depth(0.0, 0.1, (0.0, 0.0)) - 0.01).abs() < 1e-12);
    }

    #[test]
    fn limb_darkening_deepens_a_central_transit() {
        let ratio = 0.1;
        let flat = transit_depth(0.0, ratio, (0.0, 0.0));
        let darkened = transit_depth(0.0, ratio, (0.4, 0.26));
        assert!(darkened > flat * 1.15, "centre is brighter: {darkened} vs {flat}");
    }

    #[test]
    fn an_earth_analogue_transits_at_the_documented_depth() {
        let ratio = 6.371e6 / 6.957e8;
        assert!((transit_depth(0.0, ratio, (0.0, 0.0)) - 8.386e-5).abs() < 1e-8);
        // With solar limb darkening a central transit is about 21% deeper than geometric.
        let d = transit_depth(0.0, ratio, Star::SOL.limb_darkening);
        assert!((d - 1.0186e-4).abs() < 1e-7, "{d}");
    }

    #[test]
    fn depth_falls_monotonically_with_impact_parameter() {
        let mut prev = f64::INFINITY;
        let mut b = 0.0;
        while b < 1.2 {
            let d = transit_depth(b, 0.1, Star::SOL.limb_darkening);
            assert!(d <= prev + 1e-12, "not monotonic at b={b}");
            prev = d;
            b += 0.01;
        }
        assert_eq!(transit_depth(1.11, 0.1, Star::SOL.limb_darkening), 0.0);
    }

    #[test]
    fn an_occulter_larger_than_the_star_blocks_everything() {
        assert!((transit_depth(0.0, 3.0, (0.4, 0.26)) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_body_behind_the_star_is_eclipsed_not_transiting() {
        let star = Star::SOL;
        let body = Occluder::new(6.371e6);
        let front = DVec3::new(1.496e11, 0.0, 0.0);
        assert!(body.deficit(front, DVec3::X, &star) > 0.0);
        assert_eq!(body.deficit(-front, DVec3::X, &star), 0.0);
        assert!(impact_parameter(-front, DVec3::X, &star).is_none());
    }

    #[test]
    fn the_geometry_finds_a_grazing_transit() {
        let star = Star::SOL;
        let a = 1.496e11;
        // Offset perpendicular by half a stellar radius.
        let pos = DVec3::new(a, 0.0, star.radius_m * 0.5);
        let b = impact_parameter(pos, DVec3::X, &star).unwrap();
        assert!((b - 0.5).abs() < 1e-9, "impact parameter {b}");
        assert!(Occluder::new(6.371e6).deficit(pos, DVec3::X, &star) > 0.0);
    }
}
