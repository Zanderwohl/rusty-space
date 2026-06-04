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

use super::body_material::BodyWireframeMaterial;

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

/// Terminator color (gentle warm yellow)
const TERMINATOR_COLOR: LinearRgba = LinearRgba::new(0.95, 0.85, 0.4, 1.0);

/// Marker component for body wireframe mesh entities.
#[derive(Component)]
pub struct BodyWireframeMesh {
    pub body_entity: Entity,
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
        return Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
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
            };
            let material_handle = materials.add(material);

            let wireframe_entity = commands
                .spawn((
                    Mesh3d(mesh_handle),
                    MeshMaterial3d(material_handle),
                    Transform::default(),
                    Visibility::Inherited,
                    NoFrustumCulling,
                    BodyWireframeMesh { body_entity },
                    ChildOf(body_entity),
                ))
                .id();

            commands.entity(body_entity).insert(BodyWireframeLink(wireframe_entity));
        }
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

                // Create initial empty mesh (will be updated each frame)
                let mesh = generate_great_circle_tube(Vec3::X, WIRE_TUBE_RADIUS, TUBE_SIDES);
                let mesh_handle = meshes.add(mesh);

                let material = BodyWireframeMaterial {
                    base_color: TERMINATOR_COLOR,
                    emission_strength: TERMINATOR_EMISSION_STRENGTH,
                    alpha_mode: AlphaMode::Opaque,
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

/// System to update terminator meshes each frame based on star positions.
pub fn update_terminator_meshes(
    terminators: Query<(&TerminatorMesh, &Mesh3d, &ChildOf)>,
    bodies: Query<(&Transform, &BodyState)>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    for (terminator, mesh3d, child_of) in terminators.iter() {
        // Get body transform and state
        let Ok((body_transform, body_state)) = bodies.get(child_of.parent()) else {
            continue;
        };

        // Get star state
        let Ok((_, star_state)) = bodies.get(terminator.star_entity) else {
            continue;
        };

        // Compute star direction in simulation space (Z-up)
        let star_dir_sim = (star_state.current_position - body_state.current_position).normalize();

        // Convert from simulation space (Z-up) to Bevy space (Y-up): (x, y, z) -> (x, z, -y)
        let star_dir_bevy = Vec3::new(
            star_dir_sim.x as f32,
            star_dir_sim.z as f32,
            -star_dir_sim.y as f32,
        );

        // Transform to body-local space by applying inverse rotation
        let star_dir_local = body_transform.rotation.inverse() * star_dir_bevy;

        // Generate new terminator mesh
        let new_mesh = generate_great_circle_tube(star_dir_local, WIRE_TUBE_RADIUS, TUBE_SIDES);

        // Update the mesh asset
        if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
            *mesh_asset = new_mesh;
        }
    }
}

/// System to clean up orphaned wireframe and terminator entities.
pub fn cleanup_orphaned_body_wireframes(
    mut commands: Commands,
    wireframes: Query<(Entity, &BodyWireframeMesh)>,
    terminators: Query<(Entity, &TerminatorMesh)>,
    bodies: Query<Entity, With<BodyInfo>>,
) {
    // Clean up wireframes
    for (entity, wireframe) in wireframes.iter() {
        if bodies.get(wireframe.body_entity).is_err() {
            commands.entity(entity).despawn();
        }
    }

    // Clean up terminators
    for (entity, terminator) in terminators.iter() {
        if bodies.get(terminator.body_entity).is_err() {
            commands.entity(entity).despawn();
        }
    }
}
