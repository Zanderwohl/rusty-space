//! A ship's energy account. The form it is kept on is [`crate::form`], converted at its boundary in
//! `lc_world::fitting`.

use serde::{Deserialize, Serialize};

use crate::form::{Form, Part, PartId};

/// Why a round could not be begun toward a target that is itself a valid form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shortfall {
    /// The build phase costs more than the dismantle phase leaves in storage.
    Energy,
    /// This part's step would begin with no drone to do it.
    NoDrones(PartId),
}

/// The shard's tunables, `lc_world::fitting::Balance`; stated so a client's preview uses the
/// numbers the authority does.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    pub drive_efficiency: f64,
    pub recovery: f64,
    pub module_density_kg_m3: f64,
    pub conversion_efficiency: f64,
    pub solar_gain: f64,
    pub data_mass_fraction: f64,
    pub data_work_factor: f64,
    pub storage_density: f64,
    pub drone_density_w: f64,
    pub engine_density_w: f64,
    pub living_density_w: f64,
    pub data_density_b: f64,
    pub bay_mass_fraction: f64,
    pub spar_mass_fraction: f64,
    pub min_part_m3: f64,
    pub min_drone_m3: f64,
    pub spar_gap: f64,
    pub spar_thickness: f64,
    pub move_work_factor: f64,
    pub hull_areal_density: f64,
    pub envelope_margin: f64,
    pub engine_clear_half_angle_rad: f64,
    pub field_idle_k: f64,
    pub field_capacity: f64,
    pub field_tau_s: f64,
    pub clear_absorptivity: f64,
    pub field_switch_s: f64,
    pub auto_clear_above: f64,
    pub auto_black_below: f64,
    pub auto_refill_below: f64,
    pub collapse_spike_fraction: f64,
    pub collapse_spike_k: f64,
    pub collapse_afterglow_s: f64,
    pub drive_spread_rad: f64,
    pub rcs_accel_g: f64,
    pub rcs_spread_rad: f64,
    pub courtesy_fraction: f64,
}

/// A refit as the recipe it was planned from: `lc_world::refit::rounds::Round`, which both ends
/// solve to the same plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Round {
    pub from: Form,
    pub target: Form,
    pub stored_j: f64,
    pub start_s: f64,
}

/// One step of a refit, as `lc_world::refit::rounds::Change`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    Grow,
    Shrink,
    Add,
    Remove,
    Move,
}

/// The refit step another craft had under way as its light left it: an appearance, as the rest of
/// a presence is. Never the round's recipe, which would show the form it becomes before its light
/// does. See `lightcone/docs/08-networking.md` §Seeing other ships.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Building {
    /// Which step of its round, from zero, so a receiver can tell one step from the next.
    pub step: u32,
    pub part: PartId,
    pub change: Change,
    /// The part as the step leaves it, `None` for one taken apart. Its scaffold stands at this size
    /// from the step's start, so this much is visible.
    pub after: Option<Part>,
    /// Through the step, 0 to 1.
    pub fraction: f64,
    /// The whole step, coordinate seconds.
    pub duration_s: f64,
    /// Canceled, and running back toward nothing at the step's pace.
    pub reversing: bool,
}

/// A ship's energy account, settled at `since_s`, with the balance it is read under.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fitting {
    pub balance: Balance,
    /// As of `since_s`, counting the refit's steps finished by then.
    pub form: Form,
    pub stored_j: f64,
    pub since_s: f64,
    pub rapidity_since: f64,
    pub committed_j: f64,
    /// Starlight being collected in the segment that began at `since_s`, watts.
    pub solar_w: f64,
    pub refit: Option<Round>,
}
