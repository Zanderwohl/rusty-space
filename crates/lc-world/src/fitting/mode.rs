//! Clear, Black and Auto: what the field absorbs, and the switch between them. See
//! `lightcone/docs/30-the-field.md` §Clear and Black.
//!
//! A switch is kept in the account until the authority takes it with [`Fitting::take_flip`], which
//! is where the flip becomes an event. Until then every read applies it from `done_s` itself, so
//! where the account is settled never changes what it absorbed.

use super::{Balance, Fitting};
use crate::field::{Field, Mode, Segment, Stretch};
use crate::motion::ShipState;

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
        debug_assert!(Thresholds::of(balance).is_valid(), "a balance's thresholds must leave both gaps open");
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

    /// Toward `to`, done `field_switch_s` from the settlement. Settle first. Refused while any switch
    /// is kept, done or not: overwriting one not yet taken would lose its flip.
    pub fn begin_switch(&mut self, to: Mode) -> Result<(), Switching> {
        let now_s = self.since_s;
        if self.posture.switch.is_some() {
            return Err(Switching);
        }
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

    /// When Auto next begins a switch by `until_s`, and toward which shade, if the inputs in force
    /// hold and the round runs as planned. `None` outside Auto and while a switch is kept.
    pub fn auto_s(&self, motion: &ShipState, until_s: f64) -> Option<(f64, Mode)> {
        let Setting::Auto(thresholds) = self.posture.setting else { return None };
        if self.posture.switch.is_some() {
            return None;
        }
        let field = self.field();
        let max_j = field.heat_max_j();
        let shade = self.posture.shade;
        let mut found = None;
        self.walk(Some(motion), until_s, |from_s, part, heat_j, dt_s| {
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

/// Each of `part`'s stretches, with when it starts and the room storage has then.
fn stretches(field: &Field, part: &Segment, heat_j: f64) -> impl Iterator<Item = (f64, f64, Stretch)> {
    let (mut at_s, mut room_j) = (0.0, part.room_j);
    field.stretches(part, heat_j, f64::INFINITY).into_iter().map(move |s| {
        let start = (at_s, room_j, s);
        at_s += s.dt_s;
        room_j -= s.storage_w * s.dt_s;
        start
    })
}

/// Heat rises to `above_j`, or storage fills.
fn clear_due(field: &Field, part: &Segment, heat_j: f64, above_j: f64, capacity_j: f64) -> Option<f64> {
    for (at_s, room_j, s) in stretches(field, part, heat_j) {
        if room_j <= FULL * capacity_j {
            return Some(at_s);
        }
        let heat = field.time_to_rise_s(s.heat_j, above_j, s.heat_w);
        let fill = (s.storage_w > 0.0).then(|| room_j / s.storage_w);
        if let Some(t) = [heat, fill].into_iter().flatten().reduce(f64::min).filter(|&t| t <= s.dt_s) {
            return Some(at_s + t);
        }
    }
    None
}

/// Heat at or under `below_j` while storage is under `refill_j`. Over a stretch both move
/// monotonically, so each condition holds over one interval touching an end.
fn black_due(field: &Field, part: &Segment, heat_j: f64, below_j: f64, refill_j: f64, capacity_j: f64) -> Option<f64> {
    for (at_s, room_j, s) in stretches(field, part, heat_j) {
        let heat = if s.heat_j <= below_j {
            Holds::Until(field.time_to_rise_s(s.heat_j, below_j, s.heat_w).unwrap_or(f64::INFINITY))
        } else {
            field.time_to_fall_s(s.heat_j, below_j, s.heat_w).map_or(Holds::Never, Holds::From)
        };
        let level_j = capacity_j - room_j.max(0.0);
        let storage = if level_j < refill_j {
            Holds::Until(if s.storage_w > 0.0 { (refill_j - level_j) / s.storage_w } else { f64::INFINITY })
        } else if s.storage_w < 0.0 {
            Holds::From((level_j - refill_j) / -s.storage_w)
        } else {
            Holds::Never
        };
        let due = match (heat, storage) {
            (Holds::Never, _) | (_, Holds::Never) => None,
            (Holds::Until(_), Holds::Until(_)) => Some(0.0),
            (Holds::Until(end), Holds::From(start)) | (Holds::From(start), Holds::Until(end)) => (start <= end).then_some(start),
            (Holds::From(a), Holds::From(b)) => Some(a.max(b)),
        };
        if let Some(t) = due.filter(|&t| t <= s.dt_s) {
            return Some(at_s + t);
        }
    }
    None
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

impl From<&lc_proto::Field> for Posture {
    fn from(f: &lc_proto::Field) -> Self {
        Posture {
            setting: f.mode.into(),
            shade: f.shade.into(),
            switch: f.switch.map(|s| Switch { to: s.to.into(), done_s: s.done_s }),
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use super::*;
    use crate::field::Burst;
    use crate::fitting::{Account, STARTING_BROADSIDE_M2};
    use crate::form::Form;
    use crate::motion::ShipState;
    use crate::solar::{intake_w, SOLAR_CONSTANT_W_M2};
    use crate::system::UNIT_M as AU_M;

    fn rest() -> ShipState {
        ShipState::at(DVec3::ZERO)
    }

    fn close(got: f64, want: f64, rel: f64) -> bool {
        (got - want).abs() <= rel * want.abs()
    }

    fn starlight_w(b: &Balance, d_au: f64) -> f64 {
        let luminosity_w = SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * AU_M * AU_M;
        intake_w(b, STARTING_BROADSIDE_M2, luminosity_w, d_au * AU_M)
    }

    fn ship(b: Balance, stored_j: f64, heat_j: Option<f64>, posture: Posture) -> Fitting {
        let full = Fitting::full(Form::starting(), b, 0.0);
        let heat_j = heat_j.unwrap_or(full.heat_j);
        Fitting::from_account(&Account { stored_j, heat_j, posture, ..full.account() }, b)
    }

    fn auto(b: &Balance, shade: Mode) -> Posture {
        Posture { setting: Setting::Auto(Thresholds::of(b)), shade, switch: None }
    }

    /// As the authority runs it: a switch taken when done, and Auto's next begun when due. Returns
    /// each switch begun, and the heat it began at.
    fn run(f: &mut Fitting, until_s: f64) -> Vec<(f64, Mode, f64)> {
        let mut begun = Vec::new();
        loop {
            if let Some(switch) = f.posture.switch {
                if switch.done_s > until_s {
                    break;
                }
                f.settle(&rest(), switch.done_s);
                assert_eq!(f.take_flip(), Some(switch));
                continue;
            }
            match f.auto_s(&rest(), until_s) {
                Some((at_s, to)) if at_s <= until_s => {
                    f.settle(&rest(), at_s);
                    f.begin_switch(to).unwrap();
                    begun.push((at_s, to, f.heat_j));
                }
                _ => break,
            }
        }
        f.settle(&rest(), until_s);
        begun
    }

    #[test]
    fn a_switch_changes_nothing_until_it_completes() {
        let b = Balance::DEFAULT;
        let mut black = ship(b, 0.5 * Fitting::full(Form::starting(), b, 0.0).hull().capacities.storage_j, None, Posture::BLACK);
        black.set_starlight_w(starlight_w(&b, 0.1));
        let mut clear = black.clone();
        clear.set_setting(Setting::Clear).unwrap();
        let done_s = b.field_switch_s;
        assert_eq!(clear.posture.switch, Some(Switch { to: Mode::Clear, done_s }));
        for t in [0.25 * done_s, 0.999_999 * done_s, done_s] {
            assert_eq!(clear.heat_j_at(&rest(), t), black.heat_j_at(&rest(), t), "{t}");
            assert_eq!(clear.stored_j_at(&rest(), t), black.stored_j_at(&rest(), t), "{t}");
        }
        let later_s = 3.0 * done_s;
        assert!(clear.heat_j_at(&rest(), later_s) < 0.99 * black.heat_j_at(&rest(), later_s), "Clear absorbs less once done");

        let mut ticks = clear.clone();
        for k in 1..=997 {
            ticks.settle(&rest(), later_s * f64::from(k) / 997.0);
        }
        assert!(close(ticks.heat_j, clear.heat_j_at(&rest(), later_s), 1e-9), "{} {}", ticks.heat_j, clear.heat_j_at(&rest(), later_s));
        assert!(close(ticks.stored_j, clear.stored_j_at(&rest(), later_s), 1e-9));
        assert_eq!(ticks.shade_at(later_s), Mode::Clear);
        assert_eq!(ticks.take_flip(), Some(Switch { to: Mode::Clear, done_s }), "kept until taken");
        assert_eq!((ticks.posture.shade, ticks.posture.switch, ticks.take_flip()), (Mode::Clear, None, None));
    }

    #[test]
    fn a_second_switch_meanwhile_is_refused() {
        let b = Balance::DEFAULT;
        let mut f = Fitting::full(Form::starting(), b, 0.0);
        f.set_setting(Setting::Clear).unwrap();
        let running = f.posture;
        assert_eq!(f.set_setting(Setting::Black), Err(Switching));
        assert_eq!(f.set_setting(Setting::Auto(Thresholds::of(&b))), Err(Switching));
        assert_eq!(f.begin_switch(Mode::Black), Err(Switching));
        assert_eq!(f.posture, running, "a refusal changes nothing");
        f.settle(&rest(), 0.5 * b.field_switch_s);
        assert_eq!(f.set_setting(Setting::Black), Err(Switching));
        f.settle(&rest(), b.field_switch_s);
        assert_eq!(f.set_setting(Setting::Black), Err(Switching), "done, but its flip not yet taken");
        assert_eq!(f.begin_switch(Mode::Black), Err(Switching));
        assert!(f.take_flip().is_some());
        assert_eq!(f.set_setting(Setting::Black), Ok(()));
        assert_eq!(f.posture.switch, Some(Switch { to: Mode::Black, done_s: 2.0 * b.field_switch_s }));
    }

    /// Radiation and drain off, storage full: all that is absorbed is heat.
    #[test]
    fn clear_takes_its_fraction_of_starlight_and_a_spike_and_black_all_of_it() {
        let b = Balance { living_density_w: 0.0, field_tau_s: 1.0e40, ..Balance::DEFAULT };
        let arriving_w = starlight_w(&b, 1.0);
        let capacity_j = Fitting::full(Form::starting(), b, 0.0).hull().capacities.storage_j;
        let dt_s = 1.0e4;
        let spike_j = 1.0e24;
        for (shade, fraction) in [(Mode::Clear, b.clear_absorptivity), (Mode::Black, 1.0)] {
            let mut f = ship(b, capacity_j, Some(0.0), Posture { setting: Setting::Black, shade, switch: None });
            f.set_starlight_w(arriving_w);
            assert!(close(f.heat_j_at(&rest(), dt_s), fraction * arriving_w * dt_s, 1e-9), "{shade:?} {}", f.heat_j_at(&rest(), dt_s));
            assert_eq!(Burst::Arriving(spike_j).heat_j(f.absorptivity_at(dt_s)), fraction * spike_j);
        }
        assert_eq!(b.clear_absorptivity, 0.3);
    }

    /// 30 §Auto: it fills Black, turns Clear when full and stays there at about 2 400 K.
    #[test]
    fn auto_at_a_tenth_of_an_au_fills_black_and_holds_clear() {
        let b = Balance::DEFAULT;
        let mut f = ship(b, 0.0, None, auto(&b, Mode::Black));
        f.set_starlight_w(starlight_w(&b, 0.1));
        let caps = f.hull().capacities;
        let fill_s = caps.storage_j / (f.solar_w() - caps.drain_w);
        let (at_s, to) = f.auto_s(&rest(), f64::INFINITY).expect("it fills");
        assert!(close(at_s, fill_s, 1e-9) && to == Mode::Clear, "{at_s} {fill_s} {to:?}");
        assert!(f.heat_j_at(&rest(), at_s) < 0.5 * f.field().heat_max_j(), "premise: storage, not heat");

        let year_s = crate::flight::JULIAN_YEAR_S;
        let begun = run(&mut f, 2.0 * year_s);
        assert_eq!(begun.iter().map(|&(t, to, _)| (t, to)).collect::<Vec<_>>(), vec![(at_s, Mode::Clear)]);
        assert_eq!(f.stored_j, caps.storage_j);
        let k = f.temperature_k_at(&rest(), 2.0 * year_s);
        assert!((k - 2_400.0).abs() < 100.0, "{k}");
    }

    #[test]
    fn a_burst_past_a_threshold_starts_the_switch_at_once() {
        let b = Balance::DEFAULT;
        let max_j = Fitting::full(Form::starting(), b, 0.0).field().heat_max_j();
        let half_j = 0.5 * Fitting::full(Form::starting(), b, 0.0).hull().capacities.storage_j;
        let mut f = ship(b, half_j, Some(0.45 * max_j), auto(&b, Mode::Black));
        f.settle(&rest(), 1.0e5);
        assert_eq!(f.auto_s(&rest(), f64::INFINITY), None, "premise: cooling, far from any star");
        let spike_j = 0.1 * max_j;
        let hit = Fitting::from_account(&Account { heat_j: f.heat_j + Burst::Arriving(spike_j).heat_j(f.absorptivity_at(f.since_s)), ..f.account() }, b);
        assert_eq!(hit.auto_s(&rest(), f64::INFINITY), Some((1.0e5, Mode::Clear)));
    }

    /// Starlight that heats Black past `clear_above` and lets Clear cool under `black_below`, with
    /// storage too slow to fill: Auto cycles between them. Returns the switches begun, `Q_max` and
    /// the switch's length.
    fn cycle(thresholds: Thresholds) -> (Vec<(f64, Mode, f64)>, f64, f64) {
        let b = Balance { conversion_efficiency: 0.01, living_density_w: 0.0, ..Balance::DEFAULT };
        let max_j = Fitting::full(Form::starting(), b, 0.0).field().heat_max_j();
        let half_j = 0.5 * Fitting::full(Form::starting(), b, 0.0).hull().capacities.storage_j;
        let posture = Posture { setting: Setting::Auto(thresholds), shade: Mode::Black, switch: None };
        let mut f = ship(b, half_j, Some(0.1 * max_j), posture);
        f.set_starlight_w(0.8 * f.field().rated_load_w());
        let begun = run(&mut f, 400.0 * 86_400.0);
        assert!(f.stored_j < thresholds.refill_below * f.hull().capacities.storage_j, "premise: storage never fills");
        assert!(begun.len() >= 4, "premise: it cycles: {begun:?}");
        (begun, max_j, b.field_switch_s)
    }

    /// Each shade is held a good while after its switch completes before the next begins.
    fn assert_hysteresis(thresholds: Thresholds) {
        let (begun, _, switch_s) = cycle(thresholds);
        for pair in begun.windows(2) {
            let held_s = pair[1].0 - (pair[0].0 + switch_s);
            assert!(held_s >= 3.0 * switch_s, "chatters: {:?} held {held_s} s", pair[0].1);
        }
    }

    /// And begins each switch as heat crosses its threshold.
    #[test]
    fn auto_holds_each_shade_between_its_thresholds() {
        let thresholds = Thresholds::of(&Balance::DEFAULT);
        assert_hysteresis(thresholds);
        let (begun, max_j, _) = cycle(thresholds);
        for (i, &(at_s, to, heat_j)) in begun.iter().enumerate() {
            assert_eq!(to, if i % 2 == 0 { Mode::Clear } else { Mode::Black }, "alternates");
            let want = match to {
                Mode::Clear => thresholds.clear_above,
                Mode::Black => thresholds.black_below,
            };
            assert!(close(heat_j / max_j, want, 1e-9), "{at_s}: {to:?} at {}", heat_j / max_j);
        }
    }

    #[test]
    #[should_panic(expected = "chatters")]
    fn equal_thresholds_fail_the_hysteresis_test() {
        let equal = Thresholds { clear_above: 0.4, black_below: 0.4, refill_below: 0.95 };
        assert!(!equal.is_valid());
        assert_hysteresis(equal);
    }
}
