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
use em_foundations::time::{Instant, TimeDelta};
use lc_spacetime::Worldline;

/// A ship, by the identifier whoever owns it uses. Opaque here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShipId(pub i64);

/// How a ship is moving. The five are exclusive, and that exclusivity is the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Motive {
    /// Under thrust, on a planned crossing.
    Crossing(Cruise),
    /// Under thrust, closing on another craft and matching its velocity.
    ///
    /// A crossing in a moving frame, and it is a motive of its own rather than a `Crossing`
    /// because the frame is part of the answer: the plan ends at rest in the *quarry's* frame,
    /// which is what "matched" means, and reading it in the world's needs the frame back. See
    /// [`crate::pursuit`], which also says why the frame is a frozen sighting and not a handle
    /// on the quarry's live worldline.
    Rendezvous(crate::pursuit::Rendezvous),
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
        matches!(self.motive, Motive::Crossing(_) | Motive::Rendezvous(_))
    }

    /// Put a ship on an approach solved for it. The counterpart of [`Self::begin_crossing`],
    /// and it bases the crew's clock the same way.
    pub fn begin_rendezvous(&mut self, plan: crate::pursuit::Rendezvous) {
        self.crossing_clock_base_s = self.clock_s;
        self.motive = Motive::Rendezvous(plan);
        self.arrive_at = None;
    }

    /// Put one back part-way through, keeping the clock base it began with.
    pub fn resume_rendezvous(&mut self, plan: crate::pursuit::Rendezvous, clock_base_s: f64) {
        self.motive = Motive::Rendezvous(plan);
        self.arrive_at = None;
        self.crossing_clock_base_s = clock_base_s;
    }

    /// Whether two states describe the *same worldline*, rather than the same ship.
    ///
    /// The motive, and the velocity as well when the motive is a drift — because that is the
    /// one arm of [`state_at`] that reads a field outside the motive, and a ship that changes
    /// velocity while still drifting is on a different line through spacetime even though its
    /// motive has not changed shape.
    ///
    /// Deliberately *not* the whole state. Position, velocity and the crew's clock all move on
    /// every step of a crossing without the worldline changing at all, and a craft recording a
    /// stretch of history per step would remember only the last few seconds of its life.
    ///
    /// This lives beside [`state_at`] because it is a statement about what that function reads.
    /// The two are wrong together or right together, and apart they would rot.
    pub fn same_worldline_as(&self, other: &ShipState) -> bool {
        if self.motive != other.motive {
            return false;
        }
        match (&self.motive, &other.motive) {
            (Motive::Drifting { .. }, Motive::Drifting { .. }) => self.beta == other.beta,
            _ => true,
        }
    }

    /// Who this ship is closing on, if it is closing on anybody.
    pub fn pursuing(&self) -> Option<ShipId> {
        match &self.motive {
            Motive::Rendezvous(plan) => Some(plan.target),
            _ => None,
        }
    }

    /// Where the crossing under way is for, if it is for anywhere.
    pub fn bound_for(&self) -> Option<&Waypoint> {
        self.arrive_at.as_ref()
    }

    /// Put a ship on a crossing directly, for a caller that has already planned one.
    ///
    /// [`apply`] is the way in for anything a client and a server both have to agree about.
    /// This is for the crossing between stars, which has no course to resolve and no system to
    /// resolve it against.
    pub fn begin_crossing(&mut self, cruise: Cruise, arrive_at: Option<Waypoint>) {
        self.crossing_clock_base_s = self.clock_s;
        self.motive = Motive::Crossing(cruise);
        self.arrive_at = arrive_at;
    }

    /// Hold a place directly, for a caller that put the ship there rather than flying it.
    pub fn begin_holding(&mut self, waypoint: Waypoint) {
        self.motive = Motive::Holding(waypoint);
        self.arrive_at = None;
        self.beta = DVec3::ZERO;
    }

    /// Everything needed to put this ship back, as parameters. See [`crate::resume`].
    pub fn snapshot(&self) -> crate::resume::Snapshot {
        use crate::resume::{Recipe, Snapshot};
        Snapshot {
            position_ly: self.position_ly,
            beta: self.beta,
            clock_s: self.clock_s,
            drive: self.drive,
            motive: match &self.motive {
                Motive::Crossing(cruise) => Recipe::Crossing {
                    from_ly: cruise.from_ly,
                    beta0: cruise.initial_beta(),
                    to_ly: cruise.to_ly,
                    start_s: cruise.start_s,
                    drive: cruise.drive,
                    arrive_at: self.arrive_at.clone(),
                    clock_base_s: self.crossing_clock_base_s,
                },
                Motive::Rendezvous(plan) => Recipe::Rendezvous {
                    approach: plan.recipe(),
                    clock_base_s: self.crossing_clock_base_s,
                },
                Motive::Holding(waypoint) => Recipe::Holding(waypoint.clone()),
                Motive::Falling(_) => Recipe::Falling,
                Motive::Drifting { from_ly, since_t } => {
                    Recipe::Drifting { from_ly: *from_ly, since_t: *since_t }
                }
            },
        }
    }

    /// Put a crossing back **part-way through**, which is what [`Self::begin_crossing`] cannot do.
    ///
    /// The difference is the clock: beginning a crossing bases the crew's time on the clock as
    /// it stands, and resuming one has to restore the base it was given when it actually began,
    /// or the ship's own time jumps on the next step.
    pub fn resume_crossing(
        &mut self,
        cruise: Cruise,
        arrive_at: Option<Waypoint>,
        clock_base_s: f64,
    ) {
        self.motive = Motive::Crossing(cruise);
        self.arrive_at = arrive_at;
        self.crossing_clock_base_s = clock_base_s;
    }

    /// Put a ship back on a conic that was solved for it. See [`crate::resume`].
    pub fn resume_falling(&mut self, coast: Coast) {
        self.motive = Motive::Falling(coast);
        self.arrive_at = None;
    }

    /// Put a ship back on the line it was already on, rather than starting a new one here.
    ///
    /// [`Self::set_adrift`] begins a line at the ship's present position; this restores one
    /// that began elsewhere. Reading a drift from its own origin rather than integrating is
    /// what makes two sides stepping differently agree, so a restore that started the line
    /// afresh would be a small, permanent disagreement.
    pub fn resume_drifting(&mut self, from_ly: DVec3, since_t: f64) {
        self.motive = Motive::Drifting { from_ly, since_t };
        self.arrive_at = None;
    }

    /// Stop holding and stop falling: whatever it has, in a straight line from here.
    pub fn set_adrift(&mut self, now_s: f64) {
        self.motive = Motive::Drifting { from_ly: self.position_ly, since_t: now_s };
        self.arrive_at = None;
    }

    /// The ship has left the system its motive was defined against.
    ///
    /// A station and a conic are positions relative to bodies that are no longer there, so they
    /// go. A **crossing does not**: it is a straight line between two points of the world, and
    /// leaving a system is exactly what one is for. Dropping it here cancelled every
    /// interstellar flight at the moment it cleared the shell, and the ship then coasted the
    /// rest of the way with its clock running at the coordinate rate.
    pub fn leave_system(&mut self, now_s: f64) {
        match self.motive {
            Motive::Crossing(_) => {
                // The place it was flying to is gone even though the flight is not.
                self.arrive_at = None;
            }
            // An approach is defined against another craft and not against the system, so
            // leaving one takes nothing from it. Whether the quarry is still in sight is the
            // pursuit's business, not the system's.
            Motive::Rendezvous(_) => {}
            _ => self.set_adrift(now_s),
        }
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
    /// Cross to another star.
    ///
    /// The one course that is not about the local system, which is why it is not a
    /// [`Change::SetCourse`]: that one resolves a waypoint *inside* a system and refuses
    /// without one, and this is the thing a ship does when leaving.
    ///
    /// A **position**, not a star id, because this fold is pure motion and has no catalogue to
    /// look one up in. Naming the star is the wire's job — see `lc_proto::Order::Cross` — and
    /// the authority resolves it before folding, so both sides fold the same coordinate.
    Cross { to_ly: DVec3, drive: Drive },
    /// Put out a pulse. Changes nothing about the motion, and is here because the fold is what
    /// both sides run over everything that happened.
    Transmit { power_w: f64 },
    /// The ballistic arc crosses a sphere of influence and is re-solved about `about`.
    ///
    /// An event rather than something each side notices for itself. Patched conics done by
    /// detection depend on *when* the crossing is looked for, so two sides stepping differently
    /// would produce different elements and drift apart. The authority decides when it happened
    /// and everyone folds the same instant. Doc 08 says the same thing about shards: a shell
    /// crossing is already an event with a coordinate.
    ///
    /// The new primary is named rather than looked up. At a join the ship is *exactly* on a
    /// boundary, so asking which sphere contains it is a coin toss — and it came up "the one
    /// you are leaving", which re-solved the same arc and left the ship inside Earth for good.
    /// Naming it is still an order and not a trajectory: both sides solve the same conic from
    /// the same state, they are only told which frame to solve it in.
    Repatch { about: String },
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
    /// A crossing to somewhere the ship is already at least as close to as it would end up.
    ///
    /// A crossing stops [`crate::flight::STANDOFF_LY`] short of a star, which is about sixty
    /// astronomical units — so a ship already inside a system is nearer than the crossing would
    /// leave it, and flying it would carry the ship *outward* to arrive at its own star.
    AlreadyThere,
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
        // Both of these re-solve the ship's conic from its state at the event's own time, and
        // both read that state rather than taking the ship's last advanced position. An event
        // is stamped with a coordinate and may be folded later than it happened -- a predicted
        // patch always is -- and taking the position from one time and the velocity from
        // another produces an orbit the ship was never on.
        Change::Repatch { .. } | Change::CutDrive => {
            let (at, beta) = state_at(state, system, event.at_t)
                .unwrap_or((state.position_ly, state.beta));
            let velocity = beta * crate::flight::C_M_S;
            state.position_ly = at;
            state.beta = beta;
            let solved = system.and_then(|s| match &event.change {
                Change::Repatch { about } => {
                    Coast::about(s, s.body_named(about)?, at, velocity, event.at_t)
                }
                _ => Coast::from_state(s, at, velocity, event.at_t),
            });
            state.motive = match solved {
                Some(arc) => Motive::Falling(arc),
                // Between systems, or a radial state that has no conic at all. Either way it is
                // a straight line at the velocity it has.
                None => Motive::Drifting { from_ly: at, since_t: event.at_t },
            };
            Ok(())
        }
        Change::Cross { to_ly, drive } => {
            // From where it actually is at the stamped time, like every other arm: an event
            // may be folded later than it happened, and planning from the ship's last advanced
            // position would aim from somewhere it was not.
            // The velocity matters: a crossing ordered from an orbit, or from a coast, starts
            // with whatever the ship already has. Discarding it put the ship back at rest and
            // read in the interface as the speed dropping to 0.00c the moment a destination
            // was chosen. See `Cruise::plan_from`.
            let (at, beta) =
                state_at(state, system, event.at_t).unwrap_or((state.position_ly, state.beta));
            let reach = *to_ly - at;
            let approach = reach.normalize_or_zero();
            // Already there, or asked to cross to where it stands. Neither is a crossing.
            if approach == DVec3::ZERO {
                return Err(Rejected::NoSuchPlace);
            }
            // Inside the standoff already: the crossing would end further out than the ship
            // began, so "go to that star" would carry it away from the star.
            if reach.length() <= crate::flight::STANDOFF_LY {
                return Err(Rejected::AlreadyThere);
            }
            // Stopping short, because arriving *at* a star is arriving inside it.
            let stop = *to_ly - approach * crate::flight::STANDOFF_LY;
            state.position_ly = at;
            state.beta = beta;
            state.drive = *drive;
            state.begin_crossing(
                crate::flight::Cruise::plan_from(at, beta, stop, event.at_t, *drive),
                None,
            );
            Ok(())
        }
        Change::SetCourse { course, drive } => {
            let system = system.ok_or(Rejected::NotInASystem)?;
            // Where and how fast it actually is at the stamped time, like every other arm: an
            // event may be folded later than it happened, and a course planned from last
            // frame's position — or from rest, when the ship is not at rest — is a course to
            // somewhere the ship is not.
            let (at, beta) = state_at(state, Some(system), event.at_t)
                .unwrap_or((state.position_ly, state.beta));
            let waypoint = course.resolve(system, at, event.at_t).ok_or(Rejected::NoSuchPlace)?;
            let (cruise, aimed) =
                crate::navigation::plan(system, &waypoint, at, beta, event.at_t, *drive)
                    .ok_or(Rejected::NoSuchPlace)?;
            state.position_ly = at;
            state.beta = beta;
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

/// Where a ship is and how fast, at any coordinate time, without changing anything.
///
/// **A motive is a worldline.** All four arms are closed forms in `t`: a crossing carries its
/// own, a station and a conic are defined against bodies that `LocalSystem` places
/// analytically, and a drift is a line read from where it began. So a ship can be asked where
/// it will be at a time that has not happened yet, which is what a light-delay solve needs —
/// the root it is looking for *is* a future time, and a model that could only answer for the
/// present would have to be extrapolated to get there.
///
/// `None` when the motive cannot be evaluated: the body it is defined against has gone, or the
/// conic's chain is integrated rather than analytic. A caller that already has a position
/// should keep it rather than invent one.
pub fn state_at(
    state: &ShipState,
    system: Option<&LocalSystem>,
    now_s: f64,
) -> Option<(DVec3, DVec3)> {
    match &state.motive {
        Motive::Crossing(cruise) => {
            let flight = cruise.at(now_s);
            Some((flight.position_ly, flight.beta))
        }
        // The plan is relative, so the frame has to be added back. Galilean, and
        // [`crate::pursuit`] carries the bound on that.
        Motive::Rendezvous(plan) => Some(plan.state_at(now_s)),
        Motive::Holding(waypoint) => {
            let system = system?;
            let at = waypoint.place_at(system, now_s)?;
            let velocity = waypoint.velocity_at(system, now_s).unwrap_or(DVec3::ZERO);
            Some((at, coast::beta_of(velocity)))
        }
        Motive::Falling(arc) => {
            let (at, velocity) = arc.at(system?, now_s)?;
            Some((at, coast::beta_of(velocity)))
        }
        // A light-year is a year of travel at `c` by definition, so a beta is already
        // light-years per year -- and it is read from the start of the line rather than
        // accumulated, so any two step sizes land on the same place.
        Motive::Drifting { from_ly, since_t } => {
            Some((*from_ly + state.beta * (now_s - since_t) / JULIAN_YEAR_S, state.beta))
        }
    }
}

/// Which way the hull's nose points at a coordinate time, if anything decides it.
///
/// Thrust first, velocity second. A ship under way points along its drive — which is *back*
/// down its own track through a brake — and a ship with the engine off points along its
/// motion. A craft at rest with nothing burning has no attitude this can derive, and `None`
/// says so rather than inventing one; the renderer holds whatever it last had.
///
/// Proper acceleration, so a ballistic arc counts as unpowered. Falling is not thrust, and a
/// nose that followed the coordinate acceleration would point at the primary all the way round
/// an orbit.
pub fn facing(state: &ShipState, system: Option<&LocalSystem>, now_s: f64) -> Option<DVec3> {
    // The frame does not rotate, so a thrust direction in it is a thrust direction here.
    let thrusting = match &state.motive {
        Motive::Crossing(cruise) => Some(cruise.thrust_at(now_s)),
        // The plan and not its cruise: the cruise keeps the quarry frame's own time, and its
        // thrust direction is in that frame's axes. Both have to come back.
        Motive::Rendezvous(plan) => Some(plan.thrust_at(now_s)),
        _ => None,
    };
    if let Some(thrust) = thrusting
        && thrust != DVec3::ZERO
    {
        return Some(thrust.normalize());
    }
    let beta = state_at(state, system, now_s).map(|(_, beta)| beta).unwrap_or(state.beta);
    let along = beta.normalize_or_zero();
    (along != DVec3::ZERO).then_some(along)
}

/// Move a ship to a coordinate time.
///
/// Read at the new time rather than integrated from the old one, in every branch — that is
/// [`state_at`]'s job and this calls it, so a ship that is advanced and a ship that is merely
/// asked cannot end up in different places. A paused clock, a clock at a year a second and a
/// dropped frame all leave the ship the same, which is what lets a client at one frame rate and
/// a server at another agree.
///
/// What is left here is the part that is not a worldline: the proper-time clock, and the
/// transition a crossing makes when it ends.
pub fn advance(state: &mut ShipState, system: Option<&LocalSystem>, now_s: f64, elapsed_s: f64) {
    if let Some((at, beta)) = state_at(state, system, now_s) {
        state.position_ly = at;
        state.beta = beta;
    }
    match &state.motive {
        Motive::Crossing(cruise) => {
            let flight = cruise.at(now_s);
            // Read rather than integrated. Stepping `elapsed / gamma` uses one velocity for a
            // whole interval the velocity changed across, which is wrong by first order
            // everywhere and wrong by the entire last step at arrival, where the ship has
            // already stopped. The crossing carries the closed form; use it.
            state.clock_s = state.crossing_clock_base_s + flight.proper_s;
            if flight.phase == Phase::Arrived {
                state.beta = DVec3::ZERO;
                state.motive = match state.arrive_at.take() {
                    Some(waypoint) => {
                        // Placed on it at once, not next step. The crossing ends where the
                        // station *was* when the plan was made, and a step that overshoots the
                        // arrival by a little leaves the body a little further round its year:
                        // holding from the following step would show as a jump.
                        if let Some(at) = system.and_then(|s| waypoint.place_at(s, now_s)) {
                            state.position_ly = at;
                        }
                        Motive::Holding(waypoint)
                    }
                    None => Motive::Drifting { from_ly: state.position_ly, since_t: now_s },
                };
            }
        }
        Motive::Rendezvous(plan) => {
            // Sampled at a world time through the plan, which is what reconciles it with the
            // quarry frame's own clock. Reading the cruise directly asked it about a moment it
            // measures differently, and at speed those are months apart.
            let flight = plan.flight_at(now_s);
            state.clock_s = state.crossing_clock_base_s + flight.proper_s;
            if flight.phase == Phase::Arrived {
                // Arriving is not stopping. What is left is the quarry's own velocity, which
                // is the whole point of having planned in its frame — so the ship comes off
                // the approach *already* alongside and moving with it, and goes ballistic from
                // there by the same route cutting the drive takes.
                let (at, beta) = plan.state_at(now_s);
                state.position_ly = at;
                state.beta = beta;
                let velocity = beta * crate::flight::C_M_S;
                state.motive = match system
                    .and_then(|s| Coast::from_state(s, at, velocity, now_s))
                {
                    Some(arc) => Motive::Falling(arc),
                    None => Motive::Drifting { from_ly: at, since_t: now_s },
                };
            }
        }
        Motive::Falling(_) if system.is_none() => {
            // The system is gone, which means the ship has left it. Whatever the conic said,
            // out here it is a straight line.
            state.clock_s += elapsed_s;
            state.motive = Motive::Drifting { from_ly: state.position_ly, since_t: now_s };
        }
        _ => state.clock_s += elapsed_s,
    }
}

/// How far ahead of an arc to look for its next patch, in revolutions.
///
/// Three rather than one because an orbit can sit wholly inside a sphere for a revolution and
/// still meet a moon on the next. The same number [`em_sim::crossing::default_horizon_of`]
/// uses, and for the same reason.
pub const PATCH_HORIZON_REVOLUTIONS: f64 = 3.0;

/// The horizon for an arc with no revolutions to count: a hyperbola leaves, and the question
/// is only when.
pub const OPEN_PATCH_HORIZON_S: f64 = JULIAN_YEAR_S;

/// **When** the ballistic arc will leave the influence it was solved in, and the event that
/// says so.
///
/// Solved, not noticed. A patch found by looking is a patch found at whatever moment someone
/// happened to look, and at any real time warp a ship on a hyperbola can pass clean through a
/// small moon's sphere between two looks and never patch at all. This roots the boundary
/// distance in `t` instead, so the answer is a property of the arc.
///
/// Costs a search — a few hundred evaluations per candidate sphere — so the authority is meant
/// to call it once when an arc begins and remember the answer, not once a frame.
///
/// `None` when the arc meets nothing within the horizon, which is the ordinary case for an
/// orbit that stays where it is.
pub fn repatch_at(state: &ShipState, system: &LocalSystem, from_s: f64) -> Option<Event> {
    let Motive::Falling(arc) = &state.motive else { return None };
    let primary = system.body_named(&arc.primary)?;
    let candidates = em_sim::influence::spheres_within(system.sim(), primary);
    if candidates.is_empty() {
        return None;
    }

    let horizon = match arc.period_s() {
        Some(period) => period * PATCH_HORIZON_REVOLUTIONS,
        None => OPEN_PATCH_HORIZON_S,
    };
    // Off the boundary the arc may be sitting exactly on, having just been solved there. A
    // search from the join itself finds that same crossing and the walk never advances; the
    // tolerance is a millisecond, so a second is far clear of it and far inside any arc.
    const CLEARANCE_S: f64 = 1.0;
    let from = Instant::from_seconds_since_j2000(from_s + CLEARANCE_S);

    let found = em_sim::crossing::first_crossing_of(
        system.sim(),
        &arc.path(system),
        &candidates,
        from,
        TimeDelta::from_seconds(horizon),
    )?;

    // Which frame the ship lands in, from the crossing rather than from a containment test.
    // Leaving the sphere it is in hands it up to the next primary out; entering a sibling's
    // hands it down to that sibling. The other two cases -- entering the sphere you are
    // already inside, leaving one you are not in -- cannot happen from a consistent state,
    // and are not guessed at.
    let about = if found.body == primary {
        if found.entering {
            return None;
        }
        // The star's influence has no outer edge here; there is nowhere further out to go.
        system.sim().parent(primary)?
    } else if found.entering {
        found.body
    } else {
        return None;
    };

    Some(Event {
        ship: ShipId(0),
        at_t: found.time.to_j2000_seconds(),
        change: Change::Repatch { about: system.sim().name(about).to_string() },
    })
}

/// Whether the arc is *already* in the wrong sphere, and the event that says so.
///
/// The backstop to [`repatch_at`], not a substitute for it. A prediction is made when an arc
/// begins and can be defeated by anything that happens afterwards; this catches a ship that
/// has ended up somewhere the prediction did not cover. It fires at `now_s`, so it is the
/// authority's clock that stamps it — everyone else folds that stamp.
pub fn repatch_due(state: &ShipState, system: &LocalSystem, now_s: f64) -> Option<Event> {
    let Motive::Falling(arc) = &state.motive else { return None };
    let velocity = state.beta * crate::flight::C_M_S;
    let found = arc.repatched(system, state.position_ly, velocity, now_s)?;
    Some(Event { ship: ShipId(0), at_t: now_s, change: Change::Repatch { about: found.primary } })
}

/// Light-microseconds in a light-year.
///
/// A light-year is a Julian year of travel at `c` and a light-microsecond is a microsecond of
/// it, so the conversion is the year in microseconds and nothing else. `lc-spacetime` measures
/// in light-microseconds because that is the unit light delay is naturally counted in; the world
/// model measures in light-years because that is the unit a galaxy is. This is the join.
pub const LIGHT_US_PER_LY: f64 = JULIAN_YEAR_S * 1.0e6;

/// A stretch of a worldline that is over: what a ship was doing, and when it stopped.
///
/// See [`Flight`] for why a craft keeps these at all.
#[derive(Clone, Debug, PartialEq)]
pub struct Past {
    /// Coordinate seconds at which this stopped being in force.
    pub until_s: f64,
    pub motion: ShipState,
}

/// A ship's worldline, for the light-delay solve.
///
/// Borrowed rather than owned: the motive and the system are the truth, and a copy of them in
/// another shape is a copy that can be stale. Holding the two together is what makes the
/// worldline total — a station and a conic mean nothing without the bodies they are defined
/// against.
///
/// **A worldline has a past, and that is not decoration.** A motive is a closed form total in
/// `t`, so evaluating the *current* one at an earlier time answers about a ship that did not
/// exist yet: `Motive::Drifting` extrapolates backwards, so a burn would retroactively rewrite
/// where the ship was an hour ago and how fast. Every retarded solve then reads the new motion
/// at the old time — which is a client watching a manoeuvre the instant it happens, at any
/// range, and the end of the game this is all built to be.
///
/// So a craft keeps the motives it has flown, stamped with when each stopped, and this picks
/// the one that was in force. The same shape `crate::observation::Target` uses for emission
/// models, for the same reason: an event does not alter the past, it appends.
///
/// The memory is bounded, so the past runs out. [`Flight::defined_over`] says where, and a
/// solve that falls off the end returns nothing at all rather than a guess — not seeing
/// something is always safe, and inventing where it was is not.
///
/// The frame is `lc-spacetime`'s: light-microseconds from the world origin, and coordinate
/// microseconds. Note the precision this costs at galactic distances — a position is a `f64`
/// count of light-microseconds, so a ship in a system a hundred light-years out is at `3e15`
/// and resolves to about a hundred metres. Fine for a light-delay solve, useless for an orbit,
/// and the reason doc 08 shards the frame rather than keeping one origin for everything.
pub struct Flight<'a> {
    state: &'a ShipState,
    system: Option<&'a LocalSystem>,
    /// Oldest first, and each one's `until_s` later than the last.
    past: &'a [Past],
    /// The earliest coordinate second this can answer for. `-inf` when nothing has been
    /// forgotten, which is the case for a craft that has never changed what it was doing.
    known_from_s: f64,
}

impl<'a> Flight<'a> {
    /// A worldline with no past: whatever it is doing now, it has always been doing.
    ///
    /// True only of a craft that has never changed its motive. Anything the world has run is
    /// built by [`crate::craft::Craft::worldline`], which carries the real history.
    pub fn new(state: &'a ShipState, system: Option<&'a LocalSystem>) -> Self {
        Self { state, system, past: &[], known_from_s: f64::NEG_INFINITY }
    }

    pub fn with_past(
        state: &'a ShipState,
        system: Option<&'a LocalSystem>,
        past: &'a [Past],
        known_from_s: f64,
    ) -> Self {
        Self { state, system, past, known_from_s }
    }

    /// What the ship was doing at a coordinate second.
    ///
    /// The first stretch that had not ended yet, or the current motive when none of them
    /// apply. `past` is ordered, so the first match is the right one.
    fn doing_at(&self, s: f64) -> &ShipState {
        self.past
            .iter()
            .find(|entry| s < entry.until_s)
            .map(|entry| &entry.motion)
            .unwrap_or(self.state)
    }

    /// Position and beta at a coordinate microsecond, in light-years.
    ///
    /// Falls back to the ship's last known position when the motive cannot be evaluated — a
    /// body that has gone, or a chain that is integrated. The same choice [`advance`] makes:
    /// keep what is known rather than invent a position from nothing.
    fn read(&self, t_us: f64) -> (DVec3, DVec3) {
        let s = t_us * 1.0e-6;
        let state = self.doing_at(s);
        state_at(state, self.system, s).unwrap_or((state.position_ly, state.beta))
    }
}

impl Worldline for Flight<'_> {
    fn position_at(&self, t: f64) -> DVec3 {
        self.read(t).0 * LIGHT_US_PER_LY
    }

    fn velocity_at(&self, t: f64) -> DVec3 {
        self.read(t).1
    }

    /// Forward forever, and back as far as the craft still remembers.
    ///
    /// Every arm of a motive answers everywhere, so the forward end never runs out. The back
    /// end is where the history was pruned: before it this would have to extrapolate a motive
    /// the ship was not yet flying, and the solver's contract is that a root outside this range
    /// is no root at all. Which is the answer that is safe — an observer far enough away that
    /// the light it wants left before the shard remembers simply sees nothing.
    fn defined_over(&self) -> (f64, f64) {
        (self.known_from_s * 1.0e6, f64::INFINITY)
    }
}

/// How fast a ship is going, metres a second, world frame.
pub fn velocity_m_s(state: &ShipState, system: Option<&LocalSystem>, now_s: f64) -> DVec3 {
    let beta = state_at(state, system, now_s).map(|(_, beta)| beta).unwrap_or(state.beta);
    beta * crate::flight::C_M_S
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
            let step_to = |state: &mut ShipState, system: &mut LocalSystem, target: f64, now: &mut f64| {
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

    /// The property that makes a motive a worldline: asking where a ship will be at `t` gives
    /// the same answer whatever time the system's own clock happens to sit at.
    ///
    /// Before this held, a station and a conic were read out of the propagated arena, so they
    /// were only correct at the present and a caller asking about the future got the present's
    /// answer wearing the future's timestamp. A light-delay solve roots on a *future* time, so
    /// this is the difference between scheduling an arrival correctly and scheduling it against
    /// where the ship was when the question was asked.
    #[test]
    fn a_ship_can_be_asked_about_a_time_the_system_is_not_at() {
        let Some(mut system) = sol() else { return };
        let mut ship = ShipState::at(DVec3::ZERO);
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: 0.0,
            change: orbit("Earth"),
        })
        .expect("a course");

        // Fly it, cut the engine, and leave the ship on a conic about Earth.
        let arrival = match &ship.motive {
            Motive::Crossing(cruise) => cruise.duration_s(),
            _ => unreachable!("a course is a crossing"),
        };
        system.advance_to(arrival);
        advance(&mut ship, Some(&system), arrival, arrival);
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: arrival,
            change: Change::CutDrive,
        })
        .expect("the engine cuts");
        assert!(matches!(ship.motive, Motive::Falling(_)), "{:?}", ship.motive);

        // A quarter of the orbit into the future, asked from three different presents.
        let ahead = arrival + 1_800.0;
        let asked = |at: f64| {
            let stale = system.propagated_to(at);
            state_at(&ship, Some(&stale), ahead).expect("an evaluable arc")
        };
        let from_now = asked(arrival);
        for present in [arrival - 100_000.0, ahead, ahead + 500_000.0] {
            let (at, beta) = asked(present);
            assert_eq!(at, from_now.0, "the clock at {present} changed where the ship will be");
            assert_eq!(beta, from_now.1);
        }
        // And it is not answering with the present: the ship has gone somewhere in the meantime.
        let (here, _) = state_at(&ship, Some(&system), arrival).unwrap();
        assert!(
            (from_now.0 - here).length() * crate::system::M_PER_LY > 1.0e6,
            "a quarter of a low orbit is a thousand kilometres and more",
        );
    }

    /// The same for a station, which is the other motive defined against a moving body.
    #[test]
    fn a_station_is_read_at_the_time_asked_for() {
        let Some(system) = sol() else { return };
        let mut ship = ShipState::at(DVec3::ZERO);
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: Plane::Equatorial };
        let waypoint = course.resolve(&system, DVec3::ZERO, 0.0).expect("a place");
        ship.begin_holding(waypoint);

        let ahead = 43_200.0;
        let here = state_at(&ship, Some(&system), 0.0).expect("a station");
        let there = state_at(&ship, Some(&system), ahead).expect("a station");
        let stale = state_at(&ship, Some(&system.propagated_to(ahead)), ahead).expect("a station");
        assert_eq!(there, stale, "the system's clock changed the answer");
        // Half a day is most of the way round the Earth's orbit *and* many low orbits.
        assert!((there.0 - here.0).length() * crate::system::M_PER_LY > 1.0e6);
    }

    /// The unit join, checked against the thing that consumes it: a light-microsecond of
    /// distance has to be a microsecond of delay, or every arrival the server schedules is
    /// wrong by whatever the conversion is out by.
    #[test]
    fn a_light_microsecond_away_is_a_microsecond_of_delay() {
        const LIGHT_SECOND_LY: f64 = 1.0 / JULIAN_YEAR_S;
        let ship = ShipState::at(DVec3::new(LIGHT_SECOND_LY, 0.0, 0.0));
        let line = Flight::new(&ship, None);
        let arrive = lc_spacetime::arrival_time_at(0.0, DVec3::ZERO, &line).expect("it arrives");
        assert!((arrive - 1.0e6).abs() < 1.0, "a light-second took {arrive} microseconds");
    }

    /// And a moving ship is read along its own line, not held at where it started.
    #[test]
    fn a_drifting_ship_is_somewhere_else_a_year_later() {
        let mut ship = ShipState::at(DVec3::ZERO);
        ship.beta = DVec3::new(0.5, 0.0, 0.0);
        ship.set_adrift(0.0);
        let line = Flight::new(&ship, None);
        // A Julian year of coordinate time at half light is half a light-year.
        let a_year_us = JULIAN_YEAR_S * 1.0e6;
        let at = line.position_at(a_year_us);
        assert!((at.x / LIGHT_US_PER_LY - 0.5).abs() < 1.0e-12, "{}", at.x / LIGHT_US_PER_LY);
        assert_eq!(line.velocity_at(a_year_us), ship.beta);
    }

    /// An escape from Earth leaves Earth's sphere at a time that can be *solved for*, and the
    /// solve lands on the boundary rather than near it.
    ///
    /// The difference from noticing: a detector only knows the ship has left once it has
    /// stepped past the crossing, so it is late by up to a whole step, and at any real time
    /// warp a step is hours. Here a step is irrelevant — the answer is a property of the arc.
    #[test]
    fn a_patch_is_solved_for_rather_than_noticed() {
        use em_foundations::time::Instant;
        let Some(system) = sol() else { return };
        let earth = system.body_named("Earth").expect("Earth");
        let (at_m, carried) = system.body_state_at(earth, 0.0).expect("a state");
        let radius = system.sim().radius(earth) * 3.0;
        let mu = system.sim().gravitational_constant() * system.sim().mass(earth);

        // Three Earth radii out, at 1.6 times circular: comfortably above escape, so the arc
        // is a hyperbola that leaves rather than an ellipse that comes back.
        let position_ly = system.origin_ly + (at_m + DVec3::X * radius) / crate::system::M_PER_LY;
        let velocity = carried + DVec3::Y * (mu / radius).sqrt() * 1.6;
        let arc = crate::coast::Coast::from_state(&system, position_ly, velocity, 0.0)
            .expect("an arc");
        assert_eq!(arc.primary, "Earth");
        assert!(arc.is_escaping(), "e = {}", arc.elements.eccentricity);

        let mut ship = ShipState::at(position_ly);
        ship.beta = coast::beta_of(velocity);
        ship.motive = Motive::Falling(arc.clone());

        let event = repatch_at(&ship, &system, 0.0).expect("an escape leaves");
        assert!(matches!(event.change, Change::Repatch { .. }), "{:?}", event.change);

        // On the boundary, not near it: inside a second either side of the answer, and the
        // sign flips across it.
        let distance_at = |t: f64| {
            em_sim::crossing::boundary_distance_of(
                system.sim(),
                &arc.path(&system),
                earth,
                Instant::from_seconds_since_j2000(t),
            )
            .expect("evaluable")
        };
        let speed = velocity.length();
        assert!(distance_at(event.at_t).abs() < speed, "{} m off", distance_at(event.at_t));
        assert!(distance_at(event.at_t - 60.0) < 0.0, "it was already outside a minute before");
        assert!(distance_at(event.at_t + 60.0) > 0.0, "it was still inside a minute after");

        // A few days, which is what an escape from Earth takes. Named so a change in the
        // sphere model or the escape speed shows up as a number rather than as a pass.
        let days = event.at_t / 86_400.0;
        assert!((1.0..30.0).contains(&days), "{days} days to leave Earth");

        // And a detector stepping at anything coarse is late by up to a step. This is the
        // thing prediction replaces.
        const STEP_S: f64 = 3.0 * 3_600.0;
        let mut noticed = 0.0;
        while distance_at(noticed) < 0.0 {
            noticed += STEP_S;
        }
        assert!(noticed > event.at_t, "a detector cannot be early");
        assert!(
            noticed - event.at_t > 600.0,
            "a three-hour step happened to land on the crossing; pick another",
        );
    }

    /// And two authorities stepping at different rates reach the same arc about the same new
    /// primary, because the patch happened at a solved coordinate rather than on a frame.
    ///
    /// This is the determinism test the detector could not pass. A detector fires on the step
    /// that notices, so a server at 438 seconds and a client at 61 would re-solve the conic at
    /// two different instants and be on two different orbits from then on.
    #[test]
    fn the_step_size_does_not_change_which_arc_the_ship_ends_up_on() {
        let Some(system) = sol() else { return };
        let earth = system.body_named("Earth").expect("Earth");
        let (at_m, carried) = system.body_state_at(earth, 0.0).expect("a state");
        let radius = system.sim().radius(earth) * 3.0;
        let mu = system.sim().gravitational_constant() * system.sim().mass(earth);
        let position_ly = system.origin_ly + (at_m + DVec3::X * radius) / crate::system::M_PER_LY;
        let velocity = carried + DVec3::Y * (mu / radius).sqrt() * 1.6;

        let start = |state: &mut ShipState| {
            let arc = crate::coast::Coast::from_state(&system, position_ly, velocity, 0.0)
                .expect("an arc");
            state.beta = coast::beta_of(velocity);
            state.motive = Motive::Falling(arc);
        };

        // What an authority does: fold the patch at the time it was solved for, then advance.
        // The order is the point -- checking first means the event is always stamped with its
        // own coordinate, whatever step happened to run past it.
        let run = |step: f64, until: f64| {
            let mut ship = ShipState::at(position_ly);
            start(&mut ship);
            let mut patch = repatch_at(&ship, &system, 0.0);
            let mut now = 0.0;
            while now < until {
                let next = (now + step).min(until);
                let elapsed = next - now;
                now = next;
                if let Some(event) = patch.take_if(|e| e.at_t <= now) {
                    apply(&mut ship, Some(&system), &event).expect("a patch applies");
                    patch = repatch_at(&ship, &system, event.at_t);
                }
                advance(&mut ship, Some(&system), now, elapsed);
            }
            ship
        };

        let patch = repatch_at(&ShipState {
            motive: {
                let mut s = ShipState::at(position_ly);
                start(&mut s);
                s.motive
            },
            ..ShipState::at(position_ly)
        }, &system, 0.0)
        .expect("it leaves");
        let until = patch.at_t + 86_400.0;

        let coarse = run(438.0, until);
        let fine = run(61.0, until);

        // It really did change primary: out of Earth's sphere and into the Sun's.
        let Motive::Falling(arc) = &coarse.motive else { panic!("{:?}", coarse.motive) };
        assert_ne!(arc.primary, "Earth", "it never left");
        assert!((arc.epoch_s - patch.at_t).abs() < 1.0e-6, "the arc began at the wrong instant");

        assert_eq!(coarse.motive, fine.motive, "two step sizes, two different arcs");
        assert_eq!(coarse.position_ly, fine.position_ly);
        assert_eq!(coarse.beta, fine.beta);
    }

    /// An orbit that stays where it is meets nothing, and the search says so rather than
    /// inventing a patch at the end of its horizon.
    #[test]
    fn an_orbit_that_goes_nowhere_has_no_patch() {
        let Some(system) = sol() else { return };
        let mut ship = ShipState::at(DVec3::ZERO);
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: 0.0,
            change: orbit("Earth"),
        })
        .expect("a course");
        let arrival = match &ship.motive {
            Motive::Crossing(cruise) => cruise.duration_s(),
            _ => unreachable!(),
        };
        advance(&mut ship, Some(&system), arrival, arrival);
        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: arrival,
            change: Change::CutDrive,
        })
        .expect("the engine cuts");
        assert!(matches!(ship.motive, Motive::Falling(_)));
        assert!(repatch_at(&ship, &system, arrival).is_none(), "a circular orbit stays put");
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

    /// **Burn, flip and burn, seen from outside.**
    ///
    /// The whole of why the nose follows the drive rather than the velocity: for the second
    /// half of a crossing the ship is pointing back the way it came while still travelling
    /// forward at a large fraction of `c`. A hull drawn along its velocity would spend that
    /// half facing the wrong way, and nothing about the picture would say it was braking.
    #[test]
    fn a_crossing_flips_the_nose_over_while_the_ship_still_moves_forward() {
        let to = DVec3::X * 4.0;
        let cruise = crate::flight::Cruise::plan(DVec3::ZERO, to, 0.0, crate::flight::Drive::DEFAULT);
        let whole = cruise.duration_s();
        let mut state = ShipState::at(DVec3::ZERO);
        state.begin_crossing(cruise.clone(), None);

        let (mut boosted, mut braked, mut coasted) = (false, false, false);
        for k in 1..200 {
            let t = whole * k as f64 / 200.0;
            let nose = facing(&state, None, t).expect("a ship under thrust is pointing somewhere");
            let beta = state_at(&state, None, t).unwrap().1;
            // Forward, the whole way. It never turns round; only the ship does.
            assert!(beta.x > 0.0, "the ship went backwards at {t}: {beta}");
            match cruise.at(t).phase {
                crate::flight::Phase::Boost => {
                    boosted = true;
                    assert!(nose.x > 0.999, "boosting and not pointing along the line: {nose}");
                }
                crate::flight::Phase::Brake => {
                    braked = true;
                    assert!(nose.x < -0.999, "braking and not pointing back down it: {nose}");
                }
                // Nothing lit, so the nose is left along the velocity.
                crate::flight::Phase::Coast => {
                    coasted = true;
                    assert!(nose.x > 0.999, "coasting and not pointing along the motion: {nose}");
                }
                _ => {}
            }
        }
        // A coast is not guaranteed — a crossing short enough never to reach the drive's cap
        // is boost straight into brake — so it is checked where it happens and not required.
        let _ = coasted;
        assert!(boosted, "the crossing never boosted");
        assert!(braked, "the crossing never braked, which is the half this test is about");
    }

    /// Falling is not thrust: nothing is lit, so the nose is simply the way the ship is going,
    /// and it swings as the arc curves.
    ///
    /// The contrast is with [`a_crossing_flips_the_nose_over_while_the_ship_still_moves_forward`],
    /// where the drive is what decides. A `facing` built on the *coordinate* acceleration would
    /// make these the same case and leave an orbiting ship permanently nose-down.
    #[test]
    fn a_ballistic_ship_points_along_its_motion_and_turns_with_it() {
        let Some(system) = sol() else { return };
        let mut state = ShipState::at(system.body_position_ly("Earth").unwrap());
        let event = Event { ship: ShipId(1), at_t: 0.0, change: orbit("Earth") };
        apply(&mut state, Some(&system), &event).unwrap();
        // Off the station and onto the conic it was already flying, which is what cutting does.
        apply(&mut state, Some(&system), &Event { ship: ShipId(1), at_t: 0.0, change: Change::CutDrive })
            .unwrap();

        let mut noses = Vec::new();
        for k in 0..6 {
            let t = 600.0 * k as f64;
            let nose = facing(&state, Some(&system), t).expect("a moving ship points somewhere");
            let beta = state_at(&state, Some(&system), t).unwrap().1;
            assert!((nose - beta.normalize()).length() < 1e-9, "the nose left the velocity at {t}");
            noses.push(nose);
        }
        // And it is following the arc rather than being stuck on whatever it started with.
        let swing = noses[0].dot(*noses.last().unwrap()).clamp(-1.0, 1.0).acos();
        assert!(swing > 0.05, "the nose barely moved over an hour of orbit: {swing} rad");
    }

    /// A craft at rest with the engine off has no attitude this can derive, and says so rather
    /// than inventing one. What to draw instead is the renderer's problem.
    #[test]
    fn nothing_decides_where_a_parked_ship_points() {
        let state = ShipState::at(DVec3::X);
        assert_eq!(facing(&state, None, 0.0), None);
    }


    /// **An intercept, flown.** Not merely planned: stepped through `advance`, which is what a
    /// server and a client both actually do, and asked where it ended up.
    ///
    /// The claim is the one a separate injection burn would exist to make — that the ship ends
    /// alongside *and* moving with the quarry — and it comes out of one plan because the plan
    /// was made in the quarry's frame.
    #[test]
    fn a_rendezvous_ends_alongside_a_moving_quarry_and_matched_to_it() {
        use crate::pursuit::{Sighting, approach, standoff_m};

        let km = 1.0e3 / crate::system::M_PER_LY;
        let drifting = DVec3::new(0.0, 4.0e-5, 1.0e-5);
        let seen = Sighting {
            target: ShipId(2),
            position_ly: DVec3::X * 2_000.0 * km,
            beta: drifting,
            length_m: 500.0,
            emitted_s: 0.0,
        };
        let mut state = ShipState::at(DVec3::ZERO);
        let plan = approach(&state, 500.0, &seen, 0.0, crate::flight::Drive::DEFAULT).unwrap();
        let whole = plan.cruise.duration_s();
        state.begin_rendezvous(plan);

        // Stepped, so the arrival transition runs where it really would.
        let steps = 400;
        for k in 1..=steps {
            let t = whole * 1.05 * k as f64 / steps as f64;
            advance(&mut state, None, t, whole * 1.05 / steps as f64);
        }
        let end = whole * 1.05;

        let standoff = standoff_m(500.0, seen.length_m) / crate::system::M_PER_LY;
        let gap = state.position_ly.distance(seen.reckoned_at(end));
        assert!(
            (gap - standoff).abs() < standoff * 0.01,
            "ended {gap} from the quarry, wanted {standoff}",
        );
        assert!(
            (state.beta - drifting).length() < drifting.length() * 0.01,
            "ended at {} rather than matched to {drifting}",
            state.beta,
        );
        // And it is off the approach: arriving is not a state a ship stays in.
        assert!(
            matches!(state.motive, Motive::Drifting { .. }),
            "still flying the approach: {:?}",
            state.motive,
        );
        // Having matched, it stays matched — the pair coast together rather than separating.
        let before = state.position_ly.distance(seen.reckoned_at(end));
        advance(&mut state, None, end + 3_600.0, 3_600.0);
        let after = state.position_ly.distance(seen.reckoned_at(end + 3_600.0));
        assert!((after - before).abs() < standoff * 0.01, "drifted apart: {before} to {after}");
    }

    /// The nose follows the drive on an approach exactly as it does on a crossing, and the
    /// frame does not tilt it: the flip happens in the middle and points back down the track.
    #[test]
    fn an_approach_flips_its_nose_over_like_any_other_burn() {
        use crate::pursuit::{Sighting, approach};

        let km = 1.0e3 / crate::system::M_PER_LY;
        let seen = Sighting {
            target: ShipId(2),
            position_ly: DVec3::X * 2_000.0 * km,
            beta: DVec3::ZERO,
            length_m: 500.0,
            emitted_s: 0.0,
        };
        let mut state = ShipState::at(DVec3::ZERO);
        let plan = approach(&state, 500.0, &seen, 0.0, crate::flight::Drive::DEFAULT).unwrap();
        let whole = plan.cruise.duration_s();
        state.begin_rendezvous(plan);

        let early = facing(&state, None, whole * 0.1).expect("under thrust");
        let late = facing(&state, None, whole * 0.9).expect("still under thrust");
        assert!(early.x > 0.99, "not boosting toward the quarry: {early}");
        assert!(late.x < -0.99, "not braking back down the track: {late}");
    }


}
