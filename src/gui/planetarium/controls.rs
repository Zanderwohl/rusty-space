use bevy::prelude::{NextState, Res, ResMut};
use bevy_egui::egui;
use bevy_egui::egui::Ui;
use num_traits::Pow;
use crate::gui::app::AppState;
use crate::gui::menu::{MenuState, UiState};
use crate::gui::planetarium::SCI_RE;
use crate::gui::planetarium::time::SimTime;
use crate::util::format::seconds_to_naive_date;

pub fn planetarium_controls(
    mut next_app_state: ResMut<NextState<AppState>>,
    mut next_menu_state: ResMut<NextState<MenuState>>,
    mut time: &mut ResMut<SimTime>,
    ui: &mut Ui,
    mut ui_state: ResMut<UiState>,
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
            ui.label(format!("Time: {:.1}s", time.time));
        } else {
            ui.label(format!("Time: {}", seconds_to_naive_date(time.time.round() as i64)));
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
    ui.horizontal(|ui| {
        if ui.button("<<").clicked() { time.gui_speed /= 10.0 }
        if ui.button("<").clicked() { time.gui_speed -= gui_speed_step }
        ui.add(egui::DragValue::new(&mut time.gui_speed)
            .speed(gui_speed_step)
            .range(f64::MIN..=f64::MAX)
            .fixed_decimals(1)
            .custom_formatter(|n, range| {
                let s = format!("{n:e}");
                let a = s.split("e").collect::<Vec<&str>>();
                let mantissa = a[0].parse::<f64>().unwrap();
                let exponent = a[1].parse::<i64>().unwrap();
                format!("{:.3} x 10 ^ {}", mantissa, exponent)
            })
            .custom_parser(|s| {
                if !SCI_RE.is_match(s) {
                    return None;
                }
                let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
                let a = s.split("x").collect::<Vec<&str>>();
                let mantissa = a[0].parse::<f64>().ok()?;
                let b = a[1].split("^").collect::<Vec<&str>>();
                let exponent = b[1].parse::<i64>().ok()?;

                let result = mantissa * (10.0f64.pow(exponent as f64));

                return Some(result);
            })
        );
        if ui.button(">").clicked() { time.gui_speed += gui_speed_step }
        if ui.button(">>").clicked() { time.gui_speed *= 10.0 }
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut time.seconds_only, "Display as seconds");
    });

    ui.separator();
}