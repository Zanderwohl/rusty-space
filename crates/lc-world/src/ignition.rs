//! When a craft's drive lights, goes out or changes power, to the instant.
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
    /// Watts into the exhaust from this instant. Zero is the drive going out.
    pub power_w: f64,
    pub was_w: f64,
    /// Unit vector the nose pointed along.
    pub facing: DVec3,
}

/// Every transition in `(after_s, until_s]`, in order.
pub fn transitions(craft: &Craft, after_s: f64, until_s: f64) -> Vec<Transition> {
    let mut candidates = Vec::new();
    let mut began = f64::NEG_INFINITY;
    for (until, doing) in craft.stretches() {
        candidates.push(began);
        candidates.extend(phase_changes_s(doing).into_iter().filter(|t| *t >= began && *t < until));
        began = until;
    }
    candidates.retain(|t| *t > after_s && *t <= until_s);
    candidates.sort_by(f64::total_cmp);
    candidates.dedup_by(|a, b| (*a - *b).abs() < STRADDLE_S);

    candidates
        .into_iter()
        .filter_map(|at_s| {
            let was_w = power_w(craft, at_s - STRADDLE_S);
            let power_w = power_w(craft, at_s + STRADDLE_S);
            let changed = if was_w == 0.0 || power_w == 0.0 {
                (was_w == 0.0) != (power_w == 0.0)
            } else {
                (power_w - was_w).abs() > POWER_TOLERANCE * was_w
            };
            changed.then(|| Transition {
                at_s,
                power_w,
                was_w,
                facing: motion::facing_at(craft.motion_at(at_s), craft.length_m, at_s),
            })
        })
        .collect()
}

fn power_w(craft: &Craft, s: f64) -> f64 {
    let doing = craft.motion_at(s);
    let accel_g = motion::thrust_g(doing, s);
    if accel_g <= 0.0 { 0.0 } else { doing.drive.jet_power_w(craft.mass_kg(), accel_g) }
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

    /// Twenty thousand kilometres from rest along `toward`, ordered at `at_s`: a crossing whose
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
        let found = transitions(&craft, 0.0, arrive + 100.0);
        let at: Vec<f64> = found.iter().map(|t| t.at_s).collect();
        assert_eq!(at, [lit, flip, brake, arrive], "{found:?}");
        let on: Vec<bool> = found.iter().map(|t| t.power_w > 0.0).collect();
        assert_eq!(on, [true, false, true, false]);
        assert!(found[1].was_w > 0.0, "a cut says what it cut");

        // Split across two windows, the same four and none twice.
        let middle = 0.5 * (flip + brake);
        let halves = [transitions(&craft, 0.0, middle), transitions(&craft, middle, arrive + 100.0)].concat();
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

        let found = transitions(&craft, 0.0, arrive + 100.0);
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
        assert_eq!(transitions(&craft, 0.0, lit + 1_000.0), []);
    }
}
