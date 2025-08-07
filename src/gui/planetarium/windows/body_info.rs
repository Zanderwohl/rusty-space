use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use crate::body::appearance::Appearance;
use crate::body::motive::fixed_motive::FixedMotive;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::universe::Universe;
use crate::gui::menu::UiState;
use crate::gui::planetarium::camera::GoTo;
use crate::gui::settings::{Settings, UiTheme};
use crate::util::bevystuff::GlamVec;

#[derive(Resource)]
pub struct BodyInfoState {
    pub current_body_id: Option<String>,
}

impl Default for BodyInfoState {
    fn default() -> Self {
        Self {
            current_body_id: None,
        }
    }
}

pub fn body_info_window(
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
    universe: Res<Universe>,
    bodies: Query<(Entity, &BodyInfo, &BodyState, Option<&FixedMotive>, Option<&KeplerMotive>, Option<&NewtonMotive>)>,
    mut contexts: EguiContexts,
    mut body_info_state: ResMut<BodyInfoState>,
    mut go_to: EventWriter<GoTo>,
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
                // Create a sorted list of body names and their IDs
                let mut body_options: Vec<(String, String)> = universe.id_to_name_iter()
                    .map(|(id, name)| (name.clone(), id.clone()))
                    .collect();
                body_options.sort_by(|a, b| a.0.cmp(&b.0));

                body_select_dropdown(universe, &mut body_info_state, ui, body_options);

                
                // Get the body using the BodyInfo.id from bodies query
                let selected_body = bodies.iter().filter(|(e, info, state, fixed_motive, kepler_motive, newton_motive)| {
                    if body_info_state.current_body_id.is_none() { return false; }
                    <std::string::String as AsRef<str>>::as_ref(&info.id) == body_info_state.current_body_id.as_ref().unwrap()
                }).collect::<Vec<_>>();

                let selected_body = selected_body.get(0);
                match selected_body {
                    Some((e, info, state, fixed_motive, kepler_motive, newton_motive)) => {
                        if ui.button("Go to").clicked() {
                            go_to.write(GoTo {
                                entity: e.entity(),
                            });
                        }

                        display_body_info(ui, info, state, *fixed_motive, *kepler_motive, *newton_motive)
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

fn body_state_section(ui: &mut Ui, state: &BodyState) {
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

fn body_select_dropdown(universe: Res<Universe>, mut body_info_state: &mut ResMut<BodyInfoState>, ui: &mut Ui, mut body_options: Vec<(String, String)>) {
    egui::ComboBox::from_label("Body")
        .selected_text(
            body_info_state.current_body_id
                .as_ref()
                .and_then(|id| universe.get_by_id(id))
                .map(|name| name.clone())
                .unwrap_or_else(|| "Choose a body".to_string())
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut body_info_state.current_body_id,
                None,
                "Choose a body"
            );

            for (name, id) in body_options {
                ui.selectable_value(
                    &mut body_info_state.current_body_id,
                    Some(id.clone()),
                    name
                );
            }
        });
}
