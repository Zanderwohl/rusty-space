//! Every craft's field drawn round its hull, and every collapse drawn where its light shows it:
//! R11's envelope in R6's material. 32 §The field; 30 §What an observer sees, §Collapse.
//!
//! The player's field is read from the account (`Fitted`), anyone else's from `Presence.glow`, as
//! its light left it: temperature and shade, with the fill worked back from `T⁴` and no switch,
//! which nothing tells an observer about until it is done. Once a craft's envelope is up it carries
//! the field's heat, and the hull only what it reflects ([`Envelopes`]). Metering is unchanged:
//! [`crate::hull::Sent`] counts the thermal term once whether the envelope or the hull draws it.
//! Nothing draws a distant ship as a point yet; when R18 does, that point takes over from the
//! envelope where the hull stops being meshed, and neither may draw the heat while the other does.
//!
//! The envelope is the ellipsoid the shard measures the field's area on, fitted from the form's
//! parts without the grid and meshed once per design, only when a craft's stated form changes.
//!
//! A collapse is drawn from its `kind::COLLAPSE` sighting as that arrives, at the last place the
//! wreck was seen and in its last shape, never from the shard's time. The spike brightens each
//! neighbor as a hot spot toward the wreck when its light, bounced off that neighbor, reaches
//! this ship: `arrive + (|w−n| + |n−o| − |w−o|) / c` from what the client knows.

use std::collections::{BTreeMap, HashMap, HashSet};

use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use em_render::field_material::{
    BLACK, CLEAR, FieldMaterial, FieldMaterialPlugin, FieldUniform, HOT_SPOTS, RAMP, ramp_entry, ramp_kelvin,
};
use em_render::render_space::sim_to_render;
use em_spectra::{BandMapping, blackbody};
use glam::DVec3;
use lc_proto::{Glow, ShipId};
use lc_world::field::Mode;
use lc_world::fitting::Balance;
use lc_world::form::grid::Envelope;
use lc_world::form::sdf::Sdf;
use lc_world::form::{Form, FormError, Kind};

use crate::hull::Eye;
use crate::session::Session;
use crate::ship_hull::{RealHulls, ShipHull};
use crate::system::{M_PER_LY, UNIT_M};

/// Real seconds the collapse's flash lasts: long enough to see at any clock rate.
const FLASH_S: f32 = 0.5;
/// How many times its own size a wreck's debris spreads to by the end of the afterglow.
const DEBRIS_REACH: f32 = 4.0;
/// Real seconds a spike landing on a neighbor stays on its envelope.
const SPIKE_SHOWN_S: f32 = 1.0;
/// A collapse's power is its spike taken as this long, seconds: 30 §Collapse, As built.
const SPIKE_S: f64 = 1.0;
/// The most a hot spot is drawn as, in multiples of the field's own power; a beam's first moments
/// on a cold field are millions of times it and would overflow the ramp into white.
const MAX_SPOT: f32 = 20.0;
const STEFAN_BOLTZMANN: f64 = 5.670_374_419e-8;
const SHELLS_KEPT: usize = 16;
const WRECKS_KEPT: usize = 32;
const LAST_KEPT: usize = 256;

pub struct FieldPlugin;

impl Plugin for FieldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FieldMaterialPlugin)
            .init_resource::<Shells>()
            .init_resource::<Envelopes>()
            .init_resource::<Wrecks>();
    }
}

/// [`FieldUniform::spectrum`] for `mapping`: a blackbody at each ramp temperature through its bands.
pub fn spectrum(mapping: &BandMapping) -> [Vec4; RAMP] {
    thread_local! {
        static LAST: std::cell::Cell<Option<(BandMapping, [Vec4; RAMP])>> = const { std::cell::Cell::new(None) };
    }
    LAST.with(|last| match last.get() {
        Some((m, spectrum)) if m == *mapping => spectrum,
        _ => {
            let spectrum = std::array::from_fn(|i| ramp_entry(mapping.apply_f64(&blackbody::per_band(ramp_kelvin(i) as f64))));
            last.set(Some((*mapping, spectrum)));
            spectrum
        }
    })
}

/// The field's color at `kelvin` through `mapping`, linear display light, read off the ramp the
/// shader interpolates: the field bar reads this, so it shows the envelope's color and not the
/// exact blackbody, which some mappings put a few percent off it between ramp entries.
pub fn color_linear(mapping: &BandMapping, kelvin: f64) -> [f64; 3] {
    em_render::field_material::ramp_at(&spectrum(mapping), kelvin as f32).as_dvec3().to_array()
}

/// Where a field collapses, K. Intensive: the same on every envelope.
pub fn limit_k(balance: &Balance) -> f64 {
    let field = lc_world::field::Field::of(1.0, balance);
    field.temperature_k(field.heat_max_j())
}

/// `Q / Q_max` at `kelvin`, heat going as `T⁴`.
pub fn fill_at(kelvin: f64, balance: &Balance) -> f64 {
    (kelvin / limit_k(balance)).powi(4)
}

/// What the shader is told about one field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldState {
    pub kelvin: f64,
    /// `Q / Q_max`.
    pub fill: f64,
    /// Before any switch.
    pub shade: Mode,
    /// Into this, and how far through, `[0, 1]`.
    pub switch: Option<(Mode, f64)>,
}

/// `--field-k`, with `--field-mode` and `--field-switch`: the player's field held for photographing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Held {
    pub kelvin: f64,
    pub shade: Option<Mode>,
    /// Into `shade` from the other, frozen this far through.
    pub switch: Option<f64>,
}

impl Held {
    fn apply(self, state: FieldState, balance: &Balance) -> FieldState {
        let shade = self.shade.unwrap_or(state.shade);
        let (shade, switch) = match self.switch {
            Some(p) => (shade.other(), Some((shade, p.clamp(0.0, 1.0)))),
            None => (shade, None),
        };
        FieldState { kelvin: self.kelvin, fill: fill_at(self.kelvin, balance), shade, switch }
    }

    pub(crate) fn glow(self, glow: Glow) -> Glow {
        Glow { temperature_k: self.kelvin, shade: self.shade.map_or(glow.shade, Into::into) }
    }
}

pub fn mode_named(name: &str) -> Option<Mode> {
    match name {
        "clear" => Some(Mode::Clear),
        "black" => Some(Mode::Black),
        _ => None,
    }
}

/// The player's field now, from the account: the same reads as the field bar.
pub fn own_state(session: &Session) -> FieldState {
    let now = session.coordinate_time_s();
    let (state, balance) = match session.ship.fitting() {
        Some(fitting) => {
            let posture = fitting.posture();
            let switch_s = fitting.balance().field_switch_s.max(f64::MIN_POSITIVE);
            let switch = posture.switching_at(now).map(|s| (s.to, (1.0 - (s.done_s - now) / switch_s).clamp(0.0, 1.0)));
            let heat_j = fitting.heat_j_at(&session.ship.motion, now);
            let state = FieldState {
                kelvin: fitting.field().temperature_k(heat_j),
                fill: heat_j / fitting.field().heat_max_j(),
                shade: if switch.is_some() { posture.shade } else { posture.shade_at(now) },
                switch,
            };
            (state, *fitting.balance())
        }
        None => (seen_state(crate::hull::own_glow(session), &Balance::DEFAULT), Balance::DEFAULT),
    };
    match session.held_field {
        Some(held) => held.apply(state, &balance),
        None => state,
    }
}

/// Another craft's field as its light left it.
pub fn seen_state(glow: Glow, balance: &Balance) -> FieldState {
    FieldState { kelvin: glow.temperature_k, fill: fill_at(glow.temperature_k, balance), shade: glow.shade.into(), switch: None }
}

fn code(mode: Mode) -> f32 {
    match mode {
        Mode::Clear => CLEAR,
        Mode::Black => BLACK,
    }
}

/// Where a craft's field is lit from and how it is exposed.
#[derive(Clone, Debug)]
pub struct Lighting {
    /// Render axes.
    pub to_star: Vec3,
    /// A white Lambertian surface facing the star, linear display light.
    pub starlight: Vec3,
    pub exposure: Vec4,
    pub spectrum: [Vec4; RAMP],
    /// Real seconds, for the shimmer and flicker.
    pub clock_s: f32,
}

impl Lighting {
    fn at(session: &Session, star: Option<(DVec3, f64, f64)>, at_ly: DVec3, spectrum: [Vec4; RAMP], clock_s: f32) -> Self {
        let (to_star, starlight) = match star {
            Some((star_ly, radius, teff)) => {
                let lit = crate::resolved::lit_radiance(1.0, radius, teff, star_ly.distance(at_ly) * M_PER_LY);
                (star_ly - at_ly, Vec3::from_array(session.mapping.apply(&lit)))
            }
            None => (DVec3::Z, Vec3::ZERO),
        };
        Lighting {
            to_star: sim_to_render(to_star.normalize_or_zero()).as_vec3(),
            starlight,
            exposure: Vec4::new(session.tone.surface_reference, session.tone.surface_stops, crate::plume::OVERFLOW_GAIN, 0.0),
            spectrum,
            clock_s,
        }
    }
}

/// One field's uniforms. `collapse` is [`FieldUniform::collapse`], `x` negative for a field standing.
pub fn uniform(state: &FieldState, shell: &Shell, balance: &Balance, light: &Lighting, hot_spots: [Vec4; HOT_SPOTS], collapse: Vec4) -> FieldUniform {
    let (mode, previous, progress) = match state.switch {
        Some((to, p)) => (code(to), code(state.shade), p as f32),
        None => (code(state.shade), code(state.shade), 1.0),
    };
    FieldUniform {
        state: Vec4::new(state.kelvin as f32, state.fill as f32, balance.clear_absorptivity as f32, light.clock_s),
        mode: Vec4::new(mode, previous, progress, shell.reach),
        origin: shell.mind.extend(0.0),
        to_star: light.to_star.extend(0.0),
        starlight: light.starlight.extend(0.0),
        exposure: light.exposure,
        hot_spots,
        collapse,
        collapse_k: Vec4::new(balance.collapse_spike_k as f32, limit_k(balance) as f32, shell.radius, 0.0),
        spectrum: light.spectrum,
    }
}

const STANDING: Vec4 = Vec4::new(-1.0, FLASH_S, 1.0, DEBRIS_REACH);

/// The strongest [`HOT_SPOTS`] of `spots`.
fn strongest(spots: impl IntoIterator<Item = Vec4>) -> [Vec4; HOT_SPOTS] {
    let mut all: Vec<Vec4> = spots.into_iter().filter(|s| s.w > 0.0).collect();
    all.sort_by(|a, b| b.w.total_cmp(&a.w));
    std::array::from_fn(|i| all.get(i).copied().unwrap_or(Vec4::ZERO))
}

/// Power landing on a field over what it radiates, `σT⁴A`, drawn no stronger than [`MAX_SPOT`].
fn spot_strength(absorbed_w: f64, kelvin: f64, area_m2: f64) -> f32 {
    let radiated_w = STEFAN_BOLTZMANN * kelvin.powi(4) * area_m2;
    if radiated_w <= 0.0 {
        return if absorbed_w > 0.0 { MAX_SPOT } else { 0.0 };
    }
    ((absorbed_w / radiated_w) as f32).clamp(0.0, MAX_SPOT)
}

/// Beams on the player's ship, from `Outbound::Illuminated`, by beam id. Each statement replaces
/// the last for its beam; zero power is the beam gone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Incoming {
    ship: Option<ShipId>,
    beams: BTreeMap<i64, Beam>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beam {
    /// Toward the source, simulation axes.
    pub bearing: DVec3,
    /// Before the field's absorptivity.
    pub power_w: f64,
    /// Coordinate seconds this power arrived.
    pub since_s: f64,
}

impl Incoming {
    /// A statement about `ship`, which starts the list over when it is a new ship.
    pub fn illuminated(&mut self, ship: ShipId, beam: i64, bearing: [f64; 3], power_w: f64, arrive_t: i64) {
        if self.ship != Some(ship) {
            self.beams.clear();
            self.ship = Some(ship);
        }
        let since_s = arrive_t as f64 * 1e-6;
        if self.beams.get(&beam).is_some_and(|b| b.since_s > since_s) {
            return;
        }
        if power_w > 0.0 {
            self.beams.insert(beam, Beam { bearing: DVec3::from_array(bearing), power_w, since_s });
        } else {
            self.beams.remove(&beam);
        }
    }

    pub fn beams(&self) -> impl Iterator<Item = &Beam> {
        self.beams.values()
    }

    fn spots(&self, absorbs: f64, kelvin: f64, area_m2: f64) -> impl Iterator<Item = Vec4> + '_ {
        self.beams().map(move |b| {
            let toward = sim_to_render(b.bearing.normalize_or_zero()).as_vec3();
            toward.extend(spot_strength(b.power_w * absorbs, kelvin, area_m2))
        })
    }
}

/// One design's envelope. Ship frame, meters.
#[derive(Clone, Debug)]
pub struct Shell {
    pub mesh: Handle<Mesh>,
    /// Where the mesh's origin sits.
    pub center: Vec3,
    /// The farthest vertex from the center.
    pub radius: f32,
    /// The Mind, from the center: where a switch sweeps out from.
    pub mind: Vec3,
    /// The farthest vertex from the Mind.
    pub reach: f32,
    pub area_m2: f64,
}

pub struct Meshed {
    pub mesh: Mesh,
    pub center: Vec3,
    pub radius: f32,
    pub mind: Vec3,
    pub reach: f32,
    pub area_m2: f64,
}

/// Longitude and latitude divisions of every envelope: it is an ellipsoid, so one mesh's worth of
/// detail serves every size.
const LONGITUDES: u32 = 64;
const LATITUDES: u32 = 32;

/// The envelope the shard measures, [`Envelope`], as a mesh with its origin at its center.
pub fn mesh_envelope(form: &Form, balance: &Balance) -> Result<Meshed, FormError> {
    let sdf = Sdf::new(form, balance)?;
    let envelope = Envelope::of(&sdf, balance);
    let mesh = Sphere::new(1.0).mesh().uv(LONGITUDES, LATITUDES).scaled_by(envelope.semi_axes.as_vec3());
    let mind = sdf.pieces().iter().find(|p| p.kind == Kind::Mind).map_or(envelope.center, |p| p.pose.position);
    let mind = (mind - envelope.center).as_vec3();
    let reach = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.iter().map(|v| Vec3::from_array(*v).distance(mind)).fold(0.0, f32::max),
        _ => 0.0,
    };
    Ok(Meshed {
        mesh,
        center: envelope.center.as_vec3(),
        radius: envelope.semi_axes.max_element() as f32,
        mind,
        reach,
        area_m2: envelope.area_m2(),
    })
}

/// Envelopes by [`crate::hull_mesh::form_hash`], meshed on the async pool.
#[derive(Resource, Default)]
pub struct Shells {
    ready: HashMap<u64, Option<Shell>>,
    pending: HashMap<u64, Task<Option<Meshed>>>,
}

impl Shells {
    /// Meshes `hash` unless it is meshed or under way; `source` is asked only then. True when it
    /// started.
    pub fn request(&mut self, hash: u64, source: impl FnOnce() -> (Form, Balance)) -> bool {
        if self.ready.contains_key(&hash) || self.pending.contains_key(&hash) {
            return false;
        }
        let (form, balance) = source();
        let task = AsyncComputeTaskPool::get().spawn(async move {
            mesh_envelope(&form, &balance).map_err(|error| warn!("field: {error}")).ok()
        });
        self.pending.insert(hash, task);
        true
    }

    pub fn get(&self, hash: u64) -> Option<&Shell> {
        self.ready.get(&hash)?.as_ref()
    }

    fn land(&mut self, meshes: &mut Assets<Mesh>, wanted: &HashSet<u64>) {
        let ready = &mut self.ready;
        self.pending.retain(|&hash, task| match check_ready(task) {
            Some(meshed) => {
                let shell = meshed.map(|m| Shell {
                    mesh: meshes.add(m.mesh),
                    center: m.center,
                    radius: m.radius,
                    mind: m.mind,
                    reach: m.reach,
                    area_m2: m.area_m2,
                });
                ready.insert(hash, shell);
                false
            }
            None => true,
        });
        if ready.len() > SHELLS_KEPT {
            ready.retain(|hash, _| wanted.contains(hash));
        }
    }
}

/// Which craft have their field drawn round them this frame, so their hulls leave its heat to it.
#[derive(Resource, Default)]
pub struct Envelopes {
    drawn: HashSet<Option<ShipId>>,
}

impl Envelopes {
    pub fn drawn(&self, craft: Option<ShipId>) -> bool {
        self.drawn.contains(&craft)
    }
}

/// A craft's envelope, under its [`ShipHull`] root, or a wreck's under its own.
#[derive(Component)]
pub struct FieldNode {
    craft: Option<ShipId>,
    hash: u64,
    layers: [Entity; 3],
}

/// A wreck's root, placed here rather than by [`crate::ship_hull`], whose craft is gone.
#[derive(Component)]
pub struct WreckRoot(i64);

/// A craft as last seen, for drawing its end after it has left the contact list.
#[derive(Clone, Debug)]
struct Last {
    name: String,
    at_ly: DVec3,
    /// Render axes, as its hull was last drawn.
    rotation: Quat,
    hash: u64,
    kelvin: f64,
}

/// A collapse whose sighting has arrived.
#[derive(Clone, Debug)]
pub struct Wreck {
    event_id: i64,
    craft: ShipId,
    /// Coordinate seconds its light reaches this ship.
    arrive_s: f64,
    /// How far the shard's clock was ahead of this one when the sighting was taken. It is sent
    /// once its light has arrived by the shard's clock, and the hull goes from the contacts then,
    /// so its end is drawn on this one's timeline shifted to start there.
    ahead_s: f64,
    at_ly: DVec3,
    rotation: Quat,
    hash: u64,
    kelvin: f64,
    /// The envelope's longest semi-axis, meters, once its shell is known.
    radius_m: f64,
    spike_w: f64,
    /// Real seconds it was first drawn.
    shown_at: Option<f32>,
    /// Real seconds its spike was first drawn on each neighbor.
    landed: HashMap<Option<ShipId>, f32>,
}

impl Wreck {
    /// [`FieldUniform::collapse`] now, once its light has arrived: the flash in real seconds from
    /// the frame it is first drawn, the cooling in coordinate seconds from its arrival.
    fn collapse(&mut self, now_s: f64, real_s: f32, afterglow_s: f64) -> Option<Vec4> {
        if now_s + self.ahead_s < self.arrive_s {
            return None;
        }
        let since_real = real_s - *self.shown_at.get_or_insert(real_s);
        let cooled = self.cooled(now_s, real_s, afterglow_s);
        // The shader runs both off one clock, so the afterglow is stated as the real seconds that
        // put its cooling where [`Wreck::cooled`] has it.
        let afterglow_real = if cooled > 0.0 { since_real / cooled as f32 } else { f32::MAX };
        Some(Vec4::new(since_real, FLASH_S, afterglow_real.max(since_real), DEBRIS_REACH))
    }

    /// How far through the afterglow, by the coordinate clock or as it plays at the design rate,
    /// whichever is further: a clock slowed for watching would otherwise hold the debris still.
    fn cooled(&self, now_s: f64, real_s: f32, afterglow_s: f64) -> f64 {
        let afterglow_s = afterglow_s.max(f64::MIN_POSITIVE);
        let by_clock = (now_s + self.ahead_s - self.arrive_s) / afterglow_s;
        let by_design = self.shown_at.map_or(0.0, |at| f64::from(real_s - at) * crate::session::TIME_RATE / afterglow_s);
        by_clock.max(by_design)
    }

    fn over(&self, now_s: f64, real_s: f32, afterglow_s: f64) -> bool {
        self.cooled(now_s, real_s, afterglow_s) > 1.0 && self.shown_at.is_some_and(|at| real_s - at > FLASH_S)
    }

    /// Coordinate seconds the spike's light, off a craft at `at_ly`, reaches an observer at
    /// `observer_ly`.
    fn lands_s(&self, at_ly: DVec3, observer_ly: DVec3) -> f64 {
        let path = self.at_ly.distance(at_ly) + at_ly.distance(observer_ly) - self.at_ly.distance(observer_ly);
        self.arrive_s + path * lc_world::flight::JULIAN_YEAR_S
    }

    /// The spike on a craft at `at_ly`, as a hot spot toward the wreck, once its light is here.
    fn spike_on(&mut self, craft: Option<ShipId>, at_ly: DVec3, observer_ly: DVec3, now_s: f64, real_s: f32, field: (f64, f64, f64)) -> Option<Vec4> {
        if craft == Some(self.craft) || now_s + self.ahead_s < self.lands_s(at_ly, observer_ly) {
            return None;
        }
        let since = real_s - *self.landed.entry(craft).or_insert(real_s);
        let fading = (1.0 - since / SPIKE_SHOWN_S).max(0.0).powi(2);
        let (absorbs, kelvin, area_m2) = field;
        let d_m = self.at_ly.distance(at_ly) * M_PER_LY;
        if fading <= 0.0 || d_m <= 0.0 {
            return None;
        }
        let absorbed_w = self.spike_w * absorbs * area_m2 / (16.0 * std::f64::consts::PI * d_m * d_m);
        let toward = sim_to_render((self.at_ly - at_ly).normalize_or_zero()).as_vec3();
        Some(toward.extend(spot_strength(absorbed_w, kelvin, area_m2) * fading))
    }
}

/// Collapses whose light has arrived, and what the craft they end were last seen as.
#[derive(Resource, Default)]
pub struct Wrecks {
    fell: Vec<Wreck>,
    last: HashMap<ShipId, Last>,
    taken: HashSet<i64>,
}

impl Wrecks {
    /// Each wreck's debris for the exposure, as `(where, kelvin, radius_m)`: a disc of what it
    /// covers at the temperature it has cooled to, as the shader draws it. Not the flash, which is
    /// left to overflow. Without these the meter opens up once the last hull has gone, and the
    /// debris burns white.
    pub fn metered(&self, now_s: f64, real_s: f32, afterglow_s: f64, limit_k: f64) -> Vec<(DVec3, f64, f64)> {
        self.fell
            .iter()
            .filter(|w| w.shown_at.is_some() && w.radius_m > 0.0)
            .map(|w| {
                let cooled = w.cooled(now_s, real_s, afterglow_s).clamp(0.0, 1.0);
                let grow = 1.0 + f64::from(DEBRIS_REACH - 1.0) * (1.0 - (1.0 - cooled).powi(3));
                let cover = (1.0 - cooled).sqrt();
                let kelvin = limit_k * (1.0 - cooled).powf(0.6) + 300.0;
                (w.at_ly, kelvin, w.radius_m * grow * cover.sqrt())
            })
            .collect()
    }
}

/// Take each `kind::COLLAPSE` sighting as it lands: a line naming the ship as it was seen, and its
/// wreck to draw.
pub fn collapses(
    game: Res<crate::app::Game>,
    uplink: Res<crate::uplink::Uplink>,
    mut ui: ResMut<crate::app::Ui>,
    mut wrecks: ResMut<Wrecks>,
) {
    let me = uplink.joined().map(|joined| joined.ship_id);
    let balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    let wrecks = &mut *wrecks;
    for sighting in uplink.seen.iter().filter(|s| s.kind == lc_proto::kind::COLLAPSE) {
        let from = ShipId(sighting.source_id);
        if !wrecks.taken.insert(sighting.event_id) || Some(from) == me {
            continue;
        }
        let last = wrecks.last.get(&from);
        let name = last.map_or_else(|| uplink.name_of(from), |l| l.name.clone());
        let arrive_s = sighting.arrive_t as f64 * 1e-6;
        ui.0.heard(from, format!("{name} collapsed"), arrive_s);
        let Some(last) = last else { continue };
        let released_j = serde_json::from_str::<lc_proto::Released>(&sighting.payload).map_or(0.0, |r| r.released_j);
        wrecks.fell.push(Wreck {
            event_id: sighting.event_id,
            craft: from,
            arrive_s,
            ahead_s: (arrive_s - game.0.coordinate_time_s()).max(0.0),
            at_ly: last.at_ly,
            rotation: last.rotation,
            hash: last.hash,
            kelvin: last.kelvin,
            radius_m: 0.0,
            spike_w: balance.collapse_spike_fraction * released_j / SPIKE_S,
            shown_at: None,
            landed: HashMap::new(),
        });
    }
    let seen: HashSet<i64> = uplink.seen.iter().map(|s| s.event_id).collect();
    wrecks.taken.retain(|id| seen.contains(id));
    let excess = wrecks.fell.len().saturating_sub(WRECKS_KEPT);
    wrecks.fell.drain(..excess);
}

/// A craft drawn this frame, and its field.
struct Drawn {
    craft: Option<ShipId>,
    root: Entity,
    hash: u64,
    at_ly: DVec3,
    state: FieldState,
}

/// Every craft's envelope under its hull's root, and every wreck under its own.
#[allow(clippy::too_many_arguments)]
pub fn draw_fields(
    mut commands: Commands,
    (game, uplink, eye, ui, time): (Res<crate::app::Game>, Res<crate::uplink::Uplink>, Res<Eye>, Res<crate::app::Ui>, Res<Time<Real>>),
    (own, real): (Res<crate::parts::OwnForm>, Res<RealHulls>),
    (mut shells, mut envelopes, mut wrecks): (ResMut<Shells>, ResMut<Envelopes>, ResMut<Wrecks>),
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<FieldMaterial>>),
    hulls: Query<(Entity, &ShipHull, &Transform)>,
    mut nodes: Query<(Entity, &mut FieldNode, &mut Transform, &ChildOf), (Without<ShipHull>, Without<WreckRoot>)>,
    mut roots: Query<(Entity, &WreckRoot, &mut Transform), (Without<ShipHull>, Without<FieldNode>)>,
    mut layers: Query<(&mut Mesh3d, &MeshMaterial3d<FieldMaterial>)>,
) {
    let session = &game.0;
    let now = session.coordinate_time_s();
    let real_s = time.elapsed_secs();
    let spectrum = spectrum(&session.mapping);
    let star = crate::hull::lighting(session);
    let contact_balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    let own_balance = session.ship.fitting().map_or(Balance::DEFAULT, |f| *f.balance());
    let observer_ly = session.ship.motion.position_ly;
    let afterglow_s = contact_balance.collapse_afterglow_s;

    let mut drawn = Vec::new();
    for (root, hull, transform) in &hulls {
        let craft = hull.craft();
        let Some(hash) = real.stated(craft) else { continue };
        let contact = craft.and_then(|id| uplink.contacts.iter().find(|c| c.ship_id == id));
        shells.request(hash, || match contact {
            Some(c) => (Form::from(&c.form), contact_balance),
            None => (own.form().cloned().unwrap_or_default(), own.balance()),
        });
        let (at_ly, state) = match contact {
            Some(c) => (c.position_ly, seen_state(c.glow, &contact_balance)),
            None if craft.is_none() => (observer_ly, own_state(session)),
            None => continue,
        };
        if let Some(c) = contact {
            let last = Last { name: c.name.clone(), at_ly, rotation: transform.rotation, hash, kelvin: state.kelvin };
            wrecks.last.insert(c.ship_id, last);
        }
        drawn.push(Drawn { craft, root, hash, at_ly, state });
    }
    if wrecks.last.len() > LAST_KEPT {
        wrecks.last.retain(|id, _| uplink.contacts.iter().any(|c| c.ship_id == *id));
    }
    let wanted: HashSet<u64> = drawn.iter().map(|d| d.hash).chain(wrecks.fell.iter().map(|w| w.hash)).collect();
    shells.land(&mut meshes, &wanted);

    envelopes.drawn.clear();
    for d in &drawn {
        let balance = if d.craft.is_none() { own_balance } else { contact_balance };
        let node = nodes.iter_mut().find(|(_, n, _, parent)| n.craft == d.craft && parent.parent() == d.root);
        // A new form's envelope replaces the old only once it has landed.
        let shown = shells.get(d.hash).map(|s| (d.hash, s)).or_else(|| {
            let (_, n, ..) = node.as_ref()?;
            shells.get(n.hash).map(|s| (n.hash, s))
        });
        let Some((hash, shell)) = shown else { continue };
        envelopes.drawn.insert(d.craft);
        let absorbs = d.state.shade.absorptivity(balance.clear_absorptivity);
        let field = (absorbs, d.state.kelvin, shell.area_m2);
        let mut spots: Vec<Vec4> = wrecks
            .fell
            .iter_mut()
            .filter_map(|w| w.spike_on(d.craft, d.at_ly, observer_ly, now, real_s, field))
            .collect();
        if d.craft.is_none() {
            spots.extend(uplink.incoming.spots(absorbs, d.state.kelvin, shell.area_m2));
        }
        let light = Lighting::at(session, star, d.at_ly, spectrum, real_s);
        let next = uniform(&d.state, shell, &balance, &light, strongest(spots), STANDING);
        match node {
            Some((_, mut node, mut transform, _)) => {
                transform.translation = shell.center;
                if node.hash != hash {
                    node.hash = hash;
                    for layer in node.layers {
                        if let Ok((mut mesh, _)) = layers.get_mut(layer) {
                            mesh.0 = shell.mesh.clone();
                        }
                    }
                }
                restate(&node.layers, &layers, &mut materials, &next);
            }
            None => {
                let node = spawn_node(&mut commands, &mut materials, d.craft, hash, shell, next);
                commands.entity(node).insert(ChildOf(d.root));
            }
        }
    }

    let look = ui.look.forward();
    let mut kept = HashSet::new();
    let mut fell = std::mem::take(&mut wrecks.fell);
    fell.retain(|w| !w.over(now, real_s, afterglow_s));
    for wreck in &mut fell {
        let Some(shell) = shells.get(wreck.hash) else { continue };
        let Some(collapse) = wreck.collapse(now, real_s, afterglow_s) else { continue };
        wreck.radius_m = f64::from(shell.radius);
        kept.insert(wreck.event_id);
        let placed = Transform {
            translation: sim_to_render(eye.offset_m(wreck.at_ly, Some(wreck.craft), look) / UNIT_M).as_vec3(),
            rotation: wreck.rotation,
            scale: Vec3::splat((1.0 / UNIT_M) as f32),
        };
        let state = FieldState { kelvin: wreck.kelvin, fill: 1.0, shade: Mode::Black, switch: None };
        let light = Lighting::at(session, star, wreck.at_ly, spectrum, real_s);
        let next = uniform(&state, shell, &contact_balance, &light, [Vec4::ZERO; HOT_SPOTS], collapse);
        match roots.iter_mut().find(|(_, r, _)| r.0 == wreck.event_id) {
            Some((root, _, mut transform)) => {
                *transform = placed;
                let node = nodes.iter().find(|(_, _, _, parent)| parent.parent() == root);
                if let Some((_, node, ..)) = node {
                    restate(&node.layers, &layers, &mut materials, &next);
                }
            }
            None => {
                let root = commands.spawn((placed, Visibility::default(), WreckRoot(wreck.event_id))).id();
                let node = spawn_node(&mut commands, &mut materials, Some(wreck.craft), wreck.hash, shell, next);
                commands.entity(node).insert(ChildOf(root));
            }
        }
    }
    wrecks.fell = fell;
    for (root, wreck, _) in &roots {
        if !kept.contains(&wreck.0) {
            commands.entity(root).despawn();
        }
    }
}

fn restate(
    layers: &[Entity; 3],
    query: &Query<(&mut Mesh3d, &MeshMaterial3d<FieldMaterial>)>,
    materials: &mut Assets<FieldMaterial>,
    next: &FieldUniform,
) {
    for layer in layers {
        let Ok((_, material)) = query.get(*layer) else { continue };
        if let Some(mut asset) = materials.get_mut(&material.0)
            && asset.uniforms != *next
        {
            asset.uniforms = next.clone();
        }
    }
}

fn spawn_node(
    commands: &mut Commands,
    materials: &mut Assets<FieldMaterial>,
    craft: Option<ShipId>,
    hash: u64,
    shell: &Shell,
    uniforms: FieldUniform,
) -> Entity {
    // Sorted in render units, which is what the root scales meters into.
    let size = shell.radius * (1.0 / UNIT_M) as f32;
    let layers = FieldMaterial::layers(uniforms, size).map(|layer| {
        commands
            .spawn((
                Mesh3d(shell.mesh.clone()),
                MeshMaterial3d(materials.add(layer)),
                Transform::IDENTITY,
                NoFrustumCulling,
                RenderLayers::layer(crate::app::SKY_ONLY_LAYER),
            ))
            .id()
    });
    let node = commands
        .spawn((Transform::from_translation(shell.center), Visibility::default(), FieldNode { craft, hash, layers }))
        .id();
    for layer in layers {
        commands.entity(layer).insert(ChildOf(node));
    }
    node
}

/// `--field-k` and `--field-beam`, written every frame: a pin, as `crate::dev` says.
pub fn hold(
    dev: Option<Res<crate::dev::DevEntry>>,
    ui: Res<crate::app::Ui>,
    mut game: ResMut<crate::app::Game>,
    mut uplink: ResMut<crate::uplink::Uplink>,
) {
    let Some(dev) = dev else { return };
    if dev.field.is_some() && game.0.held_field != dev.field {
        game.0.held_field = dev.field;
    }
    if let Some(watts) = dev.field_beam_w {
        let ship = uplink.joined().map_or(ShipId(0), |joined| joined.ship_id);
        // From the camera's side of the ship and above it, so the spot faces the lens.
        let look = ui.look.forward();
        let bearing = (look.cross(DVec3::Z).normalize_or_zero() * 0.6 - look).normalize_or_zero();
        uplink.incoming.illuminated(ship, -1, bearing.to_array(), watts, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::tasks::TaskPool;
    use em_render::field_material::{ramp_at, wall_opacity};
    use lc_world::fitting::{Account, Fitting, Posture, Switch};
    use lc_world::sky::AuthoredStars;

    const B: Balance = Balance::DEFAULT;

    fn shell() -> Shell {
        Shell { mesh: Handle::default(), center: Vec3::ZERO, radius: 30.0, mind: Vec3::Z * 5.0, reach: 35.0, area_m2: 1.0e4 }
    }

    fn light(mapping: &BandMapping) -> Lighting {
        Lighting { to_star: Vec3::X, starlight: Vec3::ONE, exposure: Vec4::new(1.0, 5.0, 0.5, 0.0), spectrum: spectrum(mapping), clock_s: 0.0 }
    }

    /// The starting ship at `of_max` of `Q_max`, in `posture`.
    fn heated(of_max: f64, posture: Posture) -> Session {
        let mut s = Session::new(&AuthoredStars::sample(), 3);
        let full = Fitting::full(Form::starting(), B, s.coordinate_time_s());
        let account = Account { heat_j: of_max * full.field().heat_max_j(), starlight_w: 0.0, posture, ..full.account() };
        s.ship.fit(Some(Fitting::from_account(&account, B)));
        s
    }

    fn bar(s: &Session) -> crate::hud::Field {
        crate::hud::lines(s, &Default::default(), &mut Default::default()).field.expect("a fitted ship has a bar")
    }

    /// Linear display light as the bar shows a color: brightest channel at one, in sRGB.
    fn as_bar(linear: Vec3) -> [f32; 3] {
        let peak = linear.max_element();
        let srgb = Color::linear_rgb(linear.x / peak, linear.y / peak, linear.z / peak).to_srgba();
        [srgb.red, srgb.green, srgb.blue]
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|c| (a[c] - b[c]).abs() < 1e-3)
    }

    /// The shader's color for a field, as its uniforms hand it over, is the bar's, in every
    /// mapping the player can choose.
    #[test]
    fn the_envelope_is_the_color_the_bar_shows() {
        for (name, mapping) in em_spectra::presets::all() {
            for kelvin in [1_000.0, 1_800.0, 2_400.0, 3_300.0, 4_600.0] {
                let envelope = ramp_at(&spectrum(&mapping), kelvin as f32);
                if envelope.max_element() <= 0.0 {
                    continue;
                }
                let bar = crate::hud::glow(&mapping, kelvin);
                assert!(close(as_bar(envelope), bar), "{name} at {kelvin} K: {:?} against {bar:?}", as_bar(envelope));
            }
        }
    }

    /// A diving ship's field is drawn at the temperature and fill the bar reads, and so in its color.
    #[test]
    fn a_diving_ship_glows_as_its_bar_says() {
        for of_max in [0.05, 0.3, 0.9] {
            let s = heated(of_max, Posture::BLACK);
            let (state, bar) = (own_state(&s), bar(&s));
            assert!((state.kelvin - bar.kelvin).abs() < 1e-9, "{} against {}", state.kelvin, bar.kelvin);
            assert!((state.fill as f32 - bar.fraction).abs() < 1e-6);
            let drawn = uniform(&state, &shell(), &B, &light(&s.mapping), [Vec4::ZERO; HOT_SPOTS], STANDING);
            let color = ramp_at(&drawn.spectrum, drawn.state.x);
            assert!(close(as_bar(color), bar.blackbody), "{:?} against {:?} at {} K", as_bar(color), bar.blackbody, bar.kelvin);
        }
    }

    /// `--field-k` holds what the bar reads as well as what is drawn.
    #[test]
    fn a_held_field_is_held_on_the_bar_too() {
        let mut s = heated(0.05, Posture::BLACK);
        s.held_field = Some(Held { kelvin: 4_000.0, shade: Some(Mode::Clear), switch: None });
        let (state, bar) = (own_state(&s), bar(&s));
        assert_eq!((state.kelvin, bar.kelvin), (4_000.0, 4_000.0));
        assert_eq!(state.shade, Mode::Clear);
        assert!((state.fill as f32 - bar.fraction).abs() < 1e-6);
        assert_eq!(crate::hull::own_glow(&s).temperature_k, 4_000.0, "and what the exposure meters");
    }

    /// Face-on, a Clear wall shows most of what is behind it, and a Black one none of it, from the
    /// mode and absorptivity the host hands over.
    #[test]
    fn clear_shows_the_ship_and_black_hides_it() {
        let opacity = |shade| {
            let state = FieldState { kelvin: 400.0, fill: 0.0, shade, switch: None };
            let u = uniform(&state, &shell(), &B, &light(&em_spectra::presets::natural()), [Vec4::ZERO; HOT_SPOTS], STANDING);
            let absorbs = u.state.z + (1.0 - u.state.z) * u.mode.x;
            wall_opacity(absorbs, u.mode.x, 1.0)
        };
        assert!(opacity(Mode::Clear) < 0.2, "{}", opacity(Mode::Clear));
        assert_eq!(opacity(Mode::Black), 1.0);
    }

    /// A switch under way is drawn from the shade it leaves, into the one it is going to.
    #[test]
    fn a_switch_is_drawn_part_swept() {
        let now = Session::new(&AuthoredStars::sample(), 3).coordinate_time_s();
        let switch = Switch { to: Mode::Black, done_s: now + 0.25 * B.field_switch_s };
        let s = heated(0.1, Posture { switch: Some(switch), ..Posture::new_ship(&B) });
        let state = own_state(&s);
        let (to, progress) = state.switch.expect("under way");
        assert_eq!((state.shade, to), (Mode::Clear, Mode::Black));
        assert!((progress - 0.75).abs() < 1e-6, "{progress}");
        let u = uniform(&state, &shell(), &B, &light(&s.mapping), [Vec4::ZERO; HOT_SPOTS], STANDING);
        assert_eq!((u.mode.x, u.mode.y), (BLACK, CLEAR));
    }

    /// A beam restated stays one hot spot at its newest power, an older statement changes nothing,
    /// and zero power takes it off.
    #[test]
    fn a_beam_restated_is_one_spot_and_zero_power_removes_it() {
        let mut incoming = Incoming::default();
        let (ship, bearing) = (ShipId(7), [0.0, 0.6, 0.8]);
        incoming.illuminated(ship, 9, bearing, 1.0e17, 1_000_000);
        incoming.illuminated(ship, 9, bearing, 2.0e17, 2_000_000);
        incoming.illuminated(ship, 9, bearing, 5.0e17, 1_500_000);
        let powers: Vec<f64> = incoming.beams().map(|b| b.power_w).collect();
        assert_eq!(powers, [2.0e17]);
        let spots = strongest(incoming.spots(1.0, 2_000.0, 1.0e4));
        assert!(spots[0].w > 0.0 && spots[1].w == 0.0);

        incoming.illuminated(ship, 9, bearing, 0.0, 3_000_000);
        assert_eq!(incoming.beams().count(), 0);
        assert!(strongest(incoming.spots(1.0, 2_000.0, 1.0e4)).iter().all(|s| s.w == 0.0));

        incoming.illuminated(ship, 4, bearing, 1.0e17, 4_000_000);
        incoming.illuminated(ShipId(8), 5, bearing, 1.0e17, 5_000_000);
        assert_eq!(incoming.beams().count(), 1, "the successor starts with no beams");
    }

    /// A design is meshed once, and again only for a new one.
    #[test]
    fn the_envelope_remeshes_only_for_a_new_form() {
        AsyncComputeTaskPool::get_or_init(TaskPool::default);
        let mut shells = Shells::default();
        let mut asked = 0;
        let mut source = |form: Form| {
            asked += 1;
            (form, B)
        };
        assert!(shells.request(1, || source(Form::starting())));
        assert!(!shells.request(1, || source(Form::starting())), "meshed again for the same form");
        assert!(shells.request(2, || source(lc_world::form::presets::Builtin::Plate.form())));
        assert!(!shells.request(1, || source(Form::starting())));
        assert_eq!(asked, 2, "a form asked for when nothing new was wanted");
    }

    /// The envelope is closed round the hull with its margin, and its area is the shard's.
    #[test]
    fn the_envelope_is_the_shards() {
        let form = Form::starting();
        let meshed = mesh_envelope(&form, &B).unwrap();
        let grid = lc_world::form::grid::FormGrid::new(&form, &B).unwrap();
        assert_eq!(meshed.area_m2, grid.envelope_area_m2());
        assert_eq!(meshed.center, grid.envelope().center.as_vec3());
        let positions = match meshed.mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p.clone(),
            _ => panic!("no positions"),
        };
        let Some(bevy::mesh::Indices::U32(indices)) = meshed.mesh.indices() else { panic!("no indices") };
        let area: f32 = indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|i| Vec3::from_array(positions[t[i] as usize]));
                0.5 * (b - a).cross(c - a).length()
            })
            .sum();
        assert!((area as f64 / meshed.area_m2 - 1.0).abs() < 0.01, "{area} m² against {}", meshed.area_m2);
        assert!(meshed.reach >= meshed.radius && meshed.mind.length() < meshed.radius);
    }

    /// The hull leaves the field's heat to the envelope once that is drawn round it.
    #[test]
    fn an_enveloped_hull_does_not_glow_as_well() {
        let s = heated(0.9, Posture::BLACK);
        let hot = crate::hull::own_glow(&s);
        assert!(hot.temperature_k > 4_000.0, "premise: {}", hot.temperature_k);
        let bare = crate::ship_hull::finished(&s, None, DVec3::ZERO, hot, false);
        let wrapped = crate::ship_hull::finished(&s, None, DVec3::ZERO, hot, true);
        assert!(bare.glow.truncate().max_element() > 0.0);
        assert_eq!(wrapped.glow, Vec4::ZERO);
        assert_eq!(bare.reflected, wrapped.reflected, "only the heat moves");
    }

    fn sighting(event_id: i64, source_id: i64, emitted_t: i64, arrive_t: i64) -> lc_proto::Sighting {
        let released = lc_proto::Released { released_j: 3.6e24 };
        lc_proto::Sighting {
            event_id,
            source_id,
            arrive_t,
            emitted_t,
            direction: [1.0, 0.0, 0.0],
            strength: 1.0,
            kind: lc_proto::kind::COLLAPSE,
            payload: serde_json::to_string(&released).unwrap(),
        }
    }

    fn last(name: &str) -> Last {
        Last { name: name.into(), at_ly: DVec3::X * 1.0e-9, rotation: Quat::IDENTITY, hash: 3, kelvin: 4_500.0 }
    }

    /// Runs [`collapses`] with this client's clock at `now_s`.
    fn collapsing(seen: Vec<lc_proto::Sighting>, wrecks: Wrecks, now_s: f64) -> World {
        let mut world = World::new();
        let mut session = Session::new(&AuthoredStars::sample(), 3);
        session.set_coordinate_time_us((now_s * 1.0e6) as i64);
        world.insert_resource(crate::app::Game(session));
        let mut uplink = crate::uplink::Uplink::default();
        uplink.seen = seen;
        world.insert_resource(uplink);
        world.insert_resource(crate::app::Ui(Default::default()));
        world.insert_resource(wrecks);
        world.run_system_once(collapses).unwrap();
        world
    }

    use bevy::ecs::system::RunSystemOnce;

    /// The console names a ship by what its presence called it, gone from sight or not, and each
    /// sighting is taken once.
    #[test]
    fn a_collapse_is_named_as_the_ship_was_seen() {
        let mut wrecks = Wrecks::default();
        wrecks.last.insert(ShipId(5), last("Aster"));
        let mut world = collapsing(vec![sighting(1, 5, 10_000_000, 20_000_000)], wrecks, 20.0);
        let said: Vec<String> = world.resource::<crate::app::Ui>().0.notifications.iter().map(|n| n.text.clone()).collect();
        assert_eq!(said, ["Aster collapsed"]);
        world.run_system_once(collapses).unwrap();
        assert_eq!(world.resource::<crate::app::Ui>().0.notifications.len(), 1, "said twice");
        assert_eq!(world.resource::<Wrecks>().fell.len(), 1);
    }

    /// The flash starts as the sighting's light arrives, not at the collapse's own instant, and
    /// runs in real seconds from the frame it is first drawn.
    #[test]
    fn the_flash_starts_when_its_light_arrives() {
        let mut wrecks = Wrecks::default();
        wrecks.last.insert(ShipId(5), last("Aster"));
        let mut world = collapsing(vec![sighting(1, 5, 10_000_000, 20_000_000)], wrecks, 20.0);
        let mut wreck = world.resource_mut::<Wrecks>().fell.remove(0);
        let afterglow_s = B.collapse_afterglow_s;
        assert_eq!(wreck.collapse(10.0, 1.0, afterglow_s), None, "drawn at the shard's time");
        assert_eq!(wreck.collapse(19.9, 2.0, afterglow_s), None, "drawn before its light is here");
        let first = wreck.collapse(20.0, 3.0, afterglow_s).expect("its light is here");
        assert_eq!(first.x, 0.0);
        let later = wreck.collapse(20.0 + 0.1 * afterglow_s, 3.25, afterglow_s).unwrap();
        assert!((later.x - 0.25).abs() < 1e-6);
        assert!((later.x / later.z - 0.1).abs() < 1e-3, "cooling at {} of the afterglow", later.x / later.z);
    }

    /// The exposure meters a wreck's debris once it is drawn, hot at first and cooler later, so
    /// the meter does not open up on it when the last hull goes.
    #[test]
    fn the_debris_is_metered() {
        let mut wrecks = Wrecks::default();
        wrecks.last.insert(ShipId(5), last("Aster"));
        let mut world = collapsing(vec![sighting(1, 5, 10_000_000, 20_000_000)], wrecks, 20.0);
        let mut wrecks = world.resource_mut::<Wrecks>();
        let afterglow_s = B.collapse_afterglow_s;
        assert!(wrecks.metered(20.0, 1.0, afterglow_s, 4_600.0).is_empty(), "metered before it is drawn");
        wrecks.fell[0].collapse(20.0, 1.0, afterglow_s).unwrap();
        wrecks.fell[0].radius_m = 300.0;
        let [(_, hot, _)] = wrecks.metered(20.0, 1.0, afterglow_s, 4_600.0)[..] else { panic!("not metered") };
        let [(_, cooler, _)] = wrecks.metered(20.0 + 0.5 * afterglow_s, 2.0, afterglow_s, 4_600.0)[..] else { panic!() };
        assert!(hot > 4_800.0 && cooler < hot, "{hot} then {cooler}");
    }

    /// On a clock slowed almost to a stop the debris still spreads and cools, as fast as it would
    /// at the design rate.
    #[test]
    fn a_slow_clock_does_not_hold_the_debris_still() {
        let mut wrecks = Wrecks::default();
        wrecks.last.insert(ShipId(5), last("Aster"));
        let mut world = collapsing(vec![sighting(1, 5, 10_000_000, 20_000_000)], wrecks, 20.0);
        let mut wreck = world.resource_mut::<Wrecks>().fell.remove(0);
        let afterglow_s = B.collapse_afterglow_s;
        let design_s = (afterglow_s / crate::session::TIME_RATE) as f32;
        wreck.collapse(20.0, 1.0, afterglow_s).unwrap();
        let half = wreck.collapse(20.0 + 1e-6, 1.0 + 0.5 * design_s, afterglow_s).unwrap();
        assert!((half.x / half.z - 0.5).abs() < 1e-3, "{} of the afterglow", half.x / half.z);
        assert!(wreck.over(20.0 + 1e-6, 1.0 + 1.01 * design_s, afterglow_s));
    }

    /// Taken while this client's clock is behind the shard's, a collapse is drawn at once, where
    /// its hull went from the contacts, and its spikes keep their delays from it.
    #[test]
    fn a_client_behind_the_shard_draws_the_flash_as_it_is_told() {
        let mut wrecks = Wrecks::default();
        wrecks.last.insert(ShipId(5), last("Aster"));
        let mut world = collapsing(vec![sighting(1, 5, 10_000_000, 20_000_000)], wrecks, 15.0);
        let mut wreck = world.resource_mut::<Wrecks>().fell.remove(0);
        assert_eq!(wreck.collapse(15.0, 1.0, B.collapse_afterglow_s).map(|c| c.x), Some(0.0));
        let ls = 1.0 / lc_world::flight::JULIAN_YEAR_S;
        wreck.at_ly = DVec3::X * 10.0 * ls;
        let behind = wreck.at_ly * 2.0;
        let field = (1.0, 4_500.0, 1.0e4);
        assert_eq!(wreck.spike_on(Some(ShipId(6)), behind, DVec3::ZERO, 15.0 + 19.9, 1.0, field), None);
        assert!(wreck.spike_on(Some(ShipId(6)), behind, DVec3::ZERO, 15.0 + 20.0, 1.0, field).is_some());
    }

    /// A neighbor brightens when the spike's light, bounced off it, reaches the observer: later for
    /// one farther along the line of sight, and never before the collapse's own light.
    #[test]
    fn a_spike_lands_on_each_neighbor_at_its_own_retarded_time() {
        let mut wrecks = Wrecks::default();
        wrecks.last.insert(ShipId(5), last("Aster"));
        let mut world = collapsing(vec![sighting(1, 5, 10_000_000, 20_000_000)], wrecks, 20.0);
        let mut wreck = world.resource_mut::<Wrecks>().fell.remove(0);
        let observer = DVec3::ZERO;
        // Light-seconds: the wreck ten out, one neighbor a second beside it and one as far again behind.
        let ls = 1.0 / lc_world::flight::JULIAN_YEAR_S;
        wreck.at_ly = DVec3::X * 10.0 * ls;
        let beside = wreck.at_ly + DVec3::Y * ls;
        let behind = wreck.at_ly * 2.0;
        assert!((wreck.lands_s(behind, observer) - 40.0).abs() < 1e-6, "{}", wreck.lands_s(behind, observer));
        let lands = wreck.lands_s(beside, observer);
        assert!((lands - (20.0 + 1.0 + 101f64.sqrt() - 10.0)).abs() < 1e-6, "{lands}");
        let field = (1.0, 4_500.0, 1.0e4);
        assert_eq!(wreck.spike_on(Some(ShipId(6)), beside, observer, lands - 1e-3, 1.0, field), None);
        let spot = wreck.spike_on(Some(ShipId(6)), beside, observer, lands, 1.0, field).expect("landed");
        assert!(spot.w > 0.0 && spot.truncate().dot(sim_to_render(-DVec3::Y).as_vec3()) > 0.99, "{spot}");
        assert_eq!(wreck.spike_on(Some(ShipId(5)), wreck.at_ly, observer, 30.0, 1.0, field), None, "on itself");
    }
}
