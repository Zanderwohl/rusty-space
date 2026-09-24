//! A ship's modules and energy account, mirrored from `lc_world::fitting` and `lc_world::refit`.
//! `Loadout` and the per-module fields leave with S1 and F9 of
//! `lightcone/docs/plans/forms-and-fields.md`.

use serde::{Deserialize, Serialize};

/// Why a refit cannot be done. Mirrors `lc_world::refit::Shortage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shortfall {
    Unbuildable,
    Energy,
    NoDrones,
    CannotBuild(Module),
    CannotDismantle(Module),
}

/// Mirrors `lc_world::fitting::Module`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Module {
    Storage,
    Drone,
    Living,
    Engine,
    Data,
}

/// Module counts and hull slots. Mirrors `lc_world::fitting::Loadout`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loadout {
    pub storage: u32,
    pub drones: u32,
    pub living: u32,
    pub engines: u32,
    pub slots: u32,
    pub data: u32,
}

/// The shard's tunables. Mirrors `lc_world::fitting::Balance`; stated so a client's refit
/// preview uses the numbers the authority does.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    pub drive_efficiency: f64,
    pub recovery: f64,
    pub storage_per_module: f64,
    pub engine_thrust_n: f64,
    pub drone_power_w: f64,
    pub living_drain_w: f64,
    pub hull_density_kg_m3: f64,
    pub slot_volume_m3: f64,
    pub module_density_kg_m3: f64,
    pub solar_efficiency: f64,
    pub solar_gain: f64,
    pub data_per_module: f64,
    pub data_mass_fraction: f64,
    pub data_work_factor: f64,
}

/// A refit as the arguments it is planned from. Mirrors `lc_world::refit::Order`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefitOrder {
    pub from: Loadout,
    pub target: Loadout,
    pub stored_j: f64,
    pub start_s: f64,
}

/// A ship's energy account, settled at `since_s`. Mirrors `lc_world::fitting::Account`, with
/// the balance it is read under.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fitting {
    pub balance: Balance,
    pub loadout: Loadout,
    pub stored_j: f64,
    pub since_s: f64,
    pub rapidity_since: f64,
    pub committed_j: f64,
    /// Starlight being collected in the segment that began at `since_s`, watts.
    pub solar_w: f64,
    pub refit: Option<RefitOrder>,
}
