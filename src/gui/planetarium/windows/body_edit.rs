use bevy::prelude::*;
use bevy::math::DVec3;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use em_sim::body::BodyInfo;
use em_sim::motive::MotiveSelection;
use em_sim::motive::kepler::{EccentricitySMA, KeplerEulerAngles, KeplerMotive, KeplerRotation, KeplerShape};
use crate::body::universe::Universe;
use crate::gui::common;
use crate::gui::menu::UiState;
use crate::gui::planetarium::focused_body::{body_select_dropdown, sorted_body_options};
use crate::gui::planetarium::FocusedBodyState;
use crate::sim::{BodySelection, CalculateTrajectory};
use crate::sim::world::SimSystem;
use crate::gui::settings::{Settings, UiTheme};

pub fn body_edit_window(
    settings: ResMut<Settings>,
    _ui_state: ResMut<UiState>,
    universe: Res<Universe>,
    mut contexts: EguiContexts,
    mut focused_body_state: ResMut<FocusedBodyState>,
    mut system: ResMut<SimSystem>,
    mut calc: MessageWriter<CalculateTrajectory>,
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
                let body_options = sorted_body_options(&universe);
                body_select_dropdown(&universe, &mut focused_body_state, ui, &body_options);

                let selected = focused_body_state
                    .current_body_id
                    .as_deref()
                    .and_then(|id| system.0.by_name(id));

                let Some(index) = selected else {
                    ui.label("No body Selected");
                    return;
                };

                let time = system.0.time();
                let mut changed = body_info_section(ui, system.0.info_mut(index));

                // Editing goes through `motive_mut`, which marks the arena dirty so the
                // next propagation rebuilds this body's cache from the new elements.
                if let Some(selection) = system.0.motive_mut(index).motive_at_mut(time) {
                    changed |= match selection {
                        MotiveSelection::Fixed { position, .. } => fixed_motive_section(ui, position),
                        MotiveSelection::Newtonian { position, velocity } =>
                            newton_motive_section(ui, position, velocity),
                        MotiveSelection::Keplerian(kepler) => kepler_motive_section(ui, kepler),
                    };
                }

                if changed {
                    let id = system.0.info(index).id.clone();
                    calc.write(CalculateTrajectory { selection: BodySelection::IDs(vec![id]) });
                }
            });
    }
}

fn body_info_section(ui: &mut egui::Ui, info: &mut BodyInfo) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.label(info.display_name());
    });

    let mass_before = info.mass;
    ui.horizontal(|ui| {
        ui.label("Mass:");
        common::stepper(ui, "", &mut info.mass);
        ui.label("kg");
    });
    changed |= info.mass != mass_before;
    changed
}

fn fixed_motive_section(ui: &mut egui::Ui, position: &mut DVec3) -> bool {
    let before = *position;
    ui.heading("Fixed Position");
    ui.vertical(|ui| {
        vector_steppers(ui, position, "m");
    });
    *position != before
}

fn vector_steppers(ui: &mut egui::Ui, v: &mut DVec3, unit: &str) {
    ui.horizontal(|ui| {
        common::stepper(ui, "x", &mut v.x);
        ui.label(unit);
    });
    ui.horizontal(|ui| {
        common::stepper(ui, "y", &mut v.y);
        ui.label(unit);
    });
    ui.horizontal(|ui| {
        common::stepper(ui, "z", &mut v.z);
        ui.label(unit);
    });
}

fn kepler_motive_section(ui: &mut egui::Ui, motive: &mut KeplerMotive) -> bool {
    let mut changed = false;
    ui.heading("Keplerian Body");

    ui.vertical(|ui| {
        ui.heading("Shape");
        match &mut motive.shape {
            KeplerShape::EccentricitySMA(sma) => changed |= kepler_motive_shape_sma_section(ui, sma),
            KeplerShape::Apsides(_apsides) => {}
        }
    });
    ui.separator();

    ui.vertical(|ui| {
        ui.heading("Rotation");
        match &mut motive.rotation {
            KeplerRotation::EulerAngles(ea) => changed |= kepler_motive_rotation_ea_section(ui, ea),
            KeplerRotation::FlatAngles(_fa) => {}
            KeplerRotation::PrecessingEulerAngles(_pea) => {}
        }
    });
    ui.separator();

    ui.vertical(|ui| {
        ui.heading("Epoch");
    });
    changed
}

fn kepler_motive_shape_sma_section(ui: &mut egui::Ui, sma: &mut EccentricitySMA) -> bool {
    let sma_before = sma.semi_major_axis;
    let ecc_before = sma.eccentricity;
    
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
    
    sma.semi_major_axis != sma_before || sma.eccentricity != ecc_before
}

fn kepler_motive_rotation_ea_section(ui: &mut Ui, kea: &mut KeplerEulerAngles) -> bool {
    let mut changed = false;
    
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
            changed = true;
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
            changed = true;
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
            changed = true;
        }
    });
    changed
}

fn newton_motive_section(ui: &mut egui::Ui, position: &mut DVec3, velocity: &mut DVec3) -> bool {
    let pos_before = *position;
    let vel_before = *velocity;

    ui.heading("Newtonian Body");

    ui.heading("Position");
    vector_steppers(ui, position, "m");

    ui.heading("Velocity");
    vector_steppers(ui, velocity, "m/s");

    *position != pos_before || *velocity != vel_before
}
