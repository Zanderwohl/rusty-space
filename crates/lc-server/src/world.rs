//! What the server knows: things with worldlines, and the events they have made.
//!
//! A craft is [`lc_world::craft::Craft`] — the same model the client runs, folded by the same
//! [`apply`](lc_world::motion::apply). The server used to keep its own impoverished one, a
//! point that was either still or coasting, and could therefore neither validate nor reproduce
//! anything the client actually did.
//!
//! What stays here is what is the *server's* fact rather than the world's: who owns a craft,
//! what has happened, and who is due to be told.

use glam::DVec3;
use lc_proto::ShipId;
use lc_spacetime::Worldline;
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::motion::LIGHT_US_PER_LY;

/// Microseconds of coordinate time in one second.
pub const MICROS_PER_SECOND: i64 = 1_000_000;

/// A craft at rest at a point, light-microseconds from the world origin.
pub fn still(id: ShipId, at: DVec3) -> Craft {
    Craft::at(CraftId(id.0), Kind::Ship, at / LIGHT_US_PER_LY)
}

/// Moving through `at` at `beta`, from coordinate microsecond `epoch_us` onward.
pub fn coasting(id: ShipId, at: DVec3, beta: DVec3, epoch_us: i64) -> Craft {
    let mut craft = still(id, at);
    craft.motion.beta = beta;
    craft.motion.set_adrift(epoch_us as f64 * 1.0e-6);
    craft
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
pub fn schedule(event: &Event, observer: &Craft) -> Option<Scheduled> {
    let line = observer.worldline();
    let arrive = lc_spacetime::arrival_time_at(event.t as f64, event.at, &line)?;
    // Rounded up. Rounding down would put an arrival a microsecond before its true time, and
    // the gate would then release it a microsecond early -- which is the one error this whole
    // system exists to prevent, however small.
    let arrive_t = arrive.ceil() as i64;
    let travelled = (line.position_at(arrive) - event.at).length();
    Some(Scheduled {
        observer: ShipId(observer.id.0),
        event: event.id,
        arrive_t,
        strength: strength(event.power_w, travelled),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(id: i64, at: DVec3) -> Craft {
        still(ShipId(id), at)
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
        // Falling toward the source at a tenth of `c`, from the same place at the same time.
        let closing = coasting(ShipId(3), at, DVec3::new(-0.1, 0.0, 0.0), 0);

        let sent = pulse(0, DVec3::ZERO, 1.0);
        let a = schedule(&sent, &still).unwrap().arrive_t;
        let b = schedule(&sent, &closing).unwrap().arrive_t;
        assert!(b < a, "closing on the source should meet its light sooner: {b} against {a}");
        // Closing at beta, the meeting is at d/(1+beta).
        assert!((b as f64 - 1_000_000.0 / 1.1).abs() < 2.0, "{b}");
    }

    /// The point of the whole exercise: a ship whose motion is a *station in a star system*
    /// schedules its arrivals from where it actually is when the light gets there.
    ///
    /// Under the old model this ship was a point that was either still or coasting, so an
    /// orbiting receiver was scheduled as though it were parked. The error is the chord the
    /// ship covers during the light delay, and here that is most of an orbit.
    #[test]
    fn a_ship_in_orbit_is_scheduled_along_its_orbit() {
        use lc_world::navigation::{Course, Plane};
        use lc_world::sky::{AuthoredStars, StarProvider};

        let sky = AuthoredStars::sample();
        let star = sky.stars().first().expect("a star").clone();
        let Some(system) = lc_world::system::LocalSystem::for_star(&star) else { return };
        let Some(body) = system.inventory().iter().find_map(|e| match &e.target {
            lc_world::navigation::Target::Body(name) => Some(name.clone()),
            _ => None,
        }) else {
            return;
        };

        let course = Course::Orbit { body, altitude_radii: 2.0, plane: Plane::Equatorial };
        let Some(waypoint) = course.resolve(&system, system.star_position_ly()) else { return };
        let mut motion = lc_world::motion::ShipState::at(system.star_position_ly());
        motion.begin_holding(waypoint);

        let mut orbiting = Craft::at(CraftId(2), Kind::Ship, DVec3::ZERO);
        orbiting.motion = motion.clone();
        orbiting.enter(Some(std::sync::Arc::new(system)), 0.0);
        // The same ship, frozen where it was at t = 0: what the old model could represent.
        let parked = still(ShipId(3), orbiting.position_at(0.0));

        // From far enough away that the delay is many orbits.
        let far = orbiting.position_at(0.0) + DVec3::new(5.0e9, 0.0, 0.0);
        let sent = pulse(0, far, 1.0);
        let moving = schedule(&sent, &orbiting).expect("it arrives");
        let still_there = schedule(&sent, &parked).expect("it arrives");

        // Both are about the light-crossing time, and they are not the same instant.
        assert!(moving.arrive_t > 1.0e9 as i64, "{}", moving.arrive_t);
        // Most of a second apart, which is what the old model got wrong. The gate it feeds
        // measures in microseconds.
        let slip = (moving.arrive_t - still_there.arrive_t).abs();
        assert!(
            slip > 100_000,
            "an orbiting receiver was scheduled {slip} microseconds from a parked one",
        );

        // And the moving one is right: at its own arrival, it is exactly a light-delay away.
        let meeting = orbiting.position_at(moving.arrive_t as f64);
        let gap = (meeting - far).length();
        assert!(
            (gap - moving.arrive_t as f64).abs() < 2.0,
            "{gap} light-microseconds away at t = {}",
            moving.arrive_t,
        );
    }

    #[test]
    fn strength_falls_as_the_inverse_square_and_does_not_divide_by_zero() {
        assert_eq!(strength(100.0, 10.0), 1.0);
        assert_eq!(strength(100.0, 20.0), 0.25);
        assert!(strength(100.0, 0.0).is_finite(), "a coincident source is loud, not infinite");
    }
}
