//! The field's side of the account: `Q`, settled with everything that moves storage, because the two
//! share where storage fills and where it runs dry. See `lightcone/docs/30-the-field.md` §The heat
//! account.
//!
//! What the ship emits is drawn from heat first (`31-directed-energy.md` §The drive is the
//! radiator), and the commitment runs down by all of it whatever pays.

use super::{Balance, Fitting, Hull};
use crate::cost;
use crate::field::{Burst, Field, Mode, Segment};
use crate::form::capacity::Capacities;
use crate::motion::ShipState;
use crate::refit::rounds::{Phase, Plan, Step};

/// An emission with no net thrust: `power_w` from `from_s` until `until_s`, coordinate seconds.
/// Drawn as the drive's exhaust is, and committed when lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lit {
    pub from_s: f64,
    pub until_s: f64,
    pub power_w: f64,
}

/// Since the settlement: the heat reached, and storage's net change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Flow {
    pub heat_j: f64,
    pub income_j: f64,
}

/// Far from any star, with storage paying the drain.
pub(super) fn idle_j(hull: &Hull, balance: &Balance) -> f64 {
    hull.capacities.drain_w * balance.field_tau_s
}

impl Fitting {
    /// On the envelope of the last form the grid measured.
    pub fn field(&self) -> Field {
        Field::of(self.geometry.envelope_area_m2, &self.balance)
    }

    /// `motion` must be the motive in force since the settlement, as for [`Fitting::settle`].
    pub fn heat_j_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        self.flow(Some(motion), now_s).heat_j
    }

    pub fn temperature_k_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        self.field().temperature_k(self.heat_j_at(motion, now_s))
    }

    /// `P_in` of `dQ/dt = P_in − Q/τ` at `now_s`, exhaust included: the inputs then in force, so
    /// `P_in τ` is where heat is heading.
    pub fn heat_w_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        let field = self.field();
        let now_s = now_s.max(self.since_s);
        let mut heat_w = 0.0;
        // A second past `now_s`, so the last stretch offered is the one in force from it.
        self.walk(Some(motion), now_s + 1.0, |_, part, heat_j, dt_s| {
            heat_w = field.stretches(part, heat_j, dt_s).last().map_or(0.0, |s| s.heat_w);
            false
        });
        heat_w
    }

    /// Watts arriving from other craft: a neighbor's glow.
    pub fn lit_w(&self) -> f64 {
        self.lit_w
    }

    /// Settle first.
    pub fn set_lit_w(&mut self, watts: f64) {
        self.lit_w = watts.max(0.0);
    }

    /// All of it heat, whatever room storage has, absorbed in the shade the field is in. Settle first.
    pub fn take_burst(&mut self, burst: Burst) {
        self.heat_j += burst.heat_j(self.absorptivity_at(self.since_s));
    }

    /// Watts starlight stores while storage has room: capped at the engines' rating.
    pub fn solar_w(&self) -> f64 {
        self.intake(&self.hull.capacities, self.shade_at(self.since_s), 0.0, 0.0, 0.0, 0.0).stored_w()
    }

    /// `draw_w` is what leaves storage besides the drain, and goes negative for a return into it.
    fn intake(&self, caps: &Capacities, shade: Mode, losing_w: f64, room_j: f64, draw_w: f64, emitted_w: f64) -> Segment {
        Segment {
            arriving_w: self.starlight_w + self.lit_w,
            absorptivity: shade.absorptivity(self.balance.clear_absorptivity),
            internal_w: caps.drain_w + losing_w,
            rating_w: caps.aperture_w,
            efficiency: self.balance.conversion_efficiency,
            room_j,
            draw_w: caps.drain_w + draw_w,
            emitted_w,
        }
    }

    /// Averaged over `[from_s, until_s]`: the drive's exhaust and whatever else is lit.
    fn emitted_w(&self, motion: &ShipState, from_s: f64, until_s: f64) -> f64 {
        let burn_j = self.burn_spent_j(motion, until_s) - self.burn_spent_j(motion, from_s);
        (burn_j + self.lit_between_j(from_s, until_s)) / (until_s - from_s)
    }

    /// What the emissions with no net thrust put out over `[from_s, until_s]`, joules.
    pub(super) fn lit_between_j(&self, from_s: f64, until_s: f64) -> f64 {
        self.lit.iter().map(|lit| lit.power_w * (lit.until_s.min(until_s) - lit.from_s.max(from_s)).max(0.0)).sum()
    }

    /// Once the round under way is finished, if nothing else moves storage.
    pub fn stored_after_refit_j(&self, motion: &ShipState, now_s: f64) -> f64 {
        let stored_j = self.stored_j_at(motion, now_s);
        self.refit.as_ref().map_or(stored_j, |plan| skip(plan, now_s, stored_j, &self.balance).0)
    }

    /// Cut where refit steps begin and end and where the drive lights or goes out, so every input is
    /// constant over a piece. Room and free storage carry across cuts, so where settlements fall
    /// changes nothing. The burn's commitment and the builds still to run are held out of free
    /// storage, so the drain starves first.
    pub(super) fn flow(&self, motion: Option<&ShipState>, now_s: f64) -> Flow {
        self.walk(motion, now_s, |_, _, _, _| false)
    }

    /// When `Q` first reaches `Q_max` from the settlement to `until_s`, if the inputs in force hold
    /// and the round runs as planned. A vent that crosses it does so at the end of its step. A burn
    /// only ever lowers `Q`, so it can only put a collapse off.
    pub fn collapse_s(&self, motion: &ShipState, until_s: f64) -> Option<f64> {
        let field = self.field();
        let max_j = field.heat_max_j();
        // Already there, as a vent or a spike leaves it, even with nothing left to walk.
        if self.heat_j >= max_j {
            return Some(self.since_s);
        }
        let mut found = None;
        self.walk(Some(motion), until_s, |from_s, segment, heat_j, dt_s| {
            found = field.segment_time_to_rise_s(segment, heat_j, max_j).filter(|&t| t <= dt_s).map(|t| from_s + t);
            found.is_some()
        });
        found
    }

    /// [`Fitting::flow`] to `until_s`, first offering `stop` each stretch of constant inputs as its
    /// start, inputs, heat and length, and ending early where it says. A vent or spill lands between
    /// stretches, so the next starts from it. A switch completing is a cut, as a step's end is.
    pub(super) fn walk(&self, motion: Option<&ShipState>, until_s: f64, mut stop: impl FnMut(f64, &Segment, f64, f64) -> bool) -> Flow {
        let field = self.field();
        let mut flow = Flow { heat_j: self.heat_j, income_j: 0.0 };
        if until_s <= self.since_s {
            return flow;
        }
        let mut caps = self.capacities_at(self.since_s);
        let building_j = self.refit.as_ref().map_or(0.0, |plan| building_j(plan, self.since_s));
        let mut free_j = self.stored_j - self.committed_j - building_j;
        let mut at_s = self.since_s;
        let mut edges = motion.map_or_else(Vec::new, |m| cost::lit_edges(m, self.since_s, until_s));
        edges.extend(self.lit.iter().flat_map(|lit| [lit.from_s, lit.until_s]).filter(|&t| t > self.since_s && t < until_s));
        edges.extend(self.posture.switch.map(|s| s.done_s).filter(|&t| t > self.since_s && t < until_s));
        for piece in pieces(self.refit.as_ref(), &self.balance, self.since_s, until_s, &edges) {
            let dt_s = piece.until_s - at_s;
            let emitted_w = motion.map_or(0.0, |m| self.emitted_w(m, at_s, piece.until_s));
            let room_j = caps.storage_j - (self.stored_j + flow.income_j);
            let segment = self.intake(&caps, self.shade_at(at_s), piece.losing_w, room_j, piece.moving_w, emitted_w);
            let held_w = emitted_w + piece.moving_w.max(0.0);
            let (mut heat_j, mut storage_j) = (flow.heat_j, 0.0);
            let mut from_s = at_s;
            for (part, part_s) in split(&field, segment, heat_j, caps.drain_w, held_w, free_j, dt_s) {
                let part = Segment { room_j: room_j - storage_j, ..part };
                if stop(from_s, &part, heat_j, part_s) {
                    return flow;
                }
                let settled = field.settle(&part, heat_j, part_s);
                heat_j = settled.heat_j;
                storage_j += settled.storage_j;
                from_s += part_s;
            }
            flow.income_j += storage_j;
            free_j += storage_j + held_w * dt_s;
            caps = self.capacities_at(piece.until_s);
            let spilled_j = (self.stored_j + flow.income_j - caps.storage_j).max(0.0);
            flow.income_j -= spilled_j;
            free_j -= spilled_j;
            flow.heat_j = heat_j + piece.vent_j + spilled_j;
            at_s = piece.until_s;
        }
        flow
    }
}

/// A segment over `dt_s` from `heat_j`, split where free storage runs out. After that the drain
/// gets only what comes in, and its unpaid part makes no heat. `held_w`, paid from what is held
/// back, is never cut.
fn split(
    field: &Field,
    segment: Segment,
    heat_j: f64,
    drain_w: f64,
    held_w: f64,
    free_j: f64,
    dt_s: f64,
) -> impl Iterator<Item = (Segment, f64)> {
    let (mut free_j, mut at_s) = (free_j, 0.0);
    for stretch in field.stretches(&segment, heat_j, dt_s) {
        let short_w = -(stretch.storage_w + held_w);
        let empty_s = if short_w > 0.0 { free_j.max(0.0) / short_w } else { f64::INFINITY };
        if empty_s < stretch.dt_s {
            let unpaid_w = short_w.min(drain_w);
            let starved = Segment { draw_w: segment.draw_w - unpaid_w, internal_w: segment.internal_w - unpaid_w, ..segment };
            let empty_s = at_s + empty_s;
            return [Some((segment, empty_s)), Some((starved, dt_s - empty_s))].into_iter().flatten();
        }
        free_j -= short_w * stretch.dt_s;
        at_s += stretch.dt_s;
    }
    [Some((segment, dt_s)), None].into_iter().flatten()
}

/// A stretch of constant refit inputs, ending at `until_s`.
struct Piece {
    until_s: f64,
    /// To the field.
    losing_w: f64,
    /// Out of storage; negative for a return.
    moving_w: f64,
    /// Of the steps ending at `until_s`, less their planned spill: the flow spills what storage
    /// actually holds.
    vent_j: f64,
}

/// `edges` are further cuts, inside `(since_s, now_s)`.
fn pieces(plan: Option<&Plan>, balance: &Balance, since_s: f64, now_s: f64, edges: &[f64]) -> Vec<Piece> {
    let (steps, start_s) = plan.map_or((&[][..], 0.0), |plan| (plan.steps(), plan.round().start_s));
    let mut cuts: Vec<f64> = steps
        .iter()
        .flat_map(|s| [start_s + s.begins_s, start_s + s.ends_s()])
        .filter(|&t| t > since_s && t < now_s)
        .chain(edges.iter().copied())
        .chain([now_s])
        .collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut from_s = since_s;
    cuts.into_iter()
        .map(|until_s| {
            let middle_s = 0.5 * (from_s + until_s);
            let under_way = steps.iter().find(|s| start_s + s.begins_s < middle_s && middle_s < start_s + s.ends_s());
            let vent_j = steps
                .iter()
                .filter(|s| start_s + s.ends_s() == until_s)
                .map(|s| s.vented_j - s.spilled_j)
                .sum();
            from_s = until_s;
            Piece {
                until_s,
                losing_w: under_way.map_or(0.0, |s| losing_w(s, balance)),
                moving_w: under_way.map_or(0.0, |s| -s.stored_j / s.duration_s),
                vent_j,
            }
        })
        .collect()
}

fn losing_w(step: &Step, balance: &Balance) -> f64 {
    match step.change.phase() {
        Phase::Dismantle if step.duration_s > 0.0 => (1.0 - balance.recovery) * step.gross_j / step.duration_s,
        _ => 0.0,
    }
}

/// Of a step, the fraction still to run at `t`.
fn left(plan: &Plan, step: &Step, t: f64) -> f64 {
    if step.duration_s > 0.0 {
        1.0 - ((t - plan.round().start_s - step.begins_s) / step.duration_s).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

/// Still to be taken by builds at `t`, joules.
fn building_j(plan: &Plan, t: f64) -> f64 {
    let start_s = plan.round().start_s;
    plan.steps()
        .iter()
        .filter(|s| s.stored_j < 0.0 && start_s + s.ends_s() > t)
        .map(|s| -s.stored_j * left(plan, s, t))
        .sum()
}

/// Finish `plan` at once from `since_s`, step by step, spilling at each step's end. Returns storage
/// after and the heat added.
pub(super) fn skip(plan: &Plan, since_s: f64, stored_j: f64, balance: &Balance) -> (f64, f64) {
    let start_s = plan.round().start_s;
    let (mut stored_j, mut heat_j) = (stored_j, 0.0);
    for s in plan.steps().iter().filter(|s| start_s + s.ends_s() > since_s) {
        let left = left(plan, s, since_s);
        stored_j += s.stored_j * left;
        heat_j += losing_w(s, balance) * s.duration_s * left + s.vented_j - s.spilled_j;
        let capacity_j = Capacities::of(&plan.at(start_s + s.ends_s()).form, balance).storage_j;
        let spilled_j = (stored_j - capacity_j).max(0.0);
        stored_j -= spilled_j;
        heat_j += spilled_j;
    }
    (stored_j.max(0.0), heat_j)
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

    /// 30's table: full at 0.1 AU, and filling there half a year in.
    #[test]
    fn the_starting_ship_at_a_tenth_of_an_au_settles_at_thirtys_temperatures() {
        let b = Balance::DEFAULT;
        let mut full = Fitting::full(Form::starting(), b, 0.0);
        full.set_starlight_w(starlight_w(&b, 0.1));
        let later_s = 40.0 * b.field_tau_s;
        full.settle(&rest(), later_s);
        let full_k = full.temperature_k_at(&rest(), later_s);
        assert!((full_k - 3_237.0).abs() < 0.5, "full {full_k}");
        assert_eq!(full.stored_j_at(&rest(), later_s), full.hull().capacities.storage_j, "held full");

        let mut filling = starting(b, 0.0);
        filling.set_starlight_w(starlight_w(&b, 0.1));
        let half_year_s = 0.5 * crate::flight::JULIAN_YEAR_S;
        let filling_k = filling.temperature_k_at(&rest(), half_year_s);
        assert!((filling_k - 2_396.0).abs() < 0.5, "filling {filling_k}");
        let stored = filling.stored_j_at(&rest(), half_year_s) / filling.hull().capacities.storage_j;
        assert!((stored - 0.5).abs() < 0.01, "{stored} full");
    }

    /// Beyond the rating, what arrives is heat.
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
        let heat_j = fitting.heat_j_at(&rest(), dt_s) - fitting.heat_j_at(&rest(), 0.0);
        let unconverted_j = (3.0 * rating_w - b.conversion_efficiency * rating_w) * dt_s;
        assert!(close(heat_j, unconverted_j, 1e-4), "{heat_j} {unconverted_j}");
    }

    /// Storage fills partway through; one leap and a thousand settlements agree.
    #[test]
    fn one_leap_and_many_settlements_agree() {
        let b = Balance::DEFAULT;
        let mut leap = starting(b, 20.0 * me(&b));
        leap.set_starlight_w(starlight_w(&b, 0.05));
        let mut steps = leap.clone();
        let end_s = 1.0e7;
        let room_j = leap.hull().capacities.storage_j - 20.0 * me(&b);
        assert!(close(leap.flow(None, end_s).income_j, room_j, 1e-12), "premise: it fills");
        assert!(leap.intake(&leap.hull().capacities, Mode::Black, 0.0, room_j, 0.0, 0.0).fill_s().unwrap() < 0.5 * end_s, "premise: it fills early");
        leap.settle(&rest(), end_s);
        for k in 1..=1000 {
            steps.settle(&rest(), end_s * f64::from(k) / 1000.0);
        }
        assert!(close(steps.heat_j, leap.heat_j, 1e-9), "{} {}", steps.heat_j, leap.heat_j);
        assert!(close(steps.stored_j, leap.stored_j, 1e-12), "{} {}", steps.stored_j, leap.stored_j);
        assert_eq!(leap.stored_j, leap.hull().capacities.storage_j);
    }

    /// `P_in` is the rate heat moves at plus what it radiates, in the stretch holding the instant:
    /// before storage fills and after.
    #[test]
    fn the_power_in_force_is_where_heat_is_heading() {
        let b = Balance::DEFAULT;
        let mut fitting = starting(b, 20.0 * me(&b));
        fitting.set_starlight_w(starlight_w(&b, 0.05));
        let room_j = fitting.hull().capacities.storage_j - 20.0 * me(&b);
        let fill_s = fitting.intake(&fitting.hull().capacities, Mode::Black, 0.0, room_j, 0.0, 0.0).fill_s().unwrap();
        let tau_s = b.field_tau_s;
        let mut seen = Vec::new();
        for t in [0.3 * fill_s, 1.5 * fill_s] {
            let dt_s = 1.0e-4 * fill_s;
            let rate_w = (fitting.heat_j_at(&rest(), t + dt_s) - fitting.heat_j_at(&rest(), t - dt_s)) / (2.0 * dt_s);
            let heat_w = fitting.heat_w_at(&rest(), t);
            let want_w = rate_w + fitting.heat_j_at(&rest(), t) / tau_s;
            assert!(close(heat_w, want_w, 1e-5), "at {t}: {heat_w} {want_w}");
            seen.push(heat_w);
        }
        assert!(seen[1] > 1.01 * seen[0], "premise: filling converts what full storage cannot: {seen:?}");
        let later_s = fill_s + 40.0 * tau_s;
        assert!(close(fitting.heat_w_at(&rest(), 1.5 * fill_s) * tau_s, fitting.heat_j_at(&rest(), later_s), 1e-9));
    }

    /// The drain storage cannot pay makes no heat.
    #[test]
    fn an_empty_ship_far_from_a_star_cools() {
        let b = Balance::DEFAULT;
        let fitting = starting(b, 0.0);
        let idle_j = fitting.heat_j_at(&rest(), 0.0);
        let later_j = fitting.heat_j_at(&rest(), b.field_tau_s);
        assert!(close(later_j, idle_j / std::f64::consts::E, 1e-12), "{later_j} {idle_j}");
        let paid = starting(b, 1.0e3 * me(&b));
        assert!(close(paid.heat_j_at(&rest(), b.field_tau_s), idle_j, 1e-12), "the drain holds it at idle");
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
        let before_j = fitting.heat_j_at(&rest(), end_s - just_s);
        let holding_w = fitting.hull().capacities.drain_w + losing_w(step, &b);
        let relaxed_j = fitting.field().heat_after_j(before_j, holding_w, just_s);
        let jump_j = fitting.heat_j_at(&rest(), end_s) - relaxed_j;
        assert!(close(jump_j, overflow_j, 1e-9), "{jump_j} {overflow_j}");
    }

    /// With radiation off, a dismantling into full storage ends with all it took apart as heat,
    /// however it is settled or finished.
    #[test]
    fn a_dismantling_into_full_storage_ends_as_heat() {
        let b = Balance { living_density_w: 0.0, field_tau_s: 1.0e40, ..Balance::DEFAULT };
        let mut fitting = Fitting::full(Form::starting(), b, 0.0);
        assert_eq!(fitting.heat_j, 0.0, "premise: nothing holds any heat");
        let plan = begin(&mut fitting, shrunk_engine());
        let step = plan.steps()[0];
        let end_s = step.ends_s();
        let loss_j = (1.0 - b.recovery) * step.gross_j;
        assert!(close(fitting.heat_j_at(&rest(), 0.5 * end_s), 0.5 * loss_j, 1e-9), "{}", fitting.heat_j_at(&rest(), 0.5 * end_s));
        assert!(close(fitting.heat_j_at(&rest(), end_s), step.gross_j, 1e-9));

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

    /// Radiation and drain off: a return into a store starlight is filling stops at capacity, and
    /// the rest is heat.
    #[test]
    fn a_return_into_room_starlight_has_filled_is_heat() {
        let b = Balance { living_density_w: 0.0, field_tau_s: 1.0e40, ..Balance::DEFAULT };
        let mut fitting = Fitting::full(Form::starting(), b, 0.0);
        let capacity_j = fitting.hull().capacities.storage_j;
        let room_j = 0.5 * me(&b);
        fitting.drain(room_j);
        let arriving_w = fitting.hull().capacities.aperture_w;
        fitting.set_starlight_w(arriving_w);
        let plan = begin(&mut fitting, shrunk_engine());
        let [step] = plan.steps() else { panic!("{:?}", plan.steps()) };
        assert!(close(step.stored_j, room_j, 1e-12), "premise: the plan fills the room itself");
        let end_s = step.ends_s();
        assert!(fitting.solar_w() * end_s > 4.0 * room_j, "premise: starlight fills it well before the end");

        for k in 0..=400 {
            let t = end_s * f64::from(k) / 400.0;
            let stored_j = fitting.stored_j_at(&rest(), t);
            assert!(stored_j <= capacity_j * (1.0 + 1e-14), "{t}: {stored_j} over {capacity_j}");
        }
        assert_eq!(fitting.stored_j_at(&rest(), 0.5 * end_s), capacity_j, "premise: full by halfway");

        // Everything the step took apart, and all starlight brought, is storage or heat.
        let heat_0 = fitting.heat_j;
        // Until the step ends, the return it will vent is still in the part.
        let taken_j = |t: f64| if t < end_s { ((1.0 - b.recovery) * step.gross_j + step.stored_j) * t / end_s } else { step.gross_j };
        for t in [0.5 * end_s, end_s, 1.5 * end_s] {
            let gained_j = fitting.heat_j_at(&rest(), t) - heat_0 + fitting.stored_j_at(&rest(), t) - (capacity_j - room_j);
            let want_j = arriving_w * t + taken_j(t);
            assert!(close(gained_j, want_j, 1e-9), "{t}: {gained_j} {want_j}");
        }

        let mut finished = fitting.clone();
        finished.settle(&rest(), 0.25 * end_s);
        assert!(finished.finish_refit());
        let gained_j = finished.heat_j - heat_0 + finished.stored_j - (capacity_j - room_j);
        assert!(finished.stored_j <= capacity_j, "{}", finished.stored_j);
        assert!(close(gained_j, arriving_w * 0.25 * end_s + step.gross_j, 1e-9), "finished: {gained_j}");
    }

    /// A full store shrinks, then the engine grows, under starlight. Ticks fall between step ends.
    #[test]
    fn a_round_settled_at_every_tick_agrees_with_one_leap() {
        let b = Balance::DEFAULT;
        let mut leap = Fitting::full(Form::starting(), b, 0.0);
        leap.set_starlight_w(starlight_w(&b, 0.05));
        let mut target = shrunk_engine();
        target.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 7.0 / 3.0;
        target.parts.iter_mut().find(|p| p.kind == crate::form::Kind::Storage).unwrap().volume_m3 *= 0.9;
        let plan = begin(&mut leap, target.clone());
        assert!(plan.steps().len() >= 2, "premise: {:?}", plan.steps());
        assert!(plan.steps().iter().any(|s| s.spilled_j > 0.0), "premise: the store spills");
        let end_s = 1.3 * plan.duration_s();
        let mut ticks = leap.clone();
        let n = 997;
        for k in 1..=n {
            let t = end_s * f64::from(k) / f64::from(n);
            ticks.settle(&rest(), t);
            assert!(ticks.stored_j <= ticks.hull().capacities.storage_j * (1.0 + 1e-14), "{t}: over capacity");
        }
        assert!(plan.steps().iter().all(|s| (s.ends_s() * f64::from(n) / end_s).fract() > 1e-6), "premise: ends between ticks");
        // Settled exactly on every step end as well, which must count each vent and spill once.
        let mut exact = leap.clone();
        for step in plan.steps() {
            exact.settle(&rest(), plan.round().start_s + step.ends_s());
            exact.settle(&rest(), plan.round().start_s + step.ends_s());
        }
        exact.settle(&rest(), end_s);
        let (heat_j, stored_j) = (leap.heat_j_at(&rest(), end_s), leap.stored_j_at(&rest(), end_s));
        leap.settle(&rest(), end_s);
        assert_eq!((leap.heat_j, leap.stored_j), (heat_j, stored_j), "a read agrees with a settlement");
        assert_eq!(ticks.form(), &target);
        assert!(close(ticks.heat_j, leap.heat_j, 1e-9), "{} {}", ticks.heat_j, leap.heat_j);
        assert!(close(ticks.stored_j, leap.stored_j, 1e-9), "{} {}", ticks.stored_j, leap.stored_j);
        assert!(close(exact.heat_j, leap.heat_j, 1e-9), "{} {}", exact.heat_j, leap.heat_j);
        assert!(close(exact.stored_j, leap.stored_j, 1e-9), "{} {}", exact.stored_j, leap.stored_j);
        assert_eq!(leap.stored_j, leap.hull().capacities.storage_j, "premise: refilled by the end");
    }

    /// A build planned to within a little of storage, with starlight too weak for the drain: the
    /// drain starves, and the build is never short.
    #[test]
    fn the_drain_cannot_eat_what_a_build_will_take() {
        let b = Balance::DEFAULT;
        let mut target = Form::starting();
        target.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 7.0 / 5.0;
        let trial = begin(&mut starting(b, 30.0 * me(&b)), target.clone());
        let cost_j = -trial.steps().iter().map(|s| s.stored_j).sum::<f64>();
        let duration_s = trial.duration_s();
        let drain_w = Capacities::of(&Form::starting(), &b).drain_w;
        let spare_j = 0.1 * drain_w * duration_s;
        let mut leap = starting(b, cost_j + spare_j);
        leap.set_starlight_w(0.5 * drain_w / b.conversion_efficiency);
        assert!(close(leap.solar_w(), 0.5 * drain_w, 1e-12), "premise: under the rating");
        begin(&mut leap, target);
        let end_s = 1.5 * duration_s;

        for k in 0..=300 {
            let t = end_s * f64::from(k) / 300.0;
            let held_j = leap.stored_j + leap.flow(None, t).income_j;
            assert!(held_j >= -1e-9 * cost_j, "{t}: {held_j}");
        }
        let mut ticks = leap.clone();
        for k in 1..=997 {
            ticks.settle(&rest(), end_s * f64::from(k) / 997.0);
        }
        leap.settle(&rest(), end_s);
        assert!(leap.stored_j.abs() <= 1e-9 * cost_j, "the build took the rest: {}", leap.stored_j);
        assert!((ticks.stored_j - leap.stored_j).abs() <= 1e-9 * cost_j, "{} {}", ticks.stored_j, leap.stored_j);
        assert!(close(ticks.heat_j, leap.heat_j, 1e-9), "{} {}", ticks.heat_j, leap.heat_j);
    }

    /// Settled only at the round's start, the new drain and rating still apply from the step end.
    #[test]
    fn the_drain_and_rating_change_at_the_step_end() {
        let b = Balance::DEFAULT;
        let mut fitting = starting(b, 20.0 * me(&b));
        let mut target = Form::starting();
        target.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 7.0 / 5.0;
        target.parts.iter_mut().find(|p| p.kind == crate::form::Kind::Living).unwrap().volume_m3 *= 1.5;
        let plan = begin(&mut fitting, target.clone());
        let (was, now) = (fitting.hull().capacities, Capacities::of(&target, &b));
        assert!(now.aperture_w > was.aperture_w && now.drain_w > was.drain_w, "premise");
        fitting.set_starlight_w(3.0 * now.aperture_w);
        let end_s = plan.duration_s();
        let dt_s = 1.0e3;
        let rate_w = (fitting.stored_j_at(&rest(), end_s + dt_s) - fitting.stored_j_at(&rest(), end_s)) / dt_s;
        let want_w = b.conversion_efficiency * now.aperture_w - now.drain_w;
        assert!(close(rate_w, want_w, 1e-6), "{rate_w} {want_w}");

        let heat_j = fitting.heat_j_at(&rest(), end_s);
        let absorbed_w = 3.0 * now.aperture_w;
        let heat_w = absorbed_w - b.conversion_efficiency * now.aperture_w + now.drain_w;
        let want_j = fitting.field().heat_after_j(heat_j, heat_w, dt_s);
        assert!(close(fitting.heat_j_at(&rest(), end_s + dt_s), want_j, 1e-9), "{} {want_j}", fitting.heat_j_at(&rest(), end_s + dt_s));
    }

    /// Once heat is spent, all that is absorbed pays the exhaust, conversion's loss included.
    #[test]
    fn a_burn_is_paid_from_heat_then_storage_as_it_goes() {
        use crate::flight::{Cruise, Drive};
        let b = Balance::DEFAULT;
        let mut leap = Fitting::full(Form::starting(), b, 0.0);
        leap.set_starlight_w(starlight_w(&b, 1.0));
        let drive = Drive { accel_g: 1.0, ..Drive::DEFAULT };
        let cruise = Cruise::plan(DVec3::ZERO, DVec3::X * 1.0e-3, 0.0, drive);
        let end_s = cruise.duration_s();
        let mut motion = rest();
        motion.begin_crossing(cruise, None);
        let committed_j = leap.commit(&motion, 0.0);
        let capacity_j = leap.hull().capacities.storage_j;
        let (heat_j, absorbed_w) = (leap.heat_j, leap.starlight_w());
        let exhaust_w = leap.emitted_w(&motion, 0.0, end_s);
        assert!(committed_j > 0.0 && committed_j < capacity_j, "premise: {committed_j}");
        assert!(absorbed_w * end_s > 10.0 * heat_j && absorbed_w < 0.1 * exhaust_w, "premise: starlight matters, but less than the drive");

        let mut ticks = leap.clone();
        for k in 1..=1000 {
            ticks.settle(&motion, end_s * f64::from(k) / 1000.0);
        }
        assert_eq!(leap.heat_j_at(&motion, end_s), 0.0);
        let stored_j = leap.stored_j_at(&motion, end_s);
        // Radiation stops once the drive has drawn heat to the floor.
        let radiated_j = heat_j * (heat_j / (exhaust_w - absorbed_w)) / b.field_tau_s;
        let want_j = capacity_j - committed_j + heat_j + absorbed_w * end_s;
        assert!(stored_j <= want_j && stored_j >= want_j - radiated_j - 1e-12 * capacity_j, "{stored_j} {want_j} {radiated_j}");
        assert!(leap.committed_j_at(&motion, end_s) < 1e-9 * committed_j, "all of it spent");
        let mass_kg = leap.settled_mass_kg();
        leap.settle(&motion, end_s);
        let gained_j = heat_j + absorbed_w * end_s;
        let repriced_j = 2.0 * committed_j * gained_j / (mass_kg * crate::fitting::C2);
        assert!(repriced_j < 0.5 * gained_j, "premise: {repriced_j} {gained_j}");
        assert!((ticks.stored_j - leap.stored_j).abs() < repriced_j, "{} {}", ticks.stored_j, leap.stored_j);
        assert_eq!(ticks.heat_j, leap.heat_j);
    }

    /// Only the exhaust leaves.
    fn quiet() -> Balance {
        Balance { living_density_w: 0.0, field_tau_s: 1.0e40, ..Balance::DEFAULT }
    }

    /// Coasts between the burns.
    fn hop() -> (ShipState, crate::flight::Cruise) {
        use crate::flight::{Cruise, Drive};
        let drive = Drive { accel_g: 0.01, max_beta: 3.0e-6, ..Drive::DEFAULT };
        let cruise = Cruise::plan(DVec3::ZERO, DVec3::X * 1.0e-7, 0.0, drive);
        let mut motion = rest();
        motion.begin_crossing(cruise.clone(), None);
        (motion, cruise)
    }

    /// Returns the commitment.
    fn hot(b: Balance, stored_j: f64, heat_j: f64, motion: &ShipState) -> (Fitting, f64) {
        let full = Fitting::full(Form::starting(), b, 0.0);
        let mut fitting = Fitting::from_account(&Account { stored_j, heat_j, ..full.account() }, b);
        let committed_j = fitting.commit(motion, 0.0);
        (fitting, committed_j)
    }

    /// Heat pays first and storage the rest, while the commitment runs down by all of it.
    #[test]
    fn a_hot_burn_pays_storage_only_what_heat_could_not() {
        let b = quiet();
        let (motion, cruise) = hop();
        let end_s = cruise.duration_s();
        assert!(cruise.coast_s() > 0.0, "premise: it coasts");
        let stored_j = 20.0 * me(&b);
        let (_, cost_j) = hot(b, stored_j, 0.0, &motion);
        for share in [0.3, 1.5] {
            let (fitting, committed_j) = hot(b, stored_j, share * cost_j, &motion);
            let heat_j = fitting.heat_j;
            let mass_kg = fitting.settled_mass_kg();
            let heat_end_j = fitting.heat_j_at(&motion, end_s);
            // Storage is far larger than the burn, so read the change rather than a difference.
            let paid_j = -fitting.flow(Some(&motion), end_s).income_j;
            let from_heat_j = heat_j.min(committed_j);
            assert!(heat_end_j < heat_j, "{share}: colder");
            assert!((heat_end_j - (heat_j - from_heat_j)).abs() <= 1e-9 * committed_j, "{share}: {heat_end_j}");
            let want_j = committed_j - from_heat_j;
            assert!((paid_j - want_j).abs() <= 1e-9 * committed_j, "{share}: {paid_j} {want_j}");
            assert!(fitting.committed_j_at(&motion, end_s) <= 1e-9 * committed_j, "{share}: all of it released");
            // The rocket law, whatever paid.
            let lighter_kg = mass_kg * (-crate::cost::planned_rapidity(&motion)).exp();
            let mass_end_kg = fitting.mass_kg_at(&motion, end_s);
            assert!(close(mass_end_kg, lighter_kg, 1e-12), "{share}: {mass_end_kg} {lighter_kg}");
        }
    }

    /// With no heat to draw on, a burn costs storage exactly what it did before exhaust drew on it.
    #[test]
    fn a_cold_burn_costs_what_it_always_did() {
        let b = quiet();
        let (motion, cruise) = hop();
        let (fitting, committed_j) = hot(b, 20.0 * me(&b), 0.0, &motion);
        let end_s = cruise.duration_s();
        assert_eq!(fitting.heat_j_at(&motion, end_s), 0.0);
        let cost_j = -fitting.flow(Some(&motion), end_s).income_j;
        let want_j = crate::cost::energy_j(fitting.settled_mass_kg(), crate::cost::planned_rapidity(&motion), 1.0);
        assert!(close(cost_j, want_j, 1e-12) && close(committed_j, want_j, 1e-15), "{cost_j} {want_j}");
    }

    /// Heat floors in the boost, recovers in the coast and is drawn again in the brake; one leap
    /// and every tick agree.
    #[test]
    fn a_hot_burn_settled_every_tick_agrees_with_one_leap() {
        let b = Balance::DEFAULT;
        let (motion, cruise) = hop();
        let end_s = cruise.duration_s();
        let [_, line_s, boost_end_s, brake_s, _] = cruise.phase_changes_s();
        let (_, cost_j) = hot(b, 20.0 * me(&b), 0.0, &motion);
        let (mut leap, _) = hot(b, 20.0 * me(&b), 0.1 * cost_j, &motion);
        leap.set_starlight_w(starlight_w(&b, 1.0));
        let heat_j = leap.heat_j;
        let floor_s = 0.5 * (line_s + boost_end_s);
        let made_w = leap.field().heat_filling_w(&leap.intake(&leap.hull().capacities, Mode::Black, 0.0, 1.0, 0.0, 0.0));
        assert!(made_w < leap.emitted_w(&motion, line_s, boost_end_s), "premise: the boost outruns the heat made");
        assert_eq!(leap.heat_j_at(&motion, floor_s), 0.0, "premise: at the floor partway through the boost");
        let coast_s = 0.5 * (boost_end_s + brake_s);
        let coasting_j = leap.heat_j_at(&motion, coast_s);
        let unlit_j = leap.field().heat_after_j(0.0, made_w, coast_s - boost_end_s);
        assert!(close(coasting_j, unlit_j, 1e-6), "unlit through the coast: {coasting_j} {unlit_j}");

        let mut ticks = leap.clone();
        let n = 997;
        let mut coasted = None;
        for k in 1..=n {
            let t = end_s * f64::from(k) / f64::from(n);
            ticks.settle(&motion, t);
            if coasted.is_none() && t >= coast_s {
                coasted = Some((t, ticks.heat_j));
            }
        }
        let (t, ticked_j) = coasted.unwrap();
        assert!(close(ticked_j, leap.heat_j_at(&motion, t), 1e-9), "{ticked_j} {}", leap.heat_j_at(&motion, t));
        let (mass_kg, stored_j) = (leap.settled_mass_kg(), leap.stored_j);
        leap.settle(&motion, end_s);
        // A burn is priced on the mass settled, which starlight and heat change between ticks.
        let moved_j = heat_j + (leap.stored_j - stored_j).abs();
        let repriced_j = 2.0 * cost_j * moved_j / (mass_kg * crate::fitting::C2);
        assert!(leap.heat_j > 0.0, "premise: the brake draws the coast's heat without reaching the floor");
        assert!((ticks.stored_j - leap.stored_j).abs() < repriced_j, "{} {} {repriced_j}", ticks.stored_j, leap.stored_j);
        assert!((ticks.heat_j - leap.heat_j).abs() < repriced_j, "{} {} {repriced_j}", ticks.heat_j, leap.heat_j);
    }

    /// Past its rated load, a ship at rest collapses partway through the boost; burning, it does
    /// not.
    #[test]
    fn a_burn_puts_off_a_collapse() {
        use crate::flight::{Cruise, Drive};
        let b = Balance::DEFAULT;
        let cruise = Cruise::plan(DVec3::ZERO, DVec3::X * 1.0e-4, 0.0, Drive { accel_g: 2.0, ..Drive::DEFAULT });
        let boost_end_s = cruise.phase_changes_s()[2];
        let mut motion = rest();
        motion.begin_crossing(cruise, None);
        let (probe, _) = hot(b, 20.0 * me(&b), 0.0, &motion);
        let (field, caps) = (probe.field(), probe.hull().capacities);
        let (rated_w, heat_max_j) = (field.rated_load_w(), field.heat_max_j());
        let made_w = 1.2 * rated_w;
        let starlight_w = made_w + b.conversion_efficiency * caps.aperture_w - caps.drain_w;
        assert!(starlight_w > caps.aperture_w, "premise: past the rating, so all the rest is heat");
        // Reaching `Q_max` a quarter of the way through the boost.
        let heat_j = field.equilibrium_j(made_w) - field.equilibrium_j(made_w - rated_w) * (0.25 * boost_end_s / field.tau_s).exp();
        let (mut burning, _) = hot(b, 20.0 * me(&b), heat_j, &motion);
        burning.set_starlight_w(starlight_w);
        let exhaust_w = burning.emitted_w(&motion, 0.0, boost_end_s);
        assert!(made_w - exhaust_w < rated_w, "premise: burning, it settles under the limit");

        let at_rest = burning.collapse_s(&rest(), boost_end_s).expect("premise: at rest it collapses");
        assert!(close(at_rest, 0.25 * boost_end_s, 1e-6), "premise: {at_rest}");
        assert_eq!(burning.collapse_s(&motion, boost_end_s), None);
        assert!(burning.heat_j_at(&motion, boost_end_s) < heat_j);
        assert!(burning.heat_j_at(&rest(), boost_end_s) > heat_max_j);
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
