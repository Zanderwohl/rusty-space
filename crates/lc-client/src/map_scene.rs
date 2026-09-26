//! The map's entities, and where each is drawn: one per item, with its drop line, spread and
//! outline where it has them, the decade rings and the plane's spokes. [`crate::map`] has the
//! camera, the target and the frame these are laid out from.

use std::collections::HashMap;

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::{NoAutoAabb, NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use em_map::rings::MAX_RINGS;
use em_map::{ItemKey, ItemKind, MapFrame, Placement};
use em_render::body_material::BASE_TUBE_RADIUS;
use em_render::render_space::sim_to_render;
use em_render::wire_mesh;
use glam::DVec3;

use crate::map::{MAP_LAYER, Map, MapCamera};
use crate::map_line::MapLineMaterial;
use crate::map_spread::{self, ArcShape, MapSpreadOf};

/// How wide a line is drawn, in pixels. Held there per vertex by `map_line.wgsl`.
pub(crate) const LINE_PX: f32 = 1.6;

/// The most of its own unit mesh a tube may take, per family.
///
/// The shader displaces along normals in mesh space, so this is a proportion of the thing
/// drawn and not of the screen. Unclamped, a ring seen from forty times its radius would be a
/// tube a tenth of itself thick.
pub(crate) const LINE_TUBE_FRACTION: f32 = 0.02;
pub(crate) const SPHERE_TUBE_FRACTION: f32 = 0.06;

/// A disc has one normal, so displacement translates it instead of thickening it. Asking for
/// the material's base radius makes the displacement zero.
pub(crate) const DOT_TUBE_FRACTION: f32 = BASE_TUBE_RADIUS;

/// A circle's line is [`LINE_PX`] over the mark's radius in pixels, which the shader works out
/// itself: both are fixed on screen, so the distance cancels. This is that at the smallest mark
/// there is, so it never binds and every circle can share one material.
pub(crate) const CIRCLE_TUBE_FRACTION: f32 = LINE_PX / (POINT_FLOOR_PX * 0.5);

/// What a unit mesh here can reach once the shader has thickened it, with room: a circle's line
/// at [`CIRCLE_TUBE_FRACTION`] puts its outside at twice its radius. Held rather than computed,
/// because a mark changes mesh with its form and the box has to fit all of them.
const UNIT_REACH: f32 = 2.5;

/// The reference scale — rings, spokes, drop-lines — is drawn at half a line's width and half
/// its brightness. It is the ruler, not what is being measured.
pub(crate) const SCALE_PX: f32 = LINE_PX * 0.5;
const SCALE_COLOR_SCALE: f32 = LINE_COLOR_SCALE * 0.5;
/// A population's outline, dashed. At full brightness a shell's six curves outshine the map.
const POPULATION_COLOR_SCALE: f32 = LINE_COLOR_SCALE * 0.125;
/// An error bar sits well under the line it qualifies: a system of them is a thicket. The
/// selected item's are drawn at full brightness, which is when anyone is reading them.
const SPREAD_COLOR_SCALE: f32 = LINE_COLOR_SCALE * 0.125;
/// Length of the cap across each end of an error bar.
pub(crate) const SPREAD_CAP_PX: f32 = 8.0;

/// How much of the palette color a line is drawn at.
///
/// The shader gives `base_color * (1 + alpha * emission_strength)`, where alpha is the mesh's
/// line weight: 0.6 for grid, 1.0 for an equator. With no tone map in front of it the whole
/// range has to land inside the display, so 0.45 and 1.2 put a grid line at 0.77 and an
/// equator at 0.99.
const LINE_COLOR_SCALE: f32 = 0.45;
const LINE_EMISSION: f32 = 1.2;

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
pub(crate) const DASH_PX: f32 = 5.0;

/// The most dashes a drop-line is built with. A drop needing more than this is longer than the
/// viewport many times over and its dashes are sub-pixel anyway.
pub(crate) const MAX_DASHES: usize = 48;

/// A mark's apparent diameter, as a share of the viewport's height. A diameter, unlike the
/// sky's `resolved::RESOLVE_PX`, and a share because one texture serves two surfaces.
///
/// Both the threshold and the size a mark is drawn at, so a body shrinks to this and holds and
/// nothing jumps at the crossover. Below it a sphere is a dozen sub-pixel tubes over each
/// other: dearer to draw and less legible.
pub(crate) const POINT_FRACTION: f32 = 0.02;

/// The floor under that, at twice a line's width. Below it a ring has no inside left, which
/// makes it a dot.
pub(crate) const POINT_FLOOR_PX: f32 = 2.0 * LINE_PX;

/// Divisions of a mark. Sixteen is smooth at any size one reaches, and an eighth of the ring
/// mesh — a level of detail that cost more than the sphere would not be one.
const POINT_SEGMENTS: u32 = 16;

/// Everything the map spawns, so a rebuild can clear the layer without touching anything else.
#[derive(Component)]
pub struct MapDrawn;

/// Which item an entity stands for. By key, not by place in the frame's list: the list changes
/// as things come and go, and only what came or went is spawned or despawned. See [`lay`].
#[derive(Component)]
pub struct MapItemOf(pub ItemKey);

/// A decade ring, by its place in the frame's list. There are always [`MAX_RINGS`], and those
/// the frame has no ring for are hidden.
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

/// What a pixel of the map's viewport is worth. Everything on the layer is sized from one of
/// these two, and both come from the viewport's height.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Viewport {
    /// Radians a pixel subtends: every line's thickness, and half of the sphere decision.
    pub(crate) rad_per_px: f32,
    /// A mark's diameter in pixels. See [`POINT_FRACTION`].
    pub(crate) point_px: f32,
}

impl Viewport {
    pub(crate) fn new(height_px: u32, fov_y: f32) -> Self {
        Self {
            rad_per_px: match height_px {
                0 => 0.0,
                height => 2.0 * (fov_y * 0.5).tan() / height as f32,
            },
            point_px: (height_px as f32 * POINT_FRACTION).max(POINT_FLOOR_PX),
        }
    }

    /// One item's mark, in pixels: the surface's size scaled by what the thing weighs, never
    /// under the floor. On a small map the floor binds before [`em_map::weight::MIN_SCALE`].
    pub(crate) fn mark_px(self, placement: &Placement) -> f32 {
        (self.point_px * placement.symbol_scale).max(POINT_FLOOR_PX)
    }
}

/// How a body is drawn at this zoom. One decision, read in three places.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Form {
    Sphere,
    Circle,
    Dot,
}

/// What a material is drawn for: an item's own mark, or its spread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Look {
    Mark(Form),
    Spread { selected: bool },
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

pub(crate) fn render(v: DVec3) -> Vec3 {
    sim_to_render(v).as_vec3()
}

pub(crate) fn at_of(placement: &Placement) -> Vec3 {
    render(placement.at.as_dvec3())
}

/// A body is a sphere at its own size above [`Viewport::point_px`] across, and a mark below.
///
/// The threshold is the surface's size and not the mark's, so a light body steps *down* to
/// its mark at the crossover. The alternative lets a rock stay a sphere down to three pixels,
/// which is the illegible case the level of detail exists for.
///
/// A ship is always a dot: its hull size is not what anyone reads off a map.
pub(crate) fn form_of(placement: &Placement, view: Viewport) -> Form {
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
pub(crate) fn point_radius(distance: f32, rad_per_px: f32, mark_px: f32) -> f32 {
    (distance * rad_per_px * mark_px * 0.5).max(f32::MIN_POSITIVE)
}

/// A mark faces the eye, which is the render origin because every transform here is
/// camera-relative. The mesh lies in the XZ plane, so its +Y is what points back.
pub(crate) fn face_camera(at: Vec3) -> Quat {
    match at.try_normalize() {
        Some(away) => Quat::from_rotation_arc(Vec3::Y, -away),
        None => Quat::IDENTITY,
    }
}

pub(crate) fn item_transform(placement: &Placement, view: Viewport) -> Transform {
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

/// The mesh a form is drawn with.
fn mesh_of(form: Form, shapes: &Shapes) -> &Handle<Mesh> {
    match form {
        Form::Sphere => &shapes.sphere,
        Form::Circle => &shapes.point,
        Form::Dot => &shapes.dot,
    }
}

/// The shared material for an item of `kind` drawn as `look`.
pub(crate) fn material_of(
    palette: &mut std::collections::HashMap<(ItemKind, Look), Handle<MapLineMaterial>>,
    materials: &mut Assets<MapLineMaterial>,
    kind: ItemKind,
    look: Look,
) -> Handle<MapLineMaterial> {
    palette
        .entry((kind, look))
        .or_insert_with(|| {
            let (cap, scale) = match look {
                Look::Mark(Form::Sphere) => (SPHERE_TUBE_FRACTION, LINE_COLOR_SCALE),
                Look::Mark(Form::Circle) => (CIRCLE_TUBE_FRACTION, LINE_COLOR_SCALE),
                Look::Mark(Form::Dot) => (DOT_TUBE_FRACTION, LINE_COLOR_SCALE),
                Look::Spread { selected: true } => (LINE_TUBE_FRACTION, LINE_COLOR_SCALE),
                Look::Spread { selected: false } => (LINE_TUBE_FRACTION, SPREAD_COLOR_SCALE),
            };
            materials.add(line_material(color_of(kind), cap, LINE_PX, scale))
        })
        .clone()
}

/// Culled like anything else, with room for what the shader adds. See [`UNIT_REACH`]. Held
/// fixed: Bevy would otherwise refit it to the mesh at every change of form, without the room.
pub(crate) fn unit_bounds() -> (Aabb, NoAutoAabb) {
    (Aabb::from_min_max(Vec3::splat(-UNIT_REACH), Vec3::splat(UNIT_REACH)), NoAutoAabb)
}

/// How many dashes a drop wants, so each is [`DASH_PX`] long on screen. The mesh lays `n`
/// dashes and `n - 1` equal gaps over a unit height, so `n = (h / d + 1) / 2`.
pub(crate) fn dash_count(height: f32, distance: f32, rad_per_px: f32) -> usize {
    let dash = distance * rad_per_px * DASH_PX;
    if !(dash > 0.0) || !height.is_finite() {
        return 1;
    }
    (((height / dash + 1.0) * 0.5).round() as i64).clamp(1, MAX_DASHES as i64) as usize
}

/// The dashed line is a unit height along `+Y`, so it is scaled to the drop and turned onto it.
pub(crate) fn drop_transform(placement: &Placement) -> Transform {
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
pub(crate) fn ring_transform(frame: &MapFrame, radius: f32) -> Transform {
    let normal = render(frame.plane_normal.as_dvec3()).normalize();
    Transform {
        // The ship, not the focus: the scale is the observer's. See `MapFrame::rings_at`.
        translation: render(frame.rings_at.as_dvec3()),
        rotation: Quat::from_rotation_arc(Vec3::Y, normal),
        scale: Vec3::splat(radius.max(f32::MIN_POSITIVE)),
    }
}

fn line_material(color: Color, max_fraction: f32, width_px: f32, color_scale: f32)
    -> MapLineMaterial {
    let rgba = color.to_linear();
    MapLineMaterial {
        base_color: LinearRgba::new(
            rgba.red * color_scale,
            rgba.green * color_scale,
            rgba.blue * color_scale,
            1.0,
        ),
        emission_strength: LINE_EMISSION,
        base_tube_radius: BASE_TUBE_RADIUS,
        max_fraction,
        width_px,
        dash_px: 0.0,
    }
}

// The interface's own palette. `18-ui-style.md`: one source, converted at the edge.
const RING: Color = em_ui::vfd::TEXT_DIM;
const SPOKE: Color = em_ui::vfd::TEXT_DIM;
const DROP: Color = em_ui::vfd::BUTTON_BORDER;
const POPULATION: Color = em_ui::vfd::TEXT_DIM;

pub(crate) fn color_of(kind: ItemKind) -> Color {
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


/// The meshes every entity here is drawn with, built once.
pub(crate) struct Shapes {
    sphere: Handle<Mesh>,
    /// An unresolved body: a circle facing the eye. See [`POINT_FRACTION`].
    point: Handle<Mesh>,
    /// A ship, at any zoom: the same size, filled.
    dot: Handle<Mesh>,
    ring: Handle<Mesh>,
    spokes: Handle<Mesh>,
    /// One drop-line mesh per dash count, indexed from one dash, so a body drifting off the
    /// plane swaps a handle instead of rebuilding geometry.
    drops: Vec<Handle<Mesh>>,
}

impl Shapes {
    pub(crate) fn new(meshes: &mut Assets<Mesh>) -> Self {
        Self {
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
        }
    }
}

/// What is spawned for one item.
#[derive(Default)]
struct Parts {
    item: Option<Entity>,
    drop: Option<Entity>,
    annulus: Option<Entity>,
    spread: map_spread::Spawned,
}

impl Parts {
    fn despawn(self, commands: &mut Commands) {
        let spread = self.spread.entities.into_iter();
        for entity in [self.item, self.drop, self.annulus].into_iter().flatten().chain(spread) {
            commands.entity(entity).despawn();
        }
    }
}

/// The materials the scale is drawn in.
struct Lines {
    ring: Handle<MapLineMaterial>,
    drop: Handle<MapLineMaterial>,
    population: Handle<MapLineMaterial>,
    spoke: Handle<MapLineMaterial>,
}

impl Lines {
    fn new(materials: &mut Assets<MapLineMaterial>) -> Self {
        Self {
            ring: materials.add(line_material(RING, LINE_TUBE_FRACTION, SCALE_PX, SCALE_COLOR_SCALE)),
            drop: materials.add(line_material(DROP, LINE_TUBE_FRACTION, SCALE_PX, SCALE_COLOR_SCALE)),
            population: materials.add(MapLineMaterial {
                dash_px: DASH_PX,
                ..line_material(POPULATION, LINE_TUBE_FRACTION, LINE_PX, POPULATION_COLOR_SCALE)
            }),
            spoke: materials.add(line_material(SPOKE, LINE_TUBE_FRACTION, SCALE_PX, SCALE_COLOR_SCALE)),
        }
    }
}

/// What is on the layer, and what it is drawn with.
#[derive(Default)]
pub(crate) struct Scene {
    parts: HashMap<ItemKey, Parts>,
    rings: Vec<Entity>,
    spokes: Option<Entity>,
    /// One material per kind and form, and per kind for spreads (`None`), shared by every item
    /// drawn with it: a material each was a bind group each, and a draw each.
    palette: HashMap<(ItemKind, Look), Handle<MapLineMaterial>>,
    lines: Option<Lines>,
    /// What the frame was composed for: the viewport and the camera's stand-off in render units.
    pub(crate) view: Option<(Viewport, f32)>,
}

/// Bring the layer to the frame: spawn what arrived, despawn what left, and place the rest.
///
/// A change to the set used to respawn the whole layer, with a material and an outline mesh for
/// each item. That happened at every decade of a zoom, and whenever a contact or star came or
/// went.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn lay(
    mut commands: Commands,
    mut map: ResMut<Map>,
    mut materials: ResMut<Assets<MapLineMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    ui: Res<crate::app::Ui>,
    existing: Query<Entity, With<MapDrawn>>,
    mut items: Query<
        (&MapItemOf, &mut Transform, &mut Mesh3d, &mut MeshMaterial3d<MapLineMaterial>),
        (Without<MapCamera>, Without<MapDropOf>, Without<MapRingOf>, Without<MapAnnulusOf>, Without<MapSpokes>,
            Without<MapSpreadOf>),
    >,
    mut drops: Query<
        (&MapDropOf, &mut Transform, &mut Mesh3d),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapRingOf>, Without<MapSpokes>, Without<MapAnnulusOf>,
            Without<MapSpreadOf>),
    >,
    mut spreads: Query<
        (&MapSpreadOf, &mut Transform, &mut Mesh3d, &mut MeshMaterial3d<MapLineMaterial>, Option<&mut ArcShape>),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapRingOf>, Without<MapSpokes>, Without<MapAnnulusOf>,
            Without<MapDropOf>),
    >,
    mut rings: Query<
        (&MapRingOf, &mut Transform, &mut Visibility),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapDropOf>, Without<MapSpokes>, Without<MapAnnulusOf>,
            Without<MapSpreadOf>),
    >,
    mut spokes: Query<
        &mut Transform,
        (With<MapSpokes>, Without<MapCamera>, Without<MapItemOf>, Without<MapDropOf>,
            Without<MapRingOf>, Without<MapAnnulusOf>, Without<MapSpreadOf>),
    >,
    mut annuli: Query<
        (&MapAnnulusOf, &mut Transform),
        (Without<MapCamera>, Without<MapItemOf>, Without<MapDropOf>, Without<MapRingOf>,
            Without<MapSpokes>, Without<MapSpreadOf>),
    >,
) {
    let map = &mut *map;
    if !map.shown {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        map.scene.parts.clear();
        map.scene.rings.clear();
        map.scene.spokes = None;
        return;
    }
    if !map.due {
        return;
    }
    let (Some(frame), Some((view, standoff))) = (map.frame.as_ref(), map.scene.view) else { return };
    let at: HashMap<ItemKey, &Placement> = frame.placements.iter().map(|p| (p.key, p)).collect();
    sync(&mut commands, &mut map.scene, &map.shapes, frame, &at, view, standoff, &mut meshes, &mut materials);

    for (of, mut place, mut mesh, mut material) in items.iter_mut() {
        let Some(placement) = at.get(&of.0) else { continue };
        *place = item_transform(placement, view);
        // Crossing the threshold changes a mesh and a material, not what is spawned.
        let form = form_of(placement, view);
        let wanted = mesh_of(form, &map.shapes);
        if mesh.0 != *wanted {
            mesh.0 = wanted.clone();
        }
        let wanted = material_of(&mut map.scene.palette, &mut materials, placement.kind, Look::Mark(form));
        if material.0 != wanted {
            material.0 = wanted;
        }
    }
    for (of, mut place, mut mesh) in drops.iter_mut() {
        let Some(placement) = at.get(&of.0) else { continue };
        *place = drop_transform(placement);
        // Drifting off the plane gains dashes, not longer ones, so the mesh changes.
        let dashes = dash_count(place.scale.y, place.translation.length(), view.rad_per_px);
        let wanted = &map.shapes.drops[dashes - 1];
        if mesh.0 != *wanted {
            mesh.0 = wanted.clone();
        }
    }
    let selected = crate::pick::selected(&ui)
        .and_then(|chosen| map.subjects.iter().find(|(_, s)| s.is(&chosen)).map(|(key, _)| *key));
    for (of, mut place, mut mesh, mut material, shape) in spreads.iter_mut() {
        let Some(placement) = at.get(&of.key) else { continue };
        map_spread::lay(placement, of, &mut place, &mut mesh, shape, &mut meshes, view.rad_per_px);
        let look = Look::Spread { selected: selected == Some(of.key) };
        let wanted = material_of(&mut map.scene.palette, &mut materials, placement.kind, look);
        if material.0 != wanted {
            material.0 = wanted;
        }
    }
    for (of, mut place, mut shown) in rings.iter_mut() {
        match frame.rings.get(of.0) {
            Some(ring) => {
                *place = ring_transform(frame, ring.radius);
                shown.set_if_neq(Visibility::Inherited);
            }
            None => {
                shown.set_if_neq(Visibility::Hidden);
            }
        }
    }
    for (of, mut place) in annuli.iter_mut() {
        let Some(annulus) = at.get(&of.0).and_then(|p| p.annulus) else { continue };
        *place = annulus_transform(at[&of.0], annulus);
    }
    if let Ok(mut place) = spokes.single_mut() {
        *place = ring_transform(frame, standoff * SPOKE_REACH);
    }
}

/// Spawn and despawn until the layer holds what `frame` does. What is spawned is placed as it
/// is spawned; what was already there is placed by [`lay`].
#[allow(clippy::too_many_arguments)]
fn sync(
    commands: &mut Commands,
    scene: &mut Scene,
    shapes: &Shapes,
    frame: &MapFrame,
    at: &HashMap<ItemKey, &Placement>,
    view: Viewport,
    standoff: f32,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<MapLineMaterial>,
) {
    let layer = RenderLayers::layer(MAP_LAYER);
    let Scene { parts: held, rings, spokes, palette, lines, .. } = scene;
    let gone: Vec<ItemKey> = held.keys().filter(|key| !at.contains_key(key)).copied().collect();
    for key in gone {
        if let Some(parts) = held.remove(&key) {
            parts.despawn(commands);
        }
    }
    let lines = lines.get_or_insert_with(|| Lines::new(materials));

    if rings.is_empty() {
        *rings = (0..MAX_RINGS)
            .map(|index| {
                commands
                    .spawn((
                        Mesh3d(shapes.ring.clone()),
                        MeshMaterial3d(lines.ring.clone()),
                        Transform::default(),
                        Visibility::Hidden,
                        NoFrustumCulling,
                        layer.clone(),
                        MapDrawn,
                        MapRingOf(index),
                    ))
                    .id()
            })
            .collect();
    }
    if spokes.is_none() {
        *spokes = Some(
            commands
                .spawn((
                    Mesh3d(shapes.spokes.clone()),
                    MeshMaterial3d(lines.spoke.clone()),
                    ring_transform(frame, standoff * SPOKE_REACH),
                    NoFrustumCulling,
                    layer.clone(),
                    MapDrawn,
                    MapSpokes,
                ))
                .id(),
        );
    }

    for placement in &frame.placements {
        let key = placement.key;
        let parts = held.entry(key).or_default();
        if parts.item.is_none() {
            let form = form_of(placement, view);
            parts.item = Some(
                commands
                    .spawn((
                        Mesh3d(mesh_of(form, shapes).clone()),
                        MeshMaterial3d(material_of(palette, materials, placement.kind, Look::Mark(form))),
                        item_transform(placement, view),
                        unit_bounds(),
                        layer.clone(),
                        MapDrawn,
                        MapItemOf(key),
                    ))
                    .id(),
            );
        }
        match (placement.annulus, parts.annulus) {
            (Some(annulus), None) => {
                parts.annulus = Some(
                    commands
                        .spawn((
                            Mesh3d(meshes.add(annulus_mesh(annulus))),
                            MeshMaterial3d(lines.population.clone()),
                            annulus_transform(placement, annulus),
                            NoFrustumCulling,
                            layer.clone(),
                            MapDrawn,
                            MapAnnulusOf(key),
                        ))
                        .id(),
                );
            }
            (None, Some(entity)) => {
                commands.entity(entity).despawn();
                parts.annulus = None;
            }
            _ => {}
        }
        map_spread::sync(
            commands,
            &mut parts.spread,
            placement,
            || material_of(palette, materials, placement.kind, Look::Spread { selected: false }),
            &shapes.drops[0],
            meshes,
            &layer,
            view.rad_per_px,
        );
        match (placement.has_drop_line(), parts.drop) {
            (true, None) => {
                let place = drop_transform(placement);
                let dashes = dash_count(place.scale.y, place.translation.length(), view.rad_per_px);
                parts.drop = Some(
                    commands
                        .spawn((
                            Mesh3d(shapes.drops[dashes - 1].clone()),
                            MeshMaterial3d(lines.drop.clone()),
                            place,
                            NoFrustumCulling,
                            layer.clone(),
                            MapDrawn,
                            MapDropOf(key),
                        ))
                        .id(),
                );
            }
            (false, Some(entity)) => {
                commands.entity(entity).despawn();
                parts.drop = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::world::CommandQueue;

    use super::*;

    fn placed(name: &str, spread: bool) -> Placement {
        let at = Vec3::new(0.0, 40.0, 0.0);
        Placement {
            key: ItemKey::from_name(name),
            kind: ItemKind::Planet,
            weight: 0.0,
            symbol_scale: 1.0,
            label: name.into(),
            at,
            foot: at,
            radius: 1.0,
            angular_radius: 1.0 / 40.0,
            annulus: None,
            pole: Vec3::Z,
            spread: match spread {
                true => vec![
                    em_map::Spread::Bar(at * 0.9, at * 1.1),
                    em_map::Spread::Arc { points: vec![at, at + Vec3::X, at + Vec3::new(1.0, 1.0, 0.0)], closed: false },
                ],
                false => Vec::new(),
            },
        }
    }

    fn frame(placements: Vec<Placement>) -> MapFrame {
        MapFrame {
            eye_ly: DVec3::ZERO,
            meters_per_unit: 1.0,
            plane: em_map::Plane::Galactic.about(DVec3::Z),
            plane_normal: Vec3::Z,
            focus: Vec3::ZERO,
            rings_at: Vec3::ZERO,
            placements,
            rings: Vec::new(),
        }
    }

    /// A spread is an eighth as bright as the planet's own mark, and exactly as bright once
    /// its planet is selected.
    #[test]
    fn a_spread_is_dim_until_its_item_is_selected() {
        let mut materials = Assets::<MapLineMaterial>::default();
        let mut palette = HashMap::new();
        let mut red = |look| {
            let handle = material_of(&mut palette, &mut materials, ItemKind::Planet, look);
            materials.get(&handle).unwrap().base_color.red
        };
        let mark = red(Look::Mark(Form::Circle));
        assert!((red(Look::Spread { selected: false }) * 8.0 - mark).abs() < 1.0e-6);
        assert_eq!(red(Look::Spread { selected: true }), mark);
    }

    /// A change to what is drawn spawns what arrived and despawns what left, and leaves the
    /// rest, the rings and the spokes where they are.
    #[test]
    fn only_what_came_or_went_is_spawned_or_despawned() {
        let mut world = World::new();
        let (mut meshes, mut materials) = (Assets::<Mesh>::default(), Assets::<MapLineMaterial>::default());
        let shapes = Shapes::new(&mut meshes);
        let mut scene = Scene::default();
        let view = Viewport::new(720, 0.8);
        let mut draw = |world: &mut World, scene: &mut Scene, frame: &MapFrame| {
            let at: HashMap<ItemKey, &Placement> = frame.placements.iter().map(|p| (p.key, p)).collect();
            let mut queue = CommandQueue::default();
            let mut commands = Commands::new(&mut queue, world);
            sync(&mut commands, scene, &shapes, frame, &at, view, 1.0, &mut meshes, &mut materials);
            queue.apply(world);
        };
        let items = |world: &mut World| {
            let mut found: Vec<(ItemKey, Entity)> =
                world.query::<(Entity, &MapItemOf)>().iter(world).map(|(e, of)| (of.0, e)).collect();
            found.sort();
            found
        };
        let spreads = |world: &mut World| world.query::<&MapSpreadOf>().iter(world).count();
        let everything = |world: &mut World| world.query::<&MapDrawn>().iter(world).count();

        draw(&mut world, &mut scene, &frame(vec![placed("kept", false), placed("left", true)]));
        let before = items(&mut world);
        // A bar and an arc, each with its two caps.
        assert_eq!((before.len(), spreads(&mut world)), (2, 6));
        let scenery = everything(&mut world) - 2 - 6;
        assert_eq!(scenery, MAX_RINGS + 1, "the ring pool and the spokes");

        draw(&mut world, &mut scene, &frame(vec![placed("kept", false), placed("came", false)]));
        let after = items(&mut world);
        let kept = ItemKey::from_name("kept");
        let entity_of = |list: &[(ItemKey, Entity)]| list.iter().find(|(k, _)| *k == kept).map(|(_, e)| *e);
        assert_eq!(entity_of(&after), entity_of(&before), "a kept item was respawned");
        assert!(after.iter().any(|(k, _)| *k == ItemKey::from_name("came")));
        assert!(!after.iter().any(|(k, _)| *k == ItemKey::from_name("left")));
        assert_eq!(spreads(&mut world), 0, "what left took its spread with it");
        assert_eq!(everything(&mut world), 2 + scenery, "the scenery was spawned again");
    }
}
