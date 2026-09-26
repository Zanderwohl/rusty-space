//! What a ship is built from, what it weighs, and the energy it holds.
//!
//! See `lightcone/docs/19-ship-fitting.md` for the account and `29-ship-form.md` for the form it is
//! kept on. Energies are joules and masses kilograms throughout; a *module-energy* (ME) is the mass
//! of 19's slot at module density times `c²`, and it is the unit balance is argued in.
//!
//! Only a player's ship carries a [`Fitting`]. Everything else flies on
//! [`Kind::drive`](crate::craft::Kind::drive).

use std::sync::Mutex;

use crate::cost;
use crate::flight::{C_M_S, G0};
use crate::form::capacity::{aft_aperture_w, dry_mass_kg, Capacities};
use crate::form::grid::FormGrid;
use crate::form::presets::{SLOT_M3, STARTING};
use crate::form::{Form, FormError};
use crate::motion::ShipState;
use crate::refit::rounds::{Plan, Round};

pub const C2: f64 = C_M_S * C_M_S;

/// A 40 ft ISO container at its maximum gross mass, over its outside volume.
const MODULE_DENSITY_KG_M3: f64 = 30_480.0 / (12.192 * 2.438 * 2.591);
const DATA_MASS_FRACTION: f64 = 0.5;

/// 19's starting ship with nothing stored: every module, data at half, and a frame of 50 kg/m³
/// over all twenty of its slots, the five empty ones included. `hull_areal_density` is anchored so
/// the starting form weighs this, which keeps every rating derived from it where it was.
pub const STARTING_DRY_KG: f64 = (STARTING.storage
    + STARTING.drone
    + STARTING.engine
    + STARTING.living
    + DATA_MASS_FRACTION * STARTING.data)
    * MODULE_DENSITY_KG_M3
    + 20.0 * SLOT_M3 * 50.0;

/// Module-energies each of 19's storage modules held.
const STORAGE_ME_PER_SLOT: f64 = 5.0;

/// Every tunable number. A server states its own with every account, so a client assumes none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Balance {
    /// ε in `m' = m exp(−Δη/ε)`. One is a perfect photon rocket; above one is unphysical, and
    /// allowed on purpose.
    pub drive_efficiency: f64,
    /// Fraction of a part's mass-energy that dismantling it returns.
    pub recovery: f64,
    pub module_density_kg_m3: f64,
    /// η: of what is converted, the fraction stored.
    pub conversion_efficiency: f64,
    /// An unphysical multiplier on collection, because a module-energy is `mc²` and real
    /// starlight on a real hull would take billions of years to pay for one. See
    /// `lightcone/docs/20-solar-power.md`.
    pub solar_gain: f64,
    /// Of module density.
    pub data_mass_fraction: f64,
    /// How many times longer data takes to build or take apart than its energy says.
    pub data_work_factor: f64,
    /// Module-energies of capacity per m³ of storage.
    pub storage_density: f64,
    /// W/m³ of building power.
    pub drone_density_w: f64,
    /// W/m³ of aperture power; thrust is this over `c`.
    pub engine_density_w: f64,
    /// W/m³ drained, continuously.
    pub living_density_w: f64,
    /// Bytes per m³. See [`DATA_ANCHOR_S`].
    pub data_density_b: f64,
    /// Of module density. Every kind without its own fraction is one.
    pub bay_mass_fraction: f64,
    pub spar_mass_fraction: f64,
    pub min_part_m3: f64,
    pub min_drone_m3: f64,
    /// m.
    pub spar_gap: f64,
    /// Of the parent's smallest dimension.
    pub spar_thickness: f64,
    pub move_work_factor: f64,
    /// kg/m² of part surface.
    pub hull_areal_density: f64,
    /// Of the cube root of hull volume.
    pub envelope_margin: f64,
    pub engine_clear_half_angle_rad: f64,
    pub field_idle_k: f64,
    /// J/m² of envelope at collapse.
    pub field_capacity: f64,
    pub field_tau_s: f64,
    pub clear_absorptivity: f64,
    pub field_switch_s: f64,
    /// Of `Q_max`.
    pub auto_clear_above: f64,
    /// Of `Q_max`.
    pub auto_black_below: f64,
    /// Of storage capacity.
    pub auto_refill_below: f64,
    /// Of the field's energy.
    pub collapse_spike_fraction: f64,
    pub collapse_spike_k: f64,
    pub collapse_afterglow_s: f64,
    pub drive_spread_rad: f64,
    pub rcs_accel_g: f64,
    pub rcs_spread_rad: f64,
    /// Of the cooking flux.
    pub courtesy_fraction: f64,
}

/// What [`Balance::data_density_b`] is anchored to: 19's slot of data holds a year of a
/// thirty-minute stare in every band. Only raw logs take room — see
/// `lightcone/docs/24-standing-instruments.md`.
pub const DATA_ANCHOR_S: f64 = crate::flight::JULIAN_YEAR_S;
const DATA_ANCHOR_CADENCE_S: f64 = 1800.0;

/// Bytes of raw log a craft holds with no data at all: a couple of months of one star.
pub const ONBOARD_DATA_BYTES: f64 = 1_048_576.0;

/// The distance, AU from a Sun-like star, at which the starting ship broadside fills from empty in
/// [`SOLAR_ANCHOR_S`], net of its living drain. What [`Balance::solar_gain`] is derived from.
pub const SOLAR_ANCHOR_AU: f64 = 0.1;
pub const SOLAR_ANCHOR_S: f64 = crate::flight::JULIAN_YEAR_S;

/// The distance, AU from a Sun-like star, at which the starting ship full and broadside is exactly
/// at its rated load. H2 anchors the field's time constant on it, and it fixes the cooking flux.
pub const RATED_LOAD_AU: f64 = 0.05;

/// Module-energies the starting ship's field holds from empty to collapse. What
/// [`Balance::field_capacity`] is anchored to.
pub const FIELD_ANCHOR_ME: f64 = 10.0;

/// The starting form's envelope, m². Solved in `field`'s anchor tests, since the grid is not
/// `const`, and pinned there.
pub const STARTING_ENVELOPE_M2: f64 = 339248.9593564414;

impl Balance {
    pub const DEFAULT: Self = {
        let module_density_kg_m3 = MODULE_DENSITY_KG_M3;
        let me_j = SLOT_M3 * module_density_kg_m3 * C2;
        let full_kg = STARTING_DRY_KG + STARTING.storage / SLOT_M3 * STORAGE_ME_PER_SLOT * me_j / C2;
        let week_s = 7.0 * 86_400.0;
        let century_s = 100.0 * crate::flight::JULIAN_YEAR_S;
        let living_density_w = module_density_kg_m3 * C2 / century_s;
        let storage_density = STORAGE_ME_PER_SLOT / SLOT_M3;
        let conversion_efficiency = 0.7;
        let data_per_slot_b = DATA_ANCHOR_S / DATA_ANCHOR_CADENCE_S
            * em_spectra::Band::ALL.len() as f64
            * crate::knowledge::SAMPLE_BYTES;
        let day_s = 86_400.0;
        let degree = std::f64::consts::PI / 180.0;
        // Collection that fills the starting storage in the anchor time and pays the drain too,
        // over what real starlight on the broadside of a 500 m ovoid would give.
        let wanted_w = STARTING.storage * storage_density * me_j / SOLAR_ANCHOR_S + STARTING.living * living_density_w;
        let broadside_m2 = std::f64::consts::PI
            * 250.0
            * (250.0 * crate::craft::BEAM_PER_LENGTH);
        let flux = crate::solar::SOLAR_CONSTANT_W_M2 / (SOLAR_ANCHOR_AU * SOLAR_ANCHOR_AU);
        let rated_load_gain = (SOLAR_ANCHOR_AU / RATED_LOAD_AU) * (SOLAR_ANCHOR_AU / RATED_LOAD_AU);
        Self {
            drive_efficiency: 1.0,
            recovery: 0.95,
            module_density_kg_m3,
            conversion_efficiency,
            solar_gain: wanted_w / (conversion_efficiency * flux * broadside_m2),
            data_mass_fraction: DATA_MASS_FRACTION,
            data_work_factor: 3.0,
            storage_density,
            // A slot of drones builds a slot of anything else in a week.
            drone_density_w: module_density_kg_m3 * C2 / week_s,
            // A g a slot of the starting ship full, so its five slots of aft engine pull 5 g.
            engine_density_w: G0 * full_kg * C_M_S / SLOT_M3,
            // A slot of living space drains 1 ME a century.
            living_density_w,
            data_density_b: data_per_slot_b / SLOT_M3,
            bay_mass_fraction: 0.1,
            spar_mass_fraction: 0.05,
            min_part_m3: 1_000.0,
            min_drone_m3: 10_000.0,
            spar_gap: 0.5,
            spar_thickness: 0.02,
            move_work_factor: 0.25,
            // `Form::starting()` weighs [`STARTING_DRY_KG`]. Solved there, since the areas are not
            // `const`, and pinned by `form::presets`' anchor test.
            hull_areal_density: 1214.530767193309,
            envelope_margin: 0.05,
            engine_clear_half_angle_rad: 15.0 * degree,
            field_idle_k: 400.0,
            field_capacity: FIELD_ANCHOR_ME * me_j / STARTING_ENVELOPE_M2,
            // The starting ship full and broadside at `RATED_LOAD_AU` exactly at rated load. Full
            // storage turns all it absorbs into heat, and the gain is anchored on the same
            // broadside the starlight falls on, so the broadside cancels.
            field_tau_s: FIELD_ANCHOR_ME * me_j * conversion_efficiency / (wanted_w * rated_load_gain),
            clear_absorptivity: 0.3,
            field_switch_s: day_s,
            auto_clear_above: 0.5,
            auto_black_below: 0.3,
            auto_refill_below: 0.95,
            collapse_spike_fraction: 0.9,
            collapse_spike_k: 1.0e7,
            collapse_afterglow_s: 30.0 * day_s,
            drive_spread_rad: 5.0 * degree,
            rcs_accel_g: 0.01,
            rcs_spread_rad: 60.0 * degree,
            courtesy_fraction: 0.01,
        }
    };

    /// One module-energy, joules: 19's slot at module density, times `c²`. The unit, not a price:
    /// what a part costs is its own mass-energy.
    pub fn module_energy_j(&self) -> f64 {
        SLOT_M3 * self.module_density_kg_m3 * C2
    }

    /// `q_idle`, J/m² of envelope: the starting ship's living drain alone holds its field at
    /// `field_idle_k`, so its idle heat is `P τ` over [`STARTING_ENVELOPE_M2`].
    pub fn field_idle_j_m2(&self) -> f64 {
        let drain_w = STARTING.living * self.living_density_w;
        drain_w * self.field_tau_s / STARTING_ENVELOPE_M2
    }
}

impl Default for Balance {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What a craft reads off its form, worked out when the form changes rather than when it is read:
/// the grid behind `extent_m` and `gyration_m` takes tens of milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    pub capacities: Capacities,
    /// Contents and structure, nothing stored.
    pub dry_kg: f64,
    /// Newtons from the engines that fire aft. One firing fore pushes the other way, and is a
    /// weapon (`lightcone/docs/31-directed-energy.md`), so it is not in the rating.
    pub thrust_n: f64,
    /// The envelope's longest dimension, m: a fitted craft's `length_m`.
    pub extent_m: f64,
    /// Radius of gyration about the axis across the nose the form turns slowest about, m.
    pub gyration_m: f64,
}

impl Hull {
    /// `Err` only for a form whose envelope never closes, which [`rules::check`] refuses, so never
    /// for a ship's.
    ///
    /// [`rules::check`]: crate::form::rules::check
    pub fn of(form: &Form, balance: &Balance) -> Result<Hull, FormError> {
        let (extent_m, gyration_m) = measured(form, balance)?;
        let mut hull = Self::cheap(form, balance, 0.0, 0.0, 0.0);
        hull.thrust_n = aft_aperture_w(form, balance).unwrap_or(0.0) / C_M_S;
        Ok(Hull { extent_m, gyration_m, ..hull })
    }

    /// Everything but the grid, which keeps `was`'s: a form partway through a round, which may
    /// not place, and whose grid is not worth building for a step that lasts minutes.
    fn partway(form: &Form, balance: &Balance, was: &Hull) -> Hull {
        let mut hull = Self::cheap(form, balance, was.thrust_n, was.extent_m, was.gyration_m);
        if let Some(aperture_w) = aft_aperture_w(form, balance) {
            hull.thrust_n = aperture_w / C_M_S;
        }
        hull
    }

    fn cheap(form: &Form, balance: &Balance, thrust_n: f64, extent_m: f64, gyration_m: f64) -> Hull {
        Hull { capacities: Capacities::of(form, balance), dry_kg: dry_mass_kg(form, balance), thrust_n, extent_m, gyration_m }
    }

    /// Settled at a step's end, when the form it was partway through becomes the ship's.
    fn after_step(form: &Form, balance: &Balance, was: &Hull) -> Hull {
        Self::of(form, balance).unwrap_or_else(|_| Self::partway(form, balance, was))
    }
}

/// The grid's extent and transverse radius of gyration, built once per form and balance for the
/// process: every ship today is the starting form, so a shard or a test run builds one.
fn measured(form: &Form, balance: &Balance) -> Result<(f64, f64), FormError> {
    type Built = (Form, Balance, Result<(f64, f64), FormError>);
    const KEPT: usize = 16;
    static BUILT: Mutex<Vec<Built>> = Mutex::new(Vec::new());
    // Held across the build, so threads asking for one form wait for it rather than each
    // building it. That also blocks callers asking about any other form, which is harmless while
    // every ship is the starting form; once S1 lets forms differ, build outside the lock with a
    // once per key.
    let mut built = BUILT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((_, _, answer)) = built.iter().find(|(f, b, _)| f == form && b == balance) {
        return *answer;
    }
    let answer = FormGrid::new(form, balance).map(|grid| {
        let i = grid.inertia().per_kg;
        let (a, c, b) = (i.y_axis.y, i.z_axis.z, i.z_axis.y);
        // The larger eigenvalue of the tensor's block across the nose.
        let across_m2 = 0.5 * (a + c) + (0.25 * (a - c) * (a - c) + b * b).sqrt();
        (grid.extent_m(), across_m2.sqrt())
    });
    if built.len() == KEPT {
        built.drain(..1);
    }
    built.push((form.clone(), *balance, answer));
    answer
}

/// A ship's form and the energy in it, as a closed form in coordinate time.
///
/// The account is **settled** at an instant — stored energy, the form and the rapidity the motive
/// had flown by then — and read forward from it. Anything that changes a term of the closed form
/// settles first: a new motive, a refit step, a grant.
#[derive(Clone, Debug, PartialEq)]
pub struct Fitting {
    form: Form,
    hull: Hull,
    /// Not saved: a server stamps its own on every craft it loads.
    balance: Balance,
    stored_j: f64,
    since_s: f64,
    /// The motive's lit rapidity at `since_s`. See [`cost::lit_rapidity`].
    rapidity_since: f64,
    /// Energy the motive in force still has to spend as of `since_s`, joules. Reserved, so the
    /// drain cannot eat it.
    committed_j: f64,
    /// What starlight adds from `since_s`, watts, until the craft starts the next segment. See
    /// [`crate::solar`].
    solar_w: f64,
    /// A round's vents are the field's, and are dropped here until the field is kept.
    refit: Option<Plan>,
}

/// A fitting as it is saved and sent: the settled terms, and the refit's recipe.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub form: Form,
    pub stored_j: f64,
    pub since_s: f64,
    pub rapidity_since: f64,
    pub committed_j: f64,
    pub solar_w: f64,
    pub refit: Option<Round>,
}

impl Fitting {
    /// A ship of this form with its storage full. `form` must be one [`Hull::of`] can measure, as
    /// every form a ship is given is.
    pub fn full(form: Form, balance: Balance, now_s: f64) -> Self {
        let hull = Hull::of(&form, &balance)
            .expect("a ship's form closes: it is the starting form scaled, until S1 checks targets with rules::check");
        Self {
            form,
            hull,
            balance,
            stored_j: hull.capacities.storage_j,
            since_s: now_s,
            rapidity_since: 0.0,
            committed_j: 0.0,
            solar_w: 0.0,
            refit: None,
        }
    }

    pub fn account(&self) -> Account {
        Account {
            form: self.form.clone(),
            stored_j: self.stored_j,
            since_s: self.since_s,
            rapidity_since: self.rapidity_since,
            committed_j: self.committed_j,
            solar_w: self.solar_w,
            refit: self.refit.as_ref().map(|plan| plan.round().clone()),
        }
    }

    /// Put an account back. A refit whose recipe no longer plans is dropped, which leaves the
    /// ship as the account says it was when the refit began.
    pub fn from_account(account: &Account, balance: Balance) -> Self {
        let refit = account.refit.as_ref().and_then(|round| round.solve(&balance).ok());
        let full = Self::full(account.form.clone(), balance, account.since_s);
        Self {
            refit,
            // What the store holds may not be more than it can: a form re-measured under another
            // balance, or a loadout laid out other than it was, can hold less than was saved.
            stored_j: account.stored_j.min(full.stored_j),
            since_s: account.since_s,
            rapidity_since: account.rapidity_since,
            committed_j: account.committed_j,
            solar_w: account.solar_w,
            ..full
        }
    }

    pub fn balance(&self) -> &Balance {
        &self.balance
    }

    /// Read everything under another balance from here on. The hull is measured again, since
    /// densities and the envelope's margin are the balance's.
    pub fn set_balance(&mut self, balance: Balance) {
        self.balance = balance;
        self.hull = Hull::after_step(&self.form, &balance, &self.hull);
    }

    /// The form as of the last settlement, counting refit steps finished by then.
    pub fn form(&self) -> &Form {
        &self.form
    }

    /// What the settled form holds, weighs and measures.
    pub fn hull(&self) -> &Hull {
        &self.hull
    }

    pub fn refit(&self) -> Option<&Plan> {
        self.refit.as_ref()
    }

    pub fn since_s(&self) -> f64 {
        self.since_s
    }

    /// What starlight is adding in the segment in force, watts.
    pub fn solar_w(&self) -> f64 {
        self.solar_w
    }

    /// Start a segment at this power. Settle first, at the segment's start.
    pub fn set_solar_w(&mut self, watts: f64) {
        self.solar_w = watts.max(0.0);
    }

    /// The hull at a coordinate time, counting refit steps finished by then. Its extent and
    /// gyration are the settled form's until a settlement measures the new one.
    pub fn hull_at(&self, now_s: f64) -> Hull {
        match &self.refit {
            Some(plan) => Hull::partway(&plan.at(now_s).form, &self.balance, &self.hull),
            None => self.hull,
        }
    }

    pub fn capacities_at(&self, now_s: f64) -> Capacities {
        match &self.refit {
            Some(plan) => Capacities::of(&plan.at(now_s).form, &self.balance),
            None => self.hull.capacities,
        }
    }

    /// Energy committed to the motive and not yet spent, at a coordinate time.
    pub fn committed_j_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        (self.committed_j - self.burn_spent_j(motion, now_s)).max(0.0)
    }

    fn burn_spent_j(&self, motion: &ShipState, now_s: f64) -> f64 {
        let flown = cost::lit_rapidity(motion, now_s) - self.rapidity_since;
        if flown <= 0.0 {
            return 0.0;
        }
        cost::energy_j(self.settled_mass_kg(), flown, self.balance.drive_efficiency)
    }

    fn settled_mass_kg(&self) -> f64 {
        self.hull.dry_kg + self.stored_j / C2
    }

    /// What starlight has added, less what living space has drained, by `now_s`.
    ///
    /// Filling stops at capacity, and the excess is lost. Draining stops once only committed
    /// energy is left, and running out costs nothing else. The clamp is on the segment's total
    /// rather than its path, which is exact until a limit is reached and off by at most one
    /// segment's income when one is.
    fn income_j(&self, now_s: f64) -> f64 {
        let elapsed = (now_s - self.since_s).max(0.0);
        let net_w = self.solar_w - self.hull.capacities.drain_w;
        if net_w >= 0.0 {
            let room = (self.hull.capacities.storage_j - self.stored_j).max(0.0);
            (net_w * elapsed).min(room)
        } else {
            let free = (self.stored_j - self.committed_j).max(0.0);
            -(-net_w * elapsed).min(free)
        }
    }

    /// Stored energy at a coordinate time, joules.
    pub fn stored_j_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        let moved = match &self.refit {
            Some(plan) => plan.at(self.since_s).stored_j - plan.at(now_s).stored_j,
            None => 0.0,
        };
        (self.stored_j + self.income_j(now_s) - self.burn_spent_j(motion, now_s) - moved).max(0.0)
    }

    /// What is stored and not committed, joules.
    pub fn free_j_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        (self.stored_j_at(motion, now_s) - self.committed_j_at(motion, now_s)).max(0.0)
    }

    pub fn capacity_j_at(&self, now_s: f64) -> f64 {
        self.capacities_at(now_s).storage_j
    }

    /// The whole ship, stored energy included, kilograms.
    pub fn mass_kg_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        let (dry, in_hand) = match &self.refit {
            Some(plan) => {
                let progress = plan.at(now_s);
                (dry_mass_kg(&progress.form, &self.balance), progress.in_hand_kg)
            }
            None => (self.hull.dry_kg, 0.0),
        };
        dry + in_hand + self.stored_j_at(motion, now_s) / C2
    }

    /// The acceleration this ship's aft engines give it now, in g.
    pub fn rated_g_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        self.hull_at(now_s).thrust_n / self.mass_kg_at(motion, now_s) / G0
    }

    /// Fold everything up to `now_s` into the settled terms, reading the burn off `motion` —
    /// which must be the motive that was in force up to now.
    pub fn settle(&mut self, motion: &ShipState, now_s: f64) {
        if now_s <= self.since_s {
            return;
        }
        let stored = self.stored_j_at(motion, now_s);
        self.committed_j = self.committed_j_at(motion, now_s);
        if let Some(plan) = &self.refit {
            let progress = plan.at(now_s);
            if progress.form != self.form {
                self.hull = Hull::after_step(&progress.form, &self.balance, &self.hull);
                self.form = progress.form;
            }
            if plan.is_done(now_s) {
                self.refit = None;
            }
        }
        self.stored_j = stored;
        self.rapidity_since = cost::lit_rapidity(motion, now_s);
        self.since_s = now_s;
    }

    /// Take up a new motive, committing what its plan will spend. Settle with the old one first.
    ///
    /// Returns the commitment. The caller decides whether the ship could afford it: this is a
    /// fold, and it records whatever it is handed.
    pub fn commit(&mut self, motion: &ShipState, now_s: f64) -> f64 {
        self.rapidity_since = cost::lit_rapidity(motion, now_s);
        let remaining = cost::planned_rapidity(motion) - self.rapidity_since;
        self.committed_j = if remaining > 0.0 {
            cost::energy_j(self.settled_mass_kg(), remaining, self.balance.drive_efficiency)
        } else {
            0.0
        };
        self.committed_j
    }

    /// Spend a change of rapidity at once, as a burn does. Settle first.
    pub fn spend(&mut self, rapidity: f64) {
        let cost = cost::energy_j(self.settled_mass_kg(), rapidity, self.balance.drive_efficiency);
        self.stored_j = (self.stored_j - cost).max(0.0);
    }

    /// Add energy, as far as there is room for it. Settle first.
    pub fn grant(&mut self, joules: f64) {
        let capacity = self.hull.capacities.storage_j;
        self.stored_j = (self.stored_j + joules.max(0.0)).min(capacity.max(self.stored_j));
    }

    /// Settle first.
    pub fn drain(&mut self, joules: f64) {
        self.stored_j = (self.stored_j - joules.max(0.0)).max(0.0);
    }

    /// Begin a round. Settle first; the caller has already solved it from this form and store.
    pub fn begin_refit(&mut self, plan: Plan) {
        self.refit = Some(plan);
    }

    /// Only the time is skipped: the remaining steps' energy is still taken. Settle first.
    pub fn finish_refit(&mut self) -> bool {
        let Some(plan) = self.refit.take() else { return false };
        let end = plan.at(plan.round().start_s + plan.duration_s());
        let remaining_j = plan.at(self.since_s).stored_j - end.stored_j;
        self.hull = Hull::after_step(&end.form, &self.balance, &self.hull);
        self.form = end.form;
        self.stored_j = (self.stored_j - remaining_j).clamp(0.0, self.hull.capacities.storage_j.max(0.0));
        true
    }

    /// For nothing, dropping any refit under way. Storage is cut to the new capacity. Settle
    /// first. Refused for a form the grid cannot measure.
    pub fn refit_at_once(&mut self, form: Form) -> Result<(), FormError> {
        self.hull = Hull::of(&form, &self.balance)?;
        self.refit = None;
        self.form = form;
        self.stored_j = self.stored_j.min(self.hull.capacities.storage_j.max(0.0));
        Ok(())
    }

    /// Stop a refit where it is. Settle first. The step in progress is undone, or finished when
    /// storage cannot pay to put it back; see [`Plan::cancel`].
    pub fn cancel_refit(&mut self, now_s: f64) {
        let Some(plan) = self.refit.take() else { return };
        let canceled = plan.cancel(now_s, self.stored_j);
        self.hull = Hull::after_step(&canceled.form, &self.balance, &self.hull);
        self.form = canceled.form;
        self.stored_j = canceled.stored_j;
    }
}

// S1 deletes everything from here to the tests: the wire and saves still carry 19's loadout, so a
// ship crossing either is 29's starting form scaled to its counts, and its counts are read back
// off its capacities.

/// Each of the starting form's kinds in 19's slots.
const STARTING_COUNTS: lc_proto::Loadout = lc_proto::Loadout { storage: 6, drones: 2, living: 1, engines: 5, slots: 20, data: 1 };

/// The starting form, each kind scaled by its count over the starting ship's. A count of zero
/// leaves that part out, and what hung from it hangs from the storage or the Mind. One that will
/// not lay out is said so and comes back as the starting form, whose stored energy
/// [`Fitting::from_account`] clamps to its store.
fn form_of(loadout: lc_proto::Loadout, balance: &Balance) -> Form {
    let start = Form::starting();
    if (lc_proto::Loadout { slots: STARTING_COUNTS.slots, ..loadout }) == STARTING_COUNTS {
        return start;
    }
    use crate::form::Kind;
    let count = |kind: Kind| match kind {
        Kind::Storage => loadout.storage,
        Kind::Drone => loadout.drones,
        Kind::Living => loadout.living,
        Kind::Engine => loadout.engines,
        Kind::Data => loadout.data,
        Kind::Mind | Kind::Bay | Kind::Spar(_) => 0,
    };
    // A layout keeps a ship's totals, and each kind is one part here, so these are the totals.
    let totals = Form {
        parts: start.parts.iter().map(|p| crate::form::Part { volume_m3: f64::from(count(p.kind)) * SLOT_M3, ..*p }).collect(),
    };
    match start.as_layout(&totals, balance.min_part_m3) {
        Ok(layout) => layout.form,
        Err(why) => {
            eprintln!("loadout {loadout:?} does not lay out ({why}), so the ship comes back as the starting form");
            start
        }
    }
}

fn loadout_of(capacities: &Capacities, balance: &Balance) -> lc_proto::Loadout {
    let me = balance.module_energy_j();
    let slots = |total: f64, per_slot: f64| (total / (per_slot * SLOT_M3)).round() as u32;
    let aperture_per_m3 = balance.engine_density_w;
    let mut loadout = lc_proto::Loadout {
        storage: slots(capacities.storage_j, balance.storage_density * me),
        drones: slots(capacities.building_w, balance.drone_density_w),
        living: slots(capacities.drain_w, balance.living_density_w),
        engines: slots(capacities.aperture_w, aperture_per_m3),
        data: slots(capacities.data_b - ONBOARD_DATA_BYTES, balance.data_density_b),
        slots: 0,
    };
    let modules = loadout.storage + loadout.drones + loadout.living + loadout.engines + loadout.data;
    loadout.slots = modules.max(STARTING_COUNTS.slots);
    loadout
}

impl From<lc_proto::Balance> for Balance {
    fn from(b: lc_proto::Balance) -> Self {
        Self {
            drive_efficiency: b.drive_efficiency,
            recovery: b.recovery,
            module_density_kg_m3: b.module_density_kg_m3,
            conversion_efficiency: b.conversion_efficiency,
            solar_gain: b.solar_gain,
            data_mass_fraction: b.data_mass_fraction,
            data_work_factor: b.data_work_factor,
            storage_density: b.storage_density,
            drone_density_w: b.drone_density_w,
            engine_density_w: b.engine_density_w,
            living_density_w: b.living_density_w,
            data_density_b: b.data_density_b,
            bay_mass_fraction: b.bay_mass_fraction,
            spar_mass_fraction: b.spar_mass_fraction,
            min_part_m3: b.min_part_m3,
            min_drone_m3: b.min_drone_m3,
            spar_gap: b.spar_gap,
            spar_thickness: b.spar_thickness,
            move_work_factor: b.move_work_factor,
            hull_areal_density: b.hull_areal_density,
            envelope_margin: b.envelope_margin,
            engine_clear_half_angle_rad: b.engine_clear_half_angle_rad,
            field_idle_k: b.field_idle_k,
            field_capacity: b.field_capacity,
            field_tau_s: b.field_tau_s,
            clear_absorptivity: b.clear_absorptivity,
            field_switch_s: b.field_switch_s,
            auto_clear_above: b.auto_clear_above,
            auto_black_below: b.auto_black_below,
            auto_refill_below: b.auto_refill_below,
            collapse_spike_fraction: b.collapse_spike_fraction,
            collapse_spike_k: b.collapse_spike_k,
            collapse_afterglow_s: b.collapse_afterglow_s,
            drive_spread_rad: b.drive_spread_rad,
            rcs_accel_g: b.rcs_accel_g,
            rcs_spread_rad: b.rcs_spread_rad,
            courtesy_fraction: b.courtesy_fraction,
        }
    }
}

impl From<Balance> for lc_proto::Balance {
    /// The per-module fields are the densities over 19's slot. The frame's density has no
    /// counterpart, since structure goes by area.
    fn from(b: Balance) -> Self {
        Self {
            drive_efficiency: b.drive_efficiency,
            recovery: b.recovery,
            storage_per_module: b.storage_density * SLOT_M3,
            engine_thrust_n: b.engine_density_w * SLOT_M3 / C_M_S,
            drone_power_w: b.drone_density_w * SLOT_M3,
            living_drain_w: b.living_density_w * SLOT_M3,
            hull_density_kg_m3: 0.0,
            slot_volume_m3: SLOT_M3,
            module_density_kg_m3: b.module_density_kg_m3,
            conversion_efficiency: b.conversion_efficiency,
            solar_gain: b.solar_gain,
            data_per_module: b.data_density_b * SLOT_M3,
            data_mass_fraction: b.data_mass_fraction,
            data_work_factor: b.data_work_factor,
            storage_density: b.storage_density,
            drone_density_w: b.drone_density_w,
            engine_density_w: b.engine_density_w,
            living_density_w: b.living_density_w,
            data_density_b: b.data_density_b,
            bay_mass_fraction: b.bay_mass_fraction,
            spar_mass_fraction: b.spar_mass_fraction,
            min_part_m3: b.min_part_m3,
            min_drone_m3: b.min_drone_m3,
            spar_gap: b.spar_gap,
            spar_thickness: b.spar_thickness,
            move_work_factor: b.move_work_factor,
            hull_areal_density: b.hull_areal_density,
            envelope_margin: b.envelope_margin,
            engine_clear_half_angle_rad: b.engine_clear_half_angle_rad,
            field_idle_k: b.field_idle_k,
            field_capacity: b.field_capacity,
            field_tau_s: b.field_tau_s,
            clear_absorptivity: b.clear_absorptivity,
            field_switch_s: b.field_switch_s,
            auto_clear_above: b.auto_clear_above,
            auto_black_below: b.auto_black_below,
            auto_refill_below: b.auto_refill_below,
            collapse_spike_fraction: b.collapse_spike_fraction,
            collapse_spike_k: b.collapse_spike_k,
            collapse_afterglow_s: b.collapse_afterglow_s,
            drive_spread_rad: b.drive_spread_rad,
            rcs_accel_g: b.rcs_accel_g,
            rcs_spread_rad: b.rcs_spread_rad,
            courtesy_fraction: b.courtesy_fraction,
        }
    }
}

impl From<&Fitting> for lc_proto::Fitting {
    /// A round cannot be written as two loadouts, so a refit under way is not sent. None can be
    /// begun until S1.
    fn from(f: &Fitting) -> Self {
        let a = f.account();
        Self {
            balance: f.balance.into(),
            loadout: loadout_of(&f.hull.capacities, &f.balance),
            stored_j: a.stored_j,
            since_s: a.since_s,
            rapidity_since: a.rapidity_since,
            committed_j: a.committed_j,
            solar_w: a.solar_w,
            refit: None,
        }
    }
}

impl From<&lc_proto::Fitting> for Fitting {
    /// A loadout refit is dropped, as one whose recipe no longer plans always was, which leaves
    /// the ship as it was when the refit began.
    fn from(f: &lc_proto::Fitting) -> Self {
        let balance = Balance::from(f.balance);
        let account = Account {
            form: form_of(f.loadout, &balance),
            stored_j: f.stored_j,
            since_s: f.since_s,
            rapidity_since: f.rapidity_since,
            committed_j: f.committed_j,
            solar_w: f.solar_w,
            refit: None,
        };
        Fitting::from_account(&account, balance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::craft::{Craft, CraftId, Kind};
    use crate::form::PartId;
    use glam::DVec3;

    fn full() -> Fitting {
        Fitting::full(Form::starting(), Balance::DEFAULT, 0.0)
    }

    /// The first guesses 29-ship-form.md prints, to the digits it prints them.
    #[test]
    fn densities_match_the_design() {
        let b = Balance::DEFAULT;
        assert_eq!(format!("{:.2e}", b.storage_density), "1.27e-5");
        assert_eq!(format!("{:.2e}", b.drone_density_w), "5.88e13");
        assert_eq!(format!("{:.2e}", b.engine_density_w), "5.47e13");
        assert_eq!(format!("{:.2e}", b.living_density_w), "1.13e10");
        assert_eq!(format!("{:.1}", b.data_density_b), "7.5");
    }

    #[test]
    fn every_balance_field_crosses_the_wire_to_itself() {
        // Distinct in every field, so two fields swapped in either `From` cannot agree by luck.
        let b = Balance {
            drive_efficiency: 1.0,
            recovery: 2.0,
            module_density_kg_m3: 9.0,
            conversion_efficiency: 10.0,
            solar_gain: 11.0,
            data_mass_fraction: 13.0,
            data_work_factor: 14.0,
            storage_density: 15.0,
            drone_density_w: 16.0,
            engine_density_w: 17.0,
            living_density_w: 18.0,
            data_density_b: 19.0,
            bay_mass_fraction: 20.0,
            spar_mass_fraction: 21.0,
            min_part_m3: 22.0,
            min_drone_m3: 23.0,
            spar_gap: 24.0,
            spar_thickness: 25.0,
            move_work_factor: 26.0,
            hull_areal_density: 27.0,
            envelope_margin: 28.0,
            engine_clear_half_angle_rad: 29.0,
            field_idle_k: 30.0,
            field_capacity: 31.0,
            field_tau_s: 32.0,
            clear_absorptivity: 33.0,
            field_switch_s: 34.0,
            auto_clear_above: 35.0,
            auto_black_below: 36.0,
            auto_refill_below: 37.0,
            collapse_spike_fraction: 38.0,
            collapse_spike_k: 39.0,
            collapse_afterglow_s: 40.0,
            drive_spread_rad: 41.0,
            rcs_accel_g: 42.0,
            rcs_spread_rad: 43.0,
            courtesy_fraction: 44.0,
        };
        let wire: lc_proto::Balance = b.into();
        assert_eq!(wire.conversion_efficiency, b.conversion_efficiency);
        assert_eq!(Balance::from(wire), b);
    }

    /// The wire's per-module fields, which the balance no longer has, are 19's defaults again.
    #[test]
    fn the_wires_per_module_fields_are_a_slot_of_each_density() {
        let wire = lc_proto::Balance::from(Balance::DEFAULT);
        let me = Balance::DEFAULT.module_energy_j();
        assert!((wire.storage_per_module - 5.0).abs() < 1e-12);
        assert_eq!(format!("{:.2e}", wire.engine_thrust_n), "7.17e10");
        assert_eq!(format!("{:.2e}", wire.drone_power_w), "2.31e19");
        assert_eq!(format!("{:.2e}", wire.living_drain_w), "4.43e15");
        assert_eq!(format!("{:.2e}", wire.data_per_module), "2.95e6");
        assert!((wire.drone_power_w * 7.0 * 86_400.0 / me - 1.0).abs() < 1e-12, "a slot of drones builds an ME a week");
    }

    #[test]
    fn nineteens_slot_is_a_twentieth_of_the_five_hundred_meter_ovoid() {
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        craft.length_m = 500.0;
        assert!((craft.volume_m3() / SLOT_M3 - 20.0).abs() < 1.0e-9);
    }

    #[test]
    fn a_module_is_a_container_stack_the_size_of_a_slot() {
        let b = Balance::DEFAULT;
        assert!((SLOT_M3 - 392_699.08).abs() < 0.01);
        assert!((b.module_density_kg_m3 - 395.77).abs() < 0.01);
        assert!((b.module_energy_j() / 1.3968e25 - 1.0).abs() < 1.0e-4);
    }

    /// 19's figures, unchanged: the starting form weighs its dry ship by the areal density's
    /// anchor, and its one aft bell is its five engines. Empty is 13.8 g as it was.
    #[test]
    fn the_starting_ship_full_pulls_five_g_and_more_empty() {
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = full();
        let g = fitting.rated_g_at(&motion, 0.0);
        assert!((g / 5.0 - 1.0).abs() < 1.0e-9, "{g}");
        fitting.drain(f64::INFINITY);
        let empty = fitting.rated_g_at(&motion, 0.0);
        assert!((empty - 13.81).abs() < 0.01, "{empty}");
    }

    #[test]
    fn stored_energy_weighs_what_it_holds() {
        let motion = ShipState::at(DVec3::ZERO);
        let fitting = full();
        let mass = fitting.mass_kg_at(&motion, 0.0);
        let expected = fitting.hull().dry_kg + 30.0 * Balance::DEFAULT.module_energy_j() / C2;
        assert!((mass / expected - 1.0).abs() < 1.0e-12, "{mass} vs {expected}");
        assert!((fitting.hull().dry_kg / STARTING_DRY_KG - 1.0).abs() < 1.0e-9);
    }

    #[test]
    fn living_space_drains_until_nothing_free_is_left() {
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = full();
        let year = crate::flight::JULIAN_YEAR_S;
        let lost = fitting.stored_j_at(&motion, 0.0) - fitting.stored_j_at(&motion, year);
        assert!((lost / (fitting.hull().capacities.drain_w * year) - 1.0).abs() < 1.0e-9);
        // Settling part-way changes nothing about the answer.
        let before = fitting.stored_j_at(&motion, 3.0 * year);
        fitting.settle(&motion, year);
        assert!((fitting.stored_j_at(&motion, 3.0 * year) / before - 1.0).abs() < 1.0e-12);
        // Long enough and it is empty, and stays there.
        let forever = 1.0e6 * year;
        assert_eq!(fitting.stored_j_at(&motion, forever), 0.0);
    }

    /// Two more slots of engine on the bell, as 19's test built two more engines.
    fn more_engine() -> Form {
        let mut target = Form::starting();
        target.parts.iter_mut().find(|p| p.id == PartId(2)).unwrap().volume_m3 *= 7.0 / 5.0;
        target
    }

    fn begun(fitting: &mut Fitting, target: Form, start_s: f64, motion: &ShipState) {
        let round = Round {
            from: fitting.form().clone(),
            target,
            stored_j: fitting.stored_j_at(motion, start_s),
            start_s,
        };
        let plan = round.solve(fitting.balance()).unwrap();
        fitting.begin_refit(plan);
    }

    #[test]
    fn finishing_a_refit_lands_where_running_it_out_would() {
        // No drain: a finish skips the upkeep with the time, where running it out pays it.
        let b = Balance { living_density_w: 0.0, ..Balance::DEFAULT };
        let motion = ShipState::at(glam::DVec3::ZERO);
        let mut ran = Fitting::full(Form::starting(), b, 0.0);
        begun(&mut ran, more_engine(), 0.0, &motion);
        let end_s = ran.refit().unwrap().duration_s();
        let mut finished = ran.clone();

        finished.settle(&motion, end_s * 0.3);
        assert!(finished.finish_refit());
        assert!(!finished.finish_refit(), "a second finish found another refit");
        ran.settle(&motion, end_s);
        assert!(ran.refit().is_none(), "premise: running it out ends it");
        assert_eq!(finished.form(), &more_engine());
        assert_eq!(finished.hull(), ran.hull());
        assert!((finished.stored_j - ran.stored_j).abs() <= 1e-9 * ran.stored_j.abs().max(1.0));
        // More engine on the same ship pulls harder.
        assert!(ran.rated_g_at(&motion, end_s) > full().rated_g_at(&motion, 0.0));
    }

    /// Partway through a round the form, and what it weighs and holds, are the round's; the grid's
    /// numbers wait for the settlement that finishes a step.
    #[test]
    fn a_refit_under_way_reads_its_steps() {
        // No drain, which would take its own mass off.
        let b = Balance { living_density_w: 0.0, ..Balance::DEFAULT };
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = Fitting::full(Form::starting(), b, 0.0);
        begun(&mut fitting, more_engine(), 0.0, &motion);
        let end_s = fitting.refit().unwrap().duration_s();
        let halfway = fitting.mass_kg_at(&motion, 0.5 * end_s);
        // Stored energy moves into the bell, and a build loses none of it.
        assert!((halfway / fitting.mass_kg_at(&motion, 0.0) - 1.0).abs() < 1.0e-12, "{halfway}");
        assert!(fitting.capacity_j_at(0.5 * end_s) == fitting.hull().capacities.storage_j);
        let after = fitting.hull_at(end_s);
        assert!((after.thrust_n / fitting.hull().thrust_n - 1.4).abs() < 1.0e-12);
        assert_eq!(after.extent_m, fitting.hull().extent_m, "measured only once settled");
    }

    /// A loadout refit on the wire cannot be planned as a round, so it is dropped, as one whose
    /// recipe no longer planned always was. Until S1 no round can be begun on a shard to be sent.
    #[test]
    fn an_account_survives_the_wire() {
        let b = Balance::DEFAULT;
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = Fitting::full(Form::starting(), b, 10.0);
        fitting.settle(&motion, 20.0);
        fitting.drain(3.0 * b.module_energy_j());
        let wire = lc_proto::Fitting::from(&fitting);
        assert_eq!(wire.loadout, STARTING_COUNTS);
        let back = Fitting::from(&lc_proto::decode::<lc_proto::Fitting>(&lc_proto::encode(&wire)).unwrap());
        assert_eq!(back, fitting);

        let refitting = lc_proto::Fitting {
            refit: Some(lc_proto::RefitOrder { from: wire.loadout, target: wire.loadout, stored_j: 0.0, start_s: 20.0 }),
            ..wire
        };
        assert_eq!(Fitting::from(&refitting), fitting);
    }

    /// Every ship is a starting ship, and one saved with other counts comes back as the starting
    /// form with each kind scaled to them.
    #[test]
    fn a_loadout_becomes_the_starting_form_scaled_to_its_counts() {
        let wire = |loadout| lc_proto::Fitting { loadout, ..lc_proto::Fitting::from(&full()) };
        let other = lc_proto::Loadout { storage: 9, living: 2, data: 0, slots: 25, ..STARTING_COUNTS };
        let fitting = Fitting::from(&wire(other));
        let c = fitting.hull().capacities;
        let start = full().hull().capacities;
        assert!((c.storage_j / start.storage_j - 1.5).abs() < 1e-12);
        assert!((c.drain_w / start.drain_w - 2.0).abs() < 1e-12);
        assert_eq!(c.data_b, ONBOARD_DATA_BYTES, "no data part");
        assert_eq!((c.building_w, c.aperture_w), (start.building_w, start.aperture_w));
        assert!(fitting.form().parts.iter().all(|p| p.kind != crate::form::Kind::Data));
        // And its counts are read back off what it holds.
        assert_eq!(lc_proto::Fitting::from(&fitting).loadout, lc_proto::Loadout { slots: 20, ..other });
    }

    /// An account put back into a form whose store holds less than it saved, as a loadout that fell
    /// back to the starting form would be, keeps only what the store holds.
    #[test]
    fn an_account_holds_no_more_than_its_store() {
        let me = Balance::DEFAULT.module_energy_j();
        let overfull = Account { stored_j: 45.0 * me, ..full().account() };
        let back = Fitting::from_account(&overfull, Balance::DEFAULT);
        assert_eq!(back.stored_j, back.hull().capacities.storage_j);
        let partial = Account { stored_j: 12.0 * me, ..full().account() };
        assert_eq!(Fitting::from_account(&partial, Balance::DEFAULT).stored_j, 12.0 * me);
    }

    #[test]
    fn a_grant_stops_at_the_capacity() {
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = full();
        fitting.drain(1.0e20);
        fitting.grant(1.0e30);
        assert_eq!(fitting.stored_j_at(&motion, 0.0), fitting.hull().capacities.storage_j);
    }
}
