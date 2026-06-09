//! Body wireframe mesh generation and systems.
//!
//! Generates lat/lon grid tube meshes for bodies and terminator circles
//! that show the day/night boundary relative to each star.

use std::f32::consts::PI;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::color::LinearRgba;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};

use crate::body::appearance::Appearance;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::camera::PlanetariumCamera;

use super::body_material::{BodyWireframeMaterial, OccluderMaterial, MAX_SUNS};
use super::star_cache::StarLightingFrameCache;

/// Number of sides for tube cross-section
const TUBE_SIDES: u32 = 4;

/// Radius of the wireframe tubes (relative to unit sphere)
const WIRE_TUBE_RADIUS: f32 = 0.012;

/// Number of points per circle/parallel
const POINTS_PER_CIRCLE: u32 = 36;

/// Number of points per meridian (half-circle from pole to pole)
const POINTS_PER_MERIDIAN: u32 = 19;

/// Latitude spacing in degrees
const LAT_SPACING: f32 = 30.0;

/// Longitude spacing in degrees  
const LON_SPACING: f32 = 30.0;

/// Brightness for regular grid lines
const BRIGHTNESS_REGULAR: f32 = 0.6;

/// Brightness for highlight latitudes (tropics, circles)
const BRIGHTNESS_HIGHLIGHT: f32 = 0.85;

/// Brightness for equator and prime meridian
const BRIGHTNESS_PRIMARY: f32 = 1.0;

/// How far the pole skewer extends beyond the sphere (as a multiplier of radius)
const POLE_EXTENSION: f32 = 3.0;

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
    /// Current tube radius used in the mesh (for change detection)
    pub current_tube_radius: f32,
}

/// Component linking a body to its wireframe mesh entity.
#[derive(Component)]
pub struct BodyWireframeLink(pub Entity);

/// Marker component for terminator mesh entities.
#[derive(Component)]
pub struct TerminatorMesh {
    pub body_entity: Entity,
    pub star_entity: Entity,
    /// Current tube radius used in the mesh (for change detection)
    pub current_tube_radius: f32,
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

/// Find two perpendicular vectors to form a plane orthogonal to the given direction.
fn perpendicular_vectors(dir: Vec3) -> (Vec3, Vec3) {
    let not_parallel = if dir.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };

    let perp1 = dir.cross(not_parallel).normalize_or_zero();
    let perp2 = dir.cross(perp1).normalize_or_zero();

    (perp1, perp2)
}

/// Build tube geometry from a sequence of points.
/// Returns (positions, normals, colors, indices) buffers.
/// If `closed` is true, connects the last point back to the first.
fn build_tube_from_points(
    points: &[Vec3],
    brightness: f32,
    tube_radius: f32,
    tube_sides: u32,
    closed: bool,
    index_offset: u32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<u32>) {
    if points.len() < 2 {
        return (vec![], vec![], vec![], vec![]);
    }

    let ring_count = points.len();
    let verts_per_ring = tube_sides as usize;
    let total_verts = ring_count * verts_per_ring;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(total_verts);

    let connection_count = if closed { ring_count } else { ring_count - 1 };
    let mut indices: Vec<u32> = Vec::with_capacity(connection_count * verts_per_ring * 6);

    for (ring_idx, center) in points.iter().enumerate() {
        let tangent = if ring_idx == 0 {
            if closed {
                (points[1] - points[ring_count - 1]).normalize_or_zero()
            } else {
                (points[1] - *center).normalize_or_zero()
            }
        } else if ring_idx == ring_count - 1 {
            if closed {
                (points[0] - points[ring_idx - 1]).normalize_or_zero()
            } else {
                (*center - points[ring_idx - 1]).normalize_or_zero()
            }
        } else {
            (points[ring_idx + 1] - points[ring_idx - 1]).normalize_or_zero()
        };

        let (perp1, perp2) = perpendicular_vectors(tangent);

        for i in 0..tube_sides {
            let angle = (i as f32 / tube_sides as f32) * 2.0 * PI;
            let (sin_a, cos_a) = angle.sin_cos();

            let offset = perp1 * cos_a * tube_radius + perp2 * sin_a * tube_radius;
            let pos = *center + offset;
            positions.push([pos.x, pos.y, pos.z]);

            let normal = offset.normalize_or_zero();
            normals.push([normal.x, normal.y, normal.z]);

            colors.push([1.0, 1.0, 1.0, brightness]);
        }

        let should_connect = if closed {
            true
        } else {
            ring_idx < ring_count - 1
        };

        if should_connect {
            let base = index_offset + (ring_idx * verts_per_ring) as u32;
            let next_ring_idx = if ring_idx == ring_count - 1 { 0 } else { ring_idx + 1 };
            let next_base = index_offset + (next_ring_idx * verts_per_ring) as u32;

            for i in 0..tube_sides {
                let i_next = (i + 1) % tube_sides;

                indices.push(base + i);
                indices.push(next_base + i);
                indices.push(base + i_next);

                indices.push(base + i_next);
                indices.push(next_base + i);
                indices.push(next_base + i_next);
            }
        }
    }

    (positions, normals, colors, indices)
}

/// Build an empty mesh that still declares the vertex layout required by
/// `body_wireframe.wgsl` (`position`, `normal`, `color`).
fn empty_wireframe_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(Vec::new()),
    );
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

/// Convert latitude/longitude (in radians) to a point on the unit sphere.
/// Uses Bevy Y-up convention: +Y = north pole, +X = prime meridian (lon=0).
fn latlon_to_point(lat: f32, lon: f32) -> Vec3 {
    let cos_lat = lat.cos();
    Vec3::new(
        cos_lat * lon.cos(),
        lat.sin(),
        cos_lat * lon.sin(),
    )
}

/// Generate a parallel (latitude circle) as a sequence of points.
fn generate_parallel(lat_deg: f32, num_points: u32) -> Vec<Vec3> {
    let lat = lat_deg.to_radians();
    (0..num_points)
        .map(|i| {
            let lon = (i as f32 / num_points as f32) * 2.0 * PI;
            latlon_to_point(lat, lon)
        })
        .collect()
}

/// Generate a meridian (longitude half-circle from south to north pole) as a sequence of points.
fn generate_meridian(lon_deg: f32, num_points: u32) -> Vec<Vec3> {
    let lon = lon_deg.to_radians();
    (0..num_points)
        .map(|i| {
            let lat = -PI / 2.0 + (i as f32 / (num_points - 1) as f32) * PI;
            latlon_to_point(lat, lon)
        })
        .collect()
}

/// Generate a lat/lon wireframe sphere mesh.
/// `highlight_latitudes` are additional latitudes (in degrees, positive only) to draw with
/// intermediate brightness. Both +lat and -lat are drawn.
pub fn generate_latlon_sphere(highlight_latitudes: &[f64], tube_radius: f32, tube_sides: u32) -> Mesh {
    let mut all_positions: Vec<[f32; 3]> = Vec::new();
    let mut all_normals: Vec<[f32; 3]> = Vec::new();
    let mut all_colors: Vec<[f32; 4]> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();

    let mut add_tube = |points: &[Vec3], brightness: f32, closed: bool| {
        let offset = all_positions.len() as u32;
        let (pos, norm, col, idx) = build_tube_from_points(points, brightness, tube_radius, tube_sides, closed, offset);
        all_positions.extend(pos);
        all_normals.extend(norm);
        all_colors.extend(col);
        all_indices.extend(idx);
    };

    // Collect all latitudes to draw
    let mut latitudes: Vec<(f32, f32)> = Vec::new(); // (lat_deg, brightness)

    // Standard parallels at 30-degree intervals (excluding poles)
    let mut lat = -60.0f32;
    while lat <= 60.0 {
        let brightness = if lat.abs() < 0.01 {
            BRIGHTNESS_PRIMARY // Equator
        } else {
            BRIGHTNESS_REGULAR
        };
        latitudes.push((lat, brightness));
        lat += LAT_SPACING;
    }

    // Add highlight latitudes (both positive and negative)
    for &hl in highlight_latitudes {
        let hl = hl as f32;
        if hl > 0.0 {
            // Check if it's not already close to an existing latitude
            let already_exists = latitudes.iter().any(|(l, _)| (l - hl).abs() < 1.0 || (l + hl).abs() < 1.0);
            if !already_exists {
                latitudes.push((hl, BRIGHTNESS_HIGHLIGHT));
                latitudes.push((-hl, BRIGHTNESS_HIGHLIGHT));
            }
        }
    }

    // Sort by latitude for consistent ordering
    latitudes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Generate parallels
    for (lat_deg, brightness) in &latitudes {
        let points = generate_parallel(*lat_deg, POINTS_PER_CIRCLE);
        add_tube(&points, *brightness, true);
    }

    // Generate meridians
    let num_meridians = (360.0 / LON_SPACING) as u32;
    for i in 0..num_meridians {
        let lon_deg = i as f32 * LON_SPACING;
        let brightness = if lon_deg.abs() < 0.01 {
            BRIGHTNESS_PRIMARY // Prime meridian
        } else {
            BRIGHTNESS_REGULAR
        };
        let points = generate_meridian(lon_deg, POINTS_PER_MERIDIAN);
        add_tube(&points, brightness, false);
    }

    // Generate pole skewer (axis through north and south poles, extending beyond sphere)
    let pole_points = vec![
        Vec3::new(0.0, -POLE_EXTENSION, 0.0), // South extension
        Vec3::new(0.0, -1.0, 0.0),            // South pole
        Vec3::new(0.0, 1.0, 0.0),             // North pole
        Vec3::new(0.0, POLE_EXTENSION, 0.0),  // North extension
    ];
    add_tube(&pole_points, BRIGHTNESS_PRIMARY, false);

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, all_positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, all_normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(all_colors));
    mesh.insert_indices(Indices::U32(all_indices));

    mesh
}

/// Generate a great circle tube mesh perpendicular to the given normal vector.
/// Used for terminator lines.
pub fn generate_great_circle_tube(normal: Vec3, tube_radius: f32, tube_sides: u32) -> Mesh {
    let normal = normal.normalize_or_zero();
    if normal.length_squared() < 0.001 {
        return empty_wireframe_mesh();
    }

    // Find two perpendicular vectors in the great circle plane
    let (perp1, perp2) = perpendicular_vectors(normal);

    // Generate points around the great circle at unit radius
    let points: Vec<Vec3> = (0..POINTS_PER_CIRCLE)
        .map(|i| {
            let angle = (i as f32 / POINTS_PER_CIRCLE as f32) * 2.0 * PI;
            let (sin_a, cos_a) = angle.sin_cos();
            perp1 * cos_a + perp2 * sin_a
        })
        .collect();

    let (positions, normals, colors, indices) =
        build_tube_from_points(&points, BRIGHTNESS_PRIMARY, tube_radius, tube_sides, true, 0);

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(colors));
    mesh.insert_indices(Indices::U32(indices));

    mesh
}

/// System to spawn body wireframe meshes for DebugBall bodies.
pub fn spawn_body_wireframe_meshes(
    mut commands: Commands,
    bodies: Query<(Entity, &Appearance), (With<BodyInfo>, Without<BodyWireframeLink>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
) {
    for (body_entity, appearance) in bodies.iter() {
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
                        current_tube_radius: WIRE_TUBE_RADIUS,
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
    bodies: Query<(Entity, &Appearance), (With<BodyInfo>, Without<OccluderLink>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<OccluderMaterial>>,
) {
    for (body_entity, appearance) in bodies.iter() {
        if let Appearance::DebugBall(_) = appearance {
            let mesh = Sphere::new(0.97f32).mesh().ico(5).unwrap();
            let mesh_handle = meshes.add(mesh);

            let material_handle = materials.add(OccluderMaterial::default());

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
    cameras: Query<&GlobalTransform, With<PlanetariumCamera>>,
    bodies: Query<(&Transform, Option<&BodyWireframeLink>), With<BodyInfo>>,
    wireframes: Query<&BodyWireframeMesh>,
    mut occluders: Query<(&OccluderMesh, &mut Transform, &ChildOf), Without<BodyInfo>>,
) {
    let Ok(camera_global) = cameras.single() else {
        return;
    };
    let _camera_pos = camera_global.translation();

    for (_occluder, mut occluder_transform, child_of) in occluders.iter_mut() {
        let Ok((_body_transform, wireframe_link)) = bodies.get(child_of.parent()) else {
            continue;
        };

        // Get the wireframe's current tube radius to know how much the lines have grown
        // Use the BodyWireframeLink for O(1) lookup instead of iterating all wireframes
        let tube_radius_ratio = wireframe_link
            .and_then(|link| wireframes.get(link.0).ok())
            .map(|wf| wf.current_tube_radius / WIRE_TUBE_RADIUS)
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
    bodies: Query<(Entity, &Appearance), (With<BodyInfo>, Without<TerminatorLinks>)>,
    stars: Query<Entity, With<BodyInfo>>,
    star_appearances: Query<&Appearance>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
) {
    // Find all star entities
    let star_entities: Vec<Entity> = stars
        .iter()
        .filter(|&e| {
            if let Ok(app) = star_appearances.get(e) {
                matches!(app, Appearance::Star(_))
            } else {
                false
            }
        })
        .collect();

    if star_entities.is_empty() {
        return;
    }

    for (body_entity, appearance) in bodies.iter() {
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
                            current_tube_radius: WIRE_TUBE_RADIUS,
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

/// System to update terminator meshes based on star positions.
/// Mesh regeneration only happens when tube radius changes significantly.
/// Rotation is updated every frame via Transform (cheap).
/// Also adjusts tube thickness based on distance from camera and fades
/// terminators out when the body is too small on screen.
pub fn update_terminator_meshes(
    cameras: Query<(&Camera, &GlobalTransform, &Projection), With<PlanetariumCamera>>,
    mut terminators: Query<(&mut TerminatorMesh, &Mesh3d, &ChildOf, &mut Visibility, &MeshMaterial3d<BodyWireframeMaterial>, &mut Transform)>,
    bodies: Query<(&Transform, &BodyState), Without<TerminatorMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
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

    for (mut terminator, mesh3d, child_of, mut visibility, material_handle, mut term_transform) in terminators.iter_mut() {
        let Ok((body_transform, body_state)) = bodies.get(child_of.parent()) else {
            continue;
        };

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

        // Only update material if value changed to avoid spurious asset change detection
        let new_emission = TERMINATOR_EMISSION_STRENGTH * fade;
        if let Some(mat) = materials.get(material_handle.id()) {
            if (mat.emission_strength - new_emission).abs() > 0.001 {
                if let Some(mat) = materials.get_mut(material_handle.id()) {
                    mat.emission_strength = new_emission;
                }
            }
        }

        // Get star state
        let Ok((_, star_state)) = bodies.get(terminator.star_entity) else {
            continue;
        };

        let tube_radius = calculate_tube_radius_for_distance(body_scale, distance);

        // Compute star direction in simulation space (Z-up)
        let star_dir_sim = (star_state.current_position - body_state.current_position).normalize();

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

        // Only regenerate mesh when tube radius changes significantly (>5%)
        let radius_ratio = tube_radius / terminator.current_tube_radius;
        let radius_changed = radius_ratio < 0.95 || radius_ratio > 1.05;
        
        if !radius_changed {
            continue;
        }

        // Generate mesh with canonical direction - actual orientation comes from Transform
        let new_mesh = generate_great_circle_tube(TERMINATOR_CANONICAL_DIR, tube_radius, TUBE_SIDES);

        if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
            *mesh_asset = new_mesh;
        }
        
        // Update cached tube radius
        terminator.current_tube_radius = tube_radius;
    }
}

/// System to clean up orphaned wireframe and terminator entities.
/// Uses RemovedComponents to only run when bodies are actually removed.
pub fn cleanup_orphaned_body_wireframes(
    mut commands: Commands,
    mut removed_bodies: RemovedComponents<BodyInfo>,
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
    bodies: Query<(&Transform, &BodyState)>,
    star_cache: Res<StarLightingFrameCache>,
    mut materials: ResMut<Assets<BodyWireframeMaterial>>,
) {
    for (wireframe, material_handle, child_of) in wireframes.iter() {
        let Ok((body_transform, body_state)) = bodies.get(child_of.parent()) else {
            continue;
        };

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
                (star.sim_position - body_state.current_position).normalize();
            let star_dir_bevy = Vec3::new(
                star_dir_sim.x as f32,
                star_dir_sim.z as f32,
                -star_dir_sim.y as f32,
            );
            let star_dir_local = body_transform.rotation.inverse() * star_dir_bevy;

            sun_dirs[num_suns as usize] = star_dir_local.extend(0.0);
            num_suns += 1;
        }

        if let Some(mat) = materials.get_mut(material_handle.id()) {
            mat.num_suns = num_suns;
            mat.sun_dir_0 = sun_dirs[0];
            mat.sun_dir_1 = sun_dirs[1];
            mat.sun_dir_2 = sun_dirs[2];
            mat.sun_dir_3 = sun_dirs[3];
        }
    }
}

/// System to update occluder materials with sun directions for Lambert shading.
pub fn update_occluder_lighting(
    occluders: Query<(&OccluderMesh, &MeshMaterial3d<OccluderMaterial>, &ChildOf)>,
    bodies: Query<(&Transform, &BodyState)>,
    star_cache: Res<StarLightingFrameCache>,
    mut materials: ResMut<Assets<OccluderMaterial>>,
) {
    for (occluder, material_handle, child_of) in occluders.iter() {
        let Ok((body_transform, body_state)) = bodies.get(child_of.parent()) else {
            continue;
        };

        let mut num_suns = 0u32;
        let mut sun_dirs = [Vec4::ZERO; MAX_SUNS];

        for star in &star_cache.stars {
            if star.entity == occluder.body_entity {
                continue;
            }
            if num_suns as usize >= MAX_SUNS {
                break;
            }

            let star_dir_sim =
                (star.sim_position - body_state.current_position).normalize();
            let star_dir_bevy = Vec3::new(
                star_dir_sim.x as f32,
                star_dir_sim.z as f32,
                -star_dir_sim.y as f32,
            );
            let star_dir_local = body_transform.rotation.inverse() * star_dir_bevy;

            sun_dirs[num_suns as usize] = star_dir_local.extend(0.0);
            num_suns += 1;
        }

        if let Some(mat) = materials.get_mut(material_handle.id()) {
            mat.num_suns = num_suns;
            mat.sun_dir_0 = sun_dirs[0];
            mat.sun_dir_1 = sun_dirs[1];
            mat.sun_dir_2 = sun_dirs[2];
            mat.sun_dir_3 = sun_dirs[3];
        }
    }
}

/// System to update wireframe mesh tube thickness based on distance from camera.
/// Regenerates meshes each frame to maintain minimum screen-space thickness.
pub fn update_wireframe_thickness(
    cameras: Query<&GlobalTransform, With<PlanetariumCamera>>,
    bodies: Query<(&Transform, &BodyInfo), With<BodyInfo>>,
    mut wireframes: Query<(&mut BodyWireframeMesh, &Mesh3d, &ChildOf)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let Ok(camera_global) = cameras.single() else {
        return;
    };
    let camera_pos = camera_global.translation();

    for (mut wireframe, mesh3d, child_of) in wireframes.iter_mut() {
        let Ok((body_transform, _body_info)) = bodies.get(child_of.parent()) else {
            continue;
        };

        let body_center = body_transform.translation;
        let distance = (body_center - camera_pos).length();
        let body_scale = body_transform.scale.x;

        let required_radius = calculate_tube_radius_for_distance(body_scale, distance);

        // Regenerate mesh if radius changed (with small tolerance to avoid unnecessary regeneration)
        let ratio = required_radius / wireframe.current_tube_radius;
        if ratio < 0.95 || ratio > 1.05 {
            // Regenerate mesh with new tube radius
            let new_mesh = generate_latlon_sphere(
                &wireframe.highlight_latitudes,
                required_radius,
                TUBE_SIDES,
            );

            if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
                *mesh_asset = new_mesh;
            }

            wireframe.current_tube_radius = required_radius;
        }
    }
}
