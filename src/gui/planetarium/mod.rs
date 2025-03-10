use bevy::app::{App, Update};
use bevy::prelude::{in_state, info, Commands, IntoSystemSetConfigs, NextState, Plugin, Res, ResMut, Startup, SystemSet, Time, OnExit};
use bevy::prelude::IntoSystemConfigs;
use bevy_egui::{egui, EguiContexts};
use lazy_static::lazy_static;
use num_traits::{FloatConst, Pow};
use regex::Regex;
use crate::gui::app::AppState;
use crate::gui::menu::{MenuState, UiState};
use crate::gui::planetarium::time::SimTime;
use crate::gui::settings::{Settings, UiTheme};
use crate::util::format::seconds_to_naive_date;
use crate::body::unload_simulation_objects;

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
            .add_systems(OnExit(AppState::Planetarium), unload_simulation_objects)
        ;
    }
}

lazy_static! {
    static ref SCI_RE: Regex = Regex::new(r"\d?\.\d+\s?x\s?10\s?\^\s?\d+").unwrap();
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
    
    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    egui::Window::new("Controls")
        .vscroll(true)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Quit to Main Menu").clicked() {
                    next_app_state.set(AppState::MainMenu);
                    next_menu_state.set(MenuState::Planetarium);
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
                if ui.button("<<").clicked() { time.gui_speed /= 10.0}
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
    });

    // Start collapsed: https://github.com/emilk/egui/pull/5661
    egui::Window::new("Settings")
        .vscroll(true)
        .show(ctx, |ui| {
            crate::gui::menu::settings::settings_panel(&mut settings, ui);
        });

    if settings.windows.spin {
        egui::Window::new("Spin Gravity Calculator")
            .vscroll(true)
            .show(ctx, |ui| {
                ui.add(egui::Slider::new(&mut settings.windows.spin_data.radius, 0.1..=250.0)
                    .text("Radius")
                    .step_by(0.1)
                );
                ui.add(egui::Slider::new(&mut settings.windows.spin_data.rpm, 0.0..=10.0)
                    .text("RPM")
                    .step_by(0.1)
                );
                let v = 2.0 * f64::PI() * settings.windows.spin_data.radius * settings.windows.spin_data.rpm / 60.0;
                let accel = v * v / settings.windows.spin_data.radius;
                ui.label(format!("Gravity: {:.2} m/s^2 ({:.2} g)", accel, accel / 9.81));
                ui.label(format!("Tangential Velocity: {:.2} m/s", v));

                ui.separator();
                ui.label("Coriolis Effect");
                ui.add(egui::Slider::new(&mut settings.windows.spin_data.vertical_velocity, -100.0..=100.0)
                    .text("Vertical Velocity (positive is inward)")
                    .step_by(0.1)
                );
                let omega = v / settings.windows.spin_data.radius;
                let coriolis = 2.0 * omega * settings.windows.spin_data.vertical_velocity;
                ui.label(format!("Coriolis Effect (positive is spinward): {:.2} m/s^2", coriolis));
            });
    }
}

fn advance_time(mut sim_time: ResMut<SimTime>, time: Res<Time>) {
    if sim_time.playing {
        sim_time.previous_time = sim_time.time;
        sim_time.time += sim_time.gui_speed * time.delta_secs_f64();
    }
}
