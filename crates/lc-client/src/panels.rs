//! egui drawing. These systems read [`UiState`] and emit [`Action`]s; they change nothing.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use em_spectra::presets;

use crate::action::{Action, MAX_ACCEL_G, MIN_ACCEL_G};
use crate::flight::JULIAN_YEAR_S;
use crate::app::{Game, Ui};
use crate::hud;
use crate::input::Requested;
use crate::plot::CurvePlot;
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
        ui.horizontal(|ui| {
            ui.weak(&lines.ship_clock);
            if let Some(flight) = &lines.flight {
                ui.separator();
                ui.colored_label(egui::Color32::from_rgb(130, 200, 250), flight);
            }
        });
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
    mut curve: Local<CurvePlot>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };
    for panel in ui_state.open_panels().to_vec() {
        let mut open = true;
        egui::Window::new(panel.title()).open(&mut open).show(ctx, |ui| match panel {
            Panel::Escape => escape(ui, &mut out),
            Panel::Settings => settings(ui, &ui_state),
            Panel::Debug => debug(ui, &ui_state, &game, &mut out),
            Panel::Telescope => telescope(ui, &ui_state, &mut game, &mut out, &mut curve),
            Panel::System => system(ui, &ui_state, &game),
            Panel::Flight => flight(ui, &ui_state, &game, &mut out),
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
    ui.label(crate::ui::rate_label(state.time_rate));
    for (rate, name) in crate::ui::RATE_LADDER {
        if ui.selectable_label(state.time_rate == rate, name).clicked() {
            ask(out, Action::SetTimeRate(rate));
        }
    }
    ui.weak("or , and . while flying");
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

fn telescope(
    ui: &mut egui::Ui,
    state: &Ui,
    game: &mut Game,
    out: &mut MessageWriter<Requested>,
    plot: &mut CurvePlot,
) {
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
            let distance = game.distance_to(star);
            if ui
                .selectable_label(state.selected == Some(star.id), format!("{label} — {distance:.2} ly"))
                .clicked()
            {
                ask(out, Action::SelectTarget(Some(star.id)));
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Look at").clicked() {
            ask(out, Action::LookAtSelected);
        }
        if ui.button("Fly there").clicked() {
            ask(out, Action::FlyTo(None));
        }
    });
    ui.separator();

    ui.horizontal(|ui| {
        ui.label(format!("integration: {:.0} s", state.integration_s));
        ui.separator();
        ui.label(format!("{} samples", game.curve.len()));
        if game.curve.len() > 1 {
            ui.separator();
            ui.label(format!("+/- {:.1e}", game.curve.uncertainty()));
        }
    });

    let band = game.curve.band();
    ui.horizontal(|ui| {
        ui.label("curve");
        for b in em_spectra::Band::ALL {
            // A band the sensor cannot reach is shown as unavailable rather than omitted, so
            // the instrument's limits are visible instead of merely being enforced.
            if !game.telescope.sees(b) {
                ui.weak(format!("{b:?}"));
                continue;
            }
            if ui.selectable_label(band == b, format!("{b:?}")).clicked() {
                ask(out, Action::SetCurveBand(b));
            }
        }
    });
    let width = ui.available_width().max(220.0);
    let samples = game.curve.samples().to_vec();
    plot.show(ui, &samples, egui::vec2(width, 190.0));
    ui.weak(crate::plot::caption(band));

    if let Some((_, last)) = samples.last() {
        // The picture is the readout, but a number is the one form of it that survives being
        // read aloud, screenshotted, or looked at by someone who cannot see the colour.
        let (label, value) = if *last >= 0.0 {
            ("deficit", *last)
        } else {
            ("excess", -*last)
        };
        ui.label(format!("last: {label} {value:.4e}   deepest dip {:.3e}", game.curve.deepest()));
    }
}

fn flight(ui: &mut egui::Ui, state: &Ui, game: &Game, out: &mut MessageWriter<Requested>) {
    ui.label(format!("drive: {:.0} g, cap {:.3}c", game.drive.accel_g, game.drive.max_beta));
    ui.horizontal(|ui| {
        for g in [1.0, 5.0, 20.0, 100.0] {
            if ui.button(format!("{g:.0} g")).clicked() {
                ask(out, Action::SetDriveAccel(g));
            }
        }
    });
    ui.weak(format!("between {MIN_ACCEL_G} and {MAX_ACCEL_G} g"));
    ui.separator();

    match &game.cruise {
        Some(cruise) => {
            let now = game.coordinate_time_s();
            let state = cruise.at(now);
            ui.add(egui::ProgressBar::new(cruise.progress(now) as f32).show_percentage());
            ui.label(format!("{:?}", state.phase));
            ui.label(format!("speed: {:.6}c", state.beta.length()));
            ui.label(format!("peak: {:.6}c", cruise.peak_beta()));
            ui.label(format!(
                "crossing: {:.2} years, {:.2} aboard",
                cruise.duration_s() / JULIAN_YEAR_S,
                cruise.proper_duration_s() / JULIAN_YEAR_S
            ));
            if ui.button("Cut the drive").clicked() {
                ask(out, Action::AbortFlight);
            }
        }
        None => {
            ui.label("At rest.");
            match state.selected.and_then(|id| game.star(id)) {
                Some(star) => {
                    let name = star.name.clone().unwrap_or_else(|| "unnamed".into());
                    ui.label(format!("{name} — {:.2} ly", game.distance_to(star)));
                    if ui.button("Fly there").clicked() {
                        ask(out, Action::FlyTo(None));
                    }
                }
                None => {
                    ui.label("Select a target in the telescope panel first.");
                }
            }
        }
    }
    ui.separator();
    let p = game.position_ly;
    ui.weak(format!("at {:.3}, {:.3}, {:.3} ly", p.x, p.y, p.z));
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
    ui.label(format!("{:.2} ly", game.distance_to(star)));
    ui.label(format!("{:.0} K", star.star.teff_k));
    ui.label(format!("{:.3} solar luminosities", star.luminosity_solar));
    ui.label(format!("[Fe/H] {:+.2}", star.metallicity));
    ui.separator();
    ui.weak("Everything here is the retarded state: what left the system, not what is there.");
}
