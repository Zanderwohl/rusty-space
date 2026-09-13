//! Mapping data to pixels, and choosing tick positions.

/// How a data axis maps to a pixel axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scale {
    Linear,
    Log10,
    /// Logarithmic outside `linear_threshold`, linear within it, so a series that crosses
    /// zero and spans decades can be drawn at all. Deficits and residuals do both.
    SymLog { linear_threshold: f64 },
}

impl Scale {
    /// Data value to a position on `[0, 1]` across `range`.
    pub fn normalise(&self, value: f64, range: (f64, f64)) -> f64 {
        let (lo, hi) = range;
        let f = self.forward(value);
        let (flo, fhi) = (self.forward(lo), self.forward(hi));
        if (fhi - flo).abs() < f64::MIN_POSITIVE { 0.0 } else { (f - flo) / (fhi - flo) }
    }

    /// Inverse of [`Scale::normalise`], for reading a value off a chart.
    pub fn denormalise(&self, t: f64, range: (f64, f64)) -> f64 {
        let (lo, hi) = range;
        let (flo, fhi) = (self.forward(lo), self.forward(hi));
        self.inverse(flo + t * (fhi - flo))
    }

    fn forward(&self, v: f64) -> f64 {
        match *self {
            Self::Linear => v,
            Self::Log10 => {
                if v > 0.0 { v.log10() } else { f64::NEG_INFINITY }
            }
            Self::SymLog { linear_threshold } => {
                let t = linear_threshold.abs().max(f64::MIN_POSITIVE);
                if v.abs() <= t { v / t } else { v.signum() * (1.0 + (v.abs() / t).log10()) }
            }
        }
    }

    fn inverse(&self, f: f64) -> f64 {
        match *self {
            Self::Linear => f,
            Self::Log10 => 10f64.powf(f),
            Self::SymLog { linear_threshold } => {
                let t = linear_threshold.abs().max(f64::MIN_POSITIVE);
                if f.abs() <= 1.0 { f * t } else { f.signum() * t * 10f64.powf(f.abs() - 1.0) }
            }
        }
    }

    /// A range this scale can actually represent.
    ///
    /// A log axis cannot show zero or negative values, and silently producing an infinite
    /// transform draws an empty chart rather than reporting anything.
    pub fn valid_range(&self, range: (f64, f64)) -> (f64, f64) {
        let (lo, hi) = range;
        match self {
            Self::Log10 => {
                let hi = if hi > 0.0 { hi } else { 1.0 };
                let lo = if lo > 0.0 { lo.min(hi) } else { hi * 1e-9 };
                (lo, hi)
            }
            _ => (lo, hi),
        }
    }

    /// Widen a range by a fraction of its span, in the scale's own space.
    ///
    /// Additive padding is wrong for a log axis: five percent of a span that reaches 1.0
    /// takes a lower bound of 1e-6 negative, and everything after that is an infinity. Padding
    /// in the transformed space is multiplicative where it should be and additive where it
    /// should be, with no special cases at the call site.
    pub fn pad(&self, range: (f64, f64), fraction: f64) -> (f64, f64) {
        let (lo, hi) = self.valid_range(range);
        let (flo, fhi) = (self.forward(lo), self.forward(hi));
        if !flo.is_finite() || !fhi.is_finite() {
            return (lo, hi);
        }
        let mut span = fhi - flo;
        if span <= 0.0 {
            span = flo.abs().max(1.0) * 0.1;
        }
        (self.inverse(flo - span * fraction), self.inverse(fhi + span * fraction))
    }

    /// Tick positions across `range`, at most `target` of them.
    pub fn ticks(&self, range: (f64, f64), target: usize) -> Vec<f64> {
        let (lo, hi) = self.valid_range(range);
        if !(hi > lo) || target == 0 {
            return Vec::new();
        }
        match self {
            Self::Log10 => {
                let (a, b) = (lo.log10().floor(), hi.log10().ceil());
                let step = (((b - a) / target as f64).ceil() as i32).max(1);
                let mut out = Vec::new();
                let mut e = a as i32;
                while (e as f64) <= b {
                    let v = 10f64.powi(e);
                    if v >= lo && v <= hi {
                        out.push(v);
                    }
                    e += step;
                }
                out
            }
            _ => nice_ticks(lo, hi, target),
        }
    }
}

/// Ticks at 1, 2 or 5 times a power of ten — the spacings that read as round numbers.
fn nice_ticks(lo: f64, hi: f64, target: usize) -> Vec<f64> {
    let raw = (hi - lo) / target as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalised = raw / magnitude;
    let step = magnitude
        * if normalised <= 1.0 {
            1.0
        } else if normalised <= 2.0 {
            2.0
        } else if normalised <= 5.0 {
            5.0
        } else {
            10.0
        };
    let first = (lo / step).ceil() * step;
    let mut out = Vec::new();
    let mut v = first;
    // Guard against a step that rounding has made useless.
    while v <= hi + step * 1e-9 && out.len() <= target * 4 {
        out.push(v);
        v += step;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_maps_the_ends_to_zero_and_one() {
        let s = Scale::Linear;
        assert_eq!(s.normalise(0.0, (0.0, 10.0)), 0.0);
        assert_eq!(s.normalise(10.0, (0.0, 10.0)), 1.0);
        assert_eq!(s.normalise(2.5, (0.0, 10.0)), 0.25);
    }

    #[test]
    fn every_scale_round_trips() {
        let cases = [
            (Scale::Linear, (-5.0, 12.0), 3.25),
            (Scale::Log10, (1e-6, 1e3), 4.2e-2),
            (Scale::SymLog { linear_threshold: 1e-9 }, (-1e-3, 1e-3), -3.7e-7),
            (Scale::SymLog { linear_threshold: 1e-9 }, (-1e-3, 1e-3), 1e-10),
        ];
        for (scale, range, value) in cases {
            let t = scale.normalise(value, range);
            let back = scale.denormalise(t, range);
            assert!(
                (back - value).abs() <= value.abs() * 1e-9 + 1e-18,
                "{scale:?}: {value} -> {t} -> {back}"
            );
        }
    }

    #[test]
    fn symlog_crosses_zero_where_log_cannot() {
        let s = Scale::SymLog { linear_threshold: 1e-9 };
        let range = (-1e-3, 1e-3);
        assert!((s.normalise(0.0, range) - 0.5).abs() < 1e-12, "zero sits in the middle");
        assert!(s.normalise(-1e-6, range) < 0.5);
        assert!(s.normalise(1e-6, range) > 0.5);
        // And it still spans decades: a value a thousand times smaller is not at the middle.
        assert!(s.normalise(1e-9, range) > 0.5 && s.normalise(1e-9, range) < s.normalise(1e-6, range));
        // Log10 cannot represent the negative half at all.
        assert!(Scale::Log10.normalise(-1.0, (1.0, 10.0)).is_infinite());
    }

    #[test]
    fn linear_ticks_are_round_numbers_inside_the_range() {
        let t = Scale::Linear.ticks((0.0, 10.0), 5);
        assert_eq!(t, vec![0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
        for v in Scale::Linear.ticks((3.7, 18.2), 4) {
            assert!(v >= 3.7 && v <= 18.2 + 1e-9);
        }
    }

    #[test]
    fn log_ticks_are_powers_of_ten() {
        let t = Scale::Log10.ticks((1.0, 1000.0), 5);
        assert_eq!(t, vec![1.0, 10.0, 100.0, 1000.0]);
        // A wide range thins them out rather than emitting hundreds.
        assert!(Scale::Log10.ticks((1e-20, 1e20), 5).len() <= 12);
    }

    #[test]
    fn padding_a_log_axis_stays_positive() {
        // The bug this exists for: additive padding on (1e-6, 1.0) gives a negative lower
        // bound, log10 of that is -inf, and the chart renders empty.
        let (lo, hi) = Scale::Log10.pad((1e-6, 1.0), 0.06);
        assert!(lo > 0.0 && lo < 1e-6, "lower bound {lo} must widen downward and stay positive");
        assert!(hi > 1.0);
        // And it is multiplicative: equal padding in decades at both ends.
        let decades_below = (1e-6f64 / lo).log10();
        let decades_above = (hi / 1.0f64).log10();
        assert!((decades_below - decades_above).abs() < 1e-9);
    }

    #[test]
    fn padding_a_linear_axis_is_additive() {
        let (lo, hi) = Scale::Linear.pad((0.0, 10.0), 0.1);
        assert!((lo + 1.0).abs() < 1e-9 && (hi - 11.0).abs() < 1e-9);
        // A flat range still opens up rather than staying a point.
        let (a, b) = Scale::Linear.pad((5.0, 5.0), 0.1);
        assert!(b > a);
    }

    #[test]
    fn a_log_axis_refuses_an_impossible_range_rather_than_producing_infinities() {
        let (lo, hi) = Scale::Log10.valid_range((-3.0, 100.0));
        assert!(lo > 0.0 && hi == 100.0);
        assert!(!Scale::Log10.ticks((-3.0, 100.0), 5).is_empty(), "ticks must survive it");
        for v in Scale::Log10.ticks((-3.0, 100.0), 5) {
            assert!(v > 0.0 && v.is_finite());
        }
        assert!(Scale::Log10.normalise(1.0, Scale::Log10.valid_range((-3.0, 100.0))).is_finite());
    }

    #[test]
    fn degenerate_ranges_give_no_ticks() {
        assert!(Scale::Linear.ticks((1.0, 1.0), 5).is_empty());
        assert!(Scale::Linear.ticks((5.0, 1.0), 5).is_empty());
        assert!(Scale::Linear.ticks((0.0, 1.0), 0).is_empty());
    }
}
