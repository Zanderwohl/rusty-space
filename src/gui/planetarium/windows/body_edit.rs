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
use crate::gui::planetarium::{BodySelection, CalculateTrajectory};
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::gui::settings::{Settings, UiTheme};
pub fn body_edit_window(
    mut settings: ResMut<Settings>,
    mut ui_state: ResMut<UiState>,
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

                let mut selected_body = bodies.iter_mut().filter(|(e, info, state, fixed_motive, kepler_motive, newton_motive)| {
                    if body_info_state.current_body_id.is_none() { return false; }
                    <std::string::String as AsRef<str>>::as_ref(&info.id) == body_info_state.current_body_id.as_ref().unwrap()
                }).collect::<Vec<_>>();

                let selected_body = selected_body.get_mut(0);
                match selected_body {
                    None => { ui.label("No body Selected"); },
                    Some((entity, info, state, fixed_motive, kepler_motive, newton_motive)) => {
                        calc.write(CalculateTrajectory { selection: BodySelection::IDs(vec![info.id.clone()]) });
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

    ui.vertical(|ui| {
        ui.heading("Shape");
        match &mut motive.shape {
            KeplerShape::EccentricitySMA(sma) => kepler_motive_shape_sma_section(ui, sma),
            KeplerShape::Apsides(apsides) => {}
        }
    });
    ui.separator();

    ui.vertical(|ui| {
        ui.heading("Rotation");
        match &mut motive.rotation {
            KeplerRotation::EulerAngles(ea) => kepler_motive_rotation_ea_section(ui, ea),
            KeplerRotation::FlatAngles(fa) => {}
            KeplerRotation::PrecessingEulerAngles(pea) => {}
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
