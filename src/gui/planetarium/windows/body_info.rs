use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use crate::body::appearance::Appearance;
use crate::body::universe::Universe;
use crate::gui::menu::UiState;
use crate::camera::GoTo;
use crate::gui::settings::{Settings, UiTheme};

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

use crate::sim::world::{BodyEntities, SimSystem};
use em_sim::motive::MotiveSelection;
use em_sim::body::BodyInfo;
pub fn body_info_window(
    mut settings: ResMut<Settings>,
    universe: Res<Universe>,
    system: Res<SimSystem>,
    entities: Res<BodyEntities>,
    mut contexts: EguiContexts,
    mut body_info_state: ResMut<BodyInfoState>,
    mut go_to: MessageWriter<GoTo>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }
    if !settings.windows.body_info {
        return;
    }

    egui::Window::new("Body Info")
        .vscroll(true)
        .show(ctx, |ui| {
            let mut body_options: Vec<(String, String)> = universe.id_to_name_iter()
                .map(|(id, name)| (name.clone(), id.clone()))
                .collect();
            body_options.sort_by(|a, b| a.0.cmp(&b.0));
            body_select_dropdown(universe, &mut body_info_state, ui, body_options);

            let Some(selected) = body_info_state.current_body_id.as_ref() else {
                ui.label("No body selected.");
                return;
            };
            let Some(index) = system.0.by_name(selected) else {
                ui.label("That body is not in the current system.");
                return;
            };

            let id = system.0.id(index);
            if let Some(&entity) = entities.map.get(&id) {
                if ui.button("Go to").clicked() {
                    go_to.write(GoTo { entity });
                }
            }

            body_info_section(ui, system.0.info(index));
            ui.separator();
            state_section(ui, &system.0, index);
        });
}

/// Position, velocity and the motive currently in force.
///
/// All of this is read from the arena rather than from components, so what the panel
/// shows is by construction what the simulation is using.
fn state_section(ui: &mut Ui, system: &em_sim::system::System, index: em_sim::id::BodyIndex) {
    use crate::util::format::sci_not;
    ui.label("Current State");

    let position = system.position(index);
    let velocity = system.velocity(index);
    ui.horizontal(|ui| {
        ui.label("Position:");
        ui.label(format!("{}, {}, {} m", sci_not(position.x), sci_not(position.y), sci_not(position.z)));
    });
    ui.horizontal(|ui| {
        ui.label("Speed:");
        ui.label(format!("{} m/s", sci_not(velocity.length())));
    });
    if let Some(parent) = system.parent(index) {
        ui.horizontal(|ui| {
            ui.label("Orbiting:");
            ui.label(system.info(parent).display_name());
        });
        if let Some(local) = system.local_position(index) {
            ui.horizontal(|ui| {
                ui.label("Distance:");
                ui.label(format!("{} m", sci_not(local.length())));
            });
        }
    }

    let (_, selection) = system.motive(index).motive_at(system.time());
    ui.separator();
    match selection {
        MotiveSelection::Fixed { .. } => { ui.label("Fixed Body"); }
        MotiveSelection::Newtonian { .. } => { ui.label("Newtonian Body"); }
        MotiveSelection::Keplerian(kepler) => {
            ui.label("Keplerian Body");
            let mu = system.mu(index);
            for (label, value) in [
                ("Semi-major axis", format!("{} m", sci_not(kepler.semi_major_axis()))),
                ("Eccentricity", format!("{:.6}", kepler.eccentricity())),
                ("Inclination", format!("{:.4} deg", kepler.inclination())),
                ("Periapsis", format!("{} m", sci_not(kepler.periapsis()))),
                ("Apoapsis", kepler.apoapsis().map(|a| format!("{} m", sci_not(a)))
                    .unwrap_or_else(|| "open orbit".into())),
                ("Period", format!("{:.4} d", kepler.period(mu).to_days())),
            ] {
                ui.horizontal(|ui| {
                    ui.label(format!("{label}:"));
                    ui.label(value);
                });
            }
        }
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

pub(crate) fn body_select_dropdown(universe: Res<Universe>, mut body_info_state: &mut ResMut<BodyInfoState>, ui: &mut Ui, mut body_options: Vec<(String, String)>) {
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
