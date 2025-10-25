use bevy::input::mouse::MouseMotion;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use crate::gui::app::AppState;
use crate::gui::planetarium::camera::CameraAction;
use crate::gui::planetarium::PlanetariumCamera;

/// Mouse sensitivity and movement speed
#[derive(Resource)]
pub struct MovementSettings {
    pub sensitivity: f32,
    pub speed: f32,
}

impl Default for MovementSettings {
    fn default() -> Self {
        Self {
            sensitivity: 0.0000012,
            speed: 12.,
        }
    }
}

/// Key configuration
#[derive(Resource)]
pub struct KeyBindings {
    pub move_forward: KeyCode,
    pub move_backward: KeyCode,
    pub move_left: KeyCode,
    pub move_right: KeyCode,
    pub move_ascend: KeyCode,
    pub move_descend: KeyCode,
    pub toggle_grab_cursor: KeyCode,
}

impl Default for KeyBindings {
    fn default() -> Self {
        Self {
            move_forward: KeyCode::KeyW,
            move_backward: KeyCode::KeyS,
            move_left: KeyCode::KeyA,
            move_right: KeyCode::KeyD,
            move_ascend: KeyCode::Space,
            move_descend: KeyCode::ShiftLeft,
            toggle_grab_cursor: KeyCode::Backquote,
        }
    }
}

/// Used in queries when you want flycams and not other cameras
/// A marker component used in queries when you want flycams and not other cameras
#[derive(Component)]
pub struct Freecam {
    pub bevy_pos: DVec3,
}

/// Grabs/ungrabs mouse cursor
fn toggle_grab_cursor(
    cursor_options: &mut CursorOptions,
    app_state: Res<State<AppState>>,
) {
    if app_state.ne(&AppState::Planetarium) {
        return;
    }
    match cursor_options.grab_mode {
        CursorGrabMode::None => {
            cursor_options.grab_mode = CursorGrabMode::Confined;
            cursor_options.visible = false;
        }
        _ => {
            cursor_options.grab_mode = CursorGrabMode::None;
            cursor_options.visible = true;
        }
    }
}

/// Handles keyboard input and movement
fn player_move(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    cursor_options: Query<&CursorOptions, With<PrimaryWindow>>,
    settings: Res<MovementSettings>,
    key_bindings: Res<KeyBindings>,
    mut query: Query<(&mut Freecam, &Transform, &PlanetariumCamera)>, //    mut query: Query<&mut Transform, With<FlyCam>>,
) {
    if let Ok(cursor_options) = cursor_options.single() {
        for (mut freecam, transform, pcam) in query.iter_mut() {
            if pcam.action == CameraAction::Free {
                let mut velocity = DVec3::ZERO;
                let local_z = transform.local_z().as_vec3().as_dvec3();
                let forward = -DVec3::new(local_z.x, 0., local_z.z);
                let right = DVec3::new(local_z.z, 0., -local_z.x);

                for key in keys.get_pressed() {
                    match cursor_options.grab_mode {
                        CursorGrabMode::None => (),
                        _ => {
                            let key = *key;
                            if key == key_bindings.move_forward {
                                velocity += forward;
                            } else if key == key_bindings.move_backward {
                                velocity -= forward;
                            } else if key == key_bindings.move_left {
                                velocity -= right;
                            } else if key == key_bindings.move_right {
                                velocity += right;
                            } else if key == key_bindings.move_ascend {
                                velocity += DVec3::Y;
                            } else if key == key_bindings.move_descend {
                                velocity -= DVec3::Y;
                            }
                        }
                    }
                }

                velocity = velocity.normalize_or_zero();

                freecam.bevy_pos += velocity * ((time.delta_secs() * settings.speed) as f64);
            }
        }
    } else {
        warn!("Primary window not found for `player_move`!");
    }
}

/// Handles looking around if cursor is locked
fn player_look(
    settings: Res<MovementSettings>,
    primary_window: Query<(&Window, &CursorOptions), With<PrimaryWindow>>,
    mut state: MessageReader<MouseMotion>,
    mut query: Query<(&mut Transform, &PlanetariumCamera, &Projection), With<Freecam>>,
) {
    if let Ok((window, cursor_options)) = primary_window.single() {
        for (mut transform, pcam, projection) in query.iter_mut() {
            for ev in state.read() {
                if pcam.action == CameraAction::Free {
                    let (mut yaw, mut pitch, _) = transform.rotation.to_euler(EulerRot::YXZ);
                    match cursor_options.grab_mode {
                        CursorGrabMode::None => (),
                        _ => {
                            let fov_factor = match projection {
                                Projection::Perspective(p) => { p.fov.to_degrees() / 90.0 }
                                _ => 1.0,
                            };

                            // Using smallest of height or width ensures equal vertical and horizontal sensitivity
                            let window_scale = window.height().min(window.width());
                            pitch -= (settings.sensitivity * ev.delta.y * window_scale * fov_factor);
                            yaw -= (settings.sensitivity * ev.delta.x * window_scale * fov_factor);
                        }
                    }

                    pitch = pitch.clamp(-1.54, 1.54);

                    // Order is important to prevent unintended roll
                    transform.rotation =
                        Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch);
                }
            }
        }
    } else {
        warn!("Primary window not found for `player_look`!");
    }
}

fn cursor_grab(
    keys: Res<ButtonInput<KeyCode>>,
    key_bindings: Res<KeyBindings>,
    mut primary_window_cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    state: Res<State<AppState>>,
) {
    if let Ok(mut cursor_options) = primary_window_cursor.single_mut() {
        if keys.just_pressed(key_bindings.toggle_grab_cursor) {
            toggle_grab_cursor(&mut cursor_options, state);
        }
    } else {
        warn!("Primary window not found for `cursor_grab`!");
    }
}

// Grab cursor when an entity with FlyCam is added
fn initial_grab_on_flycam_spawn(
    mut cursor_options: Query<&mut CursorOptions, With<PrimaryWindow>>,
    query_added: Query<Entity, Added<Freecam>>,
    state: Res<State<AppState>>,
) {
    if query_added.is_empty() {
        return;
    }

    if let Ok(cursor_options) = &mut cursor_options.single_mut() {
        toggle_grab_cursor(cursor_options, state);
    } else {
        warn!("Primary window not found for `initial_grab_cursor`!");
    }
}

/// Same as [`PlayerPlugin`] but does not spawn a camera
pub struct FreeCamPlugin;
impl Plugin for FreeCamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MovementSettings>()
            .init_resource::<KeyBindings>()
            .add_systems(Startup, initial_grab_on_flycam_spawn)
            .add_systems(Update, player_move)
            .add_systems(Update, player_look)
            .add_systems(Update, cursor_grab);
    }
}
