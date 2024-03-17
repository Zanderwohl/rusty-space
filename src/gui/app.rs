use bevy::prelude::*;
use bevy::window::ExitCondition;
use crate::gui;
use crate::gui::{editor, menu, splash};
use crate::gui::common::{BackGlow, DisplayQuality, Volume};
use bevy::{prelude::*, winit::WinitWindows};
use winit::window::Icon;


pub(crate) fn open() {
    App::new()
        .add_plugins(DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Exotic Matters".to_string(),
                    ..Default::default()
                }),
                exit_condition: ExitCondition::OnPrimaryClosed,
                close_when_requested: true,
            })
        )
        .add_systems(Startup, set_window_icon)
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

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.spawn(Camera2dBundle::default());
}

fn set_window_icon(
    // we have to use `NonSend` here
    windows: NonSend<WinitWindows>,
) {
    // here we use the `image` crate to load our icon data from a png file
    // this is not a very bevy-native solution, but it will do
    let (icon_rgba, icon_width, icon_height) = {
        let image = image::open("assets/favicon.png")
            .expect("Failed to open icon path")
            .into_rgba8();
        let (width, height) = image.dimensions();
        let rgba = image.into_raw();
        (rgba, width, height)
    };
    let icon = Icon::from_rgba(icon_rgba, icon_width, icon_height).unwrap();

    // do it for all windows
    for window in windows.windows.values() {
        window.set_window_icon(Some(icon.clone()));
    }
}
