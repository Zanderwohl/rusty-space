//! The map's camera, its render target, and the entities it draws.
//!
//! A second `Camera3d` on its own [`MAP_LAYER`], rendering into an [`Image`] that egui shows.
//! Real geometry with real depth, not a projection painted by egui — which is what lets it
//! reuse the wireframe spheres the other product draws bodies with.
//!
//! **One camera, one image, and both surfaces share it.** Map transforms are camera-relative,
//! so an entity belongs to exactly one camera: two independently aimed views would need two
//! sets of them. The panel is the map and the minimap is the map when the panel is closed, so
//! there is never a second framing to want.
//!
//! The camera is **not** the sky's. No `Hdr`, no bloom, no tone map: the sky is a photograph
//! and is metered like one, and a diagram is not. The wireframe shader's emissive range is
//! aimed at the display instead — see [`LINE_COLOR_SCALE`].

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

/// The map's own layer. Everything else in this client is on layer 0, and the sky's camera
/// keeps that layer, so neither can see the other's entities.
pub const MAP_LAYER: usize = 1;

/// What the target starts at, before a surface has asked for a size.
const INITIAL_SIDE: u32 = 512;

/// egui reports a zero rect on the frame a window opens, and a zero-sized texture is a wgpu
/// validation failure rather than a warning. The upper bound is for a resize drag that runs
/// away on a high-density display.
const MIN_SIDE: u32 = 64;
const MAX_SIDE: u32 = 4096;

/// How wide a line is drawn, in pixels.
///
/// Screen-constant rather than world-constant: a ring scaled to ten thousand render units
/// would scale its own tube to ten thousand as well. The shader's `target_tube_radius` is the
/// dial, and it is set per entity from that entity's own scale.
const LINE_PX: f32 = 1.6;

/// And the reference scale — decade rings, spokes, drop-lines — at half that, in width and in
/// brightness both.
///
/// They are the ruler and not the thing being measured. At equal weight the grid competes with
/// the bodies on it for the same attention, and the bodies are what the map is of.

/// How fat a tube may get, as a fraction of its own unit mesh — and it is per family, because
/// one number cannot serve all three.
///
/// The shader displaces vertices along their normals in the mesh's own space, so this is a
/// proportion of the thing drawn rather than of the screen. A ring scaled to one astronomical
/// unit and seen from forty needs a tube of a tenth of a unit to be a pixel wide; unclamped
/// that is a tenth of the ring, and the twelve spokes of the reference plane came out as
/// twelve solid wedges. A point is the opposite case: it is *meant* to be a blob, and a
/// wireframe ball with hairline tubes is invisible at three pixels across.
const LINE_TUBE_FRACTION: f32 = 0.02;
const SPHERE_TUBE_FRACTION: f32 = 0.06;

/// A circle's line, as a fraction of its own radius.
///
/// The one member of the family that is a *constant* rather than a cap that occasionally
/// binds: both the symbol and its line are fixed pixel sizes, so the distance and the scale in
/// [`tube_target`] cancel and the answer is always this.
const POINT_TUBE_FRACTION: f32 = LINE_PX / (POINT_PX * 0.5);

const SCALE_PX: f32 = LINE_PX * 0.5;

/// How much of the palette color a line is drawn at.
///
/// The shader gives `base_color * (1 + alpha * emission_strength)`, and the alpha is the line
/// weight the mesh carries: 0.6 for grid, 1.0 for an equator. With no tone map in front of it
/// the whole range has to land inside the display, so the base is scaled down and the emission
/// makes up the contrast — 0.45 and 1.2 put a grid line at 0.77 and an equator at 0.99.
const LINE_COLOR_SCALE: f32 = 0.45;
const LINE_EMISSION: f32 = 1.2;

/// The reference scale's share of it. The shader multiplies the base color through, so halving
/// the base halves what reaches the screen.
const SCALE_COLOR_SCALE: f32 = LINE_COLOR_SCALE * 0.5;

/// The near and far planes, as multiples of the stand-off.
///
/// Wide enough that a ship beside the camera and the outermost decade ring are both in the
/// frustum, and no wider — `MAX_RENDER_UNITS` bounds what can be placed at all.
const NEAR_FRACTION: f32 = 1.0e-4;
const FAR_MULTIPLE: f32 = 1.0e6;

/// The map camera's vertical field of view.
///
/// Named rather than defaulted, because the panel casts the cursor's ray with it and a camera
/// and a cursor that disagree about the frustum put the anchor somewhere the pointer is not.
pub const MAP_FOV: f32 = std::f32::consts::FRAC_PI_4;

/// Divisions of a decade ring. Enough that the largest one does not read as a polygon.
const RING_SEGMENTS: u32 = 128;

/// How far the spokes reach, as a multiple of the stand-off.
///
/// **Far past the edge of the view, because a spoke's end should never be visible.** A line
/// that stops inside the frame reads as an object with a tip rather than as a rule running
/// off the picture. Forty stand-offs puts the far end forty times further than the focus, so
/// its tube is a fortieth of a pixel there and it fades out instead of ending.
///
/// Passing the eye is safe, and the elevation floor is what makes it so: the camera clears the
/// plane by `sin(3°)` of the stand-off, which is some sixteen times a line's own half-width.
const SPOKE_REACH: f32 = 40.0;

/// Where a spoke starts, as a fraction of the stand-off — so the hole in the middle stays the
/// same size whatever [`SPOKE_REACH`] is.
const SPOKE_INNER: f32 = 0.02;

/// Radial spokes in the reference plane.
///
/// **Sized to the stand-off, not to the outermost ring.** Every point of a ring is the same
/// distance from the center, so one tube radius is right for all of it; a spoke runs from
/// near the camera out to its rim, and a constant world radius that is a pixel at the far end
/// is eighty at the near one. Scaled to the outermost decade the twelve of them were twelve
/// solid wedges across the top of the view, and the arithmetic for the thickness looked
/// entirely correct — it was answering about the wrong end.
const PLANE_SPOKES: u32 = 12;

/// How long a dash is, in pixels, wherever it is drawn.
///
/// Constant in size rather than constant in count: the mesh is scaled to the drop, so a fixed
/// count gives a tall drop long dashes and a short one short ones — two kinds of line instead
/// of one line at two lengths. The host picks the count per drop to hold this.
const DASH_PX: f32 = 5.0;

/// The most dashes a drop-line is built with. A drop needing more than this is longer than the
/// viewport many times over and its dashes are sub-pixel anyway.
const MAX_DASHES: usize = 48;

/// Apparent **diameter**, in pixels, below which a body is a circle instead of a sphere.
///
/// A diameter, not a radius — the sky's `resolved::RESOLVE_PX` is a radius, and the two
/// numbers do not mean the same thing.
///
/// It is also the diameter the circle is drawn at, and that is the point of having one number:
/// a body shrinks until it reaches this size and then holds it, so nothing jumps at the
/// crossover. Below it a sphere is a dozen sub-pixel tubes drawn over each other, which is
/// both the more expensive thing to draw and the less legible one.
///
/// Twenty rather than the five it started at, because the crossover is where the sphere has to
/// earn its place and at eleven pixels across it had not: the tube cap holds its lines to a
/// third of a pixel, so what it draws is a smudge the circle says better.
const POINT_PX: f32 = 20.0;

/// Divisions of that circle. Sixteen is smooth at twenty pixels and an eighth of the ring
/// mesh — a level of detail that cost more to draw than the sphere would not be one.
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
    /// a resize lands one frame late — the frame after shows the previous texture stretched,
    /// which is a stretch and not a flicker.
    pub wanted: UVec2,
    /// Whether anything is showing the map at all. Nothing is drawn when nothing is looking.
    pub shown: bool,
    pub snapshot: MapSnapshot,
    pub frame: Option<MapFrame>,
    sphere: Handle<Mesh>,
    /// What an unresolved body is drawn with: a circle facing the eye. See [`POINT_PX`].
    point: Handle<Mesh>,
    ring: Handle<Mesh>,
    spokes: Handle<Mesh>,
    /// One drop-line mesh per dash count, indexed from one dash. Built once: every one of them
    /// is a handful of tubes, and picking a handle is cheaper than rebuilding geometry when a
    /// body drifts further off the plane.
    drops: Vec<Handle<Mesh>>,
    /// What is spawned, in order. A rebuild happens only when this stops matching the frame.
    drawn: Vec<ItemKey>,
    rings_drawn: usize,
}

impl Map {
    /// Radians per pixel of the map's own viewport, which is what decides a sphere from a
    /// point and how thick a line is.
    /// Where the reference plane is anchored: the observer, or the camera's focus when a
    /// snapshot has nobody in it.
    pub fn plane_origin_ly(&self, focus_ly: DVec3) -> DVec3 {
        self.snapshot.observer().map_or(focus_ly, |o| o.position_ly)
    }

    pub fn radians_per_pixel(&self, fov_y: f32) -> f32 {
        match self.size.y {
            0 => 0.0,
            height => 2.0 * (fov_y * 0.5).tan() / height as f32,
        }
    }
}

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(em_render::body_material::BodyWireframeMaterialPlugin)
            .add_systems(Startup, setup)
            .add_systems(Update, survey.in_set(Stage::Act))
            .add_systems(Update, (resize, place).chain().in_set(Stage::Scene));
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
        frame: None,
        sphere: meshes.add(wire_mesh::generate_latlon_sphere(&[], BASE_TUBE_RADIUS, 4)),
        point: meshes.add(wire_mesh::ring_tube(POINT_SEGMENTS, BASE_TUBE_RADIUS, 4, 1.0)),
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
        // **The target, which is the whole point, and a component of its own in Bevy 0.19
        // rather than a field on `Camera`.** Left off, this camera renders over the primary
        // window instead, on a layer with nothing on it, and clears the frame to black — the
        // sky, the readout and every panel with it. A black window is what a missing target
        // looks like, and nothing in the log says so.
        RenderTarget::Image(map_image.into()),
        Camera {
            // Before the window camera, because the window camera's frame shows what this one
            // drew: one frame behind otherwise.
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..default()
        },
        Projection::Perspective(PerspectiveProjection { fov: MAP_FOV, ..default() }),
        // No `Hdr`, no bloom, and no tone map. See the module doc.
        Tonemapping::None,
        Transform::default(),
    ));
}

fn target_image(size: UVec2) -> Image {
    let mut image = Image::new_target_texture(
        size.x.max(MIN_SIDE),
        size.y.max(MIN_SIDE),
        // An 8-bit sRGB target, so egui samples it and gets back what was drawn. A float
        // target is stored linear and comes out of `ui.image` looking wrong in a way that
        // reads as a shading bug. Named outright: `bevy_default()` is deprecated, and this is
        // the format it named.
        TextureFormat::Rgba8UnormSrgb,
        None,
    );
    image.asset_usage = RenderAssetUsages::RENDER_WORLD;
    // `new_target_texture` sets TEXTURE_BINDING | COPY_DST | RENDER_ATTACHMENT, and resizing
    // one needs COPY_SRC as well: `Image::resize` copies the old contents into the new texture
    // so a resize does not flash. Without it the first resize is a wgpu validation failure
    // that takes the whole application down — `copy_image_on_resize`, several seconds after
    // the only thing that caused it.
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
    map.snapshot = match ui.map.source {
        Source::Observed => crate::map_source::observed(&game.0, &bodies, &uplink, eye.at_ly),
        #[cfg(feature = "godview")]
        Source::God => crate::map_source::coordinate(&game.0, eye.at_ly),
    };
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

    // Follow the selection rather than a remembered position: Saturn moves, and a map that
    // centered on where it was is a map that drifts off it over an afternoon.
    //
    // **Written back, not applied to a copy.** The interface's own `focus_ly` is where every
    // action that moves the camera starts from, so leaving it stale means a pan begins
    // somewhere the camera has not been since the map opened — the world origin, by default.
    // That was one jarring jump on the first frame of the first drag and smooth after it.
    follow(ui.map.focus, &map.snapshot, &mut ui.map.orbit.focus_ly);
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
    let rad_per_px = map.radians_per_pixel(fov_y);

    let wanted: Vec<ItemKey> = frame.placements.iter().map(|p| p.key).collect();
    if wanted != map.drawn || frame.rings.len() != map.rings_drawn {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        spawn_scene(&mut commands, &map, &frame, standoff, rad_per_px, &mut meshes,
            &mut materials);
        map.drawn = wanted;
        map.rings_drawn = frame.rings.len();
        map.frame = Some(frame);
        return;
    }

    for (of, mut at, mut mesh, material) in items.iter_mut() {
        let Some(placement) = frame.placements.iter().find(|p| p.key == of.0) else { continue };
        let resolved = is_resolved(placement, rad_per_px);
        *at = item_transform(placement, rad_per_px);
        // Zooming in on a body crosses the threshold without changing the set that is drawn,
        // so the level of detail is a handle swap here and not a respawn. Same as a drop-line
        // gaining a dash.
        let (wanted, fraction) = match resolved {
            true => (&map.sphere, SPHERE_TUBE_FRACTION),
            false => (&map.point, POINT_TUBE_FRACTION),
        };
        if mesh.0 != *wanted {
            mesh.0 = wanted.clone();
        }
        set_thickness(&mut materials, material, at.scale.max_element(), rad_per_px,
            at.translation.length(), fraction, LINE_PX);
    }
    for (of, mut at, mut mesh, material) in drops.iter_mut() {
        let Some(placement) = frame.placements.iter().find(|p| p.key == of.0) else { continue };
        *at = drop_transform(placement);
        // A body drifting off the plane gains dashes rather than longer ones, so the mesh it
        // is drawn with changes. A handle swap, not a rebuild.
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
        // The scale written into the transform, not the stand-off: the shader displaces in the
        // mesh's own space, so a thickness computed against a different number is wrong by
        // exactly that ratio. Sized against the near end, which is the focus.
        set_thickness(&mut materials, material, standoff * SPOKE_REACH, rad_per_px, standoff,
            LINE_TUBE_FRACTION, SCALE_PX);
    }
    map.frame = Some(frame);
}

/// Put `focus_ly` where the focus says to look, and say whether it moved.
///
/// The interface holds one position and it has to be the one on screen: an action that pans or
/// zooms starts from it, and a stale value is a jump the moment anything does.
pub fn follow(focus: crate::ui::MapFocus, snapshot: &MapSnapshot, focus_ly: &mut DVec3) -> bool {
    match focus_position(focus, snapshot) {
        Some(at) if at != *focus_ly => {
            *focus_ly = at;
            true
        }
        _ => false,
    }
}

/// Where the camera should be looking, or `None` to leave it where it is.
///
/// A key that is no longer in the snapshot also leaves it: a body going out of range should
/// stop the camera following it, not throw the view at the world origin.
pub fn focus_position(focus: crate::ui::MapFocus, snapshot: &MapSnapshot) -> Option<DVec3> {
    match focus {
        crate::ui::MapFocus::Free => None,
        crate::ui::MapFocus::Observer => snapshot.observer().map(|o| o.position_ly),
        crate::ui::MapFocus::Item(key) => snapshot.item(key).map(|i| i.position_ly),
    }
}

/// How close a ring or a shell comes to the camera, in render units.
///
/// **A tube's width is set by the nearest part of its own mesh, never the furthest.** A ring
/// gets away with one number because every point of it is the same distance off; anything with
/// a radial extent does not, and the Oort cloud is the extreme case — a shell from 633 render
/// units out to 181 000, sized against the far edge, came back with a tube 585 units thick
/// while its inner edge passed 574 from the camera. The camera was inside it and the map went
/// flat green, which is the same failure the plane's spokes had and the same cause.
///
/// Floored at the camera's own clearance over the reference plane, so a view from *inside* a
/// belt still draws it as a hairline rather than shrinking it to nothing.
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
/// Built in simulation axes with the pole on `+Z` and converted here, so it arrives with the
/// pole on `+Y` — which is what [`ring_transform`] and [`annulus_transform`] rotate from. The
/// shape depends only on the inner-to-outer ratio and the half-angle, so it survives every
/// zoom: what changes is the scale it is drawn at.
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

/// A body is a sphere at its own size once it is worth more than [`POINT_PX`] across, and a
/// circle of exactly that below it.
///
/// **A ship is never a sphere, at any zoom.** It is a mark on a chart rather than a body seen:
/// its size is not what anyone is reading off the map, and a hull that turned into a model on
/// approach would be the one thing here drawing a shape it does not know.
fn is_resolved(placement: &Placement, rad_per_px: f32) -> bool {
    placement.kind != ItemKind::Ship
        && rad_per_px > 0.0
        && 2.0 * placement.angular_radius / rad_per_px > POINT_PX
}

/// The render-unit radius of a circle drawn [`POINT_PX`] across at `distance`.
fn point_radius(distance: f32, rad_per_px: f32) -> f32 {
    (distance * rad_per_px * POINT_PX * 0.5).max(f32::MIN_POSITIVE)
}

/// A circle is a symbol rather than an object, so it faces the eye — which is the render
/// origin, because every transform here is camera-relative. The mesh lies in the XZ plane,
/// so it is its +Y that has to point back.
fn face_camera(at: Vec3) -> Quat {
    match at.try_normalize() {
        Some(away) => Quat::from_rotation_arc(Vec3::Y, -away),
        None => Quat::IDENTITY,
    }
}

fn item_transform(placement: &Placement, rad_per_px: f32) -> Transform {
    let at = at_of(placement);
    match is_resolved(placement, rad_per_px) {
        true => Transform {
            translation: at,
            rotation: Quat::from_rotation_arc(
                Vec3::Y,
                render(placement.pole.as_dvec3()).normalize(),
            ),
            scale: Vec3::splat(placement.radius),
        },
        false => Transform {
            translation: at,
            rotation: face_camera(at),
            scale: Vec3::splat(point_radius(at.length(), rad_per_px)),
        },
    }
}

/// How many dashes a drop wants, so that each one is [`DASH_PX`] long on screen.
///
/// The mesh lays `n` dashes and `n - 1` gaps of equal length over a unit height, so a dash is
/// `1 / (2n - 1)` of the drop; wanting a dash of `d` render units out of a drop of `h` gives
/// `n = (h / d + 1) / 2`.
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

/// Keep a line the same width on screen whatever its entity is scaled to.
///
/// The shader displaces vertices along their normals in **local** space, so the world width is
/// `scale * target`. Wanting a world width of `distance * rad_per_px * LINE_PX` therefore
/// means asking for that over the scale.
/// The tube radius a mesh wants, in its own local units.
///
/// Pure, and shared by the spawn and the per-frame update — **which is the whole point**. The
/// spawn used to take the material's default and let the next frame correct it, and for that
/// one frame a spoke scaled to forty stand-offs carried a tube `0.48` of a stand-off thick
/// while the camera cleared the plane by `0.05` of one. The camera was inside the tube, and
/// the inside of a tube is a solid wall: the map flashed full green. It happened on a respawn,
/// a respawn happens when the ring count changes, and the ring count changes on every decade —
/// so it fired while scrolling and almost never while sitting still.
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
    rad_per_px: f32,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<BodyWireframeMaterial>,
) {
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
        let at = item_transform(placement, rad_per_px);
        let (mesh, fraction) = match is_resolved(placement, rad_per_px) {
            true => (&map.sphere, SPHERE_TUBE_FRACTION),
            false => (&map.point, POINT_TUBE_FRACTION),
        };
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(line_material(
                color_of(placement.kind),
                tube_target(at.scale.max_element(), rad_per_px, at.translation.length(),
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
        // Set here rather than left to the default and corrected next frame. See
        // [`tube_target`] for what that cost.
        target_tube_radius,
        // Unlit. With no suns the shader's day/night factor is one, which is what a diagram
        // wants: the map says where a thing is, and the sky says what it looks like.
        num_suns: 0,
        ..default()
    }
}

// The palette is the interface's own. `18-ui-style.md`: one source, and a conversion at the
// edge — never a second set of values chosen to look about right.
const RING: Color = em_ui::vfd::TEXT_DIM;
const SPOKE: Color = em_ui::vfd::TEXT_DIM;
const DROP: Color = em_ui::vfd::BUTTON_BORDER;
const POPULATION: Color = em_ui::vfd::TEXT_DIM;

fn color_of(kind: ItemKind) -> Color {
    match kind {
        // A star is what a system is, so it takes the one bright color.
        ItemKind::Star => em_ui::vfd::TEXT,
        ItemKind::Planet | ItemKind::Moon | ItemKind::Minor => em_ui::vfd::BUTTON_BORDER,
        ItemKind::Population => em_ui::vfd::TEXT_DIM,
        // The two the player is here to find. Amber against the green, because color is the
        // one channel a map has that a list does not.
        ItemKind::Ship | ItemKind::Station => Color::srgb(0.95, 0.70, 0.25),
        ItemKind::Observer => Color::srgb(1.0, 1.0, 1.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::MapFocus;
    use em_map::{ItemKey, ItemKind, MapItem, MapSnapshot};
    use glam::DVec3;

    fn snapshot() -> MapSnapshot {
        MapSnapshot::observed(0.0, vec![
            MapItem::body(ItemKey::from_name("observer"), "this ship", ItemKind::Observer,
                DVec3::new(1.0, 2.0, 3.0), 100.0, DVec3::Z),
            MapItem::body(ItemKey::from_id("star", 7), "Sol", ItemKind::Star,
                DVec3::new(4.0, 5.0, 6.0), 7.0e8, DVec3::Z),
        ])
    }

    /// **The ship was the one thing on the map you could not center on.**
    ///
    /// Focus was an `Option<ItemKey>` and `None` had to mean "leave the camera alone", because
    /// that is what a pan needs — so the button that asked for the observer asked for nothing
    /// and the camera stayed at the world origin. Three states, because there are three.
    #[test]
    fn centering_on_the_ship_finds_the_ship() {
        let snapshot = snapshot();
        assert_eq!(
            focus_position(MapFocus::Observer, &snapshot),
            Some(DVec3::new(1.0, 2.0, 3.0)),
        );
        assert_eq!(
            focus_position(MapFocus::Item(ItemKey::from_id("star", 7)), &snapshot),
            Some(DVec3::new(4.0, 5.0, 6.0)),
        );
    }

    /// **The first frame of a drag used to jump to the world origin.**
    ///
    /// `place` resolved the focus into a *copy* of the view, so the interface's own
    /// `focus_ly` stayed at its default — `DVec3::ZERO`, which is where the star sits — and a
    /// pan started from there rather than from what was on screen. One jarring jump, then
    /// smooth, which is the signature of a stale starting point rather than a bad delta.
    #[test]
    fn a_pan_begins_where_the_camera_actually_is() {
        let snapshot = snapshot();
        let ship = DVec3::new(1.0, 2.0, 3.0);
        let mut orbit = em_map::Orbit::framing(DVec3::ZERO, em_map::snapshot::M_PER_AU * 40.0);
        assert_eq!(orbit.focus_ly, DVec3::ZERO, "premise: it starts at the origin");

        follow(MapFocus::Observer, &snapshot, &mut orbit.focus_ly);
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

    /// And following writes only when it has something to say, so a resource that half the
    /// interface watches is not marked changed every frame for nothing.
    #[test]
    fn following_the_same_place_twice_writes_once() {
        let snapshot = snapshot();
        let mut at = DVec3::ZERO;
        assert!(follow(MapFocus::Observer, &snapshot, &mut at), "the first call should move it");
        assert!(!follow(MapFocus::Observer, &snapshot, &mut at), "the second should not");
        assert!(!follow(MapFocus::Free, &snapshot, &mut at), "free never moves it");
    }

    /// And a pan has to be able to leave the camera alone, which is the state the other two
    /// were competing with.
    #[test]
    fn a_free_camera_is_left_where_it_was_put() {
        assert_eq!(focus_position(MapFocus::Free, &snapshot()), None);
    }

    /// A map opens on the observer rather than on the world origin, which is empty space some
    /// distance from wherever the ship happens to be.
    #[test]
    fn a_map_opens_on_the_ship() {
        assert_eq!(crate::ui::MapView::default().focus, MapFocus::Observer);
    }

    /// Following something that has gone out of range stops following it. It does not throw
    /// the view at the origin, which is what a `None`-means-origin reading would do.
    #[test]
    fn following_something_that_is_gone_holds_still() {
        assert_eq!(focus_position(MapFocus::Item(ItemKey(999)), &snapshot()), None);
        assert_eq!(
            focus_position(MapFocus::Observer, &MapSnapshot::observed(0.0, Vec::new())),
            None,
        );
    }

    /// **No line is ever thicker than the camera's clearance over the plane.**
    ///
    /// This is the invariant the whole map rests on: the camera is held off the plane by
    /// `em_map::camera::ELEVATION_FLOOR`, and every line is a tube with a width of its own. A
    /// tube wider than that clearance swallows the camera, and the inside of a tube is a solid
    /// wall — the map goes flat green.
    ///
    /// Checked across the stand-offs and viewport sizes the map actually runs at, for the
    /// family that reaches furthest and so is scaled hardest.
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

    fn body_at(distance: f32, radius: f32) -> Placement {
        kind_at(ItemKind::Planet, distance, radius)
    }

    fn kind_at(kind: ItemKind, distance: f32, radius: f32) -> Placement {
        Placement {
            key: ItemKey::from_name("a body"),
            kind,
            label: "a body".into(),
            at: glam::Vec3::new(0.0, distance, 0.0),
            foot: glam::Vec3::new(0.0, distance, 0.0),
            radius,
            angular_radius: radius / distance,
            annulus: None,
            pole: glam::Vec3::Z,
        }
    }

    /// **Nothing changes size at the level of detail.**
    ///
    /// One number is both the threshold and the size the circle is drawn at, so a body shrinks
    /// until it reaches [`POINT_PX`] and then holds it. Reading that number as a radius in one
    /// place and a diameter in the other is a factor of two at the crossover — a visible pop,
    /// and nothing in the types to catch it.
    #[test]
    fn a_body_holds_its_size_where_it_stops_being_a_sphere() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        let distance = 40.0;
        // A body sitting exactly on the threshold, and one a hair under it.
        let on = POINT_PX * 0.5 * rad_per_px * distance;
        assert!(is_resolved(&body_at(distance, on * 1.01), rad_per_px), "just over should be a sphere");
        let under = body_at(distance, on * 0.99);
        assert!(!is_resolved(&under, rad_per_px), "just under should be a circle");

        let drawn = item_transform(&under, rad_per_px).scale.x;
        assert!(
            (drawn / on - 1.0).abs() < 0.05,
            "a circle of {drawn} where the sphere it replaced was {on}",
        );
    }

    /// **A ship is a mark on a chart, at every zoom there is.**
    ///
    /// It has a hull size and the map is not the place to read it off: a contact that grew a
    /// model on approach would be drawing a shape nobody sent. Given a planet's radius it must
    /// still be a circle.
    #[test]
    fn a_ship_never_becomes_a_model() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        // A radius that would fill the view, at a distance that would make anything else a
        // sphere many times over.
        let huge = kind_at(ItemKind::Ship, 1.0, 10.0);
        assert!(!is_resolved(&huge, rad_per_px), "a ship is never a sphere");
        assert!(
            is_resolved(&kind_at(ItemKind::Planet, 1.0, 10.0), rad_per_px),
            "and the exemption is the kind, not the numbers",
        );
        // Drawn at the symbol's own size, like every other circle.
        let at = item_transform(&huge, rad_per_px);
        let px = 2.0 * at.scale.x / (at.translation.length() * rad_per_px);
        assert!((px / POINT_PX - 1.0).abs() < 1.0e-3, "a ship drew {px} px across");
    }

    /// And it holds that size at every distance, which is what makes it a symbol: the far one
    /// is as readable as the near one, and only its place on the map says which is which.
    #[test]
    fn a_circle_is_the_same_size_wherever_it_is() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
        let want = POINT_PX * 0.5;
        for distance in [1.0e-3f32, 1.0, 40.0, 1.0e5] {
            // Radius zero: a ship, which has no size to draw at any zoom.
            let at = item_transform(&body_at(distance, 0.0), rad_per_px);
            let px = at.scale.x / (at.translation.length() * rad_per_px);
            assert!(
                (px / want - 1.0).abs() < 1.0e-3,
                "at {distance:e} the circle came out {px} px across the radius, wanted {want}",
            );
        }
    }

    /// **A circle faces the eye, or it is an ellipse and sometimes a line.**
    ///
    /// The eye is the render origin, because every transform on this layer is camera-relative.
    #[test]
    fn a_circle_faces_the_eye() {
        let rad_per_px = 2.0 * (std::f32::consts::FRAC_PI_4 * 0.5).tan() / 410.0;
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
            let at = item_transform(&placement, rad_per_px);
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

    /// **A dash is the same length wherever it is drawn.**
    ///
    /// The count follows the drop rather than the other way round: twice the drop is twice the
    /// dashes, not dashes twice as long. A fixed count gave a tall drop long dashes and a
    /// short one short ones, which reads as two kinds of line.
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

    /// **A belt's tube never reaches the camera either.**
    ///
    /// The fourth time this family of bug appeared, and the same cause as the plane's spokes:
    /// a width set by the far side of something that spans a range of distances. The Oort
    /// cloud is the extreme case — a shell from 633 render units to 181 000 — and sized
    /// against its outer edge it came back 585 units thick while its inner edge passed 574
    /// from the camera.
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

    /// And the reach is the *near* edge, which is the whole of the fix.
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

    /// And the material's own default is *not* good enough, which is what the bug was: the
    /// spawn took it and let the next frame correct it, so every respawn flashed.
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

    /// A line stays the same width on screen however hard its mesh is scaled. That is the
    /// whole reason the thickness is a per-entity dial rather than baked into the mesh.
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
