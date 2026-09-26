//! The editor's view: the ship on a turntable, the third mode of the main view.
//!
//! Its own camera on [`FORM_LAYER`], drawing copies of the placeholder parts in the ship's frame
//! (x nose, y port, z up), in meters about the render origin, into an image laid out under the
//! editor's chrome with the sky in the corner square. Copies rather than [`crate::parts`]' own
//! pieces: those are placed relative to the eye in astronomical units and lit by the star, and
//! an entity has one transform and one material. An image rather than the window, because two
//! cameras sharing the window's texture clear and tone-map over each other; the map does the same.
//!
//! The chrome is Bevy UI in [`em_ui`]'s widgets, under egui's windows. See
//! `lightcone/docs/29-ship-form.md` §The editor for the controls and why.

use bevy::camera::RenderTarget;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::MouseButton;
use bevy::prelude::*;
use bevy::render::render_resource::Extent3d;
use bevy::window::PrimaryWindow;
use bevy_egui::egui;
use bevy_egui::input::EguiWantsInput;
use em_render::body_surface_material::{BodySurfaceMaterial, BodySurfaceUniform};
use em_ui::{Edge, MenuTheme, MenuUi};
use glam::{DVec2, DVec3};
use lc_world::fitting::Balance;
use lc_world::form::Form;
use lc_world::form::sdf::Sdf;

use crate::action::Action;
use crate::app::Ui;
use crate::input::Requested;
use crate::ui::ViewMode;

/// The editor's own layer. The sky has 0 and [`crate::app::SKY_ONLY_LAYER`], the map 1, the haze 2
/// and the beauty shots 4.
pub const FORM_LAYER: usize = 5;

/// The editor camera's vertical field of view.
pub const FORM_FOV: f32 = std::f32::consts::FRAC_PI_4;

/// The camera the editor is drawn for.
#[derive(Component)]
pub struct FormCamera;

/// Where the camera starts: aft of the starboard beam, a little above, with the nose to the right.
const START_AZIMUTH: f64 = -120.0 * std::f64::consts::PI / 180.0;
const START_ELEVATION: f64 = 25.0 * std::f64::consts::PI / 180.0;
/// Far enough back that the whole form is in view. See [`Extent::size_m`] for the unit.
const START_DISTANCE: f64 = 1.5;

/// How far the bounds are grown, about their middle, before the eye is kept outside them. Every
/// part is inside the bounds, so an eye outside them is outside every part.
const NEAR_MARGIN: f64 = 1.15;
/// The farthest, in form sizes: the whole form a small thing in the middle of the view.
const FAR_SIZES: f64 = 4.0;

/// Stand-offs of slide per notch of Shift+wheel, and per second of a held key.
pub const SLIDE_PER_NOTCH: f64 = 0.25;
pub const SLIDE_PER_SECOND: f64 = 1.0;

/// How foreshortened the nose axis may be before a drag along it stops speeding up. Looking
/// nose-on the axis is a point on screen and a drag has nothing to slide along.
const MIN_FORESHORTENING: f32 = 0.25;

/// A key light over the camera's shoulder and a floor under it. See
/// `lightcone/docs/29-ship-form.md` §How it is drawn for why the star is not the light here.
const KEY_UP: f64 = 0.8;
const KEY_ACROSS: f64 = -0.5;
const HANGAR_LIGHT: f32 = 1.0;
const HANGAR_FILL: f32 = 0.35;
/// The tone map's window, wider than a surface's in the sky: eight stops keep the darkest paint's
/// unlit side off the floor, where five turned the engine's underside black.
const HANGAR_REFERENCE: f32 = 5.0;
const HANGAR_STOPS: f32 = 8.0;
const BACKDROP: Color = Color::srgb(0.018, 0.026, 0.024);

/// What the target starts at, before the first layout.
const INITIAL_SIDE: u32 = 512;
const MIN_SIDE: u32 = 64;
const MAX_SIDE: u32 = 8192;

/// Below the readout's strips, clear of them.
const CHROME_GAP: f32 = 6.0;

/// The editor's state in the interface.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormView {
    pub orbit: FormOrbit,
    /// Where `H` and `Escape` go back to. Never [`ViewMode::Form`].
    pub from: ViewMode,
    /// Started from the ship's own form the first time the editor opens, and kept across leaving
    /// it. `None` until then.
    pub draft: Option<crate::draft::Draft>,
    /// The part the handles and the fields are on.
    pub selected: Option<lc_world::form::PartId>,
    /// Which of [`crate::draft::PRIMITIVES`] a part taken from the list is made as.
    pub new_shape: usize,
    /// Whether the parts the draft removes are drawn. They are hidden otherwise, and never hung
    /// from either way.
    pub show_dismantled: bool,
}

/// The editor's camera: an orbit about a focus on the ship's nose axis.
///
/// Scale-free: distances are in form sizes, so a 500 m ship and a 50 km one open framed alike
/// and a zoom means the same on both. Clamped against the form every frame by [`place`], as the
/// boom is by `hull::place_eye`, because only the form knows its own extent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FormOrbit {
    /// About the ship's z, from the nose toward port, radians.
    pub azimuth: f64,
    /// Above the ship's x–y plane, radians.
    pub elevation: f64,
    /// From the focus to the eye.
    pub distance: f64,
    /// The focus, along the nose axis from the middle of the form.
    pub along: f64,
}

impl Default for FormOrbit {
    fn default() -> Self {
        Self { azimuth: START_AZIMUTH, elevation: START_ELEVATION, distance: START_DISTANCE, along: 0.0 }
    }
}

impl FormOrbit {
    pub fn turn(&mut self, d_azimuth: f64, d_elevation: f64) {
        let limit = crate::ui::Look::PITCH_LIMIT;
        self.azimuth = (self.azimuth + d_azimuth).rem_euclid(std::f64::consts::TAU);
        self.elevation = (self.elevation + d_elevation).clamp(-limit, limit);
    }

    /// Positive is closer.
    pub fn zoom(&mut self, notches: f64) {
        self.distance = (self.distance * crate::hull::ZOOM_STEP.powf(-notches)).clamp(f64::MIN_POSITIVE, 1.0e9);
    }

    pub fn slide(&mut self, standoffs: f64) {
        self.along += standoffs * self.distance;
    }

    /// This orbit, held inside what `extent` allows.
    ///
    /// The near stop is along the line of sight: where the ray from the focus toward the eye
    /// leaves the grown bounds. A stop by distance alone put the eye on the nose axis, inside a
    /// long hull, whenever the camera looked nose-on.
    pub fn held_to(mut self, extent: &Extent) -> Self {
        let (aft, fore) = extent.along();
        self.along = self.along.clamp(aft, fore);
        let near = extent.exit_m(self.focus_m(extent), self.toward_eye()) / extent.size_m();
        self.distance = self.distance.clamp(near.min(FAR_SIZES), FAR_SIZES);
        self
    }

    /// What the camera looks at, ship frame, meters.
    pub fn focus_m(&self, extent: &Extent) -> DVec3 {
        extent.middle() + DVec3::X * self.along * extent.size_m()
    }

    pub fn eye_m(&self, extent: &Extent) -> DVec3 {
        self.focus_m(extent) + self.toward_eye() * self.distance * extent.size_m()
    }

    fn toward_eye(&self) -> DVec3 {
        let (se, ce) = self.elevation.sin_cos();
        let (sa, ca) = self.azimuth.sin_cos();
        DVec3::new(ce * ca, ce * sa, se)
    }

    /// Forward, right and up of the view, ship frame. The pitch limit keeps forward off z.
    pub fn basis(&self) -> [DVec3; 3] {
        let forward = -self.toward_eye();
        let right = forward.cross(DVec3::Z).normalize();
        [forward, right, right.cross(forward)]
    }

    /// The ray through `ndc` (`[-1, 1]` across the view, `+y` up), from the eye, ship frame.
    pub fn ray(&self, extent: &Extent, fov_y: f64, aspect: f64, ndc: DVec2) -> (DVec3, DVec3) {
        let [forward, right, up] = self.basis();
        let half = (fov_y * 0.5).tan();
        let direction = forward + right * ndc.x * half * aspect + up * ndc.y * half;
        (self.eye_m(extent), direction.normalize())
    }

    /// The nose axis as it lies on screen: a direction in pixels, `+y` down, as long as the axis
    /// is foreshortened.
    pub fn nose_on_screen(&self) -> Vec2 {
        let [_, right, up] = self.basis();
        Vec2::new(DVec3::X.dot(right) as f32, -DVec3::X.dot(up) as f32)
    }
}

/// A form's bounds in the ship's frame, meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub min: DVec3,
    pub max: DVec3,
}

impl Extent {
    /// The unit the orbit is measured in: the diagonal of the bounds, which for a long ship is
    /// about its length.
    pub fn size_m(&self) -> f64 {
        (self.max - self.min).length().max(f64::MIN_POSITIVE)
    }

    pub fn middle(&self) -> DVec3 {
        (self.min + self.max) * 0.5
    }

    /// How far a ray from `from`, inside the bounds, runs along unit `direction` before it
    /// leaves them grown by [`NEAR_MARGIN`], meters.
    pub fn exit_m(&self, from: DVec3, direction: DVec3) -> f64 {
        let half = (self.max - self.min) * 0.5 * NEAR_MARGIN;
        let (low, high) = (self.middle() - half, self.middle() + half);
        (0..3)
            .filter(|&i| direction[i].abs() > f64::EPSILON)
            .map(|i| {
                let wall = if direction[i] > 0.0 { high[i] } else { low[i] };
                (wall - from[i]) / direction[i]
            })
            .fold(f64::INFINITY, f64::min)
            .max(0.0)
    }

    /// How far aft and fore the focus may slide, in form sizes: stem to stern and no further.
    pub fn along(&self) -> (f64, f64) {
        let size = self.size_m();
        ((self.min.x - self.middle().x) / size, (self.max.x - self.middle().x) / size)
    }
}

/// What a drag does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormDrag {
    /// The camera about its focus: the look button, as it turns the sky and the map.
    Orbit,
    /// The focus fore and aft, as the left button pans the map.
    Slide,
}

/// What a press over the editor's view starts, by button and by whether it landed on a part.
///
/// **A left-press on a part is not the camera's.** It selects the part or adds to it
/// ([`crate::form_handles`]), so it starts nothing here; only one on empty space slides.
pub fn drag_of(button: MouseButton, on_part: bool) -> Option<FormDrag> {
    match button {
        crate::input::LOOK_BUTTON => Some(FormDrag::Orbit),
        MouseButton::Left if on_part => None,
        MouseButton::Left => Some(FormDrag::Slide),
        _ => None,
    }
}

/// A look-button turn as a turn of the orbit.
///
/// The camera turns as a look would: left turns the view left, and up tilts it up, which is the
/// camera sinking under the ship. The map's `TurnMap` has the same signs.
pub fn orbit_from(yaw: f64, pitch: f64) -> Action {
    Action::OrbitForm { azimuth: yaw, elevation: -pitch }
}

/// A left-drag of `delta` pixels as a slide, in stand-offs.
///
/// The point under the cursor follows it, as a map follows a pan: drag the ship toward its nose
/// and the focus moves aft. Only the part of the drag along the axis on screen counts, and the
/// stand-off is what makes that the same on screen at every zoom.
pub fn slide_of(delta: Vec2, nose: Vec2, fov_y: f32, height_px: f32) -> f64 {
    let shortening = nose.length();
    let along = match shortening > f32::EPSILON {
        true => delta.dot(nose / shortening),
        // Nose-on, with no axis on screen: up the screen is forward.
        false => -delta.y,
    };
    let per_px = 2.0 * (fov_y * 0.5).tan() / height_px.max(1.0);
    -(along / shortening.max(MIN_FORESHORTENING) * per_px) as f64
}

/// Whether the ray from `origin` along unit `direction` meets the form before `limit_m`.
pub fn on_part(sdf: &Sdf, origin: DVec3, direction: DVec3, limit_m: f64) -> bool {
    const STEPS: usize = 256;
    let (min, max) = sdf.bounds();
    let tolerance = (max - min).length() * 1.0e-4;
    let mut scratch = Vec::new();
    let mut t = 0.0;
    for _ in 0..STEPS {
        let d = sdf.distance_with(origin + direction * t, &mut scratch);
        if d < tolerance {
            return true;
        }
        t += d;
        if t > limit_m {
            return false;
        }
    }
    false
}

/// Whether the parts the draft removes are drawn this frame: always while the toggle is on, and
/// while the pointer is on it.
#[derive(Resource, Default)]
pub struct Revealed(pub bool);

/// A ghost of a part the draft removes.
#[derive(Component)]
struct Dismantled;

/// The draft as drawn, and the ship it is drawn over.
#[derive(Resource, Default)]
pub struct Shown {
    /// The draft's field, and the bounds of it and the ship together, which the camera frames.
    drawn: Option<(Sdf, Extent)>,
    ghost: Option<Sdf>,
    marks: std::collections::BTreeMap<lc_world::form::PartId, crate::draft::Mark>,
    /// The draft and the ship last drawn, so a frame that changed neither draws nothing. A
    /// failure to solve is kept too, so it is not retried every frame.
    of: Option<(Form, Form)>,
}

impl Shown {
    pub fn extent(&self) -> Option<Extent> {
        self.drawn.as_ref().map(|(_, extent)| *extent)
    }

    pub fn sdf(&self) -> Option<&Sdf> {
        self.drawn.as_ref().map(|(sdf, _)| sdf)
    }

    /// The ship as it is, where the draft differs from it.
    pub fn ghost(&self) -> Option<&Sdf> {
        self.ghost.as_ref()
    }

    pub fn marks(&self) -> &std::collections::BTreeMap<lc_world::form::PartId, crate::draft::Mark> {
        &self.marks
    }
}

/// Where the picture is on the window, logical pixels, while the editor is the view.
pub fn picture(surface: &FormSurface) -> Option<egui::Rect> {
    surface.laid.map(|(rect, _)| rect)
}

/// Whether `at`, logical pixels, is on the picture rather than the corner square or the readout.
pub fn on_picture(surface: &FormSurface, at: Vec2) -> bool {
    surface.laid.is_some_and(|(rect, hole)| inside(rect, hole, at))
}

/// Where the picture goes on the window, logical pixels, and the target it is drawn into.
#[derive(Resource)]
pub struct FormSurface {
    image: Handle<Image>,
    size: UVec2,
    /// The picture's rect and the corner square left out of it, or `None` outside the editor.
    laid: Option<(egui::Rect, egui::Rect)>,
}

/// Where the picture goes on a window of `window` logical pixels whose readout ends at `foot`:
/// everything under the readout, less the corner square.
pub fn layout_of(window: Vec2, foot: f32) -> (egui::Rect, egui::Rect) {
    let whole = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(window.x, window.y));
    let mut rect = whole;
    rect.min.y = foot.clamp(whole.min.y, whole.max.y);
    (rect, crate::map_panel::corner(whole))
}

pub struct FormViewPlugin;

impl Plugin for FormViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shown>()
            .init_resource::<Revealed>()
            .add_systems(
                Update,
                (start_draft, show, reveal, lay_out, place, relight)
                    .chain()
                    .in_set(crate::app::Stage::Scene)
                    .after(crate::app::Placed)
                    .run_if(in_state(crate::app::AppState::InGame)),
            )
            .add_systems(OnExit(crate::app::AppState::InGame), put_away);
    }
}

/// Whether the editor is the view. Its input systems run in `app`'s input chain, before the
/// dispatcher, under this.
pub fn editing(ui: Res<Ui>) -> bool {
    ui.view == ViewMode::Form
}

/// The editor's camera. Spawned after the others, so it is never the first camera created.
pub fn spawn_camera(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(crate::map::target_image(UVec2::splat(INITIAL_SIDE)));
    commands.insert_resource(FormSurface { image: image.clone(), size: UVec2::splat(INITIAL_SIDE), laid: None });
    commands.spawn((
        Camera3d::default(),
        FormCamera,
        RenderLayers::layer(FORM_LAYER),
        RenderTarget::Image(image.into()),
        Camera {
            // Before the window's cameras, whose frame shows what this one drew. The map's is -1.
            order: -2,
            is_active: false,
            clear_color: ClearColorConfig::Custom(BACKDROP),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection { fov: FORM_FOV, ..default() }),
        // The parts' material tone-maps itself, as the map's lines do.
        Tonemapping::None,
        Transform::default(),
    ));
}

/// The copies of the shown form, under one root in the ship's frame.
#[derive(Component)]
struct FormViewRoot;

/// A copy's paint, kept to relight it with.
#[derive(Component)]
struct HangarPaint(Vec4);

/// Start the draft from the ship's own form the first time the editor opens, or from the
/// starting form for a ship that has none.
fn start_draft(
    ui: Res<Ui>,
    own: Res<crate::parts::OwnForm>,
    dev: Res<crate::dev::DevEntry>,
    mut out: MessageWriter<Requested>,
) {
    if ui.view != ViewMode::Form || ui.form.draft.is_some() {
        return;
    }
    let ship = own.form().cloned().unwrap_or_else(Form::starting);
    out.write(Requested(Action::StartDraft(ship.clone())));
    let staged = dev.draft.as_deref().and_then(|name| crate::draft::staged(name, &ship, &Balance::DEFAULT));
    if let Some(form) = staged {
        let edit = crate::draft::Draft::new(ship).replace(form);
        out.write(Requested(Action::EditForm(Ok(edit))));
    }
}

/// Draw the draft solid, and the ship faint wherever the draft differs from it, each changed part
/// in its mark's color.
#[allow(clippy::too_many_arguments)]
fn show(
    mut commands: Commands,
    ui: Res<Ui>,
    mut shown: ResMut<Shown>,
    roots: Query<Entity, With<FormViewRoot>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodySurfaceMaterial>>,
    mut ghosts: ResMut<Assets<StandardMaterial>>,
    surfaces: Res<crate::surfaces::Surfaces>,
) {
    let Some(draft) = &ui.form.draft else { return };
    let fresh = shown.of.as_ref().is_none_or(|(form, ship)| *form != draft.form || *ship != draft.ship);
    // Respawned when missing too, since leaving the game takes the copies down.
    if !fresh && (!roots.is_empty() || shown.drawn.is_none()) {
        return;
    }
    if fresh {
        let balance = Balance::DEFAULT;
        let ghost = Sdf::new(&draft.ship, &balance).ok();
        shown.drawn = Sdf::new(&draft.form, &balance).ok().map(|sdf| {
            let (mut min, mut max) = sdf.bounds();
            if let Some(ghost) = &ghost {
                let (gmin, gmax) = ghost.bounds();
                (min, max) = (min.min(gmin), max.max(gmax));
            }
            (sdf, Extent { min, max })
        });
        shown.ghost = ghost;
        shown.marks = draft.marks(&balance);
        shown.of = Some((draft.form.clone(), draft.ship.clone()));
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let Some((sdf, _)) = &shown.drawn else { return };
    // Ship axes to render axes, the same turn a ship flying along +x with its back to +z takes.
    let turn = crate::hull::frame(DVec3::X, Some(DVec3::Z));
    let root = commands.spawn((Transform::from_rotation(turn), Visibility::default(), FormViewRoot)).id();
    for piece in sdf.pieces() {
        let (mesh, scale) = crate::parts::solid(&piece.shape);
        let mut paint = crate::parts::paint(piece.kind);
        if let Some(mark) = shown.marks.get(&piece.part) {
            paint = paint.lerp(mark.color().to_linear().to_vec4(), MARK_TINT).with_w(paint.w);
        }
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(surfaces.flat.material(hangar(paint, DVec3::Z)))),
            crate::parts::local(piece, scale),
            NoFrustumCulling,
            RenderLayers::layer(FORM_LAYER),
            HangarPaint(paint),
            ChildOf(root),
        ));
    }
    let Some(ghost) = &shown.ghost else { return };
    for piece in ghost.pieces().iter().filter(|p| shown.marks.contains_key(&p.part)) {
        let (mesh, scale) = crate::parts::solid(&piece.shape);
        let mark = shown.marks[&piece.part];
        let ghost = commands
            .spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(ghosts.add(ghost_material(mark))),
                crate::parts::local(piece, scale),
                NoFrustumCulling,
                RenderLayers::layer(FORM_LAYER),
                ChildOf(root),
            ))
            .id();
        if mark == crate::draft::Mark::Dismantle {
            commands.entity(ghost).insert((Dismantled, Visibility::Hidden));
        }
    }
}

/// How far a changed part's paint goes toward its mark's color.
const MARK_TINT: f32 = 0.45;
const GHOST_ALPHA: f32 = 0.18;

/// Unlit, so the hangar's light does not decide how faint it is.
fn ghost_material(mark: crate::draft::Mark) -> StandardMaterial {
    StandardMaterial {
        base_color: mark.color().with_alpha(GHOST_ALPHA),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    }
}

fn reveal(revealed: Res<Revealed>, mut ghosts: Query<&mut Visibility, With<Dismantled>>) {
    let wanted = if revealed.0 { Visibility::Inherited } else { Visibility::Hidden };
    for mut visibility in &mut ghosts {
        visibility.set_if_neq(wanted);
    }
}

/// A part lit from `key`, ship frame, whatever the star is doing.
fn hangar(paint: Vec4, key: DVec3) -> BodySurfaceUniform {
    let tone = crate::tonemap::ToneMap {
        surface_reference: HANGAR_REFERENCE,
        surface_stops: HANGAR_STOPS,
        ..crate::tonemap::ToneMap::default()
    };
    // Ship axes are drawn as simulation axes are (see `show`), so `uniforms` converts them right.
    let mut lit = crate::hull::uniforms(key, Vec3::splat(HANGAR_LIGHT), Vec3::ZERO, &tone, paint);
    lit.to_star.w = HANGAR_FILL;
    lit
}

/// The key light for an orbit: over the camera's shoulder, so the side being looked at is lit.
fn key_light(orbit: &FormOrbit) -> DVec3 {
    let [forward, right, up] = orbit.basis();
    (-forward + up * KEY_UP + right * KEY_ACROSS).normalize()
}

fn relight(
    ui: Res<Ui>,
    mut materials: ResMut<Assets<BodySurfaceMaterial>>,
    pieces: Query<(&MeshMaterial3d<BodySurfaceMaterial>, &HangarPaint)>,
) {
    if ui.view != ViewMode::Form {
        return;
    }
    let key = key_light(&ui.form.orbit);
    for (material, paint) in &pieces {
        let Some(mut asset) = materials.get_mut(&material.0) else { continue };
        let next = hangar(paint.0, key);
        if asset.uniforms != next {
            asset.uniforms = next;
        }
    }
}

/// Aim the camera, and switch it on only while the editor is the view.
pub(crate) fn place(
    mut ui: ResMut<Ui>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    camera: Single<(&mut Camera, &mut Transform, &mut Projection), With<FormCamera>>,
) {
    let (mut camera, mut transform, mut projection) = camera.into_inner();
    let active = ui.view == ViewMode::Form && surface.laid.is_some() && shown.extent().is_some();
    if camera.is_active != active {
        camera.is_active = active;
    }
    let Some(extent) = shown.extent().filter(|_| active) else { return };
    let held = ui.form.orbit.held_to(&extent);
    if held != ui.form.orbit {
        ui.form.orbit = held;
    }
    let turn = crate::hull::frame(DVec3::X, Some(DVec3::Z));
    let eye = turn * held.eye_m(&extent).as_vec3();
    let focus = turn * held.focus_m(&extent).as_vec3();
    *transform = Transform::from_translation(eye).looking_at(focus, turn * Vec3::Z);
    let standoff = (held.distance * extent.size_m()) as f32;
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.near = standoff * 1.0e-3;
        perspective.far = standoff * 10.0 + extent.size_m() as f32 * 2.0;
    }
}

/// Everything on screen that is the editor's and not the camera's.
#[derive(Component)]
struct Chrome;

#[derive(Component)]
struct Picture;

#[derive(Component)]
struct Piece(usize);

#[derive(Component)]
pub(crate) struct Emit(Action);

/// Lay the picture and the chrome out on the window, resizing the target to match.
///
/// Built once on the way in and taken down on the way out; each frame only moves what moved,
/// because Bevy UI is retained and a button rebuilt every frame never shows a hover.
#[allow(clippy::too_many_arguments)]
fn lay_out(
    mut commands: Commands,
    ui: Res<Ui>,
    foot: Res<crate::panels::HudFoot>,
    window: Single<&Window, With<PrimaryWindow>>,
    assets: Res<AssetServer>,
    mut surface: ResMut<FormSurface>,
    mut images: ResMut<Assets<Image>>,
    mut chrome: Query<(Entity, &mut Node), (With<Chrome>, Without<Piece>)>,
    pictures: Query<Entity, With<Picture>>,
    mut pieces: Query<(&Piece, &mut Node, &mut ImageNode), Without<Chrome>>,
) {
    if ui.view != ViewMode::Form {
        surface.laid = None;
        for (entity, ..) in &mut chrome {
            commands.entity(entity).despawn();
        }
        for entity in &pictures {
            commands.entity(entity).despawn();
        }
        return;
    }

    let (rect, hole) = layout_of(Vec2::new(window.width(), window.height()), foot.0);
    surface.laid = Some((rect, hole));
    let scale = window.scale_factor();
    let wanted = (Vec2::new(rect.width(), rect.height()) * scale)
        .round()
        .as_uvec2()
        .clamp(UVec2::splat(MIN_SIDE), UVec2::splat(MAX_SIDE));
    if wanted != surface.size
        && let Some(mut image) = images.get_mut(&surface.image)
    {
        image.resize(Extent3d { width: wanted.x, height: wanted.y, ..default() });
        surface.size = wanted;
    }

    let top = foot.0 + CHROME_GAP;
    let mut drawn = false;
    for (_, mut node) in &mut chrome {
        drawn = true;
        if node.top != Val::Px(top) {
            node.top = Val::Px(top);
        }
    }
    if !drawn {
        build_chrome(&mut commands, top, assets.load(crate::faces::UI_FILE));
    }

    let laid = crate::map_panel::around(rect, hole);
    if pictures.is_empty() {
        let picture = commands.spawn((Node { position_type: PositionType::Absolute, ..default() },
            GlobalZIndex(-1), Picture)).id();
        for index in 0..4 {
            commands.spawn((Node::default(), ImageNode::new(surface.image.clone()), Piece(index), ChildOf(picture)));
        }
        return;
    }
    let texels = surface.size.as_vec2();
    for (piece, mut node, mut image) in &mut pieces {
        let want = match laid.get(piece.0) {
            Some(at) => {
                let uv = crate::map_panel::uv(rect, *at);
                let texture = Rect::new(uv.min.x * texels.x, uv.min.y * texels.y, uv.max.x * texels.x, uv.max.y * texels.y);
                (placed(*at), Some(texture))
            }
            None => (Node { display: Display::None, ..default() }, None),
        };
        if *node != want.0 {
            *node = want.0;
        }
        if image.rect != want.1 {
            image.rect = want.1;
        }
    }
}

fn placed(at: egui::Rect) -> Node {
    Node {
        position_type: PositionType::Absolute,
        left: Val::Px(at.min.x),
        top: Val::Px(at.min.y),
        width: Val::Px(at.width()),
        height: Val::Px(at.height()),
        ..default()
    }
}

fn build_chrome(commands: &mut Commands, top: f32, font: Handle<Font>) {
    let mut ui = MenuUi::new(commands, MenuTheme::VFD).font(font);
    let root = ui.docked(Chrome, Edge::Top, top);
    let strip = ui.strip(root);
    let row = ui.row(strip);
    ui.inline(row, "SHIP EDITOR", 16.0, em_ui::vfd::TEXT);
    ui.inline(row, "current ship", 14.0, em_ui::vfd::TEXT_DIM);
    ui.small_button(row, "Back", Emit(Action::ToggleForm));
}

pub(crate) fn press(
    buttons: Query<(&Interaction, &Emit), Changed<Interaction>>,
    carried: Res<crate::form_carry::Carried>,
    mut out: MessageWriter<Requested>,
) {
    // A press while carrying a part is the drop's, whatever button is under it.
    if carried.is_carrying() {
        return;
    }
    for (interaction, emit) in &buttons {
        if *interaction == Interaction::Pressed {
            out.write(Requested(emit.0.clone()));
        }
    }
}

fn put_away(
    mut commands: Commands,
    mut surface: ResMut<FormSurface>,
    placed: Query<Entity, Or<(With<FormViewRoot>, With<Chrome>, With<Picture>)>>,
) {
    surface.laid = None;
    for entity in &placed {
        commands.entity(entity).despawn();
    }
}

/// Whether the pointer is the editor's to read: not over an egui surface, not over one of the
/// editor's own controls.
fn pointer_free(egui: &EguiWantsInput, controls: &em_ui::Controls) -> bool {
    !egui.wants_any_pointer_input() && !controls.under_pointer()
}

/// A left-drag on empty space slides the focus. The press decides, so a drag that starts on a
/// control or a part stays theirs however far it wanders.
#[allow(clippy::too_many_arguments)]
pub fn read_drag(
    ui: Res<Ui>,
    buttons: Res<ButtonInput<MouseButton>>,
    egui: Res<EguiWantsInput>,
    controls: em_ui::Controls,
    window: Single<&Window, With<PrimaryWindow>>,
    shown: Res<Shown>,
    surface: Res<FormSurface>,
    carried: Res<crate::form_carry::Carried>,
    mut last: Local<Option<Vec2>>,
    mut out: MessageWriter<Requested>,
) {
    let cursor = window.cursor_position();
    let (Some((sdf, extent)), Some((rect, hole))) = (&shown.drawn, surface.laid) else {
        *last = None;
        return;
    };
    if !buttons.pressed(MouseButton::Left) {
        *last = None;
        return;
    }
    let size = Vec2::new(rect.width(), rect.height());
    if buttons.just_pressed(MouseButton::Left) {
        // The part is the one the handles would pick, so a press slides exactly where it selects
        // nothing.
        let lens = crate::form_handles::Lens { orbit: ui.form.orbit.held_to(extent), extent: *extent, rect };
        *last = cursor
            // A press while carrying a part puts it down, and slides nothing.
            .filter(|at| pointer_free(&egui, &controls) && inside(rect, hole, *at) && !carried.is_carrying())
            .filter(|at| {
                let on_part = crate::form_handles::pick_part(sdf, &lens, *at).is_some();
                drag_of(MouseButton::Left, on_part) == Some(FormDrag::Slide)
            });
        return;
    }
    let (Some(from), Some(to)) = (*last, cursor) else { return };
    *last = Some(to);
    let standoffs = slide_of(to - from, ui.form.orbit.nose_on_screen(), FORM_FOV, size.y);
    if standoffs != 0.0 {
        out.write(Requested(Action::SlideForm(standoffs)));
    }
}

fn inside(rect: egui::Rect, hole: egui::Rect, at: Vec2) -> bool {
    let at = egui::pos2(at.x, at.y);
    rect.contains(at) && !hole.contains(at)
}

/// The keyboard path for the slide, held like the arrows. Not while a book has the page keys.
pub fn read_slide_keys(
    keys: Res<ButtonInput<KeyCode>>,
    egui: Res<EguiWantsInput>,
    typing: em_ui::Typing,
    ui: Res<Ui>,
    time: Res<Time>,
    mut out: MessageWriter<Requested>,
) {
    if egui.wants_any_keyboard_input() || typing.active() || ui.reading.book.is_some() {
        return;
    }
    let way: f64 = crate::input::slide_bindings()
        .into_iter()
        .filter(|(key, _)| keys.pressed(*key))
        .map(|(_, way)| way)
        .sum();
    if way != 0.0 {
        out.write(Requested(Action::SlideForm(way * SLIDE_PER_SECOND * time.delta_secs_f64())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long() -> Extent {
        Extent { min: DVec3::new(-30_000.0, -800.0, -600.0), max: DVec3::new(20_000.0, 800.0, 600.0) }
    }

    fn close(a: DVec3, b: DVec3) -> bool {
        (a - b).length() < 1.0e-9 * b.length().max(1.0)
    }

    /// The right button is the look button everywhere, and the left belongs to C2 over a part.
    #[test]
    fn the_press_decides_what_a_drag_does() {
        assert_eq!(drag_of(crate::input::LOOK_BUTTON, false), Some(FormDrag::Orbit));
        assert_eq!(drag_of(crate::input::LOOK_BUTTON, true), Some(FormDrag::Orbit), "orbiting starts anywhere");
        assert_eq!(drag_of(MouseButton::Left, false), Some(FormDrag::Slide));
        assert_eq!(drag_of(MouseButton::Left, true), None, "a part's press is reserved for its handles");
        assert_eq!(drag_of(MouseButton::Middle, false), None);
    }

    /// **A drag means the same turn in every mode.** The sky reads it through `look_from`, and
    /// the orbit takes that turn with the map's signs.
    #[test]
    fn a_drag_turns_the_orbit_as_far_as_it_turns_the_sky() {
        let drag = Vec2::new(40.0, -25.0);
        let (yaw, pitch) = crate::input::look_from(drag);
        let Action::OrbitForm { azimuth, elevation } = orbit_from(yaw, pitch) else { panic!() };
        assert_eq!(azimuth.abs(), 40.0 * crate::input::MOUSE_SENSITIVITY);
        assert_eq!(elevation.abs(), 25.0 * crate::input::MOUSE_SENSITIVITY);
        // A drag right turns the map's azimuth down, and this one's too.
        assert!(azimuth < 0.0);
        // A drag up tilts the view up, which on an orbit is the camera going down.
        assert!(elevation < 0.0);
    }

    #[test]
    fn the_orbit_stays_off_the_poles_and_wraps_round() {
        let mut orbit = FormOrbit::default();
        orbit.turn(0.0, 10.0);
        assert_eq!(orbit.elevation, crate::ui::Look::PITCH_LIMIT);
        orbit.turn(7.0, -20.0);
        assert_eq!(orbit.elevation, -crate::ui::Look::PITCH_LIMIT);
        assert!((0.0..std::f64::consts::TAU).contains(&orbit.azimuth));
    }

    fn outside(extent: &Extent, at: DVec3) -> bool {
        (0..3).any(|i| at[i] < extent.min[i] || at[i] > extent.max[i])
    }

    /// **The near stop is outside the form along the line of sight**, however the camera is
    /// turned and wherever the focus has slid: nose-on, stern-on, broadside and overhead, from
    /// the middle and from both ends. A stop by distance alone put the eye 24 km inside this hull
    /// looking nose-on.
    #[test]
    fn zoomed_all_the_way_in_the_eye_is_still_outside_the_hull() {
        let extent = long();
        let limit = crate::ui::Look::PITCH_LIMIT;
        for along in [0.0, 1.0e6, -1.0e6] {
            for (azimuth, elevation) in [(0.0, 0.0), (std::f64::consts::PI, 0.0), (1.0, 0.0), (-2.0, 0.4), (0.3, limit)] {
                let mut orbit = FormOrbit { azimuth, elevation, along, ..FormOrbit::default() };
                orbit.zoom(1000.0);
                let orbit = orbit.held_to(&extent);
                let eye = orbit.eye_m(&extent);
                assert!(outside(&extent, eye), "({along}, {azimuth}, {elevation}): the eye is at {eye}, inside");
            }
        }
    }

    /// Zoom in notches, as the boom's, and the far end is a clamp too.
    #[test]
    fn zoom_is_clamped_short_of_losing_the_form() {
        let extent = long();
        let mut orbit = FormOrbit::default();
        orbit.zoom(-1000.0);
        assert_eq!(orbit.held_to(&extent).distance, FAR_SIZES);
        let mut one = FormOrbit::default();
        one.zoom(1.0);
        assert!((FormOrbit::default().distance / one.distance - crate::hull::ZOOM_STEP).abs() < 1e-12);
    }

    /// **Long ships are the point.** A 50 km hull can be slid to either end and no further.
    #[test]
    fn the_focus_slides_stem_to_stern_and_stops_there() {
        let extent = long();
        let mut orbit = FormOrbit::default();
        orbit.slide(1.0e6);
        orbit = orbit.held_to(&extent);
        assert!((orbit.focus_m(&extent).x - extent.max.x).abs() < 1e-6, "the nose");
        orbit.slide(-1.0e6);
        orbit = orbit.held_to(&extent);
        assert!((orbit.focus_m(&extent).x - extent.min.x).abs() < 1e-6, "the stern");
        // Along the axis only.
        assert_eq!(orbit.focus_m(&extent).y, extent.middle().y);
        assert_eq!(orbit.focus_m(&extent).z, extent.middle().z);
    }

    /// A slide is in stand-offs, so a notch covers the same fraction of the screen close in and
    /// far out.
    #[test]
    fn a_slide_scales_with_the_zoom() {
        let mut near = FormOrbit { distance: 0.1, ..FormOrbit::default() };
        let mut far = FormOrbit { distance: 2.0, ..FormOrbit::default() };
        near.slide(SLIDE_PER_NOTCH);
        far.slide(SLIDE_PER_NOTCH);
        assert!((far.along / near.along - 20.0).abs() < 1e-9);
    }

    /// The point under the cursor follows it: a drag toward the nose on screen moves the focus aft.
    #[test]
    fn a_drag_along_the_nose_slides_and_a_drag_across_it_does_not() {
        let nose = Vec2::new(1.0, 0.0);
        let full = slide_of(Vec2::new(720.0, 0.0), nose, FORM_FOV, 720.0);
        let expected = -2.0 * (FORM_FOV as f64 * 0.5).tan();
        assert!((full - expected).abs() < 1e-6, "{full}");
        assert_eq!(slide_of(Vec2::new(0.0, 300.0), nose, FORM_FOV, 720.0), 0.0);
        // A foreshortened axis slides further for the same drag, so the ship keeps up with the
        // cursor, until it is nearly end-on.
        let half = slide_of(Vec2::new(100.0, 0.0), nose * 0.5, FORM_FOV, 720.0);
        assert!((half / slide_of(Vec2::new(100.0, 0.0), nose, FORM_FOV, 720.0) - 2.0).abs() < 1e-6);
        let end_on = slide_of(Vec2::new(0.0, -100.0), Vec2::ZERO, FORM_FOV, 720.0);
        assert!(end_on.is_finite() && end_on < 0.0, "nose-on, up the screen still moves");
    }

    /// The nose's direction on screen is read from the same basis the camera is aimed with.
    #[test]
    fn the_nose_lies_to_the_right_from_the_starboard_side() {
        let orbit = FormOrbit { azimuth: -std::f64::consts::FRAC_PI_2, elevation: 0.0, ..FormOrbit::default() };
        let nose = orbit.nose_on_screen();
        assert!((nose - Vec2::X).length() < 1e-6, "{nose}");
        let aft = FormOrbit { azimuth: std::f64::consts::PI, elevation: 0.0, ..FormOrbit::default() };
        assert!(aft.nose_on_screen().length() < 1e-6, "from dead astern the axis is a point");
    }

    #[test]
    fn the_middle_ray_runs_from_the_eye_to_the_focus() {
        let extent = long();
        let orbit = FormOrbit::default().held_to(&extent);
        let (origin, direction) = orbit.ray(&extent, FORM_FOV as f64, 16.0 / 9.0, DVec2::ZERO);
        assert!(close(origin, orbit.eye_m(&extent)));
        assert!(close(direction, (orbit.focus_m(&extent) - origin).normalize()));
        // And a ray up the screen points further up than the middle one.
        let (_, up) = orbit.ray(&extent, FORM_FOV as f64, 16.0 / 9.0, DVec2::new(0.0, 1.0));
        assert!(up.z > direction.z);
    }

    fn starting() -> (Sdf, Extent) {
        let sdf = Sdf::new(&Form::starting(), &Balance::DEFAULT).unwrap();
        let (min, max) = sdf.bounds();
        (sdf, Extent { min, max })
    }

    /// The middle of the view is on the ship at the opening framing, and a corner of it is not.
    #[test]
    fn a_press_finds_the_part_under_it() {
        let (sdf, extent) = starting();
        let orbit = FormOrbit::default().held_to(&extent);
        let limit = 10.0 * extent.size_m();
        let cast = |ndc: DVec2| {
            let (origin, direction) = orbit.ray(&extent, FORM_FOV as f64, 16.0 / 9.0, ndc);
            on_part(&sdf, origin, direction, limit)
        };
        assert!(cast(DVec2::ZERO), "the ship is in the middle of its own editor");
        assert!(!cast(DVec2::new(0.95, 0.95)), "the corner is empty space");
    }

    /// The camera opens framing the whole form: every corner of the bounds is in front of it
    /// and inside the view.
    #[test]
    fn the_opening_framing_holds_the_whole_form() {
        let (_, extent) = starting();
        let orbit = FormOrbit::default().held_to(&extent);
        let [forward, right, up] = orbit.basis();
        let eye = orbit.eye_m(&extent);
        let half = (FORM_FOV as f64 * 0.5).tan();
        for i in 0..8 {
            let corner = DVec3::new(
                if i & 1 == 0 { extent.min.x } else { extent.max.x },
                if i & 2 == 0 { extent.min.y } else { extent.max.y },
                if i & 4 == 0 { extent.min.z } else { extent.max.z },
            ) - eye;
            let depth = corner.dot(forward);
            assert!(depth > 0.0);
            assert!(corner.dot(up).abs() / depth < half, "corner {i} is off the top or bottom");
            assert!(corner.dot(right).abs() / depth < half * 16.0 / 9.0, "corner {i} is off the side");
        }
    }

    /// The picture is under the readout and leaves the corner to the sky.
    #[test]
    fn the_picture_is_under_the_readout_with_the_sky_in_the_corner() {
        let (rect, hole) = layout_of(Vec2::new(1280.0, 720.0), 61.0);
        assert_eq!(rect.min.y, 61.0);
        assert_eq!(rect.max, egui::pos2(1280.0, 720.0));
        assert_eq!(hole, crate::map_panel::corner(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))));
        assert!(inside(rect, hole, Vec2::new(640.0, 360.0)));
        assert!(!inside(rect, hole, Vec2::new(hole.center().x, hole.center().y)), "the corner is the sky's");
        assert!(!inside(rect, hole, Vec2::new(640.0, 20.0)), "the readout is egui's");
    }

    /// The hangar light comes from the viewer's side, so the face being looked at is lit.
    #[test]
    fn the_key_light_is_on_the_cameras_side() {
        for azimuth in [0.0, 1.0, 2.5, 4.0] {
            let orbit = FormOrbit { azimuth, ..FormOrbit::default() };
            assert!(key_light(&orbit).dot(orbit.basis()[0]) < -0.5, "{azimuth}");
        }
    }
}
