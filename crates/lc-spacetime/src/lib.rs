//! Event coordinates, Minkowski intervals, worldlines and retarded time.
//!
//! No engine, no ECS, no rendering: the server links this crate.
//!
//! # Units
//!
//! All coordinates are in one privileged frame, the **server frame**, whose origin is fixed
//! at world creation. No other frame is stored, transmitted, or shown to a player.
//!
//! | Quantity | Unit |
//! |---|---|
//! | Coordinate time | microseconds, `i64`, from the world origin |
//! | Coordinate position | light-microseconds (299.792458 m), `i64` |
//! | Velocity | `beta`, dimensionless |
//! | Local position inside a system | meters, `f64`, about the barycenter |
//!
//! So **`c = 1`** and the light-cone test is integer arithmetic. Resolution is 299.79 m and
//! 1 us.
//!
//! [`frame::SystemFrame::propagation_time`] is the only place that relates coordinate time to
//! `em_foundations::Instant`.
//!
//! # Causality
//!
//! [`interval::precedes`] is frame-independent *because nothing here moves faster than
//! light*, and is the only ordering a rule may depend on. Simultaneity of spacelike pairs is
//! not defined.

#![forbid(unsafe_code)]

pub mod coord;
pub mod doppler;
pub mod frame;
pub mod interval;
pub mod proper_time;
pub mod units;
pub mod worldline;

pub use coord::{COORD_BOUND, Coord, LIGHT_MICROSECOND_M};
pub use interval::{Separation, classify, interval2, precedes};
pub use units::{Micros, Span};
pub use worldline::{Worldline, arrival_time, arrival_time_at, retarded_times, retarded_times_at};

#[cfg(test)]
mod standalone {
    //! Proves the crate is self-sufficient: no engine, no ECS.
    use glam::DVec3;

    use crate::{Coord, Micros, Separation, classify, worldline::Static};

    #[test]
    fn solves_a_light_cone_with_no_engine() {
        // A source one light-second away. Its light takes exactly 1e6 microseconds.
        let src = Coord::new(Micros::ORIGIN, 1_000_000, 0, 0).unwrap();
        let obs = Coord::new(Micros::new(1_000_000), 0, 0, 0).unwrap();
        assert_eq!(classify(src, obs), Separation::Lightlike);

        let w = Static::new(src.position());
        let roots = crate::retarded_times(obs, &w);
        assert_eq!(roots.len(), 1);
        assert!(roots[0].abs() < 1e-6, "emitted at the origin of time, got {}", roots[0]);
    }

    #[test]
    fn the_observer_learns_late_and_not_before() {
        // The premise, in miniature. A source 10 light-seconds away emits at t = 0.
        let w = Static::new(DVec3::new(10_000_000.0, 0.0, 0.0));
        let at = |t: i64| crate::retarded_times(Coord::new(Micros::new(t), 0, 0, 0).unwrap(), &w);

        // Before the light arrives there is no emission time that reaches the observer at all.
        assert!(at(5_000_000).iter().all(|&t_r| t_r < 0.0));
        // At arrival, the observer is seeing exactly t = 0.
        assert!(at(10_000_000)[0].abs() < 1e-6);
        // Later, the observer is seeing a later emission, always 10 s behind.
        assert!((at(15_000_000)[0] - 5_000_000.0).abs() < 1e-6);
    }
}
