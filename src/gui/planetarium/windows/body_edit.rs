use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use crate::body::universe::Universe;
use crate::sim::world::SimSystem;
use em_sim::body::BodyInfo;
use em_sim::motive::MotiveSelection;
use em_sim::motive::kepler::{EccentricitySMA, KeplerEulerAngles, KeplerMotive, KeplerRotation, KeplerShape};
use crate::gui::common;
use crate::gui::menu::UiState;
use crate::sim::{BodySelection, CalculateTrajectory};
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::gui::settings::{Settings, UiTheme};
pub fn body_edit_window(
    mut settings: ResMut<Settings>,
    universe: Res<Universe>,
    mut contexts: EguiContexts,
    mut body_info_state: ResMut<BodyInfoState>,
    mut system: ResMut<SimSystem>,
    mut calc: MessageWriter<CalculateTrajectory>,
) {
    let Ok(ctx) = contexts.ctx_mut() else { return };

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }
    if !settings.windows.body_edit {
        return;
    }

    egui::Window::new("Body Edit")
        .vscroll(true)
        .show(ctx, |ui| {
            let mut body_options: Vec<(String, String)> = universe.id_to_name_iter()
                .map(|(id, name)| (name.clone(), id.clone()))
                .collect();
            body_options.sort_by(|a, b| a.0.cmp(&b.0));
            crate::gui::planetarium::windows::body_info::body_select_dropdown(
                universe, &mut body_info_state, ui, body_options);

            let Some(selected) = body_info_state.current_body_id.clone() else {
                ui.label("No body Selected");
                return;
            };
            let Some(index) = system.0.by_name(&selected) else {
                ui.label("That body is not in the current system.");
                return;
            };

            calc.write(CalculateTrajectory { selection: BodySelection::IDs(vec![selected]) });

            // Edits write straight into the arena, which is the only copy of this data.
            // `info_mut` and `motive_mut` mark derived state stale, so a changed mass or
            // primary is picked up on the next propagation rather than going unnoticed.
            body_info_section(ui, system.0.info_mut(index));

            let now = system.0.time();
            let Some(selection) = system.0.motive_mut(index).motive_at_mut(now) else { return };
            match selection {
                MotiveSelection::Fixed { position, .. } => {
                    ui.heading("Fixed Position");
                    ui.vertical(|ui| {
                        for (label, value) in [("x", &mut position.x), ("y", &mut position.y), ("z", &mut position.z)] {
                            ui.horizontal(|ui| {
                                common::stepper(ui, label, value);
                                ui.label("m");
                            });
                        }
                    });
                }
                MotiveSelection::Newtonian { position, velocity } => {
                    ui.heading("Newtonian Body");
                    ui.heading("Position");
                    for (label, value) in [("x", &mut position.x), ("y", &mut position.y), ("z", &mut position.z)] {
                        ui.horizontal(|ui| {
                            common::stepper(ui, label, value);
                            ui.label("m");
                        });
                    }
                    ui.heading("Velocity");
                    for (label, value) in [("x", &mut velocity.x), ("y", &mut velocity.y), ("z", &mut velocity.z)] {
                        ui.horizontal(|ui| {
                            common::stepper(ui, label, value);
                            ui.label("m/s");
                        });
                    }
                }
                MotiveSelection::Keplerian(kepler) => kepler_motive_section(ui, kepler),
            }
        });
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

fn kepler_motive_section(ui: &mut egui::Ui, motive: &mut KeplerMotive) {
    ui.heading("Keplerian Body");

    ui.vertical(|ui| {
        ui.heading("Shape");
        match &mut motive.shape {
            KeplerShape::EccentricitySMA(sma) => kepler_motive_shape_sma_section(ui, sma),
            KeplerShape::Apsides(_) => { ui.label("Apsides editing is not implemented."); }
        }
    });
    ui.separator();

    ui.vertical(|ui| {
        ui.heading("Rotation");
        match &mut motive.rotation {
            KeplerRotation::EulerAngles(ea) => kepler_motive_rotation_ea_section(ui, ea),
            KeplerRotation::FlatAngles(_) => { ui.label("Flat angle editing is not implemented."); }
            KeplerRotation::PrecessingEulerAngles(_) => { ui.label("Precessing angle editing is not implemented."); }
        }
    });
    ui.separator();

    ui.vertical(|ui| {
        ui.heading("Epoch");
    });
}

fn kepler_motive_shape_sma_section(ui: &mut egui::Ui, sma: &mut EccentricitySMA) {
    ui.horizontal(|ui| {
        common::stepper(ui, "Semi-Major Axis", &mut sma.semi_major_axis);
        ui.label("m");
    });

    ui.horizontal(|ui| {
        ui.label("Eccentricity");
        ui.add(egui::DragValue::new(&mut sma.eccentricity)
            .speed(0.05)
            .range(0.0..=2.0)
            .clamp_existing_to_range(false)
            .fixed_decimals(1)
        );
    });

    ui.horizontal(|ui| {
       if ui.button("Circular").clicked() {
           sma.eccentricity = 0.0;
       }
        if ui.button("Escape").clicked() {
            sma.eccentricity = 1.0;
        }
    });
}

fn kepler_motive_rotation_ea_section(ui: &mut Ui, kea: &mut KeplerEulerAngles) {
    ui.horizontal(|ui| {
        ui.label("Inclination");
        let mut inclination = kea.inclination;
        let before = inclination;
        ui.add(egui::DragValue::new(&mut inclination)
            .speed(0.1)
            .range(0.0..=360.0)
            .clamp_existing_to_range(false)
            .fixed_decimals(1)
        );
        if inclination != before {
            kea.inclination = inclination;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Longitude of Ascending Node");
        let mut longitude_of_ascending_node = kea.longitude_of_ascending_node;
        let before = longitude_of_ascending_node;
        ui.add(egui::DragValue::new(&mut longitude_of_ascending_node)
            .speed(0.1)
            .range(0.0..=360.0)
            .clamp_existing_to_range(false)
            .fixed_decimals(1)
        );
        if longitude_of_ascending_node != before {
            kea.longitude_of_ascending_node = longitude_of_ascending_node;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Argument of Periapsis");
        let mut argument_of_periapsis = kea.argument_of_periapsis;
        let before = argument_of_periapsis;
        ui.add(egui::DragValue::new(&mut argument_of_periapsis)
            .speed(0.1)
            .range(0.0..=360.0)
            .clamp_existing_to_range(false)
            .fixed_decimals(1)
        );
        if argument_of_periapsis != before {
            kea.argument_of_periapsis = argument_of_periapsis;
        }
    });
}

