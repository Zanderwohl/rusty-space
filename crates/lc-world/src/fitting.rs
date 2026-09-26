//! What a ship is built from, what it weighs, and the energy it holds.
//!
//! See `lightcone/docs/19-ship-fitting.md` for the account and `29-ship-form.md` for the form it is
//! kept on. Energies are joules and masses kilograms throughout; a *module-energy* (ME) is the mass
//! of 19's slot at module density times `c²`, and it is the unit balance is argued in.
//!
//! Only a player's ship carries a [`Fitting`]. Everything else flies on
//! [`Kind::drive`](crate::craft::Kind::drive).

use std::sync::{Arc, Mutex, OnceLock};

use lc_proto::form::Geometry;

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

/// The starting form's shadow broadside, m²: what [`Balance::solar_gain`] is solved on. Pinned by
/// `solar`'s anchor tests for the same reason.
pub const STARTING_BROADSIDE_M2: f64 = 77777.67035341849;

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
        // over what real starlight on the starting form's broadside would give.
        let wanted_w = STARTING.storage * storage_density * me_j / SOLAR_ANCHOR_S + STARTING.living * living_density_w;
        let flux = crate::solar::SOLAR_CONSTANT_W_M2 / (SOLAR_ANCHOR_AU * SOLAR_ANCHOR_AU);
        let rated_load_gain = (SOLAR_ANCHOR_AU / RATED_LOAD_AU) * (SOLAR_ANCHOR_AU / RATED_LOAD_AU);
        Self {
            drive_efficiency: 1.0,
            recovery: 0.95,
            module_density_kg_m3,
            conversion_efficiency,
            solar_gain: wanted_w / (conversion_efficiency * flux * STARTING_BROADSIDE_M2),
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
    /// `Err` for a form whose envelope never closes, which [`rules::check`] refuses, or one partway
    /// through a round that does not place.
    ///
    /// [`rules::check`]: crate::form::rules::check
    pub fn of(form: &Form, balance: &Balance) -> Result<Hull, FormError> {
        Self::measure(form, balance).map(|(hull, _)| hull)
    }

    /// With the grid's geometry, per kilogram of the ship.
    fn measure(form: &Form, balance: &Balance) -> Result<(Hull, Arc<Geometry>), FormError> {
        let measured = measured(form, balance)?;
        let mut hull = Self::cheap(form, balance, 0.0, measured.extent_m, measured.gyration_m);
        hull.thrust_n = aft_aperture_w(form, balance).unwrap_or(0.0) / C_M_S;
        Ok((hull, measured.geometry))
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
}

#[derive(Clone)]
struct Measured {
    extent_m: f64,
    gyration_m: f64,
    geometry: Arc<Geometry>,
}

/// What the grid gives, built once per form and balance for the process: most ships are the
/// starting form, so a shard fitting a hundred builds one.
fn measured(form: &Form, balance: &Balance) -> Result<Measured, FormError> {
    type Cell = Arc<OnceLock<Result<Measured, FormError>>>;
    const KEPT: usize = 16;
    static BUILT: Mutex<Vec<(Form, Balance, Cell)>> = Mutex::new(Vec::new());
    // Built outside the lock, once per key: a thread asking for one form waits for whoever is
    // building it, and never for a build of another.
    let cell = {
        let mut built = BUILT.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match built.iter().find(|(f, b, _)| f == form && b == balance) {
            Some((_, _, cell)) => cell.clone(),
            None => {
                if built.len() == KEPT {
                    built.drain(..1);
                }
                let cell = Cell::default();
                built.push((form.clone(), *balance, cell.clone()));
                cell
            }
        }
    };
    cell.get_or_init(|| {
        FormGrid::new(form, balance).map(|grid| {
            Measured { extent_m: grid.extent_m(), gyration_m: grid.gyration_m(), geometry: Arc::new(grid.geometry(1.0)) }
        })
    })
    .clone()
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
    /// Of the last form the grid measured, per kilogram, which a step that does not place keeps.
    geometry: Arc<Geometry>,
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
    /// every target `rules::check` passes is.
    pub fn full(form: Form, balance: Balance, now_s: f64) -> Self {
        let (hull, geometry) = Hull::measure(&form, &balance).expect("a ship's form closes: every target passes rules::check");
        Self::holding(form, hull, geometry, balance, now_s)
    }

    fn holding(form: Form, hull: Hull, geometry: Arc<Geometry>, balance: Balance, now_s: f64) -> Self {
        Self {
            form,
            hull,
            geometry,
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
    /// ship in the form it had settled into.
    ///
    /// A form settled partway through a round need not place, so it is measured as the steps
    /// measure one: keeping the extent and gyration of the round's start. Neither placing, it keeps
    /// the starting form's.
    pub fn from_account(account: &Account, balance: Balance) -> Self {
        let refit = account.refit.as_ref().and_then(|round| round.solve(&balance).ok());
        let form = account.form.clone();
        let (hull, geometry) = Hull::measure(&form, &balance).unwrap_or_else(|_| {
            let was = account.refit.as_ref().map(|round| &round.from).and_then(|from| Hull::measure(from, &balance).ok());
            let (was, geometry) = was.unwrap_or_else(|| {
                Hull::measure(&Form::starting(), &balance).expect("the starting form closes")
            });
            (Hull::partway(&form, &balance, &was), geometry)
        });
        let full = Self::holding(form, hull, geometry, balance, account.since_s);
        Self {
            refit,
            // What the store holds may not be more than it can: a form re-measured under another
            // balance can hold less than was saved.
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
        self.take_form(self.form.clone());
    }

    /// Become `form`, measuring it if the grid can: at a step's end, when the form it was partway
    /// through becomes the ship's, and wherever else the form changes.
    fn take_form(&mut self, form: Form) {
        match Hull::measure(&form, &self.balance) {
            Ok((hull, geometry)) => {
                self.hull = hull;
                self.geometry = geometry;
            }
            Err(_) => self.hull = Hull::partway(&form, &self.balance, &self.hull),
        }
        self.form = form;
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

    /// The shadow table, broadside and moments per kilogram of the last form the grid measured.
    pub fn geometry(&self) -> &Geometry {
        &self.geometry
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
            let (progress, done) = (plan.at(now_s), plan.is_done(now_s));
            if done {
                self.refit = None;
            }
            if progress.form != self.form {
                self.take_form(progress.form);
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
        self.take_form(end.form);
        self.stored_j = (self.stored_j - remaining_j).clamp(0.0, self.hull.capacities.storage_j.max(0.0));
        true
    }

    /// For nothing, dropping any refit under way. Storage is cut to the new capacity. Settle
    /// first. Refused for a form the grid cannot measure.
    pub fn refit_at_once(&mut self, form: Form) -> Result<(), FormError> {
        (self.hull, self.geometry) = Hull::measure(&form, &self.balance)?;
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
        self.take_form(canceled.form);
        self.stored_j = canceled.stored_j;
    }

    /// The form each refit step leaves, and the coordinate second its step ends, for the steps
    /// ending after `after_s` and by `until_s`. Steps ending together leave one form.
    pub fn steps_ending(&self, after_s: f64, until_s: f64) -> Vec<(f64, Form)> {
        let Some(plan) = &self.refit else { return Vec::new() };
        let start_s = plan.round().start_s;
        let mut ends: Vec<f64> =
            plan.steps().iter().map(|s| start_s + s.ends_s()).filter(|&end| end > after_s && end <= until_s).collect();
        ends.dedup();
        ends.into_iter().map(|end| (end, plan.at(end).form)).collect()
    }
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
    fn from(b: Balance) -> Self {
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

impl From<&Round> for lc_proto::Round {
    fn from(r: &Round) -> Self {
        Self { from: (&r.from).into(), target: (&r.target).into(), stored_j: r.stored_j, start_s: r.start_s }
    }
}

impl From<&lc_proto::Round> for Round {
    fn from(r: &lc_proto::Round) -> Self {
        Self { from: (&r.from).into(), target: (&r.target).into(), stored_j: r.stored_j, start_s: r.start_s }
    }
}

impl From<&Fitting> for lc_proto::Fitting {
    fn from(f: &Fitting) -> Self {
        let a = f.account();
        Self {
            balance: f.balance.into(),
            form: (&a.form).into(),
            stored_j: a.stored_j,
            since_s: a.since_s,
            rapidity_since: a.rapidity_since,
            committed_j: a.committed_j,
            solar_w: a.solar_w,
            refit: a.refit.as_ref().map(Into::into),
        }
    }
}

impl From<&lc_proto::Fitting> for Fitting {
    fn from(f: &lc_proto::Fitting) -> Self {
        let account = Account {
            form: (&f.form).into(),
            stored_j: f.stored_j,
            since_s: f.since_s,
            rapidity_since: f.rapidity_since,
            committed_j: f.committed_j,
            solar_w: f.solar_w,
            refit: f.refit.as_ref().map(Into::into),
        };
        Fitting::from_account(&account, f.balance.into())
    }
}

impl From<&Fitting> for lc_proto::Hull {
    /// As settled. The inertia is the whole ship's, stored energy included, at the settlement.
    fn from(f: &Fitting) -> Self {
        let mut geometry = (*f.geometry).clone();
        let mass_kg = f.settled_mass_kg();
        for moment in &mut geometry.inertia_kg_m2 {
            *moment *= mass_kg;
        }
        let (c, aft_w) = (&f.hull.capacities, f.hull.thrust_n * C_M_S);
        Self {
            form: (&f.form).into(),
            scales_m: f.form.parts.iter().map(|p| p.scale_m(f.balance.min_part_m3)).collect(),
            capacities: lc_proto::form::Capacities {
                storage_j: c.storage_j,
                drone_w: c.building_w,
                aft_w,
                fore_w: (c.aperture_w - aft_w).max(0.0),
                living_w: c.drain_w,
                data_b: c.data_b,
                dry_mass_kg: f.hull.dry_kg,
            },
            geometry,
        }
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

    /// A round crosses the wire as its recipe, and both ends solve it to the same plan.
    #[test]
    fn an_account_survives_the_wire() {
        let b = Balance::DEFAULT;
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = Fitting::full(Form::starting(), b, 10.0);
        fitting.settle(&motion, 20.0);
        fitting.drain(3.0 * b.module_energy_j());
        let across = |f: &Fitting| Fitting::from(&lc_proto::decode::<lc_proto::Fitting>(&lc_proto::encode(&lc_proto::Fitting::from(f))).unwrap());
        assert_eq!(across(&fitting), fitting);

        begun(&mut fitting, more_engine(), 20.0, &motion);
        let end_s = fitting.refit().unwrap().duration_s();
        fitting.settle(&motion, 20.0 + 0.5 * end_s);
        let back = across(&fitting);
        assert!(back.refit().is_some(), "the round was dropped");
        assert_eq!(back, fitting);
    }

    /// Partway through a round the settled form may not place: a move waiting for a parent the
    /// build phase has not made. It still loads, keeping the round's start's extent.
    #[test]
    fn a_form_settled_partway_that_does_not_place_still_loads() {
        let b = Balance::DEFAULT;
        let start = Fitting::full(Form::starting(), b, 0.0);
        let mut orphan = Form::starting();
        let hung = orphan.parts.iter_mut().find(|p| p.placement.is_some()).unwrap();
        hung.placement.as_mut().unwrap().parent = PartId(99);
        assert!(Hull::of(&orphan, &b).is_err(), "premise: it does not place");
        let round = Round { from: Form::starting(), target: more_engine(), stored_j: 0.0, start_s: 0.0 };
        let account = Account { form: orphan.clone(), refit: Some(round), ..start.account() };
        let back = Fitting::from_account(&account, b);
        assert_eq!(back.form(), &orphan);
        assert_eq!(back.hull().extent_m, start.hull().extent_m);
        assert_eq!(back.hull().gyration_m, start.hull().gyration_m);
    }

    /// What `Fitted` states: a scale per part, the aft engines apart from the rest, and the grid's
    /// moments scaled to the ship as it stands.
    #[test]
    fn the_wires_hull_is_the_settled_form_measured() {
        let fitting = full();
        let hull = lc_proto::Hull::from(&fitting);
        let b = Balance::DEFAULT;
        assert_eq!(hull.form, lc_proto::Form::from(fitting.form()));
        assert_eq!(hull.scales_m.len(), fitting.form().parts.len());
        for (part, scale) in fitting.form().parts.iter().zip(&hull.scales_m) {
            assert!((part.shape(b.min_part_m3).volume() / part.drawn().volume(*scale) - 1.0).abs() < 1e-12);
        }
        assert_eq!(hull.capacities.storage_j, fitting.hull().capacities.storage_j);
        assert_eq!(hull.capacities.aft_w, fitting.hull().thrust_n * C_M_S);
        assert_eq!(hull.capacities.fore_w, 0.0, "the starting form's one bell fires aft");
        assert_eq!(hull.geometry.extent_m, fitting.hull().extent_m);
        let grid = FormGrid::new(fitting.form(), &b).unwrap();
        let mass_kg = fitting.mass_kg_at(&ShipState::at(DVec3::ZERO), 0.0);
        let whole = grid.geometry(mass_kg).inertia_kg_m2;
        for (sent, whole) in hull.geometry.inertia_kg_m2.iter().zip(whole) {
            assert!((sent - whole).abs() <= 1e-12 * whole.abs().max(1.0), "{sent} vs {whole}");
        }
    }

    #[test]
    fn each_step_that_ends_leaves_its_form_at_its_end() {
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = full();
        let mut target = more_engine();
        target.parts.iter_mut().find(|p| p.kind == crate::form::Kind::Storage).unwrap().volume_m3 *= 1.1;
        begun(&mut fitting, target.clone(), 0.0, &motion);
        let plan = fitting.refit().unwrap().clone();
        assert!(plan.steps().len() >= 2, "premise: {} steps", plan.steps().len());
        let ends = fitting.steps_ending(0.0, f64::INFINITY);
        assert_eq!(ends.len(), plan.steps().len());
        assert_eq!(ends.last().unwrap().1, target);
        for ((end, form), step) in ends.iter().zip(plan.steps()) {
            assert_eq!(*end, step.ends_s());
            assert_eq!(form, &plan.at(*end).form);
            assert_ne!(form, &plan.at(end - 1e-3 * step.duration_s).form, "it changed only at the end");
        }
        assert!(fitting.steps_ending(ends[0].0, ends[0].0).is_empty(), "a step is counted once");
    }

    /// An account put back into a form whose store holds less than it saved, as one measured under
    /// another balance may, keeps only what the store holds.
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
