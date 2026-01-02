use std::f64::consts::{PI, TAU};
use bevy::app::App;
use bevy::input::mouse::MouseMotion;
use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::EguiContexts;
use num_traits::Float;
use crate::body::appearance::Appearance;
use crate::body::motive::info::BodyState;
use crate::body::motive::newton_motive;
use crate::body::universe::save::ViewSettings;
use crate::gui::app::AppState;
use crate::gui::planetarium::position_bodies;
use crate::gui::util::freecam::{FreeCamPlugin, Freecam, MovementSettings};
use crate::util::bevystuff::GlamVec;
use crate::util::ease;

pub struct PlanetariumCameraPlugin;

impl Plugin for PlanetariumCameraPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_plugins(FreeCamPlugin)
            .add_message::<GoTo>()
            .add_systems(Update, (
                handle_gotos,
                run_goto,
                // Camera position changes must happen *before* bodies are rendered
                // to avoid jerking, because their rendered positions are relative to the camera,
                // but after all bodies have moved in the sim if the camera is located relative
                // to a simulated body.
                revolve_around.before(position_bodies),
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

pub enum CameraAction {
    Free,
    Goto(GoToInProgress),
    RevolveAround(RevolveAround),
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
}

pub struct GoToInProgress {
    start_pos: DVec3,
    start_rot: Quat,
    start_time: f64,
    end_distance: f64,
    end_altitude: f64,
    end_azimuth: f64,
    entity: Entity,
}

pub struct RevolveAround {
    entity: Entity,
    bevy_distance: f64,
    altitude: f64,
    azimuth: f64,
}

fn handle_gotos (
    mut go_tos: MessageReader<GoTo>,
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    bodies: Query<(Entity, &BodyState, &Appearance), Without<PlanetariumCamera>>,
    view_settings: Res<ViewSettings>,
    time: Res<Time>,
) {
    if let Ok((mut cam_t, mut pcam, mut fcam)) = camera.single_mut() {
        for event in go_tos.read() {
            let start_pos = fcam.bevy_pos;
            let start_rot = cam_t.rotation;

            let (entity, state, appearance) = bodies.get(event.entity).unwrap();
            let obj_pos = state.current_position;
            
            // Then, move to the nearby distance from the object
            // Calculate the direction from object to camera (opposite of look direction)
            // info!("Radius: {}", appearance.radius(), view_settings.body_scale_factor(appearance.radius()));
            let nearby_distance = 3f64 * view_settings.body_scale_factor(appearance.radius()) as f64;
            let (altitude, azimuth) = alt_az_in_bevy(obj_pos.as_bevy_scaled_dvec(view_settings.distance_factor()), fcam.bevy_pos);

            pcam.action = CameraAction::Goto(GoToInProgress {
                start_pos,
                start_rot,
                start_time: time.elapsed().as_secs_f64(),
                entity,
                end_distance: nearby_distance,
                end_altitude: altitude,
                end_azimuth: azimuth,
            });
        }
    }
}

fn run_goto (
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    bodies: Query<&BodyState, Without<PlanetariumCamera>>,
    time: Res<Time>,
    view_settings: Res<ViewSettings>,
) {
    let animation_time = 2.0;
    let now = time.elapsed().as_secs_f64();
    let mut next_action = None;

    if let Ok((mut cam_t, mut pcam, mut fcam)) = camera.single_mut() {
        match &mut pcam.action {
            CameraAction::Goto(goto) => {
                if let Ok(body_state) = bodies.get(goto.entity) {
                    // How far are we in the go-to travel?
                    let frac = f64::min(1.0, (now - goto.start_time) / animation_time);
                    let frac = ease::f64::circ(frac);

                    // get current position
                    let body_pos_in_bevy = body_state.current_position.as_bevy_scaled_dvec(view_settings.distance_factor());

                    // Set new end position based on object's current location
                    let offset = local_to_object_in_bevy(goto.end_altitude, goto.end_azimuth, goto.end_distance);
                    let final_pos = body_pos_in_bevy + offset;

                    // Set new target rotation based on where the body is now.
                    let look_at_rot = look_at(body_pos_in_bevy, final_pos, DVec3::Y);

                    // Lerp between where we started and the current target position
                    let mid_pos = goto.start_pos.lerp(final_pos, frac);
                    let mid_rot = goto.start_rot.slerp(look_at_rot.as_quat(), frac as f32);
                    fcam.bevy_pos = mid_pos;
                    cam_t.rotation = mid_rot;

                    // Transition control back to user
                    if (frac - 1.0).abs() <= f64::epsilon() {
                        next_action = Some(CameraAction::RevolveAround(RevolveAround {
                            entity: goto.entity,
                            bevy_distance: goto.end_distance,
                            altitude: goto.end_altitude,
                            azimuth: goto.end_azimuth,
                        }));
                    }
                } else {
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
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut primary_window: Query<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    view_settings: Res<ViewSettings>,
    entities: Query<(Entity, &BodyState, &Transform), Without<Freecam>>,
    mut egui_ctx: EguiContexts,
) {
    if let Ok((mut window, mut cursor_options)) = primary_window.single_mut() {
        for (mut cam_t, mut pcam, mut fcam) in camera.iter_mut() {

            match &mut pcam.action {
                CameraAction::RevolveAround(revolve) => {

                    match entities.get(revolve.entity) {
                        Ok((entity, state, transform)) => {
                            let window_scale = window.height().min(window.width());

                            if mouse_buttons.pressed(MouseButton::Left) {
                                if let Ok(ctx) = egui_ctx.ctx_mut() && ctx.wants_pointer_input() && ctx.wants_pointer_input() {
                                    // If hovering over an egui window, don't rotate around! It grabs the mouse :(
                                    cursor_options.grab_mode = CursorGrabMode::None;
                                    cursor_options.visible = true;
                                } else {
                                    cursor_options.grab_mode = CursorGrabMode::Confined;
                                    cursor_options.visible = false;
                                    for ev in mouse.read() {
                                        revolve.azimuth -= (ev.delta.x.clamp(-1000.0, 1000.0) * window_scale * settings.sensitivity) as f64;
                                        revolve.azimuth = revolve.azimuth.rem_euclid(TAU);
                                        revolve.altitude += (ev.delta.y.clamp(-1000.0, 1000.0) * window_scale * settings.sensitivity) as f64;
                                        const ALT_LIMIT: f64 = PI / 2.0 - 0.001; // ~0.057° margin
                                        revolve.altitude = revolve.altitude.clamp(-ALT_LIMIT, ALT_LIMIT);
                                    }
                                }
                            } else {
                                cursor_options.grab_mode = CursorGrabMode::None;
                                cursor_options.visible = true;
                            }

                            let body_pos_in_bevy = state.current_position.as_bevy_scaled_dvec(view_settings.distance_factor());
                            let offset = local_to_object_in_bevy(revolve.altitude, revolve.azimuth, revolve.bevy_distance);
                            let camera_pos_in_bevy = body_pos_in_bevy + offset;

                            fcam.bevy_pos = camera_pos_in_bevy;
                            if offset.is_finite() && body_pos_in_bevy.is_finite() && body_pos_in_bevy != camera_pos_in_bevy { // Guard against degenerate zero-length looking vectors
                                let look_at_rot = look_at(body_pos_in_bevy, fcam.bevy_pos, DVec3::Y);
                                cam_t.rotation = look_at_rot.as_quat();
                            }
                        }
                        Err(_) => {
                            pcam.action = CameraAction::Free;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn local_to_object_in_bevy(altitude: f64, azimuth: f64, bevy_distance: f64) -> DVec3 {
    let cos_alt = altitude.cos();
    let x = bevy_distance * cos_alt * azimuth.sin();
    let z = bevy_distance * cos_alt * azimuth.cos();
    let y = bevy_distance * altitude.sin();
    DVec3::new(x, y, z)
}

fn alt_az_in_bevy(observer: DVec3, observed: DVec3) -> (f64, f64) {
    let diff = observed - observer;
    let r = diff.length();
    let altitude = (diff.y / r).asin();
    let azimuth = diff.x.atan2(diff.z).rem_euclid(TAU);
    (altitude, azimuth)
}

fn look_at(from: DVec3, to: DVec3, up: DVec3) -> DQuat {
    let forward = (to - from).normalize();
    let right = up.cross(forward).normalize();
    let up = forward.cross(right);

    let rot_matrix = DMat3::from_cols(right, up, forward);

    DQuat::from_mat3(&rot_matrix)
}
