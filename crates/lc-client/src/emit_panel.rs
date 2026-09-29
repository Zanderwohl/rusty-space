//! The emit window: one order for dumping heat, feeding an ally and attacking, previewed before it
//! is sent, and every beam landing on this ship. See `lightcone/docs/31-directed-energy.md` §Client.
//!
//! [`preview`] and [`Beams`] carry no engine, so what the window says is tested without one. The
//! numbers are `lc_world`'s: the server lights the same spread from the same ends.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::egui;
use glam::DVec3;
use lc_proto::{Aim, Apertures, Lead, Order, Outbound, Refusal, ShipId, Shade, Spectrum};
use lc_world::craft::{BEAM_PER_LENGTH, Craft};
use lc_world::emit::{Boost, lead_uncertainty_m, received_fraction};
use lc_world::field::{Field, Segment};
use lc_world::fitting::{Balance, Lit};
use lc_world::flight::{C_M_S, G0};
use lc_world::form::Form;
use lc_world::form::capacity::{Capacities, End, aft_aperture_w, dry_mass_kg, ends};
use lc_world::pursuit::Sighting;
use lc_world::signal::Transmitter;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::field::Incoming;
use crate::input::Requested;
use crate::panels::{ask, duration, span_m};
use crate::plume::Drawn;
use crate::system::M_PER_LY;
use crate::uplink::Contact;

/// What the window sends. The order takes 1 nm to 3 cm, but wavelength buys only a tighter floor
/// and costs nothing (31 §Open), so it is not offered until it is a trade.
pub const WAVELENGTH_M: f64 = lc_world::emit::WAVELENGTH_M;
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
    pub lead: Lead,
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
            wavelength_m: WAVELENGTH_M,
            spread_rad: 0.0,
            duration_s: 600.0,
            lead: Lead::Coasting,
            followed: None,
        }
    }
}

/// What this ship knows of the craft it aims at. `None` is unknown, and the window says so.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Receiver {
    /// Unit: where the beam is sent, led as the draft says.
    pub axis: DVec3,
    /// To where the lead puts it when the beam lands.
    pub distance_m: f64,
    /// From the light it was last seen by leaving it to the beam landing on it, seconds.
    pub blind_s: f64,
    /// Its proper acceleration as two statements measured it, m/s², while its plume was lit.
    pub burn_m_s2: Option<f64>,
    /// The most it can accelerate at, m/s²: its aft rating over its dry mass.
    pub accel_m_s2: Option<f64>,
    /// Broadside to the beam, the most it can present.
    pub shadow_m2: f64,
    pub length_m: f64,
    /// Its engines' conversion rating, W.
    pub rating_w: Option<f64>,
    pub absorptivity: Option<f64>,
    /// Of what it converts, the share stored: the same for every craft.
    pub efficiency: f64,
    pub field: Option<Heated>,
}

/// A field as its glow shows it: its envelope and the heat its temperature means there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heated {
    pub field: Field,
    pub heat_j: f64,
}

impl Receiver {
    /// `contact` as seen from `here_ly` at `now_s`, led by `lead` along what `beams` watched of
    /// it, its form and glow read with `balance`.
    pub fn of(contact: &Contact, beams: &Beams, lead: Lead, here_ly: DVec3, now_s: f64, balance: &Balance) -> Self {
        let (sighting, burn) = beams.watched(contact);
        let accel = match lead {
            Lead::Burning => burn.unwrap_or(DVec3::ZERO),
            Lead::Coasting => DVec3::ZERO,
        };
        let led = lc_world::emit::lead(here_ly, now_s, &sighting, accel);
        let (axis, distance_m, blind_s) = match led {
            Some(led) => (led.axis, led.at_ly.distance(here_ly) * M_PER_LY, led.arrive_s - sighting.emitted_s),
            // Outrunning the beam, as far as the sighting says: aimed where it appears.
            None => {
                let d = contact.position_ly.distance(here_ly) * M_PER_LY;
                ((contact.position_ly - here_ly).normalize_or(DVec3::X), d, (now_s - contact.emitted_s).max(0.0) + d / C_M_S)
            }
        };
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
        let field = formed.then(|| {
            let field = Field::of(contact.glow.envelope_m2, balance);
            Heated { field, heat_j: field.heat_j_at(contact.glow.temperature_k) }
        });
        Self {
            axis,
            distance_m,
            blind_s,
            burn_m_s2: burn.map(|a| a.length() * C_M_S),
            accel_m_s2,
            shadow_m2: broadside_m2(contact.length_m),
            length_m: contact.length_m,
            rating_w,
            absorptivity,
            efficiency: balance.conversion_efficiency,
            field,
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
    /// Its field under this beam and whatever held it at its glow, if storage is full, so it is
    /// all heat.
    pub if_full: Option<Fate>,
    /// And if storage has room, so it converts up to its rating first.
    pub with_room: Option<Fate>,
}

/// What a beam does to a field over its duration.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fate {
    /// Seconds from the beam's arrival.
    Collapses(f64),
    /// Of `Q_max`, at the beam's end.
    Reaches(f64),
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
    } else if ship.motion.is_under_way() || is_lit(ship, now_s) {
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
        at: receiver.map(|r| landing(r, draft.power_w, spread_rad, draft.duration_s)),
    }
}

fn landing(receiver: &Receiver, power_w: f64, spread_rad: f64, duration_s: f64) -> Landing {
    let fraction = received_fraction(spread_rad, receiver.shadow_m2, receiver.distance_m);
    let arriving_w = power_w * fraction;
    let absorbed_w = receiver.absorptivity.map(|a| a * arriving_w);
    // An unknown shade taken as Black, which takes all of it.
    let taken_w = absorbed_w.unwrap_or(arriving_w);
    Landing {
        distance_m: receiver.distance_m,
        spot_m: spread_rad.min(SPREAD_MAX_RAD).sin() * receiver.distance_m,
        lead_m: receiver.accel_m_s2.map(|a| lead_uncertainty_m(a, receiver.blind_s)),
        fraction,
        arriving_w,
        absorbed_w,
        past_rating_w: receiver.rating_w.map(|rating| taken_w - rating).filter(|&w| w > 0.0),
        if_full: receiver.field.map(|heated| fate(&heated, taken_w, duration_s)),
        with_room: receiver.field.zip(receiver.rating_w).map(|(heated, rating_w)| {
            fate(&heated, taken_w - taken_w.min(rating_w) * receiver.efficiency, duration_s)
        }),
    }
}

/// `heat_w` more on a field steady at its glow: whatever held it there, `Q/τ`, goes on.
fn fate(heated: &Heated, heat_w: f64, duration_s: f64) -> Fate {
    let (field, max_j) = (heated.field, heated.field.heat_max_j());
    let power_w = heat_w + heated.heat_j / field.tau_s;
    match field.time_to_rise_s(heated.heat_j, max_j, power_w).filter(|&t| t <= duration_s) {
        Some(t) => Fate::Collapses(t),
        None => Fate::Reaches(field.heat_after_j(heated.heat_j, power_w, duration_s) / max_j),
    }
}

/// Whether a balanced emit is lit, which moves nothing, or a one-ended one is flying.
pub fn is_lit(ship: &Craft, now_s: f64) -> bool {
    let balanced = ship.fitting().is_some_and(|f| f.lit().iter().any(|l| l.from_s <= now_s && now_s < l.until_s));
    balanced || matches!(ship.motion.motive, lc_world::motion::Motive::Boosting(b) if !b.has_ended(now_s))
}

/// Of `drawn_w` for `duration_s`, what storage pays, joules: the ship's own account run with and
/// without it, so heat goes first exactly as the server draws it.
fn source_j(ship: &Craft, now_s: f64, drawn_w: f64, duration_s: f64) -> f64 {
    let Some(fitting) = ship.fitting() else { return 0.0 };
    let mut dark = fitting.clone();
    dark.settle(&ship.motion, now_s);
    let mut lit = dark.clone();
    lit.light(Lit { from_s: now_s, until_s: now_s + duration_s, power_w: drawn_w, half_angle_rad: 0.0 });
    let end_s = now_s + duration_s;
    (dark.stored_j_at(&ship.motion, end_s) - lit.stored_j_at(&ship.motion, end_s)).max(0.0)
}

/// What the window sends: the fields of an `Order::Emit`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emission {
    pub aim: Aim,
    pub apertures: Apertures,
    pub power_w: f64,
    pub wavelength_m: f64,
    pub spread_rad: f64,
    pub duration_s: f64,
    pub lead: Lead,
}

impl Emission {
    pub fn order(self) -> Order {
        let Emission { aim, apertures, power_w, wavelength_m, spread_rad, duration_s, lead } = self;
        Order::Emit { aim, apertures, power_w, wavelength_m, spread_rad, duration_s, lead }
    }
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

/// Back along `bearing`, in the azimuth range the window's field holds, `[0, 360)`: a negative
/// azimuth would be clamped to zero there.
pub fn aimed_back(bearing: DVec3) -> Aimed {
    let look = crate::ui::Look::aimed_at(bearing).unwrap_or_default();
    Aimed::Bearing { yaw_deg: look.yaw.to_degrees().rem_euclid(360.0), pitch_deg: look.pitch.to_degrees() }
}


/// A beam this ship has lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sent {
    pub event_id: i64,
    pub axis: DVec3,
    pub apertures: Apertures,
    pub half_angle_rad: f64,
    pub power_w: f64,
    /// Where the ship was when it was accepted.
    pub from_ly: DVec3,
    pub from_s: f64,
    pub until_s: f64,
}

/// The only beams this ship knows of: those landing on it and those it lit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beams {
    pub sent: Vec<Sent>,
    /// Every craft in sight by its last two statements, which is what a burn is measured from.
    pub watched: HashMap<ShipId, Watch>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Watch {
    pub latest: Sighting,
    pub previous: Option<Sighting>,
    /// Its plume, at the latest.
    pub lit: bool,
}

/// Whether what `presence` states was pushing it: its drive, or an emit out of one end more than
/// the other.
fn pushed(presence: &lc_proto::Presence) -> bool {
    presence.drive_w > 0.0 || presence.emit_fore_w != presence.emit_aft_w
}

impl Watch {
    /// Its proper acceleration, light-seconds per second squared, as an escort measures its
    /// quarry's: only while its plume is lit.
    pub fn burn(&self) -> Option<DVec3> {
        self.lit.then(|| self.previous.and_then(|p| lc_world::escort::acceleration_of(&p, &self.latest))).flatten()
    }
}

impl Beams {
    /// What `message` says about this ship's beams, `me` being this ship.
    pub fn fold(&mut self, message: &Outbound, me: Option<ShipId>, here_ly: DVec3, contacts: &[Contact]) {
        match message {
            Outbound::Accepted { ship_id, event_id, at_t, order } if me == Some(*ship_id) => match order {
                Order::Emit { .. } => self.lit(order, *event_id, *at_t as f64 * 1.0e-6, here_ly, contacts),
                Order::CutDrive => self.put_out(*at_t as f64 * 1.0e-6),
                _ => {}
            },
            Outbound::Present(seen) => {
                let mut watched = HashMap::with_capacity(seen.len());
                for presence in seen.iter().map(|c| c.get()) {
                    let latest = sighting_of(presence);
                    let watch = match self.watched.get(&presence.ship_id) {
                        Some(held) if held.latest.emitted_s >= latest.emitted_s => *held,
                        held => Watch { latest, previous: held.map(|h| h.latest), lit: pushed(presence) },
                    };
                    watched.insert(presence.ship_id, watch);
                }
                self.watched = watched;
            }
            Outbound::Collapsed { .. } => *self = Beams::default(),
            _ => {}
        }
    }

    /// Where `contact` was last stated to be, and the burn it was measured in.
    pub fn watched(&self, contact: &Contact) -> (Sighting, Option<DVec3>) {
        match self.watched.get(&contact.ship_id) {
            Some(watch) => (watch.latest, watch.burn()),
            None => (
                Sighting {
                    target: lc_world::motion::ShipId(contact.ship_id.0),
                    position_ly: contact.position_ly,
                    beta: contact.beta,
                    length_m: contact.length_m,
                    emitted_s: contact.emitted_s,
                },
                None,
            ),
        }
    }

    /// The axis `aim` is sent along from `here_ly` at `at_s`, led as `lead` says, as this ship sees
    /// it.
    pub fn axis(&self, aim: &Aim, lead: Lead, here_ly: DVec3, at_s: f64, contacts: &[Contact]) -> Option<DVec3> {
        match aim {
            Aim::Ship(id) => {
                let contact = contacts.iter().find(|c| c.ship_id == *id)?;
                let (sighting, burn) = self.watched(contact);
                let accel = if lead == Lead::Burning { burn.unwrap_or(DVec3::ZERO) } else { DVec3::ZERO };
                lc_world::emit::lead(here_ly, at_s, &sighting, accel).map(|led| led.axis)
            }
            Aim::Bearing(b) => DVec3::from_array(*b).try_normalize(),
            Aim::Omni | Aim::Star(_) => None,
        }
    }

    /// Fold this ship's accepted `Order::Emit`, aimed as this ship sees its target from `here_ly`.
    pub fn lit(&mut self, order: &Order, event_id: i64, at_s: f64, here_ly: DVec3, contacts: &[Contact]) {
        let Order::Emit { aim, apertures, power_w, spread_rad, duration_s, lead, .. } = *order else { return };
        let Some(axis) = self.axis(&aim, lead, here_ly, at_s, contacts) else { return };
        self.sent.retain(|s| s.until_s > at_s);
        self.sent.push(Sent {
            event_id,
            axis,
            apertures,
            half_angle_rad: spread_rad,
            power_w,
            from_ly: here_ly,
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
}

fn sighting_of(presence: &lc_proto::Presence) -> Sighting {
    Sighting {
        target: lc_world::motion::ShipId(presence.ship_id.0),
        position_ly: DVec3::from_array(presence.at_ly),
        beta: DVec3::from_array(presence.beta),
        length_m: presence.length_m,
        emitted_s: presence.emitted_t as f64 * 1.0e-6,
    }
}

/// [`Beams::fold`] for the ship `uplink` flies.
pub fn fold(uplink: &mut crate::uplink::Uplink, message: &Outbound, here_ly: DVec3) {
    let me = uplink.joined().map(|j| j.ship_id);
    uplink.beams.fold(message, me, here_ly, &uplink.contacts);
}

/// Which beam a map line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OnMap {
    /// Its event, and whether this is a balanced emit's aft beam.
    Sent(i64, bool),
    Landing(i64),
}

/// This ship's beams as cones from where they lit, each as far as its light has got and no further
/// than `reach_m`, and each beam landing at `here_ly` as a line `reach_m` long back along its bearing.
///
/// `boost` is the ship's own burn, if it is flying one: a one-ended emit lights once the nose has
/// come about and goes out when the burn does, which the acceptance does not say.
///
/// No cooking ring: its share of a cone that grows every frame would be a new mesh every frame.
pub fn on_map(
    beams: &Beams,
    incoming: &Incoming,
    here_ly: DVec3,
    boost: Option<&Boost>,
    now_s: f64,
    reach_m: f64,
) -> Vec<(OnMap, Drawn)> {
    // Keyed on the map by `OnMap`, so the jet is never read.
    let cone = |apex_ly: DVec3, axis: DVec3, length_m: f64, half_angle_rad: f64| Drawn {
        craft: None,
        jet: crate::plume::Jet::EmitAft,
        apex_ly,
        aft: axis,
        length_m,
        half_angle_rad,
        cooking_m: 0.0,
    };
    let mut drawn = Vec::new();
    for sent in &beams.sent {
        let (from_ly, from_s, until_s) = match (sent.apertures, boost) {
            // The same instant, both ends having read it off one microsecond count.
            (Apertures::Fore | Apertures::Aft, Some(b)) if (b.start_s - sent.from_s).abs() < 1.0e-6 => {
                (b.state_at(b.lights_s()).0, b.lights_s(), b.out_s().min(sent.until_s))
            }
            _ => (sent.from_ly, sent.from_s, sent.until_s),
        };
        if now_s < from_s || now_s >= until_s {
            continue;
        }
        let length_m = (C_M_S * (now_s - from_s)).min(reach_m);
        let mut ends = vec![(sent.axis, false)];
        if sent.apertures == Apertures::Both {
            ends.push((-sent.axis, true));
        }
        for (axis, aft) in ends {
            drawn.push((OnMap::Sent(sent.event_id, aft), cone(from_ly, axis, length_m, drawn_spread(sent.half_angle_rad))));
        }
    }
    for (id, landing) in incoming.by_id() {
        drawn.push((OnMap::Landing(id), cone(here_ly, landing.bearing, reach_m, 0.0)));
    }
    drawn
}

/// To a hundredth of a decade, so the map's meshes, one a shape, stay as few as the spreads it has
/// drawn rather than one an emit.
fn drawn_spread(half_angle_rad: f64) -> f64 {
    10f64.powf((half_angle_rad.log10() * 100.0).round() / 100.0)
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

/// [`span_m`], down past a meter: a spot at the floor is millimeters across at a few hundred km.
fn fine_m(m: f64) -> String {
    match m {
        m if m <= 0.0 => "0 m".to_string(),
        m if m < 1.0e-3 => format!("{:.1} µm", m * 1.0e6),
        m if m < 1.0 => format!("{:.1} mm", m * 1.0e3),
        m => span_m(m),
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
        Aimed::Craft(id) => uplink
            .contacts
            .iter()
            .find(|c| c.ship_id == id)
            .map(|c| Receiver::of(c, &uplink.beams, draft.lead, here_ly, now_s, &balance)),
        _ => None,
    };
    if matches!(draft.aimed, Aimed::Craft(_)) {
        lead_row(ui, draft, receiver.as_ref());
    }
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
        let emission = Emission {
            aim: aim_of(draft.aimed, state.0.look.forward()),
            apertures: draft.apertures,
            power_w: draft.power_w,
            wavelength_m: draft.wavelength_m,
            spread_rad: seen.spread_rad,
            duration_s: draft.duration_s,
            lead: draft.lead,
        };
        let unseen = matches!(draft.aimed, Aimed::Craft(_)) && receiver.is_none();
        if ui.add_enabled(seen.refusal.is_none() && !unseen && session.remote, egui::Button::new("Emit")).clicked() {
            ask(out, Action::Emit(emission));
        }
        match seen.refusal {
            _ if unseen => ui.colored_label(hazard(), "out of sight"),
            // The shared wording is a refit's.
            Some(Refusal::UnderWay) => ui.colored_label(hazard(), "under way"),
            Some(why) => ui.colored_label(hazard(), crate::uplink::refused(why)),
            None => ui.label(""),
        };
    });
    if is_lit(&session.ship, now_s) && ui.button("Put out").clicked() {
        ask(out, Action::AbortFlight);
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
                ui.label(format!("spot {} across", fine_m(2.0 * at.spot_m)));
                let lead = match at.lead_m {
                    Some(m) => format!("lead ±{}", fine_m(m)),
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
                match (at.if_full, at.with_room) {
                    (None, _) => {
                        ui.label("field unknown");
                    }
                    (Some(full), room) => {
                        fate_line(ui, "if full", full);
                        match room {
                            Some(room) => fate_line(ui, "with room", room),
                            None => {
                                ui.label("with room: unknown");
                            }
                        }
                    }
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

fn fate_line(ui: &mut egui::Ui, case: &str, fate: Fate) {
    match fate {
        Fate::Collapses(s) => ui.colored_label(hazard(), format!("{case}: collapses in {}", duration(s))),
        Fate::Reaches(share) => ui.label(format!("{case}: {:.0}% of collapse at the end", 100.0 * share)),
    };
}

/// Coasting or burning, and what its light last said it was doing.
fn lead_row(ui: &mut egui::Ui, draft: &mut Draft, receiver: Option<&Receiver>) {
    ui.horizontal(|ui| {
        for (label, lead) in [("coasting", Lead::Coasting), ("burning", Lead::Burning)] {
            if ui.selectable_label(draft.lead == lead, label).clicked() {
                draft.lead = lead;
            }
        }
        match receiver.and_then(|r| r.burn_m_s2) {
            Some(a) => ui.weak(format!("seen burning {:.2} g", a / G0)),
            None => ui.weak("seen coasting"),
        };
    });
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
    if uplink.incoming.beams().next().is_none() {
        ui.weak("none");
        return;
    }
    let now_s = session.coordinate_time_s();
    let fitting = session.ship.fitting();
    let absorptivity = fitting.map_or(1.0, |f| f.absorptivity_at(now_s));
    for landing in uplink.incoming.beams() {
        let back = aimed_back(landing.bearing);
        let Aimed::Bearing { yaw_deg, pitch_deg } = back else { continue };
        let row = format!(
            "{yaw_deg:.1}° {pitch_deg:+.1}° · {} · {} · absorbed {}",
            band(landing.spectrum),
            watts(landing.power_w),
            watts(landing.power_w * absorptivity)
        );
        if ui.selectable_label(draft.aimed == back, row).clicked() {
            draft.aimed = back;
        }
    }
    if let Some(fitting) = fitting {
        let (stored_w, heat_w) = field_takes(fitting, &session.ship.motion, now_s, uplink.incoming.landing_w());
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
            axis: DVec3::X,
            burn_m_s2: None,
            efficiency: B.conversion_efficiency,
            field: None,
            distance_m,
            blind_s: 2.0 * distance_m / C_M_S,
            accel_m_s2: Some(5.0 * G0),
            shadow_m2: broadside_m2(500.0),
            length_m: 500.0,
            rating_w: None,
            absorptivity: Some(1.0),
        }
    }

    #[test]
    fn the_spread_is_never_under_the_diffraction_floor() {
        let craft = ship(two_ended(), None);
        let asked = draft(Apertures::Both, 1.0e15);
        // Each end's floor, the higher of the two: the narrower face's.
        let narrowest = ends(&two_ended(), &B).map(|(f, a)| f.diameter_m.min(a.diameter_m)).unwrap();
        let floor = Transmitter::new(asked.wavelength_m, narrowest).half_angle_rad();
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
            drive_w: 0.0,
            emit_fore_w: 0.0,
            emit_aft_w: 0.0,
            emit_spread_rad: 0.0,
            emitted_t: 0,
            arrive_t: 1_000_000,
            form: (&two_ended()).into(),
            building: None,
            glow: Some(lc_proto::Glow { temperature_k: 300.0, shade: Shade::Clear, envelope_m2: 2.0e6 }),
            glare: None,
        };
        let contact = Contact::seen(presence.clone(), None);
        let seen = Receiver::of(&contact, &Beams::default(), Lead::Coasting, DVec3::ZERO, 1.0, &B);
        assert!((seen.distance_m - at_m).abs() < 1.0e-3);
        assert!((seen.blind_s - 2.0).abs() < 1.0e-9, "{}", seen.blind_s);
        assert_eq!(seen.rating_w, Some(Capacities::of(&two_ended(), &B).aperture_w));
        assert_eq!(seen.absorptivity, Some(B.clear_absorptivity));
        let dry_kg = dry_mass_kg(&two_ended(), &B);
        assert_eq!(seen.accel_m_s2, Some(aft_aperture_w(&two_ended(), &B).unwrap() / (dry_kg * C_M_S)));
        let field = Field::of(2.0e6, &B);
        assert_eq!(seen.field, Some(Heated { field, heat_j: field.heat_j_at(300.0) }), "its field from its glow");

        let bare = Contact::seen(lc_proto::Presence { form: lc_proto::Form::default(), glow: None, ..presence }, None);
        let seen = Receiver::of(&bare, &Beams::default(), Lead::Coasting, DVec3::ZERO, 1.0, &B);
        assert_eq!((seen.rating_w, seen.absorptivity, seen.accel_m_s2), (None, None, None));
        assert_eq!(seen.field, None, "a formless craft's glow is a stand-in");
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
    fn beams_are_drawn_as_far_as_their_light() {
        let mut beams = Beams::default();
        beams.sent.push(Sent {
            event_id: 9,
            axis: DVec3::X,
            apertures: Apertures::Both,
            half_angle_rad: 1.0e-3,
            power_w: 1.0e18,
            from_ly: DVec3::ZERO,
            from_s: 10.0,
            until_s: 100.0,
        });
        let mut incoming = Incoming::default();
        incoming.illuminated(ShipId(7), 4, [0.0, 2.0, 0.0], Spectrum::Line { wavelength_m: 1.0e-6 }, 1.0e18, 10_000_000);
        let drawn = on_map(&beams, &incoming, DVec3::ZERO, None, 12.0, 1.0e12);
        let keys: Vec<OnMap> = drawn.iter().map(|(k, _)| *k).collect();
        assert_eq!(keys, vec![OnMap::Sent(9, false), OnMap::Sent(9, true), OnMap::Landing(4)]);
        assert_eq!(drawn[0].1.length_m, 2.0 * C_M_S);
        assert_eq!((drawn[0].1.aft, drawn[1].1.aft), (DVec3::X, DVec3::NEG_X), "a balanced emit lights both ways");
        assert_eq!((drawn[2].1.aft, drawn[2].1.half_angle_rad, drawn[2].1.length_m), (DVec3::Y, 0.0, 1.0e12));
        beams.put_out(50.0);
        assert!(on_map(&beams, &incoming, DVec3::ZERO, None, 60.0, 1.0e12).iter().all(|(k, _)| matches!(k, OnMap::Landing(_))));
    }

    /// A source west of +X has a negative `atan2` azimuth, which the window's field, `[0, 360)`,
    /// would clamp to zero: aiming back must survive being shown there.
    #[test]
    fn aiming_back_along_a_bearing_points_at_its_source() {
        for bearing in [DVec3::NEG_Y, DVec3::new(-1.0, -1.0, 0.3).normalize(), DVec3::new(0.2, 0.5, -0.4).normalize()] {
            let Aimed::Bearing { yaw_deg, pitch_deg } = aimed_back(bearing) else { unreachable!() };
            assert!((0.0..360.0).contains(&yaw_deg), "{yaw_deg} would be clamped");
            let Aim::Bearing(sent) = aim_of(Aimed::Bearing { yaw_deg: yaw_deg.clamp(0.0, 360.0), pitch_deg }, DVec3::X) else {
                unreachable!()
            };
            assert!(DVec3::from_array(sent).distance(bearing) < 1.0e-9, "{sent:?} for {bearing}");
        }
    }

    /// A balanced emit moves nothing, so only the account says it is lit, and the server refuses a
    /// second while it is.
    #[test]
    fn a_lit_balanced_emit_refuses_another() {
        let mut craft = ship(two_ended(), None);
        assert!(!is_lit(&craft, 5.0));
        let mut fitting = craft.fitting().unwrap().clone();
        fitting.light(Lit { from_s: 0.0, until_s: 10.0, power_w: 2.0e15, half_angle_rad: 0.01 });
        craft.fit(Some(fitting));
        assert!(is_lit(&craft, 5.0) && !is_lit(&craft, 10.0));
        assert_eq!(preview(&draft(Apertures::Both, 1.0e15), &craft, 5.0, None).refusal, Some(Refusal::UnderWay));
    }

    /// A one-ended emit is a burn: its cone starts where and when the nose has come about, and goes
    /// out with the burn.
    #[test]
    fn a_one_ended_beam_lights_with_its_burn() {
        let boost = Boost::plan(DVec3::ZERO, DVec3::ZERO, 10.0, DVec3::X, DVec3::NEG_X, 0.05, 50.0, 0.01, DVec3::Y, 0.05);
        assert!(boost.turn_s() > 1.0, "premise: the nose has to come about");
        let mut beams = Beams::default();
        beams.sent.push(Sent {
            event_id: 9,
            axis: DVec3::NEG_X,
            apertures: Apertures::Fore,
            half_angle_rad: 1.0e-3,
            power_w: 1.0e18,
            from_ly: DVec3::ZERO,
            from_s: 10.0,
            until_s: 1.0e6,
        });
        let at = |now_s| on_map(&beams, &Incoming::default(), DVec3::ONE, Some(&boost), now_s, 1.0e15);
        assert!(at(boost.lights_s() - 0.5).is_empty(), "drawn before it lit");
        let lit = at(boost.lights_s() + 2.0);
        assert!((lit[0].1.length_m - 2.0 * C_M_S).abs() < 1.0e-3 * C_M_S);
        assert_eq!(lit[0].1.apex_ly, boost.state_at(boost.lights_s()).0, "from where it lit, not where the ship is");
        assert!(at(boost.out_s() + 1.0).is_empty(), "drawn after the burn");
    }

    #[test]
    fn beam_spreads_come_in_a_few_shapes() {
        let drawn: std::collections::BTreeSet<u64> =
            (0..1000).map(|i| drawn_spread(1.0e-3 * (1.0 + i as f64 * 1.0e-4)).to_bits()).collect();
        // A tenth is 0.041 of a decade: five hundredths at most, for a thousand spreads.
        assert!(drawn.len() <= 5, "{} meshes for a thousand spreads within 10%", drawn.len());
        assert!((drawn_spread(0.1) / 0.1 - 1.0).abs() < 0.012);
    }

    fn presence(emitted_s: f64, at_ls: DVec3, beta: DVec3, drive_w: f64) -> lc_proto::Presence {
        lc_proto::Presence {
            ship_id: ShipId(2),
            name: "Vela".into(),
            length_m: 500.0,
            at_ly: (at_ls / lc_world::flight::JULIAN_YEAR_S).to_array(),
            beta: beta.to_array(),
            facing: [0.0, 1.0, 0.0],
            drive_w,
            emit_fore_w: 0.0,
            emit_aft_w: 0.0,
            emit_spread_rad: 0.0,
            emitted_t: (emitted_s * 1.0e6) as i64,
            arrive_t: (emitted_s * 1.0e6) as i64,
            form: lc_proto::Form::default(),
            building: None,
            glow: None,
            glare: None,
        }
    }

    fn present(p: lc_proto::Presence) -> Outbound {
        let arrive_t = p.arrive_t;
        Outbound::Present(vec![lc_proto::Cleared::<lc_proto::Presence>::clear(p, arrive_t).unwrap()])
    }

    /// Two statements of a lit plume measure its burn, and a burning lead meets the craft where the
    /// burn carries it: `½ a T²` along from where coasting would, `T` from the light it was seen by.
    #[test]
    fn a_burning_lead_is_measured_from_two_statements() {
        let a = 5.0 * G0;
        let v = |t: f64| DVec3::Y * a * t / C_M_S;
        let at = |t: f64| DVec3::new(60.0, 0.5 * a * t * t / C_M_S, 0.0);
        let mut beams = Beams::default();
        for t in [0.0, 1.0] {
            beams.fold(&present(presence(t, at(t), v(t), 1.0e18)), Some(ShipId(7)), DVec3::ZERO, &[]);
        }
        let contact = Contact::seen(presence(1.0, at(1.0), v(1.0), 1.0e18), None);
        let burning = Receiver::of(&contact, &beams, Lead::Burning, DVec3::ZERO, 61.0, &B);
        let coasting = Receiver::of(&contact, &beams, Lead::Coasting, DVec3::ZERO, 61.0, &B);
        assert!((burning.burn_m_s2.unwrap() / a - 1.0).abs() < 1.0e-6, "{:?}", burning.burn_m_s2);
        assert_eq!(coasting.burn_m_s2, burning.burn_m_s2, "what was seen does not depend on the choice");
        let apart_m = burning.axis.angle_between(coasting.axis) * burning.distance_m;
        let half_a_t2 = 0.5 * a * burning.blind_s * burning.blind_s;
        assert!((apart_m / half_a_t2 - 1.0).abs() < 1.0e-2, "{apart_m} against {half_a_t2}");

        let mut dark = Beams::default();
        for t in [0.0, 1.0] {
            dark.fold(&present(presence(t, at(t), v(t), 0.0)), Some(ShipId(7)), DVec3::ZERO, &[]);
        }
        assert_eq!(Receiver::of(&contact, &dark, Lead::Burning, DVec3::ZERO, 61.0, &B).burn_m_s2, None, "no plume, no burn");
    }

    /// 31 §Attacking: 1.1 × 10²⁰ W on a Black starting ship collapses it in about 25 game days when
    /// its storage is full, and never when it has room to convert it.
    #[test]
    fn the_fate_is_31s_attack_table() {
        let field = Field::of(lc_world::fitting::STARTING_ENVELOPE_M2, &B);
        let rating_w = Capacities::of(&Form::starting(), &B).aperture_w;
        let starting = Receiver {
            rating_w: Some(rating_w),
            field: Some(Heated { field, heat_j: field.heat_j_at(B.field_idle_k) }),
            shadow_m2: 1.0e12,
            ..receiver(C_M_S)
        };
        let at = landing(&starting, 1.1e20, 1.0e-9, 1.0e8);
        let Some(Fate::Collapses(s)) = at.if_full else { panic!("{:?}", at.if_full) };
        let days = s / 86_400.0;
        assert!((20.0..30.0).contains(&days), "{days} days");
        assert!(matches!(at.with_room, Some(Fate::Reaches(share)) if share < 1.0), "{:?}", at.with_room);
        let brief = landing(&starting, 1.1e20, 1.0e-9, 86_400.0);
        assert!(matches!(brief.if_full, Some(Fate::Reaches(share)) if share < 1.0), "a day is not enough: {:?}", brief.if_full);
    }

    #[test]
    fn a_spot_under_a_meter_is_not_zero() {
        assert_eq!(fine_m(157.0e3 * 2.0 * 2.86e-9), "898.0 µm");
        assert_eq!(fine_m(0.45), "450.0 mm");
        assert_eq!(fine_m(4.0e-5), "40.0 µm");
        assert_eq!(fine_m(1.5e3), span_m(1.5e3));
    }
}
