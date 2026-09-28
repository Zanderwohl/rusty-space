//! The emit window: one order for dumping heat, feeding an ally and attacking, previewed before it
//! is sent, and every beam landing on this ship. See `lightcone/docs/31-directed-energy.md` §Client.
//!
//! [`preview`] and [`Beams`] carry no engine, so what the window says is tested without one. The
//! numbers are `lc_world`'s: the server lights the same spread from the same ends.

use bevy::prelude::*;
use bevy_egui::egui;
use glam::DVec3;
use lc_proto::{Aim, Apertures, Order, Refusal, ShipId, Shade, Spectrum};
use lc_world::craft::{BEAM_PER_LENGTH, Craft};
use lc_world::emit::{lead_uncertainty_m, received_fraction};
use lc_world::field::Segment;
use lc_world::fitting::{Balance, Lit};
use lc_world::flight::{C_M_S, G0};
use lc_world::form::Form;
use lc_world::form::capacity::{Capacities, End, aft_aperture_w, dry_mass_kg, ends};
use lc_world::signal::Transmitter;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::input::Requested;
use crate::panels::{ask, duration, span_m};
use crate::plume::Drawn;
use crate::system::M_PER_LY;
use crate::uplink::Contact;

/// What the server lights: 1 nm to the comms dish's 3 cm.
pub const WAVELENGTH_M: (f64, f64) = (1.0e-9, 0.03);
const SPREAD_MAX_RAD: f64 = std::f64::consts::FRAC_PI_2;
const POWER_MIN_W: f64 = 1.0e6;
const DURATION_S: (f64, f64) = (1.0, 3.0e7);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aimed {
    Craft(ShipId),
    /// Where the view is looking when sent.
    Reticle,
    /// Ecliptic degrees, as [`crate::ui::Look`] holds them.
    Bearing { yaw_deg: f64, pitch_deg: f64 },
}

/// What the window holds between frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Draft {
    pub aimed: Aimed,
    pub apertures: Apertures,
    pub power_w: f64,
    pub wavelength_m: f64,
    /// Asked for; the floor wins below it.
    pub spread_rad: f64,
    pub duration_s: f64,
    /// The selection last taken as the aim, so a new one re-aims and the window's own choice
    /// otherwise stands.
    pub followed: Option<ShipId>,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            aimed: Aimed::Reticle,
            apertures: Apertures::Both,
            power_w: 1.0e18,
            wavelength_m: 1.0e-6,
            spread_rad: 0.0,
            duration_s: 600.0,
            followed: None,
        }
    }
}

/// What this ship knows of the craft it aims at. `None` is unknown, and the window says so.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Receiver {
    pub distance_m: f64,
    /// From the light it was last seen by leaving it to the beam landing on it, seconds.
    pub blind_s: f64,
    /// The most it can accelerate at, m/s²: its aft rating over its dry mass.
    pub accel_m_s2: Option<f64>,
    /// Broadside to the beam, the most it can present.
    pub shadow_m2: f64,
    pub length_m: f64,
    /// Its engines' conversion rating, W.
    pub rating_w: Option<f64>,
    pub absorptivity: Option<f64>,
}

impl Receiver {
    /// `contact` as seen from `here_ly` at `now_s`, its form and glow read with `balance`.
    pub fn of(contact: &Contact, here_ly: DVec3, now_s: f64, balance: &Balance) -> Self {
        let distance_m = contact.position_ly.distance(here_ly) * M_PER_LY;
        let form = Form::from(&contact.form);
        let formed = !form.parts.is_empty();
        let accel_m_s2 = formed
            .then(|| aft_aperture_w(&form, balance).map(|w| w / (dry_mass_kg(&form, balance) * C_M_S)))
            .flatten();
        let rating_w = formed.then(|| Capacities::of(&form, balance).aperture_w);
        // A craft seen without a form has H7's stand-in glow, which is no statement of its shade.
        let absorptivity = formed.then(|| match contact.glow.shade {
            Shade::Black => 1.0,
            Shade::Clear => balance.clear_absorptivity,
        });
        Self {
            distance_m,
            blind_s: (now_s - contact.emitted_s).max(0.0) + distance_m / C_M_S,
            accel_m_s2,
            shadow_m2: broadside_m2(contact.length_m),
            length_m: contact.length_m,
            rating_w,
            absorptivity,
        }
    }
}

/// The ovoid a hull of `length_m` is drawn as, seen along its short axis: the shadow 31's table of
/// what arrives is worked for.
pub fn broadside_m2(length_m: f64) -> f64 {
    let half = 0.5 * length_m;
    std::f64::consts::PI * half * half * BEAM_PER_LENGTH
}

/// Everything the window shows before sending.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preview {
    pub floor_rad: f64,
    /// What the server will light: the draft's, never under either end's floor.
    pub spread_rad: f64,
    /// Each end sends `power_w`; a balanced emit draws twice it.
    pub drawn_w: f64,
    pub cost_j: f64,
    pub from_heat_j: f64,
    pub from_storage_j: f64,
    /// Proper acceleration, m/s², away from the beam. Zero for a balanced emit.
    pub recoil_m_s2: f64,
    pub refusal: Option<Refusal>,
    pub at: Option<Landing>,
}

/// What the preview expects at the receiver.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub distance_m: f64,
    /// Radius of the cap's rim, meters: `spread × d` for a narrow beam.
    pub spot_m: f64,
    pub lead_m: Option<f64>,
    pub fraction: f64,
    /// Before its absorptivity.
    pub arriving_w: f64,
    pub absorbed_w: Option<f64>,
    /// Past its rating: what it takes in over what it converts. Absorptivity one when its shade
    /// is unknown, since a Black receiver takes all of it.
    pub past_rating_w: Option<f64>,
}

/// What `draft` would do, sent from `ship` at `now_s` toward `receiver`.
pub fn preview(draft: &Draft, ship: &Craft, now_s: f64, receiver: Option<&Receiver>) -> Preview {
    let chosen: Vec<End> = ship
        .fitting()
        .and_then(|f| ends(f.form(), f.balance()))
        .map(|(fore, aft)| match draft.apertures {
            Apertures::Fore => vec![fore],
            Apertures::Aft => vec![aft],
            Apertures::Both => vec![fore, aft],
        })
        .unwrap_or_default();
    // An end with no engines has no face, and λ/0 would floor the other end's beam at a hemisphere.
    let faced = || chosen.iter().filter(|end| end.diameter_m > 0.0);
    let floor_rad = faced()
        .map(|end| Transmitter::new(draft.wavelength_m, end.diameter_m).half_angle_rad())
        .fold(0.0, f64::max);
    let spread_rad = faced()
        .map(|end| Transmitter::new(draft.wavelength_m, end.diameter_m).spread_rad(draft.spread_rad))
        .fold(0.0, f64::max);
    let ends_lit = if draft.apertures == Apertures::Both { 2.0 } else { 1.0 };
    let drawn_w = ends_lit * draft.power_w;
    let cost_j = drawn_w * draft.duration_s;
    let mass_kg = ship.mass_kg_at(now_s);
    let recoil_m_s2 = match draft.apertures {
        Apertures::Both => 0.0,
        Apertures::Fore | Apertures::Aft => draft.power_w / (mass_kg * C_M_S),
    };
    let from_storage_j = source_j(ship, now_s, drawn_w, draft.duration_s).min(cost_j);
    let refusal = if ship.is_refitting(now_s) {
        Some(Refusal::Refitting)
    } else if ship.motion.is_under_way() {
        Some(Refusal::UnderWay)
    } else if chosen.is_empty() || chosen.iter().any(|end| end.rating_w <= 0.0) {
        Some(Refusal::NoAperture)
    } else if chosen.iter().any(|end| draft.power_w > end.rating_w) {
        Some(Refusal::OverRating)
    } else if cost_j > ship.free_j_at(now_s) {
        Some(Refusal::NoEnergy)
    } else {
        None
    };
    Preview {
        floor_rad,
        spread_rad,
        drawn_w,
        cost_j,
        from_heat_j: cost_j - from_storage_j,
        from_storage_j,
        recoil_m_s2,
        refusal,
        at: receiver.map(|r| landing(r, draft.power_w, spread_rad)),
    }
}

fn landing(receiver: &Receiver, power_w: f64, spread_rad: f64) -> Landing {
    let fraction = received_fraction(spread_rad, receiver.shadow_m2, receiver.distance_m);
    let arriving_w = power_w * fraction;
    let absorbed_w = receiver.absorptivity.map(|a| a * arriving_w);
    Landing {
        distance_m: receiver.distance_m,
        spot_m: spread_rad.min(SPREAD_MAX_RAD).sin() * receiver.distance_m,
        lead_m: receiver.accel_m_s2.map(|a| lead_uncertainty_m(a, receiver.blind_s)),
        fraction,
        arriving_w,
        absorbed_w,
        past_rating_w: receiver.rating_w.map(|rating| absorbed_w.unwrap_or(arriving_w) - rating).filter(|&w| w > 0.0),
    }
}

/// Of `drawn_w` for `duration_s`, what storage pays, joules: the ship's own account run with and
/// without it, so heat goes first exactly as the server draws it.
fn source_j(ship: &Craft, now_s: f64, drawn_w: f64, duration_s: f64) -> f64 {
    let Some(fitting) = ship.fitting() else { return 0.0 };
    let mut dark = fitting.clone();
    dark.settle(&ship.motion, now_s);
    let mut lit = dark.clone();
    lit.light(Lit { from_s: now_s, until_s: now_s + duration_s, power_w: drawn_w });
    let end_s = now_s + duration_s;
    (dark.stored_j_at(&ship.motion, end_s) - lit.stored_j_at(&ship.motion, end_s)).max(0.0)
}

/// Where a draft points, and what the wire calls it. `look` is the view's axis.
pub fn aim_of(aimed: Aimed, look: DVec3) -> Aim {
    match aimed {
        Aimed::Craft(id) => Aim::Ship(id),
        Aimed::Reticle => Aim::Bearing(look.to_array()),
        Aimed::Bearing { yaw_deg, pitch_deg } => {
            let look = crate::ui::Look { yaw: yaw_deg.to_radians(), pitch: pitch_deg.to_radians() };
            Aim::Bearing(look.forward().to_array())
        }
    }
}

/// The axis `aim` points along from `here_ly`, as this ship sees it.
pub fn axis_of(aim: &Aim, here_ly: DVec3, contacts: &[Contact]) -> Option<DVec3> {
    match aim {
        Aim::Ship(id) => contacts.iter().find(|c| c.ship_id == *id).and_then(|c| (c.position_ly - here_ly).try_normalize()),
        Aim::Bearing(b) => DVec3::from_array(*b).try_normalize(),
        Aim::Omni | Aim::Star(_) => None,
    }
}

/// A beam arriving here, as `Illuminated` last stated it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Incoming {
    pub beam: i64,
    /// Unit, world axes, toward the source.
    pub bearing: DVec3,
    pub spectrum: Spectrum,
    /// Before this field's absorptivity.
    pub power_w: f64,
    pub since_s: f64,
}

/// A beam this ship has lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sent {
    pub event_id: i64,
    pub axis: DVec3,
    pub apertures: Apertures,
    pub half_angle_rad: f64,
    pub power_w: f64,
    pub from_s: f64,
    pub until_s: f64,
}

/// The only beams this ship knows of: those landing on it and those it lit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beams {
    pub incoming: Vec<Incoming>,
    pub sent: Vec<Sent>,
}

impl Beams {
    /// Fold an `Illuminated`. A restatement replaces the beam's entry, zero power removes it, and
    /// one older than what is held is dropped.
    pub fn illuminated(&mut self, beam: i64, bearing: [f64; 3], spectrum: Spectrum, power_w: f64, arrive_s: f64) {
        let held = self.incoming.iter().position(|i| i.beam == beam);
        if held.is_some_and(|at| self.incoming[at].since_s > arrive_s) {
            return;
        }
        if let Some(at) = held {
            self.incoming.remove(at);
        }
        if power_w > 0.0 {
            let bearing = DVec3::from_array(bearing).normalize_or_zero();
            self.incoming.push(Incoming { beam, bearing, spectrum, power_w, since_s: arrive_s });
        }
    }

    /// Fold this ship's accepted `Order::Emit`, aimed as this ship sees its target from `here_ly`.
    pub fn lit(&mut self, order: &Order, event_id: i64, at_s: f64, here_ly: DVec3, contacts: &[Contact]) {
        let Order::Emit { aim, apertures, power_w, spread_rad, duration_s, .. } = *order else { return };
        let Some(axis) = axis_of(&aim, here_ly, contacts) else { return };
        self.sent.push(Sent {
            event_id,
            axis,
            apertures,
            half_angle_rad: spread_rad,
            power_w,
            from_s: at_s,
            until_s: at_s + duration_s,
        });
    }

    /// Put out at `at_s` whatever was still lit.
    pub fn put_out(&mut self, at_s: f64) {
        for sent in &mut self.sent {
            sent.until_s = sent.until_s.min(at_s);
        }
    }

    pub fn prune(&mut self, now_s: f64) {
        self.sent.retain(|s| s.until_s > now_s);
    }

    /// W landing now, before absorptivity.
    pub fn landing_w(&self) -> f64 {
        self.incoming.iter().map(|i| i.power_w).sum()
    }
}

/// Which beam a map line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OnMap {
    /// Its event, and whether this is a balanced emit's aft beam.
    Sent(i64, bool),
    Landing(i64),
}

/// This ship's beams as cones from `here_ly`, each as far as its light has got and no further than
/// `reach_m`, and each beam landing here as a line `reach_m` long back along its bearing.
///
/// No cooking ring: its share of a cone that grows every frame would be a new mesh every frame.
pub fn on_map(beams: &Beams, here_ly: DVec3, now_s: f64, reach_m: f64) -> Vec<(OnMap, Drawn)> {
    let cone = |axis: DVec3, length_m: f64, half_angle_rad: f64| Drawn {
        craft: None,
        apex_ly: here_ly,
        aft: axis,
        length_m,
        half_angle_rad,
        cooking_m: 0.0,
    };
    let mut drawn = Vec::new();
    for sent in beams.sent.iter().filter(|s| s.from_s <= now_s && now_s < s.until_s) {
        let length_m = (C_M_S * (now_s - sent.from_s)).min(reach_m);
        let mut ends = vec![(sent.axis, false)];
        if sent.apertures == Apertures::Both {
            ends.push((-sent.axis, true));
        }
        for (axis, aft) in ends {
            drawn.push((OnMap::Sent(sent.event_id, aft), cone(axis, length_m, sent.half_angle_rad)));
        }
    }
    for landing in &beams.incoming {
        drawn.push((OnMap::Landing(landing.beam), cone(landing.bearing, reach_m, 0.0)));
    }
    drawn
}

pub fn wavelength(m: f64) -> String {
    match m {
        m if m < 1.0e-6 => format!("{:.0} nm", m * 1.0e9),
        m if m < 1.0e-3 => format!("{:.2} µm", m * 1.0e6),
        m if m < 1.0e-2 => format!("{:.1} mm", m * 1.0e3),
        m => format!("{:.1} cm", m * 1.0e2),
    }
}

fn band(spectrum: Spectrum) -> String {
    match spectrum {
        Spectrum::Line { wavelength_m } => wavelength(wavelength_m),
        Spectrum::Blackbody { temperature_k } => format!("{temperature_k:.1e} K"),
    }
}

fn watts(w: f64) -> String {
    format!("{w:.2e} W")
}

fn angle(rad: f64) -> String {
    match rad {
        r if r < 1.0e-6 => format!("{:.2} nrad", r * 1.0e9),
        r if r < 1.0e-3 => format!("{:.2} µrad", r * 1.0e6),
        r if r < 0.1 => format!("{:.2} mrad", r * 1.0e3),
        r => format!("{:.1}°", r.to_degrees()),
    }
}

fn hazard() -> egui::Color32 {
    crate::map_panel::color_of(crate::ui::HAZARD)
}

pub(crate) fn emit(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &Game,
    uplink: &crate::uplink::Uplink,
    draft: &mut Draft,
    out: &mut MessageWriter<Requested>,
) {
    let session = &game.0;
    let now_s = session.coordinate_time_s();
    let here_ly = session.ship.motion.position_ly;
    let Some(balance) = session.ship.fitting().map(|f| *f.balance()) else {
        ui.weak("no fitting");
        return;
    };
    if let Some(id) = state.0.selected_craft
        && draft.followed != Some(id)
        && uplink.contacts.iter().any(|c| c.ship_id == id)
    {
        draft.followed = Some(id);
        draft.aimed = Aimed::Craft(id);
    }
    if let Aimed::Craft(id) = draft.aimed
        && !uplink.contacts.iter().any(|c| c.ship_id == id)
    {
        draft.aimed = Aimed::Reticle;
    }

    aim_row(ui, uplink, draft);
    let offered = offered(session.ship.fitting().and_then(|f| ends(f.form(), f.balance())));
    if let Some(&last) = offered.last().filter(|_| !offered.contains(&draft.apertures)) {
        draft.apertures = last;
    }
    ui.horizontal(|ui| {
        for apertures in offered {
            let label = match apertures {
                Apertures::Fore => "fore",
                Apertures::Aft => "aft",
                Apertures::Both => "both",
            };
            if ui.selectable_label(draft.apertures == apertures, label).clicked() {
                draft.apertures = apertures;
            }
        }
    });
    let receiver = match draft.aimed {
        Aimed::Craft(id) => uplink.contacts.iter().find(|c| c.ship_id == id).map(|c| Receiver::of(c, here_ly, now_s, &balance)),
        _ => None,
    };
    let rating_w = session
        .ship
        .fitting()
        .and_then(|f| ends(f.form(), f.balance()))
        .map(|(fore, aft)| match draft.apertures {
            Apertures::Fore => fore.rating_w,
            Apertures::Aft => aft.rating_w,
            Apertures::Both => fore.rating_w.min(aft.rating_w),
        })
        .unwrap_or(0.0);
    egui::Grid::new("emit_settings").num_columns(2).show(ui, |ui| {
        ui.label("power");
        let top_w = rating_w.max(POWER_MIN_W * 10.0);
        ui.add(egui::Slider::new(&mut draft.power_w, POWER_MIN_W..=top_w).logarithmic(true).custom_formatter(|w, _| watts(w)));
        ui.end_row();
        ui.label("wavelength");
        ui.add(
            egui::Slider::new(&mut draft.wavelength_m, WAVELENGTH_M.0..=WAVELENGTH_M.1)
                .logarithmic(true)
                .custom_formatter(|m, _| wavelength(m)),
        );
        ui.end_row();
        ui.label("spread");
        let lit = preview(draft, &session.ship, now_s, None);
        let floor_rad = lit.floor_rad.max(f64::MIN_POSITIVE);
        let mut shown = lit.spread_rad.max(floor_rad);
        let slider = egui::Slider::new(&mut shown, floor_rad..=SPREAD_MAX_RAD).logarithmic(true).custom_formatter(|r, _| angle(r));
        if ui.add(slider).changed() {
            draft.spread_rad = shown;
        }
        ui.end_row();
        ui.label("duration");
        ui.add(
            egui::Slider::new(&mut draft.duration_s, DURATION_S.0..=DURATION_S.1)
                .logarithmic(true)
                .custom_formatter(|s, _| duration(s)),
        );
        ui.end_row();
    });

    let seen = preview(draft, &session.ship, now_s, receiver.as_ref());
    ui.separator();
    show_preview(ui, &seen, receiver.as_ref(), &balance);
    ui.horizontal(|ui| {
        let order = Order::Emit {
            aim: aim_of(draft.aimed, state.0.look.forward()),
            apertures: draft.apertures,
            power_w: draft.power_w,
            wavelength_m: draft.wavelength_m,
            spread_rad: seen.spread_rad,
            duration_s: draft.duration_s,
        };
        if ui.add_enabled(seen.refusal.is_none() && session.remote, egui::Button::new("Emit")).clicked() {
            ask(out, Action::Emit(order));
        }
        match seen.refusal {
            // The shared wording is a refit's.
            Some(Refusal::UnderWay) => ui.colored_label(hazard(), "under way"),
            Some(why) => ui.colored_label(hazard(), crate::uplink::refused(why)),
            None => ui.label(""),
        };
    });
    if !uplink.beams.sent.is_empty() && ui.button("Put out").clicked() {
        ask(out, Action::PutOut);
    }

    ui.separator();
    incoming(ui, session, uplink, draft);
}

/// The ends a ship with these can emit from, both last.
fn offered(ends: Option<(End, End)>) -> Vec<Apertures> {
    let Some((fore, aft)) = ends else { return Vec::new() };
    let mut offered = Vec::new();
    if fore.rating_w > 0.0 {
        offered.push(Apertures::Fore);
    }
    if aft.rating_w > 0.0 {
        offered.push(Apertures::Aft);
    }
    if offered.len() == 2 {
        offered.push(Apertures::Both);
    }
    offered
}

fn aim_row(ui: &mut egui::Ui, uplink: &crate::uplink::Uplink, draft: &mut Draft) {
    ui.horizontal(|ui| {
        if ui.selectable_label(draft.aimed == Aimed::Reticle, "reticle").clicked() {
            draft.aimed = Aimed::Reticle;
        }
        let bearing = matches!(draft.aimed, Aimed::Bearing { .. });
        if ui.selectable_label(bearing, "bearing").clicked() && !bearing {
            draft.aimed = Aimed::Bearing { yaw_deg: 0.0, pitch_deg: 0.0 };
        }
        let named = match draft.aimed {
            Aimed::Craft(id) => uplink.name_of(id),
            _ => "craft".to_string(),
        };
        egui::ComboBox::from_id_salt("emit_craft").selected_text(named).show_ui(ui, |ui| {
            for contact in &uplink.contacts {
                if ui.selectable_label(draft.aimed == Aimed::Craft(contact.ship_id), &contact.name).clicked() {
                    draft.aimed = Aimed::Craft(contact.ship_id);
                }
            }
        });
    });
    if let Aimed::Bearing { yaw_deg, pitch_deg } = &mut draft.aimed {
        ui.horizontal(|ui| {
            ui.label("azimuth");
            ui.add(egui::DragValue::new(yaw_deg).range(0.0..=360.0).suffix("°"));
            ui.label("elevation");
            ui.add(egui::DragValue::new(pitch_deg).range(-90.0..=90.0).suffix("°"));
        });
    }
}

fn show_preview(ui: &mut egui::Ui, seen: &Preview, receiver: Option<&Receiver>, balance: &Balance) {
    let module_j = balance.module_energy_j();
    ui.label(format!("floor {} · spread {}", angle(seen.floor_rad), angle(seen.spread_rad)));
    if let (Some(at), Some(receiver)) = (seen.at, receiver) {
        ui.horizontal(|ui| {
            spot(ui, &at, receiver);
            ui.vertical(|ui| {
                ui.label(format!("at {}", span_m(at.distance_m)));
                ui.label(format!("spot {} across", span_m(2.0 * at.spot_m)));
                let lead = match at.lead_m {
                    Some(m) => format!("lead ±{}", span_m(m)),
                    None => "lead unknown".to_string(),
                };
                match at.lead_m.is_some_and(|m| m > at.spot_m) {
                    true => ui.colored_label(hazard(), lead),
                    false => ui.label(lead),
                };
                ui.label(format!("arriving {} ({:.1e} of it)", watts(at.arriving_w), at.fraction));
                ui.label(match at.absorbed_w {
                    Some(w) => format!("absorbed {}", watts(w)),
                    None => "shade unknown".to_string(),
                });
                match receiver.rating_w {
                    Some(r) => ui.label(format!("its rating {} · room unknown", watts(r))),
                    None => ui.label("rating unknown · room unknown"),
                };
                if let Some(over) = at.past_rating_w {
                    ui.colored_label(hazard(), format!("{} past its rating", watts(over)));
                }
            });
        });
    }
    ui.label(match seen.recoil_m_s2 {
        0.0 => "no net thrust".to_string(),
        a => format!("recoil {:.3} g away from the beam", a / G0),
    });
    let me = |j: f64| match j / module_j {
        0.0 => "0 ME".to_string(),
        me if me < 0.01 => format!("{me:.2e} ME"),
        me => format!("{me:.2} ME"),
    };
    ui.label(format!(
        "{} at {} · {} from heat, {} from storage",
        me(seen.cost_j),
        watts(seen.drawn_w),
        me(seen.from_heat_j),
        me(seen.from_storage_j),
    ));
}

/// The spot, the lead uncertainty about it and the receiver's hull, to one scale.
fn spot(ui: &mut egui::Ui, at: &Landing, receiver: &Receiver) {
    const RADIUS_PX: f32 = 36.0;
    let (rect, painter) = ui.allocate_painter(egui::vec2(2.0 * RADIUS_PX + 8.0, 2.0 * RADIUS_PX + 8.0), egui::Sense::hover());
    let center = rect.rect.center();
    let hull_m = 0.5 * receiver.length_m;
    let widest = at.spot_m.max(at.lead_m.unwrap_or(0.0)).max(hull_m).max(f64::MIN_POSITIVE);
    let px = |m: f64| (m / widest) as f32 * RADIUS_PX;
    let text = ui.visuals().text_color();
    painter.circle_stroke(center, px(at.spot_m).max(0.5), egui::Stroke::new(1.5, hazard()));
    if let Some(lead) = at.lead_m {
        painter.circle_stroke(center, px(lead).max(0.5), egui::Stroke::new(1.0, ui.visuals().weak_text_color()));
    }
    painter.circle_filled(center, px(hull_m).max(1.0), text);
}

fn incoming(ui: &mut egui::Ui, session: &crate::session::Session, uplink: &crate::uplink::Uplink, draft: &mut Draft) {
    ui.label("Incoming");
    if uplink.beams.incoming.is_empty() {
        ui.weak("none");
        return;
    }
    let now_s = session.coordinate_time_s();
    let fitting = session.ship.fitting();
    let absorptivity = fitting.map_or(1.0, |f| f.absorptivity_at(now_s));
    for landing in &uplink.beams.incoming {
        let look = crate::ui::Look::aimed_at(landing.bearing).unwrap_or_default();
        let (yaw_deg, pitch_deg) = (look.yaw.to_degrees(), look.pitch.to_degrees());
        let row = format!(
            "{yaw_deg:.1}° {pitch_deg:+.1}° · {} · {} · absorbed {}",
            band(landing.spectrum),
            watts(landing.power_w),
            watts(landing.power_w * absorptivity)
        );
        let chosen = draft.aimed == Aimed::Bearing { yaw_deg, pitch_deg };
        if ui.selectable_label(chosen, row).clicked() {
            draft.aimed = Aimed::Bearing { yaw_deg, pitch_deg };
        }
    }
    if let Some(fitting) = fitting {
        let (stored_w, heat_w) = field_takes(fitting, &session.ship.motion, now_s, uplink.beams.landing_w());
        ui.label(format!("to storage {} · to heat {}", watts(stored_w), watts(heat_w)));
    }
}

/// Of `arriving_w`, what this field stores and what it keeps as heat, W.
fn field_takes(fitting: &lc_world::fitting::Fitting, motion: &lc_world::motion::ShipState, now_s: f64, arriving_w: f64) -> (f64, f64) {
    let caps = fitting.capacities_at(now_s);
    let room_j = caps.storage_j - fitting.stored_j_at(motion, now_s);
    let segment = Segment {
        arriving_w,
        absorptivity: fitting.absorptivity_at(now_s),
        internal_w: 0.0,
        rating_w: caps.aperture_w,
        efficiency: fitting.balance().conversion_efficiency,
        room_j,
        draw_w: 0.0,
        emitted_w: 0.0,
    };
    let stored_w = if room_j > 0.0 { segment.stored_w() } else { 0.0 };
    (stored_w, segment.absorbed_w() - stored_w)
}

#[cfg(test)]
mod tests {
    use lc_world::fitting::{Account, Fitting};
    use lc_world::signal::cone_solid_angle_sr;

    use super::*;

    const B: Balance = Balance::DEFAULT;
    const AU_M: f64 = 1.495_978_707e11;

    /// The plate with one of its pair turned to fire fore: rated alike at both ends.
    fn two_ended() -> Form {
        use lc_world::form::{Mount, PartId, Placement};
        let mut form = lc_world::form::presets::Builtin::Plate.form();
        let engine = form.parts.iter_mut().find(|p| p.id == PartId(3)).unwrap();
        let Some(Placement { mount: Mount::Attached { anchor, .. }, .. }) = engine.placement.as_mut() else { panic!() };
        anchor.x = -anchor.x;
        form
    }

    fn ship(form: Form, heat_j: Option<f64>) -> Craft {
        let mut session = crate::session::Session::new(&lc_world::sky::AuthoredStars::sample(), 3);
        let full = Fitting::full(form, B, 0.0);
        let fitting = match heat_j {
            Some(heat_j) => Fitting::from_account(&Account { heat_j, ..full.account() }, B),
            None => full,
        };
        session.ship.fit(Some(fitting));
        session.ship
    }

    fn draft(apertures: Apertures, power_w: f64) -> Draft {
        Draft { apertures, power_w, spread_rad: 0.0, duration_s: 10.0, ..Draft::default() }
    }

    fn receiver(distance_m: f64) -> Receiver {
        Receiver {
            distance_m,
            blind_s: 2.0 * distance_m / C_M_S,
            accel_m_s2: Some(5.0 * G0),
            shadow_m2: broadside_m2(500.0),
            length_m: 500.0,
            rating_w: None,
            absorptivity: Some(1.0),
        }
    }

    fn widest_face_m(form: &Form) -> f64 {
        let (fore, aft) = ends(form, &B).unwrap();
        fore.diameter_m.max(aft.diameter_m)
    }

    #[test]
    fn the_spread_is_never_under_the_diffraction_floor() {
        let craft = ship(two_ended(), None);
        let asked = draft(Apertures::Both, 1.0e15);
        let floor = Transmitter::new(asked.wavelength_m, widest_face_m(&two_ended())).half_angle_rad();
        let narrowest = ends(&two_ended(), &B).map(|(f, a)| f.diameter_m.min(a.diameter_m)).unwrap();
        let floor = floor.max(Transmitter::new(asked.wavelength_m, narrowest).half_angle_rad());
        let seen = preview(&asked, &craft, 0.0, None);
        assert_eq!(seen.floor_rad, floor);
        assert_eq!(seen.spread_rad, floor, "asked for none, lit at the floor");
        let under = preview(&Draft { spread_rad: 0.1 * floor, ..asked }, &craft, 0.0, None);
        assert_eq!(under.spread_rad, floor);
        let wider = preview(&Draft { spread_rad: 10.0 * floor, ..asked }, &craft, 0.0, None);
        assert_eq!(wider.spread_rad, 10.0 * floor, "a wider beam is a choice");
    }

    #[test]
    fn the_spot_is_the_spread_times_the_distance() {
        let craft = ship(two_ended(), None);
        let asked = Draft { spread_rad: 1.0e-6, ..draft(Apertures::Both, 1.0e15) };
        let at = |d: f64, spread_rad: f64| {
            preview(&Draft { spread_rad, ..asked }, &craft, 0.0, Some(&receiver(d))).at.unwrap().spot_m
        };
        assert!((at(AU_M, 1.0e-6) - 1.0e-6 * AU_M).abs() < 1.0e-9 * AU_M);
        assert!((at(2.0 * AU_M, 1.0e-6) / at(AU_M, 1.0e-6) - 2.0).abs() < 1.0e-12);
        assert!((at(AU_M, 4.0e-6) / at(AU_M, 1.0e-6) - 4.0).abs() < 1.0e-9);
    }

    /// 31 §Spread's table, from what the window is told of a 5 g target seen by its freshest light.
    #[test]
    fn the_lead_uncertainty_is_31s_table() {
        let craft = ship(two_ended(), None);
        for (light_s, want) in [(1.0, "1e2"), (10.0, "1e4"), (60.0, "3.5e5")] {
            let seen = preview(&draft(Apertures::Both, 1.0e15), &craft, 0.0, Some(&receiver(light_s * C_M_S)));
            let lead = seen.at.unwrap().lead_m.unwrap();
            let digits = if want.contains('.') { 1 } else { 0 };
            assert_eq!(format!("{lead:.digits$e}"), want, "{light_s} light-seconds");
            let half_a_t2 = 0.5 * 5.0 * G0 * (2.0 * light_s).powi(2);
            assert!((lead / half_a_t2 - 1.0).abs() < 1.0e-6, "{lead} against ½ a (2d/c)² = {half_a_t2}");
        }
    }

    /// Read off a contact: blind for its light's age and the beam's flight, its rating and pace from
    /// its form, its shade from its glow, and unknown where nothing was said.
    #[test]
    fn a_receiver_is_what_its_light_says() {
        let at_m = C_M_S;
        let presence = lc_proto::Presence {
            ship_id: ShipId(2),
            name: "Vela".into(),
            length_m: 500.0,
            at_ly: [at_m / M_PER_LY, 0.0, 0.0],
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            jet_power_w: 0.0,
            emitted_t: 0,
            arrive_t: 1_000_000,
            form: (&two_ended()).into(),
            building: None,
            glow: Some(lc_proto::Glow { temperature_k: 300.0, shade: Shade::Clear }),
            glare: None,
        };
        let contact = Contact::seen(presence.clone(), None);
        let seen = Receiver::of(&contact, DVec3::ZERO, 1.0, &B);
        assert!((seen.distance_m - at_m).abs() < 1.0e-3);
        assert!((seen.blind_s - 2.0).abs() < 1.0e-9, "{}", seen.blind_s);
        assert_eq!(seen.rating_w, Some(Capacities::of(&two_ended(), &B).aperture_w));
        assert_eq!(seen.absorptivity, Some(B.clear_absorptivity));
        let dry_kg = dry_mass_kg(&two_ended(), &B);
        assert_eq!(seen.accel_m_s2, Some(aft_aperture_w(&two_ended(), &B).unwrap() / (dry_kg * C_M_S)));

        let bare = Contact::seen(lc_proto::Presence { form: lc_proto::Form::default(), glow: None, ..presence }, None);
        let seen = Receiver::of(&bare, DVec3::ZERO, 1.0, &B);
        assert_eq!((seen.rating_w, seen.absorptivity, seen.accel_m_s2), (None, None, None));
    }

    #[test]
    fn the_fraction_arriving_is_received_fraction() {
        let craft = ship(two_ended(), None);
        let asked = draft(Apertures::Both, 1.0e15);
        let far = receiver(100.0 * AU_M);
        let seen = preview(&asked, &craft, 0.0, Some(&far));
        let at = seen.at.unwrap();
        let want = received_fraction(seen.spread_rad, far.shadow_m2, far.distance_m);
        assert!(want < 1.0, "premise: the spot is wider than the ship");
        assert_eq!(at.fraction, want);
        assert_eq!(at.arriving_w, asked.power_w * want);
        let spot_m2 = cone_solid_angle_sr(seen.spread_rad) * far.distance_m.powi(2);
        assert!((at.fraction - far.shadow_m2 / spot_m2).abs() < 1.0e-12);
    }

    /// The shadow the window assumes is the one 31 §What arrives is worked for.
    #[test]
    fn the_broadside_is_31s() {
        let floor = Transmitter::new(1.0e-6, 100.0).half_angle_rad();
        assert_eq!(format!("{:.0e}", received_fraction(floor, broadside_m2(500.0), AU_M)), "7e-2");
        assert_eq!(format!("{:.0e}", received_fraction(floor, broadside_m2(500.0), 100.0 * AU_M)), "7e-6");
    }

    /// The split the account itself makes: a hot ship pays from heat, a cold one draws only what heat
    /// it makes meanwhile, the rest from storage.
    #[test]
    fn heat_pays_first() {
        let module_j = B.module_energy_j();
        let hot = ship(two_ended(), Some(5.0 * module_j));
        let seen = preview(&draft(Apertures::Both, 1.0e16), &hot, 0.0, None);
        assert!(seen.from_storage_j <= 1.0e-9 * seen.cost_j, "{seen:?}");
        assert!((seen.from_heat_j - seen.cost_j).abs() <= 1.0e-9 * seen.cost_j);

        let cold = ship(two_ended(), Some(0.0));
        let asked = draft(Apertures::Both, 1.0e19);
        let seen = preview(&asked, &cold, 0.0, None);
        let fitting = cold.fitting().unwrap();
        let caps = fitting.capacities_at(0.0);
        let segment = Segment {
            arriving_w: fitting.starlight_w() + fitting.lit_w(),
            absorptivity: fitting.absorptivity_at(0.0),
            internal_w: caps.drain_w,
            rating_w: caps.aperture_w,
            efficiency: B.conversion_efficiency,
            room_j: caps.storage_j - fitting.stored_j_at(&cold.motion, 0.0),
            draw_w: caps.drain_w,
            emitted_w: seen.drawn_w,
        };
        let want = fitting.field().settle(&segment, 0.0, asked.duration_s).from_heat_j;
        assert!(want > 0.0 && want < 0.5 * seen.cost_j, "premise: heat pays some and storage the rest, {want}");
        assert!((seen.from_heat_j - want).abs() <= 1.0e-6 * seen.cost_j, "{} against {want}", seen.from_heat_j);
        assert_eq!(seen.from_heat_j + seen.from_storage_j, seen.cost_j);
    }

    #[test]
    fn a_balanced_emit_has_no_recoil() {
        let craft = ship(two_ended(), None);
        let power_w = 1.0e18;
        assert_eq!(preview(&draft(Apertures::Both, power_w), &craft, 0.0, None).recoil_m_s2, 0.0);
        let aft = preview(&draft(Apertures::Aft, power_w), &craft, 0.0, None);
        assert_eq!(aft.recoil_m_s2, power_w / (craft.mass_kg_at(0.0) * C_M_S));
        assert_eq!(aft.drawn_w, power_w, "one end draws what it sends");
        assert_eq!(preview(&draft(Apertures::Both, power_w), &craft, 0.0, None).drawn_w, 2.0 * power_w);
    }

    #[test]
    fn feeding_past_a_receivers_rating_warns() {
        let craft = ship(two_ended(), None);
        let close = |rating_w, absorptivity| Receiver { rating_w, absorptivity, ..receiver(1.0e3) };
        let past = |power_w, r: Receiver| preview(&draft(Apertures::Both, power_w), &craft, 0.0, Some(&r)).at.unwrap().past_rating_w;
        let rating = Some(1.0e15);
        assert_eq!(past(3.0e15, close(rating, Some(1.0))), Some(2.0e15));
        assert_eq!(past(5.0e14, close(rating, Some(1.0))), None);
        assert_eq!(past(3.0e15, close(rating, Some(B.clear_absorptivity))), None, "a Clear receiver takes 30%");
        assert_eq!(past(3.0e15, close(rating, None)), Some(2.0e15), "an unknown shade is taken as Black");
        assert_eq!(past(3.0e15, close(None, Some(1.0))), None, "an unknown rating warns of nothing");
    }

    #[test]
    fn what_the_server_would_refuse_is_said() {
        let starting = ship(Form::starting(), None);
        let both = preview(&draft(Apertures::Both, 1.0e15), &starting, 0.0, None);
        assert_eq!(both.refusal, Some(Refusal::NoAperture));
        let aft = preview(&draft(Apertures::Aft, 1.0e15), &starting, 0.0, None);
        assert_eq!(both.floor_rad, aft.floor_rad, "an end with no engines floors nothing");
        assert!(aft.floor_rad < 1.0e-6, "{}", aft.floor_rad);
        assert_eq!(offered(ends(&Form::starting(), &B)), vec![Apertures::Aft]);
        assert_eq!(offered(ends(&two_ended(), &B)), vec![Apertures::Fore, Apertures::Aft, Apertures::Both]);
        let craft = ship(two_ended(), None);
        let (fore, _) = ends(&two_ended(), &B).unwrap();
        assert_eq!(preview(&draft(Apertures::Both, 1.0e15), &craft, 0.0, None).refusal, None);
        assert_eq!(preview(&draft(Apertures::Fore, 2.0 * fore.rating_w), &craft, 0.0, None).refusal, Some(Refusal::OverRating));
        let forever = Draft { duration_s: 1.0e12, ..draft(Apertures::Both, fore.rating_w) };
        assert_eq!(preview(&forever, &craft, 0.0, None).refusal, Some(Refusal::NoEnergy));
    }

    #[test]
    fn an_illuminated_is_restated_and_put_out() {
        let line = Spectrum::Line { wavelength_m: 1.0e-6 };
        let mut beams = Beams::default();
        beams.illuminated(4, [0.0, 2.0, 0.0], line, 1.0e18, 10.0);
        beams.illuminated(5, [1.0, 0.0, 0.0], line, 3.0e17, 10.0);
        assert_eq!(beams.incoming[0].bearing, DVec3::Y);
        beams.illuminated(4, [0.0, 1.0, 0.0], line, 5.0e17, 12.0);
        beams.illuminated(4, [0.0, 1.0, 0.0], line, 9.0e17, 11.0);
        assert_eq!(beams.incoming.len(), 2, "one entry a beam");
        assert_eq!(beams.landing_w(), 8.0e17, "restated, and an older statement dropped");
        beams.illuminated(4, [0.0, 1.0, 0.0], line, 0.0, 13.0);
        assert_eq!(beams.incoming.iter().map(|i| i.beam).collect::<Vec<_>>(), vec![5]);
        beams.illuminated(4, [0.0, 1.0, 0.0], line, 0.0, 14.0);
        assert_eq!(beams.incoming.len(), 1, "a zero for a beam not held adds nothing");
    }

    #[test]
    fn beams_are_drawn_as_far_as_their_light() {
        let mut beams = Beams::default();
        beams.sent.push(Sent {
            event_id: 9,
            axis: DVec3::X,
            apertures: Apertures::Both,
            half_angle_rad: 1.0e-3,
            power_w: 1.0e18,
            from_s: 10.0,
            until_s: 100.0,
        });
        beams.illuminated(4, [0.0, 1.0, 0.0], Spectrum::Line { wavelength_m: 1.0e-6 }, 1.0e18, 10.0);
        let drawn = on_map(&beams, DVec3::ZERO, 12.0, 1.0e12);
        let keys: Vec<OnMap> = drawn.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![OnMap::Sent(9, false), OnMap::Sent(9, true), OnMap::Landing(4)]);
        assert_eq!(drawn[0].1.length_m, 2.0 * C_M_S);
        assert_eq!((drawn[0].1.aft, drawn[1].1.aft), (DVec3::X, DVec3::NEG_X), "a balanced emit lights both ways");
        assert_eq!((drawn[2].1.aft, drawn[2].1.half_angle_rad, drawn[2].1.length_m), (DVec3::Y, 0.0, 1.0e12));
        beams.put_out(50.0);
        assert!(on_map(&beams, DVec3::ZERO, 60.0, 1.0e12).iter().all(|(k, _)| matches!(k, OnMap::Landing(_))));
    }
}
