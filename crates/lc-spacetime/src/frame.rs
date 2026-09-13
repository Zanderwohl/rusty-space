//! The boundaries between coordinate systems: the global grid, a system's local frame, and
//! the time `em-sim` propagates against.
//!
//! **The global grid is the index; local arithmetic is the truth.** Causality within a system
//! is evaluated in local `f64` seconds and metres; causality between systems is evaluated on
//! the integer grid. A query that spans the boundary uses the grid to select candidates and
//! local arithmetic to refine, in that order.

use em_foundations::time::Instant;
use glam::DVec3;

use crate::coord::{Coord, LIGHT_MICROSECOND_M, OutOfBounds};
use crate::units::{MICROS_PER_SECOND, Micros, Span};

/// Metres to light-microseconds.
#[inline]
pub fn metres_to_light_micros(m: f64) -> f64 {
    m / LIGHT_MICROSECOND_M
}

/// Light-microseconds to metres.
#[inline]
pub fn light_micros_to_metres(lus: f64) -> f64 {
    lus * LIGHT_MICROSECOND_M
}

/// Where a system's local frame sits on the global grid.
///
/// `epoch` is the coordinate time that the system's own time base calls zero. It exists so
/// that the `f64` seconds handed to `em-sim` are always a small difference — see
/// [`SystemFrame::propagation_time`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemFrame {
    /// Global grid position of the system's barycentre.
    pub origin: Coord,
}

impl SystemFrame {
    pub fn new(origin: Coord) -> Self {
        Self { origin }
    }

    /// Local metres about the barycentre to a global grid coordinate.
    ///
    /// Rounding costs up to 150 m, which is 0.5 us of coordinate time and below the
    /// resolution of every gameplay rule. The local value remains authoritative; this is for
    /// indexing.
    pub fn to_global(&self, t: Micros, local_m: DVec3) -> Result<Coord, OutOfBounds> {
        let l = local_m / LIGHT_MICROSECOND_M;
        Coord::new(
            t,
            self.origin.x + l.x.round() as i64,
            self.origin.y + l.y.round() as i64,
            self.origin.z + l.z.round() as i64,
        )
    }

    /// A global grid coordinate to local metres about the barycentre.
    pub fn to_local(&self, global: Coord) -> DVec3 {
        DVec3::new(
            (global.x - self.origin.x) as f64,
            (global.y - self.origin.y) as f64,
            (global.z - self.origin.z) as f64,
        ) * LIGHT_MICROSECOND_M
    }

    /// The time to hand `em-sim`, as an **offset from this system's epoch** — never an
    /// absolute epoch.
    ///
    /// `em_foundations::Instant` is documented as seconds since J2000, and for a system built
    /// from real ephemerides that is literally what this returns. For a generated system,
    /// J2000 is simply the name of that system's zero, and the system's `BodyDef` element
    /// epochs must use the same base. Getting that wrong is the 28-days-out-of-position bug
    /// the repository already has a scar from, which is why this conversion exists in exactly
    /// one place and nothing else is permitted to construct an `Instant` from coordinate
    /// time.
    #[inline]
    pub fn propagation_time(&self, t: Micros) -> Instant {
        Instant::from_seconds_since_j2000((t - self.origin.t).as_seconds())
    }

    /// The inverse of [`SystemFrame::propagation_time`], for turning a propagation result
    /// back into coordinate time.
    #[inline]
    pub fn coordinate_time(&self, instant: Instant) -> Micros {
        let micros = (instant.to_j2000_seconds() * MICROS_PER_SECOND as f64).round() as i64;
        self.origin.t + Span::new(micros)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> SystemFrame {
        // 10 light-seconds out along +x, epoch 1e9 us after the world origin.
        SystemFrame::new(Coord::new(Micros::new(1_000_000_000), 10_000_000, 0, 0).unwrap())
    }

    #[test]
    fn metres_and_light_microseconds_round_trip() {
        let m = 1.495_978_707e11; // 1 AU, by definition
        let lus = metres_to_light_micros(m);
        // 1 AU is 499.005 light-seconds, so 4.99e8 light-microseconds.
        assert!((lus / 1e6 - 499.004_783_8).abs() < 1e-6, "1 AU is {} light-s", lus / 1e6);
        assert!((light_micros_to_metres(lus) - m).abs() < 1e-3);
    }

    #[test]
    fn local_to_global_round_trips_within_the_grid_resolution() {
        let f = frame();
        let local = DVec3::new(1.496e11, -2.5e10, 7.0e9);
        let g = f.to_global(Micros::new(1_000_000_500), local).unwrap();
        let back = f.to_local(g);
        // The grid quantises to 299.79 m, so half a cell is the worst case per axis.
        assert!((back - local).abs().max_element() <= LIGHT_MICROSECOND_M / 2.0 + 1e-6);
    }

    #[test]
    fn the_barycentre_is_the_local_origin() {
        let f = frame();
        assert_eq!(f.to_local(f.origin), DVec3::ZERO);
    }

    #[test]
    fn propagation_time_is_an_offset_from_the_system_epoch() {
        let f = frame();
        // Exactly at the epoch, em-sim sees zero, not 1e9 microseconds.
        assert_eq!(f.propagation_time(f.origin.t).to_j2000_seconds(), 0.0);
        // One second later.
        let t = f.origin.t + Span::from_seconds(1);
        assert_eq!(f.propagation_time(t).to_j2000_seconds(), 1.0);
    }

    #[test]
    fn propagation_time_round_trips() {
        let f = frame();
        for offset_s in [-86_400, -1, 0, 1, 3_600, 31_557_600] {
            let t = f.origin.t + Span::from_seconds(offset_s);
            assert_eq!(f.coordinate_time(f.propagation_time(t)), t);
        }
    }

    #[test]
    fn a_far_future_coordinate_still_converts_precisely() {
        // The whole reason time is stored as i64: at the 36 500-year horizon an absolute f64
        // second count has a 0.24 ms ulp, which is 73 km of light travel. Taking the
        // difference first keeps the value small and the conversion exact.
        let epoch = Micros::new(1_100_000_000_000_000_000); // ~34 800 years
        let f = SystemFrame::new(Coord::new(epoch, 0, 0, 0).unwrap());
        let t = epoch + Span::new(1); // one microsecond later
        assert_eq!(f.propagation_time(t).to_j2000_seconds(), 1e-6);
        assert_eq!(f.coordinate_time(f.propagation_time(t)), t);
    }
}
