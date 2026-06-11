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

use crate::body::appearance::Appearance;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::{Motive, MotiveSelection};
use crate::body::universe::save::{UniversePhysics, ViewSettings};
use crate::camera::{Freecam, PlanetariumCamera};
use crate::gui::planetarium::{FocusedBodyState, HoverState, HoveredTrajectoryMarkerKind};
use crate::gui::planetarium::{format_sim_time_for_mode, MissionClockMode, MissionClockSettings};
use crate::gui::settings::Settings;
use crate::sim::{BodySelection, CalculateTrajectory, SimTime};
use crate::util::bevystuff::GlamVec;
use bevy_egui::{egui, EguiContexts};
use bevy::ecs::message::MessageWriter;

use super::trajectory_material::TrajectoryMaterial;

/// Marker component for trajectory mesh entities.
#[derive(Component)]
pub struct TrajectoryMesh {
    /// The body entity this trajectory belongs to
    pub body_entity: Entity,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum FocusedTrajectoryMarkerKind {
    Periapsis,
    Apoapsis,
    MouseHit,
}

#[derive(Component)]
pub struct FocusedTrajectoryMarker {
    pub kind: FocusedTrajectoryMarkerKind,
    pub previous_time: Option<crate::foundations::time::Instant>,
    pub next_time: Option<crate::foundations::time::Instant>,
    /// Refined true anomaly in radians (only used for MouseHit markers).
    pub true_anomaly: Option<f64>,
    /// Cached formatted string for previous time (regenerated when time or clock mode changes).
    cached_prev_str: Option<String>,
    /// Cached formatted string for next time (regenerated when time or clock mode changes).
    cached_next_str: Option<String>,
    /// Clock mode used when formatting cached strings.
    cached_clock_mode: Option<MissionClockMode>,
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
    /// Scratch buffer for working points during mesh generation.
    /// Reused each frame to avoid repeated allocations.
    working_points: Vec<(f64, DVec3)>,
}

pub fn build_working_trajectory_points(
    cache: &TrajectoryCache,
    current_local_position: Option<DVec3>,
    body_radius: f64,
    sim_time_seconds: f64,
    include_closing_duplicate: bool,
) -> Vec<(f64, DVec3)> {
    let cycle_frac = if let (Some(interval_start), Some(interval_size)) = (cache.interval_start, cache.interval_size) {
        let elapsed = sim_time_seconds - interval_start;
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

    let mut points: Vec<(f64, DVec3)> = cache.local_points.clone();
    if !cache.is_precessing {
        if let (Some(local_pos), Some(interval_size)) = (current_local_position, cache.interval_size) {
            let current_relative_time = cycle_frac * interval_size;
            if let Some(seg) = points.windows(2).position(|w| {
                current_relative_time >= w[0].0 && current_relative_time < w[1].0
            }) {
                let dist_before = (local_pos - points[seg].1).length();
                let dist_after = (local_pos - points[seg + 1].1).length();
                if dist_before >= body_radius && dist_after >= body_radius {
                    let pos_a = points[seg].1;
                    let pos_b = points[seg + 1].1;
                    let time_a = points[seg].0;
                    let time_b = points[seg + 1].0;
                    let neighbor_count = TRANSIENT_POINT_N / 2;
                    let mut transient_points: Vec<(f64, DVec3)> = Vec::new();
                    for i in (1..=neighbor_count).rev() {
                        let t = i as f64 / (neighbor_count + 1) as f64;
                        let pos = pos_a.lerp(local_pos, t);
                        let time = time_a + (current_relative_time - time_a) * t;
                        transient_points.push((time, pos));
                    }
                    transient_points.push((current_relative_time, local_pos));
                    transient_points.push((current_relative_time + 0.0001, local_pos));
                    for i in 1..=neighbor_count {
                        let t = i as f64 / (neighbor_count + 1) as f64;
                        let pos = local_pos.lerp(pos_b, t);
                        let time = current_relative_time + (time_b - current_relative_time) * t;
                        transient_points.push((time, pos));
                    }
                    let insert_pos = seg + 1;
                    for (i, pt) in transient_points.iter().enumerate() {
                        points.insert(insert_pos + i, *pt);
                    }
                }
            }
        }
    }

    if include_closing_duplicate && cache.closed && !points.is_empty() {
        let first = points[0];
        let close_time = points.last().map(|(t, _)| t + 0.001).unwrap_or(0.0);
        points.push((close_time, first.1));
    }

    points
}

/// Number of sides for the tube cross-section (6-8 is visually sufficient)
const TUBE_SIDES: u32 = 3;

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

/// Build an empty mesh that still declares the vertex layout required by
/// `trajectory.wgsl` (position, normal, color). This prevents pipeline
/// specialization failures when a placeholder mesh is used.
fn empty_trajectory_mesh() -> Mesh {
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


/// Calculate tube radius based on distance from camera.
/// Uses a power curve for general scaling, with a minimum angular size floor
/// to prevent sub-pixel aliasing at extreme distances.
pub fn calculate_tube_radius(distance_from_camera: f32) -> f32 {
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
    let mesh = empty_trajectory_mesh();
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
    focused_body_state: Res<FocusedBodyState>,
    hover_state: Res<HoverState>,
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    sim_time: Res<SimTime>,
    physics_graph: Res<crate::body::motive::calculate_body_positions::PhysicsGraph>,
    _physics: Res<UniversePhysics>,
) {
    let distance_scale = view_settings.distance_factor();
    let current_time = sim_time.time.to_j2000_seconds();
    let camera_pos = fcam.bevy_pos;
    // Brightness (front/back range + exposure) is applied live in the shader via
    // material uniforms; the mesh only bakes the geometric along-length factor `t`
    // and per-vertex distance dimming.

    for (traj_mesh, mut cache, mut visibility, mesh3d, mut transform) in trajectory_meshes.iter_mut() {
        // Compensate for camera movement since last mesh rebuild so the
        // trajectory tracks bodies even when the mesh isn't regenerated.
        if let Some(last_cam) = cache.last_camera_pos {
            let delta = last_cam - camera_pos;
            transform.translation = delta.as_vec3();
        }

        // Find the body this trajectory belongs to using direct entity lookup (O(1))
        let Ok((state, info, _motive, appearance)) = bodies.get(traj_mesh.body_entity) else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        
        // Check visibility conditions
        let should_show = cache.valid
            && !cache.local_points.is_empty()
            && (
                view_settings.show_trajectories
                    || view_settings.body_in_any_trajectory_tag(&info.id)
                    || focused_body_state.is_focused(&info.id)
                    || hover_state.is_body_hovered(&info.id)
            );
        
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
        let _focused_matches = focused_body_state
            .current_body_id
            .as_ref()
            .map(|id| id == &info.id)
            .unwrap_or(false);
        
        // Skip mesh regeneration if nothing relevant changed
        if !cache.mesh_dirty && !camera_moved && !time_changed {
            continue;
        }
        
        // Calculate cycle fraction for brightness and transient insertion
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
        let primary_offset: Option<DVec3> = cache
            .primary_id
            .as_ref()
            .and_then(|pid| {
                physics_graph
                    .id_to_entity
                    .get(pid)
                    .and_then(|entity| bodies.get(*entity).ok())
                    .and_then(|(primary_state, _, _, _)| {
                        if primary_state.trajectory.is_none() { return None; }
                        Some(primary_state.current_position)
                    })
            });
        
        // Build the working point list with transient point insertion
        // Reuse scratch buffer to avoid per-frame allocations
        cache.working_points.clear();
        let mut transient_idx: Option<usize> = None;
        let mut transient_prime_idx: Option<usize> = None;
        
        // Calculate expected capacity: base points + transient points + closing point
        let transient_count = TRANSIENT_POINT_N + 2; // neighbors + T + T'
        let closing_extra = if cache.closed { 1 } else { 0 };
        let expected_capacity = cache.local_points.len() + transient_count + closing_extra;
        cache.working_points.reserve(expected_capacity);
        
        // Find transient insertion segment if applicable
        let transient_info: Option<(usize, DVec3, f64, DVec3, DVec3, f64, f64)> = if !cache.is_precessing {
            if let (Some(local_pos), Some(interval_size)) = (state.current_local_position, cache.interval_size) {
                let current_relative_time = cycle_frac * interval_size;
                let body_radius = appearance.map(|a| a.radius()).unwrap_or(0.0);
                
                cache.local_points.windows(2).enumerate().find_map(|(seg, w)| {
                    if current_relative_time >= w[0].0 && current_relative_time < w[1].0 {
                        let dist_before = (local_pos - w[0].1).length();
                        let dist_after = (local_pos - w[1].1).length();
                        if dist_before >= body_radius && dist_after >= body_radius {
                            Some((seg, local_pos, current_relative_time, w[0].1, w[1].1, w[0].0, w[1].0))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        } else {
            None
        };
        
        // Build points in a single pass, inserting transient points at the right position
        // Use index-based iteration to avoid borrow conflicts
        let local_points_len = cache.local_points.len();
        for idx in 0..local_points_len {
            // Insert transient points after the segment start point
            if let Some((seg, local_pos, current_relative_time, pos_a, pos_b, time_a, time_b)) = transient_info {
                if idx == seg + 1 {
                    let neighbor_count = TRANSIENT_POINT_N / 2;
                    
                    // Wake neighbors: interpolate between A and T
                    for i in (1..=neighbor_count).rev() {
                        let t = i as f64 / (neighbor_count + 1) as f64;
                        let pos = pos_a.lerp(local_pos, t);
                        let time = time_a + (current_relative_time - time_a) * t;
                        cache.working_points.push((time, pos));
                    }
                    
                    // T (brightest) - body's actual position
                    transient_idx = Some(cache.working_points.len());
                    cache.working_points.push((current_relative_time, local_pos));
                    
                    // T' (dimmest, same position as T)
                    transient_prime_idx = Some(cache.working_points.len());
                    cache.working_points.push((current_relative_time + 0.0001, local_pos));
                    
                    // Future neighbors: interpolate between T and B
                    for i in 1..=neighbor_count {
                        let t = i as f64 / (neighbor_count + 1) as f64;
                        let pos = local_pos.lerp(pos_b, t);
                        let time = current_relative_time + (time_b - current_relative_time) * t;
                        cache.working_points.push((time, pos));
                    }
                }
            }
            let pt = cache.local_points[idx];
            cache.working_points.push(pt);
        }
        
        // For closed orbits, append first point to close the loop (A')
        if cache.closed && !cache.working_points.is_empty() {
            let first = cache.working_points[0];
            let close_time = cache.working_points.last().map(|(t, _)| t + 0.001).unwrap_or(0.0);
            cache.working_points.push((close_time, first.1));
        }
        
        // Use reference to avoid another copy
        let points = &cache.working_points;
        
        if points.len() < 2 {
            *visibility = Visibility::Hidden;
            continue;
        }
        
        // Transform points to Bevy space and compute brightness + radius
        let point_count = points.len();
        
        // Points now include: (position, t, amplitude, radius)
        let transformed_points: Vec<(Vec3, f32, f32, f32)> = points.iter().enumerate().map(|(idx, (_, pos))| {
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

            // Geometric along-length factor t (0..1); the front/back brightness lerp
            // is applied in the shader so it stays live without rebuilding the mesh.
            let t = compute_brightness_t(
                idx,
                transient_idx,
                transient_prime_idx,
                cache.closed,
                point_count,
            );

            // Distance-based dimming: far trajectories are dimmer (baked per-vertex
            // amplitude; only changes when the camera moves, which rebuilds the mesh).
            let distance_dim = if distance_from_camera <= DISTANCE_DIM_REF {
                1.0
            } else {
                (DISTANCE_DIM_REF / distance_from_camera).powf(DISTANCE_DIM_POWER).max(DISTANCE_DIM_MIN)
            };

            // Near-fade is handled per-fragment in the shader for pixel-accurate fading
            (bevy_pos, t, distance_dim, radius)
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

/// Push the live trajectory brightness settings (front/back percentages and camera
/// exposure) into the tube materials' shader uniforms. This keeps brightness tweaks
/// instant without rebuilding any meshes. Only entities with a [`TrajectoryMesh`]
/// are touched, so markers (which use the identity range) are left alone. The
/// material asset is only mutated when a value actually changed, to avoid
/// re-uploading the uniform buffer every frame.
pub fn update_trajectory_material_brightness(
    settings: Res<Settings>,
    color_grading: Single<&ColorGrading>,
    tubes: Query<&MeshMaterial3d<TrajectoryMaterial>, With<TrajectoryMesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    let front = (settings.display.trajectory_brightness_front / 100.0).max(0.0);
    let back = (settings.display.trajectory_brightness_back / 100.0).max(0.0);
    let exposure = color_grading.global.exposure;
    let glow_gain = settings.display.glow.brightness_multiplier();

    for handle in tubes.iter() {
        // Read first; only take a mutable handle (which flags the asset for
        // re-upload) if something is actually out of date.
        let needs_update = materials
            .get(&handle.0)
            .map(|m| {
                m.front != front
                    || m.back != back
                    || m.exposure != exposure
                    || m.glow_gain != glow_gain
            })
            .unwrap_or(false);
        if needs_update {
            if let Some(material) = materials.get_mut(&handle.0) {
                material.front = front;
                material.back = back;
                material.exposure = exposure;
                material.glow_gain = glow_gain;
            }
        }
    }
}

pub fn update_focused_trajectory_markers(
    mut commands: Commands,
    focused_body_state: Res<FocusedBodyState>,
    sim_time: Res<SimTime>,
    view_settings: Res<ViewSettings>,
    physics: Res<UniversePhysics>,
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    physics_graph: Res<crate::body::motive::calculate_body_positions::PhysicsGraph>,
    bodies: Query<(Entity, &BodyInfo, &BodyState, &Motive)>,
    mut markers: Query<(Entity, &mut FocusedTrajectoryMarker, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    let Some(focused_id) = focused_body_state.current_body_id.as_deref() else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let Some(focused_entity) = physics_graph.id_to_entity.get(focused_id).copied() else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let Ok((_, _info, focused_state, motive)) = bodies.get(focused_entity) else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let (_, selection) = motive.motive_at(sim_time.time);
    let MotiveSelection::Keplerian(kepler) = selection else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let Some(primary_entity) = physics_graph.id_to_entity.get(&kepler.primary_id).copied() else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let Ok((_, _, primary_state, _)) = bodies.get(primary_entity) else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let primary_mass = bodies
        .get(primary_entity)
        .map(|(_, info, _, _)| info.mass)
        .unwrap_or(0.0);
    let mu = kepler
        .gravitational_parameter
        .unwrap_or(physics.gravitational_constant * primary_mass);
    let period_seconds = kepler.period(mu).to_seconds();
    let periapsis_base = kepler.time_at_periapsis_passage(mu);

    let Some(trajectory) = focused_state.trajectory.as_ref() else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };
    let periapsis_local = trajectory
        .iter()
        .min_by(|a, b| {
            a.1.length_squared()
                .partial_cmp(&b.1.length_squared())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, d)| *d);
    let apoapsis_local = if kepler.is_open() {
        None
    } else {
        trajectory
            .iter()
            .max_by(|a, b| {
                a.1.length_squared()
                    .partial_cmp(&b.1.length_squared())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, d)| *d)
    };
    let Some(periapsis_local) = periapsis_local else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };
    let periapsis_world = primary_state.current_position + periapsis_local;
    let apoapsis_world = apoapsis_local.map(|apo| primary_state.current_position + apo);
    let peri_times = repeating_event_prev_next(periapsis_base, period_seconds, sim_time.time);
    let apo_times = repeating_event_prev_next(
        crate::foundations::time::Instant::from_seconds_since_j2000(
            periapsis_base.to_j2000_seconds() + period_seconds * 0.5,
        ),
        period_seconds,
        sim_time.time,
    );

    let distance_scale = view_settings.distance_factor();
    let peri_bevy = periapsis_world.as_bevy_scaled_cheated(distance_scale, fcam.bevy_pos);
    let peri_tube_radius = calculate_tube_radius(peri_bevy.length());
    let peri_marker_radius = 2.0 * peri_tube_radius;

    let apo_bevy = apoapsis_world.map(|pos| pos.as_bevy_scaled_cheated(distance_scale, fcam.bevy_pos));
    let apo_marker_radius = apo_bevy
        .as_ref()
        .map(|pos| 2.0 * calculate_tube_radius(pos.length()));

    ensure_trajectory_marker(
        &mut commands,
        &mut markers,
        &mut meshes,
        &mut materials,
        FocusedTrajectoryMarkerKind::Periapsis,
        peri_bevy,
        peri_marker_radius,
        peri_times.0,
        peri_times.1,
        None, // Pe/Ap don't use true_anomaly field
    );

    match (apo_bevy, apo_marker_radius) {
        (Some(position), Some(radius)) => {
            ensure_trajectory_marker(
                &mut commands,
                &mut markers,
                &mut meshes,
                &mut materials,
                FocusedTrajectoryMarkerKind::Apoapsis,
                position,
                radius,
                apo_times.0,
                apo_times.1,
                None, // Pe/Ap don't use true_anomaly field
            );
        }
        _ => {
            despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::Apoapsis);
        }
    }
}

/// System to update the MouseHit trajectory marker based on hover state.
/// Spawns/updates marker when trajectory is hit, despawns when not.
pub fn update_mouse_hit_marker(
    mut commands: Commands,
    hover_state: Res<HoverState>,
    focused_body_state: Res<FocusedBodyState>,
    sim_time: Res<SimTime>,
    physics: Res<UniversePhysics>,
    _fcam: Single<&Freecam, With<PlanetariumCamera>>,
    physics_graph: Res<crate::body::motive::calculate_body_positions::PhysicsGraph>,
    bodies: Query<(&BodyInfo, &Motive, Option<&TrajectoryMeshLink>)>,
    trajectory_caches: Query<&TrajectoryCache>,
    mut markers: Query<(Entity, &mut FocusedTrajectoryMarker, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    let Some(hit_data) = hover_state.hovered_trajectory_hit.as_ref() else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

    let Some(focused_id) = focused_body_state.current_body_id.as_deref() else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

    let Some(focused_entity) = physics_graph.id_to_entity.get(focused_id).copied() else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

    let Ok((_info, motive, traj_link)) = bodies.get(focused_entity) else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

    let (_, selection) = motive.motive_at(sim_time.time);
    let MotiveSelection::Keplerian(kepler) = selection else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

    // Get trajectory cache via O(1) link lookup instead of O(n) iterator scan
    let Some(traj_link) = traj_link else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };
    let Ok(cache) = trajectory_caches.get(traj_link.0) else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };
    if !cache.valid || cache.local_points.is_empty() {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    }

    // Get primary info for mu calculation
    let primary_mass = physics_graph.id_to_entity.get(&kepler.primary_id)
        .and_then(|e| bodies.get(*e).ok())
        .map(|(info, _, _)| info.mass)
        .unwrap_or(0.0);

    let mu = kepler.gravitational_parameter
        .unwrap_or(physics.gravitational_constant * primary_mass);
    let period_seconds = kepler.period(mu).to_seconds();
    let periapsis_base = kepler.time_at_periapsis_passage(mu);

    // Compute true anomaly using Newton refinement (20 iterations in fourier_expansion)
    let true_anomaly = refine_true_anomaly_newton(
        &kepler,
        mu,
        hit_data.start_time,
        hit_data.end_time,
        hit_data.t,
        periapsis_base,
    );

    // Calculate next and previous times for this true anomaly position
    let event_times = compute_anomaly_event_times(
        &kepler,
        mu,
        true_anomaly,
        period_seconds,
        periapsis_base,
        sim_time.time,
    );

    // Marker position follows picker-provided hit position.
    let marker_bevy_pos = hit_data.hit_position_bevy;
    let tube_radius = calculate_tube_radius(marker_bevy_pos.length());
    let marker_radius = 2.0 * tube_radius;

    ensure_trajectory_marker(
        &mut commands,
        &mut markers,
        &mut meshes,
        &mut materials,
        FocusedTrajectoryMarkerKind::MouseHit,
        marker_bevy_pos,
        marker_radius,
        event_times.0,
        event_times.1,
        Some(true_anomaly),
    );
}

/// Refine true anomaly using Newton iteration within the segment bounds.
/// Takes segment endpoint times and interpolation parameter, returns refined true anomaly.
fn refine_true_anomaly_newton(
    kepler: &crate::body::motive::kepler_motive::KeplerMotive,
    mu: f64,
    start_time: f64,
    end_time: f64,
    t: f64,
    periapsis_base: crate::foundations::time::Instant,
) -> f64 {
    // Initial guess: linear interpolation of time
    let interpolated_relative_time = start_time + t * (end_time - start_time);
    let absolute_time = crate::foundations::time::Instant::from_seconds_since_j2000(
        periapsis_base.to_j2000_seconds() + interpolated_relative_time
    );
    
    // Use kepler's eccentricity via public method
    let ecc = kepler.eccentricity();
    let mean_anomaly = kepler.mean_anomaly(absolute_time, mu);

    let iterations = crate::body::motive::kepler_motive::expansion_iterations(ecc);
    
    // Apply N-iteration Fourier expansion for true anomaly refinement
    crate::foundations::kepler::true_anomaly::fourier_expansion(mean_anomaly, ecc, iterations)
}

/// Compute the previous and next times when the body will be at a given true anomaly.
fn compute_anomaly_event_times(
    kepler: &crate::body::motive::kepler_motive::KeplerMotive,
    _mu: f64,
    true_anomaly: f64,
    period_seconds: f64,
    periapsis_base: crate::foundations::time::Instant,
    now: crate::foundations::time::Instant,
) -> (Option<crate::foundations::time::Instant>, Option<crate::foundations::time::Instant>) {
    use std::f64::consts::TAU;
    
    // Convert true anomaly to mean anomaly, then to time offset from periapsis
    let ecc = kepler.eccentricity();
    let eccentric_anomaly = crate::foundations::kepler::eccentric_anomaly::from_true_anomaly(ecc, true_anomaly);
    let mean_anomaly = crate::foundations::kepler::mean_anomaly::kepler(eccentric_anomaly, ecc);
    
    // Normalize mean anomaly to [0, 2π)
    let mean_anomaly = mean_anomaly.rem_euclid(TAU);
    
    // Time offset from periapsis for this mean anomaly
    let time_offset = (mean_anomaly / TAU) * period_seconds;
    
    // Base time for this anomaly event
    let event_base = crate::foundations::time::Instant::from_seconds_since_j2000(
        periapsis_base.to_j2000_seconds() + time_offset
    );
    
    // Find previous and next occurrences
    repeating_event_prev_next(event_base, period_seconds, now)
}

fn ensure_trajectory_marker(
    commands: &mut Commands,
    markers: &mut Query<(Entity, &mut FocusedTrajectoryMarker, &mut Transform)>,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<TrajectoryMaterial>>,
    kind: FocusedTrajectoryMarkerKind,
    position: Vec3,
    radius: f32,
    previous_time: Option<crate::foundations::time::Instant>,
    next_time: Option<crate::foundations::time::Instant>,
    true_anomaly: Option<f64>,
) {
    let mut existing = None;
    for (entity, marker, _) in markers.iter_mut() {
        if marker.kind == kind {
            existing = Some(entity);
            break;
        }
    }

    if let Some(entity) = existing {
        if let Ok((_, mut marker, mut transform)) = markers.get_mut(entity) {
            // Invalidate cached strings if times changed
            if marker.previous_time != previous_time || marker.next_time != next_time {
                marker.cached_prev_str = None;
                marker.cached_next_str = None;
                marker.cached_clock_mode = None;
            }
            marker.previous_time = previous_time;
            marker.next_time = next_time;
            marker.true_anomaly = true_anomaly;
            transform.translation = position;
            transform.scale = Vec3::splat(radius);
        }
        return;
    }

    let mut mesh = Sphere::new(1.0f32).mesh().ico(2).unwrap();
    if let Some(VertexAttributeValues::Float32x3(positions)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            VertexAttributeValues::Float32x4(vec![[1.0, 1.0, 1.0, 1.0]; positions.len()]),
        );
    }
    let mesh_handle = meshes.add(mesh);
    let material_handle = materials.add(TrajectoryMaterial::default());

    commands.spawn((
        Mesh3d(mesh_handle),
        MeshMaterial3d(material_handle),
        Transform {
            translation: position,
            scale: Vec3::splat(radius),
            ..Default::default()
        },
        Visibility::Visible,
        NoFrustumCulling,
        FocusedTrajectoryMarker {
            kind,
            previous_time,
            next_time,
            true_anomaly,
            cached_prev_str: None,
            cached_next_str: None,
            cached_clock_mode: None,
        },
    ));
}

fn despawn_trajectory_marker(
    commands: &mut Commands,
    markers: &mut Query<(Entity, &mut FocusedTrajectoryMarker, &mut Transform)>,
    kind: FocusedTrajectoryMarkerKind,
) {
    for (entity, marker, _) in markers.iter_mut() {
        if marker.kind == kind {
            commands.entity(entity).despawn();
        }
    }
}

fn despawn_all_focused_trajectory_markers(
    commands: &mut Commands,
    markers: &mut Query<(Entity, &mut FocusedTrajectoryMarker, &mut Transform)>,
) {
    for (entity, _, _) in markers.iter_mut() {
        commands.entity(entity).despawn();
    }
}

fn repeating_event_prev_next(
    base: crate::foundations::time::Instant,
    period_seconds: f64,
    now: crate::foundations::time::Instant,
) -> (Option<crate::foundations::time::Instant>, Option<crate::foundations::time::Instant>) {
    if !period_seconds.is_finite() || period_seconds <= f64::EPSILON {
        return (None, None);
    }

    let base_seconds = base.to_j2000_seconds();
    let now_seconds = now.to_j2000_seconds();
    let n = ((now_seconds - base_seconds) / period_seconds).floor();
    let prev = crate::foundations::time::Instant::from_seconds_since_j2000(
        base_seconds + n * period_seconds,
    );
    let next = crate::foundations::time::Instant::from_seconds_since_j2000(
        prev.to_j2000_seconds() + period_seconds,
    );
    (Some(prev), Some(next))
}


pub fn draw_trajectory_marker_labels(
    mut marker_query: Query<(&mut FocusedTrajectoryMarker, &Transform)>,
    cameras: Query<(&Camera, &PlanetariumCamera, &Projection, &Transform)>,
    mut contexts: EguiContexts,
    clock_settings: Res<MissionClockSettings>,
    hover_state: Res<HoverState>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Background,
        egui::Id::new("trajectory_marker_labels"),
    ));
    
    let current_mode = clock_settings.mode;

    for (camera, _, _, camera_transform) in &cameras {
        let fresh_camera_gt = GlobalTransform::from(*camera_transform);
        for (mut marker, transform) in marker_query.iter_mut() {
            let world_pos = transform.translation + Vec3::Y * (transform.scale.y * 1.6);
            let Ok(screen_pos) = camera.world_to_viewport(&fresh_camera_gt, world_pos) else {
                continue;
            };

            // Update cached strings if clock mode changed or they haven't been computed yet
            let needs_cache_update = marker.cached_clock_mode != Some(current_mode);
            if needs_cache_update {
                marker.cached_next_str = marker.next_time
                    .map(|t| format_sim_time_for_mode(current_mode, t));
                marker.cached_prev_str = marker.previous_time
                    .map(|t| format_sim_time_for_mode(current_mode, t));
                marker.cached_clock_mode = Some(current_mode);
            }

            let label_text = match marker.kind {
                FocusedTrajectoryMarkerKind::MouseHit => {
                    // MouseHit always shows full label with true anomaly + times
                    let anomaly_degrees = marker.true_anomaly
                        .map(|a| a.to_degrees().rem_euclid(360.0))
                        .unwrap_or(0.0);
                    let next_line = marker.cached_next_str.as_deref().unwrap_or("N/A");
                    let prev_line = marker.cached_prev_str.as_deref().unwrap_or("N/A");
                    format!(
                        "ν = {:.1}°\nNext: {}\nPrev: {}",
                        anomaly_degrees, next_line, prev_line
                    )
                }
                FocusedTrajectoryMarkerKind::Periapsis | FocusedTrajectoryMarkerKind::Apoapsis => {
                    let marker_name = match marker.kind {
                        FocusedTrajectoryMarkerKind::Periapsis => "Periapsis",
                        FocusedTrajectoryMarkerKind::Apoapsis => "Apoapsis",
                        _ => unreachable!(),
                    };
                    let marker_summary = match marker.kind {
                        FocusedTrajectoryMarkerKind::Periapsis => "Pe",
                        FocusedTrajectoryMarkerKind::Apoapsis => "Ap",
                        _ => unreachable!(),
                    };
                    let marker_hovered = match marker.kind {
                        FocusedTrajectoryMarkerKind::Periapsis => {
                            hover_state.hovered_marker_kind == Some(HoveredTrajectoryMarkerKind::Periapsis)
                        }
                        FocusedTrajectoryMarkerKind::Apoapsis => {
                            hover_state.hovered_marker_kind == Some(HoveredTrajectoryMarkerKind::Apoapsis)
                        }
                        _ => false,
                    };

                    if marker_hovered {
                        let next_line = marker.cached_next_str.as_deref().unwrap_or("N/A");
                        let prev_line = marker.cached_prev_str.as_deref().unwrap_or("N/A");
                        format!(
                            "{}\nNext: {}\nPrev: {}",
                            marker_name, next_line, prev_line
                        )
                    } else {
                        marker_summary.to_string()
                    }
                }
            };
            
            painter.text(
                egui::pos2(screen_pos.x, screen_pos.y),
                egui::Align2::CENTER_BOTTOM,
                label_text,
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(90, 237, 175),
            );
        }
    }
}

/// Compute the along-length lerp factor `t` (0..1) for a point based on forward
/// distance from T' around the orbit. `t = 0` maps to the trajectory's front
/// brightness, `t = 1` to its back brightness (the lerp itself happens in the shader).
/// T' (right after body) is t=0, going forward through the orbit t increases,
/// reaching t=1 at T (same position as T', one full orbit later).
fn compute_brightness_t(
    idx: usize,
    transient_idx: Option<usize>,
    transient_prime_idx: Option<usize>,
    closed: bool,
    point_count: usize,
) -> f32 {
    if closed {
        if let Some(tp_idx) = transient_prime_idx {
            // Forward distance from T' (wrapping around the orbit)
            // T' = 0, going forward increases, T = point_count - 1 (just before wrapping back to T')
            let forward_dist = ((idx as isize - tp_idx as isize + point_count as isize) % point_count as isize) as f32;
            let max_dist = (point_count - 1) as f32;
            // T' (t=0) is front, T (t≈1) is back
            forward_dist / max_dist
        } else if let Some(t_idx) = transient_idx {
            // No T' but have T - use forward distance from T
            let forward_dist = ((idx as isize - t_idx as isize + point_count as isize) % point_count as isize) as f32;
            let max_dist = (point_count - 1) as f32;
            1.0 - forward_dist / max_dist
        } else {
            // No transient point - fallback to the midpoint of the range
            0.5
        }
    } else {
        // Open orbits: simple wake bright, future dim
        if let Some(t_idx) = transient_idx {
            if idx <= t_idx {
                1.0
            } else {
                0.0
            }
        } else {
            0.0
        }
    }
}

/// Generate a tube mesh from a list of points with associated brightness and radius values.
/// Each point becomes a ring of vertices; adjacent rings are connected with triangles.
/// Points are (position, t, amplitude, radius), where `t` is the along-length lerp
/// factor (baked into vertex alpha) and `amplitude` is a per-vertex brightness scale
/// such as distance dimming (baked into vertex rgb). The shader turns these into the
/// final brightness using the material's front/back/exposure uniforms.
pub fn generate_tube_mesh(points: &[(Vec3, f32, f32, f32)], sides: u32) -> Mesh {
    if points.len() < 2 {
        return empty_trajectory_mesh();
    }
    
    let ring_count = points.len();
    let verts_per_ring = sides as usize;
    let total_verts = ring_count * verts_per_ring;
    
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(total_verts);
    let mut indices: Vec<u32> = Vec::with_capacity((ring_count - 1) * verts_per_ring * 6);
    
    for (ring_idx, (center, t, amplitude, radius)) in points.iter().enumerate() {
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
            
            // RGB carries the per-vertex amplitude (distance dimming); alpha carries the
            // along-length lerp factor t. The shader combines these with the material's
            // front/back/exposure uniforms to produce the final brightness.
            colors.push([*amplitude, *amplitude, *amplitude, *t]);
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
