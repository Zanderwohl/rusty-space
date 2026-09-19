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
use crate::body::universe::save::ViewSettings;
use crate::gui::app::AppState;
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
                crate::camera::picking::update_hover_target,
                crate::camera::picking::pick_body_on_click,
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
                                .map_or(false, |ctx| ctx.egui_wants_pointer_input());

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
