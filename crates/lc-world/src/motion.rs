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

pub use crate::worldline::{Flight, Past};

/// A ship, by the identifier whoever owns it uses. Opaque here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShipId(pub i64);

/// How a ship is moving. The eight are exclusive, and that exclusivity is the model.
#[derive(Clone, Debug, PartialEq)]
pub enum Motive {
    /// Under thrust, on a planned crossing.
    Crossing(Cruise),
    /// Under thrust, on a crossing flown in a *body's* frame rather than the world's.
    ///
    /// What going from one orbit of Earth to another actually is. A motive of its own for the
    /// same reason [`Motive::Rendezvous`] is: the frame is part of the answer, and reading the
    /// plan in the world's needs the body back. See [`crate::transfer`], which also says why the
    /// world frame cannot fly this at all.
    Transfer(crate::transfer::Transfer),
    /// Under thrust, closing on another craft and matching its velocity.
    ///
    /// A crossing in a moving frame, and it is a motive of its own rather than a `Crossing`
    /// because the frame is part of the answer: the plan ends at rest in the *quarry's* frame,
    /// which is what "matched" means, and reading it in the world's needs the frame back. See
    /// [`crate::pursuit`], which also says why the frame is a frozen sighting and not a handle
    /// on the quarry's live worldline.
    Rendezvous(crate::pursuit::Rendezvous),
    /// Under thrust beside a craft that is itself under thrust, having closed on it or while
    /// closing. A rendezvous with a quarry that will not hold still: see [`crate::escort`].
    Escort(crate::escort::Escort),
    /// Closing on a craft that is falling, or holding station beside it, in the frame that falls
    /// with it. See [`crate::consort`].
    Consort(crate::consort::Consort),
    /// Held on a place by thrust. A station is a position, not a trajectory.
    Holding(Waypoint),
    /// Ballistic on a conic, about whichever body's influence it is in.
    Falling(Coast),
    /// Nothing holding it and nothing to fall toward: a straight line at whatever it has.
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
    /// Which way the nose pointed when the current motive began.
    ///
    /// A turn takes time, so where the nose *is* depends on when you ask — that is
    /// [`facing`]. This is where it starts from: the attitude the last order left it at, which
    /// is the one thing about the ship's orientation that the trajectory cannot say.
    ///
    /// Never zero. A craft with nothing to aim it at points at the vernal equinox, which is
    /// arbitrary and has to be *something* fixed in the world.
    pub attitude: DVec3,
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
            attitude: DVec3::X,
            clock_s: 0.0,
            crossing_clock_base_s: 0.0,
            arrive_at: None,
        }
    }

    pub fn is_under_way(&self) -> bool {
        matches!(
            self.motive,
            Motive::Crossing(_)
                | Motive::Transfer(_)
                | Motive::Rendezvous(_)
                | Motive::Escort(_)
                | Motive::Consort(_)
        )
    }

    /// Put a ship on an approach solved for it. The counterpart of [`Self::begin_crossing`],
    /// and it bases the crew's clock the same way.
    pub fn begin_rendezvous(&mut self, plan: crate::pursuit::Rendezvous) {
        self.crossing_clock_base_s = self.clock_s;
        self.motive = Motive::Rendezvous(plan);
        self.arrive_at = None;
    }

    /// Take up station on a quarry under thrust. Bases the crew's clock as a crossing does.
    pub fn begin_escort(&mut self, plan: crate::escort::Escort) {
        self.crossing_clock_base_s = self.clock_s;
        self.motive = Motive::Escort(plan);
        self.arrive_at = None;
    }

    /// Take up station on a falling quarry. Bases the crew's clock as a crossing does.
    pub fn begin_consort(&mut self, plan: crate::consort::Consort) {
        self.crossing_clock_base_s = self.clock_s;
        self.motive = Motive::Consort(plan);
        self.arrive_at = None;
    }

    /// Put one back part-way through, keeping the clock base it began with.
    pub fn resume_consort(&mut self, plan: crate::consort::Consort, clock_base_s: f64) {
        self.motive = Motive::Consort(plan);
        self.arrive_at = None;
        self.crossing_clock_base_s = clock_base_s;
    }

    /// Put one back part-way through, keeping the clock base it began with.
    pub fn resume_escort(&mut self, plan: crate::escort::Escort, clock_base_s: f64) {
        self.motive = Motive::Escort(plan);
        self.arrive_at = None;
        self.crossing_clock_base_s = clock_base_s;
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

    /// Who this ship is closing on or keeping station with, if anybody.
    pub fn pursuing(&self) -> Option<ShipId> {
        match &self.motive {
            Motive::Rendezvous(plan) => Some(plan.target),
            Motive::Escort(plan) => Some(plan.target),
            Motive::Consort(plan) => Some(plan.target),
            _ => None,
        }
    }

    /// Whether the approach to [`Self::pursuing`]'s quarry is still being flown, rather than
    /// done and holding alongside.
    pub fn still_closing(&self, now_s: f64) -> bool {
        match &self.motive {
            Motive::Rendezvous(plan) => !plan.has_arrived(now_s),
            Motive::Escort(plan) => !plan.has_closed(now_s),
            Motive::Consort(plan) => !plan.has_closed(now_s),
            _ => false,
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
    /// Put a ship on a transfer solved for it, basing the crew's clock as a crossing does.
    pub fn begin_transfer(&mut self, transfer: crate::transfer::Transfer, arrive_at: Waypoint) {
        self.crossing_clock_base_s = self.clock_s;
        self.motive = Motive::Transfer(transfer);
        self.arrive_at = Some(arrive_at);
    }

    /// Put one back part-way through, keeping the clock base it began with.
    pub fn resume_transfer(
        &mut self,
        transfer: crate::transfer::Transfer,
        arrive_at: Option<Waypoint>,
        clock_base_s: f64,
    ) {
        self.motive = Motive::Transfer(transfer);
        self.arrive_at = arrive_at;
        self.crossing_clock_base_s = clock_base_s;
    }

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
            attitude: self.attitude,
            clock_s: self.clock_s,
            drive: self.drive,
            motive: match &self.motive {
                Motive::Crossing(cruise) => Recipe::Crossing {
                    from_ly: cruise.from_ly,
                    beta0: cruise.initial_beta(),
                    to_ly: cruise.to_ly,
                    arrive_beta: cruise.arrive_beta(),
                    start_s: cruise.start_s,
                    drive: cruise.drive,
                    arrive_at: self.arrive_at.clone(),
                    clock_base_s: self.crossing_clock_base_s,
                },
                Motive::Transfer(transfer) => Recipe::Transfer {
                    about: transfer.about.clone(),
                    from_ly: transfer.cruise.from_ly,
                    beta0: transfer.cruise.initial_beta(),
                    to_ly: transfer.cruise.to_ly,
                    arrive_beta: transfer.cruise.arrive_beta(),
                    start_s: transfer.cruise.start_s,
                    drive: transfer.cruise.drive,
                    arrive_at: self.arrive_at.clone(),
                    clock_base_s: self.crossing_clock_base_s,
                },
                Motive::Rendezvous(plan) => Recipe::Rendezvous {
                    approach: plan.recipe(),
                    clock_base_s: self.crossing_clock_base_s,
                },
                Motive::Escort(plan) => Recipe::Escort {
                    station: plan.recipe(),
                    clock_base_s: self.crossing_clock_base_s,
                },
                Motive::Consort(plan) => Recipe::Consort {
                    formation: plan.recipe(),
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
    /// leaving a system is exactly what one is for. Dropping it here canceled every
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
            Motive::Rendezvous(_) | Motive::Escort(_) => {}
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
    /// A **position**, not a star id, because this fold is pure motion and has no catalog to
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
                crate::flight::Cruise::plan_from(
                    at,
                    beta,
                    stop,
                    state.attitude,
                    event.at_t,
                    *drive,
                ),
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
            let waypoint =
                course.resolve_moving(system, at, beta, event.at_t).ok_or(Rejected::NoSuchPlace)?;
            state.position_ly = at;
            state.beta = beta;
            state.drive = *drive;
            // **A station about the body the ship is already falling with is flown in that
            // body's frame.** In the world's, the destination runs away at the body's own speed
            // and the arrival time has no fixed point — see `crate::transfer`. Decided here
            // rather than inside the planner because it is a choice of *motive*, and both sides
            // have to make the same one from the same event.
            let about = crate::transfer::primary_for(system, &waypoint, at, event.at_t);
            let planned = about.as_deref().and_then(|about| {
                crate::transfer::plan(
                    system,
                    about,
                    &waypoint,
                    at,
                    beta,
                    state.attitude,
                    event.at_t,
                    *drive,
                )
            });
            if let Some((transfer, aimed)) = planned {
                state.begin_transfer(transfer, aimed);
                return Ok(());
            }
            let (cruise, aimed) =
                crate::navigation::plan(
                    system,
                    &waypoint,
                    at,
                    beta,
                    state.attitude,
                    event.at_t,
                    *drive,
                )
                .ok_or(Rejected::NoSuchPlace)?;
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
        // The plan is in the body's frame, so the body has to be added back. Without a system
        // there is no body to add and no answer to give.
        Motive::Transfer(transfer) => transfer.state_at(system?, now_s),
        // The plan is relative, so the frame has to be added back. Galilean, and
        // [`crate::pursuit`] carries the bound on that.
        Motive::Rendezvous(plan) => Some(plan.state_at(now_s)),
        Motive::Escort(plan) => Some(plan.state_at(now_s)),
        Motive::Consort(plan) => plan.state_at(system?, now_s),
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

/// Which way the hull's nose points at a coordinate time.
///
/// `hull_m` is how long the ship is, which is what decides how fast it turns. Passed rather
/// than carried on the state: a second copy of a craft's length is a second copy that can
/// disagree with the first.
///
/// Thrust first, velocity second. A ship under way points along its drive — which is *back*
/// down its own track through a brake — and a ship with the engine off points along its
/// motion. A craft at rest with nothing burning has no attitude this can derive, and `None`
/// says so rather than inventing one; the renderer holds whatever it last had.
///
/// Proper acceleration, so a ballistic arc counts as unpowered. Falling is not thrust, and a
/// nose that followed the coordinate acceleration would point at the primary all the way round
/// an orbit.
pub fn facing(state: &ShipState, hull_m: f64, now_s: f64) -> Option<DVec3> {
    Some(facing_at(state, hull_m, now_s))
}

/// Which way the nose points at a coordinate time.
///
/// The ship swings toward whatever its plan last asked for, at the rate its hull allows — see
/// [`crate::attitude`] — and a plan that has asked for nothing leaves it where it was. A
/// coasting ship does not turn: there is nothing to point at, and attitude control is not free.
///
/// **Not the velocity.** An earlier version pointed a coasting ship along its motion, which
/// left a ship that had just braked to a halt facing whichever way its last millimeter a second
/// happened to go.
pub fn facing_at(state: &ShipState, hull_m: f64, now_s: f64) -> DVec3 {
    let rate = crate::attitude::rate_rad_s(hull_m);
    let Some(aim) = aim_at(state, now_s) else { return state.attitude };
    let from = aim.from.unwrap_or(state.attitude);
    crate::attitude::turned(from, aim.to, rate, now_s - aim.since_s)
}

/// What the current plan is asking the nose to do, if it asks anything.
///
/// The frame does not rotate, so an aim taken in the quarry's frame is an aim here — but a
/// rendezvous keeps that frame's own *clock*, so it has to be asked in world time through the
/// plan rather than through its cruise.
fn aim_at(state: &ShipState, now_s: f64) -> Option<crate::flight::Aim> {
    match &state.motive {
        Motive::Crossing(cruise) => Some(cruise.aim_at(now_s)),
        Motive::Transfer(transfer) => Some(transfer.aim_at(now_s)),
        Motive::Rendezvous(plan) => Some(plan.aim_at(now_s)),
        Motive::Escort(plan) => Some(plan.aim_at(now_s)),
        Motive::Consort(plan) => Some(plan.aim_at(now_s)),
        // Nothing is asking. A station is held by thrust too small to turn for, and a conic
        // and a drift ask for nothing at all.
        Motive::Holding(_) | Motive::Falling(_) | Motive::Drifting { .. } => None,
    }
}

/// How hard a ship is burning at a coordinate time, in g. Zero when nothing is lit.
///
/// The magnitude of what [`facing`] gives the direction of, and it has the same rule: *proper*
/// acceleration, so a ballistic arc is zero however hard it is falling.
///
/// Holding a station is zero too, and that one is a simplification rather than a definition. A
/// station is held by thrust, but the thrust is whatever cancels the local gravity — milligravities
/// against the whole-g burns everything else here is about, and a plume nobody would see.
pub fn thrust_g(state: &ShipState, now_s: f64) -> f64 {
    let lit = match &state.motive {
        Motive::Crossing(cruise) => cruise.thrust_at(now_s) != DVec3::ZERO,
        Motive::Transfer(transfer) => transfer.thrust_at(now_s) != DVec3::ZERO,
        Motive::Rendezvous(plan) => plan.thrust_at(now_s) != DVec3::ZERO,
        // Station-keeping beside it is the quarry's tidal difference, which is the same
        // simplification as holding a station.
        Motive::Consort(plan) => plan.thrust_at(now_s) != DVec3::ZERO,
        // The one motive whose burn is not the drive's rating: beside a quarry, it is the
        // quarry's acceleration, and that is what a plume should show.
        Motive::Escort(plan) => return plan.thrust_g(now_s),
        Motive::Holding(_) | Motive::Falling(_) | Motive::Drifting { .. } => false,
    };
    if lit { state.drive.accel_g } else { 0.0 }
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
                // Whatever the crossing ended on, which is the station's velocity when there was
                // one to join and rest otherwise. Not zero: a crossing planned onto an orbit
                // finishes *moving*, and forcing it to a stop here would throw away the burn
                // that got it there. See `Cruise::plan_onto`.
                state.beta = flight.beta;
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
        Motive::Transfer(transfer) => {
            let flight = transfer.flight_at(now_s);
            state.clock_s = state.crossing_clock_base_s + flight.proper_s;
            if flight.phase == Phase::Arrived {
                // The station's velocity, in the world, which is the body's plus the plan's. The
                // transfer ends *on* it — see `Cruise::plan_onto` — so there is nothing left to
                // find here.
                if let Some((at, beta)) = state_at(state, system, now_s) {
                    state.position_ly = at;
                    state.beta = beta;
                }
                state.motive = match state.arrive_at.take() {
                    Some(waypoint) => {
                        if let Some(at) = system.and_then(|s| waypoint.place_at(s, now_s)) {
                            state.position_ly = at;
                        }
                        Motive::Holding(waypoint)
                    }
                    None => Motive::Drifting { from_ly: state.position_ly, since_t: now_s },
                };
            }
        }
        // Never ends by itself. Alongside a quarry that keeps burning is a place to *stay*, and
        // going ballistic on arrival — as a rendezvous does — would drop straight behind it.
        Motive::Escort(plan) => {
            state.clock_s = state.crossing_clock_base_s + plan.proper_s_at(now_s);
        }
        // Never ends either, for the same reason: beside a quarry on its orbit is somewhere to
        // stay, and going ballistic there drifts off by the difference between two orbits.
        Motive::Consort(plan) => {
            state.clock_s = state.crossing_clock_base_s + plan.proper_s_at(now_s);
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

    // Off the boundary the arc may be sitting exactly on, having just been solved there. A
    // search from the join itself finds that same crossing and the walk never advances; the
    // tolerance is a millisecond, so a second is far clear of it and far inside any arc.
    const CLEARANCE_S: f64 = 1.0;
    let from_s = from_s + CLEARANCE_S;

    let found = match arc.period_s() {
        Some(period) => em_sim::crossing::first_crossing_of(
            system.sim(),
            &arc.path(system),
            &candidates,
            Instant::from_seconds_since_j2000(from_s),
            TimeDelta::from_seconds(period * PATCH_HORIZON_REVOLUTIONS),
        ),
        None => crate::escape::first_crossing(
            arc,
            system,
            primary,
            &candidates,
            from_s,
            from_s + OPEN_PATCH_HORIZON_S,
        ),
    }?;

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

/// How fast a ship is going, meters a second, world frame.
pub fn velocity_m_s(state: &ShipState, system: Option<&LocalSystem>, now_s: f64) -> DVec3 {
    let beta = state_at(state, system, now_s).map(|(_, beta)| beta).unwrap_or(state.beta);
    beta * crate::flight::C_M_S
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_spacetime::Worldline;
    use crate::navigation::Plane;
    use crate::sky::{CatalogStar, StarProvider};

    fn sol() -> Option<LocalSystem> {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .ok()?;
        let sun: CatalogStar =
            provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some("Sol"))?.clone();
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
            "a quarter of a low orbit is a thousand kilometers and more",
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

    /// **A ship pointing the wrong way pays for turning round before it can go anywhere.**
    ///
    /// Through the fold, which is what both sides run: the attitude the ship is carrying is what
    /// the planner is handed, so the crossing the client predicts and the one the server flies
    /// have the same turn in them.
    #[test]
    fn a_course_set_facing_the_wrong_way_turns_before_it_burns() {
        let Some(system) = sol() else { return };
        let plan_facing = |attitude: DVec3| {
            let mut ship = ShipState::at(DVec3::ZERO);
            ship.attitude = attitude;
            apply(&mut ship, Some(&system), &Event {
                ship: ShipId(1),
                at_t: 0.0,
                change: orbit("Earth"),
            })
            .unwrap();
            let Motive::Crossing(cruise) = ship.motive.clone() else { panic!("a crossing") };
            cruise
        };

        // Which way the crossing wants to be pointed, asked of a plan that was not told where the
        // nose was and so charged for no turn. Not simply the direction of Earth: a crossing
        // stops short of a body and aims at the near point of the orbit, which is degrees off.
        let heading = plan_facing(DVec3::ZERO).aim_at(0.0).to;

        // Nose already down the line: nothing to turn, and the first burn is lit at once.
        let ready = plan_facing(heading);
        assert_eq!(ready.turn_s(), 0.0);
        assert_ne!(ready.thrust_at(0.0), DVec3::ZERO);

        // Nose the other way: a full flip first, with the drive off the whole time. The ship is
        // at rest, so it does not drift while it turns and the line is the same one.
        let backwards = plan_facing(-heading);
        // Not to the bit: turning first makes the crossing a minute longer, so Earth has moved
        // and the heading is a hair off the one the ship was facing away from.
        assert!(
            (backwards.turn_s() - Drive::DEFAULT.flip_s()).abs() < 1.0e-2,
            "turned for {} s, not the {} s a flip takes",
            backwards.turn_s(),
            Drive::DEFAULT.flip_s(),
        );
        assert_eq!(backwards.thrust_at(backwards.turn_s() * 0.5), DVec3::ZERO);
        assert_eq!(
            backwards.at(backwards.turn_s() * 0.5).phase,
            crate::flight::Phase::Turn,
        );

        // And the whole crossing is longer by exactly what the turn cost.
        let extra = backwards.duration_s() - ready.duration_s();
        assert!(
            (extra - backwards.turn_s()).abs() < 1.0,
            "the turn cost {extra} s but took {} s",
            backwards.turn_s(),
        );
    }

    /// **A course set from an orbit of a body to another orbit of the same body is a transfer**,
    /// and both sides pick that from the event rather than being told.
    ///
    /// The whole round trip through the fold: a ship on a low orbit of Earth asks for a high one,
    /// flies it, and finishes holding the station — in the right place and at the right speed.
    /// The world frame cannot do this at all; see [`crate::transfer`].
    #[test]
    fn a_course_between_two_orbits_of_one_body_is_flown_in_that_bodys_frame() {
        let Some(mut system) = sol() else { return };
        let low = Course::Orbit {
            body: "Earth".into(),
            altitude_radii: 0.5,
            plane: Plane::Equatorial,
        }
        .resolve(&system, DVec3::ZERO, 0.0)
        .expect("a low orbit of Earth");

        // Start on it, moving with it, which is what being in an orbit means.
        let mut ship = ShipState::at(low.place_at(&system, 0.0).unwrap());
        ship.beta = crate::coast::beta_of(low.velocity_at(&system, 0.0).unwrap());
        ship.begin_holding(low.clone());
        ship.beta = crate::coast::beta_of(low.velocity_at(&system, 0.0).unwrap());

        apply(&mut ship, Some(&system), &Event {
            ship: ShipId(1),
            at_t: 0.0,
            change: orbit("Earth"),
        })
        .unwrap();
        let Motive::Transfer(transfer) = ship.motive.clone() else {
            panic!("a course about the body the ship is falling with is a transfer, not {:?}", ship.motive)
        };
        assert_eq!(transfer.about, "Earth");

        let arrival = transfer.duration_s();
        let mut now = 0.0;
        while now < arrival + 1.0 {
            now = (now + 60.0).min(arrival + 1.0);
            system.advance_to(now);
            advance(&mut ship, Some(&system), now, 60.0);
        }

        let Motive::Holding(station) = ship.motive.clone() else { panic!("{:?}", ship.motive) };
        let earth = system.body_position_ly("Earth").unwrap();
        let radii = ship.position_ly.distance(earth) * crate::system::M_PER_LY / 6.371e6;
        assert!((radii - 3.0).abs() < 0.1, "ended {radii} radii out, not the three asked for");

        // And it met the station: at the instant the transfer ends, in the same place and at the
        // same velocity. Asked of the transfer at *its* arrival rather than of the ship a step
        // later, because the station is going round a corner the whole time and a second of that
        // is a meter a second.
        let (met_at, met_beta) = transfer.state_at(&system, arrival).expect("a place");
        let joining = crate::coast::beta_of(station.velocity_at(&system, arrival).unwrap());
        let miss_m = met_at.distance(station.place_at(&system, arrival).unwrap())
            * crate::system::M_PER_LY;
        let short_m_s = (met_beta - joining).length() * crate::flight::C_M_S;
        assert!(miss_m < 1.0, "arrived {miss_m:e} m off the station");
        assert!(short_m_s < 1.0e-3, "arrived {short_m_s} m/s off the station");

        // Earth ran further than the orbit is wide while this was flown, which is the premise:
        // planned in the world frame the destination is simply running away.
        let ran_m = joining.length() * crate::flight::C_M_S * arrival;
        assert!(ran_m > 3.0 * 6.371e6, "premise: Earth ran only {ran_m:e} m");
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
        let Motive::Holding(station) = ship.motive.clone() else { panic!("{:?}", ship.motive) };
        // **Arriving is not stopping.** The crossing ends *on* the station's velocity, which for
        // an orbit of Earth is most of Earth's twenty-nine kilometers a second round the sun. A
        // ship that braked to a dead halt here would have to find all of that from nowhere
        // between two samples, which is what the free injection used to be.
        let joining = crate::coast::beta_of(station.velocity_at(&system, now).unwrap());
        assert!(joining.length() > 1.0e-5, "premise: the station is moving, at {joining:?}");
        assert!(
            (ship.beta - joining).length() < 1.0e-6,
            "arrived at {:?} rather than on the station's {joining:?}",
            ship.beta,
        );

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
    /// half of a crossing the ship is pointing back the way it came while still traveling
    /// forward at a large fraction of `c`. A hull drawn along its velocity would spend that
    /// half facing the wrong way, and nothing about the picture would say it was braking.
    ///
    /// The flip is *ordered* at the end of the boost and takes the hull's own turning time to
    /// finish, so the sample that lands on the boundary is checked separately from the ones
    /// well into the brake.
    #[test]
    fn a_crossing_flips_the_nose_over_while_the_ship_still_moves_forward() {
        let to = DVec3::X * 4.0;
        let cruise = crate::flight::Cruise::plan(DVec3::ZERO, to, 0.0, crate::flight::Drive::DEFAULT);
        let whole = cruise.duration_s();
        let mut state = ShipState::at(DVec3::ZERO);
        state.begin_crossing(cruise.clone(), None);

        let hull = 500.0;
        let flip_s = crate::attitude::turn_time_s(DVec3::X, -DVec3::X, crate::attitude::rate_rad_s(hull));
        assert!(flip_s > 0.0, "premise: turning takes time");

        let (mut boosted, mut braked, mut coasted) = (false, false, false);
        for k in 1..200 {
            let t = whole * k as f64 / 200.0;
            let nose = facing(&state, hull, t).expect("a ship under way is pointing somewhere");
            let beta = state_at(&state, None, t).unwrap().1;
            // Forward, the whole way. It never turns round; only the ship does.
            assert!(beta.x > 0.0, "the ship went backwards at {t}: {beta}");
            let since_flip = t - (cruise.start_s + cruise.aim_at(whole * 0.9).since_s);
            match cruise.at(t).phase {
                crate::flight::Phase::Boost => {
                    boosted = true;
                    assert!(nose.x > 0.999, "boosting and not pointing along the line: {nose}");
                }
                // Past the turn it is round; inside it, it is on the way and neither.
                crate::flight::Phase::Brake if since_flip > flip_s => {
                    braked = true;
                    assert!(nose.x < -0.999, "braking and not pointing back down it: {nose}");
                }
                crate::flight::Phase::Brake => {
                    assert!(nose.x > -0.999, "it flipped faster than the hull can turn: {nose}");
                }
                crate::flight::Phase::Coast => coasted = true,
                _ => {}
            }
        }
        let _ = coasted;
        assert!(boosted, "the crossing never boosted");
        assert!(braked, "the crossing never braked, which is the half this test is about");
    }

    /// **The flip takes the time the hull says, and it happens where the drive is off.**
    ///
    /// Putting the turn at the end of the boost is what makes it free: the drive is already out
    /// for the changeover, so nothing is being thrust in a direction the ship is not facing.
    #[test]
    fn the_flip_is_ordered_when_the_boost_ends_and_takes_a_hulls_turning_time() {
        let cruise =
            crate::flight::Cruise::plan(DVec3::ZERO, DVec3::X * 4.0, 0.0, crate::flight::Drive::DEFAULT);
        let mut state = ShipState::at(DVec3::ZERO);
        state.begin_crossing(cruise.clone(), None);

        let ordered = cruise.aim_at(cruise.duration_s() * 0.9).since_s;
        for hull in [500.0, 50_000.0] {
            let rate = crate::attitude::rate_rad_s(hull);
            let whole = crate::attitude::turn_time_s(DVec3::X, -DVec3::X, rate);
            // At the order it has not moved; halfway it is square on; at the end it is round.
            let at = |dt: f64| facing(&state, hull, ordered + dt).unwrap();
            assert!((at(0.0) - DVec3::X).length() < 1.0e-9, "{hull} m had already turned");
            assert!(at(whole * 0.5).x.abs() < 1.0e-6, "{hull} m was not square on halfway");
            assert!((at(whole) + DVec3::X).length() < 1.0e-6, "{hull} m had not finished");
        }
        // And the big hull takes a hundred times as long over it as the small one.
        let small = crate::attitude::turn_time_s(DVec3::X, -DVec3::X, crate::attitude::rate_rad_s(500.0));
        let large = crate::attitude::turn_time_s(DVec3::X, -DVec3::X, crate::attitude::rate_rad_s(50_000.0));
        assert!((large / small - 100.0).abs() < 1.0e-9);
    }

    /// **A coasting ship does not turn.** Nothing is asking it to, and attitude control is not
    /// free — so it keeps whatever the last order left it pointing at.
    ///
    /// An earlier version pointed a coasting ship along its motion, which left a ship that had
    /// just braked to a halt facing whichever way its last millimeter a second happened to go.
    #[test]
    fn a_coasting_ship_keeps_the_attitude_it_was_left_with() {
        let Some(system) = sol() else { return };
        let mut state = ShipState::at(system.body_position_ly("Earth").unwrap());
        state.attitude = DVec3::new(0.0, 0.0, 1.0);
        let event = Event { ship: ShipId(1), at_t: 0.0, change: orbit("Earth") };
        apply(&mut state, Some(&system), &event).unwrap();
        // Off the station and onto the conic it was already flying, which is what cutting does.
        apply(&mut state, Some(&system), &Event { ship: ShipId(1), at_t: 0.0, change: Change::CutDrive })
            .unwrap();

        for t in [0.0, 600.0, 3_600.0] {
            let nose = facing(&state, 500.0, t).expect("a ship always points somewhere");
            assert!(
                (nose - DVec3::Z).length() < 1.0e-12,
                "it turned to {nose} with nothing asking it to",
            );
        }
        // And emphatically not along the velocity, which is where it used to end up.
        let beta = state_at(&state, Some(&system), 600.0).unwrap().1;
        assert!(beta.normalize().dot(DVec3::Z).abs() < 0.99, "premise: it is not going that way");
    }

    /// A parked ship points where it was left. There is no "nowhere": a hull has an
    /// orientation whether or not anything is deciding it, and the renderer needs one.
    #[test]
    fn a_parked_ship_points_where_it_was_left() {
        let mut state = ShipState::at(DVec3::X);
        assert_eq!(facing(&state, 500.0, 0.0), Some(DVec3::X), "the vernal equinox by default");
        state.attitude = DVec3::new(0.0, 1.0, 0.0);
        assert_eq!(facing(&state, 500.0, 1.0e6), Some(DVec3::Y));
    }
}
