//! Closing on another craft, from what can be seen of it.
//!
//! The one idea here is the **frame**. Matching velocity with something is arriving at rest in
//! its frame, so the approach is planned there and not in the world's: boost into the frame the
//! quarry is at rest in, hand [`Cruise::plan_from`] the pursuer's position and velocity *as
//! measured there* and a destination a standoff short of the origin, and the brachistochrone it
//! already solves comes out as a rendezvous. Burn, flip, burn and arrive matched, in one plan
//! rather than a crossing followed by a separate injection.
//!
//! The boost is a real one — see [`crate::boost`] — so this holds at any speed a ship can
//! reach. Velocities compose rather than subtract, the plan's own coordinates are related to
//! the world's by a Lorentz transformation and not by adding a drift, and the relativity of
//! simultaneity is carried rather than ignored. An earlier version did all three the Galilean
//! way and had to refuse anything past a tenth of `c`.
//!
//! **The frame is what was seen, not what is.** A pursuer has only the light that has reached
//! it, so a plan is anchored at a sighting — where the quarry was, how fast, and when the light
//! left — and dead-reckons forward from there. That is a navigator's job done with a
//! navigator's information, and it is also the only version that does not put faster-than-light
//! knowledge into the pursuer's own trajectory, which the player watches. A target that
//! manoeuvres is therefore chased on stale information until its news arrives, which across a
//! system is seconds to hours.

use glam::DVec3;

use crate::boost::{self, Event};
use crate::flight::{C_M_S, Cruise, Drive, JULIAN_YEAR_S, MAX_BETA};
use crate::motion::{Motive, ShipId, ShipState};
use crate::system::M_PER_LY;

/// How many combined hull lengths a craft hangs back at.
///
/// Combined, because the room two ships need is set by both of them: fifty kilometres of ship
/// alongside another fifty is a different proposition from fifty alongside five hundred metres.
/// Five of them is close enough to be formation flying and far enough that neither is
/// manoeuvring inside the other's hull.
///
/// A **proper** distance, measured in the frame the two of them end up sharing. Anything else
/// would have two ships closing at speed park closer together than two at rest, because the
/// world frame sees the gap between them contracted.
pub const STANDOFF_LENGTHS: f64 = 5.0;

/// How far a craft may drift from its station before it closes again, as a multiple of the
/// standoff.
///
/// A deadband and not a tolerance. Without one, a pair jostling about the station would re-plan
/// every tick for the rest of time; with it, a craft coasts until it has genuinely wandered and
/// then makes one correction.
pub const DRIFT_ALLOWANCE: f64 = 2.0;

/// How far the quarry may be from where the standing plan predicted before the plan is thrown
/// away, as a multiple of the standoff.
///
/// This is the whole of the manoeuvre response. A quarry holding its course stays inside it
/// forever and the plan runs to completion; one under thrust leaves it almost at once and is
/// re-solved against, which from outside is a pursuer tracking a burn. Deliberately not a
/// separate "match acceleration" mode: torch ships have thrust to spare, and one rule that
/// covers both is one rule that cannot disagree with itself at the boundary.
pub const REPLAN_FRACTION: f64 = 0.25;

/// A craft as its pursuer currently sees it: a sighting, and therefore the past.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sighting {
    pub target: ShipId,
    /// Light-years from the world origin, where the light left.
    pub position_ly: DVec3,
    /// How fast it was going then.
    pub beta: DVec3,
    pub length_m: f64,
    /// Coordinate seconds the light left, which is what the dead reckoning runs from.
    pub emitted_s: f64,
}

impl Sighting {
    /// Where the quarry would be now if it has held its course since.
    ///
    /// Dead reckoning, and the only kind of extrapolation a pursuer is entitled to: it uses
    /// what arrived and nothing else. Wrong exactly when the quarry has manoeuvred since, which
    /// is what [`REPLAN_FRACTION`] notices when the news of that finally lands.
    ///
    /// World coordinates throughout, so this is a straight line and needs no boost: an inertial
    /// worldline is inertial in every frame.
    pub fn reckoned_at(&self, now_s: f64) -> DVec3 {
        self.position_ly + self.beta * (now_s - self.emitted_s) / JULIAN_YEAR_S
    }
}

/// An approach, solved in the frame the quarry appeared to be at rest in.
///
/// A **frozen** frame, which is what makes this safe to hand a client: it is three numbers
/// taken from one sighting, not a handle on the quarry's live worldline. A client evaluating it
/// learns where the pursuer goes and nothing whatever about where the quarry went next.
///
/// [`Rendezvous::cruise`] is in that frame's own coordinates — its positions and, importantly,
/// its *times* are not the world's. Reading it back out is [`Rendezvous::state_at`].
#[derive(Clone, Debug, PartialEq)]
pub struct Rendezvous {
    /// The approach itself, in the quarry's frame: from where the pursuer is to a standoff
    /// short of the origin, ending at rest — which in this frame is matched.
    pub cruise: Cruise,
    /// Where the quarry was seen, light-years. The frame is pinned to this event.
    pub frame_from_ly: DVec3,
    /// How fast it was seen going.
    pub frame_beta: DVec3,
    /// Coordinate seconds the frame is anchored at: the moment the light left.
    pub since_t: f64,
    /// Who is being closed on, so the pursuit can be re-solved and the interface can name it.
    pub target: ShipId,
}

/// A rendezvous as the arguments it was solved from.
///
/// The same "recipe, never the trajectory" rule the rest of [`crate::resume`] follows: this is
/// what crosses a wire and goes on disk, and the far end re-solves it. Every number in it is
/// either measured in the quarry's frame or a **sighting** — where the quarry was seen, how
/// fast, and when the light left — so it says nothing about the quarry that the receiver's own
/// eyes could not have told it.
#[derive(Clone, Debug, PartialEq)]
pub struct Approach {
    /// Where the pursuer was when this was solved, in the quarry's frame.
    pub from_ly: DVec3,
    /// How fast it was going then, in the quarry's frame.
    pub beta0: DVec3,
    /// Where the approach ends: a standoff short of the quarry, in its frame.
    pub to_ly: DVec3,
    /// When it begins, in the quarry's frame's own seconds.
    pub start_s: f64,
    pub drive: Drive,
    pub frame_from_ly: DVec3,
    pub frame_beta: DVec3,
    pub since_t: f64,
    pub target: ShipId,
}

impl Approach {
    /// Solve it. The same call [`approach`] makes, so a restored plan and the original are the
    /// same trajectory rather than two that agree to a tolerance.
    pub fn solve(&self) -> Rendezvous {
        Rendezvous {
            cruise: Cruise::plan_from(self.from_ly, self.beta0, self.to_ly, self.start_s, self.drive),
            frame_from_ly: self.frame_from_ly,
            frame_beta: self.frame_beta,
            since_t: self.since_t,
            target: self.target,
        }
    }
}

impl Rendezvous {
    /// The arguments this was solved from, to put it back.
    pub fn recipe(&self) -> Approach {
        Approach {
            from_ly: self.cruise.from_ly,
            beta0: self.cruise.initial_beta(),
            to_ly: self.cruise.to_ly,
            start_s: self.cruise.start_s,
            drive: self.cruise.drive,
            frame_from_ly: self.frame_from_ly,
            frame_beta: self.frame_beta,
            since_t: self.since_t,
            target: self.target,
        }
    }

    /// Where the frame's origin — the quarry, dead-reckoned — is at a **world** time.
    pub fn frame_at(&self, now_s: f64) -> DVec3 {
        self.frame_from_ly + self.frame_beta * (now_s - self.since_t) / JULIAN_YEAR_S
    }

    /// World seconds elapsed at a given moment of the frame's own clock. The inverse of
    /// [`Rendezvous::frame_time_at`], and the cheap direction: no root find, just the boost.
    fn world_time_at(&self, frame_s: f64) -> f64 {
        let x = self.cruise.at(frame_s).position_ly * JULIAN_YEAR_S;
        boost::gamma_of(self.frame_beta) * (frame_s + self.frame_beta.dot(x))
    }

    /// The frame's own time at which the pursuer is at a given world time.
    ///
    /// The inversion that relativity makes necessary. `t = γ(t' + β·x'(t'))`, and `x'` depends
    /// on `t'`, so this is a root find rather than a division — the frame's clock and the
    /// world's do not merely run at different rates, they disagree about which events are
    /// simultaneous, and by an amount that changes as the ship moves through the frame.
    ///
    /// Always exactly one root: `dt/dt' = γ(1 + β·β')` and `|β'| < 1`, so it rises at no less
    /// than `γ(1 − |β|) > 0`. Newton on a bracket, which converges in a handful of steps
    /// because the profile is smooth inside each phase of the burn.
    fn frame_time_at(&self, elapsed_s: f64) -> f64 {
        let beta = self.frame_beta;
        if beta.length_squared() <= 0.0 {
            return elapsed_s;
        }
        let gamma = boost::gamma_of(beta);
        // World seconds elapsed, as a function of the frame's own elapsed seconds.
        let world_at = |t: f64| {
            let x = self.cruise.at(t).position_ly * JULIAN_YEAR_S;
            gamma * (t + beta.dot(x))
        };

        // The cruise holds its endpoints outside its own span, so `β·x'` is bounded and a
        // bracket is found by doubling out from the answer a drift alone would give.
        let mut span = (elapsed_s.abs() / gamma).max(1.0);
        let guess = elapsed_s / gamma;
        let (mut lo, mut hi) = (guess - span, guess + span);
        for _ in 0..64 {
            if world_at(lo) <= elapsed_s && world_at(hi) >= elapsed_s {
                break;
            }
            span *= 2.0;
            lo = guess - span;
            hi = guess + span;
        }

        let mut t = guess.clamp(lo, hi);
        for _ in 0..48 {
            let at = self.cruise.at(t);
            let x = at.position_ly * JULIAN_YEAR_S;
            let error = gamma * (t + beta.dot(x)) - elapsed_s;
            if error > 0.0 {
                hi = t;
            } else {
                lo = t;
            }
            // `β·β'` cannot reach −1, so this cannot vanish.
            let slope = gamma * (1.0 + beta.dot(at.beta));
            let mut next = t - error / slope;
            if !(next > lo && next < hi) {
                next = 0.5 * (lo + hi);
            }
            let tolerance = 4.0 * f64::EPSILON * t.abs().max(1.0);
            if (next - t).abs() <= tolerance || (hi - lo) <= tolerance {
                return next;
            }
            t = next;
        }
        t
    }

    /// The approach sampled at a **world** time.
    ///
    /// The one place the two clocks are reconciled. Everything that asks a question about a
    /// rendezvous at a world time goes through here, because a cruise keeps the quarry frame's
    /// time and handing it a world one is a mistake with no symptom until something arrives
    /// late — the crew's clock, the arrival transition and the thrust direction all read off
    /// this, and all three were wrong when `advance` sampled the cruise directly.
    pub fn flight_at(&self, now_s: f64) -> crate::flight::FlightState {
        self.cruise.at(self.frame_time_at(now_s - self.since_t))
    }

    /// Where the pursuer is, and how fast, in world coordinates at a world time.
    ///
    /// Built as *the quarry's position plus an offset* rather than transformed straight out of
    /// the frame, and that is a numerical decision rather than a stylistic one. The two forms
    /// are the same algebra with the `γt'` term cancelled by hand, and that term is enormous:
    /// shedding `0.99c` at five gravities takes years and carries the frame's anchor event tens
    /// of light-years away, so the direct form recovers a five-kilometre standoff by
    /// subtracting two numbers of order fifty light-years and gets about a hundred bits of
    /// signal. See [`boost::separation_in_world`].
    pub fn state_at(&self, now_s: f64) -> (DVec3, DVec3) {
        let flight = self.flight_at(now_s);
        let offset =
            boost::separation_in_world(flight.position_ly * JULIAN_YEAR_S, self.frame_beta);
        (
            self.frame_at(now_s) + offset / JULIAN_YEAR_S,
            boost::velocity_from_frame(flight.beta, self.frame_beta),
        )
    }

    /// What the approach is asking the nose to do, in world axes, at a world time.
    ///
    /// The cruise's own aim, asked in the frame's time and aberrated back — the same two
    /// corrections [`Rendezvous::thrust_at`] makes, and for the same reasons.
    pub fn aim_at(&self, now_s: f64) -> crate::flight::Aim {
        let aim = self.cruise.aim_at(self.frame_time_at(now_s - self.since_t));
        let out = |v: DVec3| boost::velocity_from_frame(v, self.frame_beta).normalize_or_zero();
        crate::flight::Aim {
            to: out(aim.to),
            from: aim.from.map(out),
            // Back into world time, so the turn is measured against the clock the caller is
            // holding rather than the quarry's.
            since_s: self.since_t + self.world_time_at(aim.since_s),
        }
    }

    /// Which way the drive points at a world time, in world axes. Zero where it is not lit.
    ///
    /// Aberrated into the world frame, which is what happens to a direction under a boost — at
    /// speed the nose swings forward of where the frame it is thrusting in would put it. What
    /// is *not* attempted is the rest of relativistic rendering: the hull is not contracted and
    /// its silhouette is not aberrated, so this is the orientation of a shape that is itself
    /// drawn unrelativistically.
    pub fn thrust_at(&self, now_s: f64) -> DVec3 {
        // The cruise already knows which way its drive points; it only has to be asked in its
        // own time rather than the world's.
        let thrust = self.cruise.thrust_at(self.frame_time_at(now_s - self.since_t));
        if thrust == DVec3::ZERO {
            return DVec3::ZERO;
        }
        boost::velocity_from_frame(thrust, self.frame_beta).normalize_or_zero()
    }

    pub fn has_arrived(&self, now_s: f64) -> bool {
        self.cruise.has_arrived(self.frame_time_at(now_s - self.since_t))
    }

    /// How far off the plan a fresh sighting puts the quarry, light-years.
    ///
    /// Zero for a quarry that has held its course, because the dead reckoning is then exact.
    /// World coordinates on both sides, so no boost: this is a question about two claims
    /// concerning the same frame.
    pub fn divergence(&self, seen: &Sighting) -> f64 {
        self.frame_at(seen.emitted_s).distance(seen.position_ly)
    }
}

/// How far a craft of `mine` metres hangs back from one of `theirs`, metres.
pub fn standoff_m(mine: f64, theirs: f64) -> f64 {
    (mine + theirs) * STANDOFF_LENGTHS
}

/// Plan an approach, or say why there is not one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Already on station. Nothing to fly; hold and watch the deadband.
    AlreadyThere,
    /// The quarry is at or past `c`, where there is no frame to match with. Nothing a ship can
    /// do reaches this; a corrupt or hostile number can.
    TooFast,
}

/// Solve the approach from where the pursuer is to a standoff off the quarry.
///
/// `now_s` is the world time the plan is made at, which is *later* than the sighting it is made
/// from — the light took time to arrive. The gap is carried by the dead reckoning rather than
/// ignored.
pub fn approach(
    pursuer: &ShipState,
    pursuer_length_m: f64,
    seen: &Sighting,
    now_s: f64,
    drive: Drive,
) -> Result<Rendezvous, Refused> {
    if seen.beta.length() >= MAX_BETA {
        return Err(Refused::TooFast);
    }
    // Into the quarry's frame, pinned to the sighting. Everything from here to the plan is
    // measured there, including the standoff, which is a proper distance.
    let here = boost::to_frame(
        Event {
            t: now_s - seen.emitted_s,
            x: (pursuer.position_ly - seen.position_ly) * JULIAN_YEAR_S,
        },
        seen.beta,
    );
    let standoff_ls = standoff_m(pursuer_length_m, seen.length_m) / C_M_S;
    let range = here.x.length();
    if range <= standoff_ls {
        return Err(Refused::AlreadyThere);
    }
    // Stopping short on the side the pursuer is already on. Aiming at the quarry itself is
    // aiming to collide, and aiming at any other side is a plan that crosses through it.
    let to = here.x / range * standoff_ls;
    Ok(Approach {
        from_ly: here.x / JULIAN_YEAR_S,
        beta0: boost::velocity_to_frame(pursuer.beta, seen.beta),
        to_ly: to / JULIAN_YEAR_S,
        start_s: here.t,
        drive,
        frame_from_ly: seen.position_ly,
        frame_beta: seen.beta,
        since_t: seen.emitted_s,
        target: seen.target,
    }
    .solve())
}

/// Whether a standing plan is still worth flying, given the newest sighting.
///
/// Three ways it stops being: the quarry is not where the plan said it would be, the plan has
/// run out, or there is no plan.
pub fn wants_replan(motive: &Motive, seen: &Sighting, pursuer_length_m: f64, now_s: f64) -> bool {
    let Motive::Rendezvous(plan) = motive else { return true };
    if plan.target != seen.target || plan.has_arrived(now_s) {
        return true;
    }
    let standoff_ly = standoff_m(pursuer_length_m, seen.length_m) / M_PER_LY;
    plan.divergence(seen) > standoff_ly * REPLAN_FRACTION
}

/// Whether a craft on station has wandered far enough to be worth closing again.
///
/// Measured in the frame the pair share, because the standoff is a proper distance. Two ships
/// running together at speed are closer in the world's reckoning than in their own, and a
/// deadband applied to the world's number would let them converge as they accelerated.
pub fn wants_closing(pursuer: &ShipState, pursuer_length_m: f64, seen: &Sighting, now_s: f64) -> bool {
    let standoff_ls = standoff_m(pursuer_length_m, seen.length_m) / C_M_S;
    let separation = (pursuer.position_ly - seen.reckoned_at(now_s)) * JULIAN_YEAR_S;
    boost::separation_in_frame(separation, seen.beta) > standoff_ls * DRIFT_ALLOWANCE
}

#[cfg(test)]
mod tests {
    use super::*;

    const KM: f64 = 1.0e3 / M_PER_LY;

    fn quarry(at_km: f64, beta: DVec3) -> Sighting {
        Sighting {
            target: ShipId(2),
            position_ly: DVec3::X * at_km * KM,
            beta,
            length_m: 500.0,
            emitted_s: 0.0,
        }
    }

    /// How long a plan takes in **world** seconds, which is not its cruise's duration: that is
    /// measured on the quarry frame's clock.
    fn world_duration(plan: &Rendezvous) -> f64 {
        let end = plan.cruise.start_s + plan.cruise.duration_s();
        let x = plan.cruise.at(end).position_ly * JULIAN_YEAR_S;
        boost::gamma_of(plan.frame_beta) * (end + plan.frame_beta.dot(x))
    }

    fn pursuer() -> ShipState {
        ShipState::at(DVec3::ZERO)
    }

    /// **The point of planning in the quarry's frame.** A plan that ends at rest *there* ends
    /// matched *here*, so there is no separate injection burn and no moment where the pursuer
    /// is parked while the quarry sails past.
    #[test]
    fn an_approach_ends_alongside_and_moving_at_the_quarrys_speed() {
        let drifting = DVec3::Y * 1.0e-4;
        let seen = quarry(1_000.0, drifting);
        let plan = approach(&pursuer(), 500.0, &seen, 0.0, Drive::DEFAULT).expect("a plan");

        let done = plan.since_t + plan.cruise.duration_s();
        let (at, beta) = plan.state_at(done);
        // Alongside: the standoff, and not the quarry's own position.
        let standoff_ly = standoff_m(500.0, 500.0) / M_PER_LY;
        let gap = at.distance(seen.reckoned_at(done));
        assert!(
            (gap - standoff_ly).abs() < standoff_ly * 1.0e-6,
            "ended {gap} from the quarry, wanted {standoff_ly}",
        );
        // And matched: what is left is the quarry's own velocity, not zero.
        assert!(
            (beta - drifting).length() < drifting.length() * 1.0e-6,
            "ended at {beta}, wanted {drifting}",
        );
        assert!(beta.length() > 0.0, "matching a moving quarry is not stopping");
    }

    /// The pursuer starts where it is and goes where it is told; the frame does not teleport it.
    #[test]
    fn an_approach_begins_where_the_pursuer_actually_is() {
        let seen = quarry(1_000.0, DVec3::Y * 1.0e-5);
        let mut me = pursuer();
        me.position_ly = DVec3::new(0.0, 3.0 * KM, -2.0 * KM);
        let plan = approach(&me, 500.0, &seen, 0.0, Drive::DEFAULT).expect("a plan");
        let (at, beta) = plan.state_at(0.0);
        assert!(at.distance(me.position_ly) < 1.0e-12 * KM, "started at {at}, not {}", me.position_ly);
        // Not exactly at rest, and the residual is not this module's. `Cruise::plan_from`
        // sheds the across-the-line part of its initial velocity in a match burn and carries
        // the along-the-line part through it unchanged, which is a straight-line stand-in for
        // a path that actually bends — so it reproduces the velocity it was given to a
        // fraction of it rather than to the bit. What matters here is that the plan starts
        // from the pursuer's own motion at all, rather than assuming it began at rest.
        let relative = seen.beta.length();
        assert!(
            beta.length() < relative * 1.0e-3,
            "started at {beta}, which is not small against a relative {relative}",
        );
    }

    /// Dead reckoning is what closes the gap between a sighting and the present. A plan made
    /// from hour-old light must aim where the quarry has got to, not where the light says.
    #[test]
    fn a_plan_aims_at_the_reckoned_position_and_not_the_seen_one() {
        let beta = DVec3::Y * 1.0e-4;
        let seen = quarry(1_000.0, beta);
        let hour = 3_600.0;
        let plan = approach(&pursuer(), 500.0, &seen, hour, Drive::DEFAULT).expect("a plan");
        let moved = beta.length() * hour / JULIAN_YEAR_S;
        assert!(moved > 0.0, "premise: the quarry went somewhere in an hour");
        let reckoned = seen.reckoned_at(hour);
        assert!(reckoned.distance(seen.position_ly) > 0.0);
        // The frame is anchored at the sighting and carries the reckoning itself.
        assert_eq!(plan.since_t, seen.emitted_s);
        assert!(plan.frame_at(hour).distance(reckoned) < 1.0e-15);
    }

    /// A quarry holding its course never diverges from the plan, and one that manoeuvres does
    /// so at once. That difference is the whole manoeuvre response.
    #[test]
    fn only_a_manoeuvre_throws_a_plan_away() {
        let held = quarry(1_000.0, DVec3::Y * 1.0e-5);
        let plan = approach(&pursuer(), 500.0, &held, 0.0, Drive::DEFAULT).expect("a plan");
        let motive = Motive::Rendezvous(plan.clone());

        // The same quarry, seen again later, still on its course.
        let later = Sighting { position_ly: held.reckoned_at(600.0), emitted_s: 600.0, ..held };
        assert_eq!(plan.divergence(&later), 0.0, "dead reckoning is exact for a held course");
        assert!(!wants_replan(&motive, &later, 500.0, 300.0));

        // And one that has been under thrust since.
        let standoff_ly = standoff_m(500.0, 500.0) / M_PER_LY;
        let swerved = Sighting {
            position_ly: later.position_ly + DVec3::Z * standoff_ly,
            ..later
        };
        assert!(wants_replan(&motive, &swerved, 500.0, 300.0));
    }

    /// A plan put back from its arguments is the same plan, not one that agrees to a
    /// tolerance. Everything downstream — where the ship is, when it arrives — reads off the
    /// solved cruise, so a restore that re-solved differently would move a ship on reconnect.
    #[test]
    fn a_plan_survives_being_reduced_to_its_arguments() {
        let seen = quarry(1_000.0, DVec3::Y * 1.0e-4);
        let plan = approach(&pursuer(), 500.0, &seen, 120.0, Drive::DEFAULT).expect("a plan");
        assert_eq!(plan.recipe().solve(), plan);
    }

    /// A plan for somebody else, or one that has run out, is not a plan for this.
    #[test]
    fn a_finished_or_misaddressed_plan_is_replanned() {
        let seen = quarry(1_000.0, DVec3::ZERO);
        let plan = approach(&pursuer(), 500.0, &seen, 0.0, Drive::DEFAULT).expect("a plan");
        let done = plan.since_t + plan.cruise.duration_s() + 1.0;
        assert!(wants_replan(&Motive::Rendezvous(plan.clone()), &seen, 500.0, done));

        let elsewhere = Sighting { target: ShipId(9), ..seen };
        assert!(wants_replan(&Motive::Rendezvous(plan), &elsewhere, 500.0, 0.0));
        assert!(wants_replan(&Motive::Drifting { from_ly: DVec3::ZERO, since_t: 0.0 }, &seen, 500.0, 0.0));
    }

    /// The standoff is set by both hulls, so a big ship is given room by a small one and not
    /// the other way about.
    #[test]
    fn the_standoff_grows_with_either_ship() {
        let small = standoff_m(500.0, 500.0);
        assert!(standoff_m(50_000.0, 500.0) > small * 10.0);
        assert_eq!(standoff_m(500.0, 50_000.0), standoff_m(50_000.0, 500.0), "it is symmetric");
    }

    /// Already on station is not a failure and not a flight: there is nowhere to go.
    #[test]
    fn a_pursuer_already_alongside_is_refused_a_plan() {
        let seen = quarry(0.001, DVec3::ZERO);
        assert_eq!(
            approach(&pursuer(), 500.0, &seen, 0.0, Drive::DEFAULT),
            Err(Refused::AlreadyThere),
        );
    }

    /// The deadband. A craft on station holds until it has genuinely wandered, or a pair would
    /// re-plan against each other every tick forever.
    #[test]
    fn station_keeping_waits_for_a_real_drift() {
        let seen = quarry(0.0, DVec3::ZERO);
        let standoff_ly = standoff_m(500.0, 500.0) / M_PER_LY;
        let mut me = pursuer();

        me.position_ly = DVec3::X * standoff_ly * 1.5;
        assert!(!wants_closing(&me, 500.0, &seen, 0.0), "inside the deadband");
        me.position_ly = DVec3::X * standoff_ly * (DRIFT_ALLOWANCE + 0.1);
        assert!(wants_closing(&me, 500.0, &seen, 0.0), "outside it");
    }

    /// **A chase at any speed, and the match still comes out exact.**
    ///
    /// The case the Galilean version had to refuse at a tenth of `c`. Everything that makes it
    /// hard is here at once: the velocities cannot be subtracted, the plan's coordinates are
    /// related to the world's by a boost rather than a drift, and the frame's clock disagrees
    /// with the world's about which events are simultaneous.
    ///
    /// The top of the range is worth reading twice. At `0.9999c` the Lorentz factor is seventy,
    /// shedding that much relative velocity at five gravities takes the better part of a
    /// century, and the frame's anchor event finishes some hundreds of light-years astern — and
    /// the standoff still comes out to a part in ten thousand of five kilometres. That is the
    /// algebraic cancellation in [`boost::separation_in_world`] doing its job; without it this
    /// case has no significant figures left at all.
    #[test]
    fn a_quarry_at_any_speed_is_matched_exactly() {
        for beta in [0.5, 0.9, 0.99, 0.999, 0.9999] {
            let running = DVec3::Y * beta;
            let seen = quarry(1_000.0, running);
            let plan = approach(&pursuer(), 500.0, &seen, 0.0, Drive::DEFAULT)
                .unwrap_or_else(|why| panic!("{beta}c was refused: {why:?}"));

            // Flown to the end, in world time.
            let done = plan.since_t + world_duration(&plan);
            let (at, ended) = plan.state_at(done);

            // Matched: what is left is the quarry's own velocity, composed rather than added.
            assert!(
                (ended - running).length() < 1.0e-6,
                "{beta}c ended at {ended} rather than {running}",
            );
            // Alongside, at the standoff measured where a standoff means something — in the
            // frame the two of them now share.
            let standoff_ls = standoff_m(500.0, seen.length_m) / C_M_S;
            let separation = (at - seen.reckoned_at(done)) * JULIAN_YEAR_S;
            let gap = boost::separation_in_frame(separation, running);
            assert!(
                (gap - standoff_ls).abs() < standoff_ls * 1.0e-4,
                "{beta}c ended {gap} light-seconds off, wanted {standoff_ls}",
            );
        }
    }

    /// Nothing a ship can do is refused. Only a velocity at or past `c`, which is not a frame
    /// at all and can only arrive from something corrupt.
    #[test]
    fn only_a_quarry_at_c_has_no_frame_to_match() {
        let seen = quarry(1_000.0, DVec3::Y * 0.9999);
        assert!(approach(&pursuer(), 500.0, &seen, 0.0, Drive::DEFAULT).is_ok());
        let past = quarry(1_000.0, DVec3::Y * 1.5);
        assert_eq!(approach(&pursuer(), 500.0, &past, 0.0, Drive::DEFAULT), Err(Refused::TooFast));
    }

    /// The world clock and the frame's are inverses of each other, which is what everything
    /// read back out of a plan depends on.
    #[test]
    fn the_two_clocks_invert_each_other() {
        let running = DVec3::new(0.0, 0.8, -0.2);
        let seen = quarry(1_000.0, running);
        let plan = approach(&pursuer(), 500.0, &seen, 0.0, Drive::DEFAULT).expect("a plan");
        let gamma = boost::gamma_of(running);
        for step in 0..12 {
            let elapsed = world_duration(&plan) * step as f64 / 11.0;
            let frame_t = plan.frame_time_at(elapsed);
            let x = plan.cruise.at(frame_t).position_ly * JULIAN_YEAR_S;
            let back = gamma * (frame_t + running.dot(x));
            assert!(
                (back - elapsed).abs() < 1.0e-6 * elapsed.abs().max(1.0),
                "{elapsed} went to {frame_t} and came back {back}",
            );
        }
    }

    /// At everyday speeds it has to give what the Galilean arithmetic it replaced gave, or
    /// every number tuned against that one is now wrong.
    #[test]
    fn a_slow_chase_is_the_answer_the_old_arithmetic_gave() {
        let creeping = DVec3::Y * 1.0e-5;
        let seen = quarry(1_000.0, creeping);
        let plan = approach(&pursuer(), 500.0, &seen, 0.0, Drive::DEFAULT).expect("a plan");
        // The frame's clock and the world's run together to within a part in 1e10.
        let elapsed = world_duration(&plan);
        assert!((plan.frame_time_at(elapsed) - elapsed).abs() < elapsed * 1.0e-9);
        // And the plan starts where the Galilean one did: the pursuer's own offset and its
        // velocity less the quarry's.
        let offset = (pursuer().position_ly - seen.reckoned_at(0.0)).length();
        assert!((plan.cruise.from_ly.length() - offset).abs() < offset * 1.0e-9);
        assert!((plan.cruise.initial_beta() - (-creeping)).length() < 1.0e-14);
    }

}
