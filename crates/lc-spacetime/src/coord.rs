//! Server-frame event coordinates on the integer light-microsecond grid.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::units::Micros;

/// One light-microsecond, in metres. Exact: `c` is defined as 299 792 458 m/s.
pub const LIGHT_MICROSECOND_M: f64 = 299.792458;

/// Every coordinate component satisfies `|c| < COORD_BOUND`: 36 534 light-years and years.
///
/// The bound is what keeps [`crate::interval::interval2`] inside `i128`. Differences are then
/// below `2^61`, their squares below `2^122`, and the sum of four below `2^124`.
pub const COORD_BOUND: i64 = 1 << 60;

/// A point in server-frame spacetime: microseconds, and light-microseconds.
///
/// Ordering is by `t` alone, so a `Coord` can key a time-sorted structure. It is deliberately
/// *not* a causal order — see [`crate::interval`].
#[derive(Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Coord {
    pub t: Micros,
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

/// A coordinate component outside the `2^60` bound that keeps interval arithmetic exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutOfBounds {
    pub axis: Axis,
    pub value: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    T,
    X,
    Y,
    Z,
}

impl std::fmt::Display for OutOfBounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "coordinate {:?} = {} is outside +/-2^60; the playable volume must fit inside it",
            self.axis, self.value
        )
    }
}
impl std::error::Error for OutOfBounds {}

impl Coord {
    pub const ORIGIN: Self = Self { t: Micros::ORIGIN, x: 0, y: 0, z: 0 };

    /// The only place [`COORD_BOUND`] is enforced, so everything downstream may assume it.
    pub fn new(t: Micros, x: i64, y: i64, z: i64) -> Result<Self, OutOfBounds> {
        check(Axis::T, t.get())?;
        check(Axis::X, x)?;
        check(Axis::Y, y)?;
        check(Axis::Z, z)?;
        Ok(Self { t, x, y, z })
    }

    /// For values a caller has already bounded; debug builds still assert.
    ///
    /// Past the bound a release build gives wrong interval signs rather than overflowing,
    /// which is worse than a panic. Use [`Coord::new`] for anything from outside.
    #[inline]
    pub fn new_unchecked(t: Micros, x: i64, y: i64, z: i64) -> Self {
        debug_assert!(in_bounds(t.get()) && in_bounds(x) && in_bounds(y) && in_bounds(z));
        Self { t, x, y, z }
    }

    /// Spatial part in light-microseconds, for the continuous solvers.
    ///
    /// Lossy past `2^53` light-microseconds (285 ly), where the low bits fall off the end of
    /// `f64`'s integer range. Harmless: the error stays proportional, reaching 2.4 km at
    /// 1100 ly, and nothing at that distance needs sub-300 m precision. Integer comparisons
    /// are unaffected.
    #[inline]
    pub fn position(self) -> DVec3 {
        DVec3::new(self.x as f64, self.y as f64, self.z as f64)
    }

    /// Coordinate time as `f64` microseconds, for the continuous solvers.
    #[inline]
    pub fn time_f64(self) -> f64 {
        self.t.get() as f64
    }

    /// Squared spatial separation, in squared light-microseconds. Exact.
    #[inline]
    pub fn spatial_distance2(self, other: Self) -> i128 {
        let dx = (other.x - self.x) as i128;
        let dy = (other.y - self.y) as i128;
        let dz = (other.z - self.z) as i128;
        dx * dx + dy * dy + dz * dz
    }

    /// Also the light travel time between the two points, in microseconds.
    #[inline]
    pub fn spatial_distance(self, other: Self) -> f64 {
        (self.spatial_distance2(other) as f64).sqrt()
    }
}

#[inline]
const fn in_bounds(v: i64) -> bool {
    v > -COORD_BOUND && v < COORD_BOUND
}

#[inline]
fn check(axis: Axis, v: i64) -> Result<(), OutOfBounds> {
    if in_bounds(v) { Ok(()) } else { Err(OutOfBounds { axis, value: v }) }
}

impl Ord for Coord {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.t.cmp(&other.t)
    }
}
impl PartialOrd for Coord {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Debug for Coord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({:?}, {}, {}, {})", self.t, self.x, self.y, self.z)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_light_microsecond_is_c_times_a_microsecond() {
        assert_eq!(LIGHT_MICROSECOND_M, 299_792_458.0 * 1e-6);
    }

    #[test]
    fn the_bound_is_rejected_and_just_inside_it_is_not() {
        assert!(Coord::new(Micros::ORIGIN, COORD_BOUND, 0, 0).is_err());
        assert!(Coord::new(Micros::ORIGIN, -COORD_BOUND, 0, 0).is_err());
        assert!(Coord::new(Micros::ORIGIN, COORD_BOUND - 1, 0, 0).is_ok());
        assert!(Coord::new(Micros::new(COORD_BOUND), 0, 0, 0).is_err());
    }

    #[test]
    fn out_of_bounds_names_the_axis() {
        let e = Coord::new(Micros::ORIGIN, 0, COORD_BOUND, 0).unwrap_err();
        assert_eq!(e.axis, Axis::Y);
        assert_eq!(e.value, COORD_BOUND);
    }

    #[test]
    fn the_bound_is_36_500_light_years() {
        let ly_m = 9.460_730_472_580_8e15;
        let ly = COORD_BOUND as f64 * LIGHT_MICROSECOND_M / ly_m;
        assert!((ly - 36_534.0).abs() < 1.0, "bound is {ly} ly");
    }

    #[test]
    fn coords_order_by_time_only() {
        let a = Coord::new(Micros::new(5), 1_000_000, 0, 0).unwrap();
        let b = Coord::new(Micros::new(7), 0, 0, 0).unwrap();
        assert!(a < b);
    }
}
