use bevy::app::{App, Update};
use bevy::prelude::{in_state, Commands, IntoSystemSetConfigs, NextState, Plugin, ResMut, Startup, SystemSet};
use bevy::prelude::IntoSystemConfigs;
use bevy_egui::{egui, EguiContexts};
use crate::gui::app::AppState;
use crate::gui::menu::{MenuState, UiState};
use crate::gui::planetarium::time::Time;
use crate::gui::settings::Settings;

pub mod time;
mod display;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumUISet;

pub struct Planetarium;

impl Plugin for Planetarium {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<Time>()
            .configure_sets(Update, (
                PlanetariumUISet.run_if(in_state(AppState::Planetarium)),
            ))
            .add_systems(Update, (
                (planetarium_ui,).in_set(PlanetariumUISet),
            ))
        ;
    }
}

fn planetarium_ui(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    mut time: ResMut<Time>,
) {
    let ctx = contexts.ctx_mut();

    egui::Window::new("Controls")
        .vscroll(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Quit to Main Menu").clicked() {
                    next_app_state.set(AppState::MainMenu);
                    next_menu_state.set(MenuState::Planetarium);
                }

                ui.disable();
                ui.button("Save");
            });
            ui.separator();
            ui.label(format!("Time: {:.1}s", time.time))
    });
}
