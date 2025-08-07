use bevy::app::App;
use bevy::input::keyboard::Key::DVR;
use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use num_traits::Float;
use url::Position;
use crate::body::appearance::Appearance;
use crate::body::motive::info::BodyState;
use crate::body::universe::save::ViewSettings;
use crate::gui::app::AppState;
use crate::gui::util::freecam::{Freecam, FreeCam};
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
}

impl PartialEq for CameraAction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (CameraAction::Free, CameraAction::Free) => true,
            (CameraAction::Goto(_), CameraAction::Goto(_)) => true,
            (_, _) => false,
        }
    }
}

#[derive(Event)]
pub struct GoTo {
    pub entity: Entity,
}

pub struct GoToInProgress {
    progress: f64,
    start_pos: DVec3,
    end_pos: DVec3,
    start_rot: Quat,
    end_rot: Quat,
    start_time: f64,
}

fn handle_gotos (
    mut go_tos: EventReader<GoTo>,
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    bodies: Query<(&BodyState, &Appearance), Without<PlanetariumCamera>>,
    view_settings: Res<ViewSettings>,
    time: Res<Time>,
) {
    if let Ok((mut cam_t, mut pcam, mut freecam)) = camera.single_mut() {
        for event in go_tos.read() {
            let start_pos = freecam.position;
            let start_rot = cam_t.rotation;

            let (state, appearance) = bodies.get(event.entity).unwrap();
            let obj_pos = state.current_position;
            
            // Then, move to the nearby distance from the object
            // Calculate the direction from object to camera (opposite of look direction)
            let camera_to_obj = (obj_pos - start_pos).normalize();
            let nearby_distance = appearance.nearby();
            let end_pos = obj_pos - (camera_to_obj * nearby_distance);

            let end_pos = end_pos.as_bevy_scaled_dvec(view_settings.distance_scale);
            let look_at_rot = look_at(obj_pos.as_bevy_scaled_dvec(view_settings.distance_scale), freecam.position, DVec3::Y);
            let end_rot = look_at_rot.as_quat();
            
            // Update camera position
            // freecam.position = end_pos;
            // cam_t.rotation = end_rot;

            pcam.action = CameraAction::Goto(GoToInProgress {
                progress: 0.0,
                start_pos,
                end_pos,
                start_rot,
                end_rot,
                start_time: time.elapsed().as_secs_f64(),
            });
        }
    }
}

fn run_goto (
    mut camera: Query<(&mut Transform, &mut PlanetariumCamera, &mut Freecam)>,
    view_settings: Res<ViewSettings>,
    time: Res<Time>,
) {
    let animation_time = 2.0;
    let now = time.elapsed().as_secs_f64();
    let mut done = false;

    if let Ok((mut cam_t, mut pcam, mut freecam)) = camera.single_mut() {
        match &mut pcam.action {
            CameraAction::Goto(goto) => {
                let frac = f64::min(1.0, (now - goto.start_time) / animation_time);

                let mid_pos = goto.start_pos.lerp(goto.end_pos, frac);
                let mis_rot = goto.start_rot.slerp(goto.end_rot, frac as f32);
                freecam.position = mid_pos;
                cam_t.rotation = mis_rot;
                if (frac - 1.0).abs() <= f64::epsilon() {
                    // Ensure that the final position is correct.
                    freecam.position = goto.end_pos;
                    cam_t.rotation = goto.end_rot;
                    done = true;
                }
            }
            _ => {}
        }

        if done {
            pcam.action = CameraAction::Free;
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
