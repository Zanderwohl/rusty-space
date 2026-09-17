//! Where a sighted craft appears, between the statements about it.
//!
//! A shard states what each client can see once a tick, and a tick is 438 coordinate seconds at
//! the design rate. Held still for that long, a craft in low orbit falls tens of thousands of
//! kilometres behind where it is — and behind a ship flying formation with it, which is drawn
//! at its own present every frame.
//!
//! So a sighting is **reckoned**: along its conic inside a system, a straight line outside one,
//! to the instant whose light reaches the observer now. Never later — the event drawn is always
//! one whose light has arrived. What it gets wrong is a manoeuvre since the light left, for at
//! most a tick, until the next statement says so.
//!
//! Backwards too. A client whose clock is behind the shard's is sent samples from its own
//! future, and holding one until its time comes was the same jump as never reckoning at all:
//! a clock twenty seconds behind in low orbit is a thousand kilometres.

use glam::DVec3;

use crate::coast::{self, Coast};
use crate::consort;
use crate::flight::JULIAN_YEAR_S;
use crate::pursuit::Sighting;
use crate::system::{LOCAL_SHELL_LY, LocalSystem};

/// Fixed-point steps for the emission time along a conic. Each shrinks the error by `beta`,
/// which [`consort::GALILEAN_BETA`] bounds at a thousandth.
const CONIC_LIGHT_STEPS: usize = 3;

/// A sighting, ready to be drawn at any later time.
#[derive(Clone, Debug, PartialEq)]
pub struct Reckoning {
    pub seen: Sighting,
    /// The conic it was on, solved once per statement rather than once per frame. `None` for a
    /// craft outside `system`, or too fast for one.
    frame: Option<Coast>,
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
    /// conic about that system's bodies.
    pub fn new(system: Option<&LocalSystem>, seen: Sighting) -> Self {
        let frame = system
            .filter(|s| seen.position_ly.distance(s.origin_ly) < LOCAL_SHELL_LY)
            .and_then(|s| consort::frame_for(s, &seen));
        Self { seen, frame }
    }

    /// What `observer_ly` sees at `now_s`.
    pub fn appearance_at(
        &self,
        system: Option<&LocalSystem>,
        observer_ly: DVec3,
        now_s: f64,
    ) -> Appearance {
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
    // Whichever form adds rather than cancels: a contact metres away has both terms near zero.
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

    /// Between the stars, a light-hour off and doing a fifth of `c`: what is drawn is on the
    /// light cone, and later than what was stated.
    #[test]
    fn a_distant_contact_is_drawn_where_its_light_left() {
        let light_hour_ly = 3_600.0 / JULIAN_YEAR_S;
        let seen = drifting(DVec3::X * light_hour_ly, DVec3::Y * 0.2);
        let reckoning = Reckoning::new(None, seen.clone());
        let now_s = 100.0 + 3_600.0 + 600.0;
        let seen_now = reckoning.appearance_at(None, DVec3::ZERO, now_s);

        assert!(seen_now.emitted_s > seen.emitted_s && seen_now.emitted_s < now_s);
        let flight_s = seen_now.position_ly.length() * JULIAN_YEAR_S;
        assert!((seen_now.emitted_s + flight_s - now_s).abs() < 1e-6, "off the light cone");
        assert_eq!(seen_now.position_ly, seen.reckoned_at(seen_now.emitted_s));
    }

    /// A client whose clock is behind the shard's is drawn the craft where it was at that
    /// clock, not where the sample is: holding the sample was a thousand-kilometre jump.
    #[test]
    fn a_sample_from_the_future_is_reckoned_back_to_now() {
        let seen = drifting(DVec3::ZERO, DVec3::Y * 1e-4);
        let reckoning = Reckoning::new(None, seen.clone());
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
        let reckoning = Reckoning::new(None, seen.clone());
        let now_s = 538.0;
        let observer_ly = seen.reckoned_at(now_s) + DVec3::X * 1.5e3 / M_PER_LY;
        let seen_now = reckoning.appearance_at(None, observer_ly, now_s);
        assert!(seen_now.emitted_s <= now_s);
        assert!(now_s - seen_now.emitted_s < 1e-5, "{} s of light over 1.5 km", now_s - seen_now.emitted_s);
    }
}
