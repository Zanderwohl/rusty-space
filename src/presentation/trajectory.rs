//! Trajectory rendering system using custom material and tube meshes.
//!
//! Replaces the old gizmo-based rendering with depth-correct tube geometry
//! that participates in Bloom for emissive segments.

use std::f32::consts::PI;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::DVec3;
use bevy::render::view::ColorGrading;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use num_traits::Pow;

use crate::body::appearance::Appearance;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::{Motive, MotiveSelection};
use crate::body::universe::save::ViewSettings;
use crate::camera::{Freecam, PlanetariumCamera};
use crate::gui::settings::{DisplayGlow, Settings};
use crate::sim::SimTime;
use crate::util::bevystuff::GlamVec;

use super::trajectory_material::TrajectoryMaterial;

/// Marker component for trajectory mesh entities.
#[derive(Component)]
pub struct TrajectoryMesh {
    /// The body entity this trajectory belongs to
    pub body_entity: Entity,
}

/// Cached trajectory data to avoid recomputing from TimeMap every frame.
/// Only rebuilt when orbital parameters change.
#[derive(Component, Default)]
pub struct TrajectoryCache {
    /// Base points as local displacements (DVec3 in simulation space).
    /// Does NOT include the transient point or closing duplicate.
    pub local_points: Vec<(f64, DVec3)>,
    /// Whether this is a closed orbit (needs closing duplicate appended).
    pub closed: bool,
    /// Period interval size for brightness calculation (only for closed orbits)
    pub interval_size: Option<f64>,
    /// Period interval start for cycle fraction calculation
    pub interval_start: Option<f64>,
    /// The body's primary ID for looking up primary position offset
    pub primary_id: Option<String>,
    /// Whether the cache is valid (set to false when trajectory needs rebuild)
    pub valid: bool,
}

/// Number of sides for the tube cross-section (6-8 is visually sufficient)
const TUBE_SIDES: u32 = 8;

/// Minimum tube radius (when very close to camera) - keeps it as a thin line
const MIN_TUBE_RADIUS: f32 = 0.001;

/// Maximum tube radius (when very far from camera) - prevents massive tubes
const MAX_TUBE_RADIUS: f32 = 0.5;

/// Reference distance for radius scaling (radius = base at this distance)
const REFERENCE_DISTANCE: f32 = 10.0;

/// Base radius at reference distance
const BASE_TUBE_RADIUS: f32 = 0.02;

/// Power for distance-to-radius scaling (0.5 = square root, good for large ranges)
const RADIUS_SCALE_POWER: f32 = 0.5;

/// Calculate tube radius based on distance from camera.
/// Uses a power curve with min/max clamping for smooth scaling across vast distance ranges.
fn calculate_tube_radius(distance_from_camera: f32) -> f32 {
    if distance_from_camera <= 0.0 {
        return MIN_TUBE_RADIUS;
    }
    
    // Scale radius based on distance using a power curve
    // At reference_distance, radius = base_radius
    // Closer → smaller, farther → larger, but with diminishing returns
    let scale_factor = (distance_from_camera / REFERENCE_DISTANCE).powf(RADIUS_SCALE_POWER);
    let radius = BASE_TUBE_RADIUS * scale_factor;
    
    // Clamp to min/max
    radius.clamp(MIN_TUBE_RADIUS, MAX_TUBE_RADIUS)
}

/// Spawns a trajectory mesh entity as a child of the given body entity.
/// Called when bodies are spawned.
pub fn spawn_trajectory_mesh(
    commands: &mut Commands,
    body_entity: Entity,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<TrajectoryMaterial>>,
) -> Entity {
    let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
    let mesh_handle = meshes.add(mesh);
    
    let material = TrajectoryMaterial::default();
    let material_handle = materials.add(material);
    
    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform::default(),
        Visibility::Hidden,
        NoFrustumCulling,
        TrajectoryMesh { body_entity },
        TrajectoryCache::default(),
    )).id()
}

/// System to rebuild trajectory caches when orbital parameters change.
/// Listens for changes to BodyState.trajectory and repopulates the cache.
pub fn rebuild_trajectory_caches(
    bodies: Query<(Entity, &BodyState, &Motive), Changed<BodyState>>,
    mut trajectory_meshes: Query<(&TrajectoryMesh, &mut TrajectoryCache)>,
) {
    for (body_entity, state, motive) in bodies.iter() {
        // Find the trajectory mesh for this body
        for (traj_mesh, mut cache) in trajectory_meshes.iter_mut() {
            if traj_mesh.body_entity != body_entity {
                continue;
            }
            
            // Only rebuild if body has a trajectory
            let Some(trajectory) = &state.trajectory else {
                cache.valid = false;
                cache.local_points.clear();
                continue;
            };
            
            // Get primary_id for Keplerian motives
            let primary_id = match motive.motive_at(crate::foundations::time::Instant::J2000) {
                (_, MotiveSelection::Keplerian(k)) => Some(k.primary_id.clone()),
                _ => None,
            };
            
            // Collect points from TimeMap (local displacements)
            cache.local_points = trajectory.iter().map(|(t, d)| (t, *d)).collect();
            
            // Set periodicity info
            if let Some(periodicity) = trajectory.periodicity() {
                cache.closed = true;
                cache.interval_size = Some(periodicity.interval_size);
                cache.interval_start = Some(periodicity.interval_start);
            } else {
                cache.closed = false;
                cache.interval_size = None;
                cache.interval_start = None;
            }
            
            cache.primary_id = primary_id;
            cache.valid = true;
        }
    }
}

/// Main system to build trajectory meshes each frame.
/// Reads cached points, inserts transient point, applies transforms, computes brightness, generates tube geometry.
pub fn build_trajectory_meshes(
    bodies: Query<(Entity, &BodyState, &BodyInfo, &Motive, Option<&Appearance>)>,
    mut trajectory_meshes: Query<(&TrajectoryMesh, &TrajectoryCache, &mut Visibility, &Mesh3d)>,
    mut meshes: ResMut<Assets<Mesh>>,
    view_settings: Res<ViewSettings>,
    settings: Res<Settings>,
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    sim_time: Res<SimTime>,
    color_grading: Single<&ColorGrading>,
) {
    let distance_scale = view_settings.distance_factor();
    let exposure = color_grading.global.exposure;
    
    // Brightness range based on glow settings
    let (min_brightness, max_brightness) = match settings.display.glow {
        DisplayGlow::None => (0.1, 1.0),
        DisplayGlow::Subtle => (0.25, 1.2),
        DisplayGlow::VFD => (1.0, 4.0),
        DisplayGlow::Defcon => (0.2, 10.0),
    };
    let exposure_adjust = 2f32.pow(-exposure);
    let min_brightness = min_brightness * exposure_adjust;
    let max_brightness = max_brightness * exposure_adjust;
    
    for (traj_mesh, cache, mut visibility, mesh3d) in trajectory_meshes.iter_mut() {
        // Find the body this trajectory belongs to
        let Some((_, state, info, _motive, appearance)) = bodies.iter().find(|(e, _, _, _, _)| *e == traj_mesh.body_entity) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        
        // Check visibility conditions
        let should_show = cache.valid 
            && !cache.local_points.is_empty()
            && (view_settings.show_trajectories || view_settings.body_in_any_trajectory_tag(&info.id));
        
        if !should_show {
            *visibility = Visibility::Hidden;
            continue;
        }
        
        *visibility = Visibility::Visible;
        
        // Calculate cycle fraction for brightness
        let cycle_frac = if let (Some(interval_start), Some(interval_size)) = (cache.interval_start, cache.interval_size) {
            let elapsed = sim_time.time.to_j2000_seconds() - interval_start;
            let position_in_cycle = elapsed % interval_size;
            let normalized = if position_in_cycle < 0.0 {
                position_in_cycle + interval_size
            } else {
                position_in_cycle
            };
            normalized / interval_size
        } else {
            0.0
        };
        
        // Get primary offset for Keplerian orbits
        let primary_offset: Option<DVec3> = cache.primary_id.as_ref().and_then(|pid| {
            bodies.iter()
                .find(|(_, _, info, _, _)| &info.id == pid)
                .and_then(|(_, primary_state, _, _, _)| {
                    if primary_state.trajectory.is_none() { return None; }
                    Some(primary_state.current_position)
                })
        });
        
        // Build the working point list with transient point insertion
        let mut points: Vec<(f64, DVec3)> = cache.local_points.clone();
        let mut transient_idx: Option<usize> = None;
        
        // Insert transient point (body's current local position)
        if let (Some(local_pos), Some(interval_size)) = (state.current_local_position, cache.interval_size) {
            let current_relative_time = cycle_frac * interval_size;
            let body_radius = appearance.map(|a| a.radius()).unwrap_or(0.0);
            
            if let Some(seg) = points.windows(2).position(|w| {
                current_relative_time >= w[0].0 && current_relative_time < w[1].0
            }) {
                let dist_before = (local_pos - points[seg].1).length();
                let dist_after = (local_pos - points[seg + 1].1).length();
                
                if dist_before >= body_radius && dist_after >= body_radius {
                    points.insert(seg + 1, (current_relative_time, local_pos));
                    transient_idx = Some(seg + 1);
                }
            }
        }
        
        // For closed orbits, append first point to close the loop
        if cache.closed && !points.is_empty() {
            let first = points[0];
            // Use a time slightly past the last point to maintain ordering
            let close_time = points.last().map(|(t, _)| t + 0.001).unwrap_or(0.0);
            points.push((close_time, first.1));
        }
        
        if points.len() < 2 {
            *visibility = Visibility::Hidden;
            continue;
        }
        
        // Transform points to Bevy space and compute brightness + radius
        let point_count = points.len();
        let transient_frac = transient_idx.map(|i| i as f32 / point_count as f32).unwrap_or(cycle_frac as f32);
        
        // Points now include: (position, brightness, radius)
        let transformed_points: Vec<(Vec3, f32, f32)> = points.iter().enumerate().map(|(idx, (_, pos))| {
            // Apply primary offset
            let world_pos = match primary_offset {
                Some(offset) => *pos + offset,
                None => *pos,
            };
            
            // Transform to Bevy space (camera is at origin in this space)
            let bevy_pos = world_pos.as_bevy_scaled_cheated(distance_scale, fcam.bevy_pos);
            
            // Distance from camera (which is at origin in cheated space)
            let distance_from_camera = bevy_pos.length();
            
            // Calculate radius based on distance
            let radius = calculate_tube_radius(distance_from_camera);
            
            // Compute brightness based on distance from transient point
            let point_frac = idx as f32 / point_count as f32;
            let brightness = compute_brightness(
                point_frac, 
                transient_frac, 
                cache.closed,
                min_brightness,
                max_brightness,
            );
            
            (bevy_pos, brightness, radius)
        }).collect();
        
        // Generate tube mesh with per-point radii
        let mesh = generate_tube_mesh(&transformed_points, TUBE_SIDES);
        
        // Update the mesh asset
        if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
            *mesh_asset = mesh;
        }
    }
}

/// Compute brightness for a point based on its arc distance from the transient point.
/// The wake (where the body just was) is brightest, fading toward the path ahead.
fn compute_brightness(
    point_frac: f32,
    transient_frac: f32,
    closed: bool,
    min_brightness: f32,
    max_brightness: f32,
) -> f32 {
    // Determine if this point is "behind" (wake) or "ahead" of the body
    let is_wake = if closed {
        // For closed orbits, "wake" means the body has passed this point recently
        // forward_dist is how far ahead this point is (0 = at body, approaching 1 = just behind)
        let forward_dist = (point_frac - transient_frac + 1.0) % 1.0;
        forward_dist > 0.5  // More than halfway around = in the wake
    } else {
        // For open orbits, wake is simply earlier indices
        point_frac < transient_frac
    };
    
    // Calculate distance along trajectory from the transient point
    let raw_dist = if closed {
        let forward_dist = (point_frac - transient_frac + 1.0) % 1.0;
        if is_wake {
            // Distance back into the wake (0 = just passed, 0.5 = opposite side)
            1.0 - forward_dist
        } else {
            // Distance ahead (0 = at body, 0.5 = opposite side)
            forward_dist
        }
    } else {
        (point_frac - transient_frac).abs()
    };
    
    if is_wake {
        // Wake: brightest at the body, fading gradually into the past
        // Points just behind the body are bright, points far behind are dimmer
        let wake_falloff = raw_dist * 2.0;  // Fade over ~half the orbit
        let brightness = 1.0 - wake_falloff.min(1.0);
        min_brightness + (max_brightness - min_brightness) * brightness
    } else {
        // Ahead: dim throughout - the body hasn't been here yet
        // Slight gradient: dimmest far ahead, slightly brighter near the body
        let ahead_brightness = 0.3 * (1.0 - raw_dist.min(1.0));
        min_brightness + (max_brightness - min_brightness) * ahead_brightness
    }
}

/// Generate a tube mesh from a list of points with associated brightness and radius values.
/// Each point becomes a ring of vertices; adjacent rings are connected with triangles.
/// Points are (position, brightness, radius).
pub fn generate_tube_mesh(points: &[(Vec3, f32, f32)], sides: u32) -> Mesh {
    if points.len() < 2 {
        return Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
    }
    
    let ring_count = points.len();
    let verts_per_ring = sides as usize;
    let total_verts = ring_count * verts_per_ring;
    
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(total_verts);
    let mut indices: Vec<u32> = Vec::with_capacity((ring_count - 1) * verts_per_ring * 6);
    
    for (ring_idx, (center, brightness, radius)) in points.iter().enumerate() {
        // Compute tangent direction (forward along the tube)
        let tangent = if ring_idx == 0 {
            (points[1].0 - *center).normalize_or_zero()
        } else if ring_idx == ring_count - 1 {
            (*center - points[ring_idx - 1].0).normalize_or_zero()
        } else {
            (points[ring_idx + 1].0 - points[ring_idx - 1].0).normalize_or_zero()
        };
        
        // Find perpendicular vectors to form the ring plane
        let (perp1, perp2) = perpendicular_vectors(tangent);
        
        // Generate ring vertices with per-point radius
        for i in 0..sides {
            let angle = (i as f32 / sides as f32) * 2.0 * PI;
            let (sin_a, cos_a) = angle.sin_cos();
            
            // Position on the ring using this point's radius
            let offset = perp1 * cos_a * *radius + perp2 * sin_a * *radius;
            let pos = *center + offset;
            positions.push([pos.x, pos.y, pos.z]);
            
            // Normal points outward from tube center
            let normal = offset.normalize_or_zero();
            normals.push([normal.x, normal.y, normal.z]);
            
            // Color: RGB is green (base color handled by material), alpha encodes brightness
            colors.push([0.0, 1.0, 0.0, *brightness]);
        }
        
        // Generate triangles connecting this ring to the next
        if ring_idx < ring_count - 1 {
            let base = (ring_idx * verts_per_ring) as u32;
            let next_base = ((ring_idx + 1) * verts_per_ring) as u32;
            
            for i in 0..sides {
                let i_next = (i + 1) % sides;
                
                // Two triangles per quad
                // Triangle 1
                indices.push(base + i);
                indices.push(next_base + i);
                indices.push(base + i_next);
                
                // Triangle 2
                indices.push(base + i_next);
                indices.push(next_base + i);
                indices.push(next_base + i_next);
            }
        }
    }
    
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(colors));
    mesh.insert_indices(Indices::U32(indices));
    
    mesh
}

/// Find two perpendicular vectors to form a plane orthogonal to the given direction.
fn perpendicular_vectors(dir: Vec3) -> (Vec3, Vec3) {
    // Choose a vector that's not parallel to dir
    let not_parallel = if dir.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };
    
    let perp1 = dir.cross(not_parallel).normalize_or_zero();
    let perp2 = dir.cross(perp1).normalize_or_zero();
    
    (perp1, perp2)
}

/// System to spawn trajectory mesh entities for bodies that don't have one.
/// Runs each frame to catch newly spawned bodies.
pub fn spawn_trajectory_meshes_for_bodies(
    mut commands: Commands,
    bodies: Query<Entity, (With<BodyInfo>, Without<TrajectoryMeshLink>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    for body_entity in bodies.iter() {
        let traj_entity = spawn_trajectory_mesh(&mut commands, body_entity, &mut meshes, &mut materials);
        // Link the body to its trajectory mesh
        commands.entity(body_entity).insert(TrajectoryMeshLink(traj_entity));
    }
}

/// Component linking a body to its trajectory mesh entity.
#[derive(Component)]
pub struct TrajectoryMeshLink(pub Entity);

/// System to clean up trajectory meshes when their parent bodies are despawned.
pub fn cleanup_orphaned_trajectory_meshes(
    mut commands: Commands,
    trajectory_meshes: Query<(Entity, &TrajectoryMesh)>,
    bodies: Query<Entity, With<BodyInfo>>,
) {
    for (traj_entity, traj_mesh) in trajectory_meshes.iter() {
        // If the body no longer exists, despawn the trajectory mesh
        if bodies.get(traj_mesh.body_entity).is_err() {
            commands.entity(traj_entity).despawn();
        }
    }
}
