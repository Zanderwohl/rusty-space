use bevy::prelude::*;
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};
use crate::body::fixed::FixedBody;
use crate::body::newton::NewtonBody;
use crate::body::SimulationSettings;
use crate::gui::body::graphical::{Renderable, spawn_as_planet, spawn_as_star};
use crate::gui::common;
use crate::gui::editor::gui;
use crate::gui::editor::gui::DebugText;

use super::super::common::{AppState, despawn_screen, DisplayQuality, Volume};

// This plugin will contain the game. In this case, it's just be a screen that will
// display the current settings for 5 seconds before returning to the menu
pub fn editor_plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Editor), editor_setup)
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
            (position_bodies_of_type::<FixedBody>).run_if(in_state(AppState::Editor)),
        )
        .add_systems(
            Update,
            (position_bodies_of_type::<NewtonBody>).run_if(in_state(AppState::Editor)),
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
    spawn_as_star::<OnEditorScreen, FixedBody>(sun1, &mut commands, &mut meshes, &mut materials);

    let sun2 = FixedBody {
        global_position: DVec3::new(1.0, 2.0, 0.0),
        properties: BodyProperties {
            mass: 0.0,
            name: "".to_string(),
        },
    };
    // sun2.spawn_as_star::<OnEditorScreen>(&mut commands, &mut meshes, &mut materials);

    let planet = NewtonBody {
        global_position: DVec3::new(1.0, 2.0, 0.0),
        properties: BodyProperties {
            mass: 1.0,
            name: "Some planet".to_string(),
        }
    };
    spawn_as_planet::<OnEditorScreen, NewtonBody>(planet, &mut commands, &mut meshes, &mut materials);
}

fn position_bodies_of_type<BodyType: Body + Component + Renderable>(mut query: Query<(&mut Transform, &BodyType)>,
    display_state: Res<DisplayState>) {
    for (mut transform, fixed_body) in query.iter_mut() {
        transform.translation = fixed_body.world_space(fixed_body.global_position(), display_state.display_scale);
        transform.scale = Vec3::new(display_state.body_scale,
                                    display_state.body_scale,
                                    display_state.body_scale,);
    }
}

fn handle_time(mut display_state: ResMut<DisplayState>,
               keyboard: Res<ButtonInput<KeyCode>>,
               mut query: Query<(&mut Transform, &mut Text), With<DebugText>>) {
    for (transform, mut text) in query.iter_mut() {
        let text = &mut text.sections[0].value;
        text.clear();
        text.push_str(&*format!("Time (left/right): {}", display_state.current_time));
    }
    if keyboard.just_pressed(KeyCode::ArrowLeft) {
        display_state.current_time -= 1.0;
    }
    if keyboard.just_pressed(KeyCode::ArrowRight) {
        display_state.current_time += 1.0;
    }
}

// Tick the timer, and change state when finished
fn editor(
    time: Res<Time>,
) {

}
