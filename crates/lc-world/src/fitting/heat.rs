//! The field's side of the account: `Q`, settled with storage's income because the two share where
//! storage fills and where it runs dry. See `lightcone/docs/30-the-field.md` §The heat account.

use super::{Balance, Fitting, Hull};
use crate::field::{Field, Mode, Segment};
use crate::refit::rounds::{Phase, Plan, Step};

/// Every field runs Black until the modes are built (H6).
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

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;
    use crate::fitting::{Account, STARTING_BROADSIDE_M2};
    use crate::form::{Form, PartId};
    use crate::motion::ShipState;
    use crate::refit::rounds::Round;
    use crate::solar::{intake_w, SOLAR_CONSTANT_W_M2};
    use crate::system::UNIT_M as AU_M;

    fn rest() -> ShipState {
        ShipState::at(DVec3::ZERO)
    }

    fn me(b: &Balance) -> f64 {
        b.module_energy_j()
    }

    fn close(got: f64, want: f64, rel: f64) -> bool {
        (got - want).abs() <= rel * want.abs()
    }

    /// Broadside, `d_au` from a Sun-like star.
    fn starlight_w(b: &Balance, d_au: f64) -> f64 {
        let luminosity_w = SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * AU_M * AU_M;
        intake_w(b, STARTING_BROADSIDE_M2, luminosity_w, d_au * AU_M)
    }

    fn starting(b: Balance, stored_j: f64) -> Fitting {
        let full = Fitting::full(Form::starting(), b, 0.0);
        Fitting::from_account(&Account { stored_j, ..full.account() }, b)
    }

    /// 30's table: the starting ship full at 0.1 AU, and filling there half a year in, when it is
    /// far from full and long past its time constant.
    #[test]
    fn the_starting_ship_at_a_tenth_of_an_au_settles_at_thirtys_temperatures() {
        let b = Balance::DEFAULT;
        let mut full = Fitting::full(Form::starting(), b, 0.0);
        full.set_starlight_w(starlight_w(&b, 0.1));
        let later_s = 40.0 * b.field_tau_s;
        full.settle(&rest(), later_s);
        let full_k = full.temperature_k_at(later_s);
        assert!((full_k - 3_237.0).abs() < 0.5, "full {full_k}");
        assert_eq!(full.stored_j_at(&rest(), later_s), full.hull().capacities.storage_j, "held full");

        let mut filling = starting(b, 0.0);
        filling.set_starlight_w(starlight_w(&b, 0.1));
        let half_year_s = 0.5 * crate::flight::JULIAN_YEAR_S;
        let filling_k = filling.temperature_k_at(half_year_s);
        assert!((filling_k - 2_396.0).abs() < 0.5, "filling {filling_k}");
        let stored = filling.stored_j_at(&rest(), half_year_s) / filling.hull().capacities.storage_j;
        assert!((stored - 0.5).abs() < 0.01, "{stored} full");
    }

    /// Past the engines' rating nothing more is converted, and what arrives beyond it is heat.
    #[test]
    fn conversion_is_rated_by_the_engines() {
        let b = Balance::DEFAULT;
        let mut fitting = starting(b, 0.0);
        let rating_w = fitting.hull().capacities.aperture_w;
        fitting.set_starlight_w(0.5 * rating_w);
        assert!(close(fitting.solar_w(), b.conversion_efficiency * 0.5 * rating_w, 1e-12));
        fitting.set_starlight_w(3.0 * rating_w);
        assert!(close(fitting.solar_w(), b.conversion_efficiency * rating_w, 1e-12));
        let dt_s = 10.0;
        let heat_j = fitting.heat_j_at(dt_s) - fitting.heat_j_at(0.0);
        let unconverted_j = (3.0 * rating_w - b.conversion_efficiency * rating_w) * dt_s;
        assert!(close(heat_j, unconverted_j, 1e-4), "{heat_j} {unconverted_j}");
    }

    /// A ship filling, then full partway, then held there: settled in one leap and in a thousand
    /// small settlements, heat and storage agree.
    #[test]
    fn one_leap_and_many_settlements_agree() {
        let b = Balance::DEFAULT;
        let mut leap = starting(b, 20.0 * me(&b));
        leap.set_starlight_w(starlight_w(&b, 0.05));
        let mut steps = leap.clone();
        let end_s = 1.0e7;
        let room_j = leap.hull().capacities.storage_j - 20.0 * me(&b);
        assert!(close(leap.flow(end_s).income_j, room_j, 1e-12), "premise: it fills");
        assert!(leap.intake(0.0, room_j).fill_s().unwrap() < 0.5 * end_s, "premise: it fills early");
        leap.settle(&rest(), end_s);
        for k in 1..=1000 {
            steps.settle(&rest(), end_s * f64::from(k) / 1000.0);
        }
        assert!(close(steps.heat_j, leap.heat_j, 1e-9), "{} {}", steps.heat_j, leap.heat_j);
        assert!(close(steps.stored_j, leap.stored_j, 1e-12), "{} {}", steps.stored_j, leap.stored_j);
        assert_eq!(leap.stored_j, leap.hull().capacities.storage_j);
    }

    /// Storage run down to nothing pays no more of the drain than conversion brings in, and what it
    /// does not pay makes no heat: far from a star, an empty ship cools.
    #[test]
    fn an_empty_ship_far_from_a_star_cools() {
        let b = Balance::DEFAULT;
        let fitting = starting(b, 0.0);
        let idle_j = fitting.heat_j_at(0.0);
        let later_j = fitting.heat_j_at(b.field_tau_s);
        assert!(close(later_j, idle_j / std::f64::consts::E, 1e-12), "{later_j} {idle_j}");
        let paid = starting(b, 1.0e3 * me(&b));
        assert!(close(paid.heat_j_at(b.field_tau_s), idle_j, 1e-12), "the drain holds it at idle");
    }

    fn shrunk_engine() -> Form {
        let mut target = Form::starting();
        target.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 3.0 / 5.0;
        target
    }

    fn begin(fitting: &mut Fitting, target: Form) -> Plan {
        let round = Round { from: fitting.form().clone(), target, stored_j: fitting.stored_j, start_s: fitting.since_s };
        let plan = round.solve(fitting.balance()).unwrap();
        fitting.begin_refit(plan.clone());
        plan
    }

    #[test]
    fn a_vent_raises_heat_by_exactly_what_storage_could_not_hold() {
        let b = Balance::DEFAULT;
        let mut fitting = Fitting::full(Form::starting(), b, 0.0);
        let room_j = me(&b);
        fitting.drain(room_j);
        let plan = begin(&mut fitting, shrunk_engine());
        let [step] = plan.steps() else { panic!("{:?}", plan.steps()) };
        let overflow_j = b.recovery * step.gross_j - room_j;
        assert!(overflow_j > 0.0 && close(step.vented_j, overflow_j, 1e-12), "premise: {} {overflow_j}", step.vented_j);

        let end_s = step.ends_s();
        let just_s = 1.0e-6 * step.duration_s;
        let before_j = fitting.heat_j_at(end_s - just_s);
        let holding_w = fitting.hull().capacities.drain_w + losing_w(step, &b);
        let relaxed_j = fitting.field().heat_after_j(before_j, holding_w, just_s);
        let jump_j = fitting.heat_j_at(end_s) - relaxed_j;
        assert!(close(jump_j, overflow_j, 1e-9), "{jump_j} {overflow_j}");
    }

    /// With the field's radiation switched off, heat is a ledger: a dismantling into full storage
    /// ends with all it took apart in the field, its loss spread over the step and the rest vented
    /// at the end. Settling across the step's end, or finishing early, lands in the same place.
    #[test]
    fn a_dismantling_into_full_storage_ends_as_heat() {
        let b = Balance { living_density_w: 0.0, field_tau_s: 1.0e40, ..Balance::DEFAULT };
        let mut fitting = Fitting::full(Form::starting(), b, 0.0);
        assert_eq!(fitting.heat_j, 0.0, "premise: nothing holds any heat");
        let plan = begin(&mut fitting, shrunk_engine());
        let step = plan.steps()[0];
        let end_s = step.ends_s();
        let loss_j = (1.0 - b.recovery) * step.gross_j;
        assert!(close(fitting.heat_j_at(0.5 * end_s), 0.5 * loss_j, 1e-9), "{}", fitting.heat_j_at(0.5 * end_s));
        assert!(close(fitting.heat_j_at(end_s), step.gross_j, 1e-9));

        let mut finished = fitting.clone();
        finished.settle(&rest(), 0.3 * end_s);
        assert!(finished.finish_refit());
        fitting.settle(&rest(), 0.7 * end_s);
        fitting.settle(&rest(), 2.0 * end_s);
        assert!(close(fitting.heat_j, step.gross_j, 1e-9), "{}", fitting.heat_j);
        assert!(close(finished.heat_j, step.gross_j, 1e-9), "{}", finished.heat_j);

        let mut canceled = Fitting::full(Form::starting(), b, 0.0);
        begin(&mut canceled, shrunk_engine());
        canceled.settle(&rest(), 0.5 * end_s);
        canceled.cancel_refit(0.5 * end_s);
        // Storage pays to put the half taken apart back, loss included, so the loss stays heat.
        assert!(close(canceled.heat_j, 0.5 * loss_j, 1e-9), "{}", canceled.heat_j);
    }

    #[test]
    fn heat_weighs_what_it_holds() {
        let b = Balance::DEFAULT;
        let cold = Fitting::from_account(&Account { heat_j: 0.0, ..Fitting::full(Form::starting(), b, 0.0).account() }, b);
        let hot = Fitting::from_account(&Account { heat_j: 5.0 * me(&b), ..cold.account() }, b);
        let difference_kg = hot.mass_kg_at(&rest(), 0.0) - cold.mass_kg_at(&rest(), 0.0);
        assert!(close(difference_kg, 5.0 * me(&b) / crate::fitting::C2, 1e-6), "{difference_kg}");
        assert!(close(hot.settled_mass_kg() - cold.settled_mass_kg(), difference_kg, 1e-9));
    }

    /// `Fitted` carries `Q`, and a fitting sent with its field comes back holding it.
    #[test]
    fn heat_crosses_the_wire() {
        let b = Balance::DEFAULT;
        let mut fitting = Fitting::full(Form::starting(), b, 0.0);
        fitting.set_starlight_w(starlight_w(&b, 0.1));
        fitting.settle(&rest(), 1.0e6);
        let field = lc_proto::Field::from(&fitting);
        assert_eq!((field.heat_j, field.since_s), (fitting.heat_j, 1.0e6));
        assert_eq!(field.shade, lc_proto::Shade::Black);
        let sent = lc_proto::Fitting::from(&fitting);
        assert_eq!(Fitting::from_wire(&sent, Some(&field)), fitting);
        assert_eq!(Fitting::from(&sent).heat_j, fitting.hull().capacities.drain_w * b.field_tau_s);
    }
}
