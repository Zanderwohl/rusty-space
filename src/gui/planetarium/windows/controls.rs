use bevy_egui::{egui, EguiContexts};
use bevy::prelude::*;
use bevy_egui::egui::Ui;
use crate::sim::world::SimMetrics;
use crate::body::universe::save::ViewSettings;
use crate::foundations::time::JD_SECONDS_PER_JULIAN_DAY;
use crate::gui::common;
use crate::gui::menu::UiState;
use crate::sim::SimTime;
use crate::gui::settings::{Settings, UiTheme};
use crate::util::format;
use crate::util::format::format_duration;

pub fn control_window(
    mut contexts: EguiContexts,
    settings: Res<Settings>,
    ui_state: Res<UiState>,
    mut time: ResMut<SimTime>,
    view_settings: ResMut<ViewSettings>,
    perf_metrics: Res<SimMetrics>,
) {
    if !settings.windows.controls {
        return;
    }

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
            planetarium_controls(&mut time, ui, &ui_state, view_settings, &perf_metrics);
    });
}

pub fn planetarium_controls(
    time: &mut ResMut<SimTime>,
    ui: &mut Ui,
    ui_state: &UiState,
    mut view_settings: ResMut<ViewSettings>,
    perf_metrics: &SimMetrics,
) {
    ui.horizontal(|ui| {
        ui.label("File:");
        match &ui_state.current_save {
            None => { ui.label("New Universe"); },
            Some(file) => { ui.label(file.file_name.clone()); }
        }
    });
    ui.label("Press Esc for menu");
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
            ui.label(format!("Time: {}", format_duration(time.time.to_j2000_seconds().round() as i64)));
        }
    });
    let gui_speed_current = time.gui_speed;
    ui.horizontal(|ui| {
        if time.seconds_only {
            ui.label(format!("Simulation speed: {:.1}s / s", gui_speed_current));
        } else {
            ui.label(format!("Simulation speed: {} / s", format_duration(gui_speed_current.round() as i64)));
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
        ui.label("Selected");
        ui.checkbox(&mut view_settings.show_selected_labels, "");
        ui.checkbox(&mut view_settings.show_selected_trajectories, "");
    });

    ui.horizontal(|ui| {
        ui.label("All");
        ui.checkbox(&mut view_settings.show_labels, "");
        ui.checkbox(&mut view_settings.show_trajectories, "");
    });

    ui.horizontal(|ui| {
        ui.label("Spheres of Influence");
        ui.checkbox(&mut view_settings.show_spheres_of_influence, "");
        ui.checkbox(&mut view_settings.show_child_spheres_of_influence, "Children");
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
        ui.label(format!("Bodies: {}", perf_metrics.bodies));
        ui.label(format!("Integrated: {}", perf_metrics.newtonian_bodies));
        ui.separator();
        if perf_metrics.analytic_jump {
            // Nothing to integrate, so the system jumped straight to the target time.
            ui.label("Fully analytic - evaluated directly");
        } else {
            ui.label(format!("Step: {:.4}s", perf_metrics.step_size_seconds));
            ui.label(format!("Steps last frame: {}", perf_metrics.steps));
        }
        ui.label(format!("Propagation: {:.4} ms", perf_metrics.propagation_ms));

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
                            let seconds_per_day = JD_SECONDS_PER_JULIAN_DAY as u64;
                            let days = secs / seconds_per_day;
                            let day_secs = secs % seconds_per_day;
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
