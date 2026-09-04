pub(crate) mod settings;
mod save_load;
pub mod escape;
mod home;
pub mod widgets;

use std::fs;

use std::ops::Deref;
use std::path::PathBuf;
use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::window::{ClosingWindow, WindowCloseRequested};
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use crate::gui::app::AppState;
use crate::gui::menu::home::HomeMenuPlugin;
use crate::gui::settings::{Settings, UiTheme};

#[derive(Resource)]
pub struct UiState {
    pub quit_requested: bool,
    pub current_save: Option<SaveFileMeta>,
}

// Re-export TagState from body::universe::save for backwards compatibility
pub use crate::body::universe::save::TagState;

impl Default for UiState {
    fn default() -> Self {
        Self {
            quit_requested: false,
            current_save: None,
        }
    }
}

pub struct MenuPlugin;

#[derive(States, Debug, Clone, PartialEq, Eq, Hash)]
pub enum MenuState {
    Home,
    Planetarium,
    Settings,
}

impl Plugin for MenuPlugin {
    fn build (&self, app: &mut App) {
        app
            .add_plugins(HomeMenuPlugin)
            .insert_state(MenuState::Home)
            .init_resource::<UiState>()
            .init_resource::<PlanetariumFiles>()
            .add_systems(OnEnter(MenuState::Planetarium), load_planetarium_files)
            .add_systems(OnEnter(AppState::MainMenu), load_planetarium_files.run_if(in_state(MenuState::Planetarium)))
            .add_systems(EguiPrimaryContextPass, (
                (save_load::planetarium_menu,).run_if(in_state(AppState::MainMenu).and(in_state(MenuState::Planetarium))),
                (settings_menu,).run_if(in_state(AppState::MainMenu).and(in_state(MenuState::Settings))),
            ))
            .add_systems(Update, (quit_system, widgets::button_hover_system))
        ;
    }
}

#[derive(Resource)]
pub struct PlanetariumFiles {
    templates: Vec<SaveFileMeta>,
    saves: Vec<SaveFileMeta>,
}

impl Default for PlanetariumFiles {
    fn default() -> Self {
        Self {
            templates: vec![],
            saves: vec![],
        }
    }
}

#[derive(Clone)]
pub struct SaveFileMeta {
    pub path: PathBuf,
    pub file_name: String,
}

pub fn load_planetarium_files(mut files: ResMut<PlanetariumFiles>) {
    files.templates.clear();
    files.saves.clear();

    let template_files = fs::read_dir("data/templates").unwrap();
    let save_files = fs::read_dir("data/saves").unwrap();

    for file in template_files {
        let file = file.unwrap();
        if file.path().is_file() {
            let path = file.path();
            let path2 = path.clone();
            let name = path2.file_name().unwrap().to_str().unwrap();
            files.templates.push(SaveFileMeta {
                path,
                file_name: name.to_string(),
            })
        }
    }
    for file in save_files {
        let file = file.unwrap();
        if file.path().is_file() {
            let path = file.path();
            let path2 = path.clone();
            let name = path2.file_name().unwrap().to_str().unwrap();
            files.saves.push(SaveFileMeta {
                path,
                file_name: name.to_string(),
            })
        }
    }
    // info!("{}, {}", files.templates.len(), files.saves.len());
}

pub fn settings_menu(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut next_menu: ResMut<NextState<MenuState>>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();
    
    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        if ui.button("Back").clicked() {
            next_menu.set(MenuState::Home)
        }

        ui.add_space(8.0);

        // Nest the panel in a centered, max-width column so the full-screen
        // menu gets comfortable left/right padding without stretching edge to edge.
        let max_width = 640.0_f32.min(ui.available_width() - 40.0);
        ui.vertical_centered(|ui| {
            ui.set_max_width(max_width);
            ui.heading("Settings");

            ui.separator();

            settings::settings_panel(settings.as_mut(), None, ui);
        });
    });
}

pub fn quit_system (
    ui_state: Res<UiState>,
    mut exit: MessageWriter<AppExit>
) {
    if ui_state.quit_requested {
        exit.write(AppExit::Success);
    }
}

/// Copied and modified from https://docs.rs/bevy_window/0.15.3/src/bevy_window/system.rs.html#42-58
pub fn close_when_requested(
    mut commands: Commands,
    mut closed: MessageReader<WindowCloseRequested>,
    closing: Query<Entity, With<ClosingWindow>>,
    settings: Res<Settings>,
) {
    // This was inserted by us on the last frame so now we can despawn the window
    let mut any_closing = false;
    for window in closing.iter() {
        commands.entity(window).despawn();
        any_closing = true;
    }
    // Mark the window as closing so we can despawn it on the next frame
    for event in closed.read() {
        // When spamming the window close button on windows (other platforms too probably)
        // we may receive a `WindowCloseRequested` for a window we've just despawned in the above
        // loop.
        commands.entity(event.window).try_insert(ClosingWindow);
    }

    // Only write settings when they've changed or when the app is closing
    if settings.is_changed() || any_closing {
        let _ = fs::write("data/settings.toml", toml::to_string_pretty(settings.deref()).unwrap());
    }
}
