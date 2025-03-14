use bevy_egui::{egui, EguiContexts};
use bevy::prelude::{NextState, Res, ResMut};
use bevy_egui::egui::Ui;
use crate::body::universe::Universe;
use crate::gui::app::AppState;
use crate::gui::menu::{MenuState, PlanetariumFiles, SaveFileMeta, UiState};
use crate::gui::settings::{Settings, UiTheme};

pub fn planetarium_menu(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    mut next_menu: ResMut<NextState<MenuState>>,
    mut next_app_state: ResMut<NextState<AppState>>,
    files: Res<PlanetariumFiles>,
    mut universe: ResMut<Universe>,
) {
    let ctx = contexts.ctx_mut();

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    egui::CentralPanel::default().show(ctx, |ui| {
        // Top button bar
        ui.horizontal(|ui| {
            let button_height = 40.0;

            if ui.add_sized([120.0, button_height], egui::Button::new("Back")).clicked() {
                next_menu.set(MenuState::Home)
            }

            ui.add_space(10.0);

            if ui.add_sized([120.0, button_height], egui::Button::new("Planetarium")).clicked() {
                next_app_state.set(AppState::PlanetariumLoading)
            }
        });

        ui.add_space(20.0);

        ui.vertical_centered(|ui| {
            ui.heading("Planetarium Select");
            ui.add_space(20.0);
        });

        ui.columns(2, |columns| {
            // Left column - Templates
            egui::Frame::none()
                .fill(if ctx.style().visuals.dark_mode {
                    egui::Color32::from_gray(40)
                } else {
                    egui::Color32::from_gray(240)
                })
                .inner_margin(10.0)
                .show(&mut columns[0], |ui| {
                    ui.vertical(|ui| {
                        ui.heading("Create from Template");
                        ui.add_space(10.0);
                        egui::ScrollArea::vertical()
                            .id_salt("planetarium-template-list")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                display_saves_list(&files.templates, ui, "Create", &mut universe, &mut ui_state, &mut next_app_state);
                            });
                    });
                });

            // Right column - Saves
            egui::Frame::new()
                .fill(if ctx.style().visuals.dark_mode {
                    egui::Color32::from_gray(40)
                } else {
                    egui::Color32::from_gray(240)
                })
                .inner_margin(10.0)
                .show(&mut columns[1], |ui| {
                    ui.vertical(|ui| {
                        ui.heading("Load from File");
                        ui.add_space(10.0);
                        egui::ScrollArea::vertical()
                            .id_salt("planetarium-save-list")
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                display_saves_list(&files.saves, ui, "Load", &mut universe, &mut ui_state, &mut next_app_state);
                            });
                    });
                });
        });
    });
}

fn display_saves_list(
    saves: &Vec<SaveFileMeta>,
    ui: &mut Ui,
    load_label: &str,
    universe: &mut ResMut<Universe>,
    mut ui_state: &mut ResMut<UiState>,
    mut next_app_state: &mut ResMut<NextState<AppState>>,
) {
    for (idx, save) in saves.iter().enumerate() {
        // Card frame for each item
        egui::Frame::new()
            .fill(if ui.style().visuals.dark_mode {
                egui::Color32::from_gray(50)
            } else {
                egui::Color32::from_gray(250)
            })
            .inner_margin(egui::vec2(10.0, 8.0))
            .outer_margin(egui::vec2(0.0, 2.0))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Expand label to take available width
                    ui.add(egui::Label::new(&save.file_name))
                        .on_hover_text(&save.file_name);

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_sized([60.0, 24.0], egui::Button::new(load_label)).clicked() {
                            ui_state.current_save = Some((*save).clone());
                            next_app_state.set(AppState::PlanetariumLoading)
                        }
                    });
                });
            });

        if idx < saves.len() - 1 {
            ui.add_space(2.0);
        }
    }
}