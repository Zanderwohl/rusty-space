use std::path::PathBuf;
use bevy::color::Color;
use bevy::core_pipeline::bloom::Bloom;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::DefaultPlugins;
use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use bevy::window::{ExitCondition, PresentMode};
use bevy_egui::EguiPlugin;
use crate::body::universe::solar_system::{write_temp_system_file, write_earth_moon_file};
use crate::body::universe::Universe;
use crate::gui::menu::{close_when_requested, MenuPlugin};
use crate::gui::planetarium::PlanetariumUI;
use crate::gui::settings;
use crate::gui::splash::SplashPlugin;
use crate::gui::util::debug::DebugPlugin;
use crate::gui::util::ensure_folders;
use crate::gui::util::freecam::{FlyCam, FreeCam};

pub fn run() {
    init();
    let settings = settings::load();
    write_temp_system_file();
    write_earth_moon_file();

    App::new()
        .add_plugins(DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Exotic Matters".into(),
                    name: Some("exotic-matters.app".into()),
                    present_mode: PresentMode::AutoVsync,
                    prevent_default_event_handling: true,
                    visible: true,
                    ..Default::default()
                }),
                exit_condition: ExitCondition::OnPrimaryClosed,
                close_when_requested: false,
            }))
        .insert_resource(settings)
        .init_resource::<Universe>()
        .add_systems(Startup, common_setup)
        .add_systems(Update, close_when_requested)
        .insert_state(AppState::Splash)
        .insert_resource(ClearColor(Color::BLACK))
        .add_plugins(EguiPlugin::default())
        .add_plugins(DebugPlugin)
        .add_plugins(SplashPlugin)
        .add_plugins(MenuPlugin)
        .add_plugins(PlanetariumUI)
        .add_plugins(FreeCam)
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
    PlanetariumLoading,
}

pub fn init() {
    ensure_folders(
        &[
            &PathBuf::from("data"),
            &PathBuf::from("data/templates"),
            &PathBuf::from("data/saves"),
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

#[derive(Component)]
pub struct PlanetariumCamera;

pub fn common_setup(
    mut commands: Commands,
    mut ambient_light: ResMut<AmbientLight>
) {
    ambient_light.brightness = 0.1;

    commands.spawn((
        Camera3d {
            ..Default::default()
        },
        Camera {
            hdr: true,
            ..Default::default()
        },
        Transform::from_xyz(20., 2.0, 0.0).looking_at(Vec3::ZERO, Vec3::Y),
        FlyCam,
        PlanetariumCamera,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,

    ));
}
