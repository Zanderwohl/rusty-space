use std::path::PathBuf;
use bevy::color::Color;
use bevy::core::FrameCount;
use bevy::DefaultPlugins;
use bevy::prelude::{App, AppExtStates, Camera3d, ClearColor, Commands, PluginGroup, Res, Single, Startup, States, Update, Window, WindowPlugin};
use bevy::window::{ExitCondition, PresentMode};
use bevy_egui::EguiPlugin;
use crate::body::universe::solar_system::{write_temp_system_file, write_tiny_system_file};
use crate::gui::menu::{close_when_requested, MenuPlugin};
use crate::gui::planetarium::Planetarium;
use crate::gui::settings;
use crate::gui::splash::SplashPlugin;
use crate::gui::util::ensure_folders;

pub fn run() {
    init();
    let settings = settings::load();
    write_temp_system_file();
    write_tiny_system_file();

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
                close_when_requested: false,
            }))
        .insert_resource(settings)
        .add_systems(Startup, common_setup)
        .add_systems(Update, close_when_requested)
        .insert_state(AppState::Splash)
        .insert_resource(ClearColor(Color::BLACK))
        .add_plugins(EguiPlugin)
        .add_plugins(SplashPlugin)
        .add_plugins(MenuPlugin)
        .add_plugins(Planetarium)
        .add_systems(Update, (
            make_visible,
        ))
        .run();
}

#[derive(States, Debug, Clone, PartialEq, Eq, Hash)]
pub enum AppState {
    Splash,
    MainMenu,
    Planetarium,
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

pub fn common_setup(mut commands: Commands) {
    commands
        .spawn(Camera3d::default())
    ;
}
