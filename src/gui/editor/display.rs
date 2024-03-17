use bevy::prelude::*;
use crate::gui::common;
use crate::gui::common::text;
use crate::gui::editor::gui;

use super::super::common::{despawn_screen, DisplayQuality, AppState, Volume};

// This plugin will contain the game. In this case, it's just be a screen that will
// display the current settings for 5 seconds before returning to the menu
pub fn editor_plugin(app: &mut App) {
    app.add_systems(OnEnter(AppState::Editor), editor_setup)
        .add_systems(Update, editor.run_if(in_state(AppState::Editor)))
        .add_systems(OnExit(AppState::Editor), despawn_screen::<OnEditorScreen>)
        .add_systems(
            Update,
            (crate::gui::menu::common::button_system, gui::menu_action).run_if(in_state(AppState::Editor)),
        );
}

// Tag component used to tag entities added on the game screen
#[derive(Component)]
pub(super) struct OnEditorScreen;

fn editor_setup(
    mut commands: Commands,
    display_quality: Res<DisplayQuality>,
    volume: Res<Volume>,
    asset_server: Res<AssetServer>,
) {
    let base_screen = common::base_screen(&mut commands);
    gui::ui_setup(&mut commands, asset_server.clone());
    
    commands.entity(base_screen)
        .insert(OnEditorScreen)
        .with_children(|parent| {

        });
}

// Tick the timer, and change state when finished
fn editor(
    time: Res<Time>,
) {

}
