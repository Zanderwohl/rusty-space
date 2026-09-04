use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::universe::Universe;
use crate::camera::{GoTo, GoToSource};
use crate::gui::planetarium::focused_body::{body_select_dropdown, sorted_body_options};
use crate::gui::planetarium::FocusedBodyState;
use crate::gui::settings::{Settings, UiTheme};

pub fn body_info_window(
    settings: Res<Settings>,
    universe: Res<Universe>,
    bodies: Query<(Entity, &BodyInfo, &BodyState, Option<&FixedMotive>, Option<&KeplerMotive>, Option<&NewtonMotive>)>,
    mut contexts: EguiContexts,
    mut focused_body_state: ResMut<FocusedBodyState>,
    mut go_to: MessageWriter<GoTo>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    if settings.windows.body_info {
        egui::Window::new("Body Info")
            .vscroll(true)
            .show(ctx, |ui| {
                let body_options = sorted_body_options(&universe);
                body_select_dropdown(&universe, &mut focused_body_state, ui, &body_options);
                
                // Get the body using the BodyInfo.id from bodies query
                let selected_body = focused_body_state
                    .current_body_id
                    .as_deref()
                    .and_then(|selected_id| {
                        bodies
                            .iter()
                            .find(|(_, info, _, _, _, _)| info.id.as_str() == selected_id)
                    });
                match selected_body {
                    Some((e, info, state, fixed_motive, kepler_motive, newton_motive)) => {
                        if ui.button("Go to").clicked() {
                            go_to.write(GoTo {
                                entity: e.entity(),
                                frame: None,
                                source: GoToSource::UiButton,
                            });
                        }

                        display_body_info(ui, info, state, fixed_motive, kepler_motive, newton_motive)
                    }
                    None => {
                        ui.label("No body selected.");
                    }
                }
            });
    }
}

fn display_body_info (
    ui: &mut Ui, 
    info: &BodyInfo, 
    state: &BodyState, 
    fixed_motive: Option<&FixedMotive>, 
    kepler_motive: Option<&KeplerMotive>, 
    newton_motive: Option<&NewtonMotive>
) {
    body_info_section(ui, info);
    ui.separator();
    body_state_section(ui, state);
    if let Some(fixed_motive) = fixed_motive {
        ui.separator();
        fixed_motive_section(ui, fixed_motive);
    }
    if let Some(kepler_motive) = kepler_motive {
        ui.separator();
        kepler_motive_section(ui, kepler_motive);
    }
    if let Some(newton_motive) = newton_motive {
        ui.separator();
        newton_motive_section(ui, newton_motive);
    }
}

fn body_info_section(ui: &mut Ui, info: &BodyInfo) {
    ui.label("Body Info");

    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.label(info.display_name());
    });

    if let Some(designation) = &info.designation {
        ui.horizontal(|ui| {
            ui.label("Designation:");
            ui.label(designation);
        });
    }

    ui.horizontal(|ui| {
        ui.label("System ID:");
        ui.label(&info.id);
    });

    if !info.tags.is_empty() {
        ui.horizontal(|ui| {
            ui.label("Tags:");
            ui.label(info.tags.join(", "));
        });
    }

    ui.separator();
    ui.label("Physical Attributes");
    
    ui.horizontal(|ui| {
        ui.label("Mass:");
        ui.label(format!("{} kg", crate::util::format::sci_not(info.mass)));
    });
}

fn body_state_section(ui: &mut Ui, _state: &BodyState) {
    ui.label("Current State");
}

fn fixed_motive_section(ui: &mut Ui, motive: &FixedMotive) {
    ui.label("Fixed Body");
    motive.display(ui);
}

fn kepler_motive_section(ui: &mut Ui, motive: &KeplerMotive) {
    ui.label("Keplerian Body");
    motive.display(ui);
}

fn newton_motive_section(ui: &mut Ui, motive: &NewtonMotive) {
    ui.label("Newtonian Body");
    motive.display(ui);
}
