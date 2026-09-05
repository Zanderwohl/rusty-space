//! Sphere-of-influence shells.
//!
//! Drawn for the focused body and, optionally, its direct children — so a transfer into a
//! moon's sphere is visible before it is flown. The physics is
//! [`em_sim::influence`]; this module is only the picture of it.
//!
//! The meshes are built once and shared by every body. Nothing here rebuilds geometry:
//! a Hill sphere breathes over an eccentric orbit and the radius arrives as a uniform,
//! so the vertex buffers never change.

use std::collections::HashSet;

use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy_mesh::{Indices, PrimitiveTopology};
use em_sim::id::BodyIndex;
use em_sim::influence;

use crate::body::universe::save::ViewSettings;
use crate::gui::planetarium::FocusedBodyState;
use crate::sim::world::{BodyRef, SimSystem};

use super::render_space::ToRender;
use super::soi_material::{SoiPointsMaterial, SoiRingMaterial, ATTRIBUTE_SOI_CORNER};
use super::trajectory::calculate_tube_radius;

/// Dots on the shell. Enough to read as a surface, few enough to see through.
const POINT_COUNT: usize = 1024;

/// Stations around the limb ring. The silhouette of an anisotropic shell is a curve, not a
/// circle, so it needs enough segments to stay smooth under the per-station solve.
const RING_STATIONS: usize = 256;

/// Sides of the ring's tube cross-section, matching the trajectory tubes' economy.
const RING_TUBE_SIDES: usize = 6;

/// Below this apparent radius a shell is a smudge a few pixels across and only adds
/// clutter, so it fades out. Radians.
const MIN_ANGULAR_RADIUS: f64 = 0.004;

/// Above this a shell wraps past the field of view and carries no information — it would
/// just be a wall of dots — so it fades out at the top end too. Radians.
const MAX_ANGULAR_RADIUS: f64 = 1.2;

/// Below this the shell is not worth drawing at all.
const FADE_CUTOFF: f32 = 0.01;

/// How much the interior dots are lifted when the camera is inside the shell, where there
/// is no limb to carry the shape.
const INSIDE_INTERIOR_BOOST: f32 = 2.0;

/// The limb is drawn finer than a trajectory line. A trajectory is a path you follow; a
/// sphere of influence is a boundary you cross, and it should not compete with the orbits
/// inside it.
const RING_THICKNESS_SCALE: f32 = 0.3;

/// The shared unit meshes, built once and referenced by every SOI entity.
#[derive(Resource)]
pub struct SoiMeshes {
    pub points: Handle<Mesh>,
    pub ring: Handle<Mesh>,
}

impl FromWorld for SoiMeshes {
    fn from_world(world: &mut World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        Self {
            points: meshes.add(soi_point_cloud_mesh(POINT_COUNT)),
            ring: meshes.add(soi_ring_mesh(RING_STATIONS, RING_TUBE_SIDES)),
        }
    }
}

/// Marker for a point-cloud entity, pointing back at the body it belongs to.
#[derive(Component)]
pub struct SoiPointsMesh {
    pub body_entity: Entity,
}

/// Marker for a limb-ring entity, pointing back at the body it belongs to.
#[derive(Component)]
pub struct SoiRingMesh {
    pub body_entity: Entity,
}

/// On a body entity: the render entities drawing its sphere of influence.
#[derive(Component)]
pub struct SoiLink {
    pub points: Entity,
    pub ring: Entity,
}

/// A unit sphere of billboard quads on a Fibonacci lattice.
///
/// `POSITION` is the unit direction, repeated for all four corners of a quad;
/// `ATTRIBUTE_SOI_CORNER` distinguishes them. The vertex shader pushes each point out to
/// the shell radius for its direction, so this mesh is radius-agnostic and immortal.
fn soi_point_cloud_mesh(count: usize) -> Mesh {
    // The golden angle: successive points land as far from their predecessors as the
    // circle allows, which is what makes the lattice even rather than spiralled.
    let golden_angle = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());

    let mut positions = Vec::with_capacity(count * 4);
    let mut corners = Vec::with_capacity(count * 4);
    let mut indices = Vec::with_capacity(count * 6);

    for i in 0..count {
        // Uniform in z gives uniform area on the sphere; the offset keeps the poles off
        // the exact ends, where a lattice point would otherwise sit alone.
        let z = 1.0 - (2.0 * i as f32 + 1.0) / count as f32;
        let radius = (1.0 - z * z).max(0.0).sqrt();
        let theta = golden_angle * i as f32;
        let direction = [radius * theta.cos(), radius * theta.sin(), z];

        let base = (i * 4) as u32;
        for corner in [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]] {
            positions.push(direction);
            corners.push(corner);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(ATTRIBUTE_SOI_CORNER, corners);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A closed tube whose vertices carry two angles rather than a position.
///
/// `POSITION` holds `(cos phi, sin phi, 0)` for the station around the ring and `NORMAL`
/// holds `(cos c, sin c, 0)` for the position around the tube's cross-section. The vertex
/// shader solves for where that station's silhouette actually lies and builds the tube
/// frame there, so this geometry is independent of the shell's radius, its shape, and the
/// camera — which is what lets it be built once for every body in the universe.
///
/// POSITION and NORMAL are the stock attributes, so the default vertex layout applies.
/// `ATTRIBUTE_COLOR` is deliberately absent: it would switch on the `VERTEX_COLORS` shader
/// define and break prepass pipeline validation.
fn soi_ring_mesh(stations: usize, sides: usize) -> Mesh {
    let mut positions = Vec::with_capacity(stations * sides);
    let mut normals = Vec::with_capacity(stations * sides);
    let mut indices = Vec::with_capacity(stations * sides * 6);

    for station in 0..stations {
        let phi = std::f32::consts::TAU * station as f32 / stations as f32;
        let (sin_phi, cos_phi) = phi.sin_cos();

        for side in 0..sides {
            let c = std::f32::consts::TAU * side as f32 / sides as f32;
            let (sin_c, cos_c) = c.sin_cos();
            positions.push([cos_phi, sin_phi, 0.0]);
            normals.push([cos_c, sin_c, 0.0]);
        }

        // Closed: the last station stitches back to the first.
        let base = (station * sides) as u32;
        let next_base = (((station + 1) % stations) * sides) as u32;
        for side in 0..sides {
            let s = side as u32;
            let s_next = ((side + 1) % sides) as u32;
            indices.extend_from_slice(&[
                base + s, next_base + s, base + s_next,
                base + s_next, next_base + s, next_base + s_next,
            ]);
        }
    }

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Give every body an SOI render entity, hidden until something asks for it.
///
/// Spawning for all bodies rather than just the focused one keeps focus changes out of
/// the ECS structure entirely: showing a shell is a visibility decision made per frame.
pub fn spawn_soi_meshes(
    mut commands: Commands,
    bodies: Query<Entity, (With<BodyRef>, Without<SoiLink>)>,
    shared: Res<SoiMeshes>,
    mut point_materials: ResMut<Assets<SoiPointsMaterial>>,
    mut ring_materials: ResMut<Assets<SoiRingMaterial>>,
) {
    for body_entity in bodies.iter() {
        let points = commands.spawn((
            Mesh3d(shared.points.clone()),
            MeshMaterial3d(point_materials.add(SoiPointsMaterial::default())),
            Transform::default(),
            Visibility::Hidden,
            // The mesh is a unit sphere expanded in the vertex shader, so its computed
            // bounds bear no relation to where it actually draws.
            NoFrustumCulling,
            SoiPointsMesh { body_entity },
        )).id();

        let ring = commands.spawn((
            Mesh3d(shared.ring.clone()),
            MeshMaterial3d(ring_materials.add(SoiRingMaterial::default())),
            Transform::default(),
            Visibility::Hidden,
            NoFrustumCulling,
            SoiRingMesh { body_entity },
        )).id();

        commands.entity(body_entity).insert(SoiLink { points, ring });
    }
}

/// Point each shell at its body, size it, and decide whether it is worth drawing.
pub fn update_soi_shells(
    system: Res<SimSystem>,
    view_settings: Res<ViewSettings>,
    focused: Res<FocusedBodyState>,
    bodies: Query<(&Transform, &SoiLink, &BodyRef)>,
    // The three queries are kept provably disjoint by marker components: Bevy derives
    // that from `With`/`Without` alone, not from the material types differing.
    mut points: Query<
        (&mut Transform, &mut Visibility, &MeshMaterial3d<SoiPointsMaterial>),
        (With<SoiPointsMesh>, Without<SoiLink>),
    >,
    mut rings: Query<
        (&mut Transform, &mut Visibility, &MeshMaterial3d<SoiRingMaterial>),
        (With<SoiRingMesh>, Without<SoiPointsMesh>, Without<SoiLink>),
    >,
    mut materials: ResMut<Assets<SoiPointsMaterial>>,
    mut ring_materials: ResMut<Assets<SoiRingMaterial>>,
) {
    let scale = view_settings.distance_factor();

    // Which bodies get a shell this frame: the focused one, plus its direct children when
    // asked for. Anything else is hidden.
    let wanted: Vec<BodyIndex> = if !view_settings.show_spheres_of_influence {
        Vec::new()
    } else {
        match focused.current_body_id.as_deref().and_then(|id| system.0.by_name(id)) {
            Some(focus) => {
                let mut wanted = vec![focus];
                if view_settings.show_child_spheres_of_influence {
                    wanted.extend(system.0.children_of(focus));
                }
                wanted
            }
            None => Vec::new(),
        }
    };

    for (body_transform, link, body_ref) in bodies.iter() {
        let shell = system
            .0
            .index_of(body_ref.0)
            .filter(|i| wanted.contains(i))
            // A root, a massless body, or one whose model is `None` has no surface.
            .and_then(|i| influence::soi_now(&system.0, i));

        let Some(soi) = shell else {
            hide(&mut points, link.points);
            hide(&mut rings, link.ring);
            continue;
        };

        let distance = body_transform.translation.length() as f64;
        let bounding = soi.bounding_radius() * scale;
        let apparent = (bounding / distance.max(f64::EPSILON)).atan();

        // One curve covers a small moon's 1e6 m sphere and a star's 1e13 m one, because
        // it gates on apparent size rather than on any physical radius.
        let fade = (smoothstep(MIN_ANGULAR_RADIUS, MIN_ANGULAR_RADIUS * 3.0, apparent)
            * (1.0 - smoothstep(MAX_ANGULAR_RADIUS, MAX_ANGULAR_RADIUS * 1.5, apparent)))
            as f32;

        if fade < FADE_CUTOFF {
            hide(&mut points, link.points);
            hide(&mut rings, link.ring);
            continue;
        }

        // From inside the shell there is no silhouette to draw at all, so the dots that
        // face the camera have to carry the shape on their own.
        let inside = distance <= bounding;
        let shape = shape_uniform(&soi, scale, fade);

        // --- point cloud ---
        if let Ok((mut transform, mut visibility, handle)) = points.get_mut(link.points) {
            // The body's translation is already camera-relative and scaled; the shell
            // shares it. Scale stays 1 — all sizing lives in the uniform, so the mesh is
            // never rebuilt.
            transform.translation = body_transform.translation;
            *visibility = Visibility::Visible;

            let defaults = SoiPointsMaterial::default().uniform;
            let interior_alpha = if inside {
                defaults.interior_alpha * INSIDE_INTERIOR_BOOST
            } else {
                defaults.interior_alpha
            };

            if let Some(current) = materials.get(handle.id()) {
                // `get_mut` flags the asset for re-upload, so only take it when something
                // actually moved.
                if shape_changed(&current.uniform.shape, &shape)
                    || (current.uniform.interior_alpha - interior_alpha).abs() > 0.001
                {
                    if let Some(material) = materials.get_mut(handle.id()) {
                        material.uniform.shape = shape.clone();
                        material.uniform.interior_alpha = interior_alpha;
                    }
                }
            }
        }

        // --- limb ring ---
        if let Ok((mut transform, mut visibility, handle)) = rings.get_mut(link.ring) {
            transform.translation = body_transform.translation;
            *visibility = if inside { Visibility::Hidden } else { Visibility::Visible };

            // Keyed off the trajectory line weight, so the two share a drawing language,
            // then thinned by `RING_THICKNESS_SCALE`.
            let tube_radius =
                calculate_tube_radius(body_transform.translation.length()) * RING_THICKNESS_SCALE;

            if let Some(current) = ring_materials.get(handle.id()) {
                if shape_changed(&current.uniform.shape, &shape)
                    || (current.uniform.tube_radius - tube_radius).abs() > 0.000001
                {
                    if let Some(material) = ring_materials.get_mut(handle.id()) {
                        material.uniform.shape = shape.clone();
                        material.uniform.tube_radius = tube_radius;
                    }
                }
            }
        }
    }
}

/// Hide one render entity, whatever material it carries.
fn hide<M: Component>(
    query: &mut Query<(&mut Transform, &mut Visibility, &M), impl bevy::ecs::query::QueryFilter>,
    entity: Entity,
) {
    if let Ok((_, mut visibility, _)) = query.get_mut(entity) {
        *visibility = Visibility::Hidden;
    }
}

/// Pack an [`influence::Soi`] into the form the shaders read.
fn shape_uniform(soi: &influence::Soi, scale: f64, fade: f32) -> super::soi_material::SoiShapeUniform {
    super::soi_material::SoiShapeUniform {
        // Sim space is Z-up and the renderer is Y-up; `ToRender` is the only conversion.
        primary_dir: soi.to_primary.to_render().normalize_or_zero().extend(0.0),
        model: model_index(soi.model),
        radius: (soi.bounding_radius() * scale) as f32,
        bounding_radius: (soi.bounding_radius() * scale) as f32,
        min_radius: (soi.min_radius() * scale) as f32,
        anisotropic: if soi.is_isotropic() { 0.0 } else { 1.0 },
        fade,
    }
}

/// The discriminant the WGSL side switches on. Must match `SoiModel`'s declaration order
/// and the `MODEL_*` constants in `shaders/soi_shape.wgsl`.
fn model_index(model: influence::SoiModel) -> u32 {
    use influence::SoiModel::*;
    match model {
        None => 0,
        Hill => 1,
        HillPeriapsis => 2,
        Laplace => 3,
        LaplaceIntegrated => 4,
        LaplaceAngled => 5,
        Bondi { .. } => 6,
        Fixed(_) => 7,
    }
}

fn shape_changed(
    a: &super::soi_material::SoiShapeUniform,
    b: &super::soi_material::SoiShapeUniform,
) -> bool {
    a.model != b.model
        || (a.radius - b.radius).abs() > 0.001
        || (a.min_radius - b.min_radius).abs() > 0.001
        || (a.fade - b.fade).abs() > 0.001
        || (a.anisotropic - b.anisotropic).abs() > 0.001
        || (a.primary_dir - b.primary_dir).length() > 0.001
}

/// Hermite fade between two thresholds. Not in `std`, and `f32::smoothstep` is unstable.
fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Despawn shells whose body has gone.
pub fn cleanup_orphaned_soi_meshes(
    mut commands: Commands,
    mut removed_bodies: RemovedComponents<BodyRef>,
    point_clouds: Query<(Entity, &SoiPointsMesh)>,
    rings: Query<(Entity, &SoiRingMesh)>,
) {
    if removed_bodies.is_empty() {
        return;
    }
    let removed: HashSet<Entity> = removed_bodies.read().collect();
    for (entity, shell) in point_clouds.iter() {
        if removed.contains(&shell.body_entity) {
            commands.entity(entity).despawn();
        }
    }
    for (entity, shell) in rings.iter() {
        if removed.contains(&shell.body_entity) {
            commands.entity(entity).despawn();
        }
    }
}
