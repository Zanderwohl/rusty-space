//! The always-visible readout.
//!
//! The game is about looking at the past, so how stale the picture is belongs on screen at
//! all times rather than in a panel a player can close.

use em_spectra::presets;
use lc_world::craft::Craft;
use lc_world::field::Mode;
use lc_world::fitting::{Fitting, Setting, Thresholds};
use lc_world::motion::Motive;

use crate::action::Action;
use crate::session::Session;
use crate::ui::{Panel, UiState, ViewMode};

const YEAR_S: f64 = 31_557_600.0;

/// Lines the head-up display shows. Strings, so a test can read them.
#[derive(Clone, Debug, PartialEq)]
pub struct Hud {
    /// Coordinate time.
    pub clock: String,
    /// The ship's own clock, `T'`. Behind [`Hud::clock`] by whatever the ship has flown.
    pub ship_clock: String,
    /// The selected target and how old its light is, if anything is selected.
    pub target: Option<String>,
    /// Band mapping in force.
    pub mapping: String,
    /// [`Hud::mapping`] as one or two letters, for a bar with no room for the name.
    pub mapping_code: &'static str,
    /// Exposure relative to automatic.
    pub exposure: String,
    /// [`Hud::exposure`] without its unit.
    pub exposure_code: String,
    /// Set while a crossing is under way.
    pub flight: Option<String>,
    /// Set while the ship is falling rather than flying.
    pub coasting: Option<String>,
    /// Set when the clock is running at something other than the canonical rate.
    pub warning: Option<String>,
    /// Stored energy, for a ship with modules.
    pub energy: Option<Energy>,
    pub field: Option<Field>,
}

impl Hud {
    pub fn band(&self, fit: Fit) -> String {
        match fit {
            Fit::Words => format!("BAND {}", self.mapping),
            Fit::Codes | Fit::Keys | Fit::Bare => format!("B {}", self.mapping_code),
        }
    }

    pub fn exposure(&self, fit: Fit) -> String {
        match fit {
            Fit::Words => format!("EXPOSURE {}", self.exposure),
            Fit::Codes | Fit::Keys | Fit::Bare => format!("E {}", self.exposure_code),
        }
    }

    /// The numbers beside the energy bar, or `None` where the bar is shown alone.
    pub fn energy_amount(&self, fit: Fit) -> Option<&str> {
        self.energy.as_ref().filter(|_| fit < Fit::Bare).map(|e| e.amount.as_str())
    }

    /// At [`Fit::Bare`], only a countdown.
    pub fn field_text(&self, fit: Fit) -> Option<&str> {
        let field = self.field.as_ref()?;
        if fit < Fit::Bare { Some(&field.text) } else { field.countdown.as_deref() }
    }

    pub fn warning(&self, fit: Fit) -> Option<String> {
        let words = self.warning.as_deref()?;
        Some(if fit == Fit::Bare { rate_code(words) } else { words.to_string() })
    }
}

/// A rate in unit symbols: "15 minutes / second" is "15 min/s". A rate off the design one is
/// never dropped from the bar, only shortened.
fn rate_code(label: &str) -> String {
    let symbol = |word| match word {
        "year" | "years" => "yr",
        "day" | "days" => "d",
        "hour" | "hours" => "h",
        "minute" | "minutes" => "min",
        "second" | "seconds" => "s",
        word => word,
    };
    label
        .split(" / ")
        .map(|side| side.split(' ').map(symbol).collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("/")
}

/// The connection note at this fit. [`Fit::Bare`] keeps the state and drops who, how fast and
/// why, which the bar shows on hover.
pub fn link(
    state: &crate::uplink::State,
    round_trip_s: Option<f64>,
    fit: Fit,
) -> Option<(crate::uplink::Note, String)> {
    use crate::uplink::State;
    let (note, words) = crate::uplink::note(state, round_trip_s)?;
    if fit < Fit::Bare {
        return Some((note, words));
    }
    let state = match state {
        State::Offline => return None,
        State::Connecting => "CONNECTING",
        State::Joined(_) => "LINKED",
        State::Refused(_) => "REFUSED",
        State::Lost(_) => "LINK LOST",
    };
    Some((note, state.to_string()))
}

/// How much of the top bar's wording fits across the window.
///
/// The window buttons are laid out right to left with no check against the readout, so a bar
/// that asks for more than the window has draws one over the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fit {
    Words,
    /// The band and exposure as codes.
    Codes,
    /// Codes, and each window button as the key that toggles it.
    Keys,
    /// Keys, the energy and field bars without their numbers, and the rate and link notes shortened.
    Bare,
}

impl Fit {
    pub const WIDEST_FIRST: [Fit; 4] = [Fit::Words, Fit::Codes, Fit::Keys, Fit::Bare];
}

/// Codes for [`presets::all`], by name so that reordering the list cannot shift them.
fn mapping_code(name: &str) -> &'static str {
    match name {
        "natural" => "N",
        "deep natural" => "DN",
        "thermal" => "TH",
        "dust penetration" => "DP",
        "composition" => "CO",
        "survey" => "SV",
        _ => "?",
    }
}

/// The energy readout: a bar, and the numbers beside it.
#[derive(Clone, Debug, PartialEq)]
pub struct Energy {
    /// Stored over capacity, `[0, 1]`.
    pub fraction: f32,
    /// `23.4 / 30.0 ME`, and what is spoken for: a plan's commitment or a refit under way.
    pub amount: String,
}

/// Where hot things start to glow visibly, K.
pub const DRAPER_K: f64 = 798.0;

/// Past [`DRAPER_K`], over which the bar's blue gives way to the blackbody, K.
const GLOW_BLEND_K: f64 = 200.0;

/// Of the rated load, net heat flow too small to call a direction.
const STEADY: f64 = 1.0e-9;

/// Of `Q_max`, past which the bar pulses, as the field shader flickers.
pub const PULSE_FILL: f64 = 0.8;

/// Coordinate seconds: a quarter of an hour at the design rate.
const COUNTDOWN_HORIZON_S: f64 = 90.0 * 86_400.0;

/// See `lightcone/docs/30-the-field.md` §The field bar.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// `Q / Q_max`, `[0, 1]`.
    pub fraction: f32,
    /// Where heat is heading, `P_in τ / Q_max`, pinned at 1 past the limit.
    pub heading: f32,
    pub kelvin: f64,
    /// sRGB, at full brightness, through the band mapping in force, as the field shader colors the ship.
    pub blackbody: [f32; 3],
    /// How far from [`PULSE_FILL`] to the limit, `[0, 1]`.
    pub stress: f32,
    /// `3 240 K +1.20 ME/yr — collapse in 4:10`. No arrows: the default fonts have none.
    pub text: String,
    pub countdown: Option<String>,
    pub setting: Setting,
    /// What the Auto button asks for: the thresholds in force, or the balance's.
    pub auto: Thresholds,
    /// What the field is in, which its button underlines: in Auto, the thermostat's choice.
    pub shade: Mode,
    /// `» BLACK` while a switch is under way, which the shard refuses another during.
    pub switch: Option<String>,
}

impl Field {
    /// sRGB in and out.
    pub fn color(&self, blue: [f32; 3]) -> [f32; 3] {
        let glow = ((self.kelvin - DRAPER_K) / GLOW_BLEND_K).clamp(0.0, 1.0) as f32;
        std::array::from_fn(|c| (1.0 - glow) * blue[c] + glow * self.blackbody[c])
    }

    /// Of full brightness, at `t_s` real seconds.
    pub fn brightness(&self, t_s: f64) -> f32 {
        if self.stress <= 0.0 {
            return 1.0;
        }
        let hz = 1.0 + 3.0 * f64::from(self.stress).powi(2);
        let dip = 0.5 - 0.5 * (std::f64::consts::TAU * hz * t_s).cos();
        1.0 - 0.45 * self.stress * dip as f32
    }
}

/// Auto's thresholds, as markers on the field bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    ClearAbove,
    BlackBelow,
}

/// The narrowest gap a drag leaves between Auto's markers, and between a marker and either end.
const MARKER_GAP: f64 = 0.02;

impl Marker {
    pub fn of(self, t: &Thresholds) -> f64 {
        match self {
            Marker::ClearAbove => t.clear_above,
            Marker::BlackBelow => t.black_below,
        }
    }

    /// To a hundredth, leaving both gaps open. Unchanged where there is no room.
    pub fn moved(self, t: Thresholds, fraction: f64) -> Thresholds {
        let (lo, hi) = match self {
            Marker::ClearAbove => (t.black_below + MARKER_GAP, 1.0 - MARKER_GAP),
            Marker::BlackBelow => (MARKER_GAP, t.clear_above - MARKER_GAP),
        };
        if lo > hi {
            return t;
        }
        // Rounded after the clamp: at most half a hundredth off a bound, so a gap stays open.
        let at = (fraction.clamp(lo, hi) * 100.0).round() / 100.0;
        match self {
            Marker::ClearAbove => Thresholds { clear_above: at, ..t },
            Marker::BlackBelow => Thresholds { black_below: at, ..t },
        }
    }
}

/// `None` outside Auto, or where the drop changes nothing.
pub fn drop_marker(field: &Field, marker: Marker, fraction: f64) -> Option<Action> {
    let Setting::Auto(t) = field.setting else { return None };
    let moved = marker.moved(t, fraction);
    (moved != t).then(|| Action::SetField(Setting::Auto(moved).into()))
}

/// How long a dropped marker waits for the shard's answer before going back, real seconds.
const AWAITING_S: f64 = 5.0;

/// Ordered and not yet in the account: drawn instead, so the marker does not jump back while the
/// order is in flight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dropped {
    pub thresholds: Thresholds,
    /// Real seconds.
    pub at_s: f64,
}

impl Dropped {
    /// A refusal shows as an overdue answer, which puts the marker back.
    pub fn holds(&self, field: &Field, now_s: f64) -> bool {
        matches!(field.setting, Setting::Auto(t) if t != self.thresholds) && now_s - self.at_s < AWAITING_S
    }
}

/// [`lc_world::ahead::collapse_by`], which clones the craft and settles it a day at a time, so it is
/// solved again only when the account is restated or the clock enters another segment. A refit
/// settling every frame is neither.
#[derive(Default)]
pub struct Collapse {
    key: Option<Key>,
    at_s: Option<f64>,
    solves: u32,
}

/// Heat and storage are read at the segment's end, where settling along the way does not move them.
#[derive(Clone, Debug)]
struct Key {
    segment_end_s: f64,
    motive: Motive,
    form: lc_world::form::Form,
    posture: lc_world::fitting::Posture,
    refit: Option<lc_world::refit::rounds::Plan>,
    lit_w: f64,
    heat_j: f64,
    stored_j: f64,
}

/// Leap and steps agree to about a part in a billion; a restated account differs by far more.
const SAME_ACCOUNT: f64 = 1.0e-6;

impl Key {
    fn of(ship: &Craft, fitting: &Fitting, segment_end_s: f64) -> Key {
        Key {
            segment_end_s,
            motive: ship.motion.motive.clone(),
            form: fitting.form().clone(),
            posture: *fitting.posture(),
            refit: fitting.refit().cloned(),
            lit_w: fitting.lit_w(),
            heat_j: fitting.heat_j_at(&ship.motion, segment_end_s),
            stored_j: fitting.stored_j_at(&ship.motion, segment_end_s),
        }
    }

    fn holds(&self, ship: &Craft, fitting: &Fitting, segment_end_s: f64) -> bool {
        let near = |a: f64, b: f64| (a - b).abs() <= SAME_ACCOUNT * a.abs().max(b.abs());
        self.segment_end_s == segment_end_s
            && self.motive == ship.motion.motive
            && &self.form == fitting.form()
            && &self.posture == fitting.posture()
            && self.refit.as_ref() == fitting.refit()
            && self.lit_w == fitting.lit_w()
            && near(self.heat_j, fitting.heat_j_at(&ship.motion, segment_end_s))
            && near(self.stored_j, fitting.stored_j_at(&ship.motion, segment_end_s))
    }
}

impl Collapse {
    pub fn of(&mut self, ship: &Craft, now_s: f64) -> Option<f64> {
        let fitting = ship.fitting()?;
        let segment_end_s = lc_world::solar::segment_end(now_s);
        if !self.key.as_ref().is_some_and(|key| key.holds(ship, fitting, segment_end_s)) {
            self.at_s = lc_world::ahead::collapse_by(ship, segment_end_s + COUNTDOWN_HORIZON_S);
            self.key = Some(Key::of(ship, fitting, segment_end_s));
            self.solves += 1;
        }
        self.at_s
    }
}

/// A button on the right of the top bar: the way to find a window without knowing its key.
#[derive(Clone, Debug, PartialEq)]
pub struct Toggle {
    pub label: &'static str,
    /// Byte offset of the letter to underline: the key that does the same thing.
    pub underline: Option<usize>,
    pub action: Action,
    pub on: bool,
    /// What the button reads at [`Fit::Keys`]: its key, or a code for a button without one.
    pub key: String,
}

impl Toggle {
    pub fn text(&self, fit: Fit) -> &str {
        match fit {
            Fit::Keys | Fit::Bare => &self.key,
            Fit::Words | Fit::Codes => self.label,
        }
    }
}

pub fn toggles(ui: &UiState) -> Vec<Toggle> {
    let panel = |label, p: Panel| (label, Action::TogglePanel(p), ui.is_open(p), None);
    [
        panel("Flight", Panel::Flight),
        panel("Telescope", Panel::Telescope),
        panel("System", Panel::System),
        ("Map", Action::ToggleView(ViewMode::Map), ui.view == ViewMode::Map, None),
        ("Ship", Action::ToggleView(ViewMode::Form), ui.view == ViewMode::Form, None),
        panel("Refit", Panel::Refit),
        panel("Comms", Panel::Chat),
        panel("Emit", Panel::Emit),
        panel("Bookshelf", Panel::Reader),
        ("Slideshow", Action::ToggleBeautyShots, ui.beauty_shots, Some("SH")),
    ]
    .into_iter()
    .map(|(label, action, on, keyless): (_, _, _, Option<&str>)| {
        let letter = crate::input::letter_for(&action);
        Toggle {
            label,
            underline: letter.and_then(|c| {
                label.char_indices().find(|(_, l)| l.eq_ignore_ascii_case(&c)).map(|(i, _)| i)
            }),
            key: match letter {
                Some(c) => c.to_ascii_uppercase().to_string(),
                None => keyless.unwrap_or(label).to_string(),
            },
            action,
            on,
        }
    })
    .collect()
}

/// What an arc reads as: the two apsides, or the periapsis alone on an escape.
///
/// Apsides rather than elements. Nobody looks at an eccentricity and knows whether they are
/// about to hit the planet.
/// `primary` is what this ship calls the body, not its key.
pub fn arc(coast: &crate::coast::Coast, primary: &str) -> String {
    let near = span(coast.periapsis_m());
    match coast.apoapsis_m() {
        Some(far) => format!("{near} by {} about {primary}", span(far)),
        None => format!("escaping {primary} past {near}"),
    }
}

/// A distance in whatever unit makes it readable.
fn span(meters: f64) -> String {
    match meters {
        m if m < 1.0e6 => format!("{:.0} km", m / 1.0e3),
        m if m < 1.0e9 => format!("{:.0} thousand km", m / 1.0e6),
        m if m < 1.0e11 => format!("{:.2} million km", m / 1.0e9),
        m => format!("{:.2} AU", m / 1.495_978_707e11),
    }
}

/// What a standing intercept reads as, in the place a crossing's progress would be: who, whether
/// the approach is still being flown, how much space there is between the two hulls, and the
/// closeness and approach the server says it is flying.
///
/// No percentage, because there is nothing to be a percentage of — a pursuit re-plans whenever
/// its quarry does something new. And between the hulls rather than between centers, because
/// that is what a closeness is set in; see `lc_world::pursuit::Closeness`.
pub fn pursuit(
    session: &Session,
    pursuit: lc_proto::Pursuit,
    quarry: Option<&crate::uplink::Contact>,
) -> String {
    let doing = match session.ship.motion.still_closing(session.coordinate_time_s()) {
        true => "closing on",
        false => "alongside",
    };
    let near_or_far = match pursuit.closeness {
        lc_proto::Closeness::Company => "in company",
        lc_proto::Closeness::Intimate => "close in",
    };
    let manner = match pursuit.approach {
        lc_proto::Approach::Courteous => "courteous",
        lc_proto::Approach::Direct => "direct",
    };
    let how = format!("{near_or_far}, {manner}");
    let Some(quarry) = quarry else {
        return format!("{doing} ship {} — {how}", pursuit.quarry.0);
    };
    let centers_m =
        session.ship.motion.position_ly.distance(quarry.position_ly) * crate::system::M_PER_LY;
    let clear_m = (centers_m - 0.5 * (session.ship.length_m + quarry.length_m)).max(0.0);
    format!("{doing} {} — {} between hulls — {how}", quarry.name, near(clear_m))
}

/// A short distance, finely enough to see a kilometer-and-a-quarter wander.
fn near(meters: f64) -> String {
    match meters {
        m if m < 1.0e3 => format!("{m:.0} m"),
        m if m < 1.0e5 => format!("{:.1} km", m / 1.0e3),
        m => span(m),
    }
}

pub fn lines(session: &Session, ui: &UiState, collapse: &mut Collapse) -> Hud {
    let name = presets::all().get(ui.preset).map(|(n, _)| *n).unwrap_or("custom");
    Hud {
        clock: format!("T + {:.2} years", session.coordinate_time_s() / YEAR_S),
        ship_clock: format!("T' + {:.2} years", session.ship.motion.clock_s / YEAR_S),
        // What this ship believes, never the catalog: a click on any light in the sky is not
        // a range to it.
        target: ui.selected.map(|id| {
            let range = crate::range::short(session.knowledge.belief(id), session.ship.motion.position_ly);
            format!("{} — {range}", session.name_of(id))
        }),
        mapping: name.to_uppercase(),
        mapping_code: mapping_code(name),
        exposure: match ui.exposure_offset {
            None => "auto".to_string(),
            Some(o) => format!("{o:+.1} stops"),
        },
        exposure_code: match ui.exposure_offset {
            None => "auto".to_string(),
            Some(o) => format!("{o:+.1}"),
        },
        flight: session.cruise().as_ref().map(|c| {
            let left = (c.duration_s() - (session.coordinate_time_s() - c.start_s)).max(0.0);
            format!(
                "{:?} — {:.0}% — {:.4}c — {:.2} years to go",
                c.at(session.coordinate_time_s()).phase,
                c.progress(session.coordinate_time_s()) * 100.0,
                session.ship.motion.beta.length(),
                left / YEAR_S,
            )
        }),
        coasting: session.coast().map(|coast| arc(&coast, &session.body_label(&coast.primary))),
        // Anything but the design rate is said on screen rather than left to look normal —
        // whether the player set it offline or a shard staging a scene stated it. The clock
        // running sixty times over is exactly when a readout of how fast earns its place.
        warning: (ui.time_rate != 1.0).then(|| crate::ui::rate_label(ui.time_rate)),
        energy: energy(session),
        field: field(session, ui.time_rate, collapse.of(&session.ship, session.coordinate_time_s())),
    }
}

fn energy(session: &Session) -> Option<Energy> {
    let now = session.coordinate_time_s();
    let ship = &session.ship;
    let fitting = ship.fitting()?;
    let module_j = fitting.balance().module_energy_j();
    let (stored, capacity) = (fitting.stored_j_at(&ship.motion, now), fitting.capacity_j_at(now));
    let mut line = format!("{:.1} / {:.1} ME", stored / module_j, capacity / module_j);
    if fitting.solar_w() > 0.0 {
        let net = fitting.solar_w() - fitting.capacities_at(now).drain_w;
        line += &format!(" {}", crate::refit_panel::me_per_year(net, module_j));
    }
    let committed = fitting.committed_j_at(&ship.motion, now);
    if committed > 0.0 {
        line += &format!(" ({:.2} committed)", committed / module_j);
    }
    if ship.is_refitting(now) {
        line += " — REFITTING";
    }
    let fraction = if capacity > 0.0 { (stored / capacity).clamp(0.0, 1.0) as f32 } else { 0.0 };
    Some(Energy { fraction, amount: line })
}

/// `rate` is the clock multiplier, for a countdown in real time.
fn field(session: &Session, rate: f64, collapse_s: Option<f64>) -> Option<Field> {
    let now = session.coordinate_time_s();
    let ship = &session.ship;
    let fitting = ship.fitting()?;
    let field = fitting.field();
    let max_j = field.heat_max_j();
    let heat_j = fitting.heat_j_at(&ship.motion, now);
    let heat_w = fitting.heat_w_at(&ship.motion, now);
    let held = session.held_field.map(|h| (h.kelvin, crate::field::fill_at(h.kelvin, fitting.balance())));
    let (kelvin, fraction) = held.unwrap_or((field.temperature_k(heat_j), heat_j / max_j));
    let module_j = fitting.balance().module_energy_j();
    let net_w = heat_w - heat_j / field.tau_s;
    let flow = match net_w {
        w if w.abs() < STEADY * field.rated_load_w() => "steady".to_string(),
        w => crate::refit_panel::me_per_year(w, module_j),
    };
    let due = collapse_s.map(|at_s| format!("collapse in {}", countdown(at_s - now, rate)));
    let mut text = format!("{} K {flow}", grouped(kelvin));
    if let Some(due) = &due {
        text += &format!(" — {due}");
    }
    let posture = fitting.posture();
    let switch = posture.switching_at(now);
    let fraction = fraction.clamp(0.0, 1.0);
    Some(Field {
        fraction: fraction as f32,
        heading: (field.equilibrium_j(heat_w) / max_j).clamp(0.0, 1.0) as f32,
        kelvin,
        blackbody: glow(&session.mapping, kelvin),
        stress: ((fraction - PULSE_FILL) / (1.0 - PULSE_FILL)).clamp(0.0, 1.0) as f32,
        text,
        countdown: due,
        setting: posture.setting,
        auto: match posture.setting {
            Setting::Auto(t) => t,
            _ => Thresholds::of(fitting.balance()),
        },
        shade: posture.shade_at(now),
        switch: switch.map(|s| format!("» {}", shade_name(s.to))),
    })
}

fn shade_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Clear => "CLEAR",
        Mode::Black => "BLACK",
    }
}

/// sRGB, brightest channel at one. Black where the mapping sees none of it.
pub(crate) fn glow(mapping: &em_spectra::BandMapping, kelvin: f64) -> [f32; 3] {
    let linear = crate::field::color_linear(mapping, kelvin);
    let peak = linear.into_iter().fold(0.0, f64::max);
    if !(peak > 0.0 && peak.is_finite()) {
        return [0.0; 3];
    }
    let [r, g, b] = linear.map(|c| (c / peak) as f32);
    let srgb = bevy::color::Color::linear_rgb(r, g, b).to_srgba();
    [srgb.red, srgb.green, srgb.blue]
}

/// Thousands set off by a space: `3 240`.
fn grouped(value: f64) -> String {
    let digits = format!("{:.0}", value.max(0.0));
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// `4:10` of real time, or days on a stopped clock.
fn countdown(left_s: f64, rate: f64) -> String {
    let left_s = left_s.max(0.0);
    if rate <= 0.0 {
        return format!("{:.1} days", left_s / 86_400.0);
    }
    let real = (left_s / (crate::session::TIME_RATE * rate)).ceil() as u64;
    let (h, m, s) = (real / 3600, real / 60 % 60, real % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;
    use crate::action::{Action, apply};

    fn fixture() -> (UiState, Session) {
        let mut session = Session::new(&AuthoredStars::sample(), 3);
        // Charted, because a ship leaves port with charts and most of these tests are about
        // something else. `session::tests` is where an unsurveyed sky is the subject.
        session.issue_charts(30.0);
        (UiState::default(), session)
    }

    #[test]
    fn each_toggle_underlines_its_key_and_the_slideshow_underlines_nothing() {
        let (mut ui, _) = fixture();
        let underlined = |ui: &UiState| {
            toggles(ui)
                .into_iter()
                .map(|t| (t.label, t.underline.map(|i| &t.label[i..i + 1]), t.on))
                .collect::<Vec<_>>()
        };
        let before = underlined(&ui);
        assert!(before.contains(&("Telescope", Some("T"), false)));
        assert!(before.contains(&("System", Some("y"), false)), "the key's letter, not the first");
        assert!(before.contains(&("Bookshelf", Some("B"), false)));
        assert!(before.contains(&("Comms", Some("C"), false)));
        assert!(before.contains(&("Slideshow", None, false)));

        apply(Action::TogglePanel(Panel::Telescope), &mut ui, &mut fixture().1);
        assert!(underlined(&ui).contains(&("Telescope", Some("T"), true)));
    }

    #[test]
    fn at_keys_each_button_reads_as_its_underlined_letter_and_no_two_read_alike() {
        let (ui, _) = fixture();
        let toggles = toggles(&ui);
        for t in &toggles {
            match t.underline {
                Some(i) => assert!(t.key.eq_ignore_ascii_case(&t.label[i..i + 1]), "{}", t.label),
                None => assert!(t.key.len() <= 2, "{} reads {}", t.label, t.key),
            }
        }
        let slideshow = toggles.iter().find(|t| t.label == "Slideshow").unwrap();
        assert_eq!(slideshow.text(Fit::Keys), "SH");
        assert_eq!(slideshow.text(Fit::Codes), "Slideshow");
        let mut keys: Vec<_> = toggles.iter().map(|t| t.key.as_str()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), toggles.len());
    }

    #[test]
    fn every_band_preset_has_its_own_code() {
        let mut codes: Vec<_> = presets::all().iter().map(|(name, _)| mapping_code(name)).collect();
        assert!(!codes.contains(&"?"), "a preset without a code: {codes:?}");
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), presets::all().len());
    }

    #[test]
    fn codes_shorten_the_band_and_exposure_but_keep_their_values() {
        let (mut ui, session) = fixture();
        let hud = lines(&session, &ui, &mut Collapse::default());
        assert_eq!(hud.band(Fit::Words), "BAND NATURAL");
        assert_eq!(hud.band(Fit::Codes), "B N");
        assert_eq!(hud.exposure(Fit::Codes), "E auto");
        ui.exposure_offset = Some(1.5);
        let hud = lines(&session, &ui, &mut Collapse::default());
        assert_eq!(hud.exposure(Fit::Words), "EXPOSURE +1.5 stops");
        assert_eq!(hud.exposure(Fit::Keys), "E +1.5");
    }

    /// The pursuit reads where a crossing's progress would, and in hull clearance.
    #[test]
    fn a_pursuit_says_who_and_how_much_space_is_between_the_hulls() {
        let (_, mut s) = fixture();
        s.ship.length_m = 500.0;
        let at_ly = s.ship.motion.position_ly + glam::DVec3::X * 3_750.0 / crate::system::M_PER_LY;
        let quarry = crate::uplink::Contact::seen(
            lc_proto::Presence {
                ship_id: lc_proto::ShipId(7),
                name: "Anvil".into(),
                length_m: 5_000.0,
                at_ly: at_ly.to_array(),
                beta: [0.0; 3],
                facing: [1.0, 0.0, 0.0],
                drive_w: 0.0,
                emit_fore_w: 0.0,
                emit_aft_w: 0.0,
                emitted_t: 0,
                arrive_t: 0,
                form: lc_proto::Form::default(),
                building: None,
                glow: None,
                glare: None,
            },
            None,
        );
        let close = lc_proto::Pursuit {
            quarry: quarry.ship_id,
            closeness: lc_proto::Closeness::Intimate,
            approach: lc_proto::Approach::Direct,
        };
        assert_eq!(
            pursuit(&s, close, Some(&quarry)),
            "alongside Anvil — 1.0 km between hulls — close in, direct",
        );
        assert_eq!(pursuit(&s, close, None), "alongside ship 7 — close in, direct");

        let polite = lc_proto::Pursuit {
            closeness: lc_proto::Closeness::Company,
            approach: lc_proto::Approach::Courteous,
            ..close
        };
        assert_eq!(pursuit(&s, polite, None), "alongside ship 7 — in company, courteous");
    }

    #[test]
    fn the_clock_is_always_shown() {
        let (ui, mut s) = fixture();
        assert!(lines(&s, &ui, &mut Collapse::default()).clock.contains("T + 0.00 years"));
        s.advance(3600.0);
        assert!(lines(&s, &ui, &mut Collapse::default()).clock.contains("T + 1.00 years"));
    }

    /// The one thing the readout exists for, and only as far as this ship knows it: the range
    /// and its error. Whose word it is on is the telescope panel's to say.
    #[test]
    fn a_selected_target_says_how_far_it_is() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        assert!(lines(&s, &ui, &mut Collapse::default()).target.is_none());
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        let target = lines(&s, &ui, &mut Collapse::default()).target.expect("a target line");
        // The sample provider's nearest star is 4.2 light-years out, charted to a percent.
        assert!(target.contains(" ± ") && target.ends_with(" ly"), "{target}");
    }

    #[test]
    fn a_fitted_ship_shows_its_energy_as_a_bar_and_numbers() {
        use lc_world::fitting::{Balance, Fitting};
        let (ui, mut s) = fixture();
        assert!(lines(&s, &ui, &mut Collapse::default()).energy.is_none(), "an unfitted ship has no energy readout");
        s.ship.fit(Some(Fitting::full(lc_world::form::Form::starting(), Balance::DEFAULT, s.coordinate_time_s())));
        let energy = lines(&s, &ui, &mut Collapse::default()).energy.expect("an energy readout");
        assert!((energy.fraction - 1.0).abs() < 1.0e-6, "{}", energy.fraction);
        assert_eq!(energy.amount, "30.0 / 30.0 ME");
    }

    /// The starting ship with heat at `of_max` of `Q_max`, storage full and `starlight_w` arriving.
    fn heated(of_max: f64, starlight_w: f64, posture: lc_world::fitting::Posture) -> (UiState, Session) {
        use lc_world::fitting::{Account, Balance};
        let (ui, mut s) = fixture();
        let full = Fitting::full(lc_world::form::Form::starting(), Balance::DEFAULT, s.coordinate_time_s());
        let heat_j = of_max * full.field().heat_max_j();
        let account = Account { heat_j, starlight_w, posture, ..full.account() };
        s.ship.fit(Some(Fitting::from_account(&account, Balance::DEFAULT)));
        s.ship.set_starlight_w(starlight_w);
        (ui, s)
    }

    fn field_of(s: &Session, ui: &UiState) -> Field {
        lines(s, ui, &mut Collapse::default()).field.expect("a fitted ship has a field bar")
    }

    const BLUE: [f32; 3] = [0.0, 0.36, 0.5];

    fn rated_w() -> f64 {
        let b = lc_world::fitting::Balance::DEFAULT;
        Fitting::full(lc_world::form::Form::starting(), b, 0.0).field().rated_load_w()
    }

    #[test]
    fn a_cold_ship_is_the_palette_blue() {
        let (ui, s) = fixture();
        assert!(lines(&s, &ui, &mut Collapse::default()).field.is_none(), "an unfitted ship has no field bar");
        let (ui, mut s) = fixture();
        let b = lc_world::fitting::Balance::DEFAULT;
        s.ship.fit(Some(Fitting::full(lc_world::form::Form::starting(), b, s.coordinate_time_s())));
        let field = field_of(&s, &ui);
        assert_eq!(field.text, "400 K steady", "idle, and storage paying the drain");
        assert_eq!(field.color(BLUE), BLUE);
        assert_eq!(field.brightness(0.37), 1.0);
    }

    /// The starting ship at 0.3 of `Q_max`, about 3 400 K, is orange.
    #[test]
    fn a_hot_ship_is_the_blackbody_past_the_draper_point() {
        let (ui, s) = heated(0.3, 0.0, lc_world::fitting::Posture::BLACK);
        let field = field_of(&s, &ui);
        assert!(field.kelvin > DRAPER_K + GLOW_BLEND_K, "{}", field.kelvin);
        let [r, g, b] = field.color(BLUE);
        assert_eq!([r, g, b], field.blackbody, "all blackbody");
        assert!(r > 0.999 && g < 0.9 && b < g, "orange at {} K: {r} {g} {b}", field.kelvin);
        assert!(field.text.starts_with(&format!("{} K -", grouped(field.kelvin))), "{}", field.text);
    }

    #[test]
    fn the_blue_gives_way_to_the_blackbody_continuously() {
        let (ui, s) = heated(0.3, 0.0, lc_world::fitting::Posture::BLACK);
        let at = |kelvin: f64| Field { kelvin, blackbody: glow(&s.mapping, kelvin), ..field_of(&s, &ui) }.color(BLUE);
        assert_eq!(at(DRAPER_K), BLUE);
        assert_eq!(at(DRAPER_K + GLOW_BLEND_K), glow(&s.mapping, DRAPER_K + GLOW_BLEND_K));
        let mut last = at(DRAPER_K - 5.0);
        for k in (DRAPER_K as i32 - 4)..(DRAPER_K + GLOW_BLEND_K) as i32 + 5 {
            let now = at(f64::from(k));
            let step = (0..3).map(|c| (now[c] - last[c]).abs()).fold(0.0, f32::max);
            assert!(step < 0.02, "a jump of {step} at {k} K");
            last = now;
        }
    }

    fn first_dip_s(field: &Field) -> Option<f64> {
        let samples: Vec<f32> = (0..2_000).map(|i| field.brightness(f64::from(i) * 1.0e-3)).collect();
        let dips = samples.windows(3).position(|w| w[1] < 1.0 && w[1] <= w[0] && w[1] <= w[2])?;
        Some((dips + 1) as f64 * 1.0e-3)
    }

    #[test]
    fn the_bar_pulses_past_eighty_percent_and_faster_toward_the_limit() {
        let bar = |of_max| {
            let (ui, s) = heated(of_max, 0.0, lc_world::fitting::Posture::BLACK);
            field_of(&s, &ui)
        };
        assert_eq!(first_dip_s(&bar(0.79)), None);
        assert_eq!(first_dip_s(&bar(PULSE_FILL)), None);
        let (warm, hot) = (first_dip_s(&bar(0.85)).expect("pulsing"), first_dip_s(&bar(0.98)).expect("pulsing"));
        assert!(hot < warm, "{hot} {warm}");
    }

    /// Where heat settles if nothing changes: the account run forty time constants on.
    #[test]
    fn the_tick_is_where_heat_is_heading_and_pinned_past_the_limit() {
        let b = lc_world::fitting::Balance::DEFAULT;
        let (ui, s) = heated(0.1, 0.5 * rated_w(), lc_world::fitting::Posture::BLACK);
        let field = field_of(&s, &ui);
        let fitting = s.ship.fitting().unwrap();
        let later_s = s.coordinate_time_s() + 40.0 * b.field_tau_s;
        let settled = fitting.heat_j_at(&s.ship.motion, later_s) / fitting.field().heat_max_j();
        assert!((f64::from(field.heading) - settled).abs() < 1.0e-6, "{} {settled}", field.heading);
        assert!(field.heading > field.fraction && field.text.contains(" K +"), "heating");

        let (ui, s) = heated(0.1, 3.0 * rated_w(), lc_world::fitting::Posture::BLACK);
        assert_eq!(field_of(&s, &ui).heading, 1.0);
    }

    #[test]
    fn a_scheduled_collapse_counts_down_in_real_time() {
        // Full and Black, starlight is all heat: `f` of the rated load heads for `f Q_max`, and
        // reaches `Q_max` from 0.99 of it in `τ ln((f − 0.99) / (f − 1))`. Half a day out.
        let tau_s = lc_world::fitting::Balance::DEFAULT.field_tau_s;
        let (_, s) = fixture();
        let now = s.coordinate_time_s();
        let e = (0.5 * (lc_world::solar::segment_end(now) - now) / tau_s).exp();
        let (mut ui, s) = heated(0.99, (e - 0.99) / (e - 1.0) * rated_w(), lc_world::fitting::Posture::BLACK);
        let at_s = s.ship.fitting().unwrap().collapse_s(&s.ship.motion, f64::INFINITY).expect("premise: it collapses");
        assert!(at_s < lc_world::solar::segment_end(now), "premise: within the starlight segment");
        assert!(at_s - now > 0.4 * (lc_world::solar::segment_end(now) - now), "premise: a countdown worth reading");
        for rate in [1.0, 0.05] {
            ui.time_rate = rate;
            let real = ((at_s - now) / (crate::session::TIME_RATE * rate)).ceil() as u64;
            let want = format!(" — collapse in {}:{:02}", real / 60, real % 60);
            let text = field_of(&s, &ui).text;
            assert!(text.ends_with(&want), "{text} wants {want}");
            let hud = lines(&s, &ui, &mut Collapse::default());
            assert_eq!(hud.field_text(Fit::Bare), Some(&want[" — ".len()..]), "never dropped");
        }
        let (ui, s) = heated(0.3, 0.0, lc_world::fitting::Posture::BLACK);
        assert!(!field_of(&s, &ui).text.contains("collapse"));
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).field_text(Fit::Bare), None);
        assert_eq!(countdown(3_725.0 * crate::session::TIME_RATE, 1.0), "1:02:05");
        assert_eq!(countdown(86_400.0 * 2.5, 0.0), "2.5 days");
    }

    #[test]
    fn auto_shows_the_shade_and_a_switch_under_way() {
        use lc_world::fitting::{Balance, Posture, Switch};
        let b = Balance::DEFAULT;
        let auto = Posture::new_ship(&b);
        let (ui, s) = heated(0.1, 0.0, auto);
        let field = field_of(&s, &ui);
        assert_eq!((field.shade, field.switch), (Mode::Clear, None));
        assert_eq!(field.setting, Setting::Auto(Thresholds::of(&b)));

        let done_s = s.coordinate_time_s() + 0.5 * b.field_switch_s;
        let (ui, s) = heated(0.1, 0.0, Posture { switch: Some(Switch { to: Mode::Black, done_s }), ..auto });
        let field = field_of(&s, &ui);
        assert_eq!((field.shade, field.switch.as_deref()), (Mode::Clear, Some("» BLACK")), "Clear until done");

        let (ui, s) = heated(0.1, 0.0, Posture::BLACK);
        let field = field_of(&s, &ui);
        assert_eq!((field.shade, field.switch, field.auto), (Mode::Black, None, Thresholds::of(&b)));
    }

    #[test]
    fn dropping_a_marker_orders_auto_with_both_gaps_open() {
        use lc_world::fitting::{Balance, Posture};
        let b = Balance::DEFAULT;
        let (mut ui, mut s) = heated(0.1, 0.0, Posture::new_ship(&b));
        s.remote = true;
        let field = field_of(&s, &ui);
        let action = drop_marker(&field, Marker::ClearAbove, 0.623).expect("a new threshold");
        let want = lc_proto::FieldMode::Auto { clear_above: 0.62, black_below: b.auto_black_below, refill_below: b.auto_refill_below };
        assert_eq!(action, Action::SetField(want));
        assert_eq!(apply(action, &mut ui, &mut s), vec![crate::action::Effect::Send(lc_proto::Order::FieldMode { mode: want })]);

        let Some(Action::SetField(below)) = drop_marker(&field, Marker::ClearAbove, 0.1) else { panic!("moved") };
        let Setting::Auto(below) = Setting::from(below) else { panic!("Auto") };
        assert!(below.is_valid() && below.clear_above > below.black_below, "{below:?}");
        let Some(Action::SetField(floor)) = drop_marker(&field, Marker::BlackBelow, 0.0) else { panic!("moved") };
        let Setting::Auto(floor) = Setting::from(floor) else { panic!("Auto") };
        assert!(floor.is_valid(), "{floor:?}");
        assert_eq!(drop_marker(&field, Marker::BlackBelow, b.auto_black_below + 0.001), None, "where it was");

        let (_, black) = heated(0.1, 0.0, Posture::BLACK);
        assert_eq!(drop_marker(&field_of(&black, &ui), Marker::ClearAbove, 0.6), None, "no markers outside Auto");
    }

    #[test]
    fn an_order_during_a_switch_is_sent() {
        use lc_world::fitting::{Balance, Posture, Switch};
        let b = Balance::DEFAULT;
        let (_, s) = heated(0.1, 0.0, Posture::new_ship(&b));
        let done_s = s.coordinate_time_s() + b.field_switch_s;
        let (mut ui, mut s) = heated(0.1, 0.0, Posture { switch: Some(Switch { to: Mode::Black, done_s }), ..Posture::new_ship(&b) });
        s.remote = true;
        let turn = apply(Action::SetField(lc_proto::FieldMode::Clear), &mut ui, &mut s);
        assert_eq!(turn, vec![crate::action::Effect::Send(lc_proto::Order::FieldMode { mode: lc_proto::FieldMode::Clear })]);
    }

    #[test]
    fn a_marker_with_no_room_stays_put_and_never_reaches_collapse() {
        let at = |clear_above, black_below| Thresholds { clear_above, black_below, refill_below: 0.95 };
        let tight = at(0.03, 0.01);
        assert!(tight.is_valid(), "premise: the shard takes it");
        assert_eq!(Marker::BlackBelow.moved(tight, 0.5), tight);
        let high = at(0.99, 0.985);
        assert!(high.is_valid());
        assert_eq!(Marker::ClearAbove.moved(high, 0.2), high);
        let top = Marker::ClearAbove.moved(at(0.5, 0.3), 1.0);
        assert_eq!(top.clear_above, 0.98, "held off collapse");
        let floor = Marker::ClearAbove.moved(at(0.5, 0.07), 0.0);
        assert_eq!(floor.clear_above, 0.09, "on a hundredth");
    }

    #[test]
    fn a_dropped_marker_holds_until_the_account_agrees() {
        use lc_world::fitting::{Balance, Posture};
        let b = Balance::DEFAULT;
        let (_, s) = heated(0.1, 0.0, Posture::new_ship(&b));
        let ui = UiState::default();
        let field = field_of(&s, &ui);
        let dropped = Dropped { thresholds: Marker::ClearAbove.moved(Thresholds::of(&b), 0.62), at_s: 10.0 };
        assert!(dropped.holds(&field, 10.0 + 0.9 * AWAITING_S), "in flight");
        assert!(!dropped.holds(&field, 10.0 + AWAITING_S), "overdue: refused, or lost");
        let agreed = Field { setting: Setting::Auto(dropped.thresholds), ..field.clone() };
        assert!(!dropped.holds(&agreed, 10.5), "accepted");
        let black = Field { setting: Setting::Black, ..field };
        assert!(!dropped.holds(&black, 10.5), "no markers outside Auto");
    }

    /// Settling along the account's own path is not a change; a restated account is.
    #[test]
    fn the_countdown_is_solved_again_only_when_the_account_changes() {
        use lc_world::fitting::{Account, Balance, Posture};
        let (_, mut s) = heated(0.3, 0.0, Posture::BLACK);
        let now = s.coordinate_time_s();
        let mut collapse = Collapse::default();
        assert_eq!(collapse.of(&s.ship, now), None);
        for k in 1..=5 {
            let t = now + f64::from(k) * 100.0;
            s.ship.settle(t);
            assert_eq!(collapse.of(&s.ship, t), None);
        }
        assert_eq!(collapse.solves, 1, "settling is not a change");

        let t = now + 600.0;
        let fitting = s.ship.fitting().unwrap();
        let hot = Account { heat_j: 0.99 * fitting.field().heat_max_j(), starlight_w: 50.0 * rated_w(), ..fitting.account() };
        s.ship.fit(Some(Fitting::from_account(&hot, Balance::DEFAULT)));
        s.ship.set_starlight_w(50.0 * rated_w());
        assert!(collapse.of(&s.ship, t).is_some(), "a restated account counts down");
        assert_eq!(collapse.solves, 2);
    }

    #[test]
    fn the_band_mapping_is_named_not_numbered() {
        let (mut ui, mut s) = fixture();
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).mapping, "NATURAL");
        apply(Action::SetBandPreset(2), &mut ui, &mut s);
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).mapping, "THERMAL");
    }

    #[test]
    fn exposure_reads_auto_until_it_is_moved() {
        let (mut ui, mut s) = fixture();
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).exposure, "auto");
        apply(Action::ExposureUp, &mut ui, &mut s);
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).exposure, "+0.5 stops");
        apply(Action::ExposureDown, &mut ui, &mut s);
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).exposure, "+0.0 stops");
        apply(Action::ExposureDown, &mut ui, &mut s);
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).exposure, "-0.5 stops");
        apply(Action::ExposureAuto, &mut ui, &mut s);
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).exposure, "auto");
    }

    /// A rate that is not the world's has to say so: a sixty-times clock that looked normal
    /// would make every duration on screen a lie.
    ///
    /// The default is the design rate and therefore says nothing, which is the point of it —
    /// a warning that is always on is a warning nobody reads.
    #[test]
    fn a_non_canonical_clock_rate_is_announced() {
        let (mut ui, mut s) = fixture();
        assert!(lines(&s, &ui, &mut Collapse::default()).warning.is_none(), "the default is the world's own rate");
        apply(Action::SetTimeRate(60.0), &mut ui, &mut s);
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).warning.unwrap(), "1 year / minute", "a fast clock is flagged");
        apply(Action::SetTimeRate(64.0), &mut ui, &mut s);
        let off_ladder = lines(&s, &ui, &mut Collapse::default()).warning.expect("one off the ladder must be flagged");
        assert_eq!(off_ladder, "1 year / 56 seconds");
        // And a rate *below* the design one is flagged just as loudly. A scene runs slowly so
        // an orbit can be looked at, and a slow clock is no more normal than a fast one.
        apply(Action::SetTimeRate(0.05), &mut ui, &mut s);
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).warning.unwrap(), "7 minutes / second");
        apply(Action::SetTimeRate(1.0), &mut ui, &mut s);
        assert!(lines(&s, &ui, &mut Collapse::default()).warning.is_none(), "the canonical rate needs no warning");
    }

    /// The narrowest fit keeps the rate on the bar in unit symbols, so a scene's slow clock is
    /// still said rather than hidden in a hover.
    #[test]
    fn a_bare_bar_shortens_the_rate_and_does_not_drop_it() {
        assert_eq!(rate_code("15 minutes / second"), "15 min/s");
        assert_eq!(rate_code("1 year / minute"), "1 yr/min");
        assert_eq!(rate_code("1 year / 10 min"), "1 yr/10 min");
        assert_eq!(rate_code("1 year / 56 seconds"), "1 yr/56 s");
        assert_eq!(rate_code("2 days / second"), "2 d/s");
        assert_eq!(rate_code("stopped"), "stopped");
        for (_, rung) in crate::ui::RATE_LADDER {
            assert!(rate_code(rung).len() <= rung.len(), "{rung} grew");
        }

        let (mut ui, mut s) = fixture();
        apply(Action::SetTimeRate(0.05), &mut ui, &mut s);
        let hud = lines(&s, &ui, &mut Collapse::default());
        assert_eq!(hud.warning(Fit::Keys).unwrap(), "7 minutes / second");
        assert_eq!(hud.warning(Fit::Bare).unwrap(), "7 min/s");
        apply(Action::SetTimeRate(1.0), &mut ui, &mut s);
        assert_eq!(lines(&s, &ui, &mut Collapse::default()).warning(Fit::Bare), None);
    }

    /// Only the bare fit leaves the energy bar without its numbers.
    #[test]
    fn a_bare_bar_shows_energy_as_the_bar_alone() {
        use lc_world::fitting::{Balance, Fitting};
        let (ui, mut s) = fixture();
        s.ship.fit(Some(Fitting::full(lc_world::form::Form::starting(), Balance::DEFAULT, s.coordinate_time_s())));
        let hud = lines(&s, &ui, &mut Collapse::default());
        for fit in [Fit::Words, Fit::Codes, Fit::Keys] {
            assert_eq!(hud.energy_amount(fit), Some("30.0 / 30.0 ME"), "{fit:?}");
        }
        assert_eq!(hud.energy_amount(Fit::Bare), None);
        assert!(hud.energy.is_some(), "the bar itself is still drawn");
    }

    /// The link note keeps its state at the bare fit, and loses who, how fast and why.
    #[test]
    fn a_bare_bar_says_the_state_of_the_link_and_no_more() {
        use crate::uplink::{Joined, Note, State};
        let joined = State::Joined(Joined {
            client_id: lc_proto::ClientId(1),
            ship_id: lc_proto::ShipId(1),
            name: "Traveler 1".into(),
        });
        assert_eq!(
            link(&joined, Some(0.012), Fit::Keys),
            Some((Note::Quiet, "LINKED Traveler 1 · 12 ms".into())),
        );
        assert_eq!(link(&joined, Some(0.012), Fit::Bare), Some((Note::Quiet, "LINKED".into())));
        assert_eq!(
            link(&State::Lost("socket closed".into()), None, Fit::Bare),
            Some((Note::Wrong, "LINK LOST".into())),
        );
        assert_eq!(
            link(&State::Refused("no".into()), None, Fit::Bare),
            Some((Note::Wrong, "REFUSED".into())),
        );
        assert_eq!(link(&State::Connecting, None, Fit::Bare), Some((Note::Working, "CONNECTING".into())));
        assert_eq!(link(&State::Offline, None, Fit::Bare), None);
    }

    /// The readout's job during a crossing: the two clocks disagree and it has to show both.
    #[test]
    fn a_crossing_reports_itself_and_the_ship_clock_falls_behind() {
        let (ui, mut s) = fixture();
        let id = s.stars[0].id;
        assert!(lines(&s, &ui, &mut Collapse::default()).flight.is_none());
        s.fly_to(id);
        s.advance(3600.0);
        let l = lines(&s, &ui, &mut Collapse::default());
        let flight = l.flight.expect("a flight line");
        assert!(flight.contains('c') && flight.contains("to go"), "{flight}");
        let coordinate: f64 = l.clock.trim_start_matches("T + ").trim_end_matches(" years").parse().unwrap();
        let aboard: f64 = l.ship_clock.trim_start_matches("T' + ").trim_end_matches(" years").parse().unwrap();
        assert!(aboard < coordinate, "ship {aboard} should be behind coordinate {coordinate}");
    }

    /// A star nobody has named still has something to call it: the designation its own
    /// discovery wrote down. Nothing falls back to a catalog.
    #[test]
    fn an_unnamed_star_still_gets_a_label() {
        let (mut ui, mut s) = fixture();
        let id = s.stars[0].id;
        s.knowledge = lc_world::knowledge::Knowledge::new(lc_world::knowledge::Witness(0));
        s.point_at(Some(id));
        s.advance(1.0);
        s.tick_instruments(1.0);
        apply(Action::SelectTarget(Some(id)), &mut ui, &mut s);
        let target = lines(&s, &ui, &mut Collapse::default()).target.unwrap();
        // A second's stare has detected nothing yet, so there is neither a name nor a range.
        assert!(target.ends_with("not detected"), "{target}");
        assert!(
            !target.starts_with("Authored"),
            "a catalog name is not a name: {target}"
        );
    }
}
