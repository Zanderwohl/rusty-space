pub(crate) mod settings;
mod save_load;

use std::fs;
use std::ops::Deref;
use std::path::PathBuf;
use bevy::app::AppExit;
use bevy::prelude::{in_state, info, App, AppExtStates, Commands, Condition, Entity, EventReader, EventWriter, IntoSystemConfigs, NextState, OnEnter, OnExit, Plugin, Query, Res, ResMut, Resource, State, States, SystemSet, Update, With};
use bevy::prelude::IntoSystemSetConfigs;
use bevy::text::cosmic_text::ttf_parser::Tag;
use bevy::utils::HashMap;
use bevy::window::{ClosingWindow, WindowCloseRequested};
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use serde::{Deserialize, Serialize};
use crate::gui::app::AppState;
use crate::gui::settings::{Settings, UiTheme};
use crate::body::universe::Universe;

#[derive(Resource)]
pub struct UiState {
    pub quit_requested: bool,
    pub current_save: Option<SaveFileMeta>,
}

#[derive(Serialize, Deserialize, Resource, Debug)]
pub struct TagState {
    pub shown: bool,
    pub members: Vec<String>,
}

impl Default for TagState {
    fn default() -> Self {
        Self {
            shown: false,
            members: Vec::new(),
        }
    }
}

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

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct MainMenuSet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumMenuSet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct SettingsMenuSet;

impl Plugin for MenuPlugin {
    fn build (&self, app: &mut App) {
        app
            .insert_state(MenuState::Home)
            .configure_sets(Update, (
                MainMenuSet.run_if(in_state(AppState::MainMenu).and(in_state(MenuState::Home))),
                PlanetariumMenuSet.run_if(in_state(AppState::MainMenu).and(in_state(MenuState::Planetarium))),
                SettingsMenuSet.run_if(in_state(AppState::MainMenu).and(in_state(MenuState::Settings))),
                ))
            .init_resource::<UiState>()
            .init_resource::<PlanetariumFiles>()
            .add_systems(OnEnter(MenuState::Planetarium), load_planetarium_files)
            .add_systems(OnEnter(AppState::MainMenu), load_planetarium_files.run_if(in_state(MenuState::Planetarium)))
            .add_systems(Update, (
                (main_menu,).in_set(MainMenuSet),
                (save_load::planetarium_menu,).in_set(PlanetariumMenuSet),
                (settings_menu,).in_set(SettingsMenuSet),
                quit_system,
            ))
        ;
    }
}

pub fn main_menu(
    mut contexts: EguiContexts,
    settings: Res<Settings>,
    mut ui_state: ResMut<UiState>,
    mut next_menu: ResMut<NextState<MenuState>>,
) {
    let ctx = contexts.ctx_mut();
    
    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.heading("Exotic Matters");
            
            // Add some spacing after the heading
            ui.add_space(20.0);

            let button_width = 200.0;
            let button_height = 40.0;

            if ui.add_sized([button_width, button_height], egui::Button::new("Planetarium")).clicked() {
                next_menu.set(MenuState::Planetarium)
            }
            
            // Add spacing between buttons
            ui.add_space(10.0);

            if ui.add_sized([button_width, button_height], egui::Button::new("Settings")).clicked() {
                next_menu.set(MenuState::Settings)
            }
            
            ui.add_space(10.0);

            if ui.add_sized([button_width, button_height], egui::Button::new("Quit")).clicked() {
                ui_state.quit_requested = true;
            }
        });
    });
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
    info!("{}, {}", files.templates.len(), files.saves.len());
}

pub fn settings_menu(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    mut menu_state: Res<State<MenuState>>,
    mut next_menu: ResMut<NextState<MenuState>>,
    mut next_app_state: ResMut<NextState<AppState>>
) {
    let ctx = contexts.ctx_mut();
    
    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        if ui.button("Back").clicked() {
            next_menu.set(MenuState::Home)
        }

        ui.vertical_centered(|ui| {
            ui.heading("Settings");

            ui.separator();

            settings::settings_panel(&mut settings, ui);
        });
    });
}

pub fn quit_system (
    ui_state: Res<UiState>,
    mut exit: EventWriter<AppExit>
) {
    if ui_state.quit_requested {
        exit.send(AppExit::Success);
    }
}

/// Copied and modified from https://docs.rs/bevy_window/0.15.3/src/bevy_window/system.rs.html#42-58
pub fn close_when_requested(
    mut commands: Commands,
    mut closed: EventReader<WindowCloseRequested>,
    closing: Query<Entity, With<ClosingWindow>>,
    settings: Res<Settings>,
) {
    // This was inserted by us on the last frame so now we can despawn the window
    for window in closing.iter() {
        commands.entity(window).despawn();
    }
    // Mark the window as closing so we can despawn it on the next frame
    for event in closed.read() {
        // When spamming the window close button on windows (other platforms too probably)
        // we may receive a `WindowCloseRequested` for a window we've just despawned in the above
        // loop.
        commands.entity(event.window).try_insert(ClosingWindow);
    }

    let _ = fs::write("data/settings.toml", toml::to_string_pretty(settings.deref()).unwrap());
}
