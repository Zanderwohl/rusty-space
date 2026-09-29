//! When a craft's drive or emit lights, goes out or changes power, to the instant.
//!
//! What another ship's plume shows used to be sampled once a server tick, and a tick is 438
//! coordinate seconds at the design rate: the sixty-second flip in the middle of a crossing
//! fell between samples and was never seen. So the shard asks this, once a tick, for every
//! transition in the tick just finished, and states each one as an event.
//!
//! Read off what the craft actually flew rather than what it was ordered to: a plan's own phase
//! boundaries, and every change of motive its history recorded. A plan replaced before it lit
//! never lit, and nothing here has to be withdrawn.

use glam::DVec3;

use crate::craft::Craft;
use crate::emit::{Ends, Jet, emit_spread_rad, emit_w, exhaust};
use crate::fitting::Balance;
use crate::flight::Drive;
use crate::motion::{self, Motive, ShipState};

/// Either side of a candidate instant, coordinate seconds. Well above the resolution of a
/// coordinate second near 1e9 (1e-7 s), and well below the shortest phase anything flies.
const STRADDLE_S: f64 = 1.0e-3;

/// A power change smaller than this fraction is not stated. An escort's thrust follows its
/// quarry's and is re-solved often; every re-solve is not news.
const POWER_TOLERANCE: f64 = 0.01;

/// The drive at one instant, and what it was just before.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transition {
    /// Coordinate seconds.
    pub at_s: f64,
    /// What the main drive sends aft from this instant, `F c`, watts: [`drive_w`]. Unchanged when
    /// only the thrusters or an emit flown as a burn changed.
    pub power_w: f64,
    pub was_w: f64,
    /// What its emits send out of each end from this instant, and just before: [`emit_w`].
    pub emit: Ends,
    pub was_emit: Ends,
    /// The half-angle they are sent in from this instant.
    pub emit_spread_rad: f64,
    /// Unit vector the nose pointed along.
    pub facing: DVec3,
}

impl Transition {
    /// Whether the main drive's power moved here, by the tolerance that decides every transition.
    pub fn drive_stepped(&self) -> bool {
        stepped(self.was_w, self.power_w)
    }

    /// Whether what leaves either end for an emit moved here.
    pub fn emit_stepped(&self) -> bool {
        stepped(self.was_emit.fore_w, self.emit.fore_w) || stepped(self.was_emit.aft_w, self.emit.aft_w)
    }
}

/// Every instant in `(after_s, until_s]` that anything lit changed, in order.
pub fn transitions(craft: &Craft, balance: &Balance, after_s: f64, until_s: f64) -> Vec<Transition> {
    let mut candidates = Vec::new();
    let mut began = f64::NEG_INFINITY;
    for (until, doing) in craft.stretches() {
        candidates.push(began);
        candidates.extend(phase_changes_s(doing).into_iter().filter(|t| *t >= began && *t < until));
        began = until;
    }
    candidates.extend(craft.balanced_changes_s());
    candidates.retain(|t| *t > after_s && *t <= until_s);
    candidates.sort_by(f64::total_cmp);
    candidates.dedup_by(|a, b| (*a - *b).abs() < STRADDLE_S);

    candidates
        .into_iter()
        .filter_map(|at_s| {
            let (before_s, after_s) = (at_s - STRADDLE_S, at_s + STRADDLE_S);
            let was = lit_w(craft, balance, before_s);
            let now = lit_w(craft, balance, after_s);
            let transition = Transition {
                at_s,
                power_w: now[0],
                was_w: was[0],
                emit: emit_w(craft, after_s),
                was_emit: emit_w(craft, before_s),
                emit_spread_rad: emit_spread_rad(craft, after_s).unwrap_or(0.0),
                facing: motion::facing_at(craft.motion_at(at_s), craft.length_m, at_s),
            };
            let changed = was.iter().zip(&now).any(|(was_w, now_w)| stepped(*was_w, *now_w)) || transition.emit_stepped();
            changed.then_some(transition)
        })
        .collect()
}

fn stepped(was_w: f64, now_w: f64) -> bool {
    if was_w == 0.0 || now_w == 0.0 {
        (was_w == 0.0) != (now_w == 0.0)
    } else {
        (now_w - was_w).abs() > POWER_TOLERANCE * was_w
    }
}

/// `F c` at `s` of the main drive, the thrusters and an emit flown as a burn, in that order.
fn lit_w(craft: &Craft, balance: &Balance, s: f64) -> [f64; 3] {
    let jets = exhaust(craft, balance, s);
    let jet_w = |kind: Jet| jets.iter().filter(|j| j.jet == kind).map(|j| j.power_w).sum();
    let doing = craft.motion_at(s);
    let boost_w = match doing.motive {
        Motive::Boosting(_) => Drive::exhaust_w(craft.mass_kg_at(s), motion::thrust_g(doing, s)),
        _ => 0.0,
    };
    [jet_w(Jet::Drive), jet_w(Jet::Thrusters), boost_w]
}

/// World instants at which a motive's thrust can change. Empty for one that never burns.
fn phase_changes_s(state: &ShipState) -> Vec<f64> {
    match &state.motive {
        Motive::Crossing(cruise) => cruise.phase_changes_s().to_vec(),
        Motive::Transfer(transfer) => transfer.cruise.phase_changes_s().to_vec(),
        Motive::Consort(plan) => plan.cruise.phase_changes_s().to_vec(),
        // Both fly their cruise on a clock of their own, which runs monotonically with the
        // world's: each boundary is carried across rather than solved for.
        Motive::Rendezvous(plan) => plan
            .cruise
            .phase_changes_s()
            .iter()
            .map(|t| plan.since_t + plan.world_elapsed(*t))
            .collect(),
        Motive::Escort(plan) => plan
            .cruise
            .phase_changes_s()
            .iter()
            .map(|tau| plan.quarry.since_t + plan.quarry.world_elapsed(*tau))
            .collect(),
        Motive::Boosting(boost) => vec![boost.lights_s(), boost.out_s()],
        Motive::Holding(_) | Motive::Falling(_) | Motive::Drifting { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::craft::{CraftId, Kind};
    use crate::flight::STANDOFF_LY;
    use crate::motion::{Change, Event, ShipId};
    use crate::system::M_PER_LY;

    /// Twenty thousand kilometers from rest along `toward`, ordered at `at_s`: a crossing whose
    /// flip is a minute long. The ship starts facing +x.
    fn crossing(at_s: f64, toward: DVec3) -> Craft {
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        let drive = craft.turning(craft.kind.drive());
        let to_ly = toward * (STANDOFF_LY + 2.0e7 / M_PER_LY);
        order(&mut craft, at_s, Change::Cross { to_ly, drive });
        craft
    }

    fn order(craft: &mut Craft, at_t: f64, change: Change) {
        craft.apply(&Event { ship: ShipId(1), at_t, change }).unwrap();
    }

    fn cruise(craft: &Craft) -> crate::flight::Cruise {
        let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("not a crossing") };
        cruise.clone()
    }

    /// Lit, out for the flip, lit, out on arrival: each at its phase boundary to the
    /// microsecond, whatever window it is asked for in.
    #[test]
    fn a_crossing_lights_and_cuts_at_its_phase_boundaries() {
        let craft = crossing(10.0, DVec3::X);
        let [lit, _, flip, brake, arrive] = cruise(&craft).phase_changes_s();
        let found = transitions(&craft, &Balance::DEFAULT, 0.0, arrive + 100.0);
        let at: Vec<f64> = found.iter().map(|t| t.at_s).collect();
        assert_eq!(at, [lit, flip, brake, arrive], "{found:?}");
        let on: Vec<bool> = found.iter().map(|t| t.power_w > 0.0).collect();
        assert_eq!(on, [true, false, true, false]);
        assert!(found[1].was_w > 0.0, "a cut says what it cut");

        // Split across two windows, the same four and none twice.
        let middle = 0.5 * (flip + brake);
        let halves = [transitions(&craft, &Balance::DEFAULT, 0.0, middle), transitions(&craft, &Balance::DEFAULT, middle, arrive + 100.0)].concat();
        assert_eq!(halves, found);
    }

    /// An order that cuts the drive mid-burn is the cut, at the order's instant; the flip the
    /// abandoned plan would have made never happens.
    #[test]
    fn cutting_the_drive_mid_burn_is_a_transition_and_ends_the_plan() {
        let mut craft = crossing(10.0, DVec3::X);
        let [lit, _, flip, _, arrive] = cruise(&craft).phase_changes_s();
        let cut_at = 0.5 * (lit + flip);
        order(&mut craft, cut_at, Change::CutDrive);

        let found = transitions(&craft, &Balance::DEFAULT, 0.0, arrive + 100.0);
        let at: Vec<f64> = found.iter().map(|t| t.at_s).collect();
        assert_eq!(at, [lit, cut_at], "{found:?}");
        assert_eq!(found[1].power_w, 0.0);
    }

    /// A plan replaced before it ever lit was never flown, and nothing says it was.
    #[test]
    fn a_plan_replaced_before_it_lights_says_nothing() {
        // Behind it, so it has to come about first.
        let mut craft = crossing(10.0, -DVec3::X);
        let [lit, ..] = cruise(&craft).phase_changes_s();
        assert!(lit > 10.0, "premise: the ship has to come about before it lights");
        order(&mut craft, 10.0 + 0.5 * (lit - 10.0), Change::CutDrive);
        assert_eq!(transitions(&craft, &Balance::DEFAULT, 0.0, lit + 1_000.0), []);
    }
}
