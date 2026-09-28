//! Craft too far to resolve, and collapses too far to resolve, drawn as points in the sky: 32
//! §The exhaust cone (From a distance), §The field.
//!
//! A craft whose hull would be under [`RESOLVE_PX`] is a point and nothing else: its hull root, and
//! with it the envelope and the aperture glows, is hidden, and it is metered as a point. So its
//! heat is drawn once, by the envelope or by the point. The point is [`crate::hull::Sent`]'s three
//! terms, each over the area it is drawn on near to (the hull's disc, or the envelope's quarter
//! for the heat), and one more for whatever it has lit: the `Glare` its `Presence` hands an
//! observer inside a beam, the whole of it; outside every beam, its lit faces seen obliquely.
//!
//! A collapse is a point where its debris would be under a pixel or two: H10's spike, then its
//! afterglow, closed form in `globals.time` from the frame its light arrived.
//!
//! Burns and emissions are read from their `kind::DRIVE` and `kind::EMIT` sightings as well as
//! from `Presence`, and each is shown for [`FLARE_S`] at least, so one that lit and went out
//! between two presences is still seen.
//!
//! The points share one mesh, written only when something in it changes.

use std::collections::HashSet;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::craft_point_material::{
    ATTRIBUTE_EVENT_CLOCK, ATTRIBUTE_EVENT_LIGHT, ATTRIBUTE_TERMS_A, ATTRIBUTE_TERMS_B, CraftPointMaterialPlugin, TERMS,
};
use em_render::relativistic_starfield_material::ATTRIBUTE_STAR_CORNER;
use em_render::render_space::sim_to_render;
use em_spectra::{Band, PerBand, blackbody};
use glam::DVec3;
use lc_proto::{Glare, ShipId, Spectrum};
use lc_world::emit::Ends;
use lc_world::fitting::Balance;

use crate::field::{Shells, Wrecks};
use crate::hull::{Eye, Hull, Sent};
use crate::lit_faces::LitFaces;
use crate::ship_hull::ShipHull;
use crate::system::M_PER_LY;
use crate::uplink::Contact;

/// Angular radius, pixels, below which a craft's hull, or a wreck's debris at its full spread, is a
/// point: the same crossover as a body's.
pub const RESOLVE_PX: f32 = crate::resolved::RESOLVE_PX;

/// Real seconds a burn or an emission seen only in its sightings is shown for at least, as a
/// collapse's flash is.
pub const FLARE_S: f32 = 0.5;

/// What a flat term's kelvin is written as: no line is that long. `craft_points.wgsl` tests for it.
const FLAT: f32 = -3.0e38;

/// Flares remembered, lit or not yet shown.
const FLARES_KEPT: usize = 256;

pub struct DistantPlugin;

impl Plugin for DistantPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(CraftPointMaterialPlugin).init_resource::<Distant>().init_resource::<Flares>();
    }
}

/// One source of a point's light.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Term {
    #[default]
    Dark,
    /// Band flux is band radiance at `kelvin` times `sr`.
    Blackbody { kelvin: f64, sr: f64 },
    Line { wavelength_m: f64, flux_w_m2: f64 },
    /// The same flux in every band, whatever the source's temperature.
    Flat { per_band_w_m2: f64 },
}

impl Term {
    /// All of a glare, in its spectrum.
    pub fn glare(glare: &Glare) -> Self {
        match glare.spectrum {
            Spectrum::Blackbody { temperature_k } => Term::blackbody_flux(temperature_k, glare.flux_w_m2),
            Spectrum::Line { wavelength_m } => Term::Line { wavelength_m, flux_w_m2: glare.flux_w_m2 },
        }
    }

    /// A blackbody at `kelvin` delivering `flux_w_m2` over every wavelength.
    pub fn blackbody_flux(kelvin: f64, flux_w_m2: f64) -> Self {
        if kelvin <= 0.0 || flux_w_m2 <= 0.0 {
            return Term::Dark;
        }
        Term::Blackbody { kelvin, sr: flux_w_m2 * std::f64::consts::PI / (blackbody::SIGMA * kelvin.powi(4)) }
    }

    /// Radiance a blackbody at `kelvin` scaled, as each of [`Sent`]'s terms is, over `sr`.
    pub fn shaped(radiance: &PerBand<f32>, kelvin: f64, sr: f64) -> Self {
        let unit = crate::session::spectrum_at(kelvin);
        let Some(band) = Band::ALL.into_iter().max_by(|a, b| unit[*a].total_cmp(&unit[*b])) else { return Term::Dark };
        if kelvin <= 0.0 || unit[band] <= 0.0 || radiance[band] <= 0.0 || sr <= 0.0 {
            return Term::Dark;
        }
        Term::Blackbody { kelvin, sr: f64::from(radiance[band]) / f64::from(unit[band]) * sr }
    }

    /// As an observer sees it whose Doppler factor toward the source is `d`: a blackbody at `d`
    /// times the temperature, a line at its wavelength over `d` carrying `d⁴` of its flux, as the
    /// blackbody's total does.
    pub fn seen_at(self, d: f64) -> Self {
        match self {
            Term::Blackbody { kelvin, sr } => Term::Blackbody { kelvin: kelvin * d, sr },
            Term::Line { wavelength_m, flux_w_m2 } => Term::Line { wavelength_m: wavelength_m / d, flux_w_m2: flux_w_m2 * d.powi(4) },
            Term::Flat { per_band_w_m2 } => Term::Flat { per_band_w_m2: per_band_w_m2 * d.powi(4) },
            Term::Dark => Term::Dark,
        }
    }

    /// W/m² in each band.
    pub fn flux(&self) -> PerBand<f64> {
        match *self {
            Term::Blackbody { kelvin, sr } => crate::session::spectrum_at(kelvin).map(|_, x| f64::from(*x) * sr),
            Term::Line { wavelength_m, flux_w_m2 } => PerBand::new(std::array::from_fn(|i| {
                let (lo, hi) = Band::ALL[i].limits_m();
                if (lo..=hi).contains(&wavelength_m) { flux_w_m2 } else { 0.0 }
            })),
            Term::Flat { per_band_w_m2 } => PerBand::splat(per_band_w_m2),
            Term::Dark => PerBand::splat(0.0),
        }
    }

    /// `(kelvin, sr)`, a line as `(−meters, flux)`, a flat term as `(`[`FLAT`]`, flux in each band)`:
    /// `craft_points.wgsl`'s.
    fn encode(&self) -> [f32; 2] {
        match *self {
            Term::Blackbody { kelvin, sr } => [kelvin as f32, sr as f32],
            Term::Line { wavelength_m, flux_w_m2 } => [-wavelength_m as f32, flux_w_m2 as f32],
            Term::Flat { per_band_w_m2 } => [FLAT, per_band_w_m2 as f32],
            Term::Dark => [0.0, 0.0],
        }
    }
}

/// A flash, then a fade of fixed area whose luminosity falls linearly to nothing: H10's spike and
/// afterglow. Real seconds, as `globals.time` reads them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// Wrapped at `wrap_s`.
    pub start_s: f32,
    pub flash_s: f32,
    pub fade_s: f32,
    pub wrap_s: f32,
    pub flash: Term,
    /// At its first temperature.
    pub fade: Term,
}

impl Event {
    /// What it sends `t_s` in: the shader's closed form.
    pub fn at(&self, t_s: f32) -> [Term; 2] {
        let flash = if (0.0..self.flash_s).contains(&t_s) { self.flash } else { Term::Dark };
        let tau = f64::from(t_s / self.fade_s.max(1e-6));
        let fade = match self.fade {
            Term::Blackbody { kelvin, sr } if (0.0..1.0).contains(&tau) => Term::Blackbody { kelvin: kelvin * (1.0 - tau).sqrt().sqrt(), sr },
            _ => Term::Dark,
        };
        [flash, fade]
    }
}

/// One point: where, and what it sends.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub at_ly: DVec3,
    pub terms: [Term; TERMS],
    /// With how many real seconds into it now.
    pub event: Option<(Event, f32)>,
}

impl Point {
    /// W/m² in each band now, before the observer's own Doppler shift, which the shader applies.
    pub fn flux(&self) -> PerBand<f64> {
        let event = self.event.map_or([Term::Dark; 2], |(e, t)| e.at(t));
        self.terms.iter().chain(&event).fold(PerBand::splat(0.0), |sum, term| {
            let f = term.flux();
            sum.map(|band, x| x + f[band])
        })
    }
}

/// Whether something `length_m` long at `distance_m` is too small to draw as a shape.
pub fn unresolved(length_m: f64, distance_m: f64, rad_per_px: f32) -> bool {
    rad_per_px > 0.0 && distance_m > 0.0 && ((0.5 * length_m / distance_m) as f32) / rad_per_px <= RESOLVE_PX
}

/// Observed over emitted frequency for light from a source at `beta` reaching an observer along
/// `toward`, before the observer's own motion.
pub fn doppler_of_source(beta: DVec3, toward: DVec3) -> f64 {
    let b2 = beta.length_squared().min(1.0 - 1e-12);
    let gamma = 1.0 / (1.0 - b2).sqrt();
    1.0 / (gamma * (1.0 - beta.dot(toward.normalize_or_zero())))
}

/// A lit face as seen: along `out`, in the frame `toward` is in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    pub out: DVec3,
    pub kelvin: f64,
    pub area_m2: f64,
}

/// What an emitting craft sends an observer `distance_m` off along `toward` from it: inside one of
/// its beams, the glare it was handed and nothing else, since that glare is its lit faces seen
/// down the beam; outside, each face's radiance over its area seen obliquely, `cos θ`, as a flat
/// face near to is drawn. Faces at more than one temperature are one blackbody carrying their sum.
pub fn emission(glare: Option<&Glare>, faces: &[Face], toward: DVec3, distance_m: f64) -> Term {
    if let Some(glare) = glare {
        return Term::glare(glare);
    }
    let toward = toward.normalize_or_zero();
    let (mut sr, mut weighted) = (0.0, 0.0);
    for face in faces.iter().filter(|f| f.kelvin > 0.0) {
        let seen = face.area_m2 * face.out.normalize_or_zero().dot(toward).max(0.0) / (distance_m * distance_m);
        sr += seen;
        weighted += seen * face.kelvin.powi(4);
    }
    if sr <= 0.0 {
        return Term::Dark;
    }
    Term::Blackbody { kelvin: (weighted / sr).sqrt().sqrt(), sr }
}

/// A craft's point before its emission: [`Sent`]'s terms over the areas they are drawn on.
pub struct Seen {
    pub sent: Sent,
    /// Its star's, whose light it reflects; `None` in the dark.
    pub star_k: Option<f64>,
    pub field_k: f64,
    pub lamp_k: f64,
    /// The hull's disc, which carries the reflected light and the windows.
    pub disc_sr: f64,
    /// A quarter of the envelope, which carries the heat.
    pub envelope_sr: f64,
}

/// A craft's point, with `emission` and every term seen at the source's Doppler factor `d`.
pub fn craft(at_ly: DVec3, seen: &Seen, emission: Term, d: f64) -> Point {
    let reflected = seen.star_k.map_or(Term::Dark, |k| Term::shaped(&seen.sent.reflected, k, seen.disc_sr));
    let terms = [
        reflected,
        Term::shaped(&seen.sent.thermal, seen.field_k, seen.envelope_sr),
        Term::shaped(&seen.sent.windows, seen.lamp_k, seen.disc_sr),
        emission,
    ];
    Point { at_ly, terms: terms.map(|t| t.seen_at(d)), event: None }
}

/// A collapse `distance_m` off, from its `released_j` and when its light arrived: the spike taken
/// as [`crate::field::SPIKE_S`] long, then H10's afterglow.
///
/// **The spike cheats.** At `collapse_spike_k` it would put a millionth of itself in any band an
/// eye has, and the most violent thing in the game would be a faint star. So its flux is drawn
/// flat, the same in every band, which keeps its energy and shows it white in any mapping. The
/// photometry an instrument reads is still the blackbody (`lc_world::afterglow`).
pub fn collapse(far: &crate::field::Far, distance_m: f64, balance: &Balance, wrap_s: f32) -> Point {
    let afterglow = lc_world::afterglow::Afterglow::of(far.at_ly, 0.0, far.released_j, balance);
    let sphere_m2 = 4.0 * std::f64::consts::PI * distance_m * distance_m;
    let event = Event {
        start_s: far.clock.x,
        flash_s: far.clock.y,
        fade_s: far.clock.z,
        wrap_s,
        flash: Term::Flat { per_band_w_m2: afterglow.spike_j / crate::field::SPIKE_S / sphere_m2 / Band::ALL.len() as f64 },
        fade: Term::Blackbody { kelvin: afterglow.limit_k, sr: 0.25 * afterglow.area_m2() / (distance_m * distance_m) },
    };
    Point { at_ly: far.at_ly, terms: [Term::Dark; TERMS], event: Some((event, far.since_s)) }
}

/// A burn or an emission from its sightings.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Lit {
    Drive(f64),
    Glare(Glare),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Flare {
    source: ShipId,
    /// The emit's beam; `None` for the drive.
    beam: Option<i64>,
    lit: Lit,
    /// When its going out arrives, coordinate seconds.
    out_s: Option<f64>,
    /// How far the shard's clock was ahead of this one when it was taken.
    ahead_s: f64,
    /// Real seconds it was first shown.
    shown: Option<f32>,
}

impl Flare {
    fn lit(&mut self, now_s: f64, real_s: f32) -> bool {
        let open = self.out_s.is_none_or(|out_s| now_s + self.ahead_s < out_s);
        let first = *self.shown.get_or_insert(real_s);
        open || real_s - first < FLARE_S
    }
}

/// What an EMIT sighting carries under `emission`, as far as an eye needs it.
#[derive(serde::Deserialize)]
struct Emitted {
    beam: i64,
    half_angle_rad: f64,
    spectrum: Spectrum,
    power_w: f64,
    #[serde(default)]
    burst_j: f64,
}

#[derive(serde::Deserialize)]
struct Carrying {
    emission: Emitted,
}

/// Burns and emissions from their sightings, until each has gone out and been shown.
#[derive(Resource, Default)]
pub struct Flares {
    taken: HashSet<i64>,
    flares: Vec<Flare>,
}

impl Flares {
    /// Take each DRIVE and EMIT sighting once, oldest first. The flux inside an emission is
    /// `lc_world::emit::flux_w_m2` over the distance its light came, as the shard's landing has it.
    pub fn take(&mut self, seen: &[lc_proto::Sighting], now_s: f64) {
        let mut new: Vec<&lc_proto::Sighting> = seen
            .iter()
            .filter(|s| matches!(s.kind, lc_proto::kind::DRIVE | lc_proto::kind::EMIT))
            .filter(|s| !self.taken.contains(&s.event_id))
            .collect();
        new.sort_by_key(|s| s.arrive_t);
        for sighting in new {
            self.taken.insert(sighting.event_id);
            let source = ShipId(sighting.source_id);
            let arrive_s = sighting.arrive_t as f64 * 1e-6;
            let (beam, lit) = if sighting.kind == lc_proto::kind::DRIVE {
                let Ok(change) = serde_json::from_str::<lc_proto::DriveChange>(&sighting.payload) else { continue };
                (None, (change.power_w > 0.0).then_some(Lit::Drive(change.power_w)))
            } else {
                let Ok(Carrying { emission }) = serde_json::from_str::<Carrying>(&sighting.payload) else { continue };
                if emission.burst_j > 0.0 {
                    continue;
                }
                let distance_m = (sighting.arrive_t - sighting.emitted_t) as f64 * 1e-6 * lc_world::flight::C_M_S;
                let flux_w_m2 = lc_world::emit::flux_w_m2(emission.power_w, emission.half_angle_rad, distance_m);
                (Some(emission.beam), (flux_w_m2 > 0.0).then_some(Lit::Glare(Glare { spectrum: emission.spectrum, flux_w_m2 })))
            };
            for open in self.flares.iter_mut().filter(|f| f.source == source && f.beam == beam && f.out_s.is_none()) {
                open.out_s = Some(arrive_s);
            }
            if let Some(lit) = lit {
                let ahead_s = (arrive_s - now_s).max(0.0);
                self.flares.push(Flare { source, beam, lit, out_s: None, ahead_s, shown: None });
            }
        }
        let taken: HashSet<i64> = seen.iter().map(|s| s.event_id).collect();
        self.taken.retain(|id| taken.contains(id));
        let excess = self.flares.len().saturating_sub(FLARES_KEPT);
        self.flares.drain(..excess);
    }

    /// Drop every flare that is out and has been shown, or will not be: its craft is not a point.
    fn sweep(&mut self, now_s: f64, real_s: f32, drawn: impl Fn(ShipId) -> bool) {
        self.flares.retain(|f| {
            let open = f.out_s.is_none_or(|out_s| now_s + f.ahead_s < out_s);
            open || (drawn(f.source) && f.shown.is_none_or(|at| real_s - at < FLARE_S))
        });
    }

    /// What `source`'s drive is sending by its sightings now, if it is lit by them.
    pub fn drive_w(&mut self, source: ShipId, now_s: f64, real_s: f32) -> Option<f64> {
        self.flares
            .iter_mut()
            .filter(|f| f.source == source)
            .filter_map(|f| match f.lit {
                Lit::Drive(w) => f.lit(now_s, real_s).then_some(w),
                Lit::Glare(_) => None,
            })
            .reduce(f64::max)
    }

    /// `source` as its light reaches this ship from inside its beams: what its presence stated, or
    /// what its sightings say when that is more, their flux summed in the brightest one's spectrum.
    pub fn glare(&mut self, source: ShipId, stated: Option<Glare>, now_s: f64, real_s: f32) -> Option<Glare> {
        let lit: Vec<Glare> = self
            .flares
            .iter_mut()
            .filter(|f| f.source == source)
            .filter_map(|f| match f.lit {
                Lit::Glare(g) => f.lit(now_s, real_s).then_some(g),
                Lit::Drive(_) => None,
            })
            .collect();
        let brightest = lit.iter().max_by(|a, b| a.flux_w_m2.total_cmp(&b.flux_w_m2));
        let sighted = brightest.map(|g| Glare { spectrum: g.spectrum, flux_w_m2: lit.iter().map(|g| g.flux_w_m2).sum() });
        match (stated, sighted) {
            (Some(s), Some(g)) => Some(if g.flux_w_m2 > s.flux_w_m2 { g } else { s }),
            (s, g) => s.or(g),
        }
    }
}

/// Which craft and which collapses are points this frame, and what the points send.
#[derive(Resource, Default)]
pub struct Distant {
    points: HashSet<ShipId>,
    /// Wrecks drawn as debris. Every other is a point, so one not yet decided is not drawn twice.
    debris: HashSet<i64>,
    /// Each point's flux, for the exposure. Last frame's, since the scene is metered first.
    pub metered: Vec<PerBand<f32>>,
    uploaded: Option<(DVec3, Vec<Point>)>,
}

impl Distant {
    /// Whether `craft` is drawn as a point, and so not as a hull, an envelope or faces.
    pub fn is_point(&self, craft: Option<ShipId>) -> bool {
        craft.is_some_and(|id| self.points.contains(&id))
    }

    /// Whether a wreck is drawn as debris rather than as a point.
    pub fn is_debris(&self, event_id: i64) -> bool {
        self.debris.contains(&event_id)
    }

    /// Decide for this frame. The craft the camera is behind is never a point.
    pub fn decide(
        &mut self,
        contacts: &[Contact],
        eye: &Eye,
        wrecks: impl Iterator<Item = (i64, DVec3, f64)>,
        rad_per_px: f32,
    ) {
        let far = |at_ly: DVec3, length_m: f64| unresolved(length_m, at_ly.distance(eye.at_ly) * M_PER_LY, rad_per_px);
        self.points = contacts
            .iter()
            .filter(|c| Some(c.ship_id) != eye.anchored && far(c.position_ly, c.length_m))
            .map(|c| c.ship_id)
            .collect();
        self.debris = wrecks.filter(|(_, at_ly, reach_m)| *reach_m > 0.0 && !far(*at_ly, 2.0 * reach_m)).map(|(id, ..)| id).collect();
    }
}

/// Before the scene is metered: which craft and wrecks are points.
pub fn resolve(
    uplink: Res<crate::uplink::Uplink>,
    eye: Res<Eye>,
    (wrecks, shells): (Res<Wrecks>, Res<Shells>),
    camera: Query<(&Projection, &Camera), With<crate::app::SkyCamera>>,
    mut distant: ResMut<Distant>,
) {
    let rad_per_px = crate::starfield::camera_scale(&camera);
    distant.decide(&uplink.contacts, &eye, wrecks.reach(&shells), rad_per_px);
}

/// After the hulls, the faces and the fields: hide what is a point, and draw the points.
#[allow(clippy::too_many_arguments)]
pub fn draw_points(
    (game, uplink, eye, ui, time): (Res<crate::app::Game>, Res<crate::uplink::Uplink>, Res<Eye>, Res<crate::app::Ui>, Res<Time>),
    (faces, wrecks, sky): (Res<LitFaces>, Res<Wrecks>, Option<Res<crate::starfield::Starfield>>),
    (mut flares, mut distant): (ResMut<Flares>, ResMut<Distant>),
    mut meshes: ResMut<Assets<Mesh>>,
    mut roots: Query<(&ShipHull, &Transform, &mut Visibility), Without<Hull>>,
    mut ovoids: Query<(&Hull, &mut Visibility), Without<ShipHull>>,
) {
    let session = &game.0;
    let (now_s, real_s) = (session.coordinate_time_s(), time.elapsed_secs());
    flares.take(&uplink.seen, now_s);

    let shown = |craft: Option<ShipId>| if distant.is_point(craft) { Visibility::Hidden } else { Visibility::Inherited };
    // Never the player's own, which is not a point and whose showing `--first-person` decides.
    for (hull, _, mut visibility) in roots.iter_mut().filter(|(h, ..)| h.craft().is_some()) {
        visibility.set_if_neq(shown(hull.craft()));
    }
    for (hull, mut visibility) in ovoids.iter_mut().filter(|(h, _)| h.0.is_some()) {
        visibility.set_if_neq(shown(hull.0));
    }

    let star = crate::hull::lighting(session);
    let lamp_k = crate::ship_hull::lamp_of("living").map_or(0.0, |(_, k)| k);
    let balance = uplink.fitting.as_ref().map_or(Balance::DEFAULT, |f| f.balance.into());
    let mut points = Vec::new();
    for contact in uplink.contacts.iter().filter(|c| distant.is_point(Some(c.ship_id))) {
        let toward = eye.at_ly - contact.position_ly;
        let distance_m = toward.length() * M_PER_LY;
        let glow = contact.glow;
        let seen = Seen {
            sent: crate::hull::radiance_at(session, star, contact.position_ly, toward, glow),
            star_k: star.map(|(_, _, k)| k),
            field_k: glow.temperature_k,
            lamp_k,
            disc_sr: f64::from(crate::hull::solid_angle_sr(contact.length_m, distance_m)),
            envelope_sr: 0.25 * glow.envelope_m2 / (distance_m * distance_m),
        };
        let glare = flares.glare(contact.ship_id, contact.glare, now_s, real_s);
        let drive_w = flares.drive_w(contact.ship_id, now_s, real_s).unwrap_or(0.0).max(contact.drive_w);
        let rotation = roots.iter().find(|(h, ..)| h.craft() == Some(contact.ship_id)).map(|(_, t, _)| t.rotation);
        let lit = lit_faces(contact.emit.with_drive(drive_w), faces.apertures(Some(contact.ship_id)), rotation);
        let emission = emission(glare.as_ref(), &lit, sim_to_render(toward), distance_m);
        points.push(craft(contact.position_ly, &seen, emission, doppler_of_source(contact.beta, toward)));
    }
    let wrap_s = time.wrap_period().as_secs_f32();
    for far in wrecks.far(real_s, ui.time_rate, balance.collapse_afterglow_s, |id| distant.is_debris(id)) {
        let distance_m = far.at_ly.distance(eye.at_ly) * M_PER_LY;
        if distance_m > 0.0 {
            points.push(collapse(&far, distance_m, &balance, wrap_s));
        }
    }
    flares.sweep(now_s, real_s, |id| distant.is_point(Some(id)));

    distant.metered = points.iter().map(|p| p.flux().map(|_, x| *x as f32)).collect();
    let Some(sky) = sky else { return };
    let origin = sky.origin_ly;
    if distant.uploaded.as_ref().is_some_and(|(o, drawn)| *o == origin && same(drawn, &points, eye.at_ly)) {
        return;
    }
    if let Some(mut mesh) = meshes.get_mut(&sky.crafts.mesh) {
        *mesh = build_mesh(&points, origin);
    }
    distant.uploaded = Some((origin, points));
}

/// A craft's faces in render axes, lit by `ends`.
fn lit_faces(ends: Ends, apertures: Option<&[lc_world::form::capacity::Aperture]>, rotation: Option<Quat>) -> Vec<Face> {
    let (Some(apertures), Some(rotation)) = (apertures, rotation) else { return Vec::new() };
    apertures
        .iter()
        .map(|a| Face { out: (rotation * a.out.as_vec3()).as_dvec3(), kelvin: ends.face_k(a), area_m2: a.area_m2() })
        .collect()
}

/// Whether two sets of points draw alike: in place to a millionth of their distance, and their light
/// to a thousandth.
fn same(a: &[Point], b: &[Point], eye_ly: DVec3) -> bool {
    let close = |x: f64, y: f64| (x - y).abs() <= 1e-3 * x.abs().max(y.abs());
    let term = |x: &Term, y: &Term| match (x, y) {
        (Term::Blackbody { kelvin: k, sr: s }, Term::Blackbody { kelvin: l, sr: t }) => close(*k, *l) && close(*s, *t),
        (Term::Line { wavelength_m: k, flux_w_m2: s }, Term::Line { wavelength_m: l, flux_w_m2: t }) => close(*k, *l) && close(*s, *t),
        (Term::Flat { per_band_w_m2: s }, Term::Flat { per_band_w_m2: t }) => close(*s, *t),
        (x, y) => x == y,
    };
    a.len() == b.len()
        && a.iter().zip(b).all(|(p, q)| {
            p.at_ly.distance(q.at_ly) <= 1e-6 * p.at_ly.distance(eye_ly)
                && p.terms.iter().zip(&q.terms).all(|(x, y)| term(x, y))
                && p.event.map(|(e, _)| e) == q.event.map(|(e, _)| e)
        })
}

/// Four vertices per point, positions relative to `origin_ly`, in render axes.
pub fn build_mesh(points: &[Point], origin_ly: DVec3) -> Mesh {
    let n = points.len().max(1);
    let (mut positions, mut corners) = (Vec::with_capacity(n * 4), Vec::with_capacity(n * 4));
    let (mut terms_a, mut terms_b) = (Vec::with_capacity(n * 4), Vec::with_capacity(n * 4));
    let (mut clocks, mut events) = (Vec::with_capacity(n * 4), Vec::with_capacity(n * 4));
    let mut indices = Vec::with_capacity(n * 6);
    for point in points {
        let at = sim_to_render(point.at_ly - origin_ly).as_vec3().to_array();
        let [t0, t1, t2, t3] = point.terms.map(|t| t.encode());
        let (clock, event) = match point.event {
            Some((e, _)) => {
                let [fk, fs] = e.flash.encode();
                let [gk, gs] = e.fade.encode();
                ([e.start_s, e.flash_s, e.fade_s, e.wrap_s], [fk, fs, gk, gs])
            }
            None => ([-1.0, 0.0, 0.0, 0.0], [0.0; 4]),
        };
        let base = positions.len() as u32;
        for corner in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            positions.push(at);
            corners.push(corner);
            terms_a.push([t0[0], t0[1], t1[0], t1[1]]);
            terms_b.push([t2[0], t2[1], t3[0], t3[1]]);
            clocks.push(clock);
            events.push(event);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    // A mesh with no vertices logs a use-after-free each frame: see `starfield::build_mesh`.
    if points.is_empty() {
        positions.push([0.0; 3]);
        corners.push([0.0; 2]);
        terms_a.push([0.0; 4]);
        terms_b.push([0.0; 4]);
        clocks.push([-1.0, 0.0, 0.0, 0.0]);
        events.push([0.0; 4]);
        indices.extend_from_slice(&[0, 0, 0]);
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(ATTRIBUTE_STAR_CORNER, corners);
    mesh.insert_attribute(ATTRIBUTE_TERMS_A, terms_a);
    mesh.insert_attribute(ATTRIBUTE_TERMS_B, terms_b);
    mesh.insert_attribute(ATTRIBUTE_EVENT_CLOCK, clocks);
    mesh.insert_attribute(ATTRIBUTE_EVENT_LIGHT, events);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

#[cfg(test)]
mod tests {
    use lc_world::afterglow::Afterglow;
    use lc_world::form::capacity::apertures;
    use lc_world::form::presets::{Builtin, turned_fore};

    use super::*;
    use crate::session::Session;

    const B: Balance = Balance::DEFAULT;
    const LY_M: f64 = M_PER_LY;

    fn session() -> Session {
        Session::new(&lc_world::sky::AuthoredStars::sample(), 3)
    }

    fn assert_near(got: PerBand<f64>, want: PerBand<f64>, tolerance: f64, what: &str) {
        for band in Band::ALL {
            let (g, w) = (got[band], want[band]);
            assert!((g - w).abs() <= tolerance * w.abs().max(1e-300), "{what}: {band:?} {g:e} against {w:e}");
        }
    }

    /// A blackbody's share of all its flux in each band, from `em_spectra` rather than the table
    /// the point reads.
    fn share(kelvin: f64) -> PerBand<f64> {
        let total = blackbody::SIGMA * kelvin.powi(4) / std::f64::consts::PI;
        PerBand::new(std::array::from_fn(|i| blackbody::band_radiance(Band::ALL[i], kelvin) / total))
    }

    fn dark() -> Seen {
        let zero = PerBand::splat(0.0);
        Seen { sent: Sent { reflected: zero, thermal: zero, windows: zero }, star_k: None, field_k: 400.0, lamp_k: 2700.0, disc_sr: 0.0, envelope_sr: 0.0 }
    }

    /// The two-ended plate in render axes, nose along `fore`, and its faces lit by `ends`.
    fn plate(fore: DVec3, ends: Ends) -> Vec<Face> {
        let faces = apertures(&turned_fore(Builtin::Plate.form(), 1), &B).unwrap();
        lit_faces(ends, Some(&faces), Some(crate::hull::frame(fore, None, 0.0)))
    }

    /// **Inside the cone the point is the glare it was handed**, all of it in its spectrum, and not
    /// the face as well: the face seen straight down the beam is that same light.
    #[test]
    fn inside_the_cone_the_point_is_the_glare_it_was_handed() {
        let kelvin = 5.3e5;
        let glare = Glare { spectrum: Spectrum::Blackbody { temperature_k: kelvin }, flux_w_m2: 5.0e-11 };
        // The observer astern, looking into the lit aft faces.
        let faces = plate(DVec3::X, Ends::default().with_drive(1.0e20));
        let toward = sim_to_render(-DVec3::X);
        assert!(emission(None, &faces, toward, LY_M) != Term::Dark, "premise: the faces face the observer");
        let point = craft(DVec3::X, &dark(), emission(Some(&glare), &faces, toward, LY_M), 1.0);
        assert_near(point.flux(), share(kelvin).map(|_, s| s * glare.flux_w_m2), 5e-3, "in the cone");

        let line = Glare { spectrum: Spectrum::Line { wavelength_m: 551e-9 }, flux_w_m2: 3.0e-12 };
        let point = craft(DVec3::X, &dark(), emission(Some(&line), &faces, toward, LY_M), 1.0);
        let mut want = PerBand::splat(0.0);
        want[Band::V] = line.flux_w_m2;
        assert_eq!(point.flux(), want, "a beam's line lands in its band alone");
    }

    /// **A beam aimed past the observer is not drawn.** Its glare never arrives, so what is left is
    /// the craft and the faces it has lit, and seen from behind a beam out of the bow, nothing of
    /// the faces either.
    #[test]
    fn a_beam_aimed_past_the_observer_is_not_drawn() {
        let fore = plate(DVec3::X, Ends { fore_w: 2.0e19, aft_w: 0.0 });
        assert_eq!(emission(None, &fore, sim_to_render(-DVec3::X), LY_M), Term::Dark, "astern of a beam out of the bow");
        let abeam = emission(None, &fore, sim_to_render(DVec3::new(1.0, 1.0, 0.0)), LY_M);
        assert!(matches!(abeam, Term::Blackbody { .. }), "the bow's faces seen obliquely: {abeam:?}");
        let unlit = craft(DVec3::X, &dark(), Term::Dark, 1.0);
        assert_eq!(craft(DVec3::X, &dark(), emission(None, &fore, sim_to_render(-DVec3::X), LY_M), 1.0), unlit);
    }

    /// Outside the cone a face is seen at `cos θ`, a flat face's radiance over its projected area,
    /// so the burn brightens by orders of magnitude as its cone reaches the observer.
    #[test]
    fn a_burn_brightens_by_orders_of_magnitude_as_its_cone_arrives() {
        let power_w = 1.0e20;
        let faces = plate(DVec3::X, Ends::default().with_drive(power_w));
        let edge = B.drive_spread_rad * 1.01;
        let outside = sim_to_render(DVec3::new(-edge.cos(), edge.sin(), 0.0));
        let face = emission(None, &faces, outside, LY_M);
        let bolometric = |t: Term| match t {
            Term::Blackbody { kelvin, sr } => blackbody::SIGMA * kelvin.powi(4) / std::f64::consts::PI * sr,
            _ => 0.0,
        };
        let seen = bolometric(face);
        let lit_w: f64 = faces.iter().filter(|f| f.kelvin > 0.0).map(|f| blackbody::SIGMA * f.kelvin.powi(4) * f.area_m2).sum();
        assert!((lit_w / power_w - 1.0).abs() < 1e-6, "premise: the faces radiate all of it");
        let want = power_w * edge.cos() / (std::f64::consts::PI * LY_M * LY_M);
        assert!((seen / want - 1.0).abs() < 1e-3, "{seen:e} against {want:e}");
        let inside = lc_world::emit::flux_w_m2(power_w, B.drive_spread_rad, LY_M);
        assert!(inside > 100.0 * seen, "{inside:e} inside, {seen:e} just outside");
    }

    /// **An unlit craft is what it sends summed**: `Sent`'s reflected light and windows over the
    /// hull's disc, and its heat over a quarter of its envelope.
    #[test]
    fn an_unlit_craft_is_the_sum_of_what_it_sends() {
        let session = session();
        let au_ly = lc_world::navigation::AU / M_PER_LY;
        let star = Some((DVec3::ZERO, em_spectra::stellar::SOLAR_RADIUS, 5772.0));
        let glow = lc_proto::Glow { temperature_k: 900.0, shade: lc_proto::Shade::Clear, envelope_m2: lc_world::fitting::STARTING_ENVELOPE_M2 };
        let at = DVec3::X * au_ly;
        let toward = DVec3::new(-1.0, 0.3, 0.0);
        let sent = crate::hull::radiance_at(&session, star, at, toward, glow);
        let (disc_sr, envelope_sr) = (3.0e-16, 2.0e-16);
        let lamp_k = crate::ship_hull::lamp_of("living").unwrap().1;
        let seen = Seen { sent, star_k: Some(5772.0), field_k: 900.0, lamp_k, disc_sr, envelope_sr };
        let point = craft(at, &seen, Term::Dark, 1.0);
        let want = PerBand::new(std::array::from_fn(|i| {
            let b = Band::ALL[i];
            f64::from(sent.reflected[b] + sent.windows[b]) * disc_sr + f64::from(sent.thermal[b]) * envelope_sr
        }));
        assert!(want.as_array().iter().all(|x| *x > 0.0), "premise: every term lit");
        assert_near(point.flux(), want, 1e-5, "unlit");
    }

    fn far(since_s: f32) -> crate::field::Far {
        crate::field::Far { at_ly: DVec3::ZERO, released_j: 1.4e26, clock: Vec3::new(10.0, 0.5, 300.0), since_s }
    }

    /// **A collapse flashes as its light arrives and fades on H10's curve.** The flash carries the
    /// whole spike, the same in every band.
    #[test]
    fn a_collapse_flashes_and_then_fades_as_the_afterglow_does() {
        let d = LY_M;
        let point = |t| collapse(&far(t), d, &B, 3600.0);
        let afterglow = Afterglow::of(DVec3::ZERO, 0.0, 1.4e26, &B);
        let spike_w = afterglow.spike_j / crate::field::SPIKE_S;
        let flash = PerBand::splat(spike_w / (4.0 * std::f64::consts::PI * d * d) / Band::ALL.len() as f64);
        let (event, _) = point(0.0).event.unwrap();
        assert_eq!(event.flash.flux(), flash, "flat, and all of the spike");
        let at_first = lc_world::glow::thermal(&afterglow.glow_at(0.0).unwrap(), d);
        assert_near(point(0.1).flux(), flash.map(|b, x| x + at_first[b]), 1e-2, "the flash");

        for fraction in [0.3, 0.6, 0.9] {
            let t = 300.0 * fraction as f32;
            let glow = afterglow.glow_at(fraction * afterglow.duration_s).unwrap();
            assert_near(point(t).flux(), lc_world::glow::thermal(&glow, d), 1e-2, &format!("{fraction} through"));
        }
        assert_eq!(point(300.0).flux(), PerBand::splat(0.0), "over");
    }

    fn sighting(event_id: i64, kind: i16, emitted_t: i64, arrive_t: i64, payload: String) -> lc_proto::Sighting {
        lc_proto::Sighting { event_id, source_id: 4, arrive_t, emitted_t, direction: [1.0, 0.0, 0.0], strength: 1.0, kind, payload }
    }

    fn drive(power_w: f64) -> String {
        serde_json::to_string(&lc_proto::DriveChange { power_w, facing: [1.0, 0.0, 0.0] }).unwrap()
    }

    /// **A burn seen only in its DRIVE sightings still shows**, though it lit and went out between
    /// two presences that both said the drive was dark: for [`FLARE_S`] of real time from the frame
    /// it is first drawn.
    #[test]
    fn a_burn_seen_only_in_its_sightings_still_shows() {
        let mut flares = Flares::default();
        let seen = vec![
            sighting(2, lc_proto::kind::DRIVE, 5_000_000, 12_000_000, drive(0.0)),
            sighting(1, lc_proto::kind::DRIVE, 3_000_000, 10_000_000, drive(4.0e19)),
        ];
        flares.take(&seen, 20.0);
        flares.take(&seen, 20.0);
        let source = ShipId(4);
        assert_eq!(flares.drive_w(source, 20.0, 100.0), Some(4.0e19), "the burn between the presences");
        assert_eq!(flares.drive_w(source, 20.0, 100.0 + 0.9 * FLARE_S), Some(4.0e19));
        let faces = plate(DVec3::X, Ends::default().with_drive(flares.drive_w(source, 20.0, 100.1).unwrap()));
        assert!(emission(None, &faces, sim_to_render(-DVec3::X), LY_M) != Term::Dark, "drawn");
        assert_eq!(flares.drive_w(source, 20.0, 100.0 + 1.1 * FLARE_S), None, "held past its flare");
        flares.sweep(20.0, 100.0 + 1.1 * FLARE_S, |_| true);
        assert!(flares.flares.is_empty());

        // Still burning: shown for as long as it is lit.
        let mut flares = Flares::default();
        flares.take(&[sighting(3, lc_proto::kind::DRIVE, 3_000_000, 10_000_000, drive(1.0e19))], 20.0);
        assert_eq!(flares.drive_w(source, 20.0, 0.0), Some(1.0e19));
        assert_eq!(flares.drive_w(source, 20.0, 1.0e3), Some(1.0e19));
    }

    /// An emission seen only in its EMIT sightings is a glare at the flux its cone carries over the
    /// distance its light came, as the shard's landing has it; a presence's brighter glare wins.
    #[test]
    fn an_emission_seen_only_in_its_sightings_is_a_glare() {
        let payload = |power_w: f64| {
            let spectrum = serde_json::to_value(Spectrum::Blackbody { temperature_k: 5.0e5 }).unwrap();
            serde_json::json!({ "emission": { "beam": 9, "from": [0.0, 0.0, 0.0], "axis": [1.0, 0.0, 0.0], "half_angle_rad": 0.08, "spectrum": spectrum, "power_w": power_w, "burst_j": 0.0 } }).to_string()
        };
        let year_us = (lc_world::flight::JULIAN_YEAR_S * 1.0e6) as i64;
        let seen = vec![
            sighting(1, lc_proto::kind::EMIT, 0, year_us, payload(1.0e20)),
            sighting(2, lc_proto::kind::EMIT, 1_000_000, year_us + 1_000_000, payload(0.0)),
        ];
        let mut flares = Flares::default();
        flares.take(&seen, year_us as f64 * 1e-6 + 5.0);
        let now = year_us as f64 * 1e-6 + 5.0;
        let glare = flares.glare(ShipId(4), None, now, 1.0).expect("the emission");
        let want = lc_world::emit::flux_w_m2(1.0e20, 0.08, lc_world::flight::JULIAN_YEAR_S * lc_world::flight::C_M_S);
        assert!((glare.flux_w_m2 / want - 1.0).abs() < 1e-9, "{} {want}", glare.flux_w_m2);
        let brighter = Glare { flux_w_m2: 2.0 * want, ..glare };
        assert_eq!(flares.glare(ShipId(4), Some(brighter), now, 1.0), Some(brighter));
        assert_eq!(flares.glare(ShipId(4), None, now, 1.0 + 1.1 * FLARE_S), None, "gone out");
    }

    fn contact(id: i64, at_ly: DVec3) -> Contact {
        let presence = lc_proto::Presence {
            ship_id: ShipId(id),
            name: format!("ship {id}"),
            length_m: 500.0,
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
        };
        Contact::seen(presence, None)
    }

    /// **No craft is both an envelope and a point**, either side of the crossover, and the craft the
    /// camera is behind is never a point however far it is.
    #[test]
    fn no_craft_is_both_an_envelope_and_a_point() {
        let rad_per_px = 7.67e-4;
        let crossover_m = 250.0 / (RESOLVE_PX * rad_per_px) as f64;
        let contacts: Vec<Contact> = [0.5, 0.99, 1.01, 2.0, 1.0e9]
            .iter()
            .enumerate()
            .map(|(i, k)| contact(i as i64 + 1, DVec3::X * k * crossover_m / M_PER_LY))
            .collect();
        let mut eye = Eye::default();
        let mut distant = Distant::default();
        distant.decide(&contacts, &eye, std::iter::empty(), rad_per_px);
        let points: Vec<bool> = contacts.iter().map(|c| distant.is_point(Some(c.ship_id))).collect();
        assert_eq!(points, [false, false, true, true, true]);
        for c in &contacts {
            let craft = Some(c.ship_id);
            assert_ne!(crate::field::draws_envelope(&distant, craft), distant.is_point(craft), "{:?}", c.ship_id);
        }
        assert!(!distant.is_point(None), "the player's own");
        eye.anchored = Some(ShipId(5));
        distant.decide(&contacts, &eye, std::iter::empty(), rad_per_px);
        assert!(!distant.is_point(Some(ShipId(5))), "the craft the camera is behind");
    }

    /// A wreck is debris only where its full spread would be resolved; one of unknown shape is a point.
    #[test]
    fn a_wreck_is_debris_only_where_it_would_be_resolved() {
        let rad_per_px = 7.67e-4;
        let mut distant = Distant::default();
        let near = DVec3::X * 1.0e4 / M_PER_LY;
        let wrecks = [(1, near, 300.0), (2, DVec3::X, 300.0), (3, near, 0.0)];
        distant.decide(&[], &Eye::default(), wrecks.into_iter(), rad_per_px);
        assert_eq!([1, 2, 3].map(|id| distant.is_debris(id)), [true, false, false]);
    }

    /// Head on at `β`, the textbook `√((1 + β) / (1 − β))`, and its inverse going away.
    #[test]
    fn a_source_is_doppler_shifted_by_its_own_motion() {
        let beta: f64 = 0.3;
        let want = ((1.0 + beta) / (1.0 - beta)).sqrt();
        assert!((doppler_of_source(DVec3::X * beta, DVec3::X) / want - 1.0).abs() < 1e-12);
        assert!((doppler_of_source(-DVec3::X * beta, DVec3::X) * want - 1.0).abs() < 1e-12);
        let hot = Term::Blackbody { kelvin: 1000.0, sr: 1.0 }.seen_at(want);
        assert_eq!(hot, Term::Blackbody { kelvin: 1000.0 * want, sr: 1.0 });
    }

    /// The shader's band limits are `em_spectra`'s, where a line is sorted into its band.
    #[test]
    fn the_shaders_bands_are_em_spectras() {
        let wgsl = include_str!("../assets/shaders/craft_points.wgsl");
        let read = |name: &str| -> Vec<f64> {
            let line = wgsl.lines().find(|l| l.starts_with(&format!("const {name}"))).unwrap();
            let inner = &line[line.rfind('(').unwrap() + 1..line.rfind(')').unwrap()];
            inner.split(',').map(|x| x.trim().parse().unwrap()).collect()
        };
        let (lo, hi) = (read("BAND_LO"), read("BAND_HI"));
        for (i, band) in Band::ALL.iter().enumerate() {
            let (l, h) = band.limits_m();
            assert!((lo[i] / l - 1.0).abs() < 1e-4 && (hi[i] / h - 1.0).abs() < 1e-4, "{band:?}: {} {} against {l} {h}", lo[i], hi[i]);
        }
    }
}
