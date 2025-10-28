use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use num_traits::Pow;
use crate::body::motive::fixed_motive::FixedMotive;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::universe::Universe;
use crate::gui::menu::UiState;
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::gui::settings::{Settings, UiTheme};
use crate::util::format;

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
        stepper(ui, "", mass);
        ui.label("kg");
    });
}

fn fixed_motive_section(ui: &mut egui::Ui, fixed_motive: &mut FixedMotive) {
    ui.heading("Fixed Position");
    ui.vertical(|ui| {
        let x = &mut fixed_motive.position.x;
        ui.horizontal(|ui| {
            stepper(ui, "x", x);
            ui.label("m");
        });
        let y = &mut fixed_motive.position.y;
        ui.horizontal(|ui| {
            stepper(ui, "y", y);
            ui.label("m");
        });
        let z = &mut fixed_motive.position.z;
        ui.horizontal(|ui| {
            stepper(ui, "z", z);
            ui.label("m");
        });
    });
}

fn stepper<S: AsRef<str>>(ui: &mut egui::Ui, label: S, mut value: &mut f64) {
    ui.horizontal(|ui| {
       ui.label(label.as_ref());
        if ui.button("<<").clicked() { *value /= 10.0; }
        if ui.button("<").clicked() { *value = bump_decimal(*value, -1.0); }
        ui.add(egui::DragValue::new(value)
            .speed(0.01)
            .range(f64::MIN..=f64::MAX)
            .fixed_decimals(1)
            .custom_formatter(|n, range| format::sci_not(n))
            .custom_parser(|s| format::sci_not_parser(s))
        );
        if ui.button(">").clicked() { *value = bump_decimal(*value, 1.0); }
        if ui.button(">>").clicked() { *value *= 10.0; }
    });
}

fn bump_decimal(x: f64, direction: f64) -> f64 {
    if x == 0.0 { return 0.0; }

    // Find order of magnitude (e.g. 3.4e5 → exp = 5)
    let exp = x.abs().log10().floor();
    // Find the multiplier that makes the number ~[1, 10)
    let scale = 10f64.powf(exp);
    let normalized = x / scale; // e.g. 3.4

    // Change the first digit after the decimal (i.e. ±0.1)
    let bumped = (normalized * 10.0 + direction).round() / 10.0;

    bumped * scale.copysign(x)
}
