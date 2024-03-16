use bevy::prelude::*;
use crate::gui;
use crate::gui::{editor, menu, splash};
use crate::gui::common::{BackGlow, DisplayQuality, Volume};


pub(crate) fn open() {
    App::new()
        .add_plugins(DefaultPlugins)
        .insert_resource(DisplayQuality::Medium)
        .insert_resource(Volume(7))
        .insert_resource(BackGlow::Subtle)
        .init_state::<gui::common::AppState>()
        .add_systems(Startup, setup)
        .add_plugins((
            splash::splash_plugin,
            menu::main::menu_plugin,
            editor::display::editor_plugin,
        ))
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera2dBundle::default());
}
