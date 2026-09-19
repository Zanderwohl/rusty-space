//! Body wireframe mesh generation and systems.
//!
//! Generates lat/lon grid tube meshes for bodies and terminator circles
//! that show the day/night boundary relative to each star.

use std::collections::HashSet;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::color::LinearRgba;
use bevy::prelude::*;

use crate::body::appearance::Appearance;
use crate::sim::world::{BodyRef, SimSystem};
use crate::camera::PlanetariumCamera;

use em_render::wire_mesh::{generate_great_circle_tube, generate_latlon_sphere};

use super::body_material::{BodyWireframeMaterial, OccluderMaterial, MAX_SUNS};
use super::star_cache::StarLightingFrameCache;

/// Number of sides for tube cross-section
const TUBE_SIDES: u32 = 4;

/// Radius of the wireframe tubes (relative to unit sphere)
const WIRE_TUBE_RADIUS: f32 = 0.012;

/// Emission strength for body wireframes
const BODY_EMISSION_STRENGTH: f32 = 3.0;

/// Emission strength for terminator (subtler)
const TERMINATOR_EMISSION_STRENGTH: f32 = 1.5;

/// Screen radius (px) at which terminators begin fading out
const TERMINATOR_FADE_START_PX: f32 = 25.0;

/// Screen radius (px) at which terminators are fully hidden
const TERMINATOR_FADE_END_PX: f32 = 12.0;

/// Terminator color (gentle warm yellow)
const TERMINATOR_COLOR: LinearRgba = LinearRgba::new(0.95, 0.85, 0.4, 1.0);

/// Minimum angular size for wireframe tubes to prevent sub-pixel aliasing.
/// This is the minimum tube_radius / distance ratio.
const MIN_ANGULAR_SIZE: f32 = 0.0008;

/// Marker component for body wireframe mesh entities.
#[derive(Component)]
pub struct BodyWireframeMesh {
    pub body_entity: Entity,
    /// Stored highlight latitudes for mesh regeneration
    pub highlight_latitudes: Vec<f64>,
}

/// Component linking a body to its wireframe mesh entity.
#[derive(Component)]
pub struct BodyWireframeLink(pub Entity);

/// Marker component for terminator mesh entities.
#[derive(Component)]
pub struct TerminatorMesh {
    pub body_entity: Entity,
    pub star_entity: Entity,
}

/// Component linking a body to its terminator mesh entities.
#[derive(Component)]
pub struct TerminatorLinks(pub Vec<Entity>);

/// Marker component for body occluder mesh entities.
#[derive(Component)]
pub struct OccluderMesh {
    pub body_entity: Entity,
}

/// Component linking a body to its occluder mesh entity.
#[derive(Component)]
pub struct OccluderLink(pub Entity);

/// System to spawn body wireframe meshes for DebugBall bodies.
pub fn spawn_body_wireframe_meshes(
    mut commands: Commands,
    bodies: Query<(Entity, &BodyRef), Without<BodyWireframeLink>>,
    system: Res<SimSystem>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
) {
    for (body_entity, body_ref) in bodies.iter() {
        let Some(bi) = system.0.index_of(body_ref.0) else { continue };
        let appearance = system.0.appearance(bi);
        if let Appearance::DebugBall(debug_ball) = appearance {
            let highlight_lats = debug_ball.highlight_latitudes();
            let mesh = generate_latlon_sphere(&highlight_lats, WIRE_TUBE_RADIUS, TUBE_SIDES);
            let mesh_handle = meshes.add(mesh);

            let color = LinearRgba::new(
                debug_ball.color.r as f32 / 255.0,
                debug_ball.color.g as f32 / 255.0,
                debug_ball.color.b as f32 / 255.0,
                1.0,
            );
            let material = BodyWireframeMaterial {
                base_color: color,
                emission_strength: BODY_EMISSION_STRENGTH,
                alpha_mode: AlphaMode::Opaque,
                ..Default::default()
            };
            let material_handle = materials.add(material);

            let wireframe_entity = commands
                .spawn((
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material_handle),
                    Transform::default(),
                    Visibility::Inherited,
                    NoFrustumCulling,
                    BodyWireframeMesh {
                        body_entity,
                        highlight_latitudes: highlight_lats.clone(),
                    },
                    ChildOf(body_entity),
                ))
                .id();

            commands.entity(body_entity).insert(BodyWireframeLink(wireframe_entity));
        }
    }
}

/// System to spawn occluder meshes as child entities for DebugBall bodies.
/// This allows independent scaling of the occluder to avoid z-fighting at distance.
pub fn spawn_body_occluders(
    mut commands: Commands,
    bodies: Query<(Entity, &BodyRef), Without<OccluderLink>>,
    system: Res<SimSystem>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<OccluderMaterial>>,
) {
    for (body_entity, body_ref) in bodies.iter() {
        let Some(bi) = system.0.index_of(body_ref.0) else { continue };
        let appearance = system.0.appearance(bi);
        if let Appearance::DebugBall(_) = appearance {
            let mesh = Sphere::new(0.97f32).mesh().ico(5).unwrap();
            let mesh_handle = meshes.add(mesh);
            let material_handle = materials.add(OccluderMaterial {
                alpha_mode: AlphaMode::Blend,
                ..Default::default()
            });

            let occluder_entity = commands
                .spawn((
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material_handle),
                    Transform::default(),
                    Visibility::Inherited,
                    OccluderMesh { body_entity },
                    ChildOf(body_entity),
                ))
                .id();

            commands.entity(body_entity).insert(OccluderLink(occluder_entity));
        }
    }
}

/// System to scale occluder meshes slightly smaller at distance to avoid z-fighting.
pub fn update_occluder_scale(
    bodies: Query<Option<&BodyWireframeLink>, With<BodyRef>>,
    wireframes: Query<&MeshMaterial3d<BodyWireframeMaterial>, With<BodyWireframeMesh>>,
    materials: Res<Assets<BodyWireframeMaterial>>,
    mut occluders: Query<(&OccluderMesh, &mut Transform, &ChildOf), Without<BodyRef>>,
) {
    for (_occluder, mut occluder_transform, child_of) in occluders.iter_mut() {
        let Ok(wireframe_link) = bodies.get(child_of.parent()) else {
            continue;
        };

        // Get the wireframe's current tube radius from its material uniform
        let tube_radius_ratio = wireframe_link
            .and_then(|link| wireframes.get(link.0).ok())
            .and_then(|mat_handle| materials.get(mat_handle.id()))
            .map(|mat| mat.target_tube_radius / mat.base_tube_radius)
            .unwrap_or(1.0);

        // Scale occluder down as tube thickness increases.
        // At base radius: scale = 1.0, at larger radii: shrink slightly
        let scale_reduction = (tube_radius_ratio - 1.0) * 0.01; // 1% per doubling
        let occluder_scale = (1.0 - scale_reduction).max(0.9); // Don't shrink more than 10%
        occluder_transform.scale = Vec3::splat(occluder_scale);
    }
}

/// System to spawn terminator meshes for each star-body pair.
pub fn spawn_terminator_meshes(
    mut commands: Commands,
    bodies: Query<(Entity, &BodyRef), Without<TerminatorLinks>>,
    system: Res<SimSystem>,
    stars: Query<(Entity, &BodyRef)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
) {
    // Find all star entities
    let star_entities: Vec<Entity> = stars
        .iter()
        .filter(|(_, body_ref)| {
            system.0.index_of(body_ref.0)
                .is_some_and(|i| matches!(system.0.appearance(i), Appearance::Star(_)))
        })
        .map(|(entity, _)| entity)
        .collect();

    if star_entities.is_empty() {
        return;
    }

    for (body_entity, body_ref) in bodies.iter() {
        let Some(bi) = system.0.index_of(body_ref.0) else { continue };
        let appearance = system.0.appearance(bi);
        if let Appearance::DebugBall(_) = appearance {
            let mut terminator_entities = Vec::new();

            for &star_entity in &star_entities {
                // Skip if body is the star itself
                if body_entity == star_entity {
                    continue;
                }

                // Create initial mesh with canonical direction (orientation comes from Transform)
                let mesh = generate_great_circle_tube(TERMINATOR_CANONICAL_DIR, WIRE_TUBE_RADIUS, TUBE_SIDES);
                let mesh_handle = meshes.add(mesh);

                let material = BodyWireframeMaterial {
                    base_color: TERMINATOR_COLOR,
                    emission_strength: TERMINATOR_EMISSION_STRENGTH,
                    alpha_mode: AlphaMode::Opaque,
                    ..Default::default()
                };
                let material_handle = materials.add(material);

                let terminator_entity = commands
                    .spawn((
                        Mesh3d(mesh_handle),
                        MeshMaterial3d(material_handle),
                        Transform::default(),
                        Visibility::Inherited,
                        NoFrustumCulling,
                        TerminatorMesh {
                            body_entity,
                            star_entity,
                        },
                        ChildOf(body_entity),
                    ))
                    .id();

                terminator_entities.push(terminator_entity);
            }

            commands.entity(body_entity).insert(TerminatorLinks(terminator_entities));
        }
    }
}

/// Canonical direction for terminator mesh generation.
/// The mesh is generated with this normal, then rotated via Transform to the actual star direction.
const TERMINATOR_CANONICAL_DIR: Vec3 = Vec3::Y;
const SUN_DIR_EPSILON: f32 = 1e-4;

fn vec4_approx_eq(a: Vec4, b: Vec4, epsilon: f32) -> bool {
    (a - b).abs().max_element() <= epsilon
}

/// System to update terminator meshes based on star positions.
/// Tube thickness is updated via material uniform (vertex-shader displacement).
/// Rotation is updated every frame via Transform (cheap).
/// Also fades terminators out when the body is too small on screen.
pub fn update_terminator_meshes(
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<PlanetariumCamera>>,
    mut terminators: Query<(&TerminatorMesh, &ChildOf, &mut Visibility, &MeshMaterial3d<BodyWireframeMaterial>, &mut Transform)>,
    bodies: Query<(&Transform, &BodyRef), Without<TerminatorMesh>>,
    system: Res<SimSystem>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
) {
    let Ok((camera, camera_global, projection)) = cameras.single() else {
        return;
    };

    let Some(viewport_size) = camera.logical_viewport_size() else {
        return;
    };

    let fov_y = match projection {
        Projection::Perspective(persp) => persp.fov,
        _ => 1.0,
    };

    let camera_pos = camera_global.translation();

    for (terminator, child_of, mut visibility, material_handle, mut term_transform) in terminators.iter_mut() {
        let Ok((body_transform, body_ref)) = bodies.get(child_of.parent()) else {
            continue;
        };
        let Some(body_i) = system.0.index_of(body_ref.0) else { continue };
        let body_position = system.0.position(body_i);

        let body_center = body_transform.translation;
        let distance = (body_center - camera_pos).length();
        if distance <= 0.0 {
            continue;
        }

        let body_scale = body_transform.scale.x;
        let angular_radius = (body_scale / distance).min(1.0);
        let screen_radius = angular_radius / (fov_y * 0.5) * viewport_size.y * 0.5;

        if screen_radius <= TERMINATOR_FADE_END_PX {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        }

        if *visibility != Visibility::Visible {
            *visibility = Visibility::Visible;
        }

        let fade = if screen_radius >= TERMINATOR_FADE_START_PX {
            1.0
        } else {
            (screen_radius - TERMINATOR_FADE_END_PX)
                / (TERMINATOR_FADE_START_PX - TERMINATOR_FADE_END_PX)
        };

        // Get star state
        let Ok((_, star_ref)) = bodies.get(terminator.star_entity) else {
            continue;
        };
        let Some(star_i) = system.0.index_of(star_ref.0) else { continue };
        let star_position = system.0.position(star_i);

        let tube_radius = calculate_tube_radius_for_distance(body_scale, distance);

        // Compute star direction in simulation space (Z-up)
        let star_dir_sim = (star_position - body_position).normalize();

        // Convert from simulation space (Z-up) to Bevy space (Y-up): (x, y, z) -> (x, z, -y)
        let star_dir_bevy = Vec3::new(
            star_dir_sim.x as f32,
            star_dir_sim.z as f32,
            -star_dir_sim.y as f32,
        );

        let star_dir_local = body_transform.rotation.inverse() * star_dir_bevy;

        // Always update transform rotation to orient the terminator toward the star.
        // This is cheap and ensures smooth rotation as the body spins.
        let rotation = Quat::from_rotation_arc(TERMINATOR_CANONICAL_DIR, star_dir_local);
        term_transform.rotation = rotation;

        // Update material uniform for emission fade and tube radius (with tolerance checks)
        let new_emission = TERMINATOR_EMISSION_STRENGTH * fade;
        if let Some(mat) = materials.get(material_handle.id()) {
            let emission_changed = (mat.emission_strength - new_emission).abs() > 0.001;
            let radius_ratio = tube_radius / mat.target_tube_radius;
            let radius_changed = radius_ratio < 0.95 || radius_ratio > 1.05;

            if emission_changed || radius_changed {
                if let Some(mut mat) = materials.get_mut(material_handle.id()) {
                    mat.emission_strength = new_emission;
                    mat.target_tube_radius = tube_radius;
                }
            }
        }
    }
}

/// System to clean up orphaned wireframe and terminator entities.
/// Uses RemovedComponents to only run when bodies are actually removed.
pub fn cleanup_orphaned_body_wireframes(
    mut commands: Commands,
    mut removed_bodies: RemovedComponents<BodyRef>,
    wireframes: Query<(Entity, &BodyWireframeMesh)>,
    terminators: Query<(Entity, &TerminatorMesh)>,
) {
    // Early exit if no bodies were removed
    if removed_bodies.is_empty() {
        return;
    }
    
    // Collect removed body entities
    let removed: std::collections::HashSet<Entity> = removed_bodies.read().collect();
    
    // Clean up wireframes
    for (entity, wireframe) in wireframes.iter() {
        if removed.contains(&wireframe.body_entity) {
            commands.entity(entity).despawn();
        }
    }

    // Clean up terminators
    for (entity, terminator) in terminators.iter() {
        if removed.contains(&terminator.body_entity) {
            commands.entity(entity).despawn();
        }
    }
}

/// Maximum tube radius (in local coordinates) to prevent rendering issues at extreme distances.
/// Beyond this, the point shader takes over anyway.
const MAX_TUBE_RADIUS: f32 = 0.15;

/// Calculate the tube radius needed to maintain minimum angular size at the given distance.
/// Returns a radius in local (unit sphere) coordinates, clamped to reasonable bounds.
fn calculate_tube_radius_for_distance(body_scale: f32, distance: f32) -> f32 {
    if distance <= 0.0 || body_scale <= 0.0 {
        return WIRE_TUBE_RADIUS;
    }

    // For minimum angular size:
    // tube_radius_world / distance >= MIN_ANGULAR_SIZE
    // tube_radius_local * body_scale / distance >= MIN_ANGULAR_SIZE
    // tube_radius_local >= MIN_ANGULAR_SIZE * distance / body_scale
    let min_local_radius = MIN_ANGULAR_SIZE * distance / body_scale;

    // Clamp between base radius and maximum
    min_local_radius.clamp(WIRE_TUBE_RADIUS, MAX_TUBE_RADIUS)
}

/// System to update wireframe materials with sun directions for day/night shading.
/// Computes star directions in body-local space and writes them into the material uniform.
pub fn update_wireframe_lighting(
    wireframes: Query<(&BodyWireframeMesh, &MeshMaterial3d<BodyWireframeMaterial>, &ChildOf)>,
    bodies: Query<(&Transform, &BodyRef)>,
    system: Res<SimSystem>,
    star_cache: Res<StarLightingFrameCache>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
) {
    for (wireframe, material_handle, child_of) in wireframes.iter() {
        let Ok((body_transform, body_ref)) = bodies.get(child_of.parent()) else {
            continue;
        };
        let Some(body_i) = system.0.index_of(body_ref.0) else { continue };
        let body_position = system.0.position(body_i);

        let mut num_suns = 0u32;
        let mut sun_dirs = [Vec4::ZERO; MAX_SUNS];

        for star in &star_cache.stars {
            if star.entity == wireframe.body_entity {
                continue;
            }
            if num_suns as usize >= MAX_SUNS {
                break;
            }

            let star_dir_sim =
                (star.sim_position - body_position).normalize();
            let star_dir_bevy = Vec3::new(
                star_dir_sim.x as f32,
                star_dir_sim.z as f32,
                -star_dir_sim.y as f32,
            );
            let star_dir_local = body_transform.rotation.inverse() * star_dir_bevy;

            sun_dirs[num_suns as usize] = star_dir_local.extend(0.0);
            num_suns += 1;
        }

        let needs_update = if let Some(mat) = materials.get(material_handle.id()) {
            mat.num_suns != num_suns
                || !vec4_approx_eq(mat.sun_dir_0, sun_dirs[0], SUN_DIR_EPSILON)
                || !vec4_approx_eq(mat.sun_dir_1, sun_dirs[1], SUN_DIR_EPSILON)
                || !vec4_approx_eq(mat.sun_dir_2, sun_dirs[2], SUN_DIR_EPSILON)
                || !vec4_approx_eq(mat.sun_dir_3, sun_dirs[3], SUN_DIR_EPSILON)
        } else {
            false
        };

        if needs_update {
            if let Some(mut mat) = materials.get_mut(material_handle.id()) {
                mat.num_suns = num_suns;
                mat.sun_dir_0 = sun_dirs[0];
                mat.sun_dir_1 = sun_dirs[1];
                mat.sun_dir_2 = sun_dirs[2];
                mat.sun_dir_3 = sun_dirs[3];
            }
        }
    }
}

/// System to update occluder materials with sun directions for Lambert shading.
pub fn update_occluder_lighting(
    occluders: Query<&MeshMaterial3d<OccluderMaterial>, With<OccluderMesh>>,
    star_cache: Res<StarLightingFrameCache>,
    mut materials: ResMut<Assets<OccluderMaterial>>,
) {
    if occluders.is_empty() {
        return;
    }

    let mut num_suns = 0u32;
    let mut sun_positions = [Vec4::ZERO; MAX_SUNS];

    for star in &star_cache.stars {
        if num_suns as usize >= MAX_SUNS {
            break;
        }

        sun_positions[num_suns as usize] = star.bevy_position.extend(0.0);
        num_suns += 1;
    }

    let mut updated_materials = HashSet::new();
    for material_handle in occluders.iter() {
        if !updated_materials.insert(material_handle.id()) {
            continue;
        }

        let needs_update = if let Some(mat) = materials.get(material_handle.id()) {
            mat.num_suns != num_suns
                || !vec4_approx_eq(mat.sun_pos_0, sun_positions[0], SUN_DIR_EPSILON)
                || !vec4_approx_eq(mat.sun_pos_1, sun_positions[1], SUN_DIR_EPSILON)
                || !vec4_approx_eq(mat.sun_pos_2, sun_positions[2], SUN_DIR_EPSILON)
                || !vec4_approx_eq(mat.sun_pos_3, sun_positions[3], SUN_DIR_EPSILON)
        } else {
            false
        };

        if needs_update {
            if let Some(mut mat) = materials.get_mut(material_handle.id()) {
                mat.num_suns = num_suns;
                mat.sun_pos_0 = sun_positions[0];
                mat.sun_pos_1 = sun_positions[1];
                mat.sun_pos_2 = sun_positions[2];
                mat.sun_pos_3 = sun_positions[3];
            }
        }
    }
}

/// System to update wireframe tube thickness based on distance from camera.
/// Updates the material uniform to drive vertex-shader displacement instead of regenerating meshes.
pub fn update_wireframe_thickness(
    cameras: Query<&GlobalTransform, With<PlanetariumCamera>>,
    bodies: Query<&Transform>,
    wireframes: Query<(&BodyWireframeMesh, &MeshMaterial3d<BodyWireframeMaterial>, &ChildOf)>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
) {
    let Ok(camera_global) = cameras.single() else {
        return;
    };
    let camera_pos = camera_global.translation();

    for (_wireframe, material_handle, child_of) in wireframes.iter() {
        let Ok(body_transform) = bodies.get(child_of.parent()) else {
            continue;
        };

        let body_center = body_transform.translation;
        let distance = (body_center - camera_pos).length();
        let body_scale = body_transform.scale.x;

        let required_radius = calculate_tube_radius_for_distance(body_scale, distance);

        // Update material uniform if radius changed (with tolerance to avoid unnecessary uploads)
        let needs_update = materials
            .get(material_handle.id())
            .map(|m| {
                let ratio = required_radius / m.target_tube_radius;
                ratio < 0.95 || ratio > 1.05
            })
            .unwrap_or(false);

        if needs_update {
            if let Some(mut mat) = materials.get_mut(material_handle.id()) {
                mat.target_tube_radius = required_radius;
            }
        }
    }
}
