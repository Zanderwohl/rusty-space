//! The map's camera, its render target, and the entities it draws.
//!
//! A second `Camera3d` on its own [`MAP_LAYER`], rendering into an [`Image`] that egui shows.
//!
//! Transforms are camera-relative, so an entity belongs to exactly one camera: the whole view
//! and the corner square share one image because two aimed views would need two sets of
//! entities.
//!
//! No `Hdr`, bloom or tone map. The sky's camera is metered for a photograph and a diagram
//! needs a different range — see [`LINE_COLOR_SCALE`].

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureFormat, TextureUsages};
use bevy_egui::EguiUserTextures;
use em_map::{ItemKey, ItemKind, MapFrame, MapSnapshot, Placement, compose};
use em_render::body_material::{BASE_TUBE_RADIUS, BodyWireframeMaterial};
use em_render::render_space::sim_to_render;
use em_render::wire_mesh;
use glam::DVec3;

use crate::app::{Game, Stage, Ui};
use crate::map_source::Source;

/// The map's own layer. The sky keeps layer 0, so neither camera sees the other's entities.
pub const MAP_LAYER: usize = 1;

/// What the target starts at, before a surface has asked for a size.
const INITIAL_SIDE: u32 = 512;

/// egui reports a zero rect on the frame a window opens, and a zero-sized texture is a wgpu
/// validation failure. The ceiling bounds a runaway resize drag on a dense display.
const MIN_SIDE: u32 = 64;
const MAX_SIDE: u32 = 4096;

/// How wide a line is drawn, in pixels. Screen-constant, so the shader's `target_tube_radius`
/// is set per entity from that entity's own scale.
const LINE_PX: f32 = 1.6;

/// The most of its own unit mesh a tube may take, per family.
///
/// The shader displaces along normals in mesh space, so this is a proportion of the thing
/// drawn and not of the screen. Unclamped, a ring seen from forty times its radius wants a
/// tube a tenth of itself thick and the camera ends up inside it.
const LINE_TUBE_FRACTION: f32 = 0.02;
const SPHERE_TUBE_FRACTION: f32 = 0.06;

/// A disc has one normal, so displacement translates it instead of thickening it. Asking for
/// the material's base radius makes the displacement zero.
const DOT_TUBE_FRACTION: f32 = BASE_TUBE_RADIUS;

/// The reference scale — rings, spokes, drop-lines — is drawn at half a line's width and half
/// its brightness. It is the ruler, not what is being measured.
const SCALE_PX: f32 = LINE_PX * 0.5;
const SCALE_COLOR_SCALE: f32 = LINE_COLOR_SCALE * 0.5;

/// How much of the palette color a line is drawn at.
///
/// The shader gives `base_color * (1 + alpha * emission_strength)`, where alpha is the mesh's
/// line weight: 0.6 for grid, 1.0 for an equator. With no tone map in front of it the whole
/// range has to land inside the display, so 0.45 and 1.2 put a grid line at 0.77 and an
/// equator at 0.99.
const LINE_COLOR_SCALE: f32 = 0.45;
const LINE_EMISSION: f32 = 1.2;

/// The near and far planes, as multiples of the stand-off: wide enough for a ship beside the
/// camera and the outermost ring at once, and no wider.
const NEAR_FRACTION: f32 = 1.0e-4;
const FAR_MULTIPLE: f32 = 1.0e6;

/// The map camera's vertical field of view. Named because the panel casts the cursor's ray
/// with it, and a camera and a cursor that disagree put the anchor away from the pointer.
pub const MAP_FOV: f32 = std::f32::consts::FRAC_PI_4;

/// Divisions of a decade ring. Enough that the largest one does not read as a polygon.
const RING_SEGMENTS: u32 = 128;

/// How far the spokes reach, as a multiple of the stand-off.
///
/// Far past the edge of the view: a spoke that ends inside the frame reads as an object with
/// a tip. At forty the far end's tube is a fortieth of a pixel, so it fades out instead.
/// Passing the eye is safe because the elevation floor clears the plane by `sin(3°)` of the
/// stand-off, sixteen times a line's half-width.
const SPOKE_REACH: f32 = 40.0;

/// Where a spoke starts, as a fraction of the stand-off, so the hole in the middle holds its
/// size whatever [`SPOKE_REACH`] is.
const SPOKE_INNER: f32 = 0.02;

/// Radial spokes in the reference plane.
///
/// Their thickness is sized to the stand-off, not to the outermost ring. A ring is all at one
/// distance so one radius serves it; a spoke runs from near the camera to its rim, and a width
/// of a pixel at the far end is eighty at the near one.
const PLANE_SPOKES: u32 = 12;

/// How long a dash is, in pixels, wherever it is drawn. The mesh is scaled to the drop, so a
/// fixed count would give a tall drop long dashes; the host picks the count per drop instead.
const DASH_PX: f32 = 5.0;

/// The most dashes a drop-line is built with. A drop needing more than this is longer than the
/// viewport many times over and its dashes are sub-pixel anyway.
const MAX_DASHES: usize = 48;

/// A mark's apparent diameter, as a share of the viewport's height. A diameter, unlike the
/// sky's `resolved::RESOLVE_PX`, and a share because one texture serves two surfaces.
///
/// Both the threshold and the size a mark is drawn at, so a body shrinks to this and holds and
/// nothing jumps at the crossover. Below it a sphere is a dozen sub-pixel tubes over each
/// other: dearer to draw and less legible.
const POINT_FRACTION: f32 = 0.02;

/// The floor under that, at twice a line's width. Below it a ring has no inside left, which
/// makes it a dot.
const POINT_FLOOR_PX: f32 = 2.0 * LINE_PX;

/// Divisions of a mark. Sixteen is smooth at any size one reaches, and an eighth of the ring
/// mesh — a level of detail that cost more than the sphere would not be one.
const POINT_SEGMENTS: u32 = 16;

/// The camera the map is drawn for.
#[derive(Component)]
pub struct MapCamera;

/// Everything the map spawns, so a rebuild can clear the layer without touching anything else.
#[derive(Component)]
pub struct MapDrawn;

/// Which item an entity stands for, so a transform can be written without respawning.
#[derive(Component)]
pub struct MapItemOf(pub ItemKey);

/// A decade ring, by its place in the frame's list.
#[derive(Component)]
pub struct MapRingOf(pub usize);

/// The drop-line under an item.
#[derive(Component)]
pub struct MapDropOf(pub ItemKey);

/// A belt, a ring system or a cloud, drawn as its own outline rather than as a point.
#[derive(Component)]
pub struct MapAnnulusOf(pub ItemKey);

/// The reference plane's spokes. One entity.
#[derive(Component)]
pub struct MapSpokes;

#[derive(Resource)]
pub struct Map {
    pub image: Handle<Image>,
    pub texture: Option<bevy_egui::egui::TextureId>,
    /// What the target currently is.
    pub size: UVec2,
    /// What the surface drawing it asked for, last frame. egui runs after the scene stage, so
    /// a resize lands one frame late and that frame shows the previous texture stretched.
    pub wanted: UVec2,
    /// Whether anything is showing the map. Nothing is drawn when nothing is looking.
    pub shown: bool,
    pub snapshot: MapSnapshot,
    /// What each item of the snapshot is, in the terms the rest of the interface selects
    /// things in. See [`crate::map_source::Picture`].
    pub subjects: Vec<(ItemKey, crate::pick::Subject)>,
    /// Which item holds the ship, when the snapshot has one. Worked out beside the snapshot
    /// because that is where the session is.
    pub primary: Option<ItemKey>,
    pub frame: Option<MapFrame>,
    sphere: Handle<Mesh>,
    /// An unresolved body: a circle facing the eye. See [`POINT_FRACTION`].
    point: Handle<Mesh>,
    /// A ship, at any zoom: the same size, filled.
    dot: Handle<Mesh>,
    ring: Handle<Mesh>,
    spokes: Handle<Mesh>,
    /// One drop-line mesh per dash count, indexed from one dash. Built once, so a body
    /// drifting off the plane swaps a handle instead of rebuilding geometry.
    drops: Vec<Handle<Mesh>>,
    /// What is spawned, in order. A rebuild happens only when this stops matching the frame.
    drawn: Vec<ItemKey>,
    rings_drawn: usize,
}

impl Map {
    /// Where the reference plane is anchored: the observer, or the camera's focus when a
    /// snapshot has nobody in it.
    pub fn plane_origin_ly(&self, focus_ly: DVec3) -> DVec3 {
        self.snapshot.observer().map_or(focus_ly, |o| o.position_ly)
    }

    fn viewport(&self, fov_y: f32) -> Viewport {
        Viewport::new(self.size.y, fov_y)
    }

    /// How big a mark is, in texture pixels. A label has to clear it, after scaling by the
    /// points that texture is shown at.
    pub fn symbol_px(&self) -> f32 {
        self.viewport(MAP_FOV).point_px
    }

    /// The radius a placement is drawn at, in texture pixels. Not the nominal mark size: a
    /// resolved body is a sphere at its own angular size and only an unresolved one falls back
    /// to the symbol. Picking reads this so that it agrees with the picture. See [`Form`].
    pub fn drawn_radius_px(&self, placement: &Placement) -> f32 {
        let view = self.viewport(MAP_FOV);
        match form_of(placement, view) {
            Form::Sphere => placement.angular_radius / view.rad_per_px.max(f32::MIN_POSITIVE),
            Form::Circle | Form::Dot => view.mark_px(placement) * 0.5,
        }
    }

    /// Points of a surface per pixel of the texture. They differ on a display that scales, and
    /// on the frame after a resize.
    pub fn points_per_pixel(&self, surface_height: f32) -> f32 {
        match self.size.y {
            0 => 1.0,
            height => surface_height / height as f32,
        }
    }
}

/// What a pixel of the map's viewport is worth. Everything on the layer is sized from one of
/// these two, and both come from the viewport's height.
#[derive(Clone, Copy, Debug)]
struct Viewport {
    /// Radians a pixel subtends: every line's thickness, and half of the sphere decision.
    rad_per_px: f32,
    /// A mark's diameter in pixels. See [`POINT_FRACTION`].
    point_px: f32,
}

impl Viewport {
    fn new(height_px: u32, fov_y: f32) -> Self {
        Self {
            rad_per_px: match height_px {
                0 => 0.0,
                height => 2.0 * (fov_y * 0.5).tan() / height as f32,
            },
            point_px: (height_px as f32 * POINT_FRACTION).max(POINT_FLOOR_PX),
        }
    }

    /// One item's mark, in pixels: the surface's size scaled by what the thing weighs, never
    /// under the floor. The pixel floor usually binds before [`em_map::weight::MIN_SCALE`].
    fn mark_px(self, placement: &Placement) -> f32 {
        (self.point_px * placement.symbol_scale).max(POINT_FLOOR_PX)
    }

    /// A circle's line, as a fraction of its radius. Both are fixed pixel sizes, so the
    /// distance and scale in [`tube_target`] cancel and this is the answer, not a cap.
    fn point_tube_fraction(self, mark_px: f32) -> f32 {
        LINE_PX / (mark_px * 0.5)
    }
}

/// How a body is drawn at this zoom. One decision, read in three places.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Form {
    Sphere,
    Circle,
    Dot,
}

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(em_render::body_material::BodyWireframeMaterialPlugin)
            .add_systems(Startup, setup)
            // **After the scene the snapshot is built from.** Taken in `Stage::Act`, it held
            // the previous frame's eye while the contacts in it were this frame's, so this
            // ship's own mark trailed one frame behind everything around it — a jitter
            // whenever the ship was under way.
            .add_systems(
                Update,
                (survey, resize, place).chain().in_set(Stage::Scene).after(crate::app::Placed),
            );
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut textures: ResMut<EguiUserTextures>,
) {
    let image = images.add(target_image(UVec2::splat(INITIAL_SIDE)));
    let texture = textures.add_image(bevy_egui::EguiTextureHandle::Strong(image.clone()));
    let map_image = image.clone();

    commands.insert_resource(Map {
        texture: Some(texture),
        size: UVec2::splat(INITIAL_SIDE),
        wanted: UVec2::splat(INITIAL_SIDE),
        shown: false,
        snapshot: MapSnapshot::observed(0.0, Vec::new()),
        subjects: Vec::new(),
        primary: None,
        frame: None,
        sphere: meshes.add(wire_mesh::generate_latlon_sphere(&[], BASE_TUBE_RADIUS, 4)),
        point: meshes.add(wire_mesh::ring_tube(POINT_SEGMENTS, BASE_TUBE_RADIUS, 4, 1.0)),
        dot: meshes.add(wire_mesh::disc(POINT_SEGMENTS, 1.0)),
        ring: meshes.add(wire_mesh::ring_tube(RING_SEGMENTS, BASE_TUBE_RADIUS, 4, 1.0)),
        spokes: meshes.add(wire_mesh::plane_spokes(
            PLANE_SPOKES,
            SPOKE_INNER / SPOKE_REACH,
            BASE_TUBE_RADIUS,
            4,
            0.6,
        )),
        drops: (1..=MAX_DASHES)
            .map(|n| meshes.add(wire_mesh::drop_line(n as u32, BASE_TUBE_RADIUS, 4, 0.8)))
            .collect(),
        drawn: Vec::new(),
        rings_drawn: 0,
        image,
    });

    commands.spawn((
        Camera3d::default(),
        MapCamera,
        RenderLayers::layer(MAP_LAYER),
        // A component of its own in Bevy 0.19, not a field on `Camera`. Left off, this camera
        // renders over the primary window on an empty layer and clears everything to black,
        // with nothing in the log to say why.
        RenderTarget::Image(map_image.into()),
        Camera {
            // Before the window camera, whose frame shows what this one drew.
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection { fov: MAP_FOV, ..default() }),
        // See the module doc.
        Tonemapping::None,
        Transform::default(),
    ));
}

fn target_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(
        size.x.max(MIN_SIDE),
        size.y.max(MIN_SIDE),
        // 8-bit sRGB, so egui samples it and gets back what was drawn. A float target is
        // stored linear and comes out of `ui.image` looking like a shading bug.
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    // `Image::resize` copies the old contents forward so a resize does not flash, and
    // `new_target_texture` leaves out the COPY_SRC that needs. Without it the first resize is
    // a wgpu validation failure that ends the process.
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image
}

/// Reallocate the target when the surface drawing it has changed size, and not otherwise:
/// resizing an `Image` asset reallocates a GPU texture.
fn resize(mut map: ResMut<Map>, mut images: ResMut<Assets<Image>>) {
    let wanted = map.wanted.clamp(UVec2::splat(MIN_SIDE), UVec2::splat(MAX_SIDE));
    if wanted == map.size {
        return;
    }
    let Some(mut image) = images.get_mut(&map.image) else { return };
    image.resize(Extent3d { width: wanted.x, height: wanted.y, ..default() });
    map.size = wanted;
}

/// Build this frame's snapshot from whichever source the interface is showing.
fn survey(
    game: Res<Game>,
    ui: Res<Ui>,
    bodies: Res<crate::starfield::Bodies>,
    uplink: Res<crate::uplink::Uplink>,
    eye: Res<crate::hull::Eye>,
    mut map: ResMut<Map>,
) {
    if !map.shown {
        return;
    }
    let picture = match ui.map.source {
        Source::Observed => crate::map_source::observed(&game.0, &bodies, &uplink, eye.at_ly),
        #[cfg(feature = "godview")]
        Source::God => crate::map_source::coordinate(&game.0, &uplink, eye.at_ly),
    };
    map.snapshot = picture.snapshot;
    map.subjects = picture.subjects;
    map.primary = crate::map_source::primary(&game.0);
}

#[allow(clippy::too_many_arguments)]
fn place(
    mut commands: Commands,
    mut map: ResMut<Map>,
    mut ui: ResMut<Ui>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut camera: Query<(&mut Transform, &mut Projection), (With<MapCamera>, Without<MapDrawn>)>,
    existing: Query<Entity, With<MapDrawn>>,
    mut items: Query<
        (&MapItemOf, &mut Transform, &mut Mesh3d, &MeshMaterial3d<BodyWireframeMaterial>),
        (Without<MapCamera>, Without<MapDropOf>, Without<MapRingOf>, Without<MapAnnulusOf>, Without<MapSpokes>),
    >,
    mut drops: Query<
        (&MapDropOf, &mut Transform, &mut Mesh3d, &MeshMaterial3d<BodyWireframeMaterial>),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapRingOf>, Without<MapSpokes>, Without<MapAnnulusOf>),
    >,
    mut rings: Query<
        (&MapRingOf, &mut Transform, &MeshMaterial3d<BodyWireframeMaterial>),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapDropOf>, Without<MapSpokes>, Without<MapAnnulusOf>),
    >,
    mut spokes: Query<
        (&mut Transform, &MeshMaterial3d<BodyWireframeMaterial>),
        (With<MapSpokes>, Without<MapCamera>, Without<MapItemOf>, Without<MapDropOf>,
            Without<MapRingOf>, Without<MapAnnulusOf>),
    >,
    mut annuli: Query<
        (&MapAnnulusOf, &mut Transform, &MeshMaterial3d<BodyWireframeMaterial>),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapDropOf>, Without<MapRingOf>,
            Without<MapSpokes>),
    >,
) {
    let Ok((mut transform, mut projection)) = camera.single_mut() else { return };
    if !map.shown {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        map.drawn.clear();
        map.rings_drawn = 0;
        map.frame = None;
        return;
    }

    // Follow the selection, not a remembered position: Saturn moves. Written back rather than
    // applied to a copy, because every action that moves the camera starts from the
    // interface's own `focus_ly`, and a stale one makes the first frame of a drag jump.
    follow(ui.map.focus, &map.snapshot, map.primary, &mut ui.map.orbit.focus_ly);
    // After the follow: the line is measured from where the camera is now looking.
    spin(&mut ui.map, &map.snapshot, map.primary);
    let view = ui.map;
    let meters_per_unit = crate::view::ScaleTier::for_distance(view.orbit.distance_m())
        .meters_per_unit();
    let frame = compose(&map.snapshot, &view.orbit, view.plane, meters_per_unit);

    // The depth range is written from the stand-off every frame rather than fixed. The sky's
    // camera spans 1e-10 to 1e9 because it has to cover everything at once; the map's distance
    // is a piece of state, so it can be exact — and reversed depth makes the span cheap.
    let standoff = (view.orbit.distance_m() / meters_per_unit) as f32;
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.near = (standoff * NEAR_FRACTION).max(f32::MIN_POSITIVE);
        perspective.far = standoff * FAR_MULTIPLE;
    }

    // The eye is the render origin, and the map looks back at its focus.
    let (forward, up) = view.orbit.orientation(view.plane);
    transform.translation = Vec3::ZERO;
    transform.look_to(render(forward), render(up));

    let fov_y = match &*projection {
        Projection::Perspective(perspective) => perspective.fov,
        _ => MAP_FOV,
    };
    let view = map.viewport(fov_y);
    let rad_per_px = view.rad_per_px;

    let wanted: Vec<ItemKey> = frame.placements.iter().map(|p| p.key).collect();
    if wanted != map.drawn || frame.rings.len() != map.rings_drawn {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        spawn_scene(&mut commands, &map, &frame, standoff, view, &mut meshes, &mut materials);
        map.drawn = wanted;
        map.rings_drawn = frame.rings.len();
        map.frame = Some(frame);
        return;
    }

    for (of, mut at, mut mesh, material) in items.iter_mut() {
        let Some(placement) = frame.placements.iter().find(|p| p.key == of.0) else { continue };
        *at = item_transform(placement, view);
        // Crossing the threshold does not change the set that is drawn, so the level of
        // detail is a handle swap rather than a respawn.
        let (wanted, fraction) = mesh_for(form_of(placement, view), placement, &map, view);
        if mesh.0 != *wanted {
            mesh.0 = wanted.clone();
        }
        set_thickness(&mut materials, material, at.scale.max_element(), view.rad_per_px,
            at.translation.length(), fraction, LINE_PX);
    }
    for (of, mut at, mut mesh, material) in drops.iter_mut() {
        let Some(placement) = frame.placements.iter().find(|p| p.key == of.0) else { continue };
        *at = drop_transform(placement);
        // Drifting off the plane gains dashes, not longer ones, so the mesh changes.
        let dashes = dash_count(at.scale.y, at.translation.length(), rad_per_px);
        let wanted = &map.drops[dashes - 1];
        if mesh.0 != *wanted {
            mesh.0 = wanted.clone();
        }
        set_thickness(&mut materials, material, 1.0, rad_per_px, at.translation.length(),
            LINE_TUBE_FRACTION, SCALE_PX);
    }
    for (of, mut at, material) in rings.iter_mut() {
        let Some(ring) = frame.rings.get(of.0) else { continue };
        *at = ring_transform(&frame, ring.radius);
        set_thickness(&mut materials, material, ring.radius, rad_per_px,
            at.translation.length().max(ring.radius), LINE_TUBE_FRACTION, SCALE_PX);
    }
    for (of, mut at, material) in annuli.iter_mut() {
        let Some(placement) = frame.placements.iter().find(|p| p.key == of.0) else { continue };
        let Some(annulus) = placement.annulus else { continue };
        *at = annulus_transform(placement, annulus);
        set_thickness(&mut materials, material, annulus.outer, rad_per_px,
            nearest_reach(at.translation.length(), annulus, standoff), LINE_TUBE_FRACTION,
            LINE_PX);
    }
    if let Ok((mut at, material)) = spokes.single_mut() {
        *at = ring_transform(&frame, standoff * SPOKE_REACH);
        // Against the transform's scale, not the stand-off: the shader displaces in mesh
        // space, so any other number is wrong by that ratio. Sized at the near end.
        set_thickness(&mut materials, material, standoff * SPOKE_REACH, rad_per_px, standoff,
            LINE_TUBE_FRACTION, SCALE_PX);
    }
    map.frame = Some(frame);
}

/// Put `focus_ly` where the focus says to look, and say whether it moved. Pans and zooms start
/// from this value, so a stale one is a jump.
pub fn follow(
    focus: crate::ui::MapFocus,
    snapshot: &MapSnapshot,
    primary: Option<ItemKey>,
    focus_ly: &mut DVec3,
) -> bool {
    match focus_position(focus, snapshot, primary) {
        Some(at) if at != *focus_ly => {
            *focus_ly = at;
            true
        }
        _ => false,
    }
}

/// Where the camera should look, or `None` to leave it. A key no longer in the snapshot also
/// leaves it, so a body going out of range stops the camera rather than moving it to nowhere.
pub fn focus_position(
    focus: crate::ui::MapFocus,
    snapshot: &MapSnapshot,
    primary: Option<ItemKey>,
) -> Option<DVec3> {
    let at = |key| snapshot.item(key).map(|i| i.position_ly);
    match focus {
        crate::ui::MapFocus::Free => None,
        crate::ui::MapFocus::Observer => snapshot.observer().map(|o| o.position_ly),
        crate::ui::MapFocus::Primary(_) => primary.and_then(at),
        crate::ui::MapFocus::Item(key) => at(key),
    }
}

/// Turn the camera with the reference line, when the focus asks for it, and say whether it
/// moved.
///
/// The line runs from the primary's center to the ship's, and the local frame holds the camera
/// against it: the ship keeps its place on screen and the rest of the system goes round. What
/// is written is the *change* in the line's bearing, so the camera's azimuth stays the one
/// number a drag, a ray and a label are all measured in.
pub fn spin(view: &mut crate::ui::MapView, snapshot: &MapSnapshot, primary: Option<ItemKey>)
    -> bool {
    let line = match view.focus {
        crate::ui::MapFocus::Primary(crate::ui::Frame::Local) => reference_line(snapshot, primary),
        _ => None,
    };
    let Some(bearing) = line.and_then(|line| view.plane.bearing(line)) else {
        // Nothing to hold onto: the camera stays where it is and starts again from whatever
        // the line reads next.
        view.bearing = None;
        return false;
    };
    let turned = match view.bearing {
        // The difference, with no mending at the seam: an azimuth is wrapped into its own
        // circle, so a step that reads as a whole turn backwards lands in the same place as
        // the hair's turn it really is.
        Some(was) => {
            view.orbit.turn(bearing - was, 0.0);
            bearing != was
        }
        None => false,
    };
    view.bearing = Some(bearing);
    turned
}

/// The primary's center to the ship's, in light-years, or `None` when the snapshot is missing
/// either end.
fn reference_line(snapshot: &MapSnapshot, primary: Option<ItemKey>) -> Option<DVec3> {
    let ship = snapshot.observer()?.position_ly;
    let at = snapshot.item(primary?)?.position_ly;
    Some(ship - at)
}

/// How close a ring or a shell comes to the camera, in render units.
///
/// A tube's width is set by the nearest part of its mesh. Sized against the far edge, a shell
/// spanning 633 to 181 000 units gets a tube thicker than its inner edge's distance, which
/// puts the camera inside it. Floored at the camera's clearance over the reference plane, so a
/// belt seen from within is a hairline rather than nothing.
fn nearest_reach(center_at: f32, annulus: em_map::Annulus, standoff: f32) -> f32 {
    let nearest = if center_at < annulus.inner {
        annulus.inner - center_at
    } else if center_at > annulus.outer {
        center_at - annulus.outer
    } else {
        0.0
    };
    nearest.max(standoff * em_map::camera::ELEVATION_FLOOR.sin() as f32)
}

/// The outline mesh for a population, normalized so its outer edge is one unit.
///
/// Built in simulation axes and converted here, so the pole arrives on `+Y` — what
/// [`annulus_transform`] rotates from. The shape depends only on the radius ratio and the
/// half-angle, so only its scale changes with zoom.
fn annulus_mesh(annulus: em_map::Annulus) -> Mesh {
    let curves: Vec<Vec<Vec3>> = em_map::outline::torus(DVec3::ZERO, DVec3::Z, annulus.unit())
        .into_iter()
        .map(|curve| curve.into_iter().map(render).collect())
        .collect();
    wire_mesh::tube_curves(&curves, BASE_TUBE_RADIUS, 4, 0.8)
}

/// A population sits at its own center, in its own plane, at its own size.
fn annulus_transform(placement: &Placement, annulus: em_map::Annulus) -> Transform {
    Transform {
        translation: at_of(placement),
        rotation: Quat::from_rotation_arc(Vec3::Y, render(placement.pole.as_dvec3()).normalize()),
        scale: Vec3::splat(annulus.outer.max(f32::MIN_POSITIVE)),
    }
}

fn render(v: DVec3) -> Vec3 {
    sim_to_render(v).as_vec3()
}

fn at_of(placement: &Placement) -> Vec3 {
    render(placement.at.as_dvec3())
}

/// A body is a sphere at its own size above [`Viewport::point_px`] across, and a mark below.
///
/// The threshold is the surface's size and not the mark's, so a light body steps *down* to
/// its mark at the crossover. The alternative lets a rock stay a sphere down to three pixels,
/// which is the illegible case the level of detail exists for.
///
/// A ship is always a dot: its hull size is not what anyone reads off a map.
fn form_of(placement: &Placement, view: Viewport) -> Form {
    match placement.kind {
        // This ship is drawn as one of them: a circle around a dot at the same place is a
        // white outline on somebody else's mark.
        ItemKind::Ship | ItemKind::Observer => Form::Dot,
        _ if view.rad_per_px > 0.0
            && 2.0 * placement.angular_radius / view.rad_per_px > view.point_px =>
        {
            Form::Sphere
        }
        _ => Form::Circle,
    }
}

/// The render-unit radius of a mark drawn `mark_px` across at `distance`.
fn point_radius(distance: f32, rad_per_px: f32, mark_px: f32) -> f32 {
    (distance * rad_per_px * mark_px * 0.5).max(f32::MIN_POSITIVE)
}

/// A mark faces the eye, which is the render origin because every transform here is
/// camera-relative. The mesh lies in the XZ plane, so its +Y is what points back.
fn face_camera(at: Vec3) -> Quat {
    match at.try_normalize() {
        Some(away) => Quat::from_rotation_arc(Vec3::Y, -away),
        None => Quat::IDENTITY,
    }
}

fn item_transform(placement: &Placement, view: Viewport) -> Transform {
    let at = at_of(placement);
    match form_of(placement, view) {
        Form::Sphere => Transform {
            translation: at,
            rotation: Quat::from_rotation_arc(
                Vec3::Y,
                render(placement.pole.as_dvec3()).normalize(),
            ),
            scale: Vec3::splat(placement.radius),
        },
        // Flat, facing the eye, at the size this one's mass earns.
        Form::Circle | Form::Dot => Transform {
            translation: at,
            rotation: face_camera(at),
            scale: Vec3::splat(point_radius(at.length(), view.rad_per_px, view.mark_px(placement))),
        },
    }
}

/// The mesh a form is drawn with, and the cap its tube is sized under.
fn mesh_for<'a>(form: Form, placement: &Placement, map: &'a Map, view: Viewport)
    -> (&'a Handle<Mesh>, f32) {
    match form {
        Form::Sphere => (&map.sphere, SPHERE_TUBE_FRACTION),
        Form::Circle => (&map.point, view.point_tube_fraction(view.mark_px(placement))),
        Form::Dot => (&map.dot, DOT_TUBE_FRACTION),
    }
}

/// How many dashes a drop wants, so each is [`DASH_PX`] long on screen. The mesh lays `n`
/// dashes and `n - 1` equal gaps over a unit height, so `n = (h / d + 1) / 2`.
fn dash_count(height: f32, distance: f32, rad_per_px: f32) -> usize {
    let dash = distance * rad_per_px * DASH_PX;
    if !(dash > 0.0) || !height.is_finite() {
        return 1;
    }
    (((height / dash + 1.0) * 0.5).round() as i64).clamp(1, MAX_DASHES as i64) as usize
}

/// The dashed line is a unit height along `+Y`, so it is scaled to the drop and turned onto it.
fn drop_transform(placement: &Placement) -> Transform {
    let foot = render(placement.foot.as_dvec3());
    let span = at_of(placement) - foot;
    let length = span.length();
    Transform {
        translation: foot,
        rotation: match length > f32::EPSILON {
            true => Quat::from_rotation_arc(Vec3::Y, span / length),
            false => Quat::IDENTITY,
        },
        scale: Vec3::new(1.0, length, 1.0),
    }
}

/// A unit ring in the XZ plane, turned onto the reference plane and grown to its radius.
fn ring_transform(frame: &MapFrame, radius: f32) -> Transform {
    let normal = render(frame.plane_normal.as_dvec3()).normalize();
    Transform {
        // The ship, not the focus: the scale is the observer's. See `MapFrame::rings_at`.
        translation: render(frame.rings_at.as_dvec3()),
        rotation: Quat::from_rotation_arc(Vec3::Y, normal),
        scale: Vec3::splat(radius.max(f32::MIN_POSITIVE)),
    }
}

/// The tube radius a mesh wants, in its own local units, to hold `width_px` on screen.
///
/// The shader displaces along normals in local space, so the world width is `scale * target`
/// and the answer is the wanted world width over the scale. Shared by the spawn and the
/// per-frame update: a spawn that took the material's default and let the next frame correct
/// it put the camera inside a spoke's tube for that frame.
pub fn tube_target(scale: f32, rad_per_px: f32, distance: f32, max_fraction: f32,
    width_px: f32) -> f32 {
    let world = (distance * rad_per_px * width_px).max(f32::MIN_POSITIVE);
    match scale > f32::MIN_POSITIVE {
        true => (world / scale).min(max_fraction),
        false => BASE_TUBE_RADIUS,
    }
}

#[allow(clippy::too_many_arguments)]
fn set_thickness(
    materials: &mut Assets<BodyWireframeMaterial>,
    material: &MeshMaterial3d<BodyWireframeMaterial>,
    scale: f32,
    rad_per_px: f32,
    distance: f32,
    max_fraction: f32,
    width_px: f32,
) {
    let Some(mut asset) = materials.get_mut(&material.0) else { return };
    asset.target_tube_radius = tube_target(scale, rad_per_px, distance, max_fraction, width_px);
}

fn spawn_scene(
    commands: &mut Commands,
    map: &Map,
    frame: &MapFrame,
    standoff: f32,
    view: Viewport,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<BodyWireframeMaterial>,
) {
    let rad_per_px = view.rad_per_px;
    let layer = RenderLayers::layer(MAP_LAYER);

    for (index, ring) in frame.rings.iter().enumerate() {
        let at = ring_transform(frame, ring.radius);
        let target = tube_target(ring.radius, rad_per_px,
            at.translation.length().max(ring.radius), LINE_TUBE_FRACTION, SCALE_PX);
        commands.spawn((
            Mesh3d(map.ring.clone()),
            MeshMaterial3d(materials.add(line_material(RING, target, SCALE_COLOR_SCALE))),
            at,
            NoFrustumCulling,
            layer.clone(),
            MapDrawn,
            MapRingOf(index),
        ));
    }

    let reach = standoff * SPOKE_REACH;
    commands.spawn((
        Mesh3d(map.spokes.clone()),
        MeshMaterial3d(materials.add(line_material(
            SPOKE,
            tube_target(reach, rad_per_px, standoff, LINE_TUBE_FRACTION, SCALE_PX),
            SCALE_COLOR_SCALE,
        ))),
        ring_transform(frame, reach),
        NoFrustumCulling,
        layer.clone(),
        MapDrawn,
        MapSpokes,
    ));

    for placement in &frame.placements {
        let at = item_transform(placement, view);
        let (mesh, fraction) = mesh_for(form_of(placement, view), placement, map, view);
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(line_material(
                color_of(placement.kind),
                tube_target(at.scale.max_element(), view.rad_per_px, at.translation.length(),
                    fraction, LINE_PX),
                LINE_COLOR_SCALE,
            ))),
            at,
            NoFrustumCulling,
            layer.clone(),
            MapDrawn,
            MapItemOf(placement.key),
        ));
        if let Some(annulus) = placement.annulus {
            let at = annulus_transform(placement, annulus);
            let target = tube_target(annulus.outer, rad_per_px,
                nearest_reach(at.translation.length(), annulus, standoff), LINE_TUBE_FRACTION,
                LINE_PX);
            commands.spawn((
                Mesh3d(meshes.add(annulus_mesh(annulus))),
                MeshMaterial3d(materials.add(line_material(POPULATION, target,
                    LINE_COLOR_SCALE))),
                at,
                NoFrustumCulling,
                layer.clone(),
                MapDrawn,
                MapAnnulusOf(placement.key),
            ));
        }
        if placement.has_drop_line() {
            let at = drop_transform(placement);
            let target = tube_target(1.0, rad_per_px, at.translation.length(),
                LINE_TUBE_FRACTION, SCALE_PX);
            let dashes = dash_count(at.scale.y, at.translation.length(), rad_per_px);
            commands.spawn((
                Mesh3d(map.drops[dashes - 1].clone()),
                MeshMaterial3d(materials.add(line_material(DROP, target, SCALE_COLOR_SCALE))),
                at,
                NoFrustumCulling,
                layer.clone(),
                MapDrawn,
                MapDropOf(placement.key),
            ));
        }
    }
}

fn line_material(color: Color, target_tube_radius: f32, color_scale: f32)
    -> BodyWireframeMaterial {
    let rgba = color.to_linear();
    BodyWireframeMaterial {
        base_color: LinearRgba::new(
            rgba.red * color_scale,
            rgba.green * color_scale,
            rgba.blue * color_scale,
            1.0,
        ),
        emission_strength: LINE_EMISSION,
        // Set here rather than corrected next frame. See [`tube_target`].
        target_tube_radius,
        // Unlit: with no suns the shader's day/night factor is one.
        num_suns: 0,
        ..default()
    }
}

// The interface's own palette. `18-ui-style.md`: one source, converted at the edge.
const RING: Color = em_ui::vfd::TEXT_DIM;
const SPOKE: Color = em_ui::vfd::TEXT_DIM;
const DROP: Color = em_ui::vfd::BUTTON_BORDER;
const POPULATION: Color = em_ui::vfd::TEXT_DIM;

fn color_of(kind: ItemKind) -> Color {
    match kind {
        ItemKind::Star => em_ui::vfd::TEXT,
        ItemKind::Planet | ItemKind::Moon | ItemKind::Minor => em_ui::vfd::BUTTON_BORDER,
        ItemKind::Population => em_ui::vfd::TEXT_DIM,
        // Amber against the green: color is the one channel a map has that a list does not.
        // This ship included — it is a craft like the others, and the palette has no white in
        // it. What says which one is the reader's is the rings, which are drawn from it.
        ItemKind::Ship | ItemKind::Station | ItemKind::Observer => em_ui::vfd::AMBER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::{Frame, MapFocus};
    use em_map::{ItemKey, ItemKind, MapItem, MapSnapshot};
    use glam::DVec3;

    fn snapshot() -> MapSnapshot {
        MapSnapshot::observed(0.0, vec![
            MapItem::body(ItemKey::from_name("observer"), "Anonymous Ship", ItemKind::Observer,
                DVec3::new(1.0, 2.0, 3.0), 100.0, DVec3::Z),
            MapItem::body(ItemKey::from_id("star", 7), "Sol", ItemKind::Star,
                DVec3::new(4.0, 5.0, 6.0), 7.0e8, DVec3::Z),
        ])
    }

    /// The observer needs a focus state of its own. `None` has to mean "leave the camera
    /// alone" for a pan, so the ship cannot share it.
    #[test]
    fn centering_on_the_ship_finds_the_ship() {
        let snapshot = snapshot();
        assert_eq!(
            focus_position(MapFocus::Observer, &snapshot, None),
            Some(DVec3::new(1.0, 2.0, 3.0)),
        );
        assert_eq!(
            focus_position(MapFocus::Item(ItemKey::from_id("star", 7)), &snapshot, None),
            Some(DVec3::new(4.0, 5.0, 6.0)),
        );
    }

    /// A pan starts from the interface's own `focus_ly`, so `place` has to write the followed
    /// position back into it and not into a copy.
    #[test]
    fn a_pan_begins_where_the_camera_actually_is() {
        let snapshot = snapshot();
        let ship = DVec3::new(1.0, 2.0, 3.0);
        let mut orbit = em_map::Orbit::framing(DVec3::ZERO, em_map::snapshot::M_PER_AU * 40.0);
        assert_eq!(orbit.focus_ly, DVec3::ZERO, "premise: it starts at the origin");

        follow(MapFocus::Observer, &snapshot, None, &mut orbit.focus_ly);
        assert_eq!(orbit.focus_ly, ship, "following did not reach the interface's copy");

        // Now the drag. A small pan has to leave the camera near the ship, not near zero.
        orbit.pan(em_map::Plane::Ecliptic, 0.05, 0.0);
        let moved = orbit.focus_ly.distance(ship);
        assert!(moved > 0.0, "the pan moved nothing");
        assert!(
            moved < 0.25 * ship.length(),
            "the pan threw the camera {moved} ly from the ship, which is the jump",
        );
    }

    /// Following writes only on a change, so a resource half the interface watches is not
    /// marked dirty every frame.
    #[test]
    fn following_the_same_place_twice_writes_once() {
        let snapshot = snapshot();
        let mut at = DVec3::ZERO;
        assert!(follow(MapFocus::Observer, &snapshot, None, &mut at), "the first call should move it");
        assert!(!follow(MapFocus::Observer, &snapshot, None, &mut at), "the second should not");
        assert!(!follow(MapFocus::Free, &snapshot, None, &mut at), "free never moves it");
    }

    /// **Every mark is drawn in the palette, and the palette has no white in it.** A color
    /// written straight into a match arm is one the interface cannot re-theme and one nothing
    /// else agrees with.
    #[test]
    fn nothing_is_drawn_outside_the_palette() {
        let palette = [
            em_ui::vfd::TEXT,
            em_ui::vfd::TEXT_DIM,
            em_ui::vfd::BUTTON_BORDER,
            em_ui::vfd::AMBER,
        ];
        for kind in [
            ItemKind::Star,
            ItemKind::Planet,
            ItemKind::Moon,
            ItemKind::Minor,
            ItemKind::Population,
            ItemKind::Ship,
            ItemKind::Station,
            ItemKind::Observer,
        ] {
            assert!(palette.contains(&color_of(kind)), "{kind:?} is drawn off the palette");
        }
    }

    /// **The primary is a mode, not the body it resolves to today.** Centering on it and
    /// centering on Earth are the same picture while the ship is at Earth, and different
    /// pictures the moment it is not.
    #[test]
    fn the_primary_is_whatever_the_map_is_told_holds_the_ship() {
        let snapshot = snapshot();
        let star = ItemKey::from_id("star", 7);
        assert_eq!(
            focus_position(MapFocus::Primary(Frame::Fixed), &snapshot, Some(star)),
            Some(DVec3::new(4.0, 5.0, 6.0)),
        );
        // Nothing holding it, and a body that is no longer in the snapshot: both leave the
        // camera where it is rather than moving it to nowhere.
        assert_eq!(focus_position(MapFocus::Primary(Frame::Fixed), &snapshot, None), None);
        assert_eq!(focus_position(MapFocus::Primary(Frame::Fixed), &snapshot, Some(ItemKey(999))), None);
    }

    /// A ship at `bearing` radians round its primary, a light-year out.
    fn ship_at(bearing: f64) -> MapSnapshot {
        let star = DVec3::new(4.0, 5.0, 6.0);
        MapSnapshot::observed(0.0, vec![
            MapItem::body(ItemKey::from_name("observer"), "Anonymous Ship", ItemKind::Observer,
                star + DVec3::new(bearing.cos(), bearing.sin(), 0.0), 100.0, DVec3::Z),
            MapItem::body(ItemKey::from_id("star", 7), "Sol", ItemKind::Star, star, 7.0e8,
                DVec3::Z),
        ])
    }

    /// The shortest turn from `a` to `b`. An azimuth is stored wrapped into a circle, so
    /// subtracting two of them is not the turn between them.
    fn apart(a: f64, b: f64) -> f64 {
        let by = (b - a).rem_euclid(std::f64::consts::TAU);
        match by > std::f64::consts::PI {
            true => by - std::f64::consts::TAU,
            false => by,
        }
    }

    fn locked_on(frame: Frame) -> crate::ui::MapView {
        crate::ui::MapView {
            focus: MapFocus::Primary(frame),
            plane: em_map::Plane::Ecliptic,
            ..Default::default()
        }
    }

    /// **The local frame holds the camera against the reference line.** A quarter of an orbit
    /// turns the camera a quarter, so the ship keeps its place on screen and the system goes
    /// round it. The fixed frame turns nothing and the ship is what moves.
    #[test]
    fn the_local_frame_turns_with_the_ship() {
        use std::f64::consts::FRAC_PI_2;
        let star = Some(ItemKey::from_id("star", 7));
        for (frame, expected) in [(Frame::Local, FRAC_PI_2), (Frame::Fixed, 0.0)] {
            let mut view = locked_on(frame);
            let was = view.orbit.azimuth;
            // The first call has nothing to measure against, so it turns nothing.
            assert!(!spin(&mut view, &ship_at(0.0), star), "{frame:?} turned on the first frame");
            assert_eq!(view.orbit.azimuth, was);

            spin(&mut view, &ship_at(FRAC_PI_2), star);
            let by = apart(was, view.orbit.azimuth);
            assert!((by - expected).abs() < 1.0e-9, "{frame:?} turned by {by}, wanted {expected}");
        }
    }

    /// **A whole orbit brings the camera back to where it started.** The step from just under
    /// +pi to just over it reads as a turn backwards round the whole circle, and the camera
    /// lands in the same place either way only because it is turned *by* the change rather
    /// than set *to* the bearing.
    #[test]
    fn a_whole_orbit_brings_the_camera_back() {
        use std::f64::consts::TAU;
        let star = Some(ItemKey::from_id("star", 7));
        for way in [1.0, -1.0] {
            let mut view = locked_on(Frame::Local);
            view.orbit.turn(1.234, 0.0);
            spin(&mut view, &ship_at(0.0), star);
            let start = view.orbit.azimuth;
            for step in 1..=16 {
                spin(&mut view, &ship_at(way * step as f64 * TAU / 16.0), star);
            }
            let off = apart(start, view.orbit.azimuth);
            assert!(off.abs() < 1.0e-9, "an orbit {way} came back {off} out");
        }
    }

    /// Leaving the frame leaves the camera where it is, and coming back starts again from
    /// wherever the line is then — neither is a jump.
    #[test]
    fn a_frame_is_left_and_entered_without_a_jump() {
        let star = Some(ItemKey::from_id("star", 7));
        let mut view = locked_on(Frame::Local);
        spin(&mut view, &ship_at(0.0), star);
        spin(&mut view, &ship_at(1.0), star);
        let held = view.orbit.azimuth;

        view.focus = MapFocus::Primary(Frame::Fixed);
        assert!(!spin(&mut view, &ship_at(2.0), star), "the fixed frame turned the camera");
        assert_eq!(view.orbit.azimuth, held);
        assert_eq!(view.bearing, None, "nothing is being tracked");

        // Back, from a line that has moved a long way since.
        view.focus = MapFocus::Primary(Frame::Local);
        assert!(!spin(&mut view, &ship_at(3.0), star), "coming back turned the camera");
        assert_eq!(view.orbit.azimuth, held);
    }

    /// Nothing to hold onto: a snapshot without one end of the line leaves the camera alone
    /// rather than turning it to an arbitrary bearing.
    #[test]
    fn a_line_with_one_end_missing_turns_nothing() {
        let mut view = locked_on(Frame::Local);
        spin(&mut view, &ship_at(0.0), Some(ItemKey::from_id("star", 7)));
        let held = view.orbit.azimuth;
        assert!(!spin(&mut view, &ship_at(1.0), None), "no primary, no line");
        assert!(!spin(&mut view, &MapSnapshot::observed(0.0, Vec::new()),
            Some(ItemKey::from_id("star", 7))), "no ship, no line");
        assert_eq!(view.orbit.azimuth, held);
        assert_eq!(view.bearing, None);
    }

    /// A pan has to be able to leave the camera where it is.
    #[test]
    fn a_free_camera_is_left_where_it_was_put() {
        assert_eq!(focus_position(MapFocus::Free, &snapshot(), None), None);
    }

    /// A map opens on the observer, not on the world origin.
    #[test]
    fn a_map_opens_on_the_ship() {
        assert_eq!(crate::ui::MapView::default().focus, MapFocus::Observer);
    }

    /// Following something out of range stops following it rather than moving the view.
    #[test]
    fn following_something_that_is_gone_holds_still() {
        assert_eq!(focus_position(MapFocus::Item(ItemKey(999)), &snapshot(), None), None);
        assert_eq!(
            focus_position(MapFocus::Observer, &MapSnapshot::observed(0.0, Vec::new()), None),
            None,
        );
    }

    /// No line is thicker than the camera's clearance over the plane.
    ///
    /// `em_map::camera::ELEVATION_FLOOR` holds the camera off the plane and every line is a
    /// tube with a width of its own. A tube wider than that clearance contains the camera, and
    /// the inside of a tube is opaque. Checked on the family that is scaled hardest.
    #[test]
    fn a_line_is_never_thicker_than_the_camera_clears_the_plane() {
        let floor = em_map::camera::ELEVATION_FLOOR.sin() as f32;
        for standoff in [1.0e-3f32, 1.0, 40.0, 1.0e3, 1.0e5] {
            for height in [64.0f32, 410.0, 2160.0] {
                let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / height;
                let reach = standoff * SPOKE_REACH;
                let world =
                    reach * tube_target(reach, rad_per_px, standoff, LINE_TUBE_FRACTION, SCALE_PX);
                let clearance = standoff * floor;
                assert!(
                    world < clearance,
                    "spokes at stand-off {standoff:e}, {height} px: tube {world:e} against a \
                     clearance of {clearance:e}",
                );
            }
        }
    }

    /// A 410-pixel surface, which is about what the panel's map area is.
    fn viewport() -> Viewport {
        Viewport::new(410, std::f32::consts::FRAC_PI_4)
    }

    fn body_at(distance: f32, radius: f32) -> Placement {
        kind_at(ItemKind::Planet, distance, radius)
    }

    fn kind_at(kind: ItemKind, distance: f32, radius: f32) -> Placement {
        Placement {
            key: ItemKey::from_name("a body"),
            kind,
            weight: 0.0,
            symbol_scale: 1.0,
            label: "a body".into(),
            at: glam::Vec3::new(0.0, distance, 0.0),
            foot: glam::Vec3::new(0.0, distance, 0.0),
            radius,
            angular_radius: radius / distance,
            annulus: None,
            pole: glam::Vec3::Z,
        }
    }

    /// Nothing changes size at the crossover. Reading the one number as a radius in one place
    /// and a diameter in the other is a factor of two, with nothing in the types to catch it.
    #[test]
    fn a_body_holds_its_size_where_it_stops_being_a_sphere() {
        let view = viewport();
        let distance = 40.0;
        // A body sitting exactly on the threshold, and one a hair under it.
        let on = view.point_px * 0.5 * view.rad_per_px * distance;
        assert_eq!(form_of(&body_at(distance, on * 1.01), view), Form::Sphere, "just over");
        let under = body_at(distance, on * 0.99);
        assert_eq!(form_of(&under, view), Form::Circle, "just under");

        let drawn = item_transform(&under, view).scale.x;
        assert!(
            (drawn / on - 1.0).abs() < 0.05,
            "a circle of {drawn} where the sphere it replaced was {on}",
        );
    }

    /// A lighter thing gets a smaller mark, and never one too small to draw.
    #[test]
    fn a_lighter_mark_is_smaller_but_never_vanishes() {
        let view = viewport();
        let marked = |scale: f32| {
            let mut placement = body_at(40.0, 0.0);
            placement.symbol_scale = scale;
            view.mark_px(&placement)
        };
        assert!(marked(1.0) > marked(0.5), "half the scale should draw smaller");
        assert!(marked(0.5) >= POINT_FLOOR_PX, "and never under a shape's worth of pixels");
        assert_eq!(marked(1.0), view.point_px, "and a full weight is the surface's own size");
        for scale in [em_map::weight::MIN_SCALE, 0.0, 1.0e-9] {
            assert!(marked(scale) >= POINT_FLOOR_PX, "{scale} drew {}", marked(scale));
        }
        // And the line thickens to match, or a small mark is a hairline ring nobody can see.
        assert!(
            view.point_tube_fraction(marked(0.5)) > view.point_tube_fraction(marked(1.0)),
            "a smaller mark wants a thicker line, as a fraction of itself",
        );
    }

    /// The crossover is the surface's size for everyone, so a light body steps down to its
    /// mark. Stepping up would be a body growing as it recedes.
    #[test]
    fn a_light_body_steps_down_at_the_crossover_and_never_up() {
        let view = viewport();
        let distance = 40.0;
        let on = view.point_px * 0.5 * view.rad_per_px * distance;
        for scale in [1.0f32, 0.5, em_map::weight::MIN_SCALE] {
            let mut under = body_at(distance, on * 0.99);
            under.symbol_scale = scale;
            assert_eq!(form_of(&under, view), Form::Circle, "{scale} should be a mark");
            let drawn = item_transform(&under, view).scale.x;
            assert!(
                drawn <= on * 1.001,
                "a mark at scale {scale} drew {drawn}, bigger than the {on} sphere it replaced",
            );
        }
    }

    /// A mark is the same share of every surface. One texture serves a 190-point corner and a
    /// panel several times that.
    #[test]
    fn a_symbol_is_a_share_of_the_view_above_its_floor() {
        let fov = std::f32::consts::FRAC_PI_4;
        for height in [200u32, 410, 1080, 2160] {
            let share = Viewport::new(height, fov).point_px / height as f32;
            assert!(
                (share / POINT_FRACTION - 1.0).abs() < 1.0e-5,
                "a {height}-pixel surface drew a symbol at {share} of itself",
            );
        }
        // And below it the floor holds, which is where a ring has no inside left and the
        // honest answer is a dot.
        for height in [0u32, 1, 64, 159] {
            let px = Viewport::new(height, fov).point_px;
            assert!((px - POINT_FLOOR_PX).abs() < 1.0e-6, "{height} px drew a symbol of {px}");
        }
        assert!(POINT_FLOOR_PX <= 2.0 * LINE_PX, "the floor is twice a line and no more");
    }

    /// A disc comes out the size the transform says. Its vertices share one normal, so the
    /// displacement translates it; only asking for the base radius makes that zero.
    #[test]
    fn a_dot_is_never_displaced() {
        let view = viewport();
        for distance in [1.0e-3f32, 1.0, 40.0, 1.0e5] {
            let scale = point_radius(distance, view.rad_per_px, view.point_px);
            let target =
                tube_target(scale, view.rad_per_px, distance, DOT_TUBE_FRACTION, LINE_PX);
            assert!(
                (target - BASE_TUBE_RADIUS).abs() < 1.0e-9,
                "at {distance:e} the disc would shift by {}",
                target - BASE_TUBE_RADIUS,
            );
        }
    }

    /// A ship is a mark at every zoom. Given a planet's radius it is still a mark.
    #[test]
    fn a_ship_never_becomes_a_model() {
        let view = viewport();
        // A radius that would fill the view, at a distance that would make anything else a
        // sphere many times over.
        let huge = kind_at(ItemKind::Ship, 1.0, 10.0);
        assert_eq!(form_of(&huge, view), Form::Dot, "a ship is never a sphere");
        assert_eq!(
            form_of(&kind_at(ItemKind::Planet, 1.0, 10.0), view),
            Form::Sphere,
            "and the exemption is the kind, not the numbers",
        );
        // Drawn at the symbol's own size, like every other mark.
        let at = item_transform(&huge, view);
        let px = 2.0 * at.scale.x / (at.translation.length() * view.rad_per_px);
        assert!((px / view.point_px - 1.0).abs() < 1.0e-3, "a ship drew {px} px across");
    }

    /// And it holds that size at every distance, so the far one reads as well as the near.
    #[test]
    fn a_circle_is_the_same_size_wherever_it_is() {
        let view = viewport();
        let want = view.point_px * 0.5;
        for distance in [1.0e-3f32, 1.0, 40.0, 1.0e5] {
            // Radius zero: nothing to draw at its own size, at any zoom.
            let at = item_transform(&body_at(distance, 0.0), view);
            let px = at.scale.x / (at.translation.length() * view.rad_per_px);
            assert!(
                (px / want - 1.0).abs() < 1.0e-3,
                "at {distance:e} the circle came out {px} px across the radius, wanted {want}",
            );
        }
    }

    /// A mark that does not face the eye is an ellipse, and edge-on a line. The eye is the
    /// render origin, because every transform on this layer is camera-relative.
    #[test]
    fn a_circle_faces_the_eye() {
        let view = viewport();
        let places = [
            glam::Vec3::new(0.0, 40.0, 0.0),
            glam::Vec3::new(-3.0, 0.5, 12.0),
            glam::Vec3::new(1.0e4, -2.0e3, 7.0),
            glam::Vec3::Y,
            -glam::Vec3::Y,
        ];
        for place in places {
            let mut placement = body_at(1.0, 0.0);
            placement.at = place;
            placement.foot = place;
            let at = item_transform(&placement, view);
            // The mesh's own normal is +Y; after the rotation it must point back at the eye.
            // Against the transform's own translation, not the placement's: these are two
            // different frames and `item_transform` is where the swizzle happens.
            let normal = at.rotation * Vec3::Y;
            let toward_eye = -at.translation.normalize();
            assert!(
                normal.dot(toward_eye) > 0.999,
                "at {place:?} the circle's normal was {normal:?}, wanted {toward_eye:?}",
            );
        }
        // And nothing blows up for something sitting on the camera.
        assert!(face_camera(Vec3::ZERO).is_finite());
    }

    /// A dash is the same length wherever it is drawn: the count follows the drop, so twice
    /// the drop is twice the dashes and not dashes twice as long.
    #[test]
    fn a_longer_drop_gets_more_dashes_not_longer_ones() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        let distance = 40.0;
        let dash_of = |height: f32| {
            let n = dash_count(height, distance, rad_per_px);
            // The mesh lays `n` dashes and `n - 1` gaps of equal length over the drop.
            height / (2 * n - 1) as f32
        };
        let want = distance * rad_per_px * DASH_PX;
        // Up to the cap, which is about a viewport's worth of line.
        for height in [0.5f32, 2.0, 9.0, 20.0] {
            let dash = dash_of(height);
            assert!(
                (dash / want - 1.0).abs() < 0.5,
                "a drop of {height} drew dashes of {dash} against a wanted {want}",
            );
        }
        assert!(
            dash_count(20.0, distance, rad_per_px) > dash_count(2.0, distance, rad_per_px),
            "a longer drop should have gained dashes",
        );
        // Past it the dashes do stretch, and that is the cap rather than the rule: a drop that
        // long runs several viewports off the screen and is not being read as dashes anyway.
        assert_eq!(dash_count(150.0, distance, rad_per_px), MAX_DASHES);
    }

    /// And it stays inside the meshes that exist, whatever it is handed.
    #[test]
    fn a_dash_count_is_always_a_mesh_there_is() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        for height in [0.0f32, -1.0, 1.0e-9, 1.0e9, f32::INFINITY, f32::NAN] {
            for distance in [0.0f32, 1.0e-6, 40.0, 1.0e5] {
                let n = dash_count(height, distance, rad_per_px);
                assert!((1..=MAX_DASHES).contains(&n), "{height} at {distance} gave {n}");
            }
        }
    }

    /// A belt's tube never reaches the camera. Anything spanning a range of distances has to
    /// be sized by its near side; the Oort cloud runs from 633 render units to 181 000.
    #[test]
    fn an_annulus_never_swallows_the_camera() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        let cases = [
            // The Oort cloud, as the solar system's generator actually produces it.
            (em_map::Annulus { inner: 6.33e2, outer: 1.81e5, half_angle_rad: 1.57 }, 59.2, 60.0),
            // The asteroid belt, seen from outside and from within.
            (em_map::Annulus { inner: 2.1, outer: 3.3, half_angle_rad: 0.2 }, 59.2, 60.0),
            (em_map::Annulus { inner: 2.1, outer: 3.3, half_angle_rad: 0.2 }, 0.5, 1.0),
            // And a camera sitting inside the band itself.
            (em_map::Annulus { inner: 2.1, outer: 3.3, half_angle_rad: 0.2 }, 2.7, 3.0),
        ];
        for (annulus, center_at, standoff) in cases {
            let reach = nearest_reach(center_at, annulus, standoff);
            let world = annulus.outer
                * tube_target(annulus.outer, rad_per_px, reach, LINE_TUBE_FRACTION, LINE_PX);
            assert!(
                world < reach,
                "outer {:e}: tube {world:e} against a reach of {reach:e}",
                annulus.outer,
            );
        }
    }

    /// And the reach is the near edge.
    #[test]
    fn the_reach_is_measured_to_the_near_edge() {
        let shell = em_map::Annulus { inner: 600.0, outer: 1.0e5, half_angle_rad: 1.57 };
        // Inside the cavity: the near edge is the inner one.
        assert!((nearest_reach(60.0, shell, 60.0) - 540.0).abs() < 1.0);
        // Outside it altogether: the near edge is the outer one.
        assert!((nearest_reach(1.2e5, shell, 60.0) - 2.0e4).abs() < 1.0);
        // Within the band, floored so a belt seen from inside is still a hairline.
        assert!(nearest_reach(1000.0, shell, 60.0) > 0.0);
    }

    /// And the material's default is not good enough, so a spawn cannot take it and wait for
    /// the next frame to correct it.
    #[test]
    fn the_default_tube_would_swallow_the_camera() {
        let standoff = 40.0f32;
        let clearance = standoff * em_map::camera::ELEVATION_FLOOR.sin() as f32;
        let world = standoff * SPOKE_REACH * BASE_TUBE_RADIUS;
        assert!(
            world > clearance,
            "the default is safe after all, so this test is no longer about anything",
        );
    }

    /// A line holds its screen width however hard its mesh is scaled, which is why the
    /// thickness is a per-entity dial rather than baked in.
    #[test]
    fn a_line_holds_its_width_on_screen_across_the_scales() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        let width_px = |scale: f32, distance: f32| {
            let world =
                scale * tube_target(scale, rad_per_px, distance, LINE_TUBE_FRACTION, SCALE_PX);
            world / distance / rad_per_px
        };
        for scale in [1.0f32, 1.0e2, 1.0e4] {
            let px = width_px(scale, scale);
            assert!((px - SCALE_PX).abs() < 1.0e-3, "{scale:e} units drew {px} px");
        }
    }
}
