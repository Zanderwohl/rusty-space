use std::fmt::Debug;
use std::fs;
use bevy::prelude::*;
use crate::body::SimulationSettings;
use crate::body::universe::{TrajectoryMode, Universe};
use crate::gui::body::graphical::spawn_bevy;
use crate::gui::common;
use crate::gui::common::BackGlow;
use crate::gui::menu::save_select::SaveEntry;
use crate::gui::planetarium::gui;
use crate::gui::planetarium::gui::{DebugText, EscMenuState};
use super::super::common::{AppState, despawn_screen, DisplayQuality, Volume};

// This plugin will contain the game. In this case, it's just be a screen that will
// display the current settings for 5 seconds before returning to the menu
pub fn planetarium_plugin(app: &mut App) {
    app
        .add_plugins(gui::esc_menu_plugin)
        .add_systems(OnEnter(AppState::Planetarium), planetarium_setup)
        .add_systems(Update, editor.run_if(in_state(AppState::Planetarium)))
        .add_systems(OnExit(AppState::Planetarium), despawn_screen::<OnPlanetariumScreen>)
        .init_gizmo_group::<OrbitalTrajectories>()
        .insert_resource(SimulationSettings {
            gravity_constant: 1.0,
        })
        .insert_resource(DisplayState {
            current_time: 0.0,
            time_scale: 1.0,
            distance_scale: 1.5,
            body_scale: 1.0,
            playing: false,
            trajectory_mode: TrajectoryMode::Global,
            trajectory_display: TrajectoryDisplay::All,
            focused: None,
        })
        .add_systems(
            Update,
            (crate::gui::menu::common::button_system, gui::menu_action).run_if(in_state(AppState::Planetarium)).run_if(in_state(AppState::Planetarium)),
        )
        .add_systems(
            PreUpdate,
            (calc_for_current_time).run_if(in_state(AppState::Planetarium)),
        )
        .add_systems(
            Update,
            (position_bodies, draw_trajectories).run_if(in_state(AppState::Planetarium)),
        )
        .add_systems(
            PostUpdate,
            (handle_time).run_if(in_state(AppState::Planetarium)),
        );
}


#[derive(Default, Reflect, GizmoConfigGroup)]
struct OrbitalTrajectories;

pub struct SaveItems {
    display_state: DisplayState,
    universe: Universe,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TrajectoryDisplay {
    None,
    FocusedOnly,
    FocusedAscendingPrimaries,
    All,
}

#[derive(Resource, Debug, Component, PartialEq, /*Eq,*/ Clone, Copy)]
struct DisplayState {
    current_time: f64,
    time_scale: f64,
    distance_scale: f64,
    body_scale: f32,
    playing: bool,
    trajectory_mode: TrajectoryMode,
    trajectory_display: TrajectoryDisplay,
    focused: Option<u32>,
}

impl DisplayState {
    pub fn time_step_size(&self) -> f64 {
        self.time_scale * self.time_scale * self.time_scale
    }
}

// Tag component used to tag entities added on the game screen
#[derive(Component, Default)]
pub(super) struct OnPlanetariumScreen;

#[derive(Component)]
pub(crate) struct BodyId(pub u32);


fn planetarium_setup(
    mut commands: Commands,
    display_quality: Res<DisplayQuality>,
    volume: Res<Volume>,
    glow: Res<BackGlow>,
    save: Res<SaveEntry>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let base_screen = common::base_screen(&mut commands);
    gui::ui_setup(&mut commands, asset_server.clone());
    
    commands.entity(base_screen)
        .insert(OnPlanetariumScreen)
        .with_children(|parent| {
        });

    let universe_data = fs::read_to_string(&save.path);
    match universe_data {
        Ok(universe_data) => {
            let universe = serde_yaml::from_str::<Universe>(&*universe_data);
            match universe {
                Ok(mut universe) => {
                    for (id, body) in universe.bodies.iter() {
                        spawn_bevy::<OnPlanetariumScreen>(*id, body, &mut commands, &mut meshes, &mut materials);
                    }
                    commands.insert_resource(universe);
                }
                Err(error) => {
                    // TODO: handle this in the GUI
                    panic!("Couldn't read universe structure! {}", error)
                }
            }
        }
        Err(error) => {
            // TODO: handle this in the GUI
            panic!("Couldn't read file! {}", error)
        }
    }

}

fn calc_for_current_time(
    mut universe: ResMut<Universe>,
    display_state: Res<DisplayState>,
) {
    universe.calc_positions_at_time(display_state.current_time);
}

fn position_bodies(
    universe: Res<Universe>,
    mut query: Query<(&mut Transform, &BodyId)>,
    display_state: Res<DisplayState>,
) {
    let time = display_state.current_time;
    for (mut transform, body_id) in query.iter_mut() {
        let position_big = universe.get_global_position_at_time(body_id.0, time);
        let position = (position_big * display_state.distance_scale).as_vec3();
        transform.translation = bevy::prelude::Vec3::new(position.x, position.y, position.z);


        transform.scale = Vec3::new(display_state.body_scale,
                                    display_state.body_scale,
                                    display_state.body_scale,);
        transform.rotation = Quat::from_rotation_z(time as f32 / 100.0);
    }
}

fn draw_trajectories(
    universe: Res<Universe>,
    display_state: Res<DisplayState>,
    mut trajectory_gizmos: Gizmos<OrbitalTrajectories>,
) {
    let display = display_state.trajectory_display;
    let mode = display_state.trajectory_mode;
    let scale = display_state.distance_scale as f32;
    match display {
        TrajectoryDisplay::None => {}
        TrajectoryDisplay::FocusedOnly => match display_state.focused {
            None => {}
            Some(body_id) => {
                display_single_trajectory(&universe, display_state.current_time, scale, &mut trajectory_gizmos, &body_id, mode);

            }
        }
        TrajectoryDisplay::FocusedAscendingPrimaries => {}
        TrajectoryDisplay::All => {
            for id in universe.bodies.keys() {
                display_single_trajectory(&universe, display_state.current_time, scale, &mut trajectory_gizmos, id, mode);
            }
        }
    }
}

fn display_single_trajectory(universe: &Res<Universe>, time: f64, scale: f32, trajectory_gizmos: &mut Gizmos<OrbitalTrajectories>, id: &u32, mode: TrajectoryMode) {
    let trajectory = universe.get_trajectory_for(*id, time, mode);
    for positions in trajectory.windows(2) {
        let pos1 = positions[0].as_vec3();
        let pos1 = bevy::prelude::Vec3::new(pos1.x, pos1.y, pos1.z) * scale;
        let pos2 = positions[1].as_vec3();
        let pos2 = bevy::prelude::Vec3::new(pos2.x, pos2.y, pos2.z) * scale;
        trajectory_gizmos.line(
            pos1,
            pos2,
            Color::BLUE,
        );
    }
}

fn handle_time(mut display_state: ResMut<DisplayState>,
               keyboard: Res<ButtonInput<KeyCode>>,
               mut query: Query<(&mut Transform, &mut Text), With<DebugText>>,
               time: Res<Time>,
               universe: Res<Universe>,
               app_state: Res<State<AppState>>) {
    let (focused, focused_name) = match display_state.focused {
        None => { (None, "Nothing".to_string()) }
        Some(body_id) => {
            if let Some(body) = universe.bodies.get(&body_id) {
                (Some(body_id), body.get_name())
            } else {
                (None, "Nothing".to_string())
            }
        }
    };
    for (_transform, mut text) in query.iter_mut() {
        let text = &mut text.sections[0].value;
        text.clear();
        text.push_str(&*format!("{}\n", if display_state.playing { "[P]laying" } else { "[P]aused" }));
        text.push_str(&*format!("Time scale ([/]): {:.2}\n", display_state.time_step_size()));
        text.push_str(&*format!("Time (left/right): {:.2}\n", display_state.current_time));
        text.push_str(&*format!("Object scale (i/o): {:.1}\n", display_state.body_scale));
        text.push_str(&*format!("Distance scale (k/l): {:.1}\n", display_state.distance_scale));
        text.push_str(&*format!("Trajectory display: (,): {:?}\n", display_state.trajectory_display));
        text.push_str(&*format!("Trajectory mode: (.): {:?}\n", display_state.trajectory_mode));
        text.push_str(&*format!("Focused (z,x): {}", focused_name));
    }
    if keyboard.just_pressed(KeyCode::ArrowLeft) && !display_state.playing {
        display_state.current_time -= 10.0 * time.delta_seconds() as f64 * display_state.time_step_size();
    }
    if keyboard.just_pressed(KeyCode::ArrowRight) && !display_state.playing  {
        display_state.current_time += 10.0 * time.delta_seconds() as f64 * display_state.time_step_size();
    }
    if keyboard.just_pressed(KeyCode::BracketLeft) {
        display_state.time_scale -= 0.2;
    }
    if keyboard.just_pressed(KeyCode::BracketRight) {
        display_state.time_scale += 0.2;
    }
    if display_state.time_scale.abs() < 0.01 {
        display_state.time_scale = 0.0;
    }
    if keyboard.pressed(KeyCode::ArrowUp) && !display_state.playing {
        display_state.current_time += 10.0 * time.delta_seconds() as f64 * display_state.time_step_size();
    }
    if keyboard.pressed(KeyCode::ArrowDown) && !display_state.playing {
        display_state.current_time -= 10.0 * time.delta_seconds() as f64 * display_state.time_step_size();
    }
    if keyboard.just_pressed(KeyCode::KeyI) {
        if display_state.body_scale > 0.1 {
            display_state.body_scale -= 0.1;
        }
    }
    if keyboard.just_pressed(KeyCode::KeyO) {
        display_state.body_scale += 0.1;
    }
    if keyboard.just_pressed(KeyCode::KeyK) {
        if display_state.distance_scale > 0.1 {
            display_state.distance_scale -= 0.1;
        }
    }
    if keyboard.just_pressed(KeyCode::KeyL) {
        display_state.distance_scale += 0.1;
    }
    if keyboard.just_pressed(KeyCode::KeyP) {
        display_state.playing = !display_state.playing;
    }
    if keyboard.just_pressed(KeyCode::KeyX) {
        display_state.focused = None;
    }
    if keyboard.just_pressed(KeyCode::KeyZ) {
        display_state.focused = Some(7);
    }
    if keyboard.just_pressed(KeyCode::Comma) {
        display_state.trajectory_display = match display_state.trajectory_display {
            TrajectoryDisplay::None => { TrajectoryDisplay::FocusedOnly }
            TrajectoryDisplay::FocusedOnly => { TrajectoryDisplay::All }
            TrajectoryDisplay::FocusedAscendingPrimaries => { TrajectoryDisplay::All }
            TrajectoryDisplay::All => { TrajectoryDisplay::FocusedOnly }
        }
    }
    if keyboard.just_pressed(KeyCode::Period) {
        display_state.trajectory_mode = match display_state.trajectory_mode {
            TrajectoryMode::Global => { TrajectoryMode::LocalToEachPrimary }
            TrajectoryMode::LocalToEachPrimary => { TrajectoryMode::Global }
            TrajectoryMode::LocalToCurrentPrimary => { TrajectoryMode::Global }
        }
    }
    if display_state.playing{
        display_state.current_time += time.delta_seconds() as f64 * display_state.time_step_size();
    }
}

// Tick the timer, and change state when finished
fn editor(
    time: Res<Time>,
) {

}
