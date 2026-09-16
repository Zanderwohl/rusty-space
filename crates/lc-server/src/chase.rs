//! What a craft can see of another, and what to do about it.
//!
//! One retarded solve serves both halves of that, and it is the point of keeping them in one
//! module: [`sighting`] is what a client is *told* about a contact and what its ship *steers*
//! by. Two copies of it would be two answers, and the one the player watched would not be the
//! one the autopilot used.
//!
//! Everything here is read-only. Deciding needs to look at the quarry while applying needs the
//! pursuer, and a `Fleet` will not lend both — so the decision comes out as a list and
//! `Server::steer_pursuits` is what acts on it.

use std::collections::HashMap;
use std::sync::Arc;

use lc_proto::{Cleared, ClientId, Presence, ShipId, Withheld};
use lc_spacetime::Worldline;
use lc_spacetime::worldline::retarded_times_at;
use lc_world::craft::{Craft, CraftId, Fleet};
use lc_world::motion::{LIGHT_US_PER_LY, Motive};
use lc_world::pursuit;
use lc_world::system::LOCAL_SHELL_LY;

use crate::server::Connected;

/// A standing intercept, and when it last produced a plan.
#[derive(Clone, Copy, Debug)]
pub struct Pursuit {
    pub quarry: ShipId,
    pub last_plan_t: i64,
}

/// The shortest gap between two plans for one chase, as a fraction of the approach being flown.
///
/// **A cost bound and nothing else.** Whether a plan is still worth flying is already decided,
/// and decided properly, by [`pursuit::wants_replan`]: it extrapolates the frozen sighting the
/// plan was built on to the newest sighting's emission time and throws the plan away when the
/// quarry is not where that said it would be. That test is a *distance*, so it already scales
/// with how the quarry is moving. All this does is stop a quarry under continuous thrust from
/// buying a fresh plan every tick, each of which is an event written to the journal and
/// scheduled to every observer.
///
/// It was a flat ten coordinate minutes, which conflated the two jobs and suppressed the good
/// signal. Ten minutes is far finer than the light delay across a system, which is what it was
/// chosen against — and a craft in high orbit of Jupiter covers **fourteen thousand
/// kilometres** in it. An orbital rendezvous spent the whole approach flying at a ten-minute-old
/// position and settled into a relative orbit a hundred to four hundred kilometres across
/// instead of onto the standoff.
///
/// A fraction of the plan rather than a time, because what a guidance loop owes is a number of
/// corrections across the manoeuvre, not a cadence in seconds — and because the approach
/// *shrinks*. A fixed floor that is reasonable for the first three-thousand-second run at a
/// quarry is hopeless for the sixty-second correction at the end of it, which is exactly how
/// ten minutes came to be too coarse without ever looking wrong.
///
/// **Ten of them, and more is worse.** That is not what you would guess, and it was measured:
/// at a fiftieth of the plan a rendezvous thrashed between three and four hundred million
/// metres before capturing, and at a two-hundredth it did not capture inside two thousand
/// ticks. Each plan is a whole manoeuvre — a burn, a flip and a burn — rather than a
/// controller's output, so re-solving faster than the manoeuvre can run keeps resetting the
/// flip and the pursuer never reaches its brake. Anything from a tenth to a third behaves the
/// same; a tenth is the middle of the plateau.
pub const STEER_FRACTION: f64 = 0.1;

/// The shortest gap before this pursuer may be given another plan, coordinate microseconds.
///
/// Zero with no plan in hand: the first solve of a chase is the one nothing is waiting for.
fn steer_floor_us(pursuer: &Craft) -> i64 {
    let Motive::Rendezvous(plan) = &pursuer.motion.motive else { return 0 };
    (plan.cruise.duration_s() * STEER_FRACTION * crate::world::MICROS_PER_SECOND as f64) as i64
}

/// Whether an observer is entitled to know a craft exists at all.
///
/// Sharing a system, which is the same [`LOCAL_SHELL_LY`] rule both ends already use to decide
/// where a ship is. Not an angular size: a hull five hundred metres long is well under a pixel
/// from anywhere in a system, and a rule drawn there would leave a player unable to find
/// traffic they are sitting in the middle of. Between the stars, where there is no system to
/// share, the same radius serves as a plain range.
///
/// A *visibility* rule and not a causality one. What it decides is which craft are worth
/// solving for; whether the light has arrived is [`Cleared::clear`]'s alone.
pub fn in_sight(observer: &Craft, other: &Craft) -> bool {
    match (&observer.system, &other.system) {
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        (None, None) => {
            observer.motion.position_ly.distance(other.motion.position_ly) < LOCAL_SHELL_LY
        }
        _ => false,
    }
}

/// What one craft can currently see of another, or nothing at all.
pub fn sighting(
    fleet: &Fleet,
    observer: CraftId,
    quarry: ShipId,
    now_t: i64,
) -> Option<pursuit::Sighting> {
    let observer = fleet.get(observer)?;
    let quarry = fleet.get(CraftId(quarry.0))?;
    if quarry.id == observer.id || !in_sight(observer, quarry) {
        return None;
    }
    let here = observer.position_at(now_t as f64);
    let worldline = quarry.worldline();
    // No root means light that has not arrived or has already gone past; there is never more
    // than one for anything sub-luminal.
    let emitted = retarded_times_at(now_t as f64, here, &worldline).first().copied()?;
    Some(pursuit::Sighting {
        target: lc_world::motion::ShipId(quarry.id.0),
        position_ly: worldline.position_at(emitted) / LIGHT_US_PER_LY,
        beta: worldline.velocity_at(emitted),
        length_m: quarry.length_m,
        emitted_s: emitted * 1.0e-6,
    })
}

/// Where everybody else appeared to be, per connection.
///
/// Every contact is a **retarded** sample. A craft a light-hour away is reported where it was
/// an hour ago, and the emission time is solved against the observer's own worldline rather
/// than subtracted from a shared clock — which is what makes it right for an observer that is
/// itself moving fast.
pub fn contacts(
    fleet: &Fleet,
    clients: &HashMap<ClientId, Connected>,
    now_t: i64,
) -> HashMap<ClientId, Vec<Cleared<Presence>>> {
    let mut out = HashMap::new();
    for (id, state) in clients {
        let Some(observer) = fleet.get(CraftId(state.ship.0)) else { continue };
        let mut seen = Vec::new();
        for craft in fleet.iter() {
            let Some(sighted) = sighting(fleet, observer.id, ShipId(craft.id.0), now_t) else {
                continue;
            };
            let presence = Presence {
                ship_id: ShipId(craft.id.0),
                name: craft.designation(),
                length_m: craft.length_m,
                at_ly: sighted.position_ly.to_array(),
                beta: sighted.beta.to_array(),
                // Where the nose actually was when the light left, part-way through a turn
                // included. A craft always has one now — see `lc_world::attitude` — so there is
                // no "undecided" case left for the receiver to paper over.
                facing: craft
                    .facing_at(sighted.emitted_s)
                    .unwrap_or(glam::DVec3::X)
                    .to_array(),
                // At the moment the light left, not now. A burn that has since stopped is
                // still burning as far as this observer is concerned.
                jet_power_w: craft.jet_power_w(sighted.emitted_s),
                emitted_t: (sighted.emitted_s * 1.0e6) as i64,
                // The solve *is* the arrival: `emitted + |x_o - w(emitted)|` equals `now` by
                // construction, so this is the light landing at this instant.
                arrive_t: now_t,
            };
            match Cleared::<Presence>::clear(presence, now_t) {
                Ok(pass) => seen.push(pass),
                // Only reachable if the solve returned a root in the observer's future, which
                // it cannot. Dropped rather than trusted: the gate is the authority here and
                // the solver is not.
                Err(Withheld::StillInFlight | Withheld::BelowNoiseFloor) => {}
            }
        }
        out.insert(*id, seen);
    }
    out
}

/// Whether a craft flying a standing intercept should be given a new plan this tick.
///
/// Two different questions wearing one name. A ship still flying an approach is asked whether
/// its quarry has stopped agreeing with it — which is the whole manoeuvre response, and which
/// it cannot notice until the light of the manoeuvre arrives. A ship that has arrived, or is
/// doing anything else, is asked whether it has drifted off station.
pub fn should_close(pursuer: &Craft, seen: &pursuit::Sighting, now_s: f64) -> bool {
    match &pursuer.motion.motive {
        Motive::Rendezvous(plan) if plan.target == seen.target && !plan.has_arrived(now_s) => {
            pursuit::wants_replan(&pursuer.motion.motive, seen, pursuer.length_m, now_s)
        }
        _ => pursuit::wants_closing(&pursuer.motion, pursuer.length_m, seen, now_s),
    }
}

/// What each standing intercept wants done this tick.
///
/// `None` against a craft means give the pursuit up: its quarry has gone out of sight, or is
/// moving too fast to match the way this plans a match. An entry missing altogether means
/// carry on doing whatever it is doing.
pub fn decide(
    fleet: &Fleet,
    pursuits: &HashMap<CraftId, Pursuit>,
    now_t: i64,
) -> Vec<(CraftId, Option<pursuit::Rendezvous>)> {
    let now_s = now_t as f64 * 1.0e-6;
    let mut decided = Vec::new();
    for (id, pursuit) in pursuits {
        let Some(pursuer) = fleet.get(*id) else { continue };
        // Out of sight is the end of it. A policy that survived would be one waiting to act on
        // a craft this one is no longer entitled to know about.
        let Some(seen) = sighting(fleet, *id, pursuit.quarry, now_t) else {
            decided.push((*id, None));
            continue;
        };
        // Saturating, because "never planned" is a legitimate thing for a caller to say and
        // the obvious way to say it overflows the subtraction.
        let since = now_t.saturating_sub(pursuit.last_plan_t);
        if since < steer_floor_us(pursuer) || !should_close(pursuer, &seen, now_s) {
            continue;
        }
        match pursuit::approach(
            &pursuer.motion,
            pursuer.length_m,
            &seen,
            now_s,
            pursuer.turning(pursuer.motion.drive),
        ) {
            Ok(plan) => decided.push((*id, Some(plan))),
            // On station. Nothing to fly, and the policy stays: it is what will notice the
            // next time this craft has drifted.
            Err(pursuit::Refused::AlreadyThere) => {}
            Err(pursuit::Refused::TooFast) => decided.push((*id, None)),
        }
    }
    decided
}
