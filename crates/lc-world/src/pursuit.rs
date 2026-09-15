//! Closing on another craft, from what can be seen of it.
//!
//! The one idea here is the **frame**. Matching velocity with something is arriving at rest in
//! its frame, so the approach is planned there and not in the world's: feed
//! [`Cruise::plan_from`] the *relative* position and the *relative* velocity and a destination
//! a standoff short of zero, and the brachistochrone it already solves comes out as a
//! rendezvous. Burn, flip, burn and arrive matched, in one plan rather than a crossing
//! followed by a separate injection.
//!
//! **The frame is what was seen, not what is.** A pursuer has only the light that has reached
//! it, so a plan is anchored at a sighting — where the quarry was, how fast, and when the light
//! left — and dead-reckons forward from there. That is a navigator's job done with a
//! navigator's information, and it is also the only version that does not put faster-than-light
//! knowledge into the pursuer's own trajectory, which the player watches. A target that
//! manoeuvres is therefore chased on stale information until its news arrives, which across a
//! system is seconds to hours.
//!
//! Velocities compose the Galilean way, here and in [`crate::motion::state_at`]'s arm for a
//! rendezvous. The error is second order in the frame's own speed: a millionth at a hundred
//! kilometres a second, which is fast for traffic inside a system, and not usable for matching
//! with something crossing between them at half `c`. [`FRAME_BETA_LIMIT`] is where this
//! refuses rather than answering wrongly.

use glam::DVec3;

use crate::flight::{Cruise, Drive, JULIAN_YEAR_S};
use crate::motion::{Motive, ShipId, ShipState};
use crate::system::M_PER_LY;

/// How many combined hull lengths a craft hangs back at.
///
/// Combined, because the room two ships need is set by both of them: fifty kilometres of ship
/// alongside another fifty is a different proposition from fifty alongside five hundred metres.
/// Five of them is close enough to be formation flying and far enough that neither is
/// manoeuvring inside the other's hull.
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

/// The frame speed past which a Galilean treatment of the relative motion stops being honest.
///
/// A tenth of `c`, where the second-order error is a per cent. Matching with something faster
/// wants the relative motion composed properly and a trajectory that can be boosted, which
/// this is not.
pub const FRAME_BETA_LIMIT: f64 = 0.1;

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
    pub fn reckoned_at(&self, now_s: f64) -> DVec3 {
        self.position_ly + self.beta * (now_s - self.emitted_s) / JULIAN_YEAR_S
    }
}

/// An approach, solved in the frame the quarry appeared to be moving in.
///
/// A **frozen** frame, which is what makes this safe to hand a client: it is three numbers
/// taken from one sighting, not a handle on the quarry's live worldline. A client evaluating it
/// learns where the pursuer goes and nothing whatever about where the quarry went next.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendezvous {
    /// The approach itself, in relative coordinates: from the offset to the quarry, to a
    /// standoff short of it, ending at rest — which in this frame is matched.
    pub cruise: Cruise,
    /// Where the quarry was seen, light-years.
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
/// either relative or a **sighting** — where the quarry was seen, how fast, and when the light
/// left — so it says nothing about the quarry that the receiver's own eyes could not have told
/// it.
#[derive(Clone, Debug, PartialEq)]
pub struct Approach {
    /// Where the pursuer was when this was solved, relative to the quarry's reckoned position.
    pub from_ly: DVec3,
    /// How fast it was going then, relative to the quarry.
    pub beta0: DVec3,
    /// Where the approach ends, relative: a standoff short of the quarry.
    pub to_ly: DVec3,
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

    /// Where the frame's origin — the quarry, dead-reckoned — is at a coordinate time.
    pub fn frame_at(&self, now_s: f64) -> DVec3 {
        self.frame_from_ly + self.frame_beta * (now_s - self.since_t) / JULIAN_YEAR_S
    }

    /// Where the pursuer is, and how fast, in the world frame.
    pub fn state_at(&self, now_s: f64) -> (DVec3, DVec3) {
        let flight = self.cruise.at(now_s);
        (flight.position_ly + self.frame_at(now_s), flight.beta + self.frame_beta)
    }

    pub fn has_arrived(&self, now_s: f64) -> bool {
        self.cruise.has_arrived(now_s)
    }

    /// How far off the plan a fresh sighting puts the quarry, light-years.
    ///
    /// Zero for a quarry that has held its course, because the dead reckoning is then exact.
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
    /// The quarry is moving too fast for the relative motion to be composed this way.
    TooFast,
}

/// Solve the approach from where the pursuer is to a standoff off the quarry.
///
/// `now_s` is the coordinate time the plan is made at, which is *later* than the sighting it is
/// made from — the light took time to arrive. The gap is carried by the dead reckoning rather
/// than ignored.
pub fn approach(
    pursuer: &ShipState,
    pursuer_length_m: f64,
    seen: &Sighting,
    now_s: f64,
    drive: Drive,
) -> Result<Rendezvous, Refused> {
    if seen.beta.length() > FRAME_BETA_LIMIT {
        return Err(Refused::TooFast);
    }
    let standoff_ly = standoff_m(pursuer_length_m, seen.length_m) / M_PER_LY;
    let offset = pursuer.position_ly - seen.reckoned_at(now_s);
    let range = offset.length();
    if range <= standoff_ly {
        return Err(Refused::AlreadyThere);
    }
    // Stopping short on the side the pursuer is already on. Aiming at the quarry itself is
    // aiming to collide, and aiming at any other side is a plan that crosses through it.
    let to = offset / range * standoff_ly;
    let relative_beta = pursuer.beta - seen.beta;
    Ok(Approach {
        from_ly: offset,
        beta0: relative_beta,
        to_ly: to,
        start_s: now_s,
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
pub fn wants_closing(pursuer: &ShipState, pursuer_length_m: f64, seen: &Sighting, now_s: f64) -> bool {
    let standoff_ly = standoff_m(pursuer_length_m, seen.length_m) / M_PER_LY;
    let range = pursuer.position_ly.distance(seen.reckoned_at(now_s));
    range > standoff_ly * DRIFT_ALLOWANCE
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

    /// Refused rather than answered wrongly. A Galilean composition of the relative motion is
    /// a per-cent error at a tenth of `c` and nonsense past that.
    #[test]
    fn matching_with_something_relativistic_is_refused() {
        let seen = quarry(1_000.0, DVec3::Y * 0.5);
        assert_eq!(approach(&pursuer(), 500.0, &seen, 0.0, Drive::DEFAULT), Err(Refused::TooFast));
    }
}
