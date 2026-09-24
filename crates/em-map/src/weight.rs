//! What mass decides: which names a map has room for, and how big a mark is drawn.

/// How light a thing may be, against the heaviest thing beside it, and still be named.
///
/// Set from Earth beside the Sun, three parts in a million, and well under it: the gap it aims
/// at is between the smallest planet and the largest asteroid. Mercury is 1.7e-7 of the Sun
/// and Ceres 4.7e-10, so a floor in the middle keeps the planets and drops the rocks.
pub const FLOOR: f64 = 1.0e-8;

/// How many times the largest mark is the smallest, however far apart the weights beside each
/// other are. A star and a moonlet are thirty million times apart in mass and cannot be drawn
/// thirty million, or even three hundred, times apart in size.
pub const SPAN: f32 = 10.0;

/// The smallest a mark is drawn, as a fraction of full size. A host applies its own pixel floor
/// under this.
pub const MIN_SCALE: f32 = 1.0 / SPAN;

/// How big a thing's mark is, as a fraction of full size: the heaviest beside it is full size.
///
/// The cube root of the mass -- the size a body of the same density would be -- until that would
/// put the lightest more than [`SPAN`] under the heaviest, and then the same log scale squeezed
/// so it lands exactly there. Squeezed only when it must be: two stars a third apart in mass are
/// a tenth apart in size, not the whole span.
///
/// Unstated and infinite weights are drawn whole: neither is in the comparison.
pub fn scale(weight: f64, lightest: f64, heaviest: f64) -> f32 {
    if !(weight.is_finite() && weight > 0.0 && heaviest > 0.0 && lightest > 0.0) {
        return 1.0;
    }
    let decades = (heaviest / lightest).log10().max(0.0);
    let slope = (f64::from(SPAN).log10() / decades).min(1.0 / 3.0);
    ((weight / heaviest).powf(slope) as f32).clamp(MIN_SCALE, 1.0)
}

/// The lightest stated weight, which the smallest mark is drawn for.
pub fn lightest(weights: impl Iterator<Item = f64>) -> f64 {
    weights.filter(|w| w.is_finite() && *w > 0.0).fold(f64::INFINITY, f64::min)
}

/// The heaviest stated weight, which [`FLOOR`] is a fraction of. Infinite weights set no bar:
/// a ship carries one, and a bar of infinity empties the map.
pub fn heaviest(weights: impl Iterator<Item = f64>) -> f64 {
    weights.filter(|w| w.is_finite()).fold(0.0f64, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUN: f64 = 1.988_41e30;
    const JUPITER: f64 = 1.898e27;
    const EARTH: f64 = 5.972e24;
    const MOON: f64 = 7.342e22;

    /// The heaviest is drawn whole and the lightest a [`SPAN`] under it, however many decades
    /// apart they are.
    #[test]
    fn a_star_and_a_moon_are_the_span_apart() {
        assert_eq!(scale(SUN, MOON, SUN), 1.0);
        assert!((scale(MOON, MOON, SUN) - MIN_SCALE).abs() < 1.0e-6);
        let (earth, jupiter) = (scale(EARTH, MOON, SUN), scale(JUPITER, MOON, SUN));
        assert!(MIN_SCALE < earth && earth < jupiter && jupiter < 1.0, "{earth} {jupiter}");
    }

    /// Log in mass: each decade takes the same share of the span.
    #[test]
    fn each_decade_takes_the_same_share() {
        let (lightest, heaviest) = (1.0e20, 1.0e30);
        let ratio = |a: f64, b: f64| scale(a, lightest, heaviest) / scale(b, lightest, heaviest);
        assert!((ratio(1.0e25, 1.0e24) - ratio(1.0e22, 1.0e21)).abs() < 1.0e-5);
    }

    /// Close weights are drawn close, not stretched across the span: size goes as the cube root
    /// of mass until the span would be exceeded.
    #[test]
    fn close_weights_are_drawn_close() {
        let (lighter, heavier) = (0.75 * SUN, SUN);
        let drawn = scale(lighter, lighter, heavier);
        assert!((drawn - 0.75f32.cbrt()).abs() < 1.0e-5, "{drawn}");
        assert_eq!(scale(EARTH, EARTH, EARTH), 1.0, "one weight is drawn whole");
    }

    /// Unstated and infinite weights are not shrunk.
    #[test]
    fn an_unstated_weight_is_drawn_whole() {
        for weight in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(scale(weight, MOON, SUN), 1.0, "{weight} should be drawn whole");
        }
        assert_eq!(scale(EARTH, f64::INFINITY, 0.0), 1.0, "and with nothing stated, everything is");
    }

    /// Neither bound is set by a ship's infinite weight or an unstated one.
    #[test]
    fn a_ship_sets_no_bound() {
        let weights = [f64::INFINITY, 0.0, EARTH, MOON];
        assert_eq!(heaviest(weights.into_iter()), EARTH);
        assert_eq!(lightest(weights.into_iter()), MOON);
        assert_eq!(heaviest([f64::INFINITY].into_iter()), 0.0);
    }
}
