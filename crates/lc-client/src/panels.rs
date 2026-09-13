//! egui drawing. These systems read [`UiState`] and emit [`Action`]s; they change nothing.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use em_spectra::presets;

use crate::action::Action;
use crate::app::{Game, Ui};
use crate::hud;
use crate::input::Requested;
use crate::ui::{MenuPage, Panel};

fn ask(out: &mut MessageWriter<Requested>, action: Action) {
    out.write(Requested(action));
}

pub fn main_menu(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    mut out: MessageWriter<Requested>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(80.0);
            ui.heading("LIGHTCONE");
            ui.label("everything you see already happened");
            ui.add_space(40.0);
            match ui_state.menu_page {
                MenuPage::Root => {
                    if ui.button("Observe").clicked() {
                        ask(&mut out, Action::StartGame);
                    }
                    if ui.button("Settings").clicked() {
                        ask(&mut out, Action::GoToMenuPage(MenuPage::Settings));
                    }
                    if ui.button("Quit").clicked() {
                        ask(&mut out, Action::Quit);
                    }
                }
                _ => {
                    ui.label(format!("{:?}", ui_state.menu_page));
                    if ui.button("Back").clicked() {
                        ask(&mut out, Action::GoToMenuPage(MenuPage::Root));
                    }
                }
            }
        });
    });
}

pub fn loading(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(120.0);
            ui.heading("Loading the sky");
            ui.spinner();
        });
    });
}

/// The always-visible readout. Never in a closable panel: it is the premise.
pub fn hud(mut contexts: EguiContexts, ui_state: Res<Ui>, game: Res<Game>) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let lines = hud::lines(&game.0, &ui_state.0);

    egui::TopBottomPanel::top("hud").show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.strong(&lines.clock);
            ui.separator();
            ui.label(format!("BAND {}", lines.mapping));
            ui.separator();
            ui.label(format!("EXPOSURE {}", lines.exposure));
            if let Some(warning) = &lines.warning {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(240, 170, 60), warning);
            }
        });
        if let Some(target) = &lines.target {
            ui.colored_label(egui::Color32::from_rgb(240, 190, 110), target);
        }
    });

    if !ui_state.notifications.is_empty() {
        egui::Window::new("notifications")
            .title_bar(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-12.0, -12.0])
            .resizable(false)
            .show(ctx, |ui| {
                for note in &ui_state.notifications {
                    ui.label(&note.text);
                }
            });
    }
}

pub fn open_panels(
    mut contexts: EguiContexts,
    ui_state: Res<Ui>,
    mut game: ResMut<Game>,
    mut out: MessageWriter<Requested>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    for panel in ui_state.open_panels().to_vec() {
        let mut open = true;
        egui::Window::new(panel.title()).open(&mut open).show(ctx, |ui| match panel {
            Panel::Escape => escape(ui, &mut out),
            Panel::Settings => settings(ui, &ui_state),
            Panel::Debug => debug(ui, &ui_state, &game, &mut out),
            Panel::Telescope => telescope(ui, &ui_state, &mut game, &mut out),
            Panel::System => system(ui, &ui_state, &game),
        });
        if !open {
            ask(&mut out, Action::ClosePanel(panel));
        }
    }
}

fn escape(ui: &mut egui::Ui, out: &mut MessageWriter<Requested>) {
    // Nothing behind this has stopped, so it does not say "paused".
    ui.label("The clock is still running.");
    ui.separator();
    if ui.button("Settings").clicked() {
        ask(out, Action::OpenPanel(Panel::Settings));
    }
    if ui.button("Quit").clicked() {
        ask(out, Action::Quit);
    }
}

fn settings(ui: &mut egui::Ui, state: &Ui) {
    ui.label("Display");
    ui.label(format!("tone window: {:.1} stops", state.0.exposure_offset.abs().max(2.5)));
    ui.separator();
    ui.label("Settings apply immediately; there is no resume to apply them on.");
}

fn debug(ui: &mut egui::Ui, state: &Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    ui.label(format!("stars: {}", game.stars.len()));
    ui.label(format!("curve samples: {}", game.curve.len()));
    ui.label(format!("coordinate time: {:.3} s", game.coordinate_time_s()));
    ui.separator();

    ui.label("clock rate (development only; the server owns this)");
    for rate in [0.0, 1.0, 60.0, 3600.0] {
        if ui.button(format!("{rate}x")).clicked() {
            ask(out, Action::SetTimeRate(rate));
        }
    }
    ui.separator();

    if cfg!(feature = "godview") {
        let mut god = state.god_view;
        if ui.checkbox(&mut god, "god view").changed() {
            ask(out, Action::ToggleGodView);
        }
    } else {
        ui.weak("god view is not compiled into this build");
    }
    if ui.button("write snapshot").clicked() {
        ask(out, Action::WriteSnapshot);
    }
}

fn telescope(ui: &mut egui::Ui, state: &Ui, game: &mut Game, out: &mut MessageWriter<Requested>) {
    ui.label("Band mapping");
    ui.horizontal(|ui| {
        for (i, (name, _)) in presets::all().iter().enumerate() {
            if ui.selectable_label(state.preset == i, *name).clicked() {
                ask(out, Action::SetBandPreset(i));
            }
        }
    });
    ui.separator();

    ui.label("Target");
    egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
        for star in game.stars.iter().take(40) {
            let label = star.name.clone().unwrap_or_else(|| format!("{:x}", star.id.get()));
            let distance = star.position_ly.length();
            if ui
                .selectable_label(state.selected == Some(star.id), format!("{label} — {distance:.1} ly"))
                .clicked()
            {
                ask(out, Action::SelectTarget(Some(star.id)));
            }
        }
    });
    ui.separator();

    ui.label(format!("integration: {:.0} s", state.integration_s));
    ui.label(format!("samples: {}", game.curve.len()));
    if game.curve.len() > 1 {
        ui.label(format!("deepest dip: {:.3e}", game.curve.deepest()));
        ui.label(format!("uncertainty: {:.3e}", game.curve.uncertainty()));
        // Colour is an information channel here, so the number is given as well.
        let samples = game.curve.samples();
        let last = samples.last().copied().unwrap_or((0.0, 0.0));
        ui.label(format!("last measurement: {:.4e}", last.1));
    }
}

fn system(ui: &mut egui::Ui, state: &Ui, game: &Game) {
    let Some(id) = state.selected else {
        ui.label("Nothing selected.");
        return;
    };
    let Some(star) = game.star(id) else {
        ui.label("That star is not loaded.");
        return;
    };
    ui.label(star.name.clone().unwrap_or_else(|| "unnamed".into()));
    ui.label(format!("{:.2} ly", star.position_ly.length()));
    ui.label(format!("{:.0} K", star.star.teff_k));
    ui.label(format!("{:.3} solar luminosities", star.luminosity_solar));
    ui.label(format!("[Fe/H] {:+.2}", star.metallicity));
    ui.separator();
    ui.weak("Everything here is the retarded state: what left the system, not what is there.");
}
