use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use crate::body::motive::fixed_motive::FixedMotive;
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::{EccentricitySMA, KeplerEulerAngles, KeplerMotive, KeplerRotation, KeplerShape};
use crate::body::motive::newton_motive::NewtonMotive;
use crate::body::universe::Universe;
use crate::gui::common;
use crate::gui::menu::UiState;
use crate::sim::{BodySelection, CalculateTrajectory};
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::gui::settings::{Settings, UiTheme};
pub fn body_edit_window(
    settings: ResMut<Settings>,
    _ui_state: ResMut<UiState>,
    universe: Res<Universe>,
    mut contexts: EguiContexts,
    mut body_info_state: ResMut<BodyInfoState>,
    mut bodies: Query<(Entity, &mut BodyInfo, &BodyState, Option<&mut FixedMotive>, Option<&mut KeplerMotive>, Option<&mut NewtonMotive>)>,
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
                let mut body_options: Vec<(String, String)> = universe.id_to_name_iter()
                    .map(|(id, name)| (name.clone(), id.clone()))
                    .collect();
                body_options.sort_by(|a, b| a.0.cmp(&b.0));
                crate::gui::planetarium::windows::body_info::body_select_dropdown(universe, &mut body_info_state, ui, body_options);

                let mut selected_body = bodies.iter_mut().filter(|(_e, info, _state, _fixed_motive, _kepler_motive, _newton_motive)| {
                    if body_info_state.current_body_id.is_none() { return false; }
                    <std::string::String as AsRef<str>>::as_ref(&info.id) == body_info_state.current_body_id.as_ref().unwrap()
                }).collect::<Vec<_>>();

                let selected_body = selected_body.get_mut(0);
                match selected_body {
                    None => { ui.label("No body Selected"); },
                    Some((_entity, info, _state, fixed_motive, kepler_motive, newton_motive)) => {
                        let mut changed = false;
                        changed |= body_info_section(ui, info);
                        if let Some(fixed_motive) = fixed_motive.as_mut() {
                            changed |= fixed_motive_section(ui, fixed_motive.as_mut());
                        }
                        if let Some(kepler_motive) = kepler_motive.as_mut() {
                            changed |= kepler_motive_section(ui, kepler_motive.as_mut());
                        }
                        if let Some(newton_motive) = newton_motive.as_mut() {
                            changed |= newton_motive_section(ui, newton_motive.as_mut());
                        }
                        if changed {
                            calc.write(CalculateTrajectory { selection: BodySelection::IDs(vec![info.id.clone()]) });
                        }
                    }
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

fn fixed_motive_section(ui: &mut egui::Ui, motive: &mut FixedMotive) -> bool {
    let pos_before = motive.position;
    ui.heading("Fixed Position");
    ui.vertical(|ui| {
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
    });
    motive.position != pos_before
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

fn newton_motive_section(ui: &mut egui::Ui, motive: &mut NewtonMotive) -> bool {
    let pos_before = motive.position;
    let vel_before = motive.velocity;
    
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
    
    motive.position != pos_before || motive.velocity != vel_before
}
