//! Planetarium camera: goto animation and revolve-around-body behavior.

use std::f64::consts::{PI, TAU};

/// Minimum orbit distance as a multiple of the body's visual radius.
const ORBIT_ZOOM_MIN_RADIUS_MULT: f64 = 1.2;
/// Maximum orbit distance in light-years.
const ORBIT_ZOOM_MAX_LY: f64 = 1.0;
/// Base scroll-zoom speed (fraction of distance per scroll tick at close range).
const ORBIT_ZOOM_BASE_SPEED: f64 = 0.001;
/// Additional speed gained per order of magnitude above min distance.
const ORBIT_ZOOM_ACCELERATION: f64 = 0.006;
/// One light-year in meters.
const LIGHT_YEAR_M: f64 = 9.460_730_472_580_8e15;
use bevy::app::App;
use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::EguiContexts;
use num_traits::Float;
use crate::body::appearance::Appearance;
use em_sim::motive::{Motive, MotiveSelection};
use em_sim::body::BodyInfo;
use crate::body::universe::save::ViewSettings;
use crate::gui::app::AppState;
use crate::gui::planetarium::{FocusedBodyState, HoverState, HoveredTrajectoryMarkerKind, TrajectoryHitData};
use crate::presentation::{FocusedTrajectoryMarker, FocusedTrajectoryMarkerKind};
use crate::sim::world::{self, BodyRef, SimSystem};
use em_sim::system::System;
use crate::sim::SimTime;
use crate::camera::freecam::{FreeCamPlugin, Freecam, MovementSettings};
use crate::presentation::render_space::ToRender;
use crate::util::ease;

pub struct PlanetariumCameraPlugin;

impl Plugin for PlanetariumCameraPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_plugins(FreeCamPlugin)
            .add_message::<GoTo>()
            .add_systems(Update, (
                handle_gotos,
                run_goto.before(world::sync_transforms).after(world::advance_simulation),
                revolve_around.before(world::sync_transforms).after(world::advance_simulation),
                update_hover_target,
                pick_body_on_click,
                ).run_if(in_state(AppState::Planetarium)))
        ;
    }
}

#[derive(Component)]
pub struct PlanetariumCamera {
    pub action: CameraAction,
}

impl PlanetariumCamera {
    pub fn new() -> Self {
        Self {
            action: CameraAction::Free,
        }
    }
}

#[derive(Clone)]
pub enum CameraAction {
    Free,
    Goto(GoToInProgress),
    RevolveAround(RevolveAround),
}

impl CameraAction {
    pub fn revolve_target(&self) -> Option<(Entity, RevolveAroundFrame)> {
        match self {
            Self::RevolveAround(revolve) => Some((revolve.entity, revolve.frame)),
            _ => None,
        }
    }

    pub fn goto_target(&self) -> Option<(Entity, RevolveAroundFrame)> {
        match self {
            Self::Goto(goto) => Some((goto.entity, goto.end_frame)),
            _ => None,
        }
    }
}

impl PartialEq for CameraAction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CameraAction::Free, CameraAction::Free) => true,
            (CameraAction::Goto(_), CameraAction::Goto(_)) => true,
            (CameraAction::RevolveAround(_), CameraAction::RevolveAround(_)) => true,
            (_, _) => false,
        }
    }
}

#[derive(Message)]
pub struct GoTo {
    pub entity: Entity,
    pub frame: Option<RevolveAroundFrame>,
    pub source: GoToSource,
}

#[derive(Debug, Clone, Copy)]
pub enum GoToSource {
    UiButton,
    KeyboardG,
    KeyboardF,
    KeyboardV,
}

#[derive(Clone)]
pub enum GoToOrigin {
    /// Fixed starting position (from Free mode)
    Position {
        position: DVec3,
        velocity: DVec3,
        sample_time: f64,
    },
    /// Revolving around an entity — track its position + offset each frame
    Revolving(RevolveAround),
}

#[derive(Clone)]
pub struct GoToInProgress {
    origin: GoToOrigin,
    start_rot: Quat,
    start_time: f64,
    end_distance: f64,
    end_altitude: f64,
    end_azimuth: f64,
    end_frame: RevolveAroundFrame,
    entity: Entity,
}

#[derive(Clone)]
pub struct RevolveAround {
    entity: Entity,
    bevy_distance: f64,
    altitude: f64,
    azimuth: f64,
    frame: RevolveAroundFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevolveAroundFrame {
    Global,
    Perifocal,
    Local,
}

impl RevolveAroundFrame {
    pub fn next(&self) -> Self {
        match self {
            Self::Global => Self::Perifocal,
            Self::Perifocal => Self::Local,
            Self::Local => Self::Global,
        }
    }
}

fn handle_gotos (
    mut go_tos: MessageReader<GoTo>,
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    bodies: Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: Res<SimSystem>,
    view_settings: Res<ViewSettings>,
    time: Res<Time>,
    sim_time: Res<SimTime>,
) {
    if let Ok((cam_t, mut pcam, fcam)) = camera.single_mut() {
        let now = time.elapsed().as_secs_f64();
        for event in go_tos.read() {
            let start_rot = cam_t.rotation;

            let origin = match &pcam.action {
                CameraAction::RevolveAround(revolve) => GoToOrigin::Revolving(revolve.clone()),
                CameraAction::Goto(current_goto) => {
                    let velocity = estimate_goto_velocity(
                        current_goto,
                        now,
                        &bodies,
                        &system.0,
                        &view_settings,
                        &sim_time,
                    ).unwrap_or(DVec3::ZERO);
                    info!(
                        "cam.goto.interrupt source={:?} entity={:?} velocity={:?}",
                        event.source,
                        event.entity,
                        velocity,
                    );
                    GoToOrigin::Position {
                        position: fcam.bevy_pos,
                        velocity,
                        sample_time: now,
                    }
                }
                _ => GoToOrigin::Position {
                    position: fcam.bevy_pos,
                    velocity: DVec3::ZERO,
                    sample_time: now,
                },
            };

            let Ok((entity, body_ref, transform)) = bodies.get(event.entity) else {
                info!("cam.goto.missing_target source={:?} entity={:?}", event.source, event.entity);
                continue;
            };
            let Some(index) = system.0.index_of(body_ref.0) else {
                info!("cam.goto.missing_body source={:?} entity={:?}", event.source, event.entity);
                continue;
            };
            let appearance = system.0.appearance(index);
            let info = system.0.info(index);
            let motive = system.0.motive(index);
            let obj_pos = system.0.position(index);

            let nearby_distance = if matches!(appearance, Appearance::Empty) {
                let max_child_sma = find_max_child_sma(&info.id, &system.0, sim_time.time);
                if max_child_sma > 0.0 {
                    1.5 * max_child_sma * view_settings.distance_factor()
                } else {
                    3.0 * view_settings.distance_factor()
                }
            } else {
                3.0 * view_settings.body_scale_factor(appearance.radius()) as f64
            };
            let body_pos = obj_pos.to_render_scaled(view_settings.distance_factor());
            let current_distance = (fcam.bevy_pos - body_pos).length();
            let preserve_distance = matches!(event.source, GoToSource::KeyboardV | GoToSource::KeyboardF)
                && current_distance.is_finite()
                && current_distance > f64::EPSILON;
            let end_distance = if preserve_distance {
                current_distance
            } else {
                nearby_distance
            };
            let requested_frame = event.frame
                .or_else(|| pcam.action.goto_target().map(|(_, frame)| frame))
                .or_else(|| pcam.action.revolve_target().map(|(_, frame)| frame))
                .unwrap_or(RevolveAroundFrame::Global);
            let frame = resolve_frame_or_fallback(requested_frame, motive, sim_time.time, entity);
            let (altitude, azimuth) = alt_az_in_frame(
                body_pos,
                fcam.bevy_pos,
                frame,
                motive,
                sim_time.time,
                transform,
                entity,
            );
            info!(
                "cam.goto.start source={:?} entity={:?} frame={:?} distance={} preserve_distance={} altitude={} azimuth={}",
                event.source,
                entity,
                frame,
                end_distance,
                preserve_distance,
                altitude,
                azimuth
            );

            pcam.action = CameraAction::Goto(GoToInProgress {
                origin,
                start_rot,
                start_time: now,
                entity,
                end_distance,
                end_altitude: altitude,
                end_azimuth: azimuth,
                end_frame: frame,
            });
        }
    }
}

fn run_goto (
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    bodies: Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: Res<SimSystem>,
    time: Res<Time>,
    view_settings: Res<ViewSettings>,
    sim_time: Res<SimTime>,
) {
    let animation_time = 2.0;
    let now = time.elapsed().as_secs_f64();
    let mut next_action = None;

    if let Ok((mut cam_t, mut pcam, mut fcam)) = camera.single_mut() {
        match &mut pcam.action {
            CameraAction::Goto(goto) => {
                let resolved = bodies.get(goto.entity).ok()
                    .and_then(|(entity, body_ref, transform)| {
                        system.0.index_of(body_ref.0).map(|i| (entity, i, transform))
                    });
                if let Some((entity, index, transform)) = resolved {
                    let motive = system.0.motive(index);
                    let frac = f64::min(1.0, (now - goto.start_time) / animation_time);
                    let frac = ease::f64::circ(frac);
                    let frame = resolve_frame_or_fallback(goto.end_frame, motive, sim_time.time, entity);
                    if frame != goto.end_frame {
                        info!(
                            "cam.goto.frame_fallback entity={:?} from={:?} to={:?}",
                            entity,
                            goto.end_frame,
                            frame
                        );
                        goto.end_frame = frame;
                    }

                    let start_pos = match &goto.origin {
                        GoToOrigin::Position { position, velocity, sample_time } => {
                            let dt = (now - *sample_time).max(0.0);
                            let evaluated = *position + *velocity * dt;
                            debug!(
                                "cam.goto.origin_kinematic entity={:?} dt={} position={:?} velocity={:?} evaluated={:?}",
                                entity,
                                dt,
                                position,
                                velocity,
                                evaluated
                            );
                            evaluated
                        }
                        GoToOrigin::Revolving(revolving) => {
                            let origin_resolved = bodies.get(revolving.entity).ok()
                                .and_then(|(e, r, t)| system.0.index_of(r.0).map(|i| (e, i, t)));
                            if let Some((origin_entity, origin_index, origin_transform)) = origin_resolved {
                                let origin_motive = system.0.motive(origin_index);
                                let origin_pos = system.0.position(origin_index).to_render_scaled(view_settings.distance_factor());
                                let (origin_offset, _) = offset_in_frame(
                                    revolving.frame,
                                    revolving.altitude,
                                    revolving.azimuth,
                                    revolving.bevy_distance,
                                    origin_motive,
                                    sim_time.time,
                                    origin_transform,
                                    origin_entity,
                                );
                                origin_pos + origin_offset
                            } else {
                                fcam.bevy_pos
                            }
                        }
                    };

                    let body_pos_in_bevy = system.0.position(index).to_render_scaled(view_settings.distance_factor());

                    let (offset, _) = offset_in_frame(
                        goto.end_frame,
                        goto.end_altitude,
                        goto.end_azimuth,
                        goto.end_distance,
                        motive,
                        sim_time.time,
                        transform,
                        entity,
                    );
                    let final_pos = body_pos_in_bevy + offset;

                    let up = up_vector_for_frame(goto.end_frame, motive, sim_time.time, transform, entity);
                    let look_at_rot = look_at(final_pos, body_pos_in_bevy, up);

                    let mid_pos = start_pos.lerp(final_pos, frac);
                    let mid_rot = goto.start_rot.slerp(look_at_rot.as_quat(), frac as f32);
                    fcam.bevy_pos = mid_pos;
                    cam_t.rotation = mid_rot;

                    if (frac - 1.0).abs() <= f64::epsilon() {
                        next_action = Some(CameraAction::RevolveAround(RevolveAround {
                            entity: goto.entity,
                            bevy_distance: goto.end_distance,
                            altitude: goto.end_altitude,
                            azimuth: goto.end_azimuth,
                            frame: goto.end_frame,
                        }));
                        info!(
                            "cam.goto.complete entity={:?} frame={:?} distance={} altitude={} azimuth={}",
                            goto.entity,
                            goto.end_frame,
                            goto.end_distance,
                            goto.end_altitude,
                            goto.end_azimuth
                        );
                    }
                } else {
                    info!("cam.goto.target_lost entity={:?}", goto.entity);
                    next_action = Some(CameraAction::Free);
                }
            }
            _ => {}
        }

        if let Some(next_action) = next_action {
            pcam.action = next_action;
        }
    }
}

#[derive(Default)]
struct PickState {
    last_pick_id: Option<String>,
    last_pick_time: f64,
}

const DOUBLE_CLICK_WINDOW: f64 = 0.5;
const HOVER_PIXEL_RADIUS: f32 = 20.0;
const MIN_PICK_RADIUS: f32 = 0.01;
const TRAJECTORY_PICK_PIXEL_RADIUS: f32 = 10.0;

#[derive(Default)]
struct HoverPickResult {
    hovered_body_id: Option<String>,
    hovered_marker_kind: Option<HoveredTrajectoryMarkerKind>,
    trajectory_hit: Option<TrajectoryHitData>,
}

fn update_hover_target(
    primary_window: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &Projection, &Transform, &Freecam), With<PlanetariumCamera>>,
    bodies: Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: Res<SimSystem>,
    markers: Query<(&FocusedTrajectoryMarker, &Transform)>,
    trajectory_caches: Query<(&crate::presentation::TrajectoryMesh, &crate::presentation::TrajectoryCache)>,
    view_settings: Res<ViewSettings>,
    sim_time: Res<SimTime>,
    focused_body_state: Res<FocusedBodyState>,
    mut egui_ctx: EguiContexts,
    mut hover_state: ResMut<HoverState>,
) {
    let hovered = pick_hover_target(
        &primary_window,
        &cameras,
        &bodies,
        &system.0,
        &markers,
        &trajectory_caches,
        &view_settings,
        &sim_time,
        &focused_body_state,
        &mut egui_ctx,
    );
    hover_state.hovered_body_id = hovered.hovered_body_id;
    hover_state.hovered_marker_kind = hovered.hovered_marker_kind;
    hover_state.hovered_trajectory_hit = hovered.trajectory_hit;
}

fn pick_body_on_click(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    primary_window: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &Projection, &Transform, &Freecam), With<PlanetariumCamera>>,
    bodies: Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: Res<SimSystem>,
    markers: Query<(&FocusedTrajectoryMarker, &Transform)>,
    trajectory_caches: Query<(&crate::presentation::TrajectoryMesh, &crate::presentation::TrajectoryCache)>,
    view_settings: Res<ViewSettings>,
    sim_time: Res<SimTime>,
    mut egui_ctx: EguiContexts,
    time: Res<Time>,
    mut pick_state: Local<PickState>,
    mut focused_body_state: ResMut<FocusedBodyState>,
) {
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }

    let hovered = pick_hover_target(
        &primary_window,
        &cameras,
        &bodies,
        &system.0,
        &markers,
        &trajectory_caches,
        &view_settings,
        &sim_time,
        &focused_body_state,
        &mut egui_ctx,
    );
    if let Some(selected_id) = hovered.hovered_body_id {
        let info = system.0.by_name(&selected_id).map(|i| system.0.info(i));
        if let Some(info) = info {
        let now = time.elapsed().as_secs_f64();
        let is_double = pick_state.last_pick_id.as_deref() == Some(&info.id)
            && (now - pick_state.last_pick_time) <= DOUBLE_CLICK_WINDOW;

        if is_double {
            focused_body_state.current_body_id = Some(info.id.clone());
            info!("Selected body: {} (id: {})", info.display_name(), info.id);
            pick_state.last_pick_id = None;
        } else {
            info!("Picked body: {} (id: {})", info.display_name(), info.id);
            pick_state.last_pick_id = Some(info.id.clone());
            pick_state.last_pick_time = now;
        }
        }
    }
}

fn pick_hover_target(
    primary_window: &Query<&Window, With<PrimaryWindow>>,
    cameras: &Query<(&Camera, &Projection, &Transform, &Freecam), With<PlanetariumCamera>>,
    bodies: &Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: &System,
    markers: &Query<(&FocusedTrajectoryMarker, &Transform)>,
    trajectory_caches: &Query<(&crate::presentation::TrajectoryMesh, &crate::presentation::TrajectoryCache)>,
    view_settings: &ViewSettings,
    sim_time: &SimTime,
    focused_body_state: &FocusedBodyState,
    egui_ctx: &mut EguiContexts,
) -> HoverPickResult {
    let egui_wants_pointer = egui_ctx.ctx_mut()
        .map_or(false, |ctx| ctx.wants_pointer_input());
    if egui_wants_pointer {
        return HoverPickResult::default();
    }

    let Ok(window) = primary_window.single() else {
        return HoverPickResult::default();
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return HoverPickResult::default();
    };
    let Ok((camera, _projection, cam_transform, freecam)) = cameras.single() else {
        return HoverPickResult::default();
    };

    let fresh_gt = GlobalTransform::from(*cam_transform);
    let Ok(ray) = camera.viewport_to_world(&fresh_gt, cursor_pos) else {
        return HoverPickResult::default();
    };

    let distance_scale = view_settings.distance_factor();
    let ray_origin = ray.origin;
    let ray_dir: Vec3 = *ray.direction;

    let mut body_sphere_hits: Vec<&BodyInfo> = Vec::new();
    let mut body_pixel_hits: Vec<&BodyInfo> = Vec::new();
    for (_, body_ref, _) in bodies.iter() {
        let Some(index) = system.index_of(body_ref.0) else { continue };
        let info = system.info(index);
        let body_pos = system.position(index)
            .to_render_relative(distance_scale, freecam.bevy_pos);

        let visual_radius = view_settings.body_scale_factor(system.radius(index));
        let pick_radius = (visual_radius * 1.5).max(MIN_PICK_RADIUS);
        if ray_hits_sphere(ray_origin, ray_dir, body_pos, pick_radius) {
            body_sphere_hits.push(info);
        }

        if let Ok(screen_pos) = camera.world_to_viewport(&fresh_gt, body_pos) {
            if cursor_pos.distance(screen_pos) <= HOVER_PIXEL_RADIUS {
                body_pixel_hits.push(info);
            }
        }
    }
    let hovered_body_id = if !body_sphere_hits.is_empty() {
        body_sphere_hits
            .iter()
            .max_by(|a, b| a.mass.partial_cmp(&b.mass).unwrap_or(std::cmp::Ordering::Equal))
            .map(|info| info.id.clone())
    } else {
        body_pixel_hits
            .iter()
            .max_by(|a, b| a.mass.partial_cmp(&b.mass).unwrap_or(std::cmp::Ordering::Equal))
            .map(|info| info.id.clone())
    };

    let mut marker_sphere_hits: Vec<(HoveredTrajectoryMarkerKind, f32)> = Vec::new();
    let mut marker_pixel_hits: Vec<(HoveredTrajectoryMarkerKind, f32)> = Vec::new();
    for (marker, marker_transform) in markers.iter() {
        let marker_pos = marker_transform.translation;
        let pick_radius = (marker_transform.scale.x * 1.5).max(MIN_PICK_RADIUS);
        let marker_kind = match marker.kind {
            FocusedTrajectoryMarkerKind::Periapsis => HoveredTrajectoryMarkerKind::Periapsis,
            FocusedTrajectoryMarkerKind::Apoapsis => HoveredTrajectoryMarkerKind::Apoapsis,
            FocusedTrajectoryMarkerKind::MouseHit => continue, // Skip MouseHit markers in picking
        };
        if ray_hits_sphere(ray_origin, ray_dir, marker_pos, pick_radius) {
            let screen_dist = camera
                .world_to_viewport(&fresh_gt, marker_pos)
                .map(|p| cursor_pos.distance(p))
                .unwrap_or(f32::MAX);
            marker_sphere_hits.push((marker_kind, screen_dist));
        }

        if let Ok(screen_pos) = camera.world_to_viewport(&fresh_gt, marker_pos) {
            let screen_dist = cursor_pos.distance(screen_pos);
            if screen_dist <= HOVER_PIXEL_RADIUS {
                marker_pixel_hits.push((marker_kind, screen_dist));
            }
        }
    }
    let hovered_marker_kind = if !marker_sphere_hits.is_empty() {
        marker_sphere_hits
            .into_iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(kind, _)| kind)
    } else {
        marker_pixel_hits
            .into_iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(kind, _)| kind)
    };

    // Fallback: trajectory segment cast only when no body or marker hit
    let trajectory_hit = if hovered_body_id.is_none() && hovered_marker_kind.is_none() {
        pick_trajectory_segment(
            focused_body_state,
            bodies,
            system,
            trajectory_caches,
            sim_time,
            view_settings,
            freecam,
            camera,
            &fresh_gt,
            cursor_pos,
            ray_origin,
            ray_dir,
        )
    } else {
        None
    };

    HoverPickResult {
        hovered_body_id,
        hovered_marker_kind,
        trajectory_hit,
    }
}

/// Raycast against the focused body's trajectory segments.
/// Returns the closest hit within the pick radius threshold.
fn pick_trajectory_segment(
    focused_body_state: &FocusedBodyState,
    bodies: &Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: &System,
    trajectory_caches: &Query<(&crate::presentation::TrajectoryMesh, &crate::presentation::TrajectoryCache)>,
    sim_time: &SimTime,
    view_settings: &ViewSettings,
    freecam: &Freecam,
    camera: &Camera,
    camera_global_transform: &GlobalTransform,
    cursor_pos: Vec2,
    ray_origin: Vec3,
    ray_dir: Vec3,
) -> Option<TrajectoryHitData> {
    let focused_id = focused_body_state.current_body_id.as_ref()?;

    // Find the focused body and its current motive selection.
    let focused_index = system.by_name(focused_id)?;
    let focused_body = system.id(focused_index);
    let focused_motive = system.motive(focused_index);
    let focused_entity = trajectory_caches
        .iter()
        .find_map(|(traj_mesh, cache)| {
            let Ok((_, body_ref, _)) = bodies.get(traj_mesh.body_entity) else {
                return None;
            };
            if body_ref.0 == focused_body && cache.valid && !cache.local_points.is_empty() {
                Some(traj_mesh.body_entity)
            } else {
                None
            }
        })?;

    // Find the trajectory cache for the focused body
    let cache = trajectory_caches
        .iter()
        .find_map(|(traj_mesh, cache)| {
            if traj_mesh.body_entity != focused_entity {
                return None;
            }
            if cache.valid && !cache.local_points.is_empty() {
                Some(cache)
            } else {
                None
            }
        })?;
    
    // Get primary position offset for Keplerian orbits
    let primary_offset: DVec3 = cache
        .primary_id
        .as_ref()
        .and_then(|pid| system.by_name(pid))
        .map(|i| system.position(i))
        .unwrap_or(DVec3::ZERO);

    // Rotate cached points by current perifocal->reference delta so picking
    // tracks the render-time trajectory transform.
    let current_perifocal_to_reference = match focused_motive.motive_at(sim_time.time) {
        (_, MotiveSelection::Keplerian(k)) => Some(DQuat::from_mat3(&k.perifocal_to_reference_matrix(sim_time.time))),
        _ => None,
    };
    let rotation_delta = if let Some(current_rot) = current_perifocal_to_reference {
        let base_rot = cache.base_perifocal_to_reference.unwrap_or(current_rot);
        current_rot * base_rot.inverse()
    } else {
        DQuat::IDENTITY
    };
    
    let distance_scale = view_settings.distance_factor();
    let camera_pos = freecam.bevy_pos;
    let ray_origin_d = DVec3::new(ray_origin.x as f64, ray_origin.y as f64, ray_origin.z as f64);
    let ray_dir_d = DVec3::new(ray_dir.x as f64, ray_dir.y as f64, ray_dir.z as f64).normalize_or_zero();
    
    // (pixel_distance, idx, seg_t, time_a, time_b, local_pos, bevy_pos)
    let mut best_hit: Option<(f32, usize, f64, f64, f64, DVec3, Vec3)> = None;
    
    // Use only cached trajectory points for picking (exclude transient points).
    let points = &cache.local_points;
    for i in 0..points.len().saturating_sub(1) {
        let (time_a, local_a) = points[i];
        let (time_b, local_b) = points[i + 1];
        
        // Transform to bevy space
        let world_a = rotation_delta * local_a + primary_offset;
        let world_b = rotation_delta * local_b + primary_offset;
        let bevy_a = world_a.to_render_relative(distance_scale, camera_pos);
        let bevy_b = world_b.to_render_relative(distance_scale, camera_pos);
        let seg_a_d = DVec3::new(bevy_a.x as f64, bevy_a.y as f64, bevy_a.z as f64);
        let seg_b_d = DVec3::new(bevy_b.x as f64, bevy_b.y as f64, bevy_b.z as f64);
        let ray_hit = ray_segment_closest_point(ray_origin_d, ray_dir_d, seg_a_d, seg_b_d);
        
        let Ok(screen_a) = camera.world_to_viewport(camera_global_transform, bevy_a) else {
            continue;
        };
        let Ok(screen_b) = camera.world_to_viewport(camera_global_transform, bevy_b) else {
            continue;
        };
        let (seg_t_screen, screen_dist) = closest_point_on_screen_segment(cursor_pos, screen_a, screen_b);
        let seg_t = ray_hit.map(|(t, _)| t).unwrap_or(seg_t_screen);

        if screen_dist <= TRAJECTORY_PICK_PIXEL_RADIUS {
            if best_hit.is_none() || screen_dist < best_hit.as_ref().unwrap().0 {
                // Interpolate local position
                let local_hit = (rotation_delta * local_a).lerp(rotation_delta * local_b, seg_t);
                let bevy_hit = bevy_a.lerp(bevy_b, seg_t as f32);
                best_hit = Some((screen_dist, i, seg_t, time_a, time_b, local_hit, bevy_hit));
            }
        }
    }
    
    // Handle closed orbit: check segment from last point back to first
    if cache.closed && points.len() >= 2 {
        let (time_a, local_a) = points[points.len() - 1];
        let (time_b, local_b) = points[0];
        
        let world_a = rotation_delta * local_a + primary_offset;
        let world_b = rotation_delta * local_b + primary_offset;
        let bevy_a = world_a.to_render_relative(distance_scale, camera_pos);
        let bevy_b = world_b.to_render_relative(distance_scale, camera_pos);
        let seg_a_d = DVec3::new(bevy_a.x as f64, bevy_a.y as f64, bevy_a.z as f64);
        let seg_b_d = DVec3::new(bevy_b.x as f64, bevy_b.y as f64, bevy_b.z as f64);
        let ray_hit = ray_segment_closest_point(ray_origin_d, ray_dir_d, seg_a_d, seg_b_d);
        
        if let (Ok(screen_a), Ok(screen_b)) = (
            camera.world_to_viewport(camera_global_transform, bevy_a),
            camera.world_to_viewport(camera_global_transform, bevy_b),
        ) {
            let (seg_t_screen, screen_dist) = closest_point_on_screen_segment(cursor_pos, screen_a, screen_b);
            let seg_t = ray_hit.map(|(t, _)| t).unwrap_or(seg_t_screen);

            if screen_dist <= TRAJECTORY_PICK_PIXEL_RADIUS {
                if best_hit.is_none() || screen_dist < best_hit.as_ref().unwrap().0 {
                    let local_hit = (rotation_delta * local_a).lerp(rotation_delta * local_b, seg_t);
                    let bevy_hit = bevy_a.lerp(bevy_b, seg_t as f32);
                    // For wrapped segment, time_b is actually the start of the period (0)
                    // but we want the logical continuation, so add period if needed
                    let adjusted_time_b = if let Some(interval_size) = cache.interval_size {
                        time_b + interval_size
                    } else {
                        time_b
                    };
                    best_hit = Some((screen_dist, points.len() - 1, seg_t, time_a, adjusted_time_b, local_hit, bevy_hit));
                }
            }
        }
    }
    
    best_hit.map(|(_, idx, seg_t, time_a, time_b, local_pos, bevy_pos)| {
        TrajectoryHitData {
            segment_start_idx: idx,
            start_time: time_a,
            end_time: time_b,
            t: seg_t,
            hit_position_local: local_pos,
            hit_position_bevy: bevy_pos,
        }
    })
}

/// Find the closest point on a 2D segment to a 2D point.
/// Returns (t, distance) where t is in [0, 1].
fn closest_point_on_screen_segment(point: Vec2, seg_a: Vec2, seg_b: Vec2) -> (f64, f32) {
    let seg = seg_b - seg_a;
    let seg_len_sq = seg.length_squared();
    if seg_len_sq <= f32::EPSILON {
        return (0.0, point.distance(seg_a));
    }

    let t = ((point - seg_a).dot(seg) / seg_len_sq).clamp(0.0, 1.0);
    let closest = seg_a + seg * t;
    (t as f64, point.distance(closest))
}

/// Closest distance between a ray and a segment.
/// Returns (segment_t, distance) with segment_t in [0, 1].
fn ray_segment_closest_point(
    ray_origin: DVec3,
    ray_dir: DVec3,
    seg_a: DVec3,
    seg_b: DVec3,
) -> Option<(f64, f64)> {
    let seg_dir = seg_b - seg_a;
    let seg_len_sq = seg_dir.length_squared();
    if seg_len_sq <= f64::EPSILON || ray_dir.length_squared() <= f64::EPSILON {
        return None;
    }

    let w0 = ray_origin - seg_a;
    let a = ray_dir.dot(ray_dir);
    let b = ray_dir.dot(seg_dir);
    let c = seg_dir.dot(seg_dir);
    let d = ray_dir.dot(w0);
    let e = seg_dir.dot(w0);
    let denom = a * c - b * b;

    let mut seg_t = if denom.abs() < 1e-12 {
        (e / c).clamp(0.0, 1.0)
    } else {
        ((a * e - b * d) / denom).clamp(0.0, 1.0)
    };
    if !seg_t.is_finite() {
        seg_t = 0.0;
    }
    let closest_on_seg = seg_a + seg_dir * seg_t;
    let ray_t = (closest_on_seg - ray_origin).dot(ray_dir).max(0.0);
    let closest_on_ray = ray_origin + ray_dir * ray_t;
    Some((seg_t, (closest_on_seg - closest_on_ray).length()))
}

fn ray_hits_sphere(ray_origin: Vec3, ray_dir: Vec3, center: Vec3, radius: f32) -> bool {
    let oc = ray_origin - center;
    let b = oc.dot(ray_dir);
    let c = oc.dot(oc) - radius * radius;
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        return false;
    }
    let t2 = -b + discriminant.sqrt();
    t2 >= 0.0
}

fn revolve_around(
    settings: Res<MovementSettings>,
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    mut mouse: MessageReader<MouseMotion>,
    mut scroll: MessageReader<MouseWheel>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut primary_window: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    view_settings: Res<ViewSettings>,
    entities: Query<(Entity, &BodyRef, &Transform), Without<Freecam>>,
    system: Res<SimSystem>,
    mut egui_ctx: EguiContexts,
    sim_time: Res<SimTime>,
) {
    if let Ok((window, mut cursor_options)) = primary_window.single_mut() {
        for (mut cam_t, mut pcam, mut fcam) in camera.iter_mut() {

            match &mut pcam.action {
                CameraAction::RevolveAround(revolve) => {

                    let resolved = entities.get(revolve.entity).ok()
                        .and_then(|(e, r, t)| system.0.index_of(r.0).map(|i| (e, i, t)));
                    match resolved {
                        Some((entity, index, transform)) => {
                            let appearance = system.0.appearance(index);
                            let motive = system.0.motive(index);
                            let frame = resolve_frame_or_fallback(revolve.frame, motive, sim_time.time, entity);
                            if frame != revolve.frame {
                                revolve.frame = frame;
                            }
                            let window_scale = window.height().min(window.width());
                            let scaled_radius = if matches!(appearance, Appearance::Empty) {
                                view_settings.distance_factor()
                            } else {
                                view_settings.body_scale_factor(appearance.radius()) as f64
                            };

                            let egui_wants_pointer = egui_ctx.ctx_mut()
                                .map_or(false, |ctx| ctx.wants_pointer_input());

                            let min_distance = ORBIT_ZOOM_MIN_RADIUS_MULT * scaled_radius;
                            let max_distance = ORBIT_ZOOM_MAX_LY * LIGHT_YEAR_M * view_settings.distance_factor();
                            let mut view_changed = false;

                            for ev in scroll.read() {
                                if !egui_wants_pointer {
                                    let prev_distance = revolve.bevy_distance;
                                    let u = (revolve.bevy_distance / min_distance).ln();
                                    let a = ORBIT_ZOOM_BASE_SPEED;
                                    let k = ORBIT_ZOOM_ACCELERATION / 10.0_f64.ln();
                                    let t = ev.y as f64;

                                    let u_new = if k.abs() > f64::EPSILON {
                                        (u + a / k) * (-k * t).exp() - a / k
                                    } else {
                                        u - a * t
                                    };

                                    revolve.bevy_distance = min_distance * u_new.exp();
                                    revolve.bevy_distance = revolve.bevy_distance
                                        .clamp(min_distance, max_distance);
                                    if (revolve.bevy_distance - prev_distance).abs() > f64::EPSILON {
                                        view_changed = true;
                                    }
                                }
                            }

                            if mouse_buttons.pressed(MouseButton::Right) {
                                if egui_wants_pointer {
                                    cursor_options.grab_mode = CursorGrabMode::None;
                                    cursor_options.visible = true;
                                } else {
                                    cursor_options.grab_mode = CursorGrabMode::Confined;
                                    cursor_options.visible = false;
                                    for ev in mouse.read() {
                                        let prev_altitude = revolve.altitude;
                                        let prev_azimuth = revolve.azimuth;
                                        revolve.azimuth -= (ev.delta.x.clamp(-1000.0, 1000.0) * window_scale * settings.sensitivity) as f64;
                                        revolve.azimuth = revolve.azimuth.rem_euclid(TAU);
                                        revolve.altitude += (ev.delta.y.clamp(-1000.0, 1000.0) * window_scale * settings.sensitivity) as f64;
                                        const ALT_LIMIT: f64 = PI / 2.0 - 0.001;
                                        revolve.altitude = revolve.altitude.clamp(-ALT_LIMIT, ALT_LIMIT);
                                        if (revolve.altitude - prev_altitude).abs() > f64::EPSILON
                                            || (revolve.azimuth - prev_azimuth).abs() > f64::EPSILON
                                        {
                                            view_changed = true;
                                        }
                                    }
                                }
                            } else {
                                cursor_options.grab_mode = CursorGrabMode::None;
                                cursor_options.visible = true;
                            }

                            let body_pos_in_bevy = system.0.position(index).to_render_scaled(view_settings.distance_factor());
                            let (offset, _) = offset_in_frame(
                                revolve.frame,
                                revolve.altitude,
                                revolve.azimuth,
                                revolve.bevy_distance,
                                motive,
                                sim_time.time,
                                transform,
                                entity,
                            );
                            let camera_pos_in_bevy = body_pos_in_bevy + offset;

                            fcam.bevy_pos = camera_pos_in_bevy;
                            if offset.is_finite() && body_pos_in_bevy.is_finite() && body_pos_in_bevy != camera_pos_in_bevy {
                                let up = up_vector_for_frame(revolve.frame, motive, sim_time.time, transform, entity);
                                let look_at_rot = look_at(fcam.bevy_pos, body_pos_in_bevy, up);
                                cam_t.rotation = look_at_rot.as_quat();
                                if view_changed {
                                }
                            }
                        }
                        None => {
                            pcam.action = CameraAction::Free;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Find the largest semi-major axis among bodies whose current Keplerian motive
/// has `target_id` as its primary. Returns 0.0 if no children are found.
fn find_max_child_sma(
    target_id: &str,
    system: &System,
    time: crate::foundations::time::Instant,
) -> f64 {
    system.indices()
        .filter_map(|i| {
            let (_, ms) = system.motive(i).motive_at(time);
            if let MotiveSelection::Keplerian(k) = ms {
                if k.primary_id == target_id {
                    return Some(k.semi_major_axis());
                }
            }
            None
        })
        .fold(0.0f64, f64::max)
}

fn local_to_object_in_basis(altitude: f64, azimuth: f64, bevy_distance: f64) -> DVec3 {
    let cos_alt = altitude.cos();
    let x = bevy_distance * cos_alt * azimuth.sin();
    let z = bevy_distance * cos_alt * azimuth.cos();
    let y = bevy_distance * altitude.sin();
    DVec3::new(x, y, z)
}

fn offset_in_frame(
    requested_frame: RevolveAroundFrame,
    altitude: f64,
    azimuth: f64,
    bevy_distance: f64,
    motive: &Motive,
    sim_time: crate::foundations::time::Instant,
    transform: &Transform,
    entity: Entity,
) -> (DVec3, RevolveAroundFrame) {
    let (basis, frame) = frame_basis(requested_frame, motive, sim_time, transform, entity);
    let local = local_to_object_in_basis(altitude, azimuth, bevy_distance);
    (basis * local, frame)
}

fn alt_az_in_frame(
    observer: DVec3,
    observed: DVec3,
    requested_frame: RevolveAroundFrame,
    motive: &Motive,
    sim_time: crate::foundations::time::Instant,
    transform: &Transform,
    entity: Entity,
) -> (f64, f64) {
    let (basis, _) = frame_basis(requested_frame, motive, sim_time, transform, entity);
    let diff = observed - observer;
    let local = basis.transpose() * diff;
    let r = local.length();
    if r <= f64::EPSILON {
        return (0.0, 0.0);
    }
    let altitude = (local.y / r).asin();
    let azimuth = local.x.atan2(local.z).rem_euclid(TAU);
    (altitude, azimuth)
}

fn resolve_frame_or_fallback(
    requested_frame: RevolveAroundFrame,
    motive: &Motive,
    sim_time: crate::foundations::time::Instant,
    entity: Entity,
) -> RevolveAroundFrame {
    let (_, selection) = motive.motive_at(sim_time);
    match (requested_frame, selection) {
        (RevolveAroundFrame::Perifocal, MotiveSelection::Keplerian(_)) => requested_frame,
        (RevolveAroundFrame::Perifocal, _) => {
            info!(
                "cam.frame.fallback entity={:?} from={:?} to={:?} reason=non_kepler",
                entity,
                requested_frame,
                RevolveAroundFrame::Global
            );
            RevolveAroundFrame::Global
        }
        _ => requested_frame,
    }
}

fn frame_basis(
    requested_frame: RevolveAroundFrame,
    motive: &Motive,
    sim_time: crate::foundations::time::Instant,
    transform: &Transform,
    entity: Entity,
) -> (DMat3, RevolveAroundFrame) {
    let frame = resolve_frame_or_fallback(requested_frame, motive, sim_time, entity);
    match frame {
        RevolveAroundFrame::Global => (DMat3::IDENTITY, frame),
        RevolveAroundFrame::Local => {
            let rotation = transform.rotation.as_dquat();
            let x = rotation * DVec3::X;
            let y = rotation * DVec3::Y;
            let z = rotation * DVec3::Z;
            (DMat3::from_cols(x, y, z), frame)
        }
        RevolveAroundFrame::Perifocal => {
            let (_, selection) = motive.motive_at(sim_time);
            let MotiveSelection::Keplerian(k) = selection else {
                return (DMat3::IDENTITY, RevolveAroundFrame::Global);
            };

            let rot_arg_peri = DMat3::from_rotation_z(k.argument_of_periapsis(sim_time).to_radians());
            let rot_inc = DMat3::from_rotation_x(k.inclination().to_radians());
            let rot_long_asc_node = DMat3::from_rotation_z(k.longitude_of_ascending_node_infallible(sim_time).to_radians());
            let sim_rot = rot_long_asc_node * rot_inc * rot_arg_peri;

            // Recompute periapsis and orbital normal every frame from the active Kepler motive.
            // This keeps the perifocal frame live for precessing or transitioning orbits.
            let p_sim = k.periapsis_vec(sim_time).normalize();
            let w_sim = (sim_rot * DVec3::Z).normalize();

            let p = p_sim.to_render_scaled(1.0).normalize();
            let mut w = w_sim.to_render_scaled(1.0).normalize();
            let mut q = w.cross(p);
            if q.length_squared() <= f64::EPSILON {
                return (DMat3::IDENTITY, RevolveAroundFrame::Global);
            }
            q = q.normalize();
            w = p.cross(q).normalize();

            // Camera spherical coordinates use local +Y as altitude axis and +Z as azimuth zero.
            // In perifocal mode we map +Z -> +P, +Y -> +W, +X -> +Q.
            (DMat3::from_cols(q, w, p), frame)
        }
    }
}

fn up_vector_for_frame(
    requested_frame: RevolveAroundFrame,
    motive: &Motive,
    sim_time: crate::foundations::time::Instant,
    transform: &Transform,
    entity: Entity,
) -> DVec3 {
    let (basis, _) = frame_basis(requested_frame, motive, sim_time, transform, entity);
    let up = basis.col(1);
    if up.is_finite() && up.length_squared() > f64::EPSILON {
        up.normalize()
    } else {
        DVec3::Y
    }
}

fn evaluate_goto_position(
    goto: &GoToInProgress,
    at_time: f64,
    bodies: &Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: &System,
    view_settings: &ViewSettings,
    sim_time: &SimTime,
) -> Option<DVec3> {
    let (entity, body_ref, transform) = bodies.get(goto.entity).ok()?;
    let index = system.index_of(body_ref.0)?;
    let motive = system.motive(index);
    let body_pos = system.position(index).to_render_scaled(view_settings.distance_factor());
    let start_pos = match &goto.origin {
        GoToOrigin::Position { position, velocity, sample_time } => {
            let dt = (at_time - *sample_time).max(0.0);
            *position + *velocity * dt
        }
        GoToOrigin::Revolving(revolving) => {
            let (origin_entity, origin_ref, origin_transform) = bodies.get(revolving.entity).ok()?;
            let origin_index = system.index_of(origin_ref.0)?;
            let origin_motive = system.motive(origin_index);
            let origin_pos = system.position(origin_index).to_render_scaled(view_settings.distance_factor());
            let (origin_offset, _) = offset_in_frame(
                revolving.frame,
                revolving.altitude,
                revolving.azimuth,
                revolving.bevy_distance,
                origin_motive,
                sim_time.time,
                origin_transform,
                origin_entity,
            );
            origin_pos + origin_offset
        }
    };

    let (offset, frame) = offset_in_frame(
        goto.end_frame,
        goto.end_altitude,
        goto.end_azimuth,
        goto.end_distance,
        motive,
        sim_time.time,
        transform,
        entity,
    );
    let final_pos = body_pos + offset;
    let frac = f64::min(1.0, (at_time - goto.start_time) / 2.0);
    let frac = ease::f64::circ(frac);
    debug!(
        "cam.goto.predict entity={:?} frame={:?} t={} frac={}",
        goto.entity,
        frame,
        at_time,
        frac
    );
    Some(start_pos.lerp(final_pos, frac))
}

fn estimate_goto_velocity(
    goto: &GoToInProgress,
    now: f64,
    bodies: &Query<(Entity, &BodyRef, &Transform), Without<PlanetariumCamera>>,
    system: &System,
    view_settings: &ViewSettings,
    sim_time: &SimTime,
) -> Option<DVec3> {
    let sample_dt = 1.0 / 60.0;
    let p0 = evaluate_goto_position(
        goto,
        now,
        bodies,
        system,
        view_settings,
        sim_time,
    )?;
    let p1 = evaluate_goto_position(
        goto,
        now + sample_dt,
        bodies,
        system,
        view_settings,
        sim_time,
    )?;
    Some((p1 - p0) / sample_dt)
}

#[allow(dead_code)] // The frame-less counterpart to `alt_az_in_frame`, kept for debugging.
fn alt_az_in_bevy(observer: DVec3, observed: DVec3) -> (f64, f64) {
    let diff = observed - observer;
    let r = diff.length();
    if r <= f64::EPSILON {
        return (0.0, 0.0);
    }
    let altitude = (diff.y / r).asin();
    let azimuth = diff.x.atan2(diff.z).rem_euclid(TAU);
    (altitude, azimuth)
}

fn look_at(from: DVec3, to: DVec3, up: DVec3) -> DQuat {
    // Bevy cameras look down local -Z, so world +Z basis must point backward.
    let forward = (to - from).normalize();
    let back = -forward;
    let right = up.cross(back).normalize();
    let up = back.cross(right);

    let rot_matrix = DMat3::from_cols(right, up, back);

    DQuat::from_mat3(&rot_matrix)
}
