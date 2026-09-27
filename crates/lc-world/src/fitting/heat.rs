//! The field's side of the account: `Q`, settled with storage's income because the two share where
//! storage fills and where it runs dry. See `lightcone/docs/30-the-field.md` §The heat account.

use super::{Balance, Fitting, Hull};
use crate::field::{Field, Mode, Segment};
use crate::refit::rounds::{Phase, Plan, Step};

/// Every field runs Black until the modes are kept.
pub const MODE: Mode = Mode::Black;

/// Heat at `now_s`, and what conversion stored less what living space drew since the settlement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Flow {
    pub heat_j: f64,
    pub income_j: f64,
}

/// Where the living drain alone holds a field of this hull: far from any star, storage paying it.
pub(super) fn idle_j(hull: &Hull, balance: &Balance) -> f64 {
    hull.capacities.drain_w * balance.field_tau_s
}

impl Fitting {
    /// On the envelope of the last form the grid measured.
    pub fn field(&self) -> Field {
        Field::of(self.geometry.envelope_area_m2, &self.balance)
    }

    /// `Q` at a coordinate time, joules.
    pub fn heat_j_at(&self, now_s: f64) -> f64 {
        self.flow(now_s).heat_j
    }

    pub fn temperature_k_at(&self, now_s: f64) -> f64 {
        self.field().temperature_k(self.heat_j_at(now_s))
    }

    /// What starlight puts into storage while it has room, watts: converted up to the engines'
    /// rating, at `conversion_efficiency`.
    pub fn solar_w(&self) -> f64 {
        self.intake(0.0, 0.0).stored_w()
    }

    fn intake(&self, losing_w: f64, room_j: f64) -> Segment {
        let caps = &self.hull.capacities;
        Segment {
            arriving_w: self.starlight_w,
            absorptivity: MODE.absorptivity(self.balance.clear_absorptivity),
            internal_w: caps.drain_w + losing_w,
            rating_w: caps.aperture_w,
            efficiency: self.balance.conversion_efficiency,
            room_j,
            draw_w: caps.drain_w,
        }
    }

    /// From the settlement to `now_s`, cut wherever a refit step begins or ends: a dismantling
    /// radiates its loss over its step, and a vent lands at the end of the step that frees it.
    ///
    /// Storage's room and what is free are carried across the cuts, so where they fall changes
    /// nothing. A refit's own transfers and a burn's spending are left out of both, as the
    /// settled terms leave them out; they are exact while neither runs.
    pub(super) fn flow(&self, now_s: f64) -> Flow {
        let field = self.field();
        let mut flow = Flow { heat_j: self.heat_j, income_j: 0.0 };
        if now_s <= self.since_s {
            return flow;
        }
        let mut room_j = self.hull.capacities.storage_j - self.stored_j;
        let mut free_j = self.stored_j - self.committed_j;
        let mut at_s = self.since_s;
        for (until_s, losing_w, vent_j) in pieces(self.refit.as_ref(), &self.balance, self.since_s, now_s) {
            let (heat_j, storage_j) = settle(&field, &self.intake(losing_w, room_j), flow.heat_j, free_j, until_s - at_s);
            flow.heat_j = heat_j + vent_j;
            flow.income_j += storage_j;
            room_j -= storage_j;
            free_j += storage_j;
            at_s = until_s;
        }
        flow
    }
}

/// [`Field::settle`], split where storage runs down to what is committed. From there the draw takes
/// only what conversion brings in, and what it cannot pay makes no heat.
fn settle(field: &Field, segment: &Segment, heat_j: f64, free_j: f64, dt_s: f64) -> (f64, f64) {
    let short_w = segment.draw_w - segment.stored_w();
    let empty_s = if short_w > 0.0 { free_j.max(0.0) / short_w } else { f64::INFINITY };
    if empty_s >= dt_s {
        let settled = field.settle(segment, heat_j, dt_s);
        return (settled.heat_j, settled.storage_j);
    }
    let draining = field.settle(segment, heat_j, empty_s);
    let paid_w = segment.stored_w();
    let starved = Segment { draw_w: paid_w, internal_w: segment.internal_w - segment.draw_w + paid_w, ..*segment };
    let rest = field.settle(&starved, draining.heat_j, dt_s - empty_s);
    (rest.heat_j, draining.storage_j + rest.storage_j)
}

/// `(until_s, losing_w, vent_j)` for each stretch of constant refit heat from `since_s` to `now_s`:
/// the dismantling's loss over it, and the vents landing at its end.
fn pieces(plan: Option<&Plan>, balance: &Balance, since_s: f64, now_s: f64) -> Vec<(f64, f64, f64)> {
    let Some(plan) = plan else { return vec![(now_s, 0.0, 0.0)] };
    let start_s = plan.round().start_s;
    let mut cuts: Vec<f64> = plan
        .steps()
        .iter()
        .flat_map(|s| [start_s + s.begins_s, start_s + s.ends_s()])
        .filter(|&t| t > since_s && t < now_s)
        .chain([now_s])
        .collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut from_s = since_s;
    cuts.into_iter()
        .map(|until_s| {
            let middle_s = 0.5 * (from_s + until_s) - start_s;
            let losing_w = plan
                .steps()
                .iter()
                .find(|s| s.begins_s < middle_s && middle_s < s.ends_s())
                .map_or(0.0, |s| losing_w(s, balance));
            let vent_j: f64 = plan.steps().iter().filter(|s| start_s + s.ends_s() == until_s).map(|s| s.vented_j).sum();
            from_s = until_s;
            (until_s, losing_w, vent_j)
        })
        .collect()
}

/// A dismantling's loss, spread over its step as the energy moves.
fn losing_w(step: &Step, balance: &Balance) -> f64 {
    match step.change.phase() {
        Phase::Dismantle if step.duration_s > 0.0 => (1.0 - balance.recovery) * step.gross_j / step.duration_s,
        _ => 0.0,
    }
}

/// What the steps not yet done by `since_s` would still lose and vent, joules: a finish skips their
/// time but not their heat.
pub(super) fn left_j(plan: &Plan, since_s: f64, balance: &Balance) -> f64 {
    let start_s = plan.round().start_s;
    plan.steps()
        .iter()
        .filter(|s| start_s + s.ends_s() > since_s)
        .map(|s| {
            let begun = ((since_s - start_s - s.begins_s) / s.duration_s).clamp(0.0, 1.0);
            let loss_j = if s.duration_s > 0.0 { losing_w(s, balance) * s.duration_s * (1.0 - begun) } else { 0.0 };
            loss_j + s.vented_j
        })
        .sum()
}
