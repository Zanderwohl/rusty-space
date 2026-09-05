//! Trajectory rendering system using custom material and tube meshes.
//!
//! Replaces the old gizmo-based rendering with depth-correct tube geometry
//! that participates in Bloom for emissive segments.

use std::f32::consts::PI;
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::math::{DQuat, DVec3};
use bevy::render::view::ColorGrading;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};

use crate::body::motive::MotiveSelection;
use crate::sim::world::{BodyRef, SimSystem, Trajectories};
use crate::body::universe::save::ViewSettings;
use crate::camera::{Freecam, PlanetariumCamera};
use crate::gui::planetarium::{FocusedBodyState, HoverState, HoveredTrajectoryMarkerKind};
use crate::gui::planetarium::{format_sim_time_for_mode, MissionClockMode, MissionClockSettings};
use crate::gui::settings::Settings;
use crate::sim::SimTime;
use crate::presentation::render_space::{ToRender, ToRenderRotation};
use bevy_egui::{egui, EguiContexts};

use super::trajectory_material::{TrajectoryMaterial, TRAJECTORY_BASE_TUBE_RADIUS};

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
    /// Rotation from perifocal to reference frame captured when `local_points`
    /// were last rebuilt.
    pub base_perifocal_to_reference: Option<DQuat>,
    /// Simulation time (J2000 seconds) when trajectory was last rebuilt.
    /// Used to trigger periodic rebuilds for precessing orbits.
    pub last_rebuild_time: f64,
    /// Whether the mesh needs to be regenerated. The geometry is in mesh-local perifocal
    /// space and does not depend on the clock or the camera, so this is set only when the
    /// sampled points themselves change.
    pub mesh_dirty: bool,
    /// World scale the geometry was baked at. The only other thing that can invalidate it.
    pub last_distance_scale: Option<f64>,
}

pub fn build_working_trajectory_points(
    cache: &TrajectoryCache,
    _current_local_position: Option<DVec3>,
    _body_radius: f64,
    _sim_time_seconds: f64,
    include_closing_duplicate: bool,
) -> Vec<(f64, DVec3)> {
    let mut points: Vec<(f64, DVec3)> = cache.local_points.clone();

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

/// Power for distance-to-radius scaling. Higher = more constant screen-space size.
/// 1.0 would be perfectly constant angular size; 0.5 is sqrt.
const RADIUS_SCALE_POWER: f32 = 0.75;

/// Minimum angular size (radius / distance) to prevent sub-pixel aliasing at extreme range.
/// ~0.001 rad ≈ 1-2 pixels on typical displays.
const MIN_ANGULAR_SIZE: f32 = 0.0005;

// Distance-based dimming now lives entirely in `trajectory.wgsl` (DISTANCE_DIM_*), so
// that a camera move no longer invalidates any geometry.

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
    let radius = TRAJECTORY_BASE_TUBE_RADIUS * scale_factor;
    
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
    
    // Trajectory tubes dim with range; markers do not.
    let material_handle = materials.add(TrajectoryMaterial {
        distance_dim: 1.0,
        ..Default::default()
    });
    
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
    bodies: Query<&BodyRef>,
    system: Res<SimSystem>,
    trajectories: Res<Trajectories>,
    mut trajectory_meshes: Query<(&TrajectoryMesh, &mut TrajectoryCache)>,
    sim_time: Res<SimTime>,
) {
    for (traj_mesh, mut cache) in trajectory_meshes.iter_mut() {
        // Find the body this trajectory belongs to using direct entity lookup (O(1))
        let Ok(body_ref) = bodies.get(traj_mesh.body_entity) else {
            continue;
        };
        let Some(body_index) = system.0.index_of(body_ref.0) else { continue };
        let motive = system.0.motive(body_index);

        let Some(trajectory) = trajectories.0.get(&body_ref.0).map(|p| &p.points) else {
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
        
        // Get primary_id, precession flag, and current perifocal->reference rotation.
        let (primary_id, is_precessing, base_perifocal_to_reference) = match motive.motive_at(sim_time.time) {
            (_, MotiveSelection::Keplerian(k)) => {
                let precessing = matches!(
                    k.rotation,
                    crate::body::motive::kepler_motive::KeplerRotation::PrecessingEulerAngles(_)
                );
                let rot = DQuat::from_mat3(&k.perifocal_to_reference_matrix(sim_time.time));
                (Some(k.primary_id.clone()), precessing, Some(rot))
            },
            _ => (None, false, None),
        };
        
        // Collect points from TimeMap (local displacements)
        cache.local_points = trajectory.iter().map(|(t, d)| (t.to_seconds(), *d)).collect();
        
        // Set periodicity info
        if let Some(periodicity) = trajectory.periodicity() {
            cache.closed = true;
            cache.interval_size = Some(periodicity.interval_size.to_seconds());
            cache.interval_start = Some(periodicity.interval_start.to_j2000_seconds());
        } else {
            cache.closed = false;
            cache.interval_size = None;
            cache.interval_start = None;
        }
        
        cache.primary_id = primary_id.clone();
        cache.is_precessing = is_precessing;
        cache.base_perifocal_to_reference = base_perifocal_to_reference;
        cache.last_rebuild_time = sim_time.time.to_j2000_seconds();
        cache.valid = true;
        cache.mesh_dirty = true; // Trigger mesh rebuild
        
    }
}

/// Precession is now handled by per-frame transform rotation in
/// `build_trajectory_meshes`, so no periodic trajectory recalculation is needed.
pub fn refresh_precessing_trajectories() {}

/// Main system to build trajectory meshes each frame.
/// Reads cached points, applies transforms, computes brightness, generates tube geometry.
/// Skips mesh regeneration when the cache is unchanged.
pub fn build_trajectory_meshes(
    bodies: Query<&BodyRef>,
    system: Res<SimSystem>,
    mut trajectory_meshes: Query<(
        &TrajectoryMesh,
        &mut TrajectoryCache,
        &mut Visibility,
        &Mesh3d,
        &MeshMaterial3d<TrajectoryMaterial>,
        &mut Transform,
    )>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
    view_settings: Res<ViewSettings>,
    focused_body_state: Res<FocusedBodyState>,
    hover_state: Res<HoverState>,
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    sim_time: Res<SimTime>,
) {
    let distance_scale = view_settings.distance_factor();
    let camera_pos = fcam.bevy_pos;
    // The mesh bakes only what is genuinely static about an orbit: its shape in
    // mesh-local perifocal space and each vertex's phase along it. Everything that moves
    // — the primary's position and the precession (transform), the body's phase and the
    // brightness range (uniforms), distance dimming (vertex shader) — is applied without
    // touching a vertex, so the clock advancing and the camera moving both cost nothing.

    for (traj_mesh, mut cache, mut visibility, mesh3d, material_handle, mut transform) in
        trajectory_meshes.iter_mut()
    {
        // Find the body this trajectory belongs to using direct entity lookup (O(1))
        let Ok(body_ref) = bodies.get(traj_mesh.body_entity) else {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        };
        let Some(body_index) = system.0.index_of(body_ref.0) else {
            if *visibility != Visibility::Hidden { *visibility = Visibility::Hidden; }
            continue;
        };
        let info = system.0.info(body_index);
        let motive = system.0.motive(body_index);

        // Check visibility conditions
        let selected_match = focused_body_state.is_focused(&info.id);
        let should_show = cache.valid
            && !cache.local_points.is_empty()
            && (
                view_settings.show_trajectories
                    || view_settings.body_in_any_trajectory_tag(&info.id)
                    || (selected_match && view_settings.show_selected_trajectories)
                    || hover_state.is_body_hovered(&info.id)
            );

        if !should_show {
            if *visibility != Visibility::Hidden {
                *visibility = Visibility::Hidden;
            }
            continue;
        }

        if *visibility == Visibility::Hidden {
            *visibility = Visibility::Visible;
        }

        // Update trajectory transform every frame:
        // - translation anchors the mesh at the primary focus
        // - rotation applies current perifocal->reference orientation
        // A named primary that does not resolve leaves the ellipse unplaceable. Anchoring
        // it at the origin drew a correctly shaped orbit around the Sun, which reads as
        // real; hide it instead.
        let primary_world_pos = match &cache.primary_id {
            Some(pid) => match system.0.by_name(pid) {
                Some(i) => system.0.position(i),
                None => {
                    *visibility = Visibility::Hidden;
                    continue;
                }
            },
            None => DVec3::ZERO,
        };
        transform.translation = primary_world_pos.to_render_relative(distance_scale, camera_pos);

        let current_perifocal_to_reference = match motive.motive_at(sim_time.time) {
            (_, MotiveSelection::Keplerian(k)) => Some(DQuat::from_mat3(&k.perifocal_to_reference_matrix(sim_time.time))),
            _ => None,
        };
        let base_perifocal_to_reference = cache.base_perifocal_to_reference.unwrap_or(DQuat::IDENTITY);
        transform.rotation = current_perifocal_to_reference
            .unwrap_or(base_perifocal_to_reference)
            .to_render_rotation();

        // Where the body is along its own sampled cycle, 0..1. This is the only thing the
        // passage of time changes about a trajectory, and it is one float in a uniform —
        // the geometry below is untouched by it.
        let current_time = sim_time.time.to_j2000_seconds();
        let (phase_now, phase_wrap) = match orbit_phase(current_time, cache.interval_start, cache.interval_size) {
            Some(phase) => (phase, 1.0),
            None => (0.0, 0.0),
        };
        // Guarded, like every other material write here: an unchanged uniform must not be
        // re-uploaded.
        let phase_changed = materials
            .get(&material_handle.0)
            .map(|m| (m.phase_now - phase_now).abs() > 1e-7 || m.phase_wrap != phase_wrap)
            .unwrap_or(false);
        if phase_changed && let Some(material) = materials.get_mut(&material_handle.0) {
            material.phase_now = phase_now;
            material.phase_wrap = phase_wrap;
        }

        // The geometry lives in mesh-local perifocal space, which the transform and the
        // uniforms above account for entirely. So it survives both the clock advancing and
        // the camera moving, and is rebuilt only when the sampled points or the world
        // scale actually change.
        let scale_changed = cache.last_distance_scale != Some(distance_scale);
        if !cache.mesh_dirty && !scale_changed {
            continue;
        }

        let point_count = cache.local_points.len();
        if point_count < 2 {
            *visibility = Visibility::Hidden;
            continue;
        }

        let base_reference_to_perifocal = base_perifocal_to_reference.inverse();
        let vertices: Vec<(Vec3, f32, f32, f32)> = cache
            .local_points
            .iter()
            .enumerate()
            .map(|(idx, (point_time, pos))| {
                let mesh_local = (base_reference_to_perifocal * *pos).to_render_scaled_f32(distance_scale);
                // A closed orbit bakes its static phase and lets the shader wrap it against
                // the body's; an open one has no cycle to wrap, so it bakes `t` directly.
                let alpha = match cache.interval_size {
                    Some(size) if phase_wrap > 0.5 && size != 0.0 => (point_time / size) as f32,
                    _ => idx as f32 / (point_count - 1) as f32,
                };
                // Amplitude is the shader's job now; the tube stays at its canonical radius
                // and the vertex shader displaces it along the normals.
                (mesh_local, alpha, 1.0, TRAJECTORY_BASE_TUBE_RADIUS)
            })
            .collect();

        if let Some(mesh_asset) = meshes.get_mut(&mesh3d.0) {
            // The samples are in perifocal space, so the orbit's plane normal is the
            // render image of simulation +Z.
            *mesh_asset = generate_tube_mesh(&vertices, TUBE_SIDES, Some(DVec3::Z.to_render()));
        }

        cache.mesh_dirty = false;
        cache.last_distance_scale = Some(distance_scale);
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
    fcam: Single<&Freecam, With<PlanetariumCamera>>,
    bodies: Query<(Entity, &BodyRef, Option<&TrajectoryMeshLink>)>,
    system: Res<SimSystem>,
    trajectories: Res<Trajectories>,
    trajectory_caches: Query<&TrajectoryCache>,
    mut markers: Query<(Entity, &mut FocusedTrajectoryMarker, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    if !view_settings.show_selected_trajectories {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    }

    let Some(focused_id) = focused_body_state.current_body_id.as_deref() else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let Some(focused_index) = system.0.by_name(focused_id) else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };
    let motive = system.0.motive(focused_index);
    let traj_link = bodies.iter()
        .find(|(_, body_ref, _)| body_ref.0 == system.0.id(focused_index))
        .and_then(|(_, _, link)| link);

    let (_, selection) = motive.motive_at(sim_time.time);
    let MotiveSelection::Keplerian(kepler) = selection else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let Some(primary_index) = system.0.by_name(&kepler.primary_id) else {
        despawn_all_focused_trajectory_markers(&mut commands, &mut markers);
        return;
    };

    let primary_position = system.0.position(primary_index);
    // Arena mu honours the explicit override; recomputing from primary mass would be
    // wrong for barycentric orbits.
    let mu = system.0.mu(focused_index);
    let period_seconds = kepler.period(mu).to_seconds();
    let periapsis_base = kepler.time_at_periapsis_passage(mu);

    let Some(trajectory) = trajectories.0.get(&system.0.id(focused_index)).map(|p| &p.points) else {
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
    let current_perifocal_to_reference = DQuat::from_mat3(&kepler.perifocal_to_reference_matrix(sim_time.time));
    let base_perifocal_to_reference = traj_link
        .and_then(|link| trajectory_caches.get(link.0).ok())
        .and_then(|cache| cache.base_perifocal_to_reference)
        .unwrap_or(current_perifocal_to_reference);
    let base_reference_to_perifocal = base_perifocal_to_reference.inverse();
    let map_local_from_base_to_current = |base_local: DVec3| {
        let perifocal = base_reference_to_perifocal * base_local;
        current_perifocal_to_reference * perifocal
    };

    let periapsis_local_current = map_local_from_base_to_current(periapsis_local);
    let apoapsis_local_current = apoapsis_local.map(map_local_from_base_to_current);

    let periapsis_world = primary_position + periapsis_local_current;
    let apoapsis_world = apoapsis_local_current.map(|apo| primary_position + apo);
    let peri_times = repeating_event_prev_next(periapsis_base, period_seconds, sim_time.time);
    let apo_times = repeating_event_prev_next(
        crate::foundations::time::Instant::from_seconds_since_j2000(
            periapsis_base.to_j2000_seconds() + period_seconds * 0.5,
        ),
        period_seconds,
        sim_time.time,
    );

    let distance_scale = view_settings.distance_factor();
    let peri_bevy = periapsis_world.to_render_relative(distance_scale, fcam.bevy_pos);
    let peri_tube_radius = calculate_tube_radius(peri_bevy.length());
    let peri_marker_radius = 2.0 * peri_tube_radius;

    let apo_bevy = apoapsis_world.map(|pos| pos.to_render_relative(distance_scale, fcam.bevy_pos));
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
    view_settings: Res<ViewSettings>,
    sim_time: Res<SimTime>,
    _fcam: Single<&Freecam, With<PlanetariumCamera>>,
    bodies: Query<(&BodyRef, Option<&TrajectoryMeshLink>)>,
    system: Res<SimSystem>,
    trajectory_caches: Query<&TrajectoryCache>,
    mut markers: Query<(Entity, &mut FocusedTrajectoryMarker, &mut Transform)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<TrajectoryMaterial>>,
) {
    if !view_settings.show_selected_trajectories {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    }

    let Some(hit_data) = hover_state.hovered_trajectory_hit.as_ref() else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

    let Some(focused_id) = focused_body_state.current_body_id.as_deref() else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

    let Some(focused_index) = system.0.by_name(focused_id) else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

    let motive = system.0.motive(focused_index);
    let focused_id_hash = system.0.id(focused_index);
    let traj_link = bodies.iter()
        .find(|(body_ref, _)| body_ref.0 == focused_id_hash)
        .and_then(|(_, link)| link);

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

    // Arena mu: G(M+m) with the explicit override honoured. Open-coding G*M here made
    // the readout disagree with the propagated position, and an unresolved primary gave
    // mu = 0, which feeds infinities into the period.
    let mu = system.0.mu(focused_index);
    let period_seconds = kepler.period(mu).to_seconds();
    let periapsis_base = kepler.time_at_periapsis_passage(mu);

    // Compute true anomaly using Newton refinement (20 iterations in fourier_expansion)
    let Some(true_anomaly) = refine_true_anomaly_newton(
        &kepler,
        mu,
        hit_data.start_time,
        hit_data.end_time,
        hit_data.t,
        periapsis_base,
    ) else {
        despawn_trajectory_marker(&mut commands, &mut markers, FocusedTrajectoryMarkerKind::MouseHit);
        return;
    };

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
) -> Option<f64> {
    // Initial guess: linear interpolation of time
    let interpolated_relative_time = start_time + t * (end_time - start_time);
    let absolute_time = crate::foundations::time::Instant::from_seconds_since_j2000(
        periapsis_base.to_j2000_seconds() + interpolated_relative_time
    );
    
    // Use kepler's eccentricity via public method
    let ecc = kepler.eccentricity();
    let mean_anomaly = kepler.mean_anomaly(absolute_time, mu);

    // Solved, not expanded: the series diverges past the Laplace limit, e > 0.6627.
    // `None` at e == 1: mean anomaly is undefined for a parabola, and substituting it
    // for the true anomaly would report a confident angle that means nothing.
    em_foundations::kepler::anomaly::true_from_mean(mean_anomaly, ecc)
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
    let material_handle = materials.add(TrajectoryMaterial {
        dynamic_thickness: 0.0,
        ..Default::default()
    });

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
/// distance along the cached polyline. `t = 0` maps to the trajectory's front
/// brightness, `t = 1` to its back brightness (the lerp itself happens in the shader).
/// Where the body sits along its own sampled cycle, as a fraction in `0..1`, or `None` for
/// a trajectory with no cycle to be a fraction of.
///
/// This is the whole of what the clock contributes to a trajectory's appearance. The
/// shader pairs it with each vertex's baked phase as `fract(phase - phase_now)`, which is
/// the offset *forward* along the orbit from the body to that vertex — so `t` is 0 at the
/// body and approaches 1 coming back round to it.
fn orbit_phase(current_time: f64, interval_start: Option<f64>, interval_size: Option<f64>) -> Option<f32> {
    let (start, size) = (interval_start?, interval_size?);
    if size == 0.0 || !size.is_finite() {
        return None;
    }
    Some(((current_time - start) / size).rem_euclid(1.0) as f32)
}

/// Generate a tube mesh from a list of points with associated brightness and radius values.
/// Each point becomes a ring of vertices; adjacent rings are connected with triangles.
/// Points are (position, t, amplitude, radius), where `t` is the along-length lerp
/// factor (baked into vertex alpha) and `amplitude` is a per-vertex brightness scale
/// such as distance dimming (baked into vertex rgb). The shader turns these into the
/// final brightness using the material's front/back/exposure uniforms.
/// `plane_normal` is the normal of the plane the curve lies in, if it lies in one. Passing
/// it is what keeps the tube from pinching; see [`ring_basis`].
pub fn generate_tube_mesh(
    points: &[(Vec3, f32, f32, f32)],
    sides: u32,
    plane_normal: Option<Vec3>,
) -> Mesh {
    if points.len() < 2 {
        return empty_trajectory_mesh();
    }
    
    let ring_count = points.len();

    // A closed orbit is sampled at both ends of its period, so the last point sits on the
    // first. Wrapping the tangent difference across that join gives those two rings the
    // same frame, and the tube meets itself instead of creasing at periapsis.
    let wraps = ring_count >= 3
        && (points[0].0 - points[ring_count - 1].0).length_squared()
            <= 1e-12 * points[0].0.length_squared().max(1.0);
    let verts_per_ring = sides as usize;
    let total_verts = ring_count * verts_per_ring;
    
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(total_verts);
    let mut indices: Vec<u32> = Vec::with_capacity((ring_count - 1) * verts_per_ring * 6);
    
    for (ring_idx, (center, t, amplitude, radius)) in points.iter().enumerate() {
        // Compute tangent direction (forward along the tube)
        let tangent = if wraps {
            let previous = if ring_idx == 0 { ring_count - 2 } else { ring_idx - 1 };
            let next = if ring_idx == ring_count - 1 { 1 } else { ring_idx + 1 };
            (points[next].0 - points[previous].0).normalize_or_zero()
        } else if ring_idx == 0 {
            (points[1].0 - *center).normalize_or_zero()
        } else if ring_idx == ring_count - 1 {
            (*center - points[ring_idx - 1].0).normalize_or_zero()
        } else {
            (points[ring_idx + 1].0 - points[ring_idx - 1].0).normalize_or_zero()
        };
        
        // Find perpendicular vectors to form the ring plane
        let (perp1, perp2) = ring_basis(tangent, plane_normal);
        
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
/// The orthonormal pair spanning one ring of a tube.
///
/// For a curve that lies in a known plane — every Keplerian orbit does, in perifocal
/// space — the plane's normal anchors the frame: `perp1` stays in the plane and `perp2`
/// along the normal, so consecutive rings agree and the frame returns to itself around a
/// closed orbit.
///
/// Without that anchor the frame comes from [`perpendicular_vectors`], which picks
/// whichever axis is least parallel to the tangent. That both drifts continuously along
/// the tube and jumps outright when the choice of axis flips, which on an ellipse happens
/// several times per revolution — the visible kink where a ring's vertices suddenly
/// reorder.
fn ring_basis(tangent: Vec3, plane_normal: Option<Vec3>) -> (Vec3, Vec3) {
    if let Some(normal) = plane_normal {
        let perp1 = tangent.cross(normal);
        // Degenerate only if the tangent left the plane, which a planar curve's cannot.
        if perp1.length_squared() > 1e-12 {
            let perp1 = perp1.normalize();
            // Same handedness as the fallback below, so winding and outward normals are
            // unchanged.
            return (perp1, tangent.cross(perp1).normalize_or_zero());
        }
    }
    perpendicular_vectors(tangent)
}

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
    bodies: Query<Entity, (With<BodyRef>, Without<TrajectoryMeshLink>)>,
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
    mut removed_bodies: RemovedComponents<BodyRef>,
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

#[cfg(test)]
mod tests {
    use super::{generate_tube_mesh, orbit_phase, ring_basis, TUBE_SIDES};
    use bevy::prelude::*;
    use bevy_mesh::VertexAttributeValues;

    /// The shader's half of the phase pair: `t` for a vertex baked at `phase`.
    fn wrapped_t(phase: f32, phase_now: f32) -> f32 {
        (phase - phase_now).rem_euclid(1.0)
    }

    fn angle_between(a: Vec3, b: Vec3) -> f32 {
        a.normalize().dot(b.normalize()).clamp(-1.0, 1.0).acos()
    }

    /// Tangents around one revolution of an orbit lying in the XZ plane, which is where
    /// perifocal space puts it once converted to render space.
    fn tangent_at(step: usize, steps: usize) -> Vec3 {
        let a = step as f32 / steps as f32 * std::f32::consts::TAU;
        Vec3::new(a.cos(), 0.0, a.sin())
    }

    /// The ring frame turns with the curve and nothing else. Anchored to the orbital
    /// plane it tracks the sampling step; left to pick its own axis it jumps 90 degrees
    /// between adjacent rings where the choice of axis flips, which is the pinch.
    #[test]
    fn the_ring_frame_follows_the_curve_instead_of_jumping() {
        let steps = 720;
        let step_angle = std::f32::consts::TAU / steps as f32;
        let (mut worst_anchored, mut worst_free) = (0.0f32, 0.0f32);
        for i in 0..steps {
            let (a, _) = ring_basis(tangent_at(i, steps), Some(Vec3::Y));
            let (b, _) = ring_basis(tangent_at(i + 1, steps), Some(Vec3::Y));
            worst_anchored = worst_anchored.max(angle_between(a, b));
            let (c, _) = ring_basis(tangent_at(i, steps), None);
            let (d, _) = ring_basis(tangent_at(i + 1, steps), None);
            worst_free = worst_free.max(angle_between(c, d));
        }
        assert!(worst_anchored <= step_angle * 1.01,
            "anchored frame turned {worst_anchored} between rings a step of {step_angle} apart");
        assert!(worst_free > 1.0,
            "the unanchored frame is supposed to be the bad case, but only turned {worst_free}");
    }

    /// The anchored frame is orthonormal and square to the tangent, with one axis in the
    /// orbital plane and one along its normal.
    #[test]
    fn the_ring_frame_is_orthonormal_and_square_to_the_curve() {
        for i in 0..64 {
            let tangent = tangent_at(i, 64);
            let (perp1, perp2) = ring_basis(tangent, Some(Vec3::Y));
            assert!((perp1.length() - 1.0).abs() < 1e-5);
            assert!((perp2.length() - 1.0).abs() < 1e-5);
            assert!(perp1.dot(tangent).abs() < 1e-5, "perp1 square to the tangent");
            assert!(perp1.dot(perp2).abs() < 1e-5, "perps square to each other");
            assert!(perp1.y.abs() < 1e-5, "perp1 stays in the orbital plane");
            assert!(perp2.x.abs() < 1e-5 && perp2.z.abs() < 1e-5, "perp2 stays along the normal");
            // Same handedness as the fallback, so winding and outward normals are unchanged.
            assert!((perp2 - tangent.cross(perp1)).length() < 1e-5);
        }
    }

    /// An ellipse sampled at both ends of its period closes on itself, and so must the
    /// tube: every ring meets its neighbour, including the last meeting the first.
    #[test]
    fn a_closed_orbits_tube_meets_itself() {
        // A sampled ellipse in the plane perifocal space puts the orbit in, with the
        // duplicated end sample `em_sim::trajectory::sample` produces.
        let resolution = 120;
        let points: Vec<(Vec3, f32, f32, f32)> = (0..=resolution)
            .map(|i| {
                let a = i as f32 / resolution as f32 * std::f32::consts::TAU;
                (Vec3::new(3.0 * a.cos(), 0.0, 2.0 * a.sin()), 0.0, 1.0, 0.05)
            })
            .collect();

        let mesh = generate_tube_mesh(&points, TUBE_SIDES, Some(Vec3::Y));
        let Some(VertexAttributeValues::Float32x3(normals)) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else { panic!("tube mesh must carry normals") };

        // Vertex 0 of each ring sits at angle 0, so its outward normal is that ring's perp1.
        let sides = TUBE_SIDES as usize;
        let perp1_of = |ring: usize| Vec3::from_array(normals[ring * sides]);

        let mut worst = 0.0f32;
        for ring in 0..resolution {
            worst = worst.max(angle_between(perp1_of(ring), perp1_of(ring + 1)));
        }
        // Neighbouring rings on a 120-sample ellipse are ~3 degrees apart; anything
        // approaching a right angle is the frame flipping rather than the curve turning.
        assert!(worst < 0.2, "worst turn between neighbouring rings was {worst} rad");

        // The duplicated end sample lands on the first, so their frames must agree exactly.
        let seam = angle_between(perp1_of(0), perp1_of(resolution));
        assert!(seam < 1e-4, "the tube creases at periapsis by {seam} rad");
    }

    /// Phase advances linearly through the cycle and wraps, and lands back on 0 exactly one
    /// period on. Anchoring on the periapsis epoch rather than the clock is what lets the
    /// geometry outlive the frame.
    #[test]
    fn phase_walks_the_cycle_and_wraps() {
        let (start, size) = (1_000.0, 400.0);
        for (t, expected) in [(1_000.0, 0.0), (1_100.0, 0.25), (1_300.0, 0.75), (1_400.0, 0.0), (1_700.0, 0.75)] {
            let got = orbit_phase(t, Some(start), Some(size)).unwrap();
            assert!((got - expected).abs() < 1e-6, "at {t}: {got} != {expected}");
        }
        // Before the epoch wraps forward rather than going negative.
        assert!((orbit_phase(900.0, Some(start), Some(size)).unwrap() - 0.75).abs() < 1e-6);
    }

    /// An open trajectory has no cycle, and a degenerate period is not one either.
    #[test]
    fn a_trajectory_without_a_cycle_has_no_phase() {
        assert!(orbit_phase(0.0, None, Some(400.0)).is_none());
        assert!(orbit_phase(0.0, Some(0.0), None).is_none());
        assert!(orbit_phase(0.0, Some(0.0), Some(0.0)).is_none());
        assert!(orbit_phase(0.0, Some(0.0), Some(f64::INFINITY)).is_none());
    }

    /// `t` is 0 at the body and rises to just under 1 immediately behind it, so the
    /// front-to-back gradient has its seam exactly where the body is — which is what the
    /// pair of transient vertices used to buy on the CPU.
    #[test]
    fn brightness_seam_sits_on_the_body() {
        let phase_now = orbit_phase(1_100.0, Some(1_000.0), Some(400.0)).unwrap();
        assert!(wrapped_t(phase_now, phase_now).abs() < 1e-6, "t is 0 at the body");

        let just_ahead = wrapped_t(phase_now + 0.001, phase_now);
        let just_behind = wrapped_t(phase_now - 0.001, phase_now);
        assert!(just_ahead < 0.01, "just ahead of the body is the bright end: {just_ahead}");
        assert!(just_behind > 0.99, "just behind it is the dim end: {just_behind}");
    }

    /// A vertex baked at phase 1.0 (the sample closing the loop) and one at 0.0 land on the
    /// same `t`, so the closed orbit has no seam of its own.
    #[test]
    fn the_closing_sample_matches_the_opening_one() {
        for phase_now in [0.0, 0.3, 0.75, 0.999] {
            assert!((wrapped_t(1.0, phase_now) - wrapped_t(0.0, phase_now)).abs() < 1e-6);
        }
    }
}
