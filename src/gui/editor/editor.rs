use std::sync::Arc;
use bevy::ecs::query::QueryData;
use bevy::prelude::*;
use bevy_trait_query::{One, RegisterExt};
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};
use crate::body::circular::CircularBody;
use crate::body::fixed::FixedBody;
use crate::body::kepler::KeplerBody;
use crate::body::newton::NewtonBody;
use crate::body::linear::LinearBody;
use crate::body::{SimulationSettings, universe};
use crate::body::universe::{FixedMotive, LinearMotive, NewBody, StupidCircle, Universe};
use crate::gui::body::graphical::{Renderable, spawn_as_planet, spawn_as_star};
use crate::gui::common;
use crate::gui::editor::gui;
use crate::gui::editor::gui::DebugText;
use crate::util::kepler::local;

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
            (position_bodies).run_if(in_state(AppState::Editor)),
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

#[derive(Component)]
pub(crate) struct BodyId(pub u32);

fn editor_setup(
    mut commands: Commands,
    display_quality: Res<DisplayQuality>,
    volume: Res<Volume>,
    mut universe: ResMut<Universe>,
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
    
    let sun_id = universe.add_body(NewBody {
        name: "Sun".to_string(),
        physics: universe::Motive::Fixed(
            FixedMotive{
                local_position: DVec3::ZERO,
            }
        ),
        mass: 10.0,
        radius: 1.0,
        parent: None,
    });
    spawn_as_star::<OnEditorScreen>(sun_id, universe.get_body(sun_id).unwrap(), &mut commands, &mut meshes, &mut materials);
    // (star_mesh, Star, ScreenTrait::default(), BodyId(body_id))

    let planet_id = universe.add_body(NewBody {
        name: "Planet".to_string(),
        physics: universe::Motive::StupidCircle(
            StupidCircle {
                radius: 3.0,
            }
        ),
        mass: 1.0,
        radius: 0.3,
        parent: Some(sun_id),
    });
    spawn_as_planet::<OnEditorScreen>(planet_id, universe.get_body(planet_id).unwrap(), &mut commands, &mut meshes, &mut materials);

    let moon_id = universe.add_body(NewBody {
        name: "Moon".to_string(),
        physics: universe::Motive::StupidCircle(
            StupidCircle {
                radius: 0.6
            }
        ),
        mass: 0.1,
        radius: 0.1,
        parent: Some(planet_id)
    });
    spawn_as_planet::<OnEditorScreen>(moon_id, universe.get_body(moon_id).unwrap(), &mut commands, &mut meshes, &mut materials);

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
            let position = universe.calc_position_at_time(time, body, origin);
            let position = (position * display_state.distance_scale).as_vec3();
            transform.translation = bevy::prelude::Vec3::new(position.x, position.y, position.z)
        }

        transform.scale = Vec3::new(display_state.body_scale,
                                    display_state.body_scale,
                                    display_state.body_scale,);
    }
}

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
