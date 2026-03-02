use bevy_egui::{egui, EguiContexts};
use bevy::prelude::*;
use bevy_egui::egui::Ui;
use num_traits::Pow;
use crate::body::motive::calculate_body_positions::SimulationPerformanceMetrics;
use crate::body::universe::save::ViewSettings;
use crate::foundations::time::JD_SECONDS_PER_JULIAN_DAY;
use crate::gui::app::AppState;
use crate::gui::common;
use crate::gui::menu::{MenuState, UiState};
use crate::gui::planetarium::time::SimTime;
use crate::gui::settings::{Settings, UiTheme};
use crate::util::format;
use crate::util::format::seconds_to_naive_date;

pub fn control_window(
    mut contexts: EguiContexts,
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    next_app_state: ResMut<NextState<AppState>>,
    next_menu_state: ResMut<NextState<MenuState>>,
    mut time: ResMut<SimTime>,
    view_settings: ResMut<ViewSettings>,
    perf_metrics: Res<SimulationPerformanceMetrics>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();
    
    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    egui::Window::new("Controls")
        .vscroll(true)
        .show(ctx, |ui| {
            planetarium_controls(next_app_state, next_menu_state, &mut time, ui, &mut ui_state, view_settings, &perf_metrics);
    });
}

pub fn planetarium_controls(
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    time: &mut ResMut<SimTime>,
    ui: &mut Ui,
    ui_state: &mut ResMut<UiState>,
    mut view_settings: ResMut<ViewSettings>,
    perf_metrics: &SimulationPerformanceMetrics,
) {
    if ui.button("Quit to Main Menu").clicked() {
        // TODO: Some kind of save nag
        ui_state.current_save = None;
        next_app_state.set(AppState::MainMenu);
        next_menu_state.set(MenuState::Planetarium);
    }
    ui.horizontal(|ui| {
        match &ui_state.current_save {
            None => { ui.label("New Universe"); },
            Some(file) => { ui.label(file.file_name.clone()); }
        }

        ui.disable();
        let _ = ui.button("Save");
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
        if time.seconds_only {
            ui.label(format!("Time: {:.1}s", time.time.to_j2000_seconds()));
        } else {
            ui.label(format!("Time: {}", seconds_to_naive_date(time.time.to_j2000_seconds().round() as i64)));
        }
    });
    let gui_speed_current = time.gui_speed;
    let gui_speed_step = {
        let s = format!("{gui_speed_current:e}");
        let a = s.split("e").collect::<Vec<&str>>();
        let exponent = a[1].parse::<i64>().unwrap();
        let step = (10.0f64.pow(exponent as f64) / 10.0).abs();
        step
    };
    ui.horizontal(|ui| {
        if time.seconds_only {
            ui.label(format!("Simulation speed: {:.1}s / s", gui_speed_current));
        } else {
            ui.label(format!("Simulation speed: {} / s", seconds_to_naive_date(gui_speed_current.round() as i64)));
        }
    });
    common::stepper(ui, "", &mut time.gui_speed);
    ui.horizontal(|ui| {
        ui.checkbox(&mut time.seconds_only, "Display as seconds");
    });
    ui.horizontal(|ui| {
        if ui.button("1 year").clicked() { time.gui_speed = JD_SECONDS_PER_JULIAN_DAY * 365.2425; } // https://www.grc.nasa.gov/www/k-12/Numbers/Math/Mathematical_Thinking/calendar_calculations.htm
        if ui.button("1 day").clicked() { time.gui_speed = JD_SECONDS_PER_JULIAN_DAY; }
        if ui.button("1 hour").clicked() { time.gui_speed = 60.0 * 60.0; }
        if ui.button("1 minute").clicked() { time.gui_speed = 60.0; }
        if ui.button("1 second").clicked() { time.gui_speed = 1.0; }
    });

    ui.separator();

    // Scale controls
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Distance Scale");
        ui.checkbox(&mut view_settings.logarithmic_distance_scale, "Logarithmic");
        if ui.button("-").clicked() { view_settings.distance_scale /= 10.0 }

        ui.label(format::sci_not(view_settings.distance_scale));
        if ui.button("+").clicked() { view_settings.distance_scale *= 10.0 }
    });
    if view_settings.logarithmic_distance_scale {
        ui.add(egui::Slider::new(&mut view_settings.logarithmic_distance_base, 2.0..=30.0)
            .text("Logarithmic Base")
            .step_by(1.0)
        );
    }
    ui.horizontal(|ui| {
        ui.label("Body Scale");
        ui.checkbox(&mut view_settings.logarithmic_body_scale, "Logarithmic");
        if ui.button("-").clicked() { view_settings.body_scale /= 10.0 }
        ui.label(format::sci_not(view_settings.body_scale));
        if ui.button("+").clicked() { view_settings.body_scale *= 10.0 }
    });
    if view_settings.logarithmic_body_scale {
        ui.add(egui::Slider::new(&mut view_settings.logarithmic_body_base, 2.0..=1000.0)
            .text("Logarithmic Base")
            .step_by(1.0)
        );
    }

    // View settings
    ui.separator();
    ui.label("Show/Hide");

    ui.horizontal(|ui| {
        ui.label("All");
        ui.checkbox(&mut view_settings.show_labels, "");
        ui.checkbox(&mut view_settings.show_trajectories, "");
    });

    for (tag_name, tag_state) in &mut view_settings.tags {
        ui.horizontal(|ui| {
            ui.label(tag_name);
            ui.checkbox(&mut tag_state.shown, "");
            ui.checkbox(&mut tag_state.trajectory, "");

        });
    }

    // Simulation performance
    ui.separator();
    ui.collapsing("Simulation Performance", |ui| {
        let step = perf_metrics.step_size;
        let completed = perf_metrics.steps_completed;
        let intended = perf_metrics.steps_intended;
        let behind = completed < intended;
        let actual = completed as f64 * step;
        let target = intended as f64 * step;
        ui.label(format!("Step: {step:.4}s"));
        let sim_text = format!("Simulated: {actual:.4} / {target:.4}s");
        if behind {
            ui.colored_label(egui::Color32::RED, &sim_text);
        } else {
            ui.label(&sim_text);
        }
        ui.separator();

        // Graph rebuild info
        ui.label(format!("Last graph rebuild: {:.4} ms", perf_metrics.last_graph_rebuild_duration_ms));
        let rebuild_ago = perf_metrics.current_sim_time.to_j2000_seconds()
            - perf_metrics.last_graph_rebuild_sim_time.to_j2000_seconds();
        ui.label(format!("Rebuild sim-time ago: {rebuild_ago:.4} s"));

        ui.separator();

        // Steps completed / intended
        let steps_text = format!("Steps: {completed} / {intended}");
        if behind {
            ui.colored_label(egui::Color32::RED, steps_text);
        } else {
            ui.label(steps_text);
        }

        ui.label(format!("Avg time per step: {:.4} ms", perf_metrics.avg_time_per_step_ms));

        ui.separator();
        ui.label("Step timing (avg):");
        ui.indent("step_timing", |ui| {
            ui.label(format!("Hierarchical: {:.4} ms", perf_metrics.avg_hierarchical_ms));
            ui.label(format!("Cache update: {:.4} ms", perf_metrics.avg_cache_update_ms));
            ui.label(format!("Newtonian:    {:.4} ms", perf_metrics.avg_newtonian_ms));
        });

        ui.separator();
        if ui.button("Snapshot").clicked() {
            match toml::to_string_pretty(perf_metrics) {
                Ok(content) => {
                    let timestamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| {
                            let secs = d.as_secs();
                            let millis = d.subsec_millis();
                            // Break epoch seconds into date/time components
                            let days = secs / 86400;
                            let day_secs = secs % 86400;
                            let hours = day_secs / 3600;
                            let mins = (day_secs % 3600) / 60;
                            let s = day_secs % 60;
                            // Days since 1970-01-01 to Y-M-D (simplified)
                            let (y, m, d) = epoch_days_to_ymd(days as i64);
                            format!("{y:04}-{m:02}-{d:02}T{hours:02}-{mins:02}-{s:02}.{millis:03}")
                        })
                        .unwrap_or_else(|_| "unknown".to_string());
                    let dir = std::path::Path::new("logs/performance");
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        eprintln!("Failed to create {}: {e}", dir.display());
                    } else if let Err(e) = std::fs::write(dir.join(format!("{timestamp}.toml")), content) {
                        eprintln!("Failed to write snapshot: {e}");
                    }
                }
                Err(e) => eprintln!("Failed to serialize metrics: {e}"),
            }
        }
    });
}

/// Convert days since Unix epoch to (year, month, day).
fn epoch_days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    // Shift to March-based year to simplify leap year handling
    days += 719468; // days from 0000-03-01 to 1970-01-01
    let era = days.div_euclid(146097);
    let doe = days.rem_euclid(146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
