use bevy::app::{App, Update};
use bevy::prelude::{in_state, Commands, IntoSystemSetConfigs, NextState, Plugin, Res, ResMut, Startup, SystemSet, Time};
use bevy::prelude::IntoSystemConfigs;
use bevy_egui::{egui, EguiContexts};
use crate::gui::app::AppState;
use crate::gui::menu::{MenuState, UiState};
use crate::gui::planetarium::time::SimTime;
use crate::gui::settings::Settings;

pub mod time;
mod display;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumUISet;

pub struct Planetarium;

impl Plugin for Planetarium {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SimTime>()
            .configure_sets(Update, (
                PlanetariumUISet.run_if(in_state(AppState::Planetarium)),
            ))
            .add_systems(Update, (
                (planetarium_ui, advance_time).in_set(PlanetariumUISet),
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
    mut time: ResMut<SimTime>,
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
            ui.horizontal(|ui| {
               if time.playing {
                   if ui.button("Pause").clicked() {
                       time.playing = false;
                   }
               } else {
                   if ui.button("Play").clicked() {
                       time.playing = true;
                   }
               }
                ui.label(format!("Time: {:.1}s", time.time))
            });
            ui.horizontal(|ui| {
                ui.label("Simulation speed");
                ui.add(egui::DragValue::new(&mut time.gui_speed)
                    .speed(0.1)
                    .range(-100.0..=100.0)
                    .fixed_decimals(1)
                );
            });

            ui.separator();
    });
}

fn advance_time(mut sim_time: ResMut<SimTime>, time: Res<Time>) {
    if sim_time.playing {
        sim_time.previous_time = sim_time.time;
        sim_time.time += sim_time.gui_speed * time.delta_secs_f64();
    }
}
