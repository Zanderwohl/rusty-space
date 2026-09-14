//! What the server knows: things with worldlines, and the events they have made.

use glam::DVec3;
use lc_proto::{ClientId, ShipId};
use lc_spacetime::Worldline;
use lc_spacetime::worldline::{Inertial, Static};

/// Microseconds of coordinate time in one second.
pub const MICROS_PER_SECOND: i64 = 1_000_000;

/// An owned worldline.
///
/// An enum rather than a boxed trait object because the server stores these and the set is
/// closed: a ship is coasting or it is not. A burn is two events and a new `Inertial` between
/// them, which is what makes a trajectory storable as a worldline at all.
#[derive(Clone, Copy, Debug)]
pub enum Path {
    Still(Static),
    Coasting(Inertial),
}

impl Path {
    /// At rest at a point, light-microseconds from the world origin.
    pub fn still(at: DVec3) -> Self {
        Path::Still(Static::new(at))
    }

    /// Moving through `at` at `beta` at coordinate time `epoch`.
    pub fn coasting(at: DVec3, beta: DVec3, epoch_us: i64) -> Self {
        Path::Coasting(Inertial::new(at, beta, epoch_us as f64))
    }

    pub fn as_worldline(&self) -> &dyn Worldline {
        match self {
            Path::Still(line) => line,
            Path::Coasting(line) => line,
        }
    }
}

impl Worldline for Path {
    fn position_at(&self, t: f64) -> DVec3 {
        self.as_worldline().position_at(t)
    }
    fn velocity_at(&self, t: f64) -> DVec3 {
        self.as_worldline().velocity_at(t)
    }
    fn defined_over(&self) -> (f64, f64) {
        self.as_worldline().defined_over()
    }
    fn is_subluminal(&self) -> bool {
        self.as_worldline().is_subluminal()
    }
    fn bounding_ball(&self, t0: f64, t1: f64) -> (DVec3, f64) {
        self.as_worldline().bounding_ball(t0, t1)
    }
}

/// A ship: a worldline the server owns, and the instrument a client sees through.
#[derive(Clone, Debug)]
pub struct Ship {
    pub id: ShipId,
    pub owner: ClientId,
    pub path: Path,
    /// Below this, an arrival is not a detection. Arrival is the hard gate; this is the one
    /// that prunes far more.
    pub noise_floor: f32,
}

/// Something that happened, at a coordinate.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: i64,
    /// The ship it happened to.
    pub source: ShipId,
    /// Coordinate microseconds.
    pub t: i64,
    /// Where, light-microseconds. Taken from the source's worldline at `t` and stored, because
    /// the worldline will have changed by the time anyone is told.
    pub at: DVec3,
    pub kind: i16,
    /// What the source put out, for the inverse-square falloff. Zero for anything silent.
    pub power_w: f64,
    pub payload: String,
}

/// An event on its way to an observer, with the time it gets there.
///
/// Computed when the event is written, not when it is read. That is what makes the tick-time
/// check a range scan rather than a pass over everything that has ever happened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheduled {
    pub observer: ShipId,
    pub event: i64,
    pub arrive_t: i64,
    pub strength: f32,
}

/// Strength at the receiver: power over the square of the distance travelled, with the
/// distance floored so a coincident source is loud rather than infinite.
pub fn strength(power_w: f64, distance: f64) -> f32 {
    (power_w / distance.max(1.0).powi(2)) as f32
}

/// When an event's light reaches a ship, and how strong it is when it does.
///
/// `None` when it never does — the ship's worldline ends first, or the light already went past
/// before it began.
pub fn schedule(event: &Event, observer: &Ship) -> Option<Scheduled> {
    let arrive =
        lc_spacetime::arrival_time_at(event.t as f64, event.at, observer.path.as_worldline())?;
    // Rounded up. Rounding down would put an arrival a microsecond before its true time, and
    // the gate would then release it a microsecond early -- which is the one error this whole
    // system exists to prevent, however small.
    let arrive_t = arrive.ceil() as i64;
    let travelled = (observer.path.position_at(arrive) - event.at).length();
    Some(Scheduled {
        observer: observer.id,
        event: event.id,
        arrive_t,
        strength: strength(event.power_w, travelled),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(id: i64, at: DVec3) -> Ship {
        Ship { id: ShipId(id), owner: ClientId(1), path: Path::still(at), noise_floor: 0.0 }
    }

    fn pulse(t: i64, at: DVec3, power_w: f64) -> Event {
        Event { id: 1, source: ShipId(0), t, at, kind: 1, power_w, payload: "{}".into() }
    }

    /// A light-microsecond of distance is a microsecond of delay, by definition. This is the
    /// unit the whole protocol is in, so it is worth pinning.
    #[test]
    fn a_light_microsecond_of_distance_is_a_microsecond_of_delay() {
        let observer = ship(2, DVec3::new(1_000_000.0, 0.0, 0.0));
        let sent = pulse(5_000, DVec3::ZERO, 1.0);
        let scheduled = schedule(&sent, &observer).expect("it arrives");
        assert_eq!(scheduled.arrive_t, 1_005_000);
        assert_eq!(scheduled.observer, ShipId(2));
    }

    /// An event at the observer's own position arrives when it happens, and not before.
    #[test]
    fn an_event_at_the_observer_arrives_at_once() {
        let observer = ship(2, DVec3::ZERO);
        let scheduled = schedule(&pulse(7_777, DVec3::ZERO, 1.0), &observer).unwrap();
        assert_eq!(scheduled.arrive_t, 7_777);
    }

    /// Rounded up, never down. A rounding that put an arrival a microsecond early would make
    /// the gate release it a microsecond early, which is the one failure that matters.
    #[test]
    fn an_arrival_is_never_rounded_earlier_than_it_is() {
        let observer = ship(2, DVec3::new(1_000.5, 0.0, 0.0));
        let scheduled = schedule(&pulse(0, DVec3::ZERO, 1.0), &observer).unwrap();
        assert_eq!(scheduled.arrive_t, 1_001, "1000.5 became {}", scheduled.arrive_t);
        assert!(scheduled.arrive_t as f64 >= 1_000.5);
    }

    /// A moving observer meets the light earlier or later than a still one at the same place,
    /// and the solve is what says which.
    #[test]
    fn a_moving_observer_meets_the_light_at_a_different_time() {
        let at = DVec3::new(1_000_000.0, 0.0, 0.0);
        let still = ship(2, at);
        let mut closing = ship(3, at);
        // Falling toward the source at a tenth of `c`, from the same place at the same time.
        closing.path = Path::coasting(at, DVec3::new(-0.1, 0.0, 0.0), 0);

        let sent = pulse(0, DVec3::ZERO, 1.0);
        let a = schedule(&sent, &still).unwrap().arrive_t;
        let b = schedule(&sent, &closing).unwrap().arrive_t;
        assert!(b < a, "closing on the source should meet its light sooner: {b} against {a}");
        // Closing at beta, the meeting is at d/(1+beta).
        assert!((b as f64 - 1_000_000.0 / 1.1).abs() < 2.0, "{b}");
    }

    #[test]
    fn strength_falls_as_the_inverse_square_and_does_not_divide_by_zero() {
        assert_eq!(strength(100.0, 10.0), 1.0);
        assert_eq!(strength(100.0, 20.0), 0.25);
        assert!(strength(100.0, 0.0).is_finite(), "a coincident source is loud, not infinite");
    }
}
