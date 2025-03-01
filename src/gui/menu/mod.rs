use bevy::app::AppExit;
use bevy::prelude::{in_state, App, AppExtStates, EventWriter, IntoSystemConfigs, Plugin, Res, ResMut, Resource, States, SystemSet, Update};
use bevy::prelude::IntoSystemSetConfigs;
use bevy_egui::{egui, EguiContexts};
use crate::gui::app::AppState;
use crate::gui::settings::Settings;

#[derive(Resource)]
pub struct UiState {
    quit_requested: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            quit_requested: false,
        }
    }
}

pub struct MenuPlugin;

#[derive(States, Debug, Clone, PartialEq, Eq, Hash)]
pub enum MenuState {
    Home,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct MainMenuSet;

impl Plugin for MenuPlugin {
    fn build (&self, app: &mut App) {
        app
            .insert_state(MenuState::Home)
            .configure_sets(Update, (
                MainMenuSet.run_if(in_state(AppState::MainMenu)),
                ))
            .init_resource::<UiState>()
            .add_systems(Update, (
                (main_menu,).in_set(MainMenuSet),
                quit_system,
            ))
        ;
    }
}

pub fn main_menu(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,

) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.heading("Exotic Matters");

            if ui.button("Quit").clicked() {
                ui_state.quit_requested = true;
            }
        });
    });
}

pub fn quit_system(
    ui_state: Res<UiState>,
    mut exit: EventWriter<AppExit>
) {
    if ui_state.quit_requested {
        exit.send(AppExit::Success);
    }
}
