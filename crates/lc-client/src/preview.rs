//! The editor's live budget and preview: what the draft would cost as a round begun now, what it
//! would do to the field, and what the ship it makes would be. See
//! `lightcone/docs/29-ship-form.md` §The budget and §What else it shows.
//!
//! The round is solved from the inputs the shard's `Craft::begin_refit` takes: the ship's form,
//! what it stores now, and now. So the budget is [`Budget::of`] on the plan an Apply would get.
//!
//! Heat is not kept yet (H3), so the ship's `Q` is unknown: the field starts at its idle heat,
//! `q_idle` over its envelope. H3 swaps in the ship's real `Q` at [`Heat::of`]'s `base_j`.

use lc_world::field::Field;
use lc_world::fitting::{Balance, C2, Fitting};
use lc_world::flight::{C_M_S, G0};
use lc_world::form::capacity::{Capacities, aft_aperture_w, dry_mass_kg};
use lc_world::form::grid::FormGrid;
use lc_world::form::{Form, FormError};
use lc_world::refit::rounds::{Phase, Plan, Refusal, Round};
use lc_world::solar;

use crate::draft::{Draft, Edit, What};
use crate::ledger::Budget;
use crate::session::Session;
use crate::ui::UiState;

/// A round's recipe but its target: what the ship would begin one with now.
#[derive(Clone, Debug, PartialEq)]
pub struct Start {
    pub from: Form,
    pub stored_j: f64,
    pub start_s: f64,
    pub balance: Balance,
}

impl Start {
    /// `None` for a ship with no form, which only a shard gives it.
    pub fn of(session: &Session) -> Option<Start> {
        let fitting = session.ship.fitting()?;
        let now = session.coordinate_time_s();
        let stored_j = fitting.stored_j_at(&session.ship.motion, now);
        let start = Start { from: fitting.form().clone(), stored_j, start_s: now, balance: *fitting.balance() };
        // The draft is edited against a running round's target (`ledger::base`), so the next round
        // begins there, with what this one has still to take or return.
        Some(match fitting.refit() {
            Some(plan) => {
                let end = plan.round().start_s + plan.duration_s();
                let left_j = plan.at(end).stored_j - plan.at(now.max(plan.round().start_s)).stored_j;
                Start { from: plan.target().clone(), stored_j: (stored_j + left_j).max(0.0), ..start }
            }
            None => start,
        })
    }

    pub fn round(&self, target: &Form) -> Round {
        Round { from: self.from.clone(), target: target.clone(), stored_j: self.stored_j, start_s: self.start_s }
    }

    pub fn solve(&self, target: &Form) -> Result<Plan, Refusal> {
        self.round(target).solve(&self.balance)
    }

    /// How far storage falls short of paying for `target`: zero when it can, `None` when the round
    /// is refused for something that is not the budget's.
    pub fn short_j(&self, target: &Form) -> Option<f64> {
        match self.solve(target) {
            Ok(_) => Some(0.0),
            Err(Refusal::Energy { short_j }) => Some(short_j),
            Err(_) => None,
        }
    }

    /// Whether `edit` leaves the draft no further short than its gesture began: a part that cannot
    /// be paid for cannot be placed, and a handle stops where storage runs out. Measured from the
    /// gesture's start rather than the last frame, so a drag at its limit can still come back.
    ///
    /// A whole new draft (a preset, the ship) is not held to it, and a refusal that is not the
    /// budget's is left to the edit and to Apply.
    pub fn allows(&self, draft: &Draft, edit: &Edit) -> bool {
        if edit.what == What::Whole {
            return true;
        }
        let with = |edit: &Edit| {
            let mut d = draft.clone();
            d.apply(edit, &self.balance).map(|()| d.form)
        };
        let Ok(after) = with(edit) else { return true };
        let began = with(&edit.inverse()).unwrap_or_else(|_| draft.form.clone());
        match (self.short_j(&began), self.short_j(&after)) {
            (Some(was), Some(now)) => now <= was,
            _ => true,
        }
    }
}

/// The field through a round: the heat it peaks at, and after which step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heat {
    pub peak_j: f64,
    pub peak_k: f64,
    /// `None` when nothing the round does raises it.
    pub step: Option<usize>,
    pub max_j: f64,
    /// Where it fails: the same for every field, about 4 600 K.
    pub max_k: f64,
}

impl Heat {
    /// From `base_j`, held there by whatever holds it now. Each dismantling adds what it loses,
    /// spread over the step, and each vent its burst at the step's end, which is where the heat
    /// peaks: between vents it only relaxes toward the equilibrium.
    pub fn of(plan: &Plan, field: &Field, base_j: f64, balance: &Balance) -> Heat {
        let holding_w = base_j / field.tau_s;
        let (mut heat_j, mut peak_j, mut step) = (base_j, base_j, None);
        for (i, s) in plan.steps().iter().enumerate() {
            let losing_w = match s.change.phase() {
                Phase::Dismantle if s.duration_s > 0.0 => (1.0 - balance.recovery) * s.gross_j / s.duration_s,
                _ => 0.0,
            };
            heat_j = field.heat_after_j(heat_j, holding_w + losing_w, s.duration_s) + s.vented_j;
            if heat_j > peak_j {
                (peak_j, step) = (heat_j, Some(i));
            }
        }
        let max_j = field.heat_max_j();
        Heat { peak_j, peak_k: field.temperature_k(peak_j), step, max_j, max_k: field.temperature_k(max_j) }
    }

    pub fn collapses(&self) -> bool {
        self.peak_j >= self.max_j
    }
}

/// The ship's field as it stands, and the heat it starts a round with: idle, until H3.
pub fn field_now(fitting: &Fitting) -> (Field, f64) {
    let field = Field::of(fitting.geometry().envelope_area_m2, fitting.balance());
    (field, field.idle_j_m2 * field.area_m2)
}

/// What the grid says about a form. Tens of milliseconds, so the editor builds it off the frame
/// when the draft changes, and the preview reads the latest one done.
#[derive(Clone, Debug, PartialEq)]
pub struct Measured {
    pub form: Form,
    /// Per kilogram, as `Fitted` states it.
    pub geometry: lc_proto::form::Geometry,
    pub envelope_m2: f64,
    pub broadside_m2: f64,
    pub gyration_m: f64,
    pub extent_m: f64,
}

impl Measured {
    pub fn of(form: &Form, balance: &Balance) -> Result<Measured, FormError> {
        let grid = FormGrid::new(form, balance)?;
        Ok(Measured {
            form: form.clone(),
            geometry: grid.geometry(1.0),
            envelope_m2: grid.envelope_area_m2(),
            broadside_m2: grid.broadside_m2(),
            gyration_m: grid.gyration_m(),
            extent_m: grid.extent_m(),
        })
    }
}

/// The draft's geometry, its field, and what it would collect here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
    pub broadside_m2: f64,
    pub envelope_m2: f64,
    pub slew_rad_s: f64,
    pub rated_load_w: f64,
    /// Between the field's idle heat and collapse, joules: the most a burst can add.
    pub headroom_j: f64,
    /// Starlight collected at the ship's distance from its star now, held as an idle ship holds
    /// itself to it, which is also how bright it is in reflected light. Zero under way, and between
    /// systems.
    pub starlight_w: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    pub budget: Result<Budget, Refusal>,
    pub heat: Option<Heat>,
    pub duration_s: Option<f64>,
    pub capacities: Capacities,
    /// With storage full.
    pub accel_g: f64,
    /// `None` until the draft as it is now has been measured.
    pub shape: Option<Shape>,
}

impl Preview {
    /// `None` with no draft, or a ship with no form. `measured` counts only if it is of the draft.
    pub fn of(session: &Session, ui: &UiState, measured: Option<&Measured>) -> Option<Preview> {
        let draft = ui.form.draft.as_ref()?;
        let start = Start::of(session)?;
        let fitting = session.ship.fitting()?;
        let balance = &start.balance;
        let plan = start.solve(&draft.form);
        let heat = plan.as_ref().ok().map(|plan| {
            let (field, base_j) = field_now(fitting);
            Heat::of(plan, &field, base_j, balance)
        });
        let capacities = Capacities::of(&draft.form, balance);
        let full_kg = dry_mass_kg(&draft.form, balance) + capacities.storage_j / C2;
        let thrust_n = aft_aperture_w(&draft.form, balance).unwrap_or(0.0) / C_M_S;
        let shape = measured.filter(|m| m.form == draft.form).map(|m| {
            let field = Field::of(m.envelope_m2, balance);
            Shape {
                broadside_m2: m.broadside_m2,
                envelope_m2: m.envelope_m2,
                slew_rad_s: lc_world::attitude::rate_for_gyration(m.gyration_m),
                rated_load_w: field.rated_load_w(),
                headroom_j: field.heat_max_j() - field.idle_j_m2 * field.area_m2,
                starlight_w: starlight_w(session, &m.geometry, balance, start.start_s),
            }
        });
        Some(Preview {
            budget: plan.as_ref().map(|plan| Budget::of(plan, balance)).map_err(|r| *r),
            heat,
            duration_s: plan.as_ref().ok().map(Plan::duration_s),
            capacities,
            accel_g: thrust_n / full_kg / G0,
            shape,
        })
    }

    pub fn collapses(&self) -> bool {
        self.heat.is_some_and(|h| h.collapses())
    }
}

/// What `geometry` would collect where the ship is at `t`, turned as `solar` turns an idle hull.
fn starlight_w(session: &Session, geometry: &lc_proto::form::Geometry, balance: &Balance, t: f64) -> f64 {
    let ship = &session.ship;
    let (Some(system), Some(distance_m), false) = (ship.system.as_deref(), ship.star_distance_m_at(t), ship.motion.is_under_way()) else {
        return 0.0;
    };
    let shadow_m2 = solar::shadow_m2(geometry, solar::idle_cos(geometry));
    solar::power_w(balance, shadow_m2, system.star_luminosity_w(), distance_m)
}

/// Whether Apply has to ask again: the draft's round would vent the field past collapse.
pub fn collapses(session: &Session, draft: &Draft) -> bool {
    let (Some(start), Some(fitting)) = (Start::of(session), session.ship.fitting()) else { return false };
    let Ok(plan) = start.solve(&draft.form) else { return false };
    let (field, base_j) = field_now(fitting);
    Heat::of(&plan, &field, base_j, &start.balance).collapses()
}

#[cfg(test)]
mod tests {
    use lc_world::form::{Kind, PartId};
    use lc_world::sky::AuthoredStars;

    use super::*;

    const B: Balance = Balance::DEFAULT;

    fn docked() -> Session {
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        s.set_coordinate_time_us(4_000_000_000_000);
        s.ship.fit(Some(Fitting::full(Form::starting(), B, 0.0)));
        s
    }

    fn part(form: &Form, kind: Kind) -> PartId {
        form.parts.iter().find(|p| p.kind == kind).unwrap().id
    }

    /// The data taken apart and a bay built on the storage.
    fn rebuilt() -> Draft {
        let mut d = Draft::new(Form::starting());
        d.apply(&d.remove(part(&d.form, Kind::Data)).unwrap(), &B).unwrap();
        let storage = part(&d.form, Kind::Storage);
        d.apply(&d.add(storage, Kind::Bay, crate::draft::PRIMITIVES[3], glam::DVec3::NEG_Y, &B).unwrap(), &B).unwrap();
        d
    }

    fn with(draft: Draft) -> UiState {
        let mut ui = UiState::default();
        ui.form.draft = Some(draft);
        ui
    }

    #[test]
    fn the_local_round_is_the_one_the_shard_would_plan() {
        let mut s = docked();
        let now = s.coordinate_time_s();
        s.ship.drain(3.0 * B.module_energy_j(), now);
        let draft = rebuilt();
        let local = Start::of(&s).unwrap().solve(&draft.form).expect("it plans");

        let mut shard = s.ship.clone();
        shard.begin_refit(draft.form.clone(), now).expect("the shard plans it");
        let planned = shard.fitting().unwrap().refit().unwrap();
        assert_eq!(&local, planned);

        let preview = Preview::of(&s, &with(draft), None).unwrap();
        assert_eq!(preview.budget, Ok(Budget::of(planned, &B)));
        assert_eq!(preview.duration_s, Some(planned.duration_s()));
    }

    /// While a round runs, the next begins from its target with what it will leave.
    #[test]
    fn a_running_round_is_the_start_of_the_next() {
        let mut s = docked();
        let now = s.coordinate_time_s();
        let target = rebuilt().form;
        s.ship.begin_refit(target.clone(), now).unwrap();
        let start = Start::of(&s).unwrap();
        assert_eq!(start.from, target);
        let plan = s.ship.fitting().unwrap().refit().unwrap();
        let left = plan.at(now + plan.duration_s()).stored_j;
        assert!((start.stored_j - left).abs() < 1e-6 * left, "{} {left}", start.stored_j);
    }

    #[test]
    fn a_short_round_has_no_budget_but_says_how_short() {
        let mut s = docked();
        let now = s.coordinate_time_s();
        let full = s.ship.fitting().unwrap().capacity_j_at(now);
        s.ship.drain(full, now);
        let mut d = Draft::new(Form::starting());
        let engine = *d.part(part(&d.form, Kind::Engine)).unwrap();
        d.apply(&d.resize(engine.id, 2.0 * engine.volume_m3).unwrap(), &B).unwrap();
        let preview = Preview::of(&s, &with(d), None).unwrap();
        let Err(Refusal::Energy { short_j }) = preview.budget else { panic!("{:?}", preview.budget) };
        assert!(short_j > 0.0);
        assert!(preview.heat.is_none() && preview.duration_s.is_none());
    }

    #[test]
    fn a_growth_past_what_storage_pays_is_not_allowed_and_shrinking_back_is() {
        let mut s = docked();
        let now = s.coordinate_time_s();
        let full = s.ship.fitting().unwrap().capacity_j_at(now);
        s.ship.drain(full - 0.2 * B.module_energy_j(), now);
        let start = Start::of(&s).unwrap();
        let d = Draft::new(Form::starting());
        let data = *d.part(part(&d.form, Kind::Data)).unwrap();
        assert!(start.allows(&d, &d.resize(data.id, data.volume_m3 * 1.1).unwrap()));
        let huge = d.resize(data.id, data.volume_m3 * 20.0).unwrap();
        assert!(!start.allows(&d, &huge));
        // Already too big, as a reset leaves a draft when storage has run down: smaller is fine.
        let mut big = d.clone();
        big.apply(&huge, &B).unwrap();
        let back = big.resize(data.id, data.volume_m3 * 10.0).unwrap();
        assert!(start.allows(&big, &Edit { before: vec![*big.part(data.id).unwrap()], ..back }));
    }

    /// 30's anchor: one 5 ME vent into an idle starting field takes it to about 3 850 K.
    #[test]
    fn the_idle_field_reads_as_thirty_says() {
        let fitting = Fitting::full(Form::starting(), B, 0.0);
        let (field, base_j) = field_now(&fitting);
        assert!((field.temperature_k(base_j) - B.field_idle_k).abs() < 1.0, "{}", field.temperature_k(base_j));
        let vented = field.temperature_k(base_j + 5.0 * B.module_energy_j());
        assert!((vented - 3850.0).abs() < 25.0, "{vented}");
    }

    #[test]
    fn heat_peaks_at_a_vent_and_decays_between() {
        let s = docked();
        let plan = Start::of(&s).unwrap().solve(&rebuilt().form).unwrap();
        let (field, base_j) = field_now(s.ship.fitting().unwrap());
        let heat = Heat::of(&plan, &field, base_j, &B);
        let vents: Vec<f64> = plan.steps().iter().map(|s| s.vented_j).collect();
        assert!(vents.iter().any(|v| *v > 0.0), "premise: full storage vents the return");
        let i = heat.step.expect("a vent raises it");
        assert!(plan.steps()[i].vented_j > 0.0, "it peaks as a vent lands");
        let largest = vents.iter().copied().fold(0.0, f64::max);
        let losses: f64 = plan.steps().iter().map(|s| s.gross_j * (1.0 - B.recovery)).sum();
        assert!(heat.peak_j >= base_j + largest && heat.peak_j <= base_j + vents.iter().sum::<f64>() + losses, "{heat:?}");
        assert!(!heat.collapses());

        let still = Start::of(&s).unwrap().solve(&Form::starting()).unwrap();
        assert_eq!(Heat::of(&still, &field, base_j, &B).step, None, "an empty round leaves it idle");
    }

    /// A full store shrunk to nothing spills everything it held.
    #[test]
    fn a_vent_past_the_fields_capacity_collapses_it() {
        let s = docked();
        let mut d = Draft::new(Form::starting());
        let storage = *d.part(part(&d.form, Kind::Storage)).unwrap();
        d.apply(&d.resize(storage.id, storage.volume_m3 * 0.3).unwrap(), &B).unwrap();
        assert!(collapses(&s, &d));
        let preview = Preview::of(&s, &with(d.clone()), None).unwrap();
        assert!(preview.collapses() && preview.heat.unwrap().peak_k > 4_500.0, "{:?}", preview.heat);
        assert!(!collapses(&s, &rebuilt()));
    }

    #[test]
    fn the_shape_is_the_drafts_measured_and_only_the_drafts() {
        let s = docked();
        let d = rebuilt();
        let measured = Measured::of(&d.form, &B).unwrap();
        let stale = Measured::of(&Form::starting(), &B).unwrap();
        let ui = with(d.clone());
        assert_eq!(Preview::of(&s, &ui, Some(&stale)).unwrap().shape, None);
        let shape = Preview::of(&s, &ui, Some(&measured)).unwrap().shape.unwrap();
        let grid = FormGrid::new(&d.form, &B).unwrap();
        assert_eq!(shape.envelope_m2, grid.envelope_area_m2());
        assert_eq!(shape.broadside_m2, grid.broadside_m2());
        let field = Field::of(grid.envelope_area_m2(), &B);
        assert_eq!(shape.rated_load_w, field.heat_max_j() / B.field_tau_s);
        assert!(shape.headroom_j > 0.0 && shape.headroom_j < field.heat_max_j());
    }

    /// The starting form's figures, which 29 and 30 state.
    #[test]
    fn the_starting_form_previews_as_the_docs_say() {
        let s = docked();
        let measured = Measured::of(&Form::starting(), &B).unwrap();
        let p = Preview::of(&s, &with(Draft::new(Form::starting())), Some(&measured)).unwrap();
        let shape = p.shape.unwrap();
        assert!((shape.envelope_m2 / lc_world::fitting::STARTING_ENVELOPE_M2 - 1.0).abs() < 1e-9);
        assert!((lc_world::attitude::flip_time_s(shape.slew_rad_s) - 64.0).abs() < 1.0, "{}", shape.slew_rad_s);
        assert!((shape.rated_load_w / 7.6e19 - 1.0).abs() < 0.01, "{}", shape.rated_load_w);
        assert!((p.accel_g - 5.0).abs() < 0.05, "five slots of aft engine pull 5 g full: {}", p.accel_g);
        assert_eq!(p.budget.unwrap().spent_j, 0.0);
    }
}
