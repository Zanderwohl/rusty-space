use std::sync::Arc;
use bevy::prelude::*;
use bevy_trait_query::{One, RegisterExt};
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};
use crate::body::circular::CircularBody;
use crate::body::fixed::FixedBody;
use crate::body::kepler::KeplerBody;
use crate::body::newton::NewtonBody;
use crate::body::linear::LinearBody;
use crate::body::SimulationSettings;
use crate::body::universe::Universe;
use crate::gui::body::graphical::{Renderable, spawn_as_planet, spawn_as_star};
use crate::gui::common;
use crate::gui::editor::gui;
use crate::gui::editor::gui::DebugText;

use super::super::common::{AppState, despawn_screen, DisplayQuality, Volume};

// This plugin will contain the game. In this case, it's just be a screen that will
// display the current settings for 5 seconds before returning to the menu
pub fn editor_plugin(app: &mut App) {
    app
        .register_component_as::<dyn Body, FixedBody>()
        .register_component_as::<dyn Body, CircularBody>()
        .register_component_as::<dyn Body, LinearBody>()
        .register_component_as::<dyn Body, NewtonBody>()
        .register_component_as::<dyn Body, KeplerBody>()
        .add_systems(OnEnter(AppState::Editor), editor_setup)
        .add_systems(Update, editor.run_if(in_state(AppState::Editor)))
        .add_systems(OnExit(AppState::Editor), despawn_screen::<OnEditorScreen>)
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
        .insert_resource(
            Universe::default()
        )
        .add_systems(
            Update,
            (crate::gui::menu::common::button_system, gui::menu_action).run_if(in_state(AppState::Editor)),
        )
        .add_systems(
            Update,
            (position_bodies_of_type).run_if(in_state(AppState::Editor)),
        )
        .add_systems(
            Update,
            handle_time.run_if(in_state(AppState::Editor)),
        );
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
pub(super) struct OnEditorScreen;

#[derive(Component)]
pub(crate) struct Star;

#[derive(Component)]
pub(crate) struct Planet;

fn editor_setup(
    mut commands: Commands,
    display_quality: Res<DisplayQuality>,
    volume: Res<Volume>,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>
) {
    let base_screen = common::base_screen(&mut commands);
    gui::ui_setup(&mut commands, asset_server.clone());
    
    commands.entity(base_screen)
        .insert(OnEditorScreen)
        .with_children(|parent| {
        });
    
    let sun1 = FixedBody {
        global_position: DVec3::ZERO,
        properties: BodyProperties {
            mass: 10.0,
            name: "".to_string(),
            size: 1.0,
        },
    };
    let sun_id = spawn_as_star::<OnEditorScreen, FixedBody>(sun1, &mut commands, &mut meshes, &mut materials);

    let sun2 = FixedBody {
        global_position: DVec3::new(1.0, 2.0, 0.0),
        properties: BodyProperties {
            mass: 0.0,
            name: "".to_string(),
            size: 0.2,
        },
    };
    // sun2.spawn_as_star::<OnEditorScreen>(&mut commands, &mut meshes, &mut materials);

    let planet = CircularBody {
        properties: BodyProperties {
            mass: 1.0,
            name: "Some planet".to_string(),
            size: 0.2,
        },
        radius: 3.0,
    };
    let planet = spawn_as_planet::<OnEditorScreen, CircularBody>(planet, &mut commands, &mut meshes, &mut materials);

    let moon = CircularBody {
        properties: BodyProperties {
            mass: 0.1,
            name: "Some moon".to_string(),
            size: 0.1,
        },
        radius: 0.5,
    };
    let moon = spawn_as_planet::<OnEditorScreen, CircularBody>(moon, &mut commands, &mut meshes, &mut materials);
    commands.entity(planet).push_children(&[moon]);

    let free = NewtonBody {
        global_position: DVec3::new(3.0, 1.0, 0.0),
        global_velocity: DVec3::ZERO,
        properties: BodyProperties {
            mass: 1.0,
            name: "Free body".to_string(),
            size: 0.8,
        },
    };
    let free = spawn_as_planet::<OnEditorScreen, NewtonBody>(free, &mut commands, &mut meshes, &mut materials);
}

fn position_bodies_of_type(
    mut query: Query<(&mut Transform, One<&dyn Body>)>,
    display_state: Res<DisplayState>
) {
    for (mut transform, body) in query.iter_mut() {
        let world_position = (body.local_position_after_time(display_state.current_time) * display_state.distance_scale).as_vec3();
        let converted_position = Vec3::new(world_position.x, world_position.y, world_position.z);
        transform.translation = converted_position;
        transform.scale = Vec3::new(display_state.body_scale,
                                    display_state.body_scale,
                                    display_state.body_scale, );
    }
}

// fn move_newton_bodies

fn handle_time(mut display_state: ResMut<DisplayState>,
               keyboard: Res<ButtonInput<KeyCode>>,
               mut query: Query<(&mut Transform, &mut Text), With<DebugText>>,
               time: Res<Time>,) {
    for (_transform, mut text) in query.iter_mut() {
        let text = &mut text.sections[0].value;
        text.clear();
        text.push_str(&*format!("{}\n", if display_state.playing { "[P]laying" } else { "[P]aused" }));
        text.push_str(&*format!("Time scale ([/]): {:.2}\n", display_state.time_step_size()));
        text.push_str(&*format!("Time (left/right): {:.2}\n", display_state.current_time));
        text.push_str(&*format!("Object scale (i/o): {:.1}\n", display_state.body_scale));
        text.push_str(&*format!("Distance scale (k/l): {:.1}", display_state.distance_scale));
    }
    if keyboard.just_pressed(KeyCode::ArrowLeft) {
        display_state.current_time -= 1.0 * display_state.time_scale;
    }
    if keyboard.just_pressed(KeyCode::ArrowRight) {
        display_state.current_time += 1.0 * display_state.time_scale;
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
    if keyboard.pressed(KeyCode::ArrowUp) {
        display_state.current_time += 1.0 * display_state.time_step_size();
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        display_state.current_time -= 1.0 * display_state.time_step_size();
    }
    if keyboard.just_pressed(KeyCode::KeyI) {
        display_state.body_scale -= 0.1;
    }
    if keyboard.just_pressed(KeyCode::KeyO) {
        display_state.body_scale += 0.1;
    }
    if keyboard.just_pressed(KeyCode::KeyK) {
        display_state.distance_scale -= 0.1;
    }
    if keyboard.just_pressed(KeyCode::KeyL) {
        display_state.distance_scale += 0.1;
    }
    if keyboard.just_pressed(KeyCode::KeyP) {
        display_state.playing = !display_state.playing;
    }
    if display_state.playing {
        display_state.current_time += 10.0f64 * time.delta_seconds() as f64 * display_state.time_scale;
    }
}

// Tick the timer, and change state when finished
fn editor(
    time: Res<Time>,
) {

}
