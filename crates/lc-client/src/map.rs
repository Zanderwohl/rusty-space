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

/// How wide a line is drawn, in pixels, whatever it is a line of.
///
/// Screen-constant rather than world-constant: a ring scaled to ten thousand render units
/// would scale its own tube to ten thousand as well. The shader's `target_tube_radius` is the
/// dial, and it is set per entity from that entity's own scale.
const LINE_PX: f32 = 1.6;

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
const POINT_TUBE_FRACTION: f32 = 0.5;

/// How much of the palette color a line is drawn at.
///
/// The shader gives `base_color * (1 + alpha * emission_strength)`, and the alpha is the line
/// weight the mesh carries: 0.6 for grid, 1.0 for an equator. With no tone map in front of it
/// the whole range has to land inside the display, so the base is scaled down and the emission
/// makes up the contrast — 0.45 and 1.2 put a grid line at 0.77 and an equator at 0.99.
const LINE_COLOR_SCALE: f32 = 0.45;
const LINE_EMISSION: f32 = 1.2;

/// The near and far planes, as multiples of the stand-off.
///
/// Wide enough that a ship beside the camera and the outermost decade ring are both in the
/// frustum, and no wider — `MAX_RENDER_UNITS` bounds what can be placed at all.
const NEAR_FRACTION: f32 = 1.0e-4;
const FAR_MULTIPLE: f32 = 1.0e6;

/// Divisions of a decade ring. Enough that the largest one does not read as a polygon.
const RING_SEGMENTS: u32 = 128;

/// How far the spokes reach, as a fraction of the stand-off.
///
/// Short of the camera, deliberately. Reaching exactly the stand-off put the rim of one spoke
/// at precisely the eye, so an edge-on view was taken from inside a tube.
const SPOKE_REACH: f32 = 0.5;

/// Radial spokes in the reference plane.
///
/// **Sized to the stand-off, not to the outermost ring.** Every point of a ring is the same
/// distance from the center, so one tube radius is right for all of it; a spoke runs from
/// near the camera out to its rim, and a constant world radius that is a pixel at the far end
/// is eighty at the near one. Scaled to the outermost decade the twelve of them were twelve
/// solid wedges across the top of the view, and the arithmetic for the thickness looked
/// entirely correct — it was answering about the wrong end.
const PLANE_SPOKES: u32 = 12;

/// Dashes in a drop-line, and the count is fixed so it is the same line at every zoom.
const DROP_DASHES: u32 = 9;

/// Angular radius, in pixels, past which an item is drawn as a wireframe sphere rather than a
/// point. Below it a sphere is a handful of sub-pixel tubes and reads as nothing at all.
const SPHERE_PX: f32 = 3.0;

/// What a point is drawn at, in render units per unit of distance: a small constant angle, so
/// everything unresolved is the same size on screen.
const POINT_ANGULAR_RADIUS: f32 = 0.004;

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
    ring: Handle<Mesh>,
    spokes: Handle<Mesh>,
    drop: Handle<Mesh>,
    /// What is spawned, in order. A rebuild happens only when this stops matching the frame.
    drawn: Vec<ItemKey>,
    rings_drawn: usize,
}

impl Map {
    /// Radians per pixel of the map's own viewport, which is what decides a sphere from a
    /// point and how thick a line is.
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
        ring: meshes.add(wire_mesh::ring_tube(RING_SEGMENTS, BASE_TUBE_RADIUS, 4, 1.0)),
        spokes: meshes.add(wire_mesh::plane_spokes(PLANE_SPOKES, BASE_TUBE_RADIUS, 4, 0.6)),
        drop: meshes.add(wire_mesh::drop_line(DROP_DASHES, BASE_TUBE_RADIUS, 4, 0.8)),
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
    ui: Res<Ui>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
    mut camera: Query<(&mut Transform, &mut Projection), (With<MapCamera>, Without<MapDrawn>)>,
    existing: Query<Entity, With<MapDrawn>>,
    mut items: Query<
        (&MapItemOf, &mut Transform, &MeshMaterial3d<BodyWireframeMaterial>),
        (Without<MapCamera>, Without<MapDropOf>, Without<MapRingOf>, Without<MapSpokes>),
    >,
    mut drops: Query<
        (&MapDropOf, &mut Transform, &MeshMaterial3d<BodyWireframeMaterial>),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapRingOf>, Without<MapSpokes>),
    >,
    mut rings: Query<
        (&MapRingOf, &mut Transform, &MeshMaterial3d<BodyWireframeMaterial>),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapDropOf>, Without<MapSpokes>),
    >,
    mut spokes: Query<
        (&mut Transform, &MeshMaterial3d<BodyWireframeMaterial>),
        (With<MapSpokes>, Without<MapCamera>, Without<MapItemOf>, Without<MapDropOf>,
            Without<MapRingOf>),
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

    let mut view = ui.map;
    // Follow the selection rather than a remembered position: Saturn moves, and a map that
    // centered on where it was is a map that drifts off it over an afternoon.
    if let Some(item) = view.focus.and_then(|key| map.snapshot.item(key)) {
        view.orbit.focus_ly = item.position_ly;
    }
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
        _ => std::f32::consts::FRAC_PI_4,
    };
    let rad_per_px = map.radians_per_pixel(fov_y);

    let wanted: Vec<ItemKey> = frame.placements.iter().map(|p| p.key).collect();
    if wanted != map.drawn || frame.rings.len() != map.rings_drawn {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        spawn_scene(&mut commands, &map, &frame, standoff, &mut materials);
        map.drawn = wanted;
        map.rings_drawn = frame.rings.len();
        map.frame = Some(frame);
        return;
    }

    for (of, mut at, material) in items.iter_mut() {
        let Some(placement) = frame.placements.iter().find(|p| p.key == of.0) else { continue };
        let resolved = is_resolved(placement, rad_per_px);
        *at = item_transform(placement, rad_per_px);
        set_thickness(&mut materials, material, at.scale.max_element(), rad_per_px,
            at.translation.length(),
            if resolved { SPHERE_TUBE_FRACTION } else { POINT_TUBE_FRACTION });
    }
    for (of, mut at, material) in drops.iter_mut() {
        let Some(placement) = frame.placements.iter().find(|p| p.key == of.0) else { continue };
        *at = drop_transform(placement);
        set_thickness(&mut materials, material, 1.0, rad_per_px, at.translation.length(),
            LINE_TUBE_FRACTION);
    }
    for (of, mut at, material) in rings.iter_mut() {
        let Some(ring) = frame.rings.get(of.0) else { continue };
        *at = ring_transform(&frame, ring.radius);
        set_thickness(&mut materials, material, ring.radius, rad_per_px,
            at.translation.length().max(ring.radius), LINE_TUBE_FRACTION);
    }
    if let Ok((mut at, material)) = spokes.single_mut() {
        *at = ring_transform(&frame, standoff * SPOKE_REACH);
        set_thickness(&mut materials, material, standoff, rad_per_px, standoff,
            LINE_TUBE_FRACTION);
    }
    map.frame = Some(frame);
}

fn render(v: DVec3) -> Vec3 {
    sim_to_render(v).as_vec3()
}

fn at_of(placement: &Placement) -> Vec3 {
    render(placement.at.as_dvec3())
}

/// A body is a sphere at its own size once it is worth more than a few pixels, and a point of
/// a fixed angular size below that.
fn is_resolved(placement: &Placement, rad_per_px: f32) -> bool {
    rad_per_px > 0.0 && placement.angular_radius / rad_per_px > SPHERE_PX
}

fn item_transform(placement: &Placement, rad_per_px: f32) -> Transform {
    let at = at_of(placement);
    let resolved = is_resolved(placement, rad_per_px);
    let radius = match resolved {
        true => placement.radius,
        false => (at.length() * POINT_ANGULAR_RADIUS).max(f32::MIN_POSITIVE),
    };
    Transform {
        translation: at,
        rotation: Quat::from_rotation_arc(Vec3::Y, render(placement.pole.as_dvec3()).normalize()),
        scale: Vec3::splat(radius),
    }
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
        translation: render(frame.focus.as_dvec3()),
        rotation: Quat::from_rotation_arc(Vec3::Y, normal),
        scale: Vec3::splat(radius.max(f32::MIN_POSITIVE)),
    }
}

/// Keep a line the same width on screen whatever its entity is scaled to.
///
/// The shader displaces vertices along their normals in **local** space, so the world width is
/// `scale * target`. Wanting a world width of `distance * rad_per_px * LINE_PX` therefore
/// means asking for that over the scale.
fn set_thickness(
    materials: &mut Assets<BodyWireframeMaterial>,
    material: &MeshMaterial3d<BodyWireframeMaterial>,
    scale: f32,
    rad_per_px: f32,
    distance: f32,
    max_fraction: f32,
) {
    let Some(mut asset) = materials.get_mut(&material.0) else { return };
    let world = (distance * rad_per_px * LINE_PX).max(f32::MIN_POSITIVE);
    let target = match scale > f32::MIN_POSITIVE {
        true => (world / scale).min(max_fraction),
        false => BASE_TUBE_RADIUS,
    };
    asset.target_tube_radius = target;
}

fn spawn_scene(
    commands: &mut Commands,
    map: &Map,
    frame: &MapFrame,
    standoff: f32,
    materials: &mut Assets<BodyWireframeMaterial>,
) {
    let layer = RenderLayers::layer(MAP_LAYER);

    for (index, ring) in frame.rings.iter().enumerate() {
        commands.spawn((
            Mesh3d(map.ring.clone()),
            MeshMaterial3d(materials.add(line_material(RING))),
            ring_transform(frame, ring.radius),
            NoFrustumCulling,
            layer.clone(),
            MapDrawn,
            MapRingOf(index),
        ));
    }

    commands.spawn((
        Mesh3d(map.spokes.clone()),
        MeshMaterial3d(materials.add(line_material(SPOKE))),
        ring_transform(frame, standoff * SPOKE_REACH),
        NoFrustumCulling,
        layer.clone(),
        MapDrawn,
        MapSpokes,
    ));

    for placement in &frame.placements {
        commands.spawn((
            Mesh3d(map.sphere.clone()),
            MeshMaterial3d(materials.add(line_material(color_of(placement.kind)))),
            item_transform(placement, 0.0),
            NoFrustumCulling,
            layer.clone(),
            MapDrawn,
            MapItemOf(placement.key),
        ));
        if placement.has_drop_line() {
            commands.spawn((
                Mesh3d(map.drop.clone()),
                MeshMaterial3d(materials.add(line_material(DROP))),
                drop_transform(placement),
                NoFrustumCulling,
                layer.clone(),
                MapDrawn,
                MapDropOf(placement.key),
            ));
        }
    }
}

fn line_material(color: Color) -> BodyWireframeMaterial {
    let rgba = color.to_linear();
    BodyWireframeMaterial {
        base_color: LinearRgba::new(
            rgba.red * LINE_COLOR_SCALE,
            rgba.green * LINE_COLOR_SCALE,
            rgba.blue * LINE_COLOR_SCALE,
            1.0,
        ),
        emission_strength: LINE_EMISSION,
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
