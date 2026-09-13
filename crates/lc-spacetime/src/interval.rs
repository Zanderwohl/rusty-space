//! The Minkowski interval, and what may be built on it.

use crate::coord::Coord;

/// How two events are separated. Frame-independent **because nothing in this design moves
/// faster than light**; state it that way rather than as an unconditional fact, because the
/// two differ the moment anything does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Separation {
    /// `s^2 > 0`. Causally connected, and the order of the pair is the same in every frame.
    Timelike,
    /// `s^2 == 0`. Connected by light exactly.
    Lightlike,
    /// `s^2 < 0`. No causal contact. **Their order is frame-dependent and nothing may depend
    /// on it.**
    Spacelike,
}

/// The invariant interval squared, `dt^2 - dr^2`, in squared microseconds.
///
/// Exact. `c = 1` in these units, so there is no constant in the expression. `i128` is used
/// rather than `i64` because the squares overflow it; the `2^60` construction bound on
/// [`Coord`] is what keeps `i128` sufficient.
#[inline]
pub fn interval2(a: Coord, b: Coord) -> i128 {
    let dt = (b.t - a.t).get() as i128;
    dt * dt - a.spatial_distance2(b)
}

/// Classify a pair.
#[inline]
pub fn classify(a: Coord, b: Coord) -> Separation {
    match interval2(a, b).signum() {
        1 => Separation::Timelike,
        0 => Separation::Lightlike,
        _ => Separation::Spacelike,
    }
}

/// True when `a` could have influenced `b`: `b` is at or after `a` in coordinate time, and
/// the two are not spacelike separated.
///
/// This is the only ordering a rule may depend on. It is reflexive, antisymmetric, and
/// transitive over the events of any sub-luminal world, and it is frame-independent, so a
/// rule written against it means the same thing to every observer.
#[inline]
pub fn precedes(a: Coord, b: Coord) -> bool {
    b.t >= a.t && interval2(a, b) >= 0
}

// There is deliberately no `simultaneous` function.
//
// Equal `t` is `a.t == b.t` and needs no help. Dressing it up as a physics predicate would
// invite code that treats a frame convention as a fact: two spacelike-separated events the
// server calls simultaneous are not simultaneous in a moving ship's frame, and their order
// can be reversed by a boost. Simultaneity of spacelike pairs is not defined.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coord::{COORD_BOUND, Coord};
    use crate::units::Micros;

    fn c(t: i64, x: i64, y: i64, z: i64) -> Coord {
        Coord::new(Micros::new(t), x, y, z).unwrap()
    }

    #[test]
    fn the_classification_table() {
        let o = Coord::ORIGIN;
        // 10 us later, 5 light-us away: light has time to spare.
        assert_eq!(classify(o, c(10, 5, 0, 0)), Separation::Timelike);
        // exactly on the cone, on each axis and on a diagonal.
        assert_eq!(classify(o, c(10, 10, 0, 0)), Separation::Lightlike);
        assert_eq!(classify(o, c(5, 3, 4, 0)), Separation::Lightlike);
        assert_eq!(classify(o, c(13, 3, 4, 12)), Separation::Lightlike);
        // 10 us later, 11 light-us away: light cannot make it.
        assert_eq!(classify(o, c(10, 11, 0, 0)), Separation::Spacelike);
        // an event with itself is lightlike, being zero separation in both.
        assert_eq!(classify(o, o), Separation::Lightlike);
    }

    #[test]
    fn classification_is_symmetric_but_precedence_is_not() {
        let a = c(0, 0, 0, 0);
        let b = c(10, 5, 0, 0);
        assert_eq!(classify(a, b), classify(b, a));
        assert!(precedes(a, b));
        assert!(!precedes(b, a));
    }

    #[test]
    fn spacelike_pairs_never_precede_in_either_direction() {
        let a = c(0, 0, 0, 0);
        let b = c(10, 11, 0, 0);
        assert!(!precedes(a, b));
        assert!(!precedes(b, a));
    }

    #[test]
    fn precedence_is_reflexive_and_transitive() {
        let a = c(0, 0, 0, 0);
        let b = c(100, 10, 0, 0);
        let d = c(250, 10, 50, 0);
        assert!(precedes(a, a));
        assert!(precedes(a, b) && precedes(b, d));
        assert!(precedes(a, d), "causal precedence must be transitive");
    }

    #[test]
    fn interval_arithmetic_does_not_overflow_at_the_bound() {
        // The extreme case the 2^60 bound exists for: opposite corners of the legal volume,
        // which makes every component difference 2^61.
        let lo = COORD_BOUND - 1;
        let a = c(-lo, -lo, -lo, -lo);
        let b = c(lo, lo, lo, lo);
        let s2 = interval2(a, b);
        // dt^2 - 3*dx^2 with dt == dx, so exactly -2*dx^2, and it must be representable.
        let dx = 2i128 * lo as i128;
        assert_eq!(s2, -2 * dx * dx);
        assert!(s2 > i128::MIN / 2, "must not be near the i128 floor");
        assert_eq!(classify(a, b), Separation::Spacelike);
    }

    #[test]
    fn light_travel_time_equals_distance_in_these_units() {
        // A point 1 light-second away is 1e6 light-us away and its light takes 1e6 us.
        let src = Coord::ORIGIN;
        let dst = c(1_000_000, 1_000_000, 0, 0);
        assert_eq!(classify(src, dst), Separation::Lightlike);
        assert_eq!(src.spatial_distance(dst), 1_000_000.0);
    }
}
