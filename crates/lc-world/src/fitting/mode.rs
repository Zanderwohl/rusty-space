//! Clear, Black and Auto: what the field absorbs, and the switch between them. See
//! `lightcone/docs/30-the-field.md` §Clear and Black.
//!
//! A switch is kept in the account until the authority takes it with [`Fitting::take_flip`], which
//! is where the flip becomes an event. Until then every read applies it from `done_s` itself, so
//! where the account is settled never changes what it absorbed.

use super::{Balance, Fitting};
use crate::field::{Field, Mode, Segment};

/// What the player chose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setting {
    Clear,
    Black,
    Auto(Thresholds),
}

/// Heat as fractions of `Q_max`, and `refill_below` of storage capacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub clear_above: f64,
    pub black_below: f64,
    pub refill_below: f64,
}

impl Thresholds {
    pub fn of(balance: &Balance) -> Self {
        Self {
            clear_above: balance.auto_clear_above,
            black_below: balance.auto_black_below,
            refill_below: balance.auto_refill_below,
        }
    }

    /// Both gaps open. Equal thresholds would switch back as soon as a switch completed.
    pub fn is_valid(&self) -> bool {
        0.0 < self.black_below
            && self.black_below < self.clear_above
            && self.clear_above <= 1.0
            && 0.0 < self.refill_below
            && self.refill_below < 1.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Switch {
    pub to: Mode,
    /// Coordinate seconds.
    pub done_s: f64,
}

/// A field's mode, the shade it is in and any switch not yet taken.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Posture {
    pub setting: Setting,
    /// Before [`switch`](Self::switch), if there is one.
    pub shade: Mode,
    pub switch: Option<Switch>,
}

impl Posture {
    pub const BLACK: Posture = Posture { setting: Setting::Black, shade: Mode::Black, switch: None };

    /// A new ship's: Auto, in the shade Auto keeps a full store in.
    pub fn new_ship(balance: &Balance) -> Self {
        Posture { setting: Setting::Auto(Thresholds::of(balance)), shade: Mode::Clear, switch: None }
    }

    pub fn shade_at(&self, t: f64) -> Mode {
        match self.switch {
            Some(switch) if switch.done_s <= t => switch.to,
            _ => self.shade,
        }
    }

    /// Under way at `t`: begun and not yet done.
    pub fn switching_at(&self, t: f64) -> Option<Switch> {
        self.switch.filter(|s| s.done_s > t)
    }
}

/// Refused: another switch is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switching;

impl Mode {
    pub fn other(self) -> Mode {
        match self {
            Mode::Clear => Mode::Black,
            Mode::Black => Mode::Clear,
        }
    }
}

/// Storage within this of capacity is full: settling to a fill lands there only to rounding.
const FULL: f64 = 1.0e-9;

impl Fitting {
    pub fn posture(&self) -> &Posture {
        &self.posture
    }

    /// Replace it whole, as a new ship or a test does. Settle first.
    pub fn set_posture(&mut self, posture: Posture) {
        self.posture = posture;
    }

    pub fn shade_at(&self, t: f64) -> Mode {
        self.posture.shade_at(t)
    }

    /// Of what arrives at `t`, the fraction the field absorbs: starlight, a beam, exhaust or a spike.
    pub fn absorptivity_at(&self, t: f64) -> f64 {
        self.shade_at(t).absorptivity(self.balance.clear_absorptivity)
    }

    /// The player's order. Clear or Black begins a switch unless already in that shade; Auto leaves
    /// the next switch to [`Fitting::auto_s`]. Settle first.
    pub fn set_setting(&mut self, setting: Setting) -> Result<(), Switching> {
        let now_s = self.since_s;
        if self.posture.switching_at(now_s).is_some() {
            return Err(Switching);
        }
        let to = match setting {
            Setting::Clear => Some(Mode::Clear),
            Setting::Black => Some(Mode::Black),
            Setting::Auto(_) => None,
        };
        if let Some(to) = to.filter(|&to| to != self.shade_at(now_s)) {
            self.begin_switch(to)?;
        }
        self.posture.setting = setting;
        Ok(())
    }

    /// Toward `to`, done `field_switch_s` from the settlement. Settle first.
    pub fn begin_switch(&mut self, to: Mode) -> Result<(), Switching> {
        let now_s = self.since_s;
        if self.posture.switching_at(now_s).is_some() {
            return Err(Switching);
        }
        self.posture.shade = self.shade_at(now_s);
        self.posture.switch = Some(Switch { to, done_s: now_s + self.balance.field_switch_s });
        Ok(())
    }

    /// A switch done by the settlement, folded into the shade. Only what takes it states the flip.
    pub fn take_flip(&mut self) -> Option<Switch> {
        let switch = self.posture.switch.filter(|s| s.done_s <= self.since_s)?;
        self.posture.shade = switch.to;
        self.posture.switch = None;
        Some(switch)
    }

    /// When Auto next begins a switch, and toward which shade, if the inputs in force hold and the
    /// round runs as planned. `None` outside Auto and while a switch is kept. Reads no burn, as
    /// [`Fitting::heat_j_at`] does.
    pub fn auto_s(&self) -> Option<(f64, Mode)> {
        let Setting::Auto(thresholds) = self.posture.setting else { return None };
        if self.posture.switch.is_some() {
            return None;
        }
        let field = self.field();
        let max_j = field.heat_max_j();
        let shade = self.posture.shade;
        let mut found = None;
        self.walk(None, f64::INFINITY, |from_s, part, heat_j, dt_s| {
            let capacity_j = self.capacities_at(from_s).storage_j;
            let due = match shade {
                Mode::Black => clear_due(&field, part, heat_j, thresholds.clear_above * max_j, capacity_j),
                Mode::Clear => black_due(&field, part, heat_j, thresholds.black_below * max_j, thresholds.refill_below * capacity_j, capacity_j),
            };
            found = due.filter(|&t| t <= dt_s).map(|t| from_s + t);
            found.is_some()
        });
        found.map(|at_s| (at_s, shade.other()))
    }
}

/// Heat rises to `above_j`, or storage fills.
fn clear_due(field: &Field, part: &Segment, heat_j: f64, above_j: f64, capacity_j: f64) -> Option<f64> {
    if part.room_j <= FULL * capacity_j {
        return Some(0.0);
    }
    let heat = field.segment_time_to_rise_s(part, heat_j, above_j);
    [heat, part.fill_s()].into_iter().flatten().reduce(f64::min)
}

/// Heat at or under `below_j` while storage is under `refill_j`. Once storage fills it is not, so
/// only the stretch before the fill is searched, and over it both heat and storage are monotonic:
/// each condition holds over one interval touching an end.
fn black_due(field: &Field, part: &Segment, heat_j: f64, below_j: f64, refill_j: f64, capacity_j: f64) -> Option<f64> {
    let power_w = field.heat_filling_w(part);
    let heat = if heat_j <= below_j {
        Holds::Until(field.time_to_rise_s(heat_j, below_j, power_w).unwrap_or(f64::INFINITY))
    } else {
        field.time_to_fall_s(heat_j, below_j, power_w).map_or(Holds::Never, Holds::From)
    };
    let level_j = capacity_j - part.room_j.max(0.0);
    let net_w = part.stored_w() - part.draw_w;
    let storage = if level_j < refill_j {
        Holds::Until(if net_w > 0.0 { (refill_j - level_j) / net_w } else { f64::INFINITY })
    } else if net_w < 0.0 {
        Holds::From((level_j - refill_j) / -net_w)
    } else {
        Holds::Never
    };
    let due = match (heat, storage) {
        (Holds::Never, _) | (_, Holds::Never) => None,
        (Holds::Until(_), Holds::Until(_)) => Some(0.0),
        (Holds::Until(end), Holds::From(start)) | (Holds::From(start), Holds::Until(end)) => (start <= end).then_some(start),
        (Holds::From(a), Holds::From(b)) => Some(a.max(b)),
    };
    due.filter(|&t| part.fill_s().is_none_or(|fill_s| t < fill_s))
}

/// Where a condition holds from the start of a stretch.
enum Holds {
    Until(f64),
    From(f64),
    Never,
}

impl From<Mode> for lc_proto::Shade {
    fn from(m: Mode) -> Self {
        match m {
            Mode::Clear => lc_proto::Shade::Clear,
            Mode::Black => lc_proto::Shade::Black,
        }
    }
}

impl From<lc_proto::Shade> for Mode {
    fn from(s: lc_proto::Shade) -> Self {
        match s {
            lc_proto::Shade::Clear => Mode::Clear,
            lc_proto::Shade::Black => Mode::Black,
        }
    }
}

impl From<Setting> for lc_proto::FieldMode {
    fn from(s: Setting) -> Self {
        match s {
            Setting::Clear => lc_proto::FieldMode::Clear,
            Setting::Black => lc_proto::FieldMode::Black,
            Setting::Auto(t) => lc_proto::FieldMode::Auto {
                clear_above: t.clear_above,
                black_below: t.black_below,
                refill_below: t.refill_below,
            },
        }
    }
}

impl From<lc_proto::FieldMode> for Setting {
    fn from(m: lc_proto::FieldMode) -> Self {
        match m {
            lc_proto::FieldMode::Clear => Setting::Clear,
            lc_proto::FieldMode::Black => Setting::Black,
            lc_proto::FieldMode::Auto { clear_above, black_below, refill_below } => {
                Setting::Auto(Thresholds { clear_above, black_below, refill_below })
            }
        }
    }
}

impl From<&Posture> for (lc_proto::FieldMode, lc_proto::Shade, Option<lc_proto::Switch>) {
    fn from(p: &Posture) -> Self {
        let switch = p.switch.map(|s| lc_proto::Switch { to: s.to.into(), done_s: s.done_s });
        (p.setting.into(), p.shade.into(), switch)
    }
}

impl From<&lc_proto::Field> for Posture {
    fn from(f: &lc_proto::Field) -> Self {
        Posture {
            setting: f.mode.into(),
            shade: f.shade.into(),
            switch: f.switch.map(|s| Switch { to: s.to.into(), done_s: s.done_s }),
        }
    }
}
