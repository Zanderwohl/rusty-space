//! Distributions over orbital elements.

use serde::{Deserialize, Serialize};

/// A distribution as quadrature nodes: `(value, weight)`, weights summing to 1.
///
/// Used for semi-major axis and eccentricity, which enter the occultation integral only
/// through expectations and therefore need no shape beyond what a few nodes carry.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Distribution {
    nodes: Vec<(f64, f64)>,
}

impl Distribution {
    pub fn delta(value: f64) -> Self {
        Self { nodes: vec![(value, 1.0)] }
    }

    /// Midpoint nodes across `[lo, hi]`.
    pub fn uniform(lo: f64, hi: f64, nodes: usize) -> Self {
        assert!(nodes > 0 && hi >= lo);
        let w = 1.0 / nodes as f64;
        let step = (hi - lo) / nodes as f64;
        Self {
            nodes: (0..nodes).map(|k| (lo + (k as f64 + 0.5) * step, w)).collect(),
        }
    }

    /// Normal, truncated at three sigma and renormalised.
    pub fn normal(mean: f64, sigma: f64, nodes: usize) -> Self {
        assert!(nodes > 0);
        if sigma <= 0.0 {
            return Self::delta(mean);
        }
        let (lo, hi) = (mean - 3.0 * sigma, mean + 3.0 * sigma);
        let step = (hi - lo) / nodes as f64;
        let mut raw: Vec<(f64, f64)> = (0..nodes)
            .map(|k| {
                let x = lo + (k as f64 + 0.5) * step;
                let z = (x - mean) / sigma;
                (x, (-0.5 * z * z).exp())
            })
            .collect();
        let total: f64 = raw.iter().map(|(_, w)| w).sum();
        for n in &mut raw {
            n.1 /= total;
        }
        Self { nodes: raw }
    }

    pub fn nodes(&self) -> &[(f64, f64)] {
        &self.nodes
    }

    pub fn mean(&self) -> f64 {
        self.nodes.iter().map(|(v, w)| v * w).sum()
    }

    /// `E[f(x)]` over the nodes.
    pub fn expectation(&self, f: impl Fn(f64) -> f64) -> f64 {
        self.nodes.iter().map(|(v, w)| w * f(*v)).sum()
    }

    /// Draw a node by weight. For generating representative instances, which the renderer
    /// does rather than reading members that do not exist.
    pub fn sample(&self, h: u64) -> f64 {
        let mut target = crate::rng::uniform(h);
        for &(v, w) in &self.nodes {
            target -= w;
            if target <= 0.0 {
                return v;
            }
        }
        self.nodes.last().map(|(v, _)| *v).unwrap_or(0.0)
    }
}

/// An inclination distribution, held as bins in `u = cos(i)`.
///
/// Cos-space, not angle-space, because the latitude density carries a `1/sqrt(sin^2 i -
/// sin^2 phi)` factor whose endpoint singularity defeats point quadrature. In `u` each bin
/// integrates to an `arcsin` difference exactly, which also makes the isotropic case come out
/// at `N/4pi` to the last bit rather than to a few parts in a thousand.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inclination {
    /// `(u_lo, u_hi, weight)`, ascending, non-overlapping, weights summing to 1.
    bins: Vec<(f64, f64, f64)>,
}

impl Inclination {
    /// Uniform in `cos(i)`: orbit poles uniform on the sphere.
    pub fn isotropic() -> Self {
        Self { bins: vec![(0.0, 1.0, 1.0)] }
    }

    /// Uniform in the angle `i` over `[lo, hi]` radians.
    pub fn uniform_angle(lo: f64, hi: f64, bins: usize) -> Self {
        assert!(bins > 0 && (0.0..=std::f64::consts::FRAC_PI_2).contains(&lo));
        assert!(hi >= lo && hi <= std::f64::consts::FRAC_PI_2);
        let step = (hi - lo) / bins as f64;
        let w = 1.0 / bins as f64;
        let mut out: Vec<(f64, f64, f64)> = (0..bins)
            .map(|k| {
                let (a, b) = (lo + k as f64 * step, lo + (k + 1) as f64 * step);
                (b.cos(), a.cos(), w)
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { bins: out }
    }

    /// A band of half-width `spread` about `center`, in radians.
    pub fn band(center: f64, spread: f64, bins: usize) -> Self {
        let lo = (center - spread).max(0.0);
        let hi = (center + spread).min(std::f64::consts::FRAC_PI_2);
        Self::uniform_angle(lo, hi, bins)
    }

    pub fn bins(&self) -> &[(f64, f64, f64)] {
        &self.bins
    }

    /// Sky density per element, per steradian, at latitude `phi` from the population's pole.
    ///
    /// `Sigma(phi) = (1/2pi^2) * integral p(i) / sqrt(sin^2 i - sin^2 phi) di`, evaluated bin
    /// by bin in `u = cos i` where the integral is `arcsin(u / cos phi)`.
    pub fn sky_density(&self, phi: f64) -> f64 {
        let cos_phi = phi.cos().abs();
        if cos_phi <= 0.0 {
            return 0.0;
        }
        let mut total = 0.0;
        for &(u_lo, u_hi, w) in &self.bins {
            let width = u_hi - u_lo;
            if width <= 0.0 {
                continue;
            }
            // Only u < cos(phi) contributes: those are the inclinations that reach this
            // latitude at all.
            let hi = u_hi.min(cos_phi);
            if hi <= u_lo {
                continue;
            }
            let f = |u: f64| (u / cos_phi).clamp(-1.0, 1.0).asin();
            total += w / width * (f(hi) - f(u_lo));
        }
        total / (2.0 * std::f64::consts::PI * std::f64::consts::PI)
    }

    /// Draw an inclination, in radians.
    pub fn sample(&self, h: u64) -> f64 {
        let mut target = crate::rng::uniform(h);
        for &(u_lo, u_hi, w) in &self.bins {
            target -= w;
            if target <= 0.0 {
                let u = crate::rng::uniform_in(crate::rng::mix(h), u_lo, u_hi);
                return u.clamp(-1.0, 1.0).acos();
            }
        }
        self.bins.last().map(|&(u, _, _)| u.clamp(-1.0, 1.0).acos()).unwrap_or(0.0)
    }

    /// The largest inclination with any weight, in radians. Beyond it the population casts
    /// nothing.
    pub fn max_inclination(&self) -> f64 {
        self.bins
            .iter()
            .filter(|(_, _, w)| *w > 0.0)
            .map(|(u_lo, _, _)| u_lo.clamp(-1.0, 1.0).acos())
            .fold(0.0, f64::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_PI_2, PI};

    #[test]
    fn distributions_are_normalised_and_have_the_right_mean() {
        for d in [
            Distribution::delta(3.0),
            Distribution::uniform(2.0, 4.0, 9),
            Distribution::normal(3.0, 0.5, 41),
        ] {
            let total: f64 = d.nodes().iter().map(|(_, w)| w).sum();
            assert!((total - 1.0).abs() < 1e-12);
            assert!((d.mean() - 3.0).abs() < 1e-9, "{:?}", d.mean());
        }
    }

    #[test]
    fn expectation_reproduces_the_inverse_square_identity() {
        // <1/r^2> over a Kepler orbit is 1/(a^2 sqrt(1-e^2)), which is why the radial part of
        // the occultation integral needs no quadrature at all.
        let a = Distribution::delta(2.0);
        let e = Distribution::delta(0.6);
        let got = a.expectation(|a| 1.0 / (a * a)) * e.expectation(|e| 1.0 / (1.0 - e * e).sqrt());
        assert!((got - 1.0 / (4.0 * 0.8)).abs() < 1e-12, "{got}");
    }

    #[test]
    fn isotropic_inclinations_give_a_uniform_sky() {
        let iso = Inclination::isotropic();
        let want = 1.0 / (4.0 * PI);
        for phi in [0.0, 0.3, 0.8, 1.2, 1.5] {
            let got = iso.sky_density(phi);
            assert!((got - want).abs() < 1e-15, "phi={phi}: {got} vs {want}");
        }
    }

    #[test]
    fn a_band_casts_nothing_outside_its_inclination_limit() {
        let band = Inclination::uniform_angle(0.0, 0.3, 16);
        assert!(band.sky_density(0.1) > 0.0);
        assert_eq!(band.sky_density(0.4), 0.0);
        assert!((band.max_inclination() - 0.3).abs() < 1e-12);
    }

    #[test]
    fn confining_inclinations_concentrates_the_sky_density() {
        let iso = Inclination::isotropic().sky_density(0.0);
        let band = Inclination::uniform_angle(0.0, 0.1, 32).sky_density(0.0);
        assert!(band > iso * 5.0, "a 0.1 rad band should be much denser in-plane");
    }

    #[test]
    fn total_sky_density_integrates_to_the_element_count() {
        // Integrating Sigma over the sphere must return 1 element per element, whatever the
        // inclination distribution does.
        for inc in [
            Inclination::isotropic(),
            Inclination::uniform_angle(0.0, 0.4, 24),
            Inclination::band(0.5, 0.2, 24),
        ] {
            let n = 20_000;
            let mut total = 0.0;
            for k in 0..n {
                // Uniform in sin(phi) so each sample carries equal solid angle.
                let s = -1.0 + 2.0 * (k as f64 + 0.5) / n as f64;
                total += inc.sky_density(s.asin());
            }
            total *= 4.0 * PI / n as f64;
            assert!((total - 1.0).abs() < 0.02, "{total} for {inc:?}");
        }
        assert!(Inclination::uniform_angle(0.0, FRAC_PI_2, 4).max_inclination() > 1.5);
    }
}
