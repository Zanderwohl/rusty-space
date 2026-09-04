use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use em_sim::body::BodyInfo;
use em_sim::motive::{MotiveSelection, kepler::KeplerMotive};
use em_foundations::time::Instant;
use crate::body::universe::Universe;
use crate::camera::{GoTo, GoToSource};
use crate::gui::planetarium::focused_body::{body_select_dropdown, sorted_body_options};
use crate::gui::planetarium::FocusedBodyState;
use crate::gui::settings::{Settings, UiTheme};
use crate::sim::world::{BodyEntities, SimSystem};

pub fn body_info_window(
    settings: Res<Settings>,
    universe: Res<Universe>,
    system: Res<SimSystem>,
    body_entities: Res<BodyEntities>,
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

                let selected = focused_body_state
                    .current_body_id
                    .as_deref()
                    .and_then(|id| system.0.by_name(id));

                match selected {
                    Some(index) => {
                        let id = system.0.id(index);
                        if let Some(entity) = body_entities.map.get(&id).copied() {
                            if ui.button("Go to").clicked() {
                                go_to.write(GoTo {
                                    entity,
                                    frame: None,
                                    source: GoToSource::UiButton,
                                });
                            }
                        }

                        display_body_info(ui, &system.0, index);
                    }
                    None => {
                        ui.label("No body selected.");
                    }
                }
            });
    }
}

fn display_body_info(ui: &mut Ui, system: &em_sim::system::System, index: em_sim::id::BodyIndex) {
    let time = system.time();
    body_info_section(ui, system.info(index));
    ui.separator();
    body_state_section(ui, system, index);

    // One motive covers all three kinds, so the section is chosen by what is active
    // *now* rather than by which component happens to be attached.
    let (_, selection) = system.motive(index).motive_at(time);
    ui.separator();
    match selection {
        MotiveSelection::Fixed { primary_id, position } => {
            ui.label("Fixed Body");
            labelled(ui, "Primary:", primary_id.as_deref().unwrap_or("(origin)"));
            vector_row(ui, "Offset:", *position);
        }
        MotiveSelection::Newtonian { position, velocity } => {
            ui.label("Newtonian Body");
            vector_row(ui, "Position:", *position);
            vector_row(ui, "Velocity:", *velocity);
        }
        MotiveSelection::Keplerian(kepler) => {
            ui.label("Keplerian Body");
            kepler_motive_section(ui, kepler, system.mu(index), time);
        }
    }
}

fn body_info_section(ui: &mut Ui, info: &BodyInfo) {
    ui.label("Body Info");

    labelled(ui, "Name:", info.display_name());

    if let Some(designation) = &info.designation {
        labelled(ui, "Designation:", designation);
    }

    labelled(ui, "System ID:", &info.id);

    if !info.tags.is_empty() {
        labelled(ui, "Tags:", &info.tags.join(", "));
    }

    ui.separator();
    ui.label("Physical Attributes");

    labelled(ui, "Mass:", &format!("{} kg", crate::util::format::sci_not(info.mass)));
}

fn body_state_section(ui: &mut Ui, system: &em_sim::system::System, index: em_sim::id::BodyIndex) {
    ui.label("Current State");
    vector_row(ui, "Position:", system.position(index));
    vector_row(ui, "Velocity:", system.velocity(index));
    if let Some(local) = system.local_position(index) {
        vector_row(ui, "Local:", local);
    }
}

fn kepler_motive_section(ui: &mut Ui, motive: &KeplerMotive, mu: f64, time: Instant) {
    labelled(ui, "Primary:", &motive.primary_id);
    labelled(ui, "Semi-major axis:", &format!("{} m", crate::util::format::sci_not(motive.semi_major_axis())));
    labelled(ui, "Eccentricity:", &format!("{:.6}", motive.eccentricity()));
    labelled(ui, "Periapsis:", &format!("{} m", crate::util::format::sci_not(motive.periapsis())));
    match motive.apoapsis() {
        Some(apoapsis) => labelled(ui, "Apoapsis:", &format!("{} m", crate::util::format::sci_not(apoapsis))),
        None => labelled(ui, "Apoapsis:", "none (open orbit)"),
    }
    labelled(ui, "Inclination:", &format!("{:.4}°", motive.inclination()));
    labelled(ui, "Ascending node:", &format!("{:.4}°", motive.longitude_of_ascending_node_infallible(time)));
    labelled(ui, "Arg. of periapsis:", &format!("{:.4}°", motive.argument_of_periapsis(time)));
    labelled(ui, "Mean anomaly:", &format!("{:.4}°", motive.mean_anomaly(time, mu).to_degrees()));
    labelled(ui, "True anomaly:", &format!("{:.4}°", motive.true_anomaly(time, mu).to_degrees()));
    labelled(ui, "Period:", &format!("{} s", crate::util::format::sci_not(motive.period(mu).to_seconds())));
    if motive.is_precessing() {
        ui.label("Precessing elements");
    }
}

fn labelled(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.label(value);
    });
}

fn vector_row(ui: &mut Ui, label: &str, v: bevy::math::DVec3) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.label(format!(
            "{}, {}, {}",
            crate::util::format::sci_not(v.x),
            crate::util::format::sci_not(v.y),
            crate::util::format::sci_not(v.z),
        ));
    });
}
