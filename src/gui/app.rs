use std::path::PathBuf;
use bevy::app::Plugin;
use bevy::core::FrameCount;
use bevy::DefaultPlugins;
use bevy::prelude::{App, Camera2d, Commands, PluginGroup, Res, Single, Startup, SystemSet, Update, Window, WindowPlugin};
use bevy::window::{ExitCondition, PresentMode};
use crate::gui::settings;
use crate::gui::util::ensure_folders;

pub fn open() {
    init();
    let settings = settings::load();

    App::new()
        .add_plugins(DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Exotic Matters".into(),
                    name: Some("exotic-matters.app".into()),
                    present_mode: PresentMode::AutoVsync,
                    prevent_default_event_handling: true,
                    visible: false,
                    ..Default::default()
                }),
                exit_condition: ExitCondition::OnPrimaryClosed,
                close_when_requested: true,
            }))
        .insert_resource(settings)
        .add_systems(Update, make_visible)
        .run();
}

pub fn init() {
    ensure_folders(
        &[
            &PathBuf::from("data"),
        ])
        .unwrap_or_else(|message| {
            println!("Client startup error: {}", message);
            std::process::exit(1);
        });
}

pub fn make_visible(mut window: Single<&mut Window>, frames: Res<FrameCount>) {
    if frames.0 == 3 {
        window.visible = true;
    }
}
