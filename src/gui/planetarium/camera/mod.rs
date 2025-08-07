use std::f64::consts::{PI, TAU};
use bevy::app::App;
use bevy::ecs::query::QueryEntityError;
use bevy::input::mouse::MouseMotion;
use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, PrimaryWindow};
use num_traits::Float;
use crate::body::appearance::Appearance;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::universe::save::ViewSettings;
use crate::gui::app::AppState;
use crate::gui::util::freecam::{Freecam, FreeCam, MovementSettings};
use crate::util::bevystuff::GlamVec;

pub struct PlanetariumCameraPlugin;

impl Plugin for PlanetariumCameraPlugin {
    fn build(&self, app: &mut App) {
        app
            .add_plugins(FreeCam)
            .add_event::<GoTo>()
            .add_systems(Update, (
                handle_gotos,
                run_goto,
                revolve_around,
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

#[derive(Event)]
pub struct GoTo {
    pub entity: Entity,
}

pub struct GoToInProgress {
    start_pos: DVec3,
    end_pos: DVec3,
    start_rot: Quat,
    end_rot: Quat,
    start_time: f64,
    entity: Entity,
}

pub struct RevolveAround {
    entity: Entity,
    bevy_distance: f64,
    altitude: f64,
    azimuth: f64,
}

fn handle_gotos (
    mut go_tos: EventReader<GoTo>,
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    bodies: Query<(Entity, &BodyState, &Appearance), Without<PlanetariumCamera>>,
    view_settings: Res<ViewSettings>,
    time: Res<Time>,
) {
    if let Ok((mut cam_t, mut pcam, mut freecam)) = camera.single_mut() {
        for event in go_tos.read() {
            let start_pos = freecam.bevy_pos;
            let start_rot = cam_t.rotation;

            let (entity, state, appearance) = bodies.get(event.entity).unwrap();
            let obj_pos = state.current_position;
            
            // Then, move to the nearby distance from the object
            // Calculate the direction from object to camera (opposite of look direction)
            let camera_to_obj = (obj_pos - start_pos).normalize();
            let nearby_distance = appearance.nearby();
            let end_pos = obj_pos - (camera_to_obj * nearby_distance);

            let end_pos = end_pos.as_bevy_scaled_dvec(view_settings.distance_scale);
            let look_at_rot = look_at(obj_pos.as_bevy_scaled_dvec(view_settings.distance_scale), freecam.bevy_pos, DVec3::Y);
            let end_rot = look_at_rot.as_quat();

            pcam.action = CameraAction::Goto(GoToInProgress {
                start_pos,
                end_pos,
                start_rot,
                end_rot,
                start_time: time.elapsed().as_secs_f64(),
                entity,
            });
        }
    }
}

fn run_goto (
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    bodies: Query<(Entity, &BodyState), Without<PlanetariumCamera>>,
    time: Res<Time>,
    view_settings: Res<ViewSettings>,
) {
    let animation_time = 2.0;
    let now = time.elapsed().as_secs_f64();
    let mut next_action = None;

    if let Ok((mut cam_t, mut pcam, mut freecam)) = camera.single_mut() {
        match &mut pcam.action {
            CameraAction::Goto(goto) => {
                let frac = f64::min(1.0, (now - goto.start_time) / animation_time);

                let mid_pos = goto.start_pos.lerp(goto.end_pos, frac);
                let mis_rot = goto.start_rot.slerp(goto.end_rot, frac as f32);
                freecam.bevy_pos = mid_pos;
                cam_t.rotation = mis_rot;
                if (frac - 1.0).abs() <= f64::epsilon() {
                    // Ensure that the final position is correct.
                    freecam.bevy_pos = goto.end_pos;
                    cam_t.rotation = goto.end_rot;

                    if let Ok((_, body_state)) = bodies.get(goto.entity) {
                        let bevy_distance = (body_state.current_position.as_bevy_scaled_dvec(view_settings.distance_scale)).distance(freecam.bevy_pos);
                        next_action = Some(CameraAction::RevolveAround(RevolveAround {
                            entity: goto.entity,
                            bevy_distance,
                            altitude: 0.0,
                            azimuth: 0.0,
                        }));
                    } else {
                        next_action = Some(CameraAction::Free);
                    }
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
    mut mouse: EventReader<MouseMotion>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut primary_window: Query<(&mut Window), With<PrimaryWindow>>,
    view_settings: Res<ViewSettings>,
    entities: Query<(Entity, &BodyState, &Transform), Without<Freecam>>,
) {
    if let Ok(mut window) = primary_window.single_mut() {
        for (mut cam_t, mut pcam, mut fcam) in camera.iter_mut() {
            for ev in mouse.read() {
                match &mut pcam.action {
                    CameraAction::RevolveAround(revolve) => {
                        if mouse_buttons.pressed(MouseButton::Left) {
                            window.cursor_options.grab_mode = CursorGrabMode::Confined;
                            window.cursor_options.visible = false;
                            match entities.get(revolve.entity) {
                                Ok((entity, state, transform)) => {
                                    let window_scale = window.height().min(window.width());

                                    revolve.azimuth -= (ev.delta.x.clamp(-1000.0, 1000.0) * window_scale * settings.sensitivity) as f64;
                                    revolve.azimuth = revolve.azimuth.rem_euclid(TAU);
                                    revolve.altitude += (ev.delta.y.clamp(-1000.0, 1000.0) * window_scale * settings.sensitivity) as f64;
                                    const ALT_LIMIT: f64 = PI / 2.0 - 0.001; // ~0.057° margin
                                    revolve.altitude = revolve.altitude.clamp(-ALT_LIMIT, ALT_LIMIT);

                                    let bevy_center = state.current_position.as_bevy_scaled_dvec(view_settings.distance_scale);
                                    let cos_alt = revolve.altitude.cos();
                                    let x = revolve.bevy_distance * cos_alt * revolve.azimuth.sin();
                                    let z = revolve.bevy_distance * cos_alt * revolve.azimuth.cos();
                                    let y = revolve.bevy_distance * revolve.altitude.sin();
                                    let offset = DVec3::new(x, y, z);

                                    if offset.is_finite() {
                                        let bevy_pos = (bevy_center + offset);

                                        fcam.bevy_pos = bevy_pos;
                                        if bevy_center.is_finite() && bevy_center != bevy_pos { // Guard against degenerate zero-length looking vectors
                                            let look_at_rot = look_at(bevy_center, fcam.bevy_pos, DVec3::Y);
                                            cam_t.rotation = look_at_rot.as_quat();
                                        }
                                    }
                                }
                                Err(_) => {
                                    pcam.action = CameraAction::Free;
                                }
                            }
                        } else {
                            window.cursor_options.grab_mode = CursorGrabMode::None;
                            window.cursor_options.visible = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn look_at(from: DVec3, to: DVec3, up: DVec3) -> DQuat {
    let forward = (to - from).normalize();
    let right = up.cross(forward).normalize();
    let up = forward.cross(right);

    let rot_matrix = DMat3::from_cols(right, up, forward);

    DQuat::from_mat3(&rot_matrix)
}
