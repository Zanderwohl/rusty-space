use std::fmt::Debug;
use std::fs;
use bevy::prelude::*;
use crate::body::SimulationSettings;
use crate::body::universe::Universe;
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
        .insert_resource(SimulationSettings {
            gravity_constant: 1.0,
        })
        .insert_resource(DisplayState {
            current_time: 0.0,
            time_scale: 1.0,
            distance_scale: 1.5,
            body_scale: 1.0,
            playing: false,
        })
        .add_systems(
            Update,
            (crate::gui::menu::common::button_system, gui::menu_action).run_if(in_state(AppState::Planetarium)).run_if(in_state(AppState::Planetarium)),
        )
        .add_systems(
            Update,
            (position_bodies, handle_time).run_if(in_state(AppState::Planetarium)).run_if(in_state(AppState::Planetarium)),
        );
}

pub struct SaveItems {
    display_state: DisplayState,
    universe: Universe,
}

#[derive(Resource, Debug, Component, PartialEq, /*Eq,*/ Clone, Copy)]
struct DisplayState {
    current_time: f64,
    time_scale: f64,
    distance_scale: f64,
    body_scale: f32,
    playing: bool,
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
                    universe.recount();
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

fn position_bodies(
    universe: Res<Universe>,
    mut query: Query<(&mut Transform, &BodyId)>,
    display_state: Res<DisplayState>,
) {
    let time = display_state.current_time;
    for (mut transform, body_id) in query.iter_mut() {
        let body = universe.get_body(body_id.0);
        if let Some(body) = body {
            let origin = universe.calc_origin_at_time(time, body);
            let position_big = universe.calc_position_at_time(time, body, origin);
            let position = (position_big * display_state.distance_scale).as_vec3();
            transform.translation = bevy::prelude::Vec3::new(position.x, position.y, position.z);
        }

        transform.scale = Vec3::new(display_state.body_scale,
                                    display_state.body_scale,
                                    display_state.body_scale,);
        transform.rotation = Quat::from_rotation_z(time as f32 / 100.0);
    }
}

fn handle_time(mut display_state: ResMut<DisplayState>,
               keyboard: Res<ButtonInput<KeyCode>>,
               mut query: Query<(&mut Transform, &mut Text), With<DebugText>>,
               time: Res<Time>,
               app_state: Res<State<AppState>>) {
    for (_transform, mut text) in query.iter_mut() {
        let text = &mut text.sections[0].value;
        text.clear();
        text.push_str(&*format!("{}\n", if display_state.playing { "[P]laying" } else { "[P]aused" }));
        text.push_str(&*format!("Time scale ([/]): {:.2}\n", display_state.time_step_size()));
        text.push_str(&*format!("Time (left/right): {:.2}\n", display_state.current_time));
        text.push_str(&*format!("Object scale (i/o): {:.1}\n", display_state.body_scale));
        text.push_str(&*format!("Distance scale (k/l): {:.1}", display_state.distance_scale));
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
    if display_state.playing{
        display_state.current_time += time.delta_seconds() as f64 * display_state.time_step_size();
    }
}

fn show_trajectories(
    mut universe: ResMut<Universe>,
) {

}

// Tick the timer, and change state when finished
fn editor(
    time: Res<Time>,
) {

}
