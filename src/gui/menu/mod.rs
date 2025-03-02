use bevy::app::AppExit;
use bevy::prelude::{in_state, App, AppExtStates, Condition, EventWriter, IntoSystemConfigs, NextState, Plugin, Res, ResMut, Resource, State, States, SystemSet, Update};
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
    Planetarium,
}

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct MainMenuSet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumMenuSet;

impl Plugin for MenuPlugin {
    fn build (&self, app: &mut App) {
        app
            .insert_state(MenuState::Home)
            .configure_sets(Update, (
                MainMenuSet.run_if(in_state(AppState::MainMenu).and(in_state(MenuState::Home))),
                PlanetariumMenuSet.run_if(in_state(AppState::MainMenu).and(in_state(MenuState::Planetarium))),
                ))
            .init_resource::<UiState>()
            .add_systems(Update, (
                (main_menu,).in_set(MainMenuSet),
                (planetarium_menu,).in_set(PlanetariumMenuSet),
                quit_system,
            ))
        ;
    }
}

pub fn main_menu(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    mut menu_state: Res<State<MenuState>>,
    mut next_menu: ResMut<NextState<MenuState>>,
) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.heading("Exotic Matters");

            if ui.button("Planetarium").clicked() {
                next_menu.set(MenuState::Planetarium)
            }

            if ui.button("Quit").clicked() {
                ui_state.quit_requested = true;
            }
        });
    });
}

pub fn planetarium_menu(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    mut menu_state: Res<State<MenuState>>,
    mut next_menu: ResMut<NextState<MenuState>>,
    mut next_app_state: ResMut<NextState<AppState>>
) {
    let ctx = contexts.ctx_mut();

    egui::CentralPanel::default().show(ctx, |ui| {
        if ui.button("Back").clicked() {
            next_menu.set(MenuState::Home)
        }

        if ui.button("Planetarium").clicked() {
            next_app_state.set(AppState::Planetarium)
        }

        ui.vertical_centered(|ui| {
            ui.heading("Planetarium Select");
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.heading("Create from Template");
                    ui.allocate_space(ui.available_size() / 2.0);
                    egui::ScrollArea::vertical()
                        .id_salt("planetarium-template-list")
                        .auto_shrink([true, false])
                        .show(ui, |ui| {
                        })
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.heading("Load from File");
                    ui.allocate_space(ui.available_size());
                    egui::ScrollArea::vertical()
                        .id_salt("planetarium-save-list")
                        .auto_shrink([true, false])
                        .show(ui, |ui| {
                        })
                });
            })
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
