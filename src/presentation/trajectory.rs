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
use crate::body::universe::save::{UniversePhysics, ViewSettings};
use crate::camera::{Freecam, PlanetariumCamera};
use crate::gui::settings::{DisplayGlow, Settings};
use crate::sim::{BodySelection, CalculateTrajectory, SimTime};
use crate::util::bevystuff::GlamVec;
use bevy::ecs::message::MessageWriter;

use super::trajectory_material::TrajectoryMaterial;

/// Marker component for trajectory mesh entities.
#[derive(Component)]
pub struct TrajectoryMesh {
    /// The body entity this trajectory belongs to
    pub body_entity: Entity,
}

/// How often to rebuild trajectories for precessing orbits (in simulation seconds)
const PRECESSION_REBUILD_INTERVAL: f64 = 86400.0; // One Julian day

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
    /// Whether this orbit has precession (apsidal or nodal).
    /// Precessing orbits need periodic trajectory recalculation.
    pub is_precessing: bool,
    /// Simulation time (J2000 seconds) when trajectory was last rebuilt.
    /// Used to trigger periodic rebuilds for precessing orbits.
    pub last_rebuild_time: f64,
    /// Whether the mesh needs to be regenerated (set true when cache, camera, or time changes).
    pub mesh_dirty: bool,
    /// Last camera position (cheated bevy space) used for mesh generation.
    pub last_camera_pos: Option<DVec3>,
    /// Last simulation time (J2000 seconds) used for mesh generation.
    pub last_mesh_time: f64,
}

/// Number of sides for the tube cross-section (6-8 is visually sufficient)
const TUBE_SIDES: u32 = 4;

/// Minimum tube radius (when very close to camera) - keeps it as a thin line
const MIN_TUBE_RADIUS: f32 = 0.000005;

/// Maximum tube radius (when very far from camera) - prevents massive tubes
const MAX_TUBE_RADIUS: f32 = 1000.0;

/// Reference distance for radius scaling (radius = base at this distance)
const REFERENCE_DISTANCE: f32 = 10.0;

/// Base radius at reference distance
const BASE_TUBE_RADIUS: f32 = 0.015;

/// Power for distance-to-radius scaling. Higher = more constant screen-space size.
/// 1.0 would be perfectly constant angular size; 0.5 is sqrt.
const RADIUS_SCALE_POWER: f32 = 0.75;

/// Minimum angular size (radius / distance) to prevent sub-pixel aliasing at extreme range.
/// ~0.001 rad ≈ 1-2 pixels on typical displays.
const MIN_ANGULAR_SIZE: f32 = 0.0005;

/// Distance (bevy meters) at/below which trajectories are at full brightness.
const DISTANCE_DIM_REF: f32 = 5.0;

/// Power for distance-based dimming. Lower = more gradual fade over distance.
const DISTANCE_DIM_POWER: f32 = 0.4;

/// Minimum dimming factor - distant trajectories never go fully invisible.
const DISTANCE_DIM_MIN: f32 = 0.01;

/// Number of transient points to insert (T and T' count as 2, plus neighbors on each side)
const TRANSIENT_POINT_N: usize = 5;

/// Spacing between transient point neighbors in radians (5 degrees)
const TRANSIENT_SPACING: f64 = 0.05 * std::f64::consts::PI / 180.0;


/// Calculate tube radius based on distance from camera.
/// Uses a power curve for general scaling, with a minimum angular size floor
/// to prevent sub-pixel aliasing at extreme distances.
fn calculate_tube_radius(distance_from_camera: f32) -> f32 {
    if distance_from_camera <= 0.0 {
        return MIN_TUBE_RADIUS;
    }
    
    // Power curve: closer to 1.0 = more constant screen-space appearance
    let scale_factor = (distance_from_camera / REFERENCE_DISTANCE).powf(RADIUS_SCALE_POWER);
    let radius = BASE_TUBE_RADIUS * scale_factor;
    
    // Floor: ensure minimum angular size to avoid sub-pixel flicker at extreme distance
    let angular_floor = MIN_ANGULAR_SIZE * distance_from_camera;
    
    radius.max(angular_floor).clamp(MIN_TUBE_RADIUS, MAX_TUBE_RADIUS)
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
/// Only rebuilds when the trajectory data actually changes, not on every BodyState mutation.
pub fn rebuild_trajectory_caches(
    bodies: Query<(&BodyState, &Motive)>,
    mut trajectory_meshes: Query<(&TrajectoryMesh, &mut TrajectoryCache)>,
    sim_time: Res<SimTime>,
) {
    for (traj_mesh, mut cache) in trajectory_meshes.iter_mut() {
        // Find the body this trajectory belongs to using direct entity lookup (O(1))
        let Ok((state, motive)) = bodies.get(traj_mesh.body_entity) else {
            continue;
        };
        
        // Check if body has a trajectory
        let Some(trajectory) = &state.trajectory else {
            if cache.valid {
                cache.valid = false;
                cache.local_points.clear();
            }
            continue;
        };
        
        // Skip rebuild if cache is already valid and trajectory hasn't changed.
        // We detect actual trajectory changes by comparing:
        // 1. Point count (changes if resolution changes)
        // 2. First point's position (changes when trajectory is recalculated, e.g. for precession)
        let traj_len = trajectory.len();
        let first_point_matches = if let (Some(cached_first), Some((_, traj_first))) = 
            (cache.local_points.first(), trajectory.iter().next()) 
        {
            // Compare with small epsilon for floating point
            (cached_first.1 - *traj_first).length_squared() < 1e-10
        } else {
            false
        };
        
        if cache.valid 
            && cache.local_points.len() == traj_len 
            && first_point_matches
        {
            // Cache is up to date, skip rebuild
            continue;
        }
        
        // Get primary_id and check for precession
        let (primary_id, is_precessing) = match motive.motive_at(crate::foundations::time::Instant::J2000) {
            (_, MotiveSelection::Keplerian(k)) => {
                let precessing = matches!(
                    k.rotation,
                    crate::body::motive::kepler_motive::KeplerRotation::PrecessingEulerAngles(_)
                );
                (Some(k.primary_id.clone()), precessing)
            },
            _ => (None, false),
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
        
        cache.primary_id = primary_id.clone();
        cache.is_precessing = is_precessing;
        cache.last_rebuild_time = sim_time.time.to_j2000_seconds();
        cache.valid = true;
        cache.mesh_dirty = true; // Trigger mesh rebuild
        
        // Debug: log when precessing orbit cache is built
        if is_precessing {
            if let Some(pid) = &primary_id {
                info!("Built trajectory cache for precessing orbit (primary: {}), rebuild_time: {:.0}", 
                      pid, cache.last_rebuild_time);
            }
        }
    }
}

/// System to trigger trajectory recalculation for precessing orbits periodically.
/// Precessing orbits drift from their cached trajectory over time.
pub fn refresh_precessing_trajectories(
    mut trajectory_meshes: Query<(&TrajectoryMesh, &mut TrajectoryCache)>,
    bodies: Query<&BodyInfo>,
    sim_time: Res<SimTime>,
    mut calc_writer: MessageWriter<CalculateTrajectory>,
) {
    let current_time = sim_time.time.to_j2000_seconds();
    
    for (traj_mesh, mut cache) in trajectory_meshes.iter_mut() {
        // Only check precessing orbits with valid caches
        if !cache.is_precessing || !cache.valid {
            continue;
        }
        
        // Check if enough simulation time has passed since last rebuild
        let time_since_rebuild = (current_time - cache.last_rebuild_time).abs();
        if time_since_rebuild >= PRECESSION_REBUILD_INTERVAL {
            // Get the body's ID to request trajectory recalculation
            if let Ok(info) = bodies.get(traj_mesh.body_entity) {
                info!("Refreshing precessing trajectory for {} (time since rebuild: {:.0}s)", 
                      info.id, time_since_rebuild);
                calc_writer.write(CalculateTrajectory {
                    selection: BodySelection::IDs(vec![info.id.clone()]),
                });
                
                // Invalidate cache so rebuild_trajectory_caches will update it
                cache.valid = false;
            }
        }
    }
}

/// Threshold for camera movement before mesh rebuild (in bevy units)
const CAMERA_MOVE_THRESHOLD: f64 = 0.005;

/// Main system to build trajectory meshes each frame.
/// Reads cached points, inserts transient point, applies transforms, computes brightness, generates tube geometry.
/// Skips mesh regeneration when camera and sim time haven't changed significantly.
pub fn build_trajectory_meshes(
    bodies: Query<(&BodyState, &BodyInfo, &Motive, Option<&Appearance>)>,
    mut trajectory_meshes: Query<(&TrajectoryMesh, &mut TrajectoryCache, &mut Visibility, &Mesh3d, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    view_settings: Res<ViewSettings>,
    settings: Res<Settings>,
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    sim_time: Res<SimTime>,
    color_grading: Single<&ColorGrading>,
    physics_graph: Res<crate::body::motive::calculate_body_positions::PhysicsGraph>,
    physics: Res<UniversePhysics>,
) {
    let distance_scale = view_settings.distance_factor();
    let exposure = color_grading.global.exposure;
    let current_time = sim_time.time.to_j2000_seconds();
    let camera_pos = fcam.bevy_pos;
    
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
    
    for (traj_mesh, mut cache, mut visibility, mesh3d, mut transform) in trajectory_meshes.iter_mut() {
        // Compensate for camera movement since last mesh rebuild so the
        // trajectory tracks bodies even when the mesh isn't regenerated.
        if let Some(last_cam) = cache.last_camera_pos {
            let delta = last_cam - camera_pos;
            transform.translation = delta.as_vec3();
        }

        // Find the body this trajectory belongs to using direct entity lookup (O(1))
        let Ok((state, info, motive, appearance)) = bodies.get(traj_mesh.body_entity) else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        
        // Check visibility conditions
        let should_show = cache.valid 
            && !cache.local_points.is_empty()
            && (view_settings.show_trajectories || view_settings.body_in_any_trajectory_tag(&info.id));
        
        if !should_show {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        }
        
        // Track if visibility just changed to visible (needs rebuild)
        let was_hidden = *visibility == Visibility::Hidden;
        if was_hidden {
            *visibility = Visibility::Visible;
            cache.mesh_dirty = true;
        }
        
        // Check if we need to rebuild the mesh
        let camera_moved = cache.last_camera_pos
            .map(|last| (last - camera_pos).length() > CAMERA_MOVE_THRESHOLD)
            .unwrap_or(true);
        let time_changed = (cache.last_mesh_time - current_time).abs() > 0.001;
        
        // Skip mesh regeneration if nothing relevant changed
        if !cache.mesh_dirty && !camera_moved && !time_changed {
            continue;
        }
        
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
        
        // Get primary offset for Keplerian orbits using O(1) lookup via PhysicsGraph
        let primary_offset: Option<DVec3> = cache.primary_id.as_ref().and_then(|pid| {
            physics_graph.id_to_entity.get(pid)
                .and_then(|entity| bodies.get(*entity).ok())
                .and_then(|(primary_state, _, _, _)| {
                    if primary_state.trajectory.is_none() { return None; }
                    Some(primary_state.current_position)
                })
        });
        
        // Build the working point list with transient point insertion
        let mut points: Vec<(f64, DVec3)> = cache.local_points.clone();
        let mut transient_idx: Option<usize> = None;
        let mut transient_prime_idx: Option<usize> = None;
        
        // Insert transient points: T (body position), T' (same position, dimmest), and neighbors
        // for increased local resolution. Skip for precessing orbits.
        if !cache.is_precessing {
            if let (Some(local_pos), Some(interval_size)) = (state.current_local_position, cache.interval_size) {
                let current_relative_time = cycle_frac * interval_size;
                let body_radius = appearance.map(|a| a.radius()).unwrap_or(0.0);
                
                if let Some(seg) = points.windows(2).position(|w| {
                    current_relative_time >= w[0].0 && current_relative_time < w[1].0
                }) {
                    let dist_before = (local_pos - points[seg].1).length();
                    let dist_after = (local_pos - points[seg + 1].1).length();
                    
                    if dist_before >= body_radius && dist_after >= body_radius {
                        // Get segment boundary positions
                        let pos_a = points[seg].1;     // Position before T
                        let pos_b = points[seg + 1].1; // Position after T
                        let time_a = points[seg].0;
                        let time_b = points[seg + 1].0;
                        
                        // Number of neighbors on each side of T
                        let neighbor_count = TRANSIENT_POINT_N / 2;
                        
                        // Build transient points by interpolating within the segment
                        // Order: wake neighbors (A toward T), T, T', future neighbors (T toward B)
                        let mut transient_points: Vec<(f64, DVec3)> = Vec::new();
                        
                        // Wake neighbors: interpolate between A and T
                        // Evenly space them, with the last one closest to T
                        for i in (1..=neighbor_count).rev() {
                            // t=0 at A, t=1 at T; we want positions at t = i/(neighbor_count+1)
                            let t = i as f64 / (neighbor_count + 1) as f64;
                            let pos = pos_a.lerp(local_pos, t);
                            let time = time_a + (current_relative_time - time_a) * t;
                            transient_points.push((time, pos));
                        }
                        
                        // T (brightest) - body's actual position
                        let t_local_idx = transient_points.len();
                        transient_points.push((current_relative_time, local_pos));
                        
                        // T' (dimmest, same position as T)
                        let tp_local_idx = transient_points.len();
                        transient_points.push((current_relative_time + 0.0001, local_pos));
                        
                        // Future neighbors: interpolate between T and B
                        // Evenly space them, with the first one closest to T
                        for i in 1..=neighbor_count {
                            // t=0 at T, t=1 at B; we want positions at t = i/(neighbor_count+1)
                            let t = i as f64 / (neighbor_count + 1) as f64;
                            let pos = local_pos.lerp(pos_b, t);
                            let time = current_relative_time + (time_b - current_relative_time) * t;
                            transient_points.push((time, pos));
                        }
                        
                        // Insert all transient points at the correct position
                        let insert_pos = seg + 1;
                        for (i, pt) in transient_points.iter().enumerate() {
                            points.insert(insert_pos + i, *pt);
                        }
                        
                        transient_idx = Some(insert_pos + t_local_idx);
                        transient_prime_idx = Some(insert_pos + tp_local_idx);
                    }
                }
            }
        }
        
        // For closed orbits, append first point to close the loop (A')
        // Note: A and A' don't visually connect since the brightness fades to min at A'
        if cache.closed && !points.is_empty() {
            let first = points[0];
            let close_time = points.last().map(|(t, _)| t + 0.001).unwrap_or(0.0);
            points.push((close_time, first.1));
        }
        
        if points.len() < 2 {
            *visibility = Visibility::Hidden;
            continue;
        }
        
        // Transform points to Bevy space and compute brightness + radius
        let point_count = points.len();
        
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
            
            // Compute orbital brightness based on forward distance from T' around the orbit
            let orbital_brightness = compute_brightness(
                idx,
                transient_idx,
                transient_prime_idx,
                cache.closed,
                min_brightness,
                max_brightness,
                point_count,
            );
            
            // Distance-based dimming: far trajectories are dimmer
            let distance_dim = if distance_from_camera <= DISTANCE_DIM_REF {
                1.0
            } else {
                (DISTANCE_DIM_REF / distance_from_camera).powf(DISTANCE_DIM_POWER).max(DISTANCE_DIM_MIN)
            };
            
            // Near-fade is handled per-fragment in the shader for pixel-accurate fading
            let brightness = orbital_brightness * distance_dim;
            
            (bevy_pos, brightness, radius)
        }).collect();
        
        // Generate tube mesh with per-point radii
        let mesh = generate_tube_mesh(&transformed_points, TUBE_SIDES);
        
        // Update the mesh asset
        if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
            *mesh_asset = mesh;
        }
        
        // Mesh is now built for the current camera position; clear the drift offset.
        transform.translation = Vec3::ZERO;

        // Update cache tracking for dirty detection
        cache.mesh_dirty = false;
        cache.last_camera_pos = Some(camera_pos);
        cache.last_mesh_time = current_time;
    }
}

/// Compute brightness for a point based on forward distance from T' around the orbit.
/// T' (right after body) is dimmest, going forward through the orbit brightness increases,
/// reaching max at T (same position as T', one full orbit later).
/// This creates a continuous gradient: T (max) → future → wake → T' (min).
fn compute_brightness(
    idx: usize,
    transient_idx: Option<usize>,
    transient_prime_idx: Option<usize>,
    closed: bool,
    min_brightness: f32,
    max_brightness: f32,
    point_count: usize,
) -> f32 {
    if closed {
        if let Some(tp_idx) = transient_prime_idx {
            // Forward distance from T' (wrapping around the orbit)
            // T' = 0, going forward increases, T = point_count - 1 (just before wrapping back to T')
            let forward_dist = ((idx as isize - tp_idx as isize + point_count as isize) % point_count as isize) as f32;
            let max_dist = (point_count - 1) as f32;
            let progress = forward_dist / max_dist;
            // T' (progress=0) is min, T (progress≈1) is max
            min_brightness + (max_brightness - min_brightness) * progress
        } else if let Some(t_idx) = transient_idx {
            // No T' but have T - use forward distance from T
            let forward_dist = ((idx as isize - t_idx as isize + point_count as isize) % point_count as isize) as f32;
            let max_dist = (point_count - 1) as f32;
            let progress = 1.0 - forward_dist / max_dist;
            min_brightness + (max_brightness - min_brightness) * progress
        } else {
            // No transient point - fallback to mid brightness
            (min_brightness + max_brightness) / 2.0
        }
    } else {
        // Open orbits: simple wake bright, future dim
        if let Some(t_idx) = transient_idx {
            if idx <= t_idx {
                max_brightness
            } else {
                min_brightness
            }
        } else {
            min_brightness
        }
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
            colors.push([1.0, 1.0, 1.0, *brightness]);
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
/// Uses RemovedComponents to only run when bodies are actually removed.
pub fn cleanup_orphaned_trajectory_meshes(
    mut commands: Commands,
    mut removed_bodies: RemovedComponents<BodyInfo>,
    trajectory_meshes: Query<(Entity, &TrajectoryMesh)>,
) {
    // Early exit if no bodies were removed
    if removed_bodies.is_empty() {
        return;
    }
    
    // Collect removed body entities
    let removed: std::collections::HashSet<Entity> = removed_bodies.read().collect();
    
    // Despawn trajectory meshes for removed bodies
    for (traj_entity, traj_mesh) in trajectory_meshes.iter() {
        if removed.contains(&traj_mesh.body_entity) {
            commands.entity(traj_entity).despawn();
        }
    }
}
