//! What a ship is built from, what it weighs, and the energy it holds.
//!
//! See `lightcone/docs/19-ship-fitting.md`. Energies are joules and masses kilograms throughout;
//! a *module-energy* is one module's dry mass times `c²`, and it is the unit balance is argued in.
//!
//! Only a player's ship carries a [`Fitting`]. Everything else flies on
//! [`Kind::drive`](crate::craft::Kind::drive).

use crate::cost;
use crate::flight::{C_M_S, G0};
use crate::motion::ShipState;
use crate::refit::Refit;

pub const C2: f64 = C_M_S * C_M_S;

/// The volume of the reference 500 m hull, which is [`Loadout::STARTING`]'s twenty slots.
const REFERENCE_HULL_M3: f64 = OVOID_M3_PER_CUBIC_M * 500.0 * 500.0 * 500.0;

/// An ovoid five long, three across and one deep holds `π L³ / 50`.
const OVOID_M3_PER_CUBIC_M: f64 = std::f64::consts::PI
    * 0.5
    * (0.5 * crate::craft::BEAM_PER_LENGTH)
    * (0.5 * crate::craft::HEIGHT_PER_LENGTH)
    * 4.0
    / 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Module {
    Storage,
    Drone,
    Living,
    Engine,
    Data,
}

impl Module {
    pub const ALL: [Module; 5] = [Module::Storage, Module::Drone, Module::Living, Module::Engine, Module::Data];

    pub fn name(self) -> &'static str {
        match self {
            Module::Storage => "energy storage",
            Module::Drone => "worker drones",
            Module::Living => "living space",
            Module::Engine => "engines",
            Module::Data => "data storage",
        }
    }
}

/// How many of each module, and how many slots the hull has for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Loadout {
    pub storage: u32,
    pub drones: u32,
    pub living: u32,
    pub engines: u32,
    pub slots: u32,
    pub data: u32,
}

impl Loadout {
    /// What a new ship is given, and what a ship saved before fittings existed comes back as.
    pub const STARTING: Self = Self { storage: 6, drones: 2, living: 1, engines: 5, slots: 20, data: 1 };

    pub fn count(&self, module: Module) -> u32 {
        match module {
            Module::Storage => self.storage,
            Module::Drone => self.drones,
            Module::Living => self.living,
            Module::Engine => self.engines,
            Module::Data => self.data,
        }
    }

    pub fn count_mut(&mut self, module: Module) -> &mut u32 {
        match module {
            Module::Storage => &mut self.storage,
            Module::Drone => &mut self.drones,
            Module::Living => &mut self.living,
            Module::Engine => &mut self.engines,
            Module::Data => &mut self.data,
        }
    }

    pub const fn modules(&self) -> u32 {
        self.storage + self.drones + self.living + self.engines + self.data
    }

    pub fn free_slots(&self) -> u32 {
        self.slots.saturating_sub(self.modules())
    }

    /// Something a refit may aim at: every module has a slot, and a drone is left to do the work.
    pub fn is_buildable(&self) -> bool {
        self.modules() <= self.slots && self.drones >= 1
    }
}

/// Every tunable number. A server states its own with every account, so a client assumes none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Balance {
    /// ε in `m' = m exp(−Δη/ε)`. One is a perfect photon rocket; above one is unphysical, and
    /// allowed on purpose.
    pub drive_efficiency: f64,
    /// Fraction of a module's build energy that dismantling it returns.
    pub recovery: f64,
    /// Energy one storage module holds, in module-energies.
    pub storage_per_module: f64,
    /// Newtons each engine pushes with.
    pub engine_thrust_n: f64,
    /// Watts each drone moves into or out of a module.
    pub drone_power_w: f64,
    /// Watts each living module drains, continuously.
    pub living_drain_w: f64,
    /// The frame's own density, kg/m³, over every slot whether filled or not.
    pub hull_density_kg_m3: f64,
    pub slot_volume_m3: f64,
    pub module_density_kg_m3: f64,
    /// η: of what is converted, the fraction stored.
    pub conversion_efficiency: f64,
    /// An unphysical multiplier on collection, because a module-energy is `mc²` and real
    /// starlight on a real hull would take billions of years to pay for one. See
    /// `lightcone/docs/20-solar-power.md`.
    pub solar_gain: f64,
    /// Bytes of knowledge one data module holds. See [`DATA_ANCHOR_S`].
    pub data_per_module: f64,
    /// A data module's mass over any other module's, and so its build energy over theirs.
    pub data_mass_fraction: f64,
    /// How many times longer a data module takes to build or take apart than any other module.
    pub data_work_factor: f64,
    /// Module-energies of capacity per m³ of storage.
    pub storage_density: f64,
    /// W/m³ of building power.
    pub drone_density_w: f64,
    /// W/m³ of aperture power; thrust is this over `c`.
    pub engine_density_w: f64,
    /// W/m³ drained, continuously.
    pub living_density_w: f64,
    /// Bytes per m³.
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

/// What [`Balance::data_per_module`] is anchored to: one module holds a year of a
/// thirty-minute stare in every band. Only raw logs take room — see
/// `lightcone/docs/24-standing-instruments.md`.
pub const DATA_ANCHOR_S: f64 = crate::flight::JULIAN_YEAR_S;
const DATA_ANCHOR_CADENCE_S: f64 = 1800.0;

/// Bytes of raw log a craft holds with no data modules at all: a couple of months of one star.
pub const ONBOARD_DATA_BYTES: f64 = 1_048_576.0;

/// The distance, AU from a Sun-like star, at which the starting ship broadside fills from empty in
/// [`SOLAR_ANCHOR_S`], net of its living drain. What [`Balance::solar_gain`] is derived from.
pub const SOLAR_ANCHOR_AU: f64 = 0.1;
pub const SOLAR_ANCHOR_S: f64 = crate::flight::JULIAN_YEAR_S;

impl Balance {
    pub const DEFAULT: Self = {
        let slot_volume_m3 = REFERENCE_HULL_M3 / Loadout::STARTING.slots as f64;
        // A 40 ft ISO container at its maximum gross mass, over its outside volume.
        let module_density_kg_m3 = 30_480.0 / (12.192 * 2.438 * 2.591);
        let hull_density_kg_m3 = 50.0;
        let storage_per_module = 5.0;
        let data_mass_fraction = 0.5;
        let start = Loadout::STARTING;
        let module_kg = slot_volume_m3 * module_density_kg_m3;
        let start_modules = (start.modules() - start.data) as f64 + start.data as f64 * data_mass_fraction;
        let full_kg = (start_modules + start.storage as f64 * storage_per_module)
            * module_kg
            + start.slots as f64 * slot_volume_m3 * hull_density_kg_m3;
        let week_s = 7.0 * 86_400.0;
        let century_s = 100.0 * crate::flight::JULIAN_YEAR_S;
        let living_drain_w = module_kg * C2 / century_s;
        let conversion_efficiency = 0.7;
        let data_per_module = DATA_ANCHOR_S / DATA_ANCHOR_CADENCE_S
            * em_spectra::Band::ALL.len() as f64
            * crate::knowledge::SAMPLE_BYTES;
        let engine_thrust_n = G0 * full_kg;
        let drone_power_w = module_kg * C2 / week_s;
        let day_s = 86_400.0;
        let degree = std::f64::consts::PI / 180.0;
        // Collection that fills the starting storage in the anchor time and pays the drain too,
        // over what real starlight on the broadside of a 500 m hull would give.
        let wanted_w = storage_per_module * start.storage as f64 * module_kg * C2 / SOLAR_ANCHOR_S
            + start.living as f64 * living_drain_w;
        let broadside_m2 = std::f64::consts::PI
            * 250.0
            * (250.0 * crate::craft::BEAM_PER_LENGTH);
        let flux = crate::solar::SOLAR_CONSTANT_W_M2 / (SOLAR_ANCHOR_AU * SOLAR_ANCHOR_AU);
        Self {
            drive_efficiency: 1.0,
            recovery: 0.95,
            storage_per_module,
            // One g of the starting ship with its storage full, so its five engines give 5 g.
            engine_thrust_n,
            drone_power_w,
            living_drain_w,
            hull_density_kg_m3,
            slot_volume_m3,
            module_density_kg_m3,
            conversion_efficiency,
            solar_gain: wanted_w / (conversion_efficiency * flux * broadside_m2),
            data_per_module,
            data_mass_fraction,
            data_work_factor: 3.0,
            storage_density: storage_per_module / slot_volume_m3,
            drone_density_w: drone_power_w / slot_volume_m3,
            engine_density_w: engine_thrust_n * C_M_S / slot_volume_m3,
            living_density_w: living_drain_w / slot_volume_m3,
            data_density_b: data_per_module / slot_volume_m3,
            bay_mass_fraction: 0.1,
            spar_mass_fraction: 0.05,
            min_part_m3: 1_000.0,
            min_drone_m3: 10_000.0,
            spar_gap: 0.5,
            spar_thickness: 0.02,
            move_work_factor: 0.25,
            // Placeholder: anchored by F5.
            hull_areal_density: 0.0,
            envelope_margin: 0.05,
            engine_clear_half_angle_rad: 15.0 * degree,
            field_idle_k: 400.0,
            // Placeholder: anchored by H2 to 10 ME on the starting envelope. Zero makes `Q_max`
            // zero, so nothing may rely on it before then.
            field_capacity: 0.0,
            // Anchored by H2; 30's figure until then.
            field_tau_s: 1.84e6,
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

    pub fn module_mass_kg(&self) -> f64 {
        self.slot_volume_m3 * self.module_density_kg_m3
    }

    /// One module-energy, joules: what building any module but a data module costs.
    pub fn module_energy_j(&self) -> f64 {
        self.module_mass_kg() * C2
    }

    pub fn mass_of_kg(&self, module: Module) -> f64 {
        match module {
            Module::Data => self.data_mass_fraction * self.module_mass_kg(),
            _ => self.module_mass_kg(),
        }
    }

    /// What building one `module` costs, joules.
    pub fn build_energy_j(&self, module: Module) -> f64 {
        self.mass_of_kg(module) * C2
    }

    /// How long building or taking apart one `module` keeps drones of `power_w` busy, seconds.
    /// Not proportional to its energy: a data module is cheap and slow.
    pub fn build_s(&self, module: Module, power_w: f64) -> f64 {
        let factor = match module {
            Module::Data => self.data_work_factor,
            _ => 1.0,
        };
        factor * self.module_energy_j() / power_w
    }

    pub fn slot_structure_kg(&self) -> f64 {
        self.slot_volume_m3 * self.hull_density_kg_m3
    }

    /// What growing the hull by one slot costs, joules.
    pub fn slot_energy_j(&self) -> f64 {
        self.slot_structure_kg() * C2
    }

    pub fn dry_mass_kg(&self, loadout: &Loadout) -> f64 {
        Module::ALL.iter().map(|&m| loadout.count(m) as f64 * self.mass_of_kg(m)).sum::<f64>()
            + loadout.slots as f64 * self.slot_structure_kg()
    }

    pub fn data_capacity(&self, loadout: &Loadout) -> f64 {
        ONBOARD_DATA_BYTES + loadout.data as f64 * self.data_per_module
    }

    pub fn capacity_j(&self, loadout: &Loadout) -> f64 {
        loadout.storage as f64 * self.storage_per_module * self.module_energy_j()
    }

    pub fn drain_w(&self, loadout: &Loadout) -> f64 {
        loadout.living as f64 * self.living_drain_w
    }

    pub fn refit_power_w(&self, loadout: &Loadout) -> f64 {
        loadout.drones as f64 * self.drone_power_w
    }

    /// The proper acceleration the engines give a ship of this mass, in g.
    pub fn accel_g(&self, loadout: &Loadout, mass_kg: f64) -> f64 {
        loadout.engines as f64 * self.engine_thrust_n / mass_kg / G0
    }

    /// The hull length whose ovoid holds `slots` slots, meters.
    pub fn length_m(&self, slots: u32) -> f64 {
        (slots as f64 * self.slot_volume_m3 / OVOID_M3_PER_CUBIC_M).cbrt()
    }
}

impl Default for Balance {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A ship's modules and the energy in them, as a closed form in coordinate time.
///
/// The account is **settled** at an instant — stored energy, the loadout and the rapidity the
/// motive had flown by then — and read forward from it. Anything that changes a term of the
/// closed form settles first: a new motive, a refit step, a grant.
#[derive(Clone, Debug, PartialEq)]
pub struct Fitting {
    pub loadout: Loadout,
    /// Not saved: a server stamps its own on every craft it loads.
    pub balance: Balance,
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
    refit: Option<Refit>,
}

/// A fitting as it is saved and sent: the settled terms, and the refit's recipe.
#[derive(Clone, Debug, PartialEq)]
pub struct Account {
    pub loadout: Loadout,
    pub stored_j: f64,
    pub since_s: f64,
    pub rapidity_since: f64,
    pub committed_j: f64,
    pub solar_w: f64,
    pub refit: Option<crate::refit::Order>,
}

impl Fitting {
    /// A ship with this loadout and its storage full.
    pub fn full(loadout: Loadout, balance: Balance, now_s: f64) -> Self {
        Self {
            loadout,
            balance,
            stored_j: balance.capacity_j(&loadout),
            since_s: now_s,
            rapidity_since: 0.0,
            committed_j: 0.0,
            solar_w: 0.0,
            refit: None,
        }
    }

    pub fn account(&self) -> Account {
        Account {
            loadout: self.loadout,
            stored_j: self.stored_j,
            since_s: self.since_s,
            rapidity_since: self.rapidity_since,
            committed_j: self.committed_j,
            solar_w: self.solar_w,
            refit: self.refit.as_ref().map(Refit::order),
        }
    }

    /// Put an account back. A refit whose recipe no longer plans is dropped, which leaves the
    /// ship as the account says it was when the refit began.
    pub fn from_account(account: &Account, balance: Balance) -> Self {
        let refit = account.refit.as_ref().and_then(|order| order.solve(&balance).ok());
        Self {
            loadout: account.loadout,
            balance,
            stored_j: account.stored_j,
            since_s: account.since_s,
            rapidity_since: account.rapidity_since,
            committed_j: account.committed_j,
            solar_w: account.solar_w,
            refit,
        }
    }

    pub fn refit(&self) -> Option<&Refit> {
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

    /// The loadout at a coordinate time, counting refit steps finished by then.
    pub fn loadout_at(&self, now_s: f64) -> Loadout {
        match &self.refit {
            Some(refit) => refit.at(now_s).loadout,
            None => self.loadout,
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
        self.balance.dry_mass_kg(&self.loadout) + self.stored_j / C2
    }

    /// What starlight has added, less what living space has drained, by `now_s`.
    ///
    /// Filling stops at capacity, and the excess is lost. Draining stops once only committed
    /// energy is left, and running out costs nothing else. The clamp is on the segment's total
    /// rather than its path, which is exact until a limit is reached and off by at most one
    /// segment's income when one is.
    fn income_j(&self, now_s: f64) -> f64 {
        let elapsed = (now_s - self.since_s).max(0.0);
        let net_w = self.solar_w - self.balance.drain_w(&self.loadout);
        if net_w >= 0.0 {
            let room = (self.balance.capacity_j(&self.loadout) - self.stored_j).max(0.0);
            (net_w * elapsed).min(room)
        } else {
            let free = (self.stored_j - self.committed_j).max(0.0);
            -(-net_w * elapsed).min(free)
        }
    }

    /// Stored energy at a coordinate time, joules.
    pub fn stored_j_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        let moved = match &self.refit {
            Some(refit) => refit.at(now_s).consumed_j - refit.at(self.since_s).consumed_j,
            None => 0.0,
        };
        (self.stored_j + self.income_j(now_s) - self.burn_spent_j(motion, now_s) - moved).max(0.0)
    }

    /// What is stored and not committed, joules.
    pub fn free_j_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        (self.stored_j_at(motion, now_s) - self.committed_j_at(motion, now_s)).max(0.0)
    }

    pub fn capacity_j_at(&self, now_s: f64) -> f64 {
        self.balance.capacity_j(&self.loadout_at(now_s))
    }

    /// The whole ship, stored energy included, kilograms.
    pub fn mass_kg_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        let (dry, in_hand) = match &self.refit {
            Some(refit) => {
                let progress = refit.at(now_s);
                (self.balance.dry_mass_kg(&progress.loadout), progress.in_hand_kg)
            }
            None => (self.balance.dry_mass_kg(&self.loadout), 0.0),
        };
        dry + in_hand + self.stored_j_at(motion, now_s) / C2
    }

    /// The acceleration this ship's engines give it now, in g.
    pub fn rated_g_at(&self, motion: &ShipState, now_s: f64) -> f64 {
        self.balance.accel_g(&self.loadout_at(now_s), self.mass_kg_at(motion, now_s))
    }

    /// Fold everything up to `now_s` into the settled terms, reading the burn off `motion` —
    /// which must be the motive that was in force up to now.
    pub fn settle(&mut self, motion: &ShipState, now_s: f64) {
        if now_s <= self.since_s {
            return;
        }
        let stored = self.stored_j_at(motion, now_s);
        self.committed_j = self.committed_j_at(motion, now_s);
        if let Some(refit) = &self.refit {
            let progress = refit.at(now_s);
            self.loadout = progress.loadout;
            if refit.is_done(now_s) {
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
        let capacity = self.balance.capacity_j(&self.loadout);
        self.stored_j = (self.stored_j + joules.max(0.0)).min(capacity.max(self.stored_j));
    }

    /// Settle first.
    pub fn drain(&mut self, joules: f64) {
        self.stored_j = (self.stored_j - joules.max(0.0)).max(0.0);
    }

    /// Begin a refit. Settle first; the caller has already checked it plans.
    pub fn begin_refit(&mut self, refit: Refit) {
        self.refit = Some(refit);
    }

    /// Only the time is skipped: the remaining steps' energy is still taken. Settle first.
    pub fn finish_refit(&mut self) -> bool {
        let Some(refit) = self.refit.take() else { return false };
        let end = refit.at(refit.order().start_s + refit.duration_s());
        let remaining_j = end.consumed_j - refit.at(self.since_s).consumed_j;
        self.loadout = end.loadout;
        let capacity = self.balance.capacity_j(&self.loadout);
        self.stored_j = (self.stored_j - remaining_j).clamp(0.0, capacity.max(0.0));
        true
    }

    /// For nothing, dropping any refit under way. Storage is cut to the new capacity. Settle
    /// first.
    pub fn refit_at_once(&mut self, loadout: Loadout) {
        self.refit = None;
        self.loadout = loadout;
        self.stored_j = self.stored_j.min(self.balance.capacity_j(&loadout).max(0.0));
    }

    /// Stop a refit where it is. Settle first. The step in progress is reversed.
    pub fn cancel_refit(&mut self, now_s: f64) {
        let Some(refit) = self.refit.take() else { return };
        let progress = refit.at(now_s);
        self.loadout = progress.loadout;
        self.stored_j = (self.stored_j + progress.reversal_j).max(0.0);
    }
}

impl From<lc_proto::Loadout> for Loadout {
    fn from(l: lc_proto::Loadout) -> Self {
        Self { storage: l.storage, drones: l.drones, living: l.living, engines: l.engines, slots: l.slots, data: l.data }
    }
}

impl From<Loadout> for lc_proto::Loadout {
    fn from(l: Loadout) -> Self {
        Self { storage: l.storage, drones: l.drones, living: l.living, engines: l.engines, slots: l.slots, data: l.data }
    }
}

impl From<lc_proto::Balance> for Balance {
    fn from(b: lc_proto::Balance) -> Self {
        Self {
            drive_efficiency: b.drive_efficiency,
            recovery: b.recovery,
            storage_per_module: b.storage_per_module,
            engine_thrust_n: b.engine_thrust_n,
            drone_power_w: b.drone_power_w,
            living_drain_w: b.living_drain_w,
            hull_density_kg_m3: b.hull_density_kg_m3,
            slot_volume_m3: b.slot_volume_m3,
            module_density_kg_m3: b.module_density_kg_m3,
            conversion_efficiency: b.conversion_efficiency,
            solar_gain: b.solar_gain,
            data_per_module: b.data_per_module,
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
            storage_per_module: b.storage_per_module,
            engine_thrust_n: b.engine_thrust_n,
            drone_power_w: b.drone_power_w,
            living_drain_w: b.living_drain_w,
            hull_density_kg_m3: b.hull_density_kg_m3,
            slot_volume_m3: b.slot_volume_m3,
            module_density_kg_m3: b.module_density_kg_m3,
            conversion_efficiency: b.conversion_efficiency,
            solar_gain: b.solar_gain,
            data_per_module: b.data_per_module,
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
    fn from(f: &Fitting) -> Self {
        let a = f.account();
        Self {
            balance: f.balance.into(),
            loadout: a.loadout.into(),
            stored_j: a.stored_j,
            since_s: a.since_s,
            rapidity_since: a.rapidity_since,
            committed_j: a.committed_j,
            solar_w: a.solar_w,
            refit: a.refit.map(|o| lc_proto::RefitOrder {
                from: o.from.into(),
                target: o.target.into(),
                stored_j: o.stored_j,
                start_s: o.start_s,
            }),
        }
    }
}

impl From<&lc_proto::Fitting> for Fitting {
    fn from(f: &lc_proto::Fitting) -> Self {
        let account = Account {
            loadout: f.loadout.into(),
            stored_j: f.stored_j,
            since_s: f.since_s,
            rapidity_since: f.rapidity_since,
            committed_j: f.committed_j,
            solar_w: f.solar_w,
            refit: f.refit.map(|o| crate::refit::Order {
                from: o.from.into(),
                target: o.target.into(),
                stored_j: o.stored_j,
                start_s: o.start_s,
            }),
        };
        Fitting::from_account(&account, f.balance.into())
    }
}

impl From<crate::refit::Shortage> for lc_proto::Shortfall {
    fn from(s: crate::refit::Shortage) -> Self {
        use crate::refit::Shortage;
        match s {
            Shortage::Unbuildable => Self::Unbuildable,
            Shortage::Energy => Self::Energy,
            Shortage::NoDrones => Self::NoDrones,
            Shortage::CannotBuild(m) => Self::CannotBuild(m.into()),
            Shortage::CannotDismantle(m) => Self::CannotDismantle(m.into()),
        }
    }
}

impl From<Module> for lc_proto::Module {
    fn from(m: Module) -> Self {
        match m {
            Module::Storage => Self::Storage,
            Module::Drone => Self::Drone,
            Module::Living => Self::Living,
            Module::Engine => Self::Engine,
            Module::Data => Self::Data,
        }
    }
}

impl From<lc_proto::Module> for Module {
    fn from(m: lc_proto::Module) -> Self {
        match m {
            lc_proto::Module::Storage => Self::Storage,
            lc_proto::Module::Drone => Self::Drone,
            lc_proto::Module::Living => Self::Living,
            lc_proto::Module::Engine => Self::Engine,
            lc_proto::Module::Data => Self::Data,
        }
    }
}

impl From<lc_proto::Shortfall> for crate::refit::Shortage {
    fn from(short: lc_proto::Shortfall) -> Self {
        use lc_proto::Shortfall;
        match short {
            Shortfall::Unbuildable => Self::Unbuildable,
            Shortfall::Energy => Self::Energy,
            Shortfall::NoDrones => Self::NoDrones,
            Shortfall::CannotBuild(m) => Self::CannotBuild(m.into()),
            Shortfall::CannotDismantle(m) => Self::CannotDismantle(m.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::craft::{Craft, CraftId, Kind};
    use glam::DVec3;

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
            storage_per_module: 3.0,
            engine_thrust_n: 4.0,
            drone_power_w: 5.0,
            living_drain_w: 6.0,
            hull_density_kg_m3: 7.0,
            slot_volume_m3: 8.0,
            module_density_kg_m3: 9.0,
            conversion_efficiency: 10.0,
            solar_gain: 11.0,
            data_per_module: 12.0,
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
    fn the_reference_hull_is_exactly_twenty_slots_and_five_hundred_meters() {
        let b = Balance::DEFAULT;
        assert!((b.length_m(20) - 500.0).abs() < 1.0e-9, "{}", b.length_m(20));
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        craft.length_m = b.length_m(20);
        assert!((craft.volume_m3() / b.slot_volume_m3 - 20.0).abs() < 1.0e-9);
        // Doubling the slots is the cube root of two on the length.
        assert!((b.length_m(40) / b.length_m(20) - 2f64.cbrt()).abs() < 1.0e-12);
    }

    #[test]
    fn a_module_is_a_container_stack_the_size_of_a_slot() {
        let b = Balance::DEFAULT;
        assert!((b.slot_volume_m3 - 392_699.08).abs() < 0.01);
        assert!((b.module_density_kg_m3 - 395.77).abs() < 0.01);
        assert!((b.module_energy_j() / 1.3968e25 - 1.0).abs() < 1.0e-4);
    }

    #[test]
    fn the_starting_ship_full_pulls_five_g_and_more_empty() {
        let b = Balance::DEFAULT;
        let start = Loadout::STARTING;
        let dry = b.dry_mass_kg(&start);
        let full = dry + b.capacity_j(&start) / C2;
        let g = b.accel_g(&start, full);
        assert!((g / 5.0 - 1.0).abs() < 1.0e-12, "{g}");
        let empty = b.accel_g(&start, dry);
        assert!((empty - 13.81).abs() < 0.01, "{empty}");
    }

    #[test]
    fn stored_energy_weighs_what_it_holds() {
        let motion = ShipState::at(DVec3::ZERO);
        let full = Fitting::full(Loadout::STARTING, Balance::DEFAULT, 0.0);
        let b = Balance::DEFAULT;
        let mass = full.mass_kg_at(&motion, 0.0);
        let expected = b.dry_mass_kg(&Loadout::STARTING) + 30.0 * b.module_mass_kg();
        assert!((mass / expected - 1.0).abs() < 1.0e-12, "{mass} vs {expected}");
    }

    #[test]
    fn living_space_drains_until_nothing_free_is_left() {
        let b = Balance::DEFAULT;
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = Fitting::full(Loadout::STARTING, b, 0.0);
        let year = crate::flight::JULIAN_YEAR_S;
        let lost = fitting.stored_j_at(&motion, 0.0) - fitting.stored_j_at(&motion, year);
        assert!((lost / (Loadout::STARTING.living as f64 * b.living_drain_w * year) - 1.0).abs() < 1.0e-9);
        // Settling part-way changes nothing about the answer.
        let before = fitting.stored_j_at(&motion, 3.0 * year);
        fitting.settle(&motion, year);
        assert!((fitting.stored_j_at(&motion, 3.0 * year) / before - 1.0).abs() < 1.0e-12);
        // Long enough and it is empty, and stays there.
        let forever = 1.0e6 * year;
        assert_eq!(fitting.stored_j_at(&motion, forever), 0.0);
    }

    #[test]
    fn finishing_a_refit_lands_where_running_it_out_would() {
        // No drain: a finish skips the upkeep with the time, where running it out pays it.
        let b = Balance { living_drain_w: 0.0, ..Balance::DEFAULT };
        let target = Loadout { engines: 7, ..Loadout::STARTING };
        let begun = |fitting: &mut Fitting| {
            let order = crate::refit::Order {
                from: Loadout::STARTING,
                target,
                stored_j: fitting.stored_j,
                start_s: 0.0,
            };
            fitting.begin_refit(order.solve(&b).unwrap());
        };
        let motion = ShipState::at(glam::DVec3::ZERO);
        let mut ran = Fitting::full(Loadout::STARTING, b, 0.0);
        begun(&mut ran);
        let end_s = ran.refit().unwrap().duration_s();
        let mut finished = ran.clone();

        finished.settle(&motion, end_s * 0.3);
        assert!(finished.finish_refit());
        assert!(!finished.finish_refit(), "a second finish found another refit");
        ran.settle(&motion, end_s);
        assert!(ran.refit().is_none(), "premise: running it out ends it");
        assert_eq!(finished.loadout, target);
        assert!((finished.stored_j - ran.stored_j).abs() <= 1e-9 * ran.stored_j.abs().max(1.0));
    }

    #[test]
    fn an_account_with_a_refit_survives_the_wire() {
        let b = Balance::DEFAULT;
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = Fitting::full(Loadout::STARTING, b, 10.0);
        fitting.settle(&motion, 20.0);
        let order = crate::refit::Order {
            from: fitting.loadout,
            target: Loadout { engines: 6, ..Loadout::STARTING },
            stored_j: fitting.stored_j_at(&motion, 20.0),
            start_s: 20.0,
        };
        fitting.begin_refit(order.solve(&b).unwrap());
        let wire = lc_proto::Fitting::from(&fitting);
        let back = Fitting::from(&lc_proto::decode::<lc_proto::Fitting>(&lc_proto::encode(&wire)).unwrap());
        assert_eq!(back, fitting);
    }

    #[test]
    fn a_grant_stops_at_the_capacity() {
        let b = Balance::DEFAULT;
        let motion = ShipState::at(DVec3::ZERO);
        let mut fitting = Fitting::full(Loadout::STARTING, b, 0.0);
        fitting.grant(1.0e30);
        assert_eq!(fitting.stored_j_at(&motion, 0.0), b.capacity_j(&Loadout::STARTING));
    }
}
