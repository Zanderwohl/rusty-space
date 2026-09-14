//! Perceptually uniform colour maps.
//!
//! Uniform matters here beyond taste: a rainbow map invents banding that is not in the data,
//! and this is a game about inferring structure from noisy measurements.

use crate::primitives::Rgba;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMap {
    Viridis,
    Magma,
    /// Blue through white to red, centred on the middle of the range. For residuals.
    Diverging,
}

// Sampled colour values, not approximations of anything; clippy reads 0.318 as 1/pi.
#[allow(clippy::approx_constant)]
const VIRIDIS: [[f32; 3]; 11] = [
    [0.267, 0.005, 0.329], [0.283, 0.141, 0.458], [0.254, 0.265, 0.530],
    [0.207, 0.372, 0.553], [0.164, 0.471, 0.558], [0.128, 0.567, 0.551],
    [0.135, 0.659, 0.518], [0.267, 0.749, 0.441], [0.478, 0.821, 0.318],
    [0.741, 0.873, 0.150], [0.993, 0.906, 0.144],
];

#[allow(clippy::approx_constant)]
const MAGMA: [[f32; 3]; 11] = [
    [0.001, 0.000, 0.014], [0.071, 0.048, 0.184], [0.185, 0.068, 0.353],
    [0.316, 0.072, 0.485], [0.451, 0.122, 0.506], [0.584, 0.177, 0.491],
    [0.720, 0.230, 0.443], [0.855, 0.306, 0.365], [0.951, 0.452, 0.290],
    [0.988, 0.653, 0.354], [0.987, 0.991, 0.750],
];

#[allow(clippy::approx_constant)]
const DIVERGING: [[f32; 3]; 5] = [
    [0.020, 0.188, 0.380], [0.404, 0.663, 0.812], [0.969, 0.969, 0.969],
    [0.839, 0.376, 0.302], [0.404, 0.000, 0.121],
];

impl ColorMap {
    fn stops(&self) -> &'static [[f32; 3]] {
        match self {
            Self::Viridis => &VIRIDIS,
            Self::Magma => &MAGMA,
            Self::Diverging => &DIVERGING,
        }
    }

    /// Sample at `t`, clamped to `[0, 1]`.
    pub fn sample(&self, t: f64) -> Rgba {
        let stops = self.stops();
        let t = t.clamp(0.0, 1.0) as f32 * (stops.len() - 1) as f32;
        let i = (t.floor() as usize).min(stops.len() - 2);
        let f = t - i as f32;
        let (a, b) = (stops[i], stops[i + 1]);
        Rgba::opaque(
            a[0] + (b[0] - a[0]) * f,
            a[1] + (b[1] - a[1]) * f,
            a[2] + (b[2] - a[2]) * f,
        )
    }

    /// Sample a value within a range.
    pub fn sample_in(&self, value: f64, range: (f64, f64)) -> Rgba {
        let (lo, hi) = range;
        if !crate::decimate::spans(lo, hi) {
            return self.sample(0.0);
        }
        self.sample((value - lo) / (hi - lo))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_map_is_continuous_and_in_gamut() {
        for map in [ColorMap::Viridis, ColorMap::Magma, ColorMap::Diverging] {
            let mut prev = map.sample(0.0);
            for k in 1..=1000 {
                let c = map.sample(k as f64 / 1000.0);
                for v in [c.0, c.1, c.2, c.3] {
                    assert!((0.0..=1.0).contains(&v), "{map:?} left the gamut: {c:?}");
                }
                let jump = (c.0 - prev.0).abs() + (c.1 - prev.1).abs() + (c.2 - prev.2).abs();
                assert!(jump < 0.05, "{map:?} has a discontinuity at {k}");
                prev = c;
            }
        }
    }

    #[test]
    fn viridis_runs_dark_blue_to_bright_yellow() {
        let lo = ColorMap::Viridis.sample(0.0);
        let hi = ColorMap::Viridis.sample(1.0);
        assert!(lo.2 > lo.1 && lo.0 < 0.4, "the low end is dark and blue");
        assert!(hi.0 > 0.9 && hi.1 > 0.8 && hi.2 < 0.3, "the high end is yellow");
    }

    #[test]
    fn the_diverging_map_is_neutral_in_the_middle() {
        let mid = ColorMap::Diverging.sample(0.5);
        assert!((mid.0 - mid.1).abs() < 0.05 && (mid.1 - mid.2).abs() < 0.05, "{mid:?}");
        assert!(mid.0 > 0.8, "the centre should be light");
        assert!(ColorMap::Diverging.sample(0.0).2 > ColorMap::Diverging.sample(0.0).0);
        assert!(ColorMap::Diverging.sample(1.0).0 > ColorMap::Diverging.sample(1.0).2);
    }

    #[test]
    fn out_of_range_clamps_rather_than_wrapping() {
        assert_eq!(ColorMap::Viridis.sample(-5.0), ColorMap::Viridis.sample(0.0));
        assert_eq!(ColorMap::Viridis.sample(5.0), ColorMap::Viridis.sample(1.0));
        assert_eq!(ColorMap::Magma.sample_in(1.0, (2.0, 2.0)), ColorMap::Magma.sample(0.0));
    }
}
