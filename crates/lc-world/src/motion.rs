//! How a ship moves, and the events that change it.
//!
//! One implementation, run on both sides. The server is authoritative and the client predicts
//! locally, and the only way those agree is by being the same code — so the *event* carries the
//! order rather than the trajectory, and each side works the trajectory out from it. A crossing
//! is not sent; "set this course, at this time, with this drive" is sent, and
//! [`Cruise::plan`](crate::flight::Cruise::plan) does the rest identically at both ends.
//!
//! See `lightcone/docs/08-networking.md`, which calls this fold `lc_world::apply`.

use glam::DVec3;

use crate::coast::{self, Coast};
use crate::flight::{Cruise, Drive, JULIAN_YEAR_S, Phase};
use crate::navigation::{Course, Waypoint};
use crate::system::LocalSystem;

/// A ship, by the identifier whoever owns it uses. Opaque here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShipId(pub i64);

/// How a ship is moving. The four are exclusive, and that exclusivity is the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Motive {
    /// Under thrust, on a planned crossing.
    Crossing(Cruise),
    /// Held on a place by thrust. A station is a position, not a trajectory.
    Holding(Waypoint),
    /// Ballistic on a conic, about whichever body's influence it is in.
    Falling(Coast),
    /// Nothing holding it and nothing to fall towards: a straight line at whatever it has.
    ///
    /// Carries where and when the line started, so it is read rather than integrated. Adding
    /// `beta * elapsed` each step is not the same number at two step sizes, and two sides that
    /// stepped differently would drift apart by the rounding.
    Drifting { from_ly: DVec3, since_t: f64 },
}

/// Everything about a ship that moves.
#[derive(Clone, Debug, PartialEq)]
pub struct ShipState {
    /// Light-years from the world origin.
    pub position_ly: DVec3,
    /// Velocity as a fraction of `c`.
    pub beta: DVec3,
    pub motive: Motive,
    pub drive: Drive,
    /// Seconds on the ship's own clock.
    pub clock_s: f64,
    /// [`ShipState::clock_s`] when the current crossing began. A crossing carries its own proper
    /// time as a closed form, so the clock is read from it rather than integrated.
    crossing_clock_base_s: f64,
    /// Where the crossing under way is *for*. Arriving turns into holding this, rather than
    /// into drifting away from the place the crossing was flown to.
    arrive_at: Option<Waypoint>,
}

impl ShipState {
    /// At rest at a point.
    pub fn at(position_ly: DVec3) -> Self {
        Self {
            position_ly,
            beta: DVec3::ZERO,
            motive: Motive::Drifting { from_ly: position_ly, since_t: 0.0 },
            drive: Drive::DEFAULT,
            clock_s: 0.0,
            crossing_clock_base_s: 0.0,
            arrive_at: None,
        }
    }

    pub fn is_under_way(&self) -> bool {
        matches!(self.motive, Motive::Crossing(_))
    }
}

/// What can happen to a ship.
///
/// The order, never the trajectory. A `SetCourse` that carried the solved crossing would be a
/// second copy of [`Cruise::plan`](crate::flight::Cruise::plan)'s output on the wire, free to
/// disagree with the one the other side would have worked out.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Go somewhere, at this acceleration.
    SetCourse { course: Course, drive: Drive },
    /// Cut the engine. Not a stop: whatever velocity it had, it keeps.
    CutDrive,
    /// Put out a pulse. Changes nothing about the motion, and is here because the fold is what
    /// both sides run over everything that happened.
    Transmit { power_w: f64 },
    /// The ballistic arc crosses into another body's influence and is re-solved about it.
    ///
    /// An event rather than something each side notices for itself. Patched conics done by
    /// detection depend on *when* the crossing is looked for, so two sides stepping differently
    /// would produce different elements and drift apart. The authority decides when it happened
    /// and everyone folds the same instant. Doc 08 says the same thing about shards: a shell
    /// crossing is already an event with a coordinate.
    Repatch,
}

/// One thing that happened to one ship, at one coordinate time.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub ship: ShipId,
    /// Coordinate seconds.
    pub at_t: f64,
    pub change: Change,
}

/// Why an event could not be applied.
///
/// A refusal, not a failure: the same event applied on the server and on a client that has a
/// staler view of the system can legitimately differ, and the client's answer is the one that
/// gets corrected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// There is no local system to resolve the course against.
    NotInASystem,
    /// The system has nothing answering to it.
    NoSuchPlace,
}

/// Fold one event into a ship.
///
/// **This is the function both sides run.** Anything that decides where a ship goes has to
/// happen here or in what it calls, because a rule applied on one side only is a desynchronised
/// client waiting to happen.
pub fn apply(
    state: &mut ShipState,
    system: Option<&LocalSystem>,
    event: &Event,
) -> Result<(), Rejected> {
    match &event.change {
        Change::Transmit { .. } => Ok(()),
        Change::Repatch => {
            // The arc is re-solved about whatever holds the ship *now*. Both sides get the same
            // answer because they are given the same time -- which is the whole reason a shell
            // crossing is an event with a coordinate rather than something each side notices.
            let velocity = velocity_m_s(state, system, event.at_t);
            state.motive = match system
                .and_then(|s| Coast::from_state(s, state.position_ly, velocity, event.at_t))
            {
                Some(arc) => Motive::Falling(arc),
                None => Motive::Drifting { from_ly: state.position_ly, since_t: event.at_t },
            };
            Ok(())
        }
        Change::CutDrive => {
            let velocity = velocity_m_s(state, system, event.at_t);
            state.beta = coast::beta_of(velocity);
            state.motive = match system
                .and_then(|s| Coast::from_state(s, state.position_ly, velocity, event.at_t))
            {
                Some(arc) => Motive::Falling(arc),
                // Between systems, or a radial state that has no conic at all. Either way it is
                // a straight line at the velocity it has.
                None => Motive::Drifting { from_ly: state.position_ly, since_t: event.at_t },
            };
            Ok(())
        }
        Change::SetCourse { course, drive } => {
            let system = system.ok_or(Rejected::NotInASystem)?;
            let waypoint = course.resolve(system, state.position_ly).ok_or(Rejected::NoSuchPlace)?;
            let (cruise, aimed) =
                crate::navigation::plan(system, &waypoint, state.position_ly, event.at_t, *drive)
                    .ok_or(Rejected::NoSuchPlace)?;
            state.drive = *drive;
            state.crossing_clock_base_s = state.clock_s;
            state.motive = Motive::Crossing(cruise);
            // Remembered so that arriving becomes holding rather than drifting away from the
            // place the crossing was for.
            state.arrive_at = Some(aimed);
            Ok(())
        }
    }
}

/// Move a ship to a coordinate time.
///
/// Read at the new time rather than integrated from the old one, in every branch. A paused
/// clock, a clock at a year a second and a dropped frame all leave the ship in the same place,
/// which is what lets a client at one frame rate and a server at another agree.
pub fn advance(state: &mut ShipState, system: Option<&LocalSystem>, now_s: f64, elapsed_s: f64) {
    match &state.motive {
        Motive::Crossing(cruise) => {
            let flight = cruise.at(now_s);
            state.position_ly = flight.position_ly;
            state.beta = flight.beta;
            // Read rather than integrated. Stepping `elapsed / gamma` uses one velocity for a
            // whole interval the velocity changed across, which is wrong by first order
            // everywhere and wrong by the entire last step at arrival, where the ship has
            // already stopped. The crossing carries the closed form; use it.
            state.clock_s = state.crossing_clock_base_s + flight.proper_s;
            if flight.phase == Phase::Arrived {
                state.beta = DVec3::ZERO;
                state.motive = match state.arrive_at.take() {
                    Some(waypoint) => Motive::Holding(waypoint),
                    None => Motive::Drifting { from_ly: state.position_ly, since_t: now_s },
                };
            }
        }
        Motive::Holding(waypoint) => {
            state.clock_s += elapsed_s;
            if let Some(at) = system.and_then(|s| waypoint.place(s)) {
                state.position_ly = at;
            }
        }
        Motive::Falling(arc) => {
            state.clock_s += elapsed_s;
            let Some(system) = system else {
                // The system is gone, which means the ship has left it. Whatever the conic
                // said, out here it is a straight line.
                state.motive =
                    Motive::Drifting { from_ly: state.position_ly, since_t: now_s };
                return;
            };
            // Read, and nothing else. Re-solving the arc here would make it depend on the step
            // size; that is [`Change::Repatch`]'s job and the authority's timing.
            if let Some((at, velocity)) = arc.at(system, now_s) {
                state.position_ly = at;
                state.beta = coast::beta_of(velocity);
            }
        }
        Motive::Drifting { from_ly, since_t } => {
            state.clock_s += elapsed_s;
            // A light-year is a year of travel at `c` by definition, so a beta is already
            // light-years per year -- and it is read from the start of the line rather than
            // accumulated, so any two step sizes land on the same place.
            state.position_ly = *from_ly + state.beta * (now_s - since_t) / JULIAN_YEAR_S;
        }
    }
}

/// Whether the ballistic arc has left the influence it was solved in, and the event that says
/// so.
///
/// Called by whoever is authoritative — the server, or a client with no server. Everyone else
/// is *told*, which is what keeps the two sides on the same arc.
pub fn repatch_due(state: &ShipState, system: &LocalSystem, now_s: f64) -> Option<Event> {
    let Motive::Falling(arc) = &state.motive else { return None };
    let velocity = state.beta * crate::flight::C_M_S;
    arc.repatched(system, state.position_ly, velocity, now_s)?;
    Some(Event { ship: ShipId(0), at_t: now_s, change: Change::Repatch })
}

/// How fast a ship is going, metres a second, world frame.
pub fn velocity_m_s(state: &ShipState, system: Option<&LocalSystem>, now_s: f64) -> DVec3 {
    match &state.motive {
        Motive::Crossing(cruise) => cruise.at(now_s).beta * crate::flight::C_M_S,
        Motive::Holding(waypoint) => {
            system.and_then(|s| waypoint.velocity_at(s)).unwrap_or(DVec3::ZERO)
        }
        Motive::Falling(_) | Motive::Drifting { .. } => state.beta * crate::flight::C_M_S,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::Plane;
    use crate::sky::{CatalogueStar, StarProvider};

    fn sol() -> Option<LocalSystem> {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .ok()?;
        let sun: CatalogueStar =
            provider.stars().iter().find(|s| s.name.as_deref() == Some("Sol"))?.clone();
        let mut system = LocalSystem::for_star(&sun)?;
        system.advance_to(0.0);
        Some(system)
    }

    fn orbit(body: &str) -> Change {
        Change::SetCourse {
            course: Course::Orbit {
                body: body.into(),
                altitude_radii: 2.0,
                plane: Plane::Equatorial,
            },
            drive: Drive::DEFAULT,
        }
    }

    /// The property the whole arrangement exists for: two sides folding the same events over
    /// the same system reach the same state, to the bit.
    ///
    /// This is what "the client predicts by running the same code" has to mean. If the event
    /// carried the solved crossing instead of the order, this would still pass and would stop
    /// being worth anything — the two would agree because one was told the answer.
    #[test]
    fn two_sides_folding_the_same_events_agree_exactly() {
        let Some(mut server_system) = sol() else { return };
        let Some(mut client_system) = sol() else { return };

        let events = [
            Event { ship: ShipId(1), at_t: 0.0, change: orbit("Earth") },
            Event { ship: ShipId(1), at_t: 40_000.0, change: Change::Transmit { power_w: 1.0e9 } },
            Event { ship: ShipId(1), at_t: 90_000.0, change: Change::CutDrive },
        ];

        let mut server = ShipState::at(DVec3::ZERO);
        let mut client = ShipState::at(DVec3::ZERO);

        // Deliberately different step sizes: the server ticks at 438 s of coordinate time and
        // a client redraws far more often. Both land on the same instants -- the events, and
        // one shared end -- because that is the only comparison that means anything.
        const END_T: f64 = 300_000.0;
        let run = |state: &mut ShipState, system: &mut LocalSystem, step: f64| {
            let mut now = 0.0;
            let mut step_to = |state: &mut ShipState, system: &mut LocalSystem, target: f64, now: &mut f64| {
                while *now < target {
                    let next = (*now + step).min(target);
                    let elapsed = next - *now;
                    *now = next;
                    system.advance_to(*now);
                    advance(state, Some(system), *now, elapsed);
                }
            };
            for event in &events {
                step_to(state, system, event.at_t, &mut now);
                system.advance_to(now);
                apply(state, Some(system), event).expect("the event applies");
            }
            step_to(state, system, END_T, &mut now);
            now
        };
        let server_end = run(&mut server, &mut server_system, 438.0);
        let client_end = run(&mut client, &mut client_system, 61.0);

        assert_eq!(server_end.round(), client_end.round(), "the runs ended at different times");
        assert_eq!(
            server.position_ly, client.position_ly,
            "the two sides disagree about where the ship is",
        );
        assert_eq!(server.beta, client.beta);
        assert_eq!(server.motive, client.motive);
    }

    /// A crossing that arrives becomes a station, not a drift. The place was the point of it.
    #[test]
    fn arriving_becomes_holding() {
        let Some(mut system) = sol() else { return };
        let mut ship = ShipState::at(DVec3::ZERO);
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: 0.0,
            change: orbit("Earth"),
        })
        .unwrap();
        assert!(ship.is_under_way());

        let Motive::Crossing(cruise) = ship.motive.clone() else { panic!("a crossing") };
        let arrival = cruise.duration_s();
        let mut now = 0.0;
        while now < arrival + 1.0 {
            now = (now + 500.0).min(arrival + 1.0);
            system.advance_to(now);
            advance(&mut ship, Some(&system), now, 500.0);
        }
        assert!(matches!(ship.motive, Motive::Holding(_)), "{:?}", ship.motive);
        assert_eq!(ship.beta, DVec3::ZERO, "a ship on station is not still burning");

        // And it stays on it: the body moves and the ship goes with it.
        let before = ship.position_ly;
        for _ in 0..20 {
            now += 500.0;
            system.advance_to(now);
            advance(&mut ship, Some(&system), now, 500.0);
        }
        assert_ne!(ship.position_ly, before, "a station does not stand still in the world");
        let earth = system.body_position_ly("Earth").unwrap();
        let radius = ship.position_ly.distance(earth) * crate::system::M_PER_LY / 6.371e6;
        assert!((radius - 3.0).abs() < 0.05, "drifted off station to {radius} radii");
    }

    /// Cutting the engine keeps the velocity, which inside a system is a conic.
    #[test]
    fn cutting_the_drive_leaves_a_conic() {
        let Some(system) = sol() else { return };
        let mut ship = ShipState::at(DVec3::ZERO);
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: 0.0,
            change: orbit("Earth"),
        })
        .unwrap();
        // Part-way through the burn, so it has real speed.
        let mut now = 0.0;
        for _ in 0..30 {
            now += 500.0;
            advance(&mut ship, Some(&system), now, 500.0);
        }
        let moving = ship.beta;
        assert!(moving.length() > 1.0e-6);

        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: now,
            change: Change::CutDrive,
        })
        .unwrap();
        assert!(matches!(ship.motive, Motive::Falling(_)), "{:?}", ship.motive);
        assert!((ship.beta - moving).length() < moving.length() * 1e-9, "it braked");
    }

    /// Between systems there is no conic, and a cut engine is a straight line at whatever it
    /// had. The distance is exactly the beta times the years, because that is what a light-year
    /// means.
    #[test]
    fn with_no_system_a_cut_drive_is_a_straight_line() {
        let mut ship = ShipState::at(DVec3::ZERO);
        ship.beta = DVec3::new(0.5, 0.0, 0.0);
        apply(&mut ship, None, &Event { ship: ShipId(1), at_t: 0.0, change: Change::CutDrive })
            .unwrap();
        assert!(matches!(ship.motive, Motive::Drifting { .. }), "{:?}", ship.motive);

        let year = JULIAN_YEAR_S;
        advance(&mut ship, None, year, year);
        assert!((ship.position_ly.x - 0.5).abs() < 1e-12, "{}", ship.position_ly.x);
        assert_eq!(ship.clock_s, year, "a drifting clock runs with coordinate time");
    }

    /// A course to nowhere is refused rather than applied as something else. Both sides have to
    /// refuse it identically, which is why it is a value and not a panic.
    #[test]
    fn a_course_with_nowhere_to_go_is_refused() {
        let Some(system) = sol() else { return };
        let mut ship = ShipState::at(DVec3::ZERO);
        let nowhere = Event {
            ship: ShipId(1),
            at_t: 0.0,
            change: Change::SetCourse {
                course: Course::Rings("Earth".into()),
                drive: Drive::DEFAULT,
            },
        };
        assert_eq!(apply(&mut ship, Some(&system), &nowhere), Err(Rejected::NoSuchPlace));
        assert!(
            matches!(ship.motive, Motive::Drifting { .. }),
            "a refused course changed the ship anyway",
        );

        assert_eq!(apply(&mut ship, None, &nowhere), Err(Rejected::NotInASystem));
    }

    /// A transmission changes nothing about the motion. It is in the fold because the fold is
    /// what runs over everything that happened, not because it moves anything.
    #[test]
    fn a_transmission_moves_nothing() {
        let mut ship = ShipState::at(DVec3::new(1.0, 2.0, 3.0));
        let before = ship.clone();
        apply(&mut ship, None, &Event {
            ship: ShipId(1),
            at_t: 5.0,
            change: Change::Transmit { power_w: 1.0e9 },
        })
        .unwrap();
        assert_eq!(ship, before);
    }
}
