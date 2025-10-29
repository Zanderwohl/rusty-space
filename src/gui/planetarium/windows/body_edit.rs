use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use crate::body::motive::fixed_motive::FixedMotive;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::universe::Universe;
use crate::gui::common;
use crate::gui::menu::UiState;
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::gui::settings::{Settings, UiTheme};
pub fn body_edit_window(
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    universe: Res<Universe>,
    mut contexts: EguiContexts,
    mut body_info_state: ResMut<BodyInfoState>,
    mut bodies: Query<(Entity, &mut BodyInfo, &BodyState, Option<&mut FixedMotive>, Option<&mut KeplerMotive>, Option<&mut NewtonMotive>)>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }
    
    if settings.windows.body_edit {
        egui::Window::new("Body Edit")
            .vscroll(true)
            .show(ctx, |ui| {
                let mut body_options: Vec<(String, String)> = universe.id_to_name_iter()
                    .map(|(id, name)| (name.clone(), id.clone()))
                    .collect();
                body_options.sort_by(|a, b| a.0.cmp(&b.0));
                crate::gui::planetarium::windows::body_info::body_select_dropdown(universe, &mut body_info_state, ui, body_options);

                let mut selected_body = bodies.iter_mut().filter(|(e, info, state, fixed_motive, kepler_motive, newton_motive)| {
                    if body_info_state.current_body_id.is_none() { return false; }
                    <std::string::String as AsRef<str>>::as_ref(&info.id) == body_info_state.current_body_id.as_ref().unwrap()
                }).collect::<Vec<_>>();

                let selected_body = selected_body.get_mut(0);
                match selected_body {
                    None => { ui.label("No body Selected"); },
                    Some((entity, info, state, fixed_motive, kepler_motive, newton_motive)) => {
                        body_info_section(ui, info);
                        if let Some(fixed_motive) = fixed_motive.as_mut() {
                            fixed_motive_section(ui, fixed_motive.as_mut())
                        }
                        if let Some(kepler_motive) = kepler_motive.as_mut() {
                            kepler_motive_section(ui, kepler_motive.as_mut())
                        }
                        if let Some(newton_motive) = newton_motive.as_mut() {
                            newton_motive_section(ui, newton_motive.as_mut())
                        }
                    }
                }
            });
    }
}

fn body_info_section(ui: &mut egui::Ui, info: &mut BodyInfo) {
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.label(info.display_name());
    });

    let mass = &mut info.mass;
    ui.horizontal(|ui| {
        ui.label("Mass:");
        common::stepper(ui, "", mass);
        ui.label("kg");
    });
}

fn fixed_motive_section(ui: &mut egui::Ui, motive: &mut FixedMotive) {
    ui.heading("Fixed Position");
    ui.vertical(|ui| {
        let x = &mut motive.position.x;
        ui.horizontal(|ui| {
            common::stepper(ui, "x", x);
            ui.label("m");
        });
        let y = &mut motive.position.y;
        ui.horizontal(|ui| {
            common::stepper(ui, "y", y);
            ui.label("m");
        });
        let z = &mut motive.position.z;
        ui.horizontal(|ui| {
            common::stepper(ui, "z", z);
            ui.label("m");
        });
    });
}

fn kepler_motive_section(ui: &mut egui::Ui, motive: &mut KeplerMotive) {
    ui.heading("Keplerian Body");
}

fn newton_motive_section(ui: &mut egui::Ui, motive: &mut NewtonMotive) {
    ui.heading("Newtonian Body");

    ui.heading("Position");
    ui.horizontal(|ui| {
        common::stepper(ui, "x", &mut motive.position.x);
        ui.label("m");
    });
    ui.horizontal(|ui| {
        common::stepper(ui, "y", &mut motive.position.y);
        ui.label("m");
    });
    ui.horizontal(|ui| {
        common::stepper(ui, "z", &mut motive.position.z);
        ui.label("m");
    });

    ui.heading("Velocity");
    ui.horizontal(|ui| {
        common::stepper(ui, "x", &mut motive.velocity.x);
        ui.label("m/s");
    });
    ui.horizontal(|ui| {
        common::stepper(ui, "y", &mut motive.velocity.y);
        ui.label("m/s");
    });
    ui.horizontal(|ui| {
        common::stepper(ui, "z", &mut motive.velocity.z);
        ui.label("m/s");
    });
}
