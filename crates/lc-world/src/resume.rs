//! Putting a ship back exactly as it was.
//!
//! A ship is not a position and a velocity. It is *doing* something — holding a low polar orbit
//! of Titan, braking into Proxima, falling round a moon on a conic — and that is the part a
//! reconnect used to lose. The welcome carried a point, the client placed its ship there at
//! rest, and a player who signed out of an orbit signed back into a drift.
//!
//! What crosses the wire is the **recipe, never the trajectory** — the same rule
//! [`crate::motion`] states for orders. A crossing goes as the five values
//! [`Cruise::plan_from`] takes and is re-planned at the far end; a conic goes as nothing at all,
//! because [`Coast::from_state`] re-solves it from the state that is already there. Sending the
//! solved answer would put a second copy of it in play, free to disagree with the one the
//! receiver would have worked out.
//!
//! The mirror in [`lc_proto`] is kept honest the way [`crate::navigation`]'s is: both matches
//! are exhaustive, so a motive the world gains and the wire has not learned is a compile error.

use glam::DVec3;

use crate::coast::Coast;
use crate::flight::{C_M_S, Cruise, Drive};
use crate::motion::ShipState;
use crate::navigation::{Anchor, LagrangePoint, Orbit, Waypoint};
use crate::system::LocalSystem;

/// Everything needed to put a ship back, as parameters rather than as a solved path.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub position_ly: DVec3,
    pub beta: DVec3,
    /// Seconds on the ship's own clock. Never recomputed from the world's: no resynchronising
    /// un-ages a crew. See `lightcone/docs/17-reconciliation.md`.
    pub clock_s: f64,
    pub drive: Drive,
    pub motive: Recipe,
}

/// How to rebuild a [`Motive`].
#[derive(Clone, Debug, PartialEq)]
pub enum Recipe {
    /// The arguments of [`Cruise::plan_from`], plus where the crossing is *for*.
    Crossing {
        from_ly: DVec3,
        beta0: DVec3,
        to_ly: DVec3,
        start_s: f64,
        drive: Drive,
        arrive_at: Option<Waypoint>,
        /// The ship's clock when the crossing began. A crossing carries its own proper time as
        /// a closed form and reads the clock from this, so restoring the current value alone
        /// would make the crew's clock jump on the next step.
        clock_base_s: f64,
    },
    /// The arguments an approach was solved from, and re-solved at the far end.
    ///
    /// Every one of them is relative or a sighting — see [`crate::pursuit::Approach`] — so this
    /// tells its receiver nothing about the quarry that the receiver's own eyes could not.
    Rendezvous {
        approach: crate::pursuit::Approach,
        /// The ship's clock when the approach began; see [`Recipe::Crossing`].
        clock_base_s: f64,
    },
    /// The station itself, which is already the parameter.
    Holding(Waypoint),
    /// Carries nothing: the conic is re-solved from the position and velocity above, against
    /// the system the ship is in. Both ends pick the same primary by the same influence test.
    Falling,
    Drifting {
        from_ly: DVec3,
        since_t: f64,
    },
}

impl Snapshot {
    /// Rebuild a ship.
    ///
    /// `system` is whichever one holds [`Snapshot::position_ly`], by the shell rule both ends
    /// apply. Without one a conic has nothing to be about, so a falling ship comes back
    /// drifting — the same degradation [`ShipState::leave_system`] makes, and for the same
    /// reason.
    pub fn restore(self, system: Option<&LocalSystem>, now_s: f64) -> ShipState {
        let mut state = ShipState::at(self.position_ly);
        state.beta = self.beta;
        state.drive = self.drive;
        state.clock_s = self.clock_s;
        match self.motive {
            Recipe::Crossing { from_ly, beta0, to_ly, start_s, drive, arrive_at, clock_base_s } => {
                let cruise = Cruise::plan_from(from_ly, beta0, to_ly, start_s, drive);
                state.resume_crossing(cruise, arrive_at, clock_base_s);
            }
            Recipe::Rendezvous { approach, clock_base_s } => {
                state.resume_rendezvous(approach.solve(), clock_base_s);
            }
            Recipe::Holding(waypoint) => state.begin_holding(waypoint),
            Recipe::Falling => {
                let coast = system.and_then(|system| {
                    Coast::from_state(system, self.position_ly, velocity_m_s(self.beta), now_s)
                });
                match coast {
                    Some(coast) => state.resume_falling(coast),
                    None => state.set_adrift(now_s),
                }
            }
            Recipe::Drifting { from_ly, since_t } => {
                state.resume_drifting(from_ly, since_t);
            }
        }
        state
    }
}

/// Metres a second from a fraction of `c`.
fn velocity_m_s(beta: DVec3) -> DVec3 {
    beta * C_M_S
}

/// The wire's motion and the world's, converted.
///
/// Total in both directions and exhaustive in both, which is the whole of what keeps the two
/// types from drifting apart.
impl From<&Snapshot> for lc_proto::Motion {
    fn from(snapshot: &Snapshot) -> Self {
        Self {
            at_ly: snapshot.position_ly.to_array(),
            beta: snapshot.beta.to_array(),
            clock_s: snapshot.clock_s,
            drive: drive_out(snapshot.drive),
            motive: match &snapshot.motive {
                Recipe::Crossing {
                    from_ly,
                    beta0,
                    to_ly,
                    start_s,
                    drive,
                    arrive_at,
                    clock_base_s,
                } => lc_proto::Motive::Crossing {
                    from_ly: from_ly.to_array(),
                    beta0: beta0.to_array(),
                    to_ly: to_ly.to_array(),
                    start_s: *start_s,
                    drive: drive_out(*drive),
                    arrive_at: arrive_at.as_ref().map(waypoint_out),
                    clock_base_s: *clock_base_s,
                },
                Recipe::Rendezvous { approach, clock_base_s } => lc_proto::Motive::Rendezvous {
                    from_ly: approach.from_ly.to_array(),
                    beta0: approach.beta0.to_array(),
                    to_ly: approach.to_ly.to_array(),
                    start_s: approach.start_s,
                    drive: drive_out(approach.drive),
                    frame_from_ly: approach.frame_from_ly.to_array(),
                    frame_beta: approach.frame_beta.to_array(),
                    since_t: approach.since_t,
                    target: lc_proto::ShipId(approach.target.0),
                    clock_base_s: *clock_base_s,
                },
                Recipe::Holding(waypoint) => lc_proto::Motive::Holding(waypoint_out(waypoint)),
                Recipe::Falling => lc_proto::Motive::Falling,
                Recipe::Drifting { from_ly, since_t } => lc_proto::Motive::Drifting {
                    from_ly: from_ly.to_array(),
                    since_t: *since_t,
                },
            },
        }
    }
}

impl From<&lc_proto::Motion> for Snapshot {
    fn from(motion: &lc_proto::Motion) -> Self {
        Self {
            position_ly: DVec3::from_array(motion.at_ly),
            beta: DVec3::from_array(motion.beta),
            clock_s: motion.clock_s,
            drive: drive_in(motion.drive),
            motive: match &motion.motive {
                lc_proto::Motive::Crossing {
                    from_ly,
                    beta0,
                    to_ly,
                    start_s,
                    drive,
                    arrive_at,
                    clock_base_s,
                } => Recipe::Crossing {
                    from_ly: DVec3::from_array(*from_ly),
                    beta0: DVec3::from_array(*beta0),
                    to_ly: DVec3::from_array(*to_ly),
                    start_s: *start_s,
                    drive: drive_in(*drive),
                    arrive_at: arrive_at.as_ref().map(waypoint_in),
                    clock_base_s: *clock_base_s,
                },
                lc_proto::Motive::Rendezvous {
                    from_ly,
                    beta0,
                    to_ly,
                    start_s,
                    drive,
                    frame_from_ly,
                    frame_beta,
                    since_t,
                    target,
                    clock_base_s,
                } => Recipe::Rendezvous {
                    approach: crate::pursuit::Approach {
                        from_ly: DVec3::from_array(*from_ly),
                        beta0: DVec3::from_array(*beta0),
                        to_ly: DVec3::from_array(*to_ly),
                        start_s: *start_s,
                        drive: drive_in(*drive),
                        frame_from_ly: DVec3::from_array(*frame_from_ly),
                        frame_beta: DVec3::from_array(*frame_beta),
                        since_t: *since_t,
                        target: crate::motion::ShipId(target.0),
                    },
                    clock_base_s: *clock_base_s,
                },
                lc_proto::Motive::Holding(waypoint) => Recipe::Holding(waypoint_in(waypoint)),
                lc_proto::Motive::Falling => Recipe::Falling,
                lc_proto::Motive::Drifting { from_ly, since_t } => Recipe::Drifting {
                    from_ly: DVec3::from_array(*from_ly),
                    since_t: *since_t,
                },
            },
        }
    }
}

fn drive_out(drive: Drive) -> lc_proto::Drive {
    lc_proto::Drive { accel_g: drive.accel_g, max_beta: drive.max_beta }
}

fn drive_in(drive: lc_proto::Drive) -> Drive {
    Drive { accel_g: drive.accel_g, max_beta: drive.max_beta }
}

fn waypoint_out(waypoint: &Waypoint) -> lc_proto::Waypoint {
    match waypoint {
        Waypoint::Fixed(at) => lc_proto::Waypoint::Fixed(at.to_array()),
        Waypoint::Orbit(orbit) => lc_proto::Waypoint::Orbit {
            about: match &orbit.about {
                Anchor::Star => lc_proto::Anchor::Star,
                Anchor::Body(name) => lc_proto::Anchor::Body(name.clone()),
            },
            radius_m: orbit.radius_m,
            pole: orbit.pole.to_array(),
            phase_rad: orbit.phase_rad,
        },
        Waypoint::Lagrange { body, point } => {
            lc_proto::Waypoint::Lagrange { body: body.clone(), point: point_out(*point) }
        }
        Waypoint::Libration(libration) => lc_proto::Waypoint::Libration {
            body: libration.body.clone(),
            point: point_out(libration.point),
            standoff_m: libration.standoff_m,
            radial_m: libration.radial_m,
            vertical_m: libration.vertical_m,
            planar_rate: libration.planar_rate,
            vertical_rate: libration.vertical_rate,
            amplitude_ratio: libration.amplitude_ratio,
            phase_rad: libration.phase_rad,
            vertical_phase_rad: libration.vertical_phase_rad,
            epoch_s: libration.epoch_s,
        },
    }
}

fn waypoint_in(waypoint: &lc_proto::Waypoint) -> Waypoint {
    match waypoint {
        lc_proto::Waypoint::Fixed(at) => Waypoint::Fixed(DVec3::from_array(*at)),
        lc_proto::Waypoint::Orbit { about, radius_m, pole, phase_rad } => Waypoint::Orbit(Orbit {
            about: match about {
                lc_proto::Anchor::Star => Anchor::Star,
                lc_proto::Anchor::Body(name) => Anchor::Body(name.clone()),
            },
            radius_m: *radius_m,
            pole: DVec3::from_array(*pole),
            phase_rad: *phase_rad,
        }),
        lc_proto::Waypoint::Lagrange { body, point } => {
            Waypoint::Lagrange { body: body.clone(), point: point_in(*point) }
        }
        lc_proto::Waypoint::Libration {
            body,
            point,
            standoff_m,
            radial_m,
            vertical_m,
            planar_rate,
            vertical_rate,
            amplitude_ratio,
            phase_rad,
            vertical_phase_rad,
            epoch_s,
        } => Waypoint::Libration(crate::libration::Libration {
            body: body.clone(),
            point: point_in(*point),
            standoff_m: *standoff_m,
            radial_m: *radial_m,
            vertical_m: *vertical_m,
            planar_rate: *planar_rate,
            vertical_rate: *vertical_rate,
            amplitude_ratio: *amplitude_ratio,
            phase_rad: *phase_rad,
            vertical_phase_rad: *vertical_phase_rad,
            epoch_s: *epoch_s,
        }),
    }
}

fn point_out(point: LagrangePoint) -> lc_proto::LagrangePoint {
    match point {
        LagrangePoint::L1 => lc_proto::LagrangePoint::L1,
        LagrangePoint::L2 => lc_proto::LagrangePoint::L2,
    }
}

fn point_in(point: lc_proto::LagrangePoint) -> LagrangePoint {
    match point {
        lc_proto::LagrangePoint::L1 => LagrangePoint::L1,
        lc_proto::LagrangePoint::L2 => LagrangePoint::L2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{Change, Event, Motive, ShipId, advance, apply};
    use crate::navigation::{Course, Plane};
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
            course: Course::Orbit { body: body.into(), altitude_radii: 2.0, plane: Plane::Equatorial },
            drive: crate::flight::Drive::DEFAULT,
        }
    }

    /// Snapshot, across the wire and back, and restored. The whole path a reconnect takes.
    fn round_trip(state: &ShipState, system: Option<&LocalSystem>, now_s: f64) -> ShipState {
        let sent: lc_proto::Motion = (&state.snapshot()).into();
        let bytes = lc_proto::encode(&sent);
        let read: lc_proto::Motion = lc_proto::decode(&bytes).expect("it decodes");
        assert_eq!(read, sent, "the wire changed it");
        Snapshot::from(&read).restore(system, now_s)
    }

    /// Run a ship to `until_s` in `step` increments, as a client does.
    fn run(state: &mut ShipState, system: &mut LocalSystem, from_s: f64, until_s: f64, step: f64) {
        let mut now = from_s;
        while now < until_s {
            let next = (now + step).min(until_s);
            let elapsed = next - now;
            now = next;
            system.advance_to(now);
            advance(state, Some(system), now, elapsed);
        }
    }

    /// Fly a ship to a station and leave it holding there.
    fn holding_earth() -> Option<(LocalSystem, ShipState)> {
        let mut system = sol()?;
        let mut ship = ShipState::at(DVec3::ZERO);
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: 0.0,
            change: orbit("Earth"),
        })
        .expect("a course");
        let arrival = match &ship.motive {
            Motive::Crossing(cruise) => cruise.duration_s(),
            _ => unreachable!("a course is a crossing"),
        };
        run(&mut ship, &mut system, 0.0, arrival + 1.0, 438.0);
        assert!(matches!(ship.motive, Motive::Holding(_)), "it should have arrived: {:?}", ship.motive);
        Some((system, ship))
    }

    /// The bug, stated plainly. A welcome that carried a position put this ship back at rest.
    #[test]
    fn a_ship_holding_an_orbit_comes_back_holding_it() {
        let Some((system, ship)) = holding_earth() else { return };
        let there = round_trip(&ship, Some(&system), system.time_s());
        assert_eq!(ship.motive, there.motive, "it came back doing something else");
        assert!(matches!(there.motive, Motive::Holding(Waypoint::Orbit(_))));
    }

    /// And the claim that actually matters: it is in the same place a day later, exactly.
    ///
    /// A station is a position in a system, so a ship restored into one and a ship that never
    /// left agree by running the same code over the same waypoint. Exact, not near: this is the
    /// determinism `lightcone/docs/17-reconciliation.md` rests on.
    #[test]
    fn a_restored_station_is_in_the_same_place_a_day_later() {
        let Some((mut system, mut ship)) = holding_earth() else { return };
        let now = system.time_s();
        let mut there = round_trip(&ship, Some(&system), now);

        let mut other = system.clone();
        let until = now + 86_400.0;
        run(&mut ship, &mut system, now, until, 438.0);
        run(&mut there, &mut other, now, until, 438.0);
        assert_eq!(ship.position_ly, there.position_ly, "the restored ship is somewhere else");
        assert_eq!(ship.beta, there.beta);
    }

    /// A crossing is resumed **part-way through**, from the parameters rather than the path.
    ///
    /// Two things have to survive: the velocity it was planned from, or the re-plan is a
    /// different flight; and the clock it began at, or the crew's own time jumps.
    #[test]
    fn a_crossing_is_resumed_mid_burn_with_the_crews_clock_intact() {
        let Some(mut system) = sol() else { return };
        let mut ship = ShipState::at(DVec3::ZERO);
        // Under way first, so the crossing is planned from a velocity and not from rest.
        ship.beta = DVec3::new(0.0, 0.002, 0.0);
        ship.set_adrift(0.0);
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: 10.0,
            change: orbit("Earth"),
        })
        .expect("a course");
        let Motive::Crossing(cruise) = &ship.motive else { unreachable!() };
        assert!(cruise.initial_beta().length() > 0.0, "the plan should carry the velocity");
        let half = cruise.start_s + cruise.duration_s() * 0.5;

        run(&mut ship, &mut system, 10.0, half, 438.0);
        assert!(matches!(ship.motive, Motive::Crossing(_)), "still under way");
        let mut there = round_trip(&ship, Some(&system), half);
        assert_eq!(ship.motive, there.motive, "the re-plan is a different crossing");
        assert_eq!(ship.clock_s, there.clock_s, "the crew's clock moved");

        let mut other = system.clone();
        let until = half + 5_000.0;
        run(&mut ship, &mut system, half, until, 438.0);
        run(&mut there, &mut other, half, until, 438.0);
        assert_eq!(ship.position_ly, there.position_ly, "the crossings diverged");
        assert_eq!(ship.clock_s, there.clock_s, "the crews aged differently");
    }

    /// A conic carries nothing on the wire and is re-solved from the state beside it.
    ///
    /// Near rather than exact: re-solving passes through `atan2` and a Kepler iteration, so the
    /// two arcs agree to the precision the fold does and not to the bit. That is divergence
    /// class 3 of `lightcone/docs/17-reconciliation.md` and it is the price of not putting a
    /// solved orbit on the wire.
    #[test]
    fn a_ship_falling_on_a_conic_comes_back_on_the_same_conic() {
        let Some((mut system, mut ship)) = holding_earth() else { return };
        let now = system.time_s();
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: now,
            change: Change::CutDrive,
        })
        .expect("the engine cuts");
        let Motive::Falling(before) = &ship.motive else {
            unreachable!("cutting in orbit is a conic, not {:?}", ship.motive)
        };
        let primary = before.primary.clone();

        let mut there = round_trip(&ship, Some(&system), now);
        let Motive::Falling(after) = &there.motive else {
            unreachable!("it came back {:?}", there.motive)
        };
        assert_eq!(after.primary, primary, "it is falling round something else");

        let mut other = system.clone();
        let until = now + 3_600.0;
        run(&mut ship, &mut system, now, until, 438.0);
        run(&mut there, &mut other, now, until, 438.0);
        // A metre, against an orbit twelve thousand kilometres across.
        let apart = (ship.position_ly - there.position_ly).length() * crate::system::M_PER_LY;
        assert!(apart < 1.0, "the arcs are {apart} metres apart after an hour");
    }

    /// With no system there is nothing for a conic to be about, so it degrades to a line —
    /// the same answer `ShipState::leave_system` gives, rather than a motive about bodies that
    /// are not there.
    #[test]
    fn a_conic_with_no_system_comes_back_as_a_drift() {
        let Some((system, ship)) = holding_earth() else { return };
        let mut ship = ship;
        let now = system.time_s();
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: now,
            change: Change::CutDrive,
        })
        .expect("the engine cuts");
        let adrift = round_trip(&ship, None, now);
        assert!(matches!(adrift.motive, Motive::Drifting { .. }), "{:?}", adrift.motive);
        assert_eq!(adrift.position_ly, ship.position_ly, "it should still be where it was");
        assert_eq!(adrift.beta, ship.beta, "and still moving");
    }

    /// A drift is restored to the line it was already on, not to a new one starting here.
    ///
    /// Both forms put the ship in the same place now; they are read from different origins, and
    /// two sides reading from different origins is a disagreement that never closes.
    #[test]
    fn a_drift_keeps_the_line_it_was_already_on() {
        let mut ship = ShipState::at(DVec3::new(1.5, 0.0, 0.0));
        ship.beta = DVec3::new(0.0, 0.3, 0.0);
        ship.resume_drifting(DVec3::new(1.5, 0.0, 0.0), 0.0);
        advance(&mut ship, None, 4.0e7, 4.0e7);

        let there = round_trip(&ship, None, 4.0e7);
        assert_eq!(ship.motive, there.motive, "the line was redrawn from somewhere else");
        match there.motive {
            Motive::Drifting { from_ly, since_t } => {
                assert_eq!(from_ly, DVec3::new(1.5, 0.0, 0.0));
                assert_eq!(since_t, 0.0, "it should still start when it started");
            }
            other => unreachable!("{other:?}"),
        }
    }

    /// Re-planning from a crossing's own recipe gives that crossing back, including the case
    /// where the plan had to move the target because the ship could not stop in time.
    #[test]
    fn re_planning_a_crossing_from_its_own_recipe_is_the_same_crossing() {
        let drive = crate::flight::Drive::DEFAULT;
        for beta0 in [DVec3::ZERO, DVec3::new(0.0, 0.4, 0.0), DVec3::new(-0.9, 0.0, 0.0)] {
            let first = Cruise::plan_from(DVec3::ZERO, beta0, DVec3::new(0.1, 0.0, 0.0), 7.0, drive);
            let again =
                Cruise::plan_from(first.from_ly, first.initial_beta(), first.to_ly, first.start_s, drive);
            assert_eq!(first, again, "re-planning changed the crossing, from {beta0}");
        }
    }
}
