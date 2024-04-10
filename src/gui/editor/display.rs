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
            display_scale: 1.5,
            body_scale: 1.0,
        })
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
    display_scale: f64,
    body_scale: f32,
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
        },
    };
    let sun_id = spawn_as_star::<OnEditorScreen, FixedBody>(sun1, &mut commands, &mut meshes, &mut materials);

    let sun2 = FixedBody {
        global_position: DVec3::new(1.0, 2.0, 0.0),
        properties: BodyProperties {
            mass: 0.0,
            name: "".to_string(),
        },
    };
    // sun2.spawn_as_star::<OnEditorScreen>(&mut commands, &mut meshes, &mut materials);

    let planet = CircularBody {
        properties: BodyProperties {
            mass: 1.0,
            name: "Some planet".to_string(),
        },
        radius: 3.0,
    };
    spawn_as_planet::<OnEditorScreen, CircularBody>(planet, &mut commands, &mut meshes, &mut materials);
}

fn position_bodies_of_type(
    mut query: Query<(&mut Transform, One<&dyn Body>)>,
    display_state: Res<DisplayState>
) {
    for (mut transform, body) in query.iter_mut() {
        let world_position = (body.global_position_after_time(display_state.current_time) * display_state.display_scale).as_vec3();
        transform.translation = world_position;
        transform.scale = Vec3::new(display_state.body_scale,
                                    display_state.body_scale,
                                    display_state.body_scale, );
    }
}

fn handle_time(mut display_state: ResMut<DisplayState>,
               keyboard: Res<ButtonInput<KeyCode>>,
               mut query: Query<(&mut Transform, &mut Text), With<DebugText>>) {
    for (_transform, mut text) in query.iter_mut() {
        let text = &mut text.sections[0].value;
        text.clear();
        text.push_str(&*format!("Time (left/right): {}", display_state.current_time));
    }
    if keyboard.just_pressed(KeyCode::ArrowLeft) {
        display_state.current_time -= 100.0;
    }
    if keyboard.just_pressed(KeyCode::ArrowRight) {
        display_state.current_time += 100.0;
    }
    if keyboard.pressed(KeyCode::ArrowUp) {
        display_state.current_time += 100.0;
    }
    if keyboard.pressed(KeyCode::ArrowDown) {
        display_state.current_time -= 100.0;
    }
}

// Tick the timer, and change state when finished
fn editor(
    time: Res<Time>,
) {

}
