//! The field's state, and light a craft puts out or takes in on purpose. See
//! `lightcone/docs/30-the-field.md` and `lightcone/docs/31-directed-energy.md`.

use serde::{Deserialize, Serialize};

/// What the field is set to. Auto's thresholds are fractions of `Q_max`, and `refill_below` of
/// storage capacity.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum FieldMode {
    Clear,
    Black,
    Auto { clear_above: f64, black_below: f64, refill_below: f64 },
}

/// What the field is actually doing, which in Auto is whichever the thermostat last chose.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shade {
    Clear,
    Black,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Switch {
    pub to: Shade,
    /// Ship seconds, on the account's clock.
    pub done_s: f64,
}

/// A ship's own field, settled at `since_s` as the rest of its account is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub heat_j: f64,
    pub since_s: f64,
    pub mode: FieldMode,
    pub shade: Shade,
    pub switch: Option<Switch>,
}

/// Another craft's field, as its light shows it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glow {
    pub temperature_k: f64,
    pub shade: Shade,
}

/// What light an emission is made of: a beam's one wavelength, or a drive face's blackbody.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Spectrum {
    Line { wavelength_m: f64 },
    Blackbody { temperature_k: f64 },
}

/// An emitter seen from inside its beam: its light's flux at the observer, W/m², in its spectrum.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Glare {
    pub spectrum: Spectrum,
    pub flux_w_m2: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Apertures {
    Fore,
    Aft,
    /// No net thrust when the two are rated alike.
    Both,
}
