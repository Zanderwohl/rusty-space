//! Beauty shots: every [`PERIOD_S`] of real time, a photograph through the ship's own telescope
//! of something worth looking at, shown in a square beside the map's. Experimental, and off
//! unless asked for. `lightcone/docs/28-beauty-shots.md` is the design.
//!
//! The camera stands where the sky's does, at the render origin, and draws the same layer, so a
//! photograph costs no second scene: only one extra pass, one frame in [`PERIOD_S`]. What the
//! two views cannot share is handled where it lives — a star's size in pixels by
//! `drawn_rad_per_px` in the starfield's uniform, a body only the telescope resolves by
//! [`ShotBody`], and the ship's own hull by `app::SKY_ONLY_LAYER`.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Exposure, Hdr, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy_egui::{EguiContexts, EguiUserTextures, egui};
use em_render::render_space::sim_to_render;
use glam::DVec3;
use lc_world::knowledge::survey::Duty;
use lc_world::knowledge::{BodyId, Subject as Known};
use lc_world::motion::Motive;
use lc_world::sky::StarId;

use crate::app::{Game, Ui};
use crate::session::Session;
use crate::system::{Drawable, M_PER_LY};

/// Drawn by the telescope and nothing else: a sphere for a body the sky still shows as a point.
pub const SHOT_LAYER: usize = 4;

const PERIOD_S: f32 = 10.0;
/// Until the first, so a run photographed a second or two in has one.
const FIRST_S: f32 = 1.0;
/// When there is nothing to photograph, how soon to look again.
const RETRY_S: f32 = 2.0;
/// Frames the subject is aimed at before the shutter opens, so a sphere spawned for it exists
/// and has been placed.
const SETTLE_FRAMES: u32 = 2;
/// Frames between the shutter and showing the picture. Rendering is pipelined: the frame the
/// camera was active on is drawn while the next is being built.
const DEVELOP_FRAMES: u32 = 2;
/// The longest the shutter waits for a sphere's surface to finish baking. A body first resolved
/// for the shot is flat until it has; a bake that never lands must not hold the rotation.
const BAKE_WAIT_S: f32 = 3.0;
const FADE_S: f32 = 0.6;

/// The fewest pixels across a shot is drawn at. A field too small to hold this many of the
/// telescope's resolution elements is widened until it does, which is how a far target comes
/// out as a few soft pixels rather than a sharp disc the optics could never have delivered.
const MIN_SIDE_PX: u32 = 12;
/// Until the interface has said how big the square is.
const DEFAULT_SIDE_PX: u32 = 384;

/// The widest shot. Framing a planet from low orbit asks for nearly a hemisphere, and past this
/// a perspective lens is mostly stretched edge.
const MAX_FIELD_RAD: f64 = 1.75;
/// How much sky around a framed disc, as a multiple of its diameter.
const FRAME_MARGIN: f64 = 1.3;
/// How much of the limb the horizon shot spans, and how much ground the shot below does, in the
/// body's radii. Lengths rather than angles, so a higher orbit is a longer lens on the same
/// scene. Both were an angle once, set from low orbit, and from twenty radii out each one took
/// in the whole disc.
const HORIZON_SPAN_RADII: f64 = 0.228;
const NADIR_SPAN_RADII: f64 = 0.18;
/// How far above the limb the horizon shot looks, as a fraction of its field, so the limb sits
/// in the lower part of the frame with sky over it.
const HORIZON_LIFT: f64 = 0.25;
/// Past this many of its radii from its center a body is not *below*: its horizon and the
/// ground under the ship are both most of the disc again, and the whole-disc shot says it.
const BELOW_RADII: f64 = 30.0;
/// A body within this many of its radii of where the ship is going is what it is going to.
const ARRIVING_RADII: f64 = 50.0;
/// How wide a body's disc has to be on the sky for it to be a neighbor worth a shot of its own.
/// The Moon from Earth is nine milliradians; Venus and Jupiter at their closest are a little
/// over this, and Mars at opposition a little under.
const NEIGHBOR_MIN_RAD: f64 = 1.5e-4;
/// The most of a target's disc that may be hidden behind something nearer. A moonrise is a
/// picture; a moon wholly behind its planet is a black frame.
const MAX_COVERED: f64 = 0.7;
/// The least of a target's face that must be lit. A thin crescent is a picture; a new moon is
/// not.
const MIN_LIT: f64 = 0.3;
/// A star's disc is almost never resolved, so this is the patch of sky around it.
const STAR_FIELD_RAD: f64 = 0.01;

/// Where a shot of the sky puts the star it is metered on, in stops over the top of the window:
/// over, so it is a point with its glare round it rather than a dim dot.
const POINT_ABOVE: f32 = 8.0;
/// How far a shot's exposure may move from the view's, in stops.
const MAX_STOPS: f32 = 24.0;

/// What a photograph is of.
#[derive(Clone, Debug, PartialEq)]
pub enum Subject {
    /// The body the ship is held by, framed whole.
    Whole(String),
    /// Along its limb.
    Horizon(String),
    /// Straight down onto it.
    Nadir(String),
    /// The body the ship is flying to.
    Approach(String),
    /// Where the ship is flying to, when no body is there: across the gap, a star.
    Destination(DVec3),
    /// The body a survey last measured.
    Surveyed(String),
    /// A body big on the sky that none of the above is about, picked at random each time.
    Neighbor(String),
    /// The star a stare or a watch is on.
    Star(StarId),
    /// The sweep's field being exposed now.
    Field { center: DVec3, field_rad: f64, index: u64, of: u64 },
}

impl Subject {
    /// For `--beauty-kind`, which holds the rotation on one kind.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Whole(_) => "whole",
            Self::Horizon(_) => "horizon",
            Self::Nadir(_) => "nadir",
            Self::Approach(_) => "approach",
            Self::Destination(_) => "destination",
            Self::Surveyed(_) => "survey",
            Self::Neighbor(_) => "neighbor",
            Self::Star(_) => "star",
            Self::Field { .. } => "field",
        }
    }
}

/// Where the camera looks and how much it takes in. Directions are unit, simulation axes.
#[derive(Clone, Debug, PartialEq)]
pub struct Shot {
    pub forward: DVec3,
    pub up: DVec3,
    /// Vertical, and the shot is square.
    pub field_rad: f64,
    pub side_px: u32,
    /// A body to draw as a sphere if the shot resolves it.
    pub body: Option<String>,
    /// How much brighter than the view the starfield is exposed. A body the shot resolves is
    /// metered for itself instead; see `resolved::metered_for`.
    pub stops: f32,
    /// The color of a star at the middle of the frame, which is drawn with the diffraction
    /// spikes the secondary mirror's supports throw.
    pub spikes: Option<Vec3>,
    pub caption: String,
}

/// The body the shot being taken wants drawn as a sphere, and the scale it is taken at.
/// Read by `resolved::update_resolved`.
#[derive(Resource, Default)]
pub struct ShotBody {
    pub name: Option<String>,
    pub rad_per_px: f32,
}

/// The telescope's camera.
#[derive(Component)]
pub struct BeautyCamera;

#[derive(Clone, Debug)]
enum Phase {
    Waiting { at_s: f32 },
    Aiming { subject: Subject, frames: u32, since_s: f32 },
    Developing { frames: u32, caption: String, spikes: Option<Vec3>, kind: &'static str },
}

#[derive(Resource)]
pub struct Beauty {
    phase: Phase,
    taken: u64,
    /// Two, so the one on show is never the one being drawn into.
    images: [Handle<Image>; 2],
    textures: [egui::TextureId; 2],
    /// Which image is on show, and since when (real seconds), for the fade.
    front: Option<(usize, f32)>,
    caption: String,
    spikes: Option<Vec3>,
    /// Physical pixels across the square, as the interface last laid it out.
    side_px: u32,
    /// Whether the last look found anything to photograph.
    idle: bool,
}

pub struct BeautyPlugin;

impl Plugin for BeautyPlugin {
    fn build(&self, app: &mut App) {
        use crate::app::{AppState, Stage};
        app.init_resource::<ShotBody>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                shoot
                    .in_set(Stage::Scene)
                    // Which places the eye first; the eye itself is placed in the menu too, and
                    // a system added twice cannot be ordered against.
                    .after(crate::starfield::update_bodies)
                    .before(crate::resolved::update_resolved)
                    .run_if(in_state(AppState::InGame)),
            )
            .add_systems(OnExit(AppState::InGame), put_away);
    }
}

fn target_image(side: u32) -> Image {
    let side = side.max(1);
    // 8-bit sRGB for the reason the map's is: egui samples it and gets back what was drawn.
    let mut image = Image::new_target_texture(side, side, TextureFormat::Rgba8UnormSrgb, None);
    image.asset_usage = bevy::asset::RenderAssetUsages::RENDER_WORLD;
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut textures: ResMut<EguiUserTextures>,
) {
    let images = [images.add(target_image(MIN_SIDE_PX)), images.add(target_image(MIN_SIDE_PX))];
    let textures = images
        .clone()
        .map(|image| textures.add_image(bevy_egui::EguiTextureHandle::Strong(image)));
    commands.spawn((
        Camera3d::default(),
        BeautyCamera,
        RenderLayers::from_layers(&[0, SHOT_LAYER]),
        RenderTarget::Image(images[0].clone().into()),
        Camera {
            order: -3,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            near: crate::app::NEAR_PLANE,
            far: 1.0e9,
            ..default()
        }),
        // The sky's, so a photograph is metered and bloomed the way the view is.
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
        // Read only by the starfield, as a ratio to the sky's. See `drawn_exposure`.
        Exposure::default(),
        Transform::default(),
    ));
    commands.insert_resource(Beauty {
        phase: Phase::Waiting { at_s: 0.0 },
        taken: 0,
        images,
        textures,
        front: None,
        caption: String::new(),
        spikes: None,
        side_px: DEFAULT_SIDE_PX,
        idle: false,
    });
}

fn put_away(
    mut camera: Single<&mut Camera, With<BeautyCamera>>,
    mut shot: ResMut<ShotBody>,
    mut beauty: ResMut<Beauty>,
) {
    camera.is_active = false;
    shot.name = None;
    beauty.phase = Phase::Waiting { at_s: 0.0 };
}

#[allow(clippy::too_many_arguments)]
fn shoot(
    mut commands: Commands,
    time: Res<Time<Real>>,
    ui: Res<Ui>,
    game: Res<Game>,
    bodies: Res<crate::starfield::Bodies>,
    eye: Res<crate::hull::Eye>,
    dev: Res<crate::dev::DevEntry>,
    mut beauty: ResMut<Beauty>,
    mut shot_body: ResMut<ShotBody>,
    surfaces: Res<crate::surfaces::Surfaces>,
    bakes: Res<crate::procedural::Bakes>,
    mut images: ResMut<Assets<Image>>,
    camera: Single<
        (&mut Camera, &mut Transform, &mut Projection, &mut RenderTarget, &mut Exposure),
        With<BeautyCamera>,
    >,
) {
    let (mut camera, mut transform, mut projection, mut target, mut exposure) =
        camera.into_inner();
    let now = time.elapsed_secs();
    // From the ship. Watching from another craft, the eye is somewhere the ship is not.
    if !ui.beauty_shots || eye.anchored.is_some() {
        if camera.is_active {
            camera.is_active = false;
        }
        shot_body.name = None;
        beauty.phase = Phase::Waiting { at_s: now + FIRST_S };
        return;
    }

    if let Phase::Waiting { at_s } = beauty.phase {
        if now < at_s {
            return;
        }
        // Seeded from the clock as well, so two sessions do not visit the neighbors in step.
        let seed = beauty.taken ^ time.elapsed().as_nanos() as u64;
        let mut list = subjects(&game.0, &bodies.drawn, eye.at_ly, seed);
        if let Some(kind) = dev.beauty_kind.as_deref() {
            list.retain(|s| s.kind() == kind);
        }
        beauty.idle = list.is_empty();
        let Some(subject) = list.get((beauty.taken % list.len().max(1) as u64) as usize) else {
            beauty.phase = Phase::Waiting { at_s: now + RETRY_S };
            return;
        };
        beauty.phase =
            Phase::Aiming { subject: subject.clone(), frames: SETTLE_FRAMES, since_s: now };
    }

    match beauty.phase.clone() {
        Phase::Waiting { .. } => {}
        Phase::Aiming { subject, frames, since_s } => {
            let resolution = game.optics().band().map_or(0.0, |b| game.optics().resolution_rad(b));
            let Some(shot) =
                aim(&subject, &game.0, &bodies.drawn, eye.at_ly, resolution, beauty.side_px)
            else {
                shot_body.name = None;
                beauty.phase = Phase::Waiting { at_s: now + RETRY_S };
                return;
            };
            shot_body.name = shot.body.clone();
            shot_body.rad_per_px =
                crate::starfield::radians_per_pixel(shot.field_rad as f32, shot.side_px as f32);
            let back = beauty.front.map_or(0, |(i, _)| 1 - i);
            if images.get(&beauty.images[back]).is_some_and(|i| i.width() != shot.side_px) {
                if let Some(mut image) = images.get_mut(&beauty.images[back]) {
                    *image = target_image(shot.side_px);
                }
            }
            if frames > 0 {
                beauty.phase = Phase::Aiming { subject, frames: frames - 1, since_s };
                return;
            }
            let baked = shot.body.as_deref().is_none_or(|name| surfaces.ready(name, &bakes));
            if !baked && now - since_s < BAKE_WAIT_S {
                return;
            }
            *transform = Transform::default().looking_to(
                sim_to_render(shot.forward).as_vec3(),
                sim_to_render(shot.up).as_vec3(),
            );
            if let Projection::Perspective(lens) = &mut *projection {
                lens.fov = shot.field_rad as f32;
            }
            exposure.ev100 = Exposure::default().ev100 - shot.stops;
            *target = RenderTarget::Image(beauty.images[back].clone().into());
            camera.is_active = true;
            // On the shutter's own frame: a screenshot of an image captures what is drawn into
            // it that frame, and taken later it captured nothing and wrote that back over it.
            if let Some(dir) = dev.beauty_dir.as_deref() {
                let path = format!("{dir}/{:03}-{}.png", beauty.taken, subject.kind());
                info!("beauty shot {path}: {}", shot.caption);
                commands
                    .spawn(bevy::render::view::screenshot::Screenshot::image(
                        beauty.images[back].clone(),
                    ))
                    .observe(bevy::render::view::screenshot::save_to_disk(path));
            }
            beauty.phase = Phase::Developing {
                frames: DEVELOP_FRAMES,
                caption: shot.caption,
                spikes: shot.spikes,
                kind: subject.kind(),
            };
        }
        Phase::Developing { frames, caption, spikes, kind } => {
            if camera.is_active {
                camera.is_active = false;
            }
            if frames > 0 {
                beauty.phase = Phase::Developing { frames: frames - 1, caption, spikes, kind };
                return;
            }
            let back = beauty.front.map_or(0, |(i, _)| 1 - i);
            beauty.front = Some((back, now));
            beauty.caption = caption;
            beauty.spikes = spikes;
            beauty.taken += 1;
            shot_body.name = None;
            beauty.phase =
                Phase::Waiting { at_s: now + dev.beauty_period_s.unwrap_or(PERIOD_S) };
        }
    }
}

/// Everything worth photographing now, in the order the rotation takes them.
///
/// Anything that would come out mostly hidden or mostly dark is left out; see [`MAX_COVERED`]
/// and [`MIN_LIT`].
pub fn subjects(session: &Session, drawn: &[Drawable], eye_ly: DVec3, seed: u64) -> Vec<Subject> {
    let now = session.coordinate_time_s();
    let system = session.system.as_deref();
    let mut out = Vec::new();
    let star = system.and_then(|s| Some((s.star_position_at(now)?, s.star_radius_m())));
    let occluders: Vec<(DVec3, f64)> =
        drawn.iter().map(|d| (d.position_ly, d.radius_m)).chain(star).collect();
    let body_ok = |b: &Drawable| {
        covered(b.position_ly, b.radius_m, eye_ly, &occluders) <= MAX_COVERED
            && star.is_none_or(|(at, _)| lit(b.position_ly, at, eye_ly) >= MIN_LIT)
    };
    let named_ok = |name: &str| drawn.iter().find(|d| d.name == name).is_some_and(body_ok);

    match &session.observatory.duty {
        Duty::Idle => {}
        duty @ (Duty::Stare(_) | Duty::Watch { .. }) => {
            if let Some(star) = duty.target_at(now) {
                out.push(Subject::Star(star));
            }
        }
        Duty::Survey { star, .. } => {
            let observed = session.doing.observing.or(session.observatory.observed());
            let body = match (observed, system) {
                (Some(Known::Body { star: of, body }), Some(system)) if of == system.star => {
                    drawn.iter().find(|d| BodyId::of(of, &d.name) == body)
                }
                _ => None,
            };
            out.push(match body {
                Some(body) => Subject::Surveyed(body.name.clone()),
                None => Subject::Star(*star),
            });
        }
        Duty::Sweep(sweep) => {
            let plan = sweep.plan();
            let index = sweep.field_at(&plan, now);
            if let Some(center) = plan.center_of(index) {
                out.push(Subject::Field {
                    center,
                    field_rad: sweep.field_rad,
                    index,
                    of: plan.fields(),
                });
            }
        }
    }

    let held = system.and_then(|system| {
        let index = system.holding(session.ship.motion.position_ly, now);
        (index != system.primary()).then(|| system.sim().name(index).to_string())
    });

    if let Some(to) = destination(session) {
        let arriving = drawn
            .iter()
            .filter(|d| d.radius_m > 0.0)
            .map(|d| (d, d.position_ly.distance(to) * M_PER_LY / d.radius_m))
            .filter(|(_, radii)| *radii < ARRIVING_RADII)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match arriving {
            Some((body, _)) if held.as_deref() != Some(body.name.as_str()) => {
                out.push(Subject::Approach(body.name.clone()));
            }
            Some(_) => {}
            None => out.push(Subject::Destination(to)),
        }
    }

    if let Some(name) = held.clone()
        && let Some(body) = drawn.iter().find(|d| d.name == name)
    {
        out.push(Subject::Whole(name.clone()));
        if body.position_ly.distance(eye_ly) * M_PER_LY < BELOW_RADII * body.radius_m {
            out.push(Subject::Horizon(name.clone()));
            // Not over the night side, where the ground below is black.
            let day = star.is_none_or(|(at, _)| {
                (eye_ly - body.position_ly).dot(at - body.position_ly) > 0.0
            });
            if day {
                out.push(Subject::Nadir(name.clone()));
            }
        }
    }

    let neighbors: Vec<&Drawable> = drawn
        .iter()
        .filter(|d| d.radius_m > 0.0 && held.as_deref() != Some(d.name.as_str()))
        .filter(|d| {
            let distance = d.position_ly.distance(eye_ly) * M_PER_LY;
            2.0 * (d.radius_m / distance.max(d.radius_m)).asin() > NEIGHBOR_MIN_RAD
        })
        .filter(|d| !out.iter().any(|s| matches!(s,
            Subject::Approach(n) | Subject::Surveyed(n) if *n == d.name)))
        .filter(|d| body_ok(d))
        .collect();
    if !neighbors.is_empty() {
        let pick = lc_world::rng::hash(&[seed, 0xbea0]) % neighbors.len() as u64;
        out.push(Subject::Neighbor(neighbors[pick as usize].name.clone()));
    }

    out.retain(|subject| match subject {
        Subject::Whole(n) | Subject::Approach(n) | Subject::Surveyed(n) => named_ok(n),
        Subject::Star(id) => session.star(*id).is_some_and(|s| {
            covered(s.position_ly, s.star.radius_m, eye_ly, &occluders) <= MAX_COVERED
        }),
        _ => true,
    });
    out
}

/// How much of a disc of `radius_m` at `at_ly` is behind nearer discs, seen from `eye_ly`, as
/// a fraction.
///
/// Flat geometry on angles, which is exact for small discs and good enough at the threshold for
/// a planet filling half the sky. Where two occluders overlap each other their shares are both
/// counted, which errs toward calling the target hidden.
pub fn covered(at_ly: DVec3, radius_m: f64, eye_ly: DVec3, occluders: &[(DVec3, f64)]) -> f64 {
    let to = (at_ly - eye_ly) * M_PER_LY;
    let distance = to.length();
    if distance <= radius_m || radius_m <= 0.0 {
        return 0.0;
    }
    let a = (radius_m / distance).asin();
    let mut hidden = 0.0;
    for &(other_ly, other_m) in occluders {
        let toward = (other_ly - eye_ly) * M_PER_LY;
        let other_distance = toward.length();
        // Itself, and anything not in front of it.
        if other_distance >= distance * (1.0 - 1e-9) || other_m <= 0.0 {
            continue;
        }
        let b = (other_m / other_distance.max(other_m)).asin();
        let apart = to.angle_between(toward);
        hidden += disc_overlap(a, b, apart);
    }
    (hidden / (std::f64::consts::PI * a * a)).min(1.0)
}

/// The area two discs of radii `a` and `b` share, their centers `apart`.
fn disc_overlap(a: f64, b: f64, apart: f64) -> f64 {
    use std::f64::consts::PI;
    if apart >= a + b {
        return 0.0;
    }
    if apart <= (a - b).abs() {
        return PI * a.min(b).powi(2);
    }
    let lens = |r: f64, other: f64| {
        r * r * ((apart * apart + r * r - other * other) / (2.0 * apart * r)).clamp(-1.0, 1.0).acos()
    };
    let kite = (-apart + a + b) * (apart + a - b) * (apart - a + b) * (apart + a + b);
    lens(a, b) + lens(b, a) - 0.5 * kite.max(0.0).sqrt()
}

/// How much of a sphere's face toward `eye_ly` is lit by a star at `star_ly`: one full, zero new.
pub fn lit(at_ly: DVec3, star_ly: DVec3, eye_ly: DVec3) -> f64 {
    let phase = (star_ly - at_ly).angle_between(eye_ly - at_ly);
    if phase.is_nan() {
        return 1.0;
    }
    0.5 * (1.0 + phase.cos())
}

/// Where the ship is flying to, light-years from the world origin.
fn destination(session: &Session) -> Option<DVec3> {
    match &session.ship.motion.motive {
        Motive::Crossing(cruise) => Some(cruise.to_ly),
        // A transfer's plan is in the frame of the body it is flown about.
        Motive::Transfer(transfer) => {
            let system = session.system.as_deref()?;
            let (origin, _) = transfer.frame_at(system, session.coordinate_time_s())?;
            Some(origin + transfer.cruise.to_ly)
        }
        _ => None,
    }
}

/// The camera for a subject, or `None` if it has gone.
pub fn aim(
    subject: &Subject,
    session: &Session,
    drawn: &[Drawable],
    eye_ly: DVec3,
    resolution_rad: f64,
    max_side_px: u32,
) -> Option<Shot> {
    let body = |name: &str| drawn.iter().find(|d| d.name == name);
    let to_m = |at_ly: DVec3| (at_ly - eye_ly) * M_PER_LY;
    let toward_star = session
        .system
        .as_deref()
        .and_then(|s| s.star_position_at(session.coordinate_time_s()))
        .map(|star| (star - eye_ly).normalize_or_zero())
        .unwrap_or(DVec3::X);

    let (forward, up, field_rad, sphere, caption) = match subject {
        Subject::Whole(name)
        | Subject::Approach(name)
        | Subject::Surveyed(name)
        | Subject::Neighbor(name) => {
            let b = body(name)?;
            let (forward, field) = framed(to_m(b.position_ly), b.radius_m);
            let label = session.body_label(name);
            let caption = match subject {
                Subject::Approach(_) => format!("{label} · ahead"),
                Subject::Surveyed(_) => format!("{label} · survey"),
                _ => label,
            };
            (forward, upright(forward, b.pole), field, Some(name.clone()), caption)
        }
        Subject::Horizon(name) => {
            let b = body(name)?;
            let (forward, up, field) = horizon(to_m(b.position_ly), b.radius_m, toward_star)?;
            let caption = format!("{} · horizon", session.body_label(name));
            (forward, up, field, Some(name.clone()), caption)
        }
        Subject::Nadir(name) => {
            let b = body(name)?;
            let to = to_m(b.position_ly);
            let forward = to.try_normalize()?;
            let field = nadir_field(to.length(), b.radius_m)?;
            let caption = format!("{} · below", session.body_label(name));
            (forward, upright(forward, b.pole), field, Some(name.clone()), caption)
        }
        Subject::Destination(to_ly) => {
            let forward = (*to_ly - eye_ly).try_normalize()?;
            (forward, upright(forward, DVec3::Z), STAR_FIELD_RAD, None, "ahead".into())
        }
        Subject::Star(id) => {
            let star = session.star(*id)?;
            let forward = session.apparent_dir(star).try_normalize()?;
            let distance_m = session.distance_to(star) * M_PER_LY;
            let disc = (star.star.radius_m / distance_m.max(1.0)).min(1.0).asin();
            let field = (2.0 * disc * FRAME_MARGIN).max(STAR_FIELD_RAD);
            (forward, upright(forward, DVec3::Z), field, None, session.name_of(*id))
        }
        Subject::Field { center, field_rad, index, of } => {
            let forward = center.try_normalize()?;
            let caption = format!("field {} of {of}", index + 1);
            (forward, upright(forward, DVec3::Z), *field_rad, None, caption)
        }
    };
    let (field_rad, side_px) = fidelity(field_rad, resolution_rad, max_side_px);
    let metered = match subject {
        Subject::Star(id) => session.star(*id).and_then(|star| shaded(session, star)),
        Subject::Destination(_) | Subject::Field { .. } => brightest(session, forward, field_rad),
        _ => None,
    };
    let stops = metered
        .as_ref()
        .map_or(0.0, |star| (POINT_ABOVE - star.stops).clamp(-MAX_STOPS, MAX_STOPS));
    let spikes = matches!(subject, Subject::Star(_)).then(|| metered.map(|s| s.chroma)).flatten();
    Some(Shot { forward, up, field_rad, side_px, body: sphere, stops, spikes, caption })
}

/// A star as the sky's window shades it. `None` for one that sends nothing in the bands on
/// show, which a default `Shaded` cannot be told apart from.
fn shaded(session: &Session, star: &lc_world::sky::CatalogStar) -> Option<crate::tonemap::Shaded> {
    let shaded = session.tone.shade(&session.radiance_from(star), &session.mapping);
    (shaded.stops != 0.0 || shaded.value > 0.0).then_some(shaded)
}

/// The brightest star in a square field, if there is one in it.
fn brightest(session: &Session, forward: DVec3, field_rad: f64) -> Option<crate::tonemap::Shaded> {
    // The square's corners are this far out: half its diagonal.
    let reach = (field_rad * std::f64::consts::FRAC_1_SQRT_2).cos();
    session
        .stars
        .iter()
        .filter(|s| session.apparent_dir(s).dot(forward) > reach)
        .filter_map(|s| shaded(session, s))
        .max_by(|a, b| a.stops.total_cmp(&b.stops))
}

/// The field actually taken and the pixels across it, for the optics' resolution.
///
/// The field is widened to hold [`MIN_SIDE_PX`] resolution elements, and the pixels are as many
/// as the optics can fill, up to what the square shows. A distant planet therefore comes out as
/// a few soft pixels, and sharpens as the ship closes.
pub fn fidelity(field_rad: f64, resolution_rad: f64, max_side_px: u32) -> (f64, u32) {
    let max_side_px = max_side_px.max(MIN_SIDE_PX);
    if !(resolution_rad > 0.0) {
        return (field_rad.min(MAX_FIELD_RAD), max_side_px);
    }
    let field = field_rad.max(MIN_SIDE_PX as f64 * resolution_rad).min(MAX_FIELD_RAD);
    let side = (field / resolution_rad).ceil().min(max_side_px as f64) as u32;
    (field, side.max(MIN_SIDE_PX))
}

/// Look at a sphere `to_m` away and fit it in the frame, with [`FRAME_MARGIN`] around it.
pub fn framed(to_m: DVec3, radius_m: f64) -> (DVec3, f64) {
    let distance = to_m.length();
    let forward = to_m.normalize_or(DVec3::X);
    let half = (radius_m / distance.max(f64::MIN_POSITIVE)).min(1.0).asin();
    (forward, (2.0 * half * FRAME_MARGIN).min(MAX_FIELD_RAD))
}

/// The field that spans [`NADIR_SPAN_RADII`] of ground straight down. `None` from inside.
pub fn nadir_field(distance_m: f64, radius_m: f64) -> Option<f64> {
    let altitude = distance_m - radius_m;
    (altitude > 0.0)
        .then(|| (2.0 * (NADIR_SPAN_RADII * radius_m / (2.0 * altitude)).atan()).min(MAX_FIELD_RAD))
}

/// The field that spans [`HORIZON_SPAN_RADII`] of limb, seen from `distance_m` off the center.
/// The limb is the tangent point, so it is this far away. `None` from inside.
pub fn horizon_field(distance_m: f64, radius_m: f64) -> Option<f64> {
    let to_limb = (distance_m * distance_m - radius_m * radius_m).sqrt();
    (to_limb > 0.0).then(|| (HORIZON_SPAN_RADII * radius_m / to_limb).min(MAX_FIELD_RAD))
}

/// Along the limb of a sphere whose center is `to_center_m` away, on the side toward the star,
/// with the planet below and the sky above: the view, its up, and its field. `None` from inside.
pub fn horizon(to_center_m: DVec3, radius_m: f64, toward_star: DVec3)
    -> Option<(DVec3, DVec3, f64)> {
    let distance = to_center_m.length();
    let field = horizon_field(distance, radius_m)?;
    let down = to_center_m / distance;
    // The limb is this far off straight down.
    let dip = (radius_m / distance).asin();
    let across = (toward_star - down * toward_star.dot(down))
        .try_normalize()
        .unwrap_or_else(|| down.any_orthonormal_vector());
    let limb = down * dip.cos() + across * dip.sin();
    // Up is away from the center, square to the line of sight.
    let up_at = |f: DVec3| (-down - f * (-down).dot(f)).normalize_or(across);
    let lift = field * HORIZON_LIFT;
    let forward = (limb * lift.cos() + up_at(limb) * lift.sin()).normalize();
    Some((forward, up_at(forward), field))
}

/// An up for `forward`, as close to `pole` as it can be, falling back to anything square to it.
fn upright(forward: DVec3, pole: DVec3) -> DVec3 {
    (pole - forward * pole.dot(forward))
        .try_normalize()
        .unwrap_or_else(|| forward.any_orthonormal_vector())
}

/// The square beside the map's, in the same row and the same size.
fn square(viewport: egui::Rect) -> egui::Rect {
    let corner = crate::map_panel::corner(viewport);
    let rect = corner.translate(egui::vec2(corner.width() + crate::map_panel::CORNER_INSET, 0.0));
    rect.intersect(viewport.shrink(crate::map_panel::CORNER_INSET))
}

/// Show the latest photograph, faded in over the one before it.
pub fn draw(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    time: Res<Time<Real>>,
    mut beauty: ResMut<Beauty>,
) {
    if !ui_state.beauty_shots {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let rect = square(ctx.viewport_rect());
    if !rect.is_positive() {
        return;
    }
    beauty.side_px = (rect.height() * ctx.pixels_per_point()).round().max(1.0) as u32;
    let now = time.elapsed_secs();
    let text = crate::map_panel::color_of(em_ui::vfd::TEXT);
    let dim = crate::map_panel::color_of(em_ui::vfd::TEXT_DIM);
    let caption = match (&beauty.front, beauty.idle) {
        (None, true) => "nothing to photograph".to_string(),
        (None, false) => "focusing".to_string(),
        (Some(_), _) => beauty.caption.clone(),
    };
    egui::Area::new(egui::Id::new("beauty shot"))
        // Middle, as the map's square is: an open window covers it.
        .order(egui::Order::Middle)
        .fixed_pos(rect.min)
        .show(ctx, |ui| {
            ui.allocate_rect(rect, egui::Sense::hover());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::BLACK);
            let whole = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            if let Some((front, since)) = beauty.front {
                let t = ((now - since) / FADE_S).clamp(0.0, 1.0);
                if t < 1.0 && beauty.taken > 1 {
                    painter.image(beauty.textures[1 - front], rect, whole, egui::Color32::WHITE);
                }
                painter.image(beauty.textures[front], rect, whole,
                    egui::Color32::WHITE.gamma_multiply(t));
                if let Some(chroma) = beauty.spikes {
                    spikes(painter, rect, chroma, t);
                }
            }
            let font = egui::TextStyle::Small.resolve(ui.style());
            let galley = painter.layout_no_wrap(caption, font, text);
            let at = rect.left_bottom() + egui::vec2(4.0, -4.0 - galley.size().y);
            painter.rect_filled(
                egui::Rect::from_min_size(at, galley.size()).expand(2.0),
                0.0,
                egui::Color32::from_black_alpha(160),
            );
            painter.galley(at, galley, text);
            painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, dim), egui::StrokeKind::Inside);
        });
    // Repaint through the fade; nothing else asks for a frame while the ship is still.
    if beauty.front.is_some_and(|(_, since)| now - since < FADE_S) {
        ctx.request_repaint();
    }
}

/// How far a diffraction spike reaches, as a fraction of the square's side, and how wide it is
/// at the star, in points. The glow round the star is in points too.
const SPIKE_REACH: f32 = 0.45;
const SPIKE_WIDTH: f32 = 1.2;
const GLOW_RADIUS: f32 = 5.0;

/// Four spikes and a glow at the middle of the frame, each fading to nothing. Drawn over the
/// photograph rather than in it: the starfield's glare is round by design, and the spikes are a
/// property of this instrument, not of how the sky is drawn.
fn spikes(painter: &egui::Painter, rect: egui::Rect, chroma: Vec3, alpha: f32) {
    let c = chroma.clamp(Vec3::ZERO, Vec3::ONE);
    let color = |a: f32| {
        egui::Color32::from_rgb((c.x * 255.0) as u8, (c.y * 255.0) as u8, (c.z * 255.0) as u8)
            .gamma_multiply(a * alpha)
    };
    let middle = rect.center();
    let reach = rect.width() * SPIKE_REACH;
    let mut mesh = egui::Mesh::default();
    for k in 0..4 {
        let angle = k as f32 * std::f32::consts::FRAC_PI_2;
        let along = egui::vec2(angle.cos(), angle.sin());
        let across = egui::vec2(-along.y, along.x) * SPIKE_WIDTH * 0.5;
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(middle + across, color(0.7));
        mesh.colored_vertex(middle - across, color(0.7));
        mesh.colored_vertex(middle + along * reach, color(0.0));
        mesh.add_triangle(base, base + 1, base + 2);
    }
    let center = mesh.vertices.len() as u32;
    mesh.colored_vertex(middle, color(0.9));
    const SEGMENTS: u32 = 16;
    for k in 0..SEGMENTS {
        let angle = k as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        mesh.colored_vertex(middle + egui::vec2(angle.cos(), angle.sin()) * GLOW_RADIUS, color(0.0));
        mesh.add_triangle(center, center + 1 + k, center + 1 + (k + 1) % SEGMENTS);
    }
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;

    const EARTH_M: f64 = 6.371e6;

    fn angle(a: DVec3, b: DVec3) -> f64 {
        a.normalize().dot(b.normalize()).clamp(-1.0, 1.0).acos()
    }

    #[test]
    fn a_framed_disc_fills_the_frame_less_its_margin() {
        let (forward, field) = framed(DVec3::new(0.0, 4.0e8, 0.0), EARTH_M);
        assert_eq!(forward, DVec3::Y);
        let disc = 2.0 * (EARTH_M / 4.0e8).asin();
        assert!((field / disc - FRAME_MARGIN).abs() < 1e-9);
    }

    #[test]
    fn framing_from_low_orbit_stops_at_the_widest_lens() {
        let (_, field) = framed(DVec3::new(EARTH_M + 4.0e5, 0.0, 0.0), EARTH_M);
        assert_eq!(field, MAX_FIELD_RAD);
    }

    /// The horizon shot looks just over the limb: the line to the limb is square to the radius
    /// there, and the view is lifted off it by a fixed part of the field.
    #[test]
    fn the_horizon_is_just_below_the_middle_of_the_frame() {
        let center = DVec3::new(0.0, 0.0, -(EARTH_M + 4.0e5));
        let star = DVec3::X;
        let (forward, up, field) = horizon(center, EARTH_M, star).expect("outside the planet");
        assert!(forward.dot(up).abs() < 1e-12, "up is square to the view");
        assert!(up.z > 0.0, "up is away from the planet");
        assert!(forward.x > 0.0, "and it faces the lit side");

        let dip = (EARTH_M / center.length()).asin();
        let limb_off_down = angle(forward, center);
        let lifted = field * HORIZON_LIFT;
        assert!((limb_off_down - dip - lifted).abs() < 1e-9, "{limb_off_down} vs {dip}");
    }

    /// Further out is a longer lens on the same scene: the ground below and the stretch of limb
    /// in frame are the same size from every orbit.
    #[test]
    fn a_higher_orbit_is_a_longer_lens_on_the_same_scene() {
        for radii in [1.15, 5.0, 20.0] {
            let distance = radii * EARTH_M;
            let below = nadir_field(distance, EARTH_M).unwrap();
            let ground = 2.0 * (distance - EARTH_M) * (below / 2.0).tan();
            assert!((ground / EARTH_M - NADIR_SPAN_RADII).abs() < 1e-9, "{ground} m at {radii}");

            let along = horizon_field(distance, EARTH_M).unwrap();
            let limb = along * (distance * distance - EARTH_M * EARTH_M).sqrt();
            assert!((limb / EARTH_M - HORIZON_SPAN_RADII).abs() < 1e-9, "{limb} m at {radii}");
        }
        assert!(nadir_field(EARTH_M * 20.0, EARTH_M) < nadir_field(EARTH_M * 5.0, EARTH_M));
    }

    /// Lower still, the ground below would want a lens wider than any shot takes, and gets the
    /// widest instead: less ground, not a stretched picture.
    #[test]
    fn skimming_the_surface_holds_the_widest_lens() {
        assert_eq!(nadir_field(EARTH_M * 1.02, EARTH_M), Some(MAX_FIELD_RAD));
    }

    const MOON_M: f64 = 1.737e6;
    const LY: f64 = 1.0 / M_PER_LY;

    /// How much of the Moon Earth hides, with Earth between them on the sky.
    fn moon_behind_earth(apart_moon_radii: f64) -> f64 {
        let eye = DVec3::ZERO;
        let earth = DVec3::new(0.0, 4.0e7 * LY, 0.0);
        let moon_distance = 4.0e8;
        let moon_rad = MOON_M / moon_distance;
        let earth_rad = (EARTH_M / 4.0e7_f64).asin();
        // The Moon's center this many of its radii outside Earth's limb.
        let angle = earth_rad + apart_moon_radii * moon_rad;
        let moon = DVec3::new(angle.sin(), angle.cos(), 0.0) * moon_distance * LY;
        covered(moon, MOON_M, eye, &[(earth, EARTH_M), (moon, MOON_M)])
    }

    /// A moonrise is kept and a moon behind the planet is not: the cut is at how much is hidden.
    #[test]
    fn a_moon_rising_is_kept_and_one_behind_the_planet_is_not() {
        assert_eq!(moon_behind_earth(3.0), 0.0, "clear of the limb");
        assert!(moon_behind_earth(-3.0) > 0.999, "wholly behind");
        let half = moon_behind_earth(0.0);
        assert!(half > 0.4 && half < 0.6, "straddling the limb, {half} hidden");
        assert!(moon_behind_earth(0.0) <= MAX_COVERED, "a moonrise passes");
        assert!(moon_behind_earth(-0.9) > MAX_COVERED, "a sliver does not");
    }

    #[test]
    fn nothing_behind_the_target_covers_it() {
        let near = DVec3::new(0.0, 1.0e8 * LY, 0.0);
        let far = DVec3::new(0.0, 1.0e9 * LY, 0.0);
        assert_eq!(covered(near, MOON_M, DVec3::ZERO, &[(far, EARTH_M * 100.0)]), 0.0);
    }

    #[test]
    fn full_is_lit_and_new_is_dark() {
        let at = DVec3::new(0.0, 1.0, 0.0);
        assert!((lit(at, DVec3::new(0.0, 2.0, 0.0), DVec3::ZERO) - 0.0).abs() < 1e-12, "new");
        assert!((lit(at, DVec3::ZERO, DVec3::ZERO * 0.5 + DVec3::new(0.0, -1.0, 0.0)) - 1.0).abs()
            < 1e-12, "full");
        assert!((lit(at, DVec3::new(1.0, 1.0, 0.0), DVec3::ZERO) - 0.5).abs() < 1e-12, "quarter");
    }

    #[test]
    fn there_is_no_horizon_from_inside() {
        assert!(horizon(DVec3::new(0.0, 0.0, -1.0), EARTH_M, DVec3::X).is_none());
    }

    /// The whole of what distance does to a picture: a field the optics cannot fill is widened
    /// and drawn at fewer pixels, so a far target is a soft blob rather than a sharp disc.
    #[test]
    fn a_far_target_is_drawn_at_what_the_optics_resolve() {
        let resolution = 6.0e-7;
        let (field, side) = fidelity(1.0e-3, resolution, 400);
        assert_eq!((field, side), (1.0e-3, 400), "near: the square is the limit");

        let (field, side) = fidelity(4.0e-6, resolution, 400);
        assert_eq!(side, MIN_SIDE_PX);
        assert!((field - MIN_SIDE_PX as f64 * resolution).abs() < 1e-15, "widened to {field}");

        let (_, middle) = fidelity(1.0e-4, resolution, 400);
        assert!(middle > MIN_SIDE_PX && middle < 400, "{middle} pixels between the two");
    }

    #[test]
    fn up_is_square_to_the_view_whatever_the_pole() {
        for pole in [DVec3::Z, DVec3::X, DVec3::new(0.3, 0.2, 0.9).normalize()] {
            let forward = DVec3::new(0.0, 0.0, 1.0);
            let up = upright(forward, pole);
            assert!(up.dot(forward).abs() < 1e-12);
            assert!((up.length() - 1.0).abs() < 1e-12);
        }
    }
}
