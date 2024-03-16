use bevy::prelude::{Commands, Component, Res};
use crate::gui::common;
use crate::gui::common::{text, Volume};

#[derive(Component)]
pub(crate) struct OnUITestScreen;

pub fn ui_test_menu_setup(mut commands: Commands) {
    let button_style = common::button_style();
    let button_text_style = text::primary();

    let base_screen = common::base_screen(&mut commands);
    commands.entity(base_screen)
        .insert(OnUITestScreen);
}
