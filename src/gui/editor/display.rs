use bevy::prelude::*;
use glam::DVec3;
use crate::body::body::{Body, BodyProperties};
use crate::body::fixed::FixedBody;
use crate::gui::body::engine::VisibleBody;
use crate::gui::body::graphical;
use crate::gui::body::graphical::{Renderable, spawn_as_planet, spawn_as_star};
use crate::gui::common;
use crate::gui::editor::gui;

use super::super::common::{AppState, despawn_screen, DisplayQuality, Volume};

// This plugin will contain the game. In this case, it's just be a screen that will
// display the current settings for 5 seconds before returning to the menu
pub fn editor_plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Editor), editor_setup)
        .add_systems(Update, editor.run_if(in_state(AppState::Editor)))
        .add_systems(OnExit(AppState::Editor), despawn_screen::<OnEditorScreen>)
        .add_systems(
            Update,
            (crate::gui::menu::common::button_system, gui::menu_action).run_if(in_state(AppState::Editor)),
        ).add_systems(
        Update,
        (position_bodies_fixed::<FixedBody>).run_if(in_state(AppState::Editor)),
        );
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
            mass: 0.0,
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

    let planet = FixedBody {
        global_position: DVec3::new(1.0, 2.0, 0.0),
        properties: BodyProperties {
            mass: 0.0,
            name: "Some planet".to_string(),
        }
    };
    spawn_as_planet::<OnEditorScreen, FixedBody>(planet, &mut commands, &mut meshes, &mut materials);
}

fn position_bodies_fixed<BodyType: Body + Component + Renderable>(mut query: Query<(&mut Transform, &BodyType)>) {
    for (mut transform, fixed_body) in query.iter_mut() {
        transform.translation = fixed_body.world_space(fixed_body.global_position(), 1.4);
    }
}

// Tick the timer, and change state when finished
fn editor(
    time: Res<Time>,
) {

}
