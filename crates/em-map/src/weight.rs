//! What mass decides: which names a map has room for, and how big a mark is drawn.
//!
//! Both questions are the same comparison — this thing against the heaviest thing it is drawn
//! beside — so they are answered from one constant rather than two that would drift apart.

/// How light a thing may be, as a fraction of the heaviest thing beside it, and still be named
/// and still be drawn whole.
///
/// Set from the case that has to work: **Earth beside the Sun**, which is three parts in a
/// million. Well under it, because what it is really aimed at is the gap between the smallest
/// planet and the largest asteroid — Mercury is 1.7e-7 of the Sun and Ceres 4.7e-10, a factor
/// of three hundred wide — and a floor in the middle of that keeps all eight planets while
/// dropping every numbered rock.
pub const FLOOR: f64 = 1.0e-8;

/// The smallest a mark is drawn, as a fraction of the full size: two decades under the floor
/// and no further. The marks are already a few pixels across and a hundredth of one is nothing
/// at all — and a host has its own pixel floor under this, because a shape needs pixels to be
/// a shape in.
pub const MIN_SCALE: f32 = 0.25;

/// How big a thing's mark is, as a fraction of the full size.
///
/// **Two per decade: ten times the mass is twice the radius.** Eight orders of magnitude of
/// mass cannot be eight orders of pixels, and a logarithm is the only honest way to hold a
/// range like that in one picture.
///
/// One at the floor and above, so anything worth naming is drawn whole, and shrinking below
/// it — a rock still shows, it just shows as a rock. A weight nobody stated is drawn whole
/// rather than shrunk by a comparison it was never in, and so is a ship's infinite one.
pub fn scale(weight: f64, floor: f64) -> f32 {
    if !weight.is_finite() || weight <= 0.0 || !(floor > 0.0) {
        return 1.0;
    }
    let decades = (weight / floor).log10();
    (2.0f64.powf(decades) as f32).clamp(MIN_SCALE, 1.0)
}

/// The heaviest stated weight, which is what [`FLOOR`] is a fraction of.
///
/// Infinite weights set no bar. A ship carries one, and a bar of infinity is an empty map.
pub fn heaviest(weights: impl Iterator<Item = f64>) -> f64 {
    weights.filter(|w| w.is_finite()).fold(0.0f64, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUN: f64 = 1.988_41e30;
    const EARTH: f64 = 5.972e24;
    const CERES: f64 = 9.39e20;

    /// **Ten times the mass is twice the radius**, which is the whole of the relationship.
    #[test]
    fn ten_times_the_mass_is_twice_the_radius() {
        let floor = 1.0e22;
        for decades in 1..=2 {
            let heavier = scale(floor / 10.0f64.powi(decades - 1), floor);
            let lighter = scale(floor / 10.0f64.powi(decades), floor);
            assert!(
                (heavier / lighter - 2.0).abs() < 1.0e-5,
                "{decades} decades down: {heavier} against {lighter}",
            );
        }
        assert!((scale(floor / 10.0, floor) - 0.5).abs() < 1.0e-6);
        assert!((scale(floor / 100.0, floor) - 0.25).abs() < 1.0e-6);
    }

    /// Anything worth naming is drawn whole. Above the floor the mark stops growing, or a star
    /// beside a planet is a hundred times its size and off the edge of the map.
    #[test]
    fn anything_worth_naming_is_drawn_whole() {
        let floor = SUN * FLOOR;
        assert_eq!(scale(SUN, floor), 1.0, "the heaviest thing there is");
        assert_eq!(scale(EARTH, floor), 1.0, "and the case the floor is set from");
        assert_eq!(scale(floor, floor), 1.0, "and the floor itself");
        assert!(scale(CERES, floor) < 1.0, "but a rock is drawn as one");
        assert!(scale(CERES, floor) > MIN_SCALE, "and is still more than the least there is");
    }

    /// It is bounded below, because the marks are a few pixels across to begin with.
    #[test]
    fn it_is_bounded_below() {
        let floor = SUN * FLOOR;
        // A pebble, nine orders under the floor.
        assert_eq!(scale(floor * 1.0e-9, floor), MIN_SCALE);
        assert_eq!(scale(f64::MIN_POSITIVE, floor), MIN_SCALE);
    }

    /// A weight nobody stated is not shrunk by a comparison it is not in, and a ship's
    /// infinite one is not shrunk by anything.
    #[test]
    fn an_unstated_weight_is_drawn_whole() {
        let floor = SUN * FLOOR;
        for weight in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(scale(weight, floor), 1.0, "{weight} should be drawn whole");
        }
        // And a map where nothing stated a mass draws everything whole.
        assert_eq!(scale(EARTH, 0.0), 1.0);
    }

    /// The bar is the heaviest thing stated, and a ship states nothing it could be.
    #[test]
    fn a_ship_sets_no_bar() {
        assert_eq!(heaviest([f64::INFINITY, EARTH, CERES].into_iter()), EARTH);
        assert_eq!(heaviest([f64::INFINITY].into_iter()), 0.0);
        assert_eq!(heaviest([].into_iter()), 0.0);
    }
}
