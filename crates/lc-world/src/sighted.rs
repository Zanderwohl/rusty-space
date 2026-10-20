//! Where a sighted craft appears, between the statements about it.
//!
//! A shard states what each client can see once a tick, and a tick is 438 coordinate seconds at
//! the design rate. Held still for that long, a craft in low orbit falls tens of thousands of
//! kilometers behind where it is — and behind a ship flying formation with it, which is drawn
//! at its own present every frame.
//!
//! So a sighting is **reckoned**: along its conic inside a system, a straight line outside one,
//! to the instant whose light reaches the observer now. Never later — the event drawn is always
//! one whose light has arrived. What it gets wrong is a maneuvere since the light left, for at
//! most a tick, until the next statement says so.
//!
//! Backwards too. A client whose clock is behind the shard's is sent samples from its own
//! future, and holding one until its time comes was the same jump as never reckoning at all:
//! a clock twenty seconds behind in low orbit is a thousand kilometers.
//!
//! **A burning craft keeping pace with the observer is drawn at its offset from it instead.**
//! Reckoned on its own it coasts for up to a tick while the observer, drawn at its own present,
//! goes on burning, and the two part and snap back once a statement: at five gravities and the
//! design rate that is thousands of kilometers, twenty times a second, between a pursuer and the
//! quarry it is matching. Their shared motion is what neither end can reckon, and at an offset it
//! cancels: what is left to get wrong is the difference between the two burns, which in a chase
//! or a flotilla is nothing. Gated on the craft burning, since a coasting one is reckoned
//! exactly, and one beside an observer that burns away from it would otherwise be dragged along.

use glam::DVec3;

use crate::boost;
use crate::coast::{self, Coast};
use crate::consort;
use crate::flight::JULIAN_YEAR_S;
use crate::pursuit::Sighting;
use crate::system::{LOCAL_SHELL_LY, LocalSystem};

/// Fixed-point steps for the emission time along a conic. Each shrinks the error by `beta`,
/// which [`consort::GALILEAN_BETA`] bounds at a thousandth.
const CONIC_LIGHT_STEPS: usize = 3;

/// The fastest a craft may go relative to the observer and still be drawn at its offset from it.
/// The offset drifts by the difference of the two velocities, which is Galilean.
pub const ALONGSIDE_BETA: f64 = consort::GALILEAN_BETA;

/// The observer as a statement's light reached it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Receiver {
    pub position_ly: DVec3,
    pub beta: DVec3,
    /// Coordinate seconds the light arrived.
    pub at_s: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Alongside {
    offset_ly: DVec3,
    drift: DVec3,
    arrived_s: f64,
}

/// A sighting, ready to be drawn at any later time.
#[derive(Clone, Debug, PartialEq)]
pub struct Reckoning {
    pub seen: Sighting,
    /// The conic it was on, solved once per statement rather than once per frame. `None` for a
    /// craft outside `system`, or too fast for one.
    frame: Option<Coast>,
    alongside: Option<Alongside>,
}

/// What an observer sees of a craft at one instant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    /// Light-years from the world origin, where the light left.
    pub position_ly: DVec3,
    pub beta: DVec3,
    /// Coordinate seconds the light left.
    pub emitted_s: f64,
}

impl Reckoning {

    /// `system` is the one the observer is in; a sighting outside its shell is not put on a
    /// conic about that system's bodies. `burning` is whether anything was lit as its light left.
    pub fn new(system: Option<&LocalSystem>, seen: Sighting, burning: bool, receiver: Receiver) -> Self {
        let frame = system
            .filter(|s| seen.position_ly.distance(s.origin_ly) < LOCAL_SHELL_LY)
            .and_then(|s| consort::frame_for(s, &seen));
        let alongside = (burning
            && boost::velocity_to_frame(seen.beta, receiver.beta).length() < ALONGSIDE_BETA)
            .then(|| Alongside {
                offset_ly: seen.position_ly - receiver.position_ly,
                drift: seen.beta - receiver.beta,
                arrived_s: receiver.at_s,
            });
        Self { seen, frame, alongside }
    }

    /// What `observer_ly` sees at `now_s`.
    pub fn appearance_at(
        &self,
        system: Option<&LocalSystem>,
        observer_ly: DVec3,
        now_s: f64,
    ) -> Appearance {
        if let Some(alongside) = &self.alongside {
            let since_s = now_s - alongside.arrived_s;
            return Appearance {
                position_ly: observer_ly + alongside.offset_ly + alongside.drift * since_s / JULIAN_YEAR_S,
                beta: self.seen.beta,
                emitted_s: self.seen.emitted_s + since_s,
            };
        }
        if let Some((frame, system)) = self.frame.as_ref().zip(system)
            && let Some(seen) = along_conic(frame, system, observer_ly, now_s)
        {
            return seen;
        }
        let emitted_s = now_s - light_time_along_line(&self.seen, observer_ly, now_s);
        Appearance {
            position_ly: self.seen.reckoned_at(emitted_s),
            beta: self.seen.beta,
            emitted_s,
        }
    }
}

fn along_conic(
    frame: &Coast,
    system: &LocalSystem,
    observer_ly: DVec3,
    now_s: f64,
) -> Option<Appearance> {
    let mut emitted_s = now_s;
    for _ in 0..CONIC_LIGHT_STEPS {
        let (at_ly, _) = frame.at(system, emitted_s)?;
        emitted_s = now_s - observer_ly.distance(at_ly) * JULIAN_YEAR_S;
    }
    let (position_ly, velocity) = frame.at(system, emitted_s)?;
    Some(Appearance { position_ly, beta: coast::beta_of(velocity), emitted_s })
}

/// Seconds the light arriving at `now_s` has been in flight from a straight worldline, closed
/// form. With `d` from where the craft is at `now_s` to the observer and `c = 1`, the flight
/// time `s` solves `|d + b s| = s`: `(1 - b^2) s^2 - 2 (d.b) s - d^2 = 0`, positive root.
fn light_time_along_line(seen: &Sighting, observer_ly: DVec3, now_s: f64) -> f64 {
    let d = observer_ly - seen.reckoned_at(now_s);
    let b = seen.beta;
    // A sighting is sub-luminal; the floor only keeps a malformed one finite.
    let a = (1.0 - b.length_squared()).max(f64::EPSILON);
    let along = d.dot(b);
    let root = (along * along + a * d.length_squared()).sqrt();
    // Whichever form adds rather than cancels: a contact meters away has both terms near zero.
    let years = if along >= 0.0 {
        (along + root) / a
    } else if root - along > 0.0 {
        d.length_squared() / (root - along)
    } else {
        0.0
    };
    years * JULIAN_YEAR_S
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::ShipId;
    use crate::system::M_PER_LY;

    fn drifting(position_ly: DVec3, beta: DVec3) -> Sighting {
        Sighting { target: ShipId(2), position_ly, beta, length_m: 500.0, emitted_s: 100.0 }
    }

    fn coasting(seen: &Sighting) -> Reckoning {
        let receiver = Receiver { position_ly: DVec3::ZERO, beta: DVec3::ZERO, at_s: seen.emitted_s };
        Reckoning::new(None, *seen, false, receiver)
    }

    /// Two craft a kilometer apart at nine-tenths of `c`, both burning, and the observer drawn a
    /// tick of its own burn later: the contact is still a kilometer off, where reckoning it on
    /// its own line would have left it the whole burn behind.
    #[test]
    fn a_craft_burning_alongside_holds_its_offset() {
        let beta = DVec3::X * 0.9;
        let observer_ly = DVec3::ZERO;
        let seen = drifting(observer_ly + DVec3::Y * 1.0e3 / M_PER_LY, beta);
        let receiver = Receiver { position_ly: observer_ly, beta, at_s: seen.emitted_s };
        let reckoning = Reckoning::new(None, seen, true, receiver);

        let tick_s = 438.0;
        let burned_m = 0.5 * 50.0 * tick_s * tick_s;
        let now_s = seen.emitted_s + tick_s;
        let observer_now_ly = observer_ly + beta * tick_s / JULIAN_YEAR_S + DVec3::X * burned_m / M_PER_LY;
        let drawn = reckoning.appearance_at(None, observer_now_ly, now_s);
        let range_m = (drawn.position_ly - observer_now_ly) * M_PER_LY;
        assert!((range_m - DVec3::Y * 1.0e3).length() < 1.0, "{range_m:?}");
        assert_eq!(drawn.emitted_s, now_s);

        let alone = coasting(&seen).appearance_at(None, observer_now_ly, now_s);
        assert!(alone.position_ly.distance(observer_now_ly) * M_PER_LY > 0.9 * burned_m);
    }

    /// Coasting beside an observer that burns away from it, a craft stays on its own line rather
    /// than being carried off with the observer; and one passing fast is never alongside.
    #[test]
    fn only_a_burning_craft_keeping_pace_is_drawn_alongside() {
        let beta = DVec3::X * 0.5;
        let seen = drifting(DVec3::Y * 1.0e3 / M_PER_LY, beta);
        let at = |receiver_beta: DVec3, burning: bool| {
            let receiver = Receiver { position_ly: DVec3::ZERO, beta: receiver_beta, at_s: seen.emitted_s };
            Reckoning::new(None, seen, burning, receiver)
                .appearance_at(None, DVec3::X * 5.0e6 / M_PER_LY, seen.emitted_s + 100.0)
        };
        let alone = coasting(&seen).appearance_at(None, DVec3::X * 5.0e6 / M_PER_LY, seen.emitted_s + 100.0);
        assert_eq!(at(beta, false), alone);
        let passing = beta + DVec3::X * ALONGSIDE_BETA;
        assert_eq!(at(passing, true), alone);
        assert_ne!(at(beta, true), alone);
    }

    /// Between the stars, a light-hour off and doing a fifth of `c`: what is drawn is on the
    /// light cone, and later than what was stated.
    #[test]
    fn a_distant_contact_is_drawn_where_its_light_left() {
        let light_hour_ly = 3_600.0 / JULIAN_YEAR_S;
        let seen = drifting(DVec3::X * light_hour_ly, DVec3::Y * 0.2);
        let reckoning = coasting(&seen);
        let now_s = 100.0 + 3_600.0 + 600.0;
        let seen_now = reckoning.appearance_at(None, DVec3::ZERO, now_s);

        assert!(seen_now.emitted_s > seen.emitted_s && seen_now.emitted_s < now_s);
        let flight_s = seen_now.position_ly.length() * JULIAN_YEAR_S;
        assert!((seen_now.emitted_s + flight_s - now_s).abs() < 1e-6, "off the light cone");
        assert_eq!(seen_now.position_ly, seen.reckoned_at(seen_now.emitted_s));
    }

    /// A client whose clock is behind the shard's is drawn the craft where it was at that
    /// clock, not where the sample is: holding the sample was a thousand-kilometer jump.
    #[test]
    fn a_sample_from_the_future_is_reckoned_back_to_now() {
        let seen = drifting(DVec3::ZERO, DVec3::Y * 1e-4);
        let reckoning = coasting(&seen);
        let now_s = seen.emitted_s - 20.0;
        let observer_ly = seen.reckoned_at(now_s) + DVec3::X * 1.5e3 / M_PER_LY;
        let early = reckoning.appearance_at(None, observer_ly, now_s);
        let range_m = observer_ly.distance(early.position_ly) * M_PER_LY;
        assert!((range_m - 1.5e3).abs() < 1e-2, "{range_m} m, not 1500");
        assert!(early.emitted_s < now_s);
    }

    /// Nothing is drawn past now, at any range.
    #[test]
    fn a_contact_alongside_is_drawn_at_the_present() {
        let seen = drifting(DVec3::ZERO, DVec3::Y * 1e-4);
        let reckoning = coasting(&seen);
        let now_s = 538.0;
        let observer_ly = seen.reckoned_at(now_s) + DVec3::X * 1.5e3 / M_PER_LY;
        let seen_now = reckoning.appearance_at(None, observer_ly, now_s);
        assert!(seen_now.emitted_s <= now_s);
        assert!(now_s - seen_now.emitted_s < 1e-5, "{} s of light over 1.5 km", now_s - seen_now.emitted_s);
    }
}
