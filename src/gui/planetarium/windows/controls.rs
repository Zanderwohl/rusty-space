use bevy_egui::{egui, EguiContexts};
use bevy::prelude::*;
use bevy_egui::egui::Ui;
use num_traits::Pow;
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
            planetarium_controls(next_app_state, next_menu_state, &mut time, ui, &mut ui_state, view_settings);
    });
}

pub fn planetarium_controls(
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    time: &mut ResMut<SimTime>,
    ui: &mut Ui,
    ui_state: &mut ResMut<UiState>,
    mut view_settings: ResMut<ViewSettings>,
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
}
