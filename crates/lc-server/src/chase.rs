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

use lc_proto::{Cleared, ClientId, Presence, ShipId, Withheld};
use lc_spacetime::Worldline;
use lc_spacetime::worldline::retarded_times_at;
use lc_world::craft::{Craft, CraftId, Fleet};
use lc_world::consort;
use lc_world::courtesy::{Arrival, Manners};
use lc_world::escort;
use lc_world::fitting::Balance;
use lc_world::motion::{LIGHT_US_PER_LY, Motive};
use lc_world::pursuit::{self, Closeness, Refused};
use lc_world::system::LOCAL_SHELL_LY;

use crate::server::Connected;

/// A standing intercept, and when it last produced a plan.
#[derive(Clone, Copy, Debug)]
pub struct Pursuit {
    pub quarry: ShipId,
    pub closeness: Closeness,
    pub approach: lc_proto::Approach,
    pub last_plan_t: i64,
    /// The sighting before the newest, which is the only way a pursuer learns how hard its
    /// quarry is burning: two things that arrived, and the difference between them.
    pub last_seen: Option<pursuit::Sighting>,
}

/// What guidance decided to fly.
#[derive(Clone, Debug, PartialEq)]
pub enum Plan {
    /// Close on a quarry that is coasting, and arrive matched.
    Rendezvous(pursuit::Rendezvous),
    /// Close on a quarry that is burning, and stay with it. See [`lc_world::escort`].
    Escort(escort::Escort),
    /// Close on a quarry that is falling, and stay with it. See [`lc_world::consort`].
    Consort(consort::Consort),
}

impl Plan {
    /// Put a craft on it.
    pub fn fly(self, craft: &mut Craft, now_s: f64) {
        match self {
            Plan::Rendezvous(plan) => craft.begin_rendezvous(plan, now_s),
            Plan::Escort(plan) => craft.begin_escort(plan, now_s),
            Plan::Consort(plan) => craft.begin_consort(plan, now_s),
        }
    }
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
/// kilometers** in it. An orbital rendezvous spent the whole approach flying at a ten-minute-old
/// position and settled into a relative orbit a hundred to four hundred kilometers across
/// instead of onto the standoff.
///
/// A fraction of the plan rather than a time, because what a guidance loop owes is a number of
/// corrections across the maneuvere, not a cadence in seconds — and because the approach
/// *shrinks*. A fixed floor that is reasonable for the first three-thousand-second run at a
/// quarry is hopeless for the sixty-second correction at the end of it, which is exactly how
/// ten minutes came to be too coarse without ever looking wrong.
///
/// **Ten of them, and more is worse.** That is not what you would guess, and it was measured:
/// at a fiftieth of the plan a rendezvous thrashed between three and four hundred million
/// meters before capturing, and at a two-hundredth it did not capture inside two thousand
/// ticks. Each plan is a whole maneuvere — a burn, a flip and a burn — rather than a
/// controller's output, so re-solving faster than the maneuvere can run keeps resetting the
/// flip and the pursuer never reaches its brake. Anything from a tenth to a third behaves the
/// same; a tenth is the middle of the plateau.
pub const STEER_FRACTION: f64 = 0.1;

/// The shortest gap before this pursuer may be given another plan, coordinate microseconds.
///
/// Zero with no plan in hand: the first solve of a chase is the one nothing is waiting for.
fn steer_floor_us(pursuer: &Craft) -> i64 {
    let duration_s = match &pursuer.motion.motive {
        Motive::Rendezvous(plan) => plan.cruise.duration_s(),
        Motive::Escort(plan) => plan.cruise.duration_s(),
        Motive::Consort(plan) => plan.cruise.duration_s(),
        _ => return 0,
    };
    (duration_s * STEER_FRACTION * crate::world::MICROS_PER_SECOND as f64) as i64
}

/// How far the quarry's burn may move from the one an escort is modeling, as a fraction of the
/// pursuer's drive, before a new plan skips [`STEER_FRACTION`]'s wait.
///
/// The wait is a fraction of the approach, and following a quarry that pulls as hard as the
/// pursuer leaves a sliver of thrust to close with, so the approach — and the wait — runs to
/// hours. Held to it, a pursuer went on burning outward long after its quarry had flipped and
/// braked, and ended eight million kilometers past it. A burn starting, stopping or turning
/// round is a new maneuvere rather than a correction, so it is answered at once; a steady burn
/// matches its model and is still rate-limited.
pub const BURN_CHANGE_FRACTION: f64 = 0.25;

/// The quarry's proper acceleration, when its plume was lit at the sighting and two sightings
/// can measure it.
fn burn_of(fleet: &Fleet, seen: &pursuit::Sighting, previous: Option<&pursuit::Sighting>) -> Option<glam::DVec3> {
    let lit = fleet
        .get(CraftId(seen.target.0))
        .is_some_and(|quarry| quarry.jet_power_w(seen.emitted_s) > 0.0);
    lit.then(|| previous.and_then(|p| escort::acceleration_of(p, seen))).flatten()
}

/// Whether the quarry is doing something other than what the pursuer's plan assumes of its drive.
fn burn_changed(pursuer: &Craft, burn: Option<glam::DVec3>) -> bool {
    let Motive::Escort(plan) = &pursuer.motion.motive else { return burn.is_some() };
    let drive = pursuer.turning(pursuer.motion.drive);
    let now = escort::followed(burn.unwrap_or_default(), drive);
    let off_g = (now - plan.quarry.accel).length() * lc_world::flight::C_M_S / lc_world::flight::G0;
    off_g > drive.accel_g * BURN_CHANGE_FRACTION
}

/// Whether an observer is entitled to know a craft exists at all.
///
/// Sharing a system, which is the same [`LOCAL_SHELL_LY`] rule both ends already use to decide
/// where a ship is. Not an angular size: a hull five hundred meters long is well under a pixel
/// from anywhere in a system, and a rule drawn there would leave a player unable to find
/// traffic they are sitting in the middle of. Between the stars, where there is no system to
/// share, the same radius serves as a plain range.
///
/// A *visibility* rule and not a causality one. What it decides is which craft are worth
/// solving for; whether the light has arrived is [`Cleared::clear`]'s alone.
///
/// Against every system the other still remembers being in: a craft that jumped out of one is
/// still arriving there as old light.
pub fn in_sight(observer: &Craft, other: &Craft) -> bool {
    match (&observer.system, &other.system) {
        (Some(a), _) => other.has_been_in(a),
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
    // No root means light that has not arrived or has already gone past. A worldline that
    // jumped has one per piece, and the newest is where the craft appears now.
    let emitted = retarded_times_at(now_t as f64, here, &worldline).last().copied()?;
    let emitted_s = emitted * 1.0e-6;
    Some(pursuit::Sighting {
        target: lc_world::motion::ShipId(quarry.id.0),
        position_ly: worldline.position_at(emitted) / LIGHT_US_PER_LY,
        beta: worldline.velocity_at(emitted),
        // A refit that has since lengthened it has not been seen yet.
        length_m: quarry.seen_length_m_at(emitted_s),
        emitted_s,
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
            let then = craft.seen_at(sighted.emitted_s);
            let presence = Presence {
                ship_id: ShipId(craft.id.0),
                name: craft.designation(),
                length_m: sighted.length_m,
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
                // As its light left it, as everything else here is: a ship seen mid-refit is seen
                // in the shape it had then, and a new one only once that light arrives.
                form: then.map(|then| (&*then.form).into()).unwrap_or_default(),
                building: then.and_then(|then| then.underway(sighted.emitted_s)).map(Into::into),
                glow: None,
                glare: None,
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
/// Two different questions wearing one name. A ship still flying a plan for this quarry is
/// asked whether its quarry has stopped agreeing with it — which is the whole maneuvere
/// response, and which it cannot notice until the light of the maneuvere arrives — or whether
/// the plan is for another closeness. A ship that has arrived, or is doing anything else, is
/// asked whether it has drifted off station.
pub fn should_close(
    pursuer: &Craft,
    seen: &pursuit::Sighting,
    closeness: Closeness,
    arrival: &Arrival,
    now_s: f64,
) -> bool {
    let standoff = closeness.standoff_m(pursuer.length_m, seen.length_m);
    let replan = closeness.replan_m(standoff);
    let motive = &pursuer.motion.motive;
    match motive {
        Motive::Rendezvous(plan) if plan.target == seen.target && !plan.has_arrived(now_s) => {
            pursuit::wants_replan(motive, seen, standoff, replan, arrival, now_s)
        }
        Motive::Escort(plan) if plan.target == seen.target => {
            pursuit::wants_replan(motive, seen, standoff, replan, arrival, now_s)
        }
        // Never runs out and never drifts, so only the quarry leaving its conic, a new closeness
        // or the end of a courteous leg short of the station is a reason.
        Motive::Consort(plan) if plan.target == seen.target => {
            let to = plan.cruise.to_ly;
            let off = pursuer.system.as_deref().and_then(|system| plan.divergence_m(system, seen));
            !pursuit::aims_for(to, standoff, arrival)
                || (pursuit::on_the_way(to, standoff, arrival) && plan.has_closed(now_s))
                || off.is_none_or(|off| off > replan)
        }
        _ => pursuit::wants_closing(&pursuer.motion, closeness.band_m(standoff), seen, now_s),
    }
}

/// The plan for taking station on a quarry, of whichever of the three kinds it needs.
///
/// **A quarry whose plume was lit when the light left is escorted**, once there are two
/// sightings to measure its acceleration from. A quarry holding an orbit accelerates too — by
/// gravity, and so does the pursuer — and escorting it would chase where the planet takes it
/// while ignoring what the planet does to the ship chasing. So a quarry that is not burning is
/// **reckoned along its conic** where there is one, which is anywhere in a system at a speed
/// the Galilean frame holds for, and **met by a rendezvous** everywhere else.
///
/// A pursuer already escorting stays an escort when the quarry cuts its drive out there, at
/// zero acceleration. Handing it back to a rendezvous would have it arrive and go ballistic
/// beside something it had been matching under thrust, and a ship coasting outward at a good
/// fraction of `c` is the most expensive thing there is to keep patching into spheres of
/// influence.
pub fn plan(
    fleet: &Fleet,
    pursuer: &Craft,
    seen: &pursuit::Sighting,
    previous: Option<&pursuit::Sighting>,
    closeness: Closeness,
    arrival: Arrival,
    now_s: f64,
) -> Result<Plan, Refused> {
    let standoff = closeness.standoff_m(pursuer.length_m, seen.length_m);
    let drive = pursuer.turning(pursuer.motion.drive);
    if let Some(accel) = burn_of(fleet, seen, previous) {
        return escort::escort(&pursuer.motion, standoff, arrival, seen, accel, now_s, drive).map(Plan::Escort);
    }
    let falling = pursuer.system.as_deref().and_then(|system| {
        consort::approach(system, &pursuer.motion, standoff, arrival, seen, now_s, drive)
    });
    if let Some(plan) = falling {
        return Ok(Plan::Consort(plan));
    }
    if matches!(pursuer.motion.motive, Motive::Escort(_)) {
        return escort::escort(&pursuer.motion, standoff, arrival, seen, glam::DVec3::ZERO, now_s, drive)
            .map(Plan::Escort);
    }
    pursuit::approach(&pursuer.motion, standoff, arrival, seen, now_s, drive).map(Plan::Rendezvous)
}

/// How a pursuer arrives under `approach`, for its mass and drive now.
pub fn arrival(approach: lc_proto::Approach, balance: &Balance, pursuer: &Craft, now_s: f64) -> Arrival {
    match approach {
        lc_proto::Approach::Direct => Arrival::Direct,
        lc_proto::Approach::Courteous => {
            let drive = pursuer.turning(pursuer.motion.drive);
            let id = lc_world::motion::ShipId(pursuer.id.0);
            Arrival::Courteous(Manners::new(balance, pursuer.mass_kg_at(now_s), drive, id))
        }
    }
}

/// What each standing intercept wants done this tick.
///
/// `None` against a craft means give the pursuit up: its quarry has gone out of sight, or is
/// moving too fast to match the way this plans a match. An entry missing altogether means
/// carry on doing whatever it is doing.
pub fn decide(
    fleet: &Fleet,
    pursuits: &mut HashMap<CraftId, Pursuit>,
    balance: &Balance,
    now_t: i64,
) -> Vec<(CraftId, Option<Plan>)> {
    let now_s = now_t as f64 * 1.0e-6;
    let mut decided = Vec::new();
    for (id, pursuit) in pursuits.iter_mut() {
        let Some(pursuer) = fleet.get(*id) else { continue };
        // Out of sight is the end of it. A policy that survived would be one waiting to act on
        // a craft this one is no longer entitled to know about.
        let Some(seen) = sighting(fleet, *id, pursuit.quarry, now_t) else {
            decided.push((*id, None));
            continue;
        };
        // Remembered every tick, planned or not, so the acceleration it measures is over one
        // sighting interval rather than over however long the last plan happened to last.
        let previous = pursuit.last_seen.replace(seen);
        // Saturating, because "never planned" is a legitimate thing for a caller to say and
        // the obvious way to say it overflows the subtraction.
        let since = now_t.saturating_sub(pursuit.last_plan_t);
        let waiting = since < steer_floor_us(pursuer)
            && !burn_changed(pursuer, burn_of(fleet, &seen, previous.as_ref()));
        let arrival = arrival(pursuit.approach, balance, pursuer, now_s);
        if waiting || !should_close(pursuer, &seen, pursuit.closeness, &arrival, now_s) {
            continue;
        }
        match plan(fleet, pursuer, &seen, previous.as_ref(), pursuit.closeness, arrival, now_s) {
            Ok(plan) => decided.push((*id, Some(plan))),
            // On station. Nothing to fly, and the policy stays: it is what will notice the
            // next time this craft has drifted.
            Err(Refused::AlreadyThere) => {}
            // At `c`, where there is no frame to match. A quarry merely out-pulling the
            // pursuer is followed; see `escort::escort`.
            Err(Refused::TooFast) => decided.push((*id, None)),
        }
    }
    decided
}
