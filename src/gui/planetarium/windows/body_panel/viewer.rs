//! The Viewer: everything the simulation knows about one body, read-only.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use bevy_egui::egui::Ui;
use em_foundations::time::{Instant, TimeDelta};
use em_sim::appearance::Appearance;
use em_sim::body::{BodyRotation, RotationMode};
use em_sim::id::BodyIndex;
use em_sim::motive::kepler::KeplerMotive;
use em_sim::motive::{MotiveSelection, TransitionEvent};
use em_sim::system::System;

use crate::camera::{GoTo, GoToSource};
use crate::gui::planetarium::FocusedBodyState;
use crate::gui::settings::{Settings, UiTheme};
use crate::sim::world::{BodyEntities, SimSystem};

use super::fields::{self, NONE};
use super::picker::{body_picker, breadcrumb, BodyPickerState};

pub fn viewer_window(
    settings: Res<Settings>,
    system: Res<SimSystem>,
    body_entities: Res<BodyEntities>,
    mut contexts: EguiContexts,
    mut focused: ResMut<FocusedBodyState>,
    mut picker: ResMut<BodyPickerState>,
    mut go_to: MessageWriter<GoTo>,
) {
    if !settings.windows.body_info {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    egui::Window::new("Viewer")
        .default_width(320.0)
        .vscroll(true)
        .show(ctx, |ui| {
            body_picker(ui, "viewer", &system.0, &mut focused, &mut picker);

            let selected = focused
                .current_body_id
                .as_deref()
                .and_then(|id| system.0.by_name(id));

            let Some(index) = selected else {
                ui.separator();
                ui.weak("No body selected.");
                return;
            };

            ui.separator();
            breadcrumb(ui, &system.0, index, &mut focused);

            if let Some(entity) = body_entities.map.get(&system.0.id(index)).copied() {
                if ui.button("Go to").on_hover_text("Point the camera at this body").clicked() {
                    go_to.write(GoTo { entity, frame: None, source: GoToSource::UiButton });
                }
            }

            warn_unresolved(ui, &system.0, index);
            ui.separator();
            body_details(ui, &system.0, index);
        });
}

/// A body whose primary is missing still draws, wrongly. Nothing else says so.
fn warn_unresolved(ui: &mut Ui, system: &System, index: BodyIndex) {
    let id = &system.info(index).id;
    if let Some((_, primary)) = system.unresolved_primaries().iter().find(|(body, _)| body == id) {
        ui.colored_label(
            egui::Color32::from_rgb(230, 160, 60),
            format!("Primary \"{primary}\" is not in this system"),
        )
        .on_hover_text(
            "The orbit anchors at the origin and its gravitational parameter falls back \
             to this body's own mass, so the position shown is wrong.",
        );
    }
}

fn body_details(ui: &mut Ui, system: &System, index: BodyIndex) {
    let time = system.time();

    fields::section(ui, "Identity", true, |ui| {
        let info = system.info(index);
        fields::grid(ui, "viewer_identity", |ui| {
            fields::text_row(ui, "Name", "", info.display_name(), "");
            fields::text_row(ui, "ID", "The key this body is stored and referenced under", &info.id, "");
            if let Some(designation) = &info.designation {
                fields::text_row(ui, "Designation", "Catalogue designation", designation, "");
            }
            let tags = info.tags.join(", ");
            fields::text_row(
                ui,
                "Tags",
                "Group memberships; the show/hide panel works on these",
                if tags.is_empty() { NONE } else { &tags },
                "",
            );
            fields::text_row(
                ui,
                "Major",
                "Major bodies pull on Newtonian bodies; minor ones are only pulled",
                if info.major { "yes" } else { "no" },
                "",
            );
        });
    });

    fields::section(ui, "Physical", true, |ui| {
        let info = system.info(index);
        let radius = system.radius(index);
        let mu = system.gravitational_constant() * info.mass;
        fields::grid(ui, "viewer_physical", |ui| {
            fields::text_row(ui, "Mass", "", &fields::mass(info.mass), &fields::mass_hint(info.mass));
            fields::text_row(ui, "Radius", "", &fields::distance(radius), &fields::distance_hint(radius));
            if radius > 0.0 && info.mass > 0.0 {
                let surface_gravity = mu / (radius * radius);
                let escape_velocity = (2.0 * mu / radius).sqrt();
                let volume = 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3);
                fields::text_row(
                    ui,
                    "Gravity",
                    "Gravitational acceleration at the surface",
                    &format!("{surface_gravity:.3} m/s²"),
                    &format!("{:.4} g", surface_gravity / fields::STANDARD_GRAVITY),
                );
                fields::text_row(
                    ui,
                    "Escape",
                    "Escape velocity from the surface",
                    &fields::speed(escape_velocity),
                    "",
                );
                fields::text_row(
                    ui,
                    "Density",
                    "Mean density, taking the body as a sphere of the radius above",
                    &format!("{:.1} kg/m³", info.mass / volume),
                    "",
                );
            }
            fields::text_row(ui, "μ", "G × mass, this body alone", &format!("{} m³/s²", crate::util::format::sci_not(mu)), "");
        });
    });

    let (event, selection) = system.motive(index).motive_at(time);
    fields::section(ui, "Motion", true, |ui| {
        fields::grid(ui, "viewer_motion", |ui| {
            fields::text_row(ui, "Kind", "How this body's position is worked out", kind_name(selection), kind_hint(selection));
            fields::text_row(ui, "Since", "The event this motive started at", event_name(event), "");
            let primary = selection
                .primary_id()
                .map(|id| id.to_string())
                .unwrap_or_else(|| NONE.to_string());
            fields::text_row(ui, "Primary", "The body this motion is measured against", &primary, "");
        });

        match selection {
            MotiveSelection::Fixed { position, .. } => {
                fields::grid(ui, "viewer_fixed", |ui| {
                    fields::vector_rows(ui, "Offset", "Displacement from the primary", *position, false);
                });
            }
            MotiveSelection::Newtonian { position, velocity } => {
                fields::grid(ui, "viewer_newtonian", |ui| {
                    fields::vector_rows(ui, "Seed pos.", "Position the integration started from", *position, false);
                    fields::vector_rows(ui, "Seed vel.", "Velocity the integration started from", *velocity, true);
                });
            }
            MotiveSelection::Keplerian(kepler) => {
                orbit_grid(ui, system, index, kepler, time);
            }
        }

        let events = system.motive(index).iter_events().count();
        if events > 1 {
            ui.separator();
            ui.label(format!("{events} motive events"))
                .on_hover_text("This body changes how it moves over the course of the timeline");
            fields::grid(ui, "viewer_events", |ui| {
                for (at, event, selection) in system.motive(index).iter_events() {
                    fields::text_row(
                        ui,
                        &fields::instant(at),
                        &fields::instant_hint(at),
                        &format!("{} · {}", event_name(event), kind_name(selection)),
                        "",
                    );
                }
            });
        }
    });

    fields::section(ui, "State", true, |ui| {
        fields::grid(ui, "viewer_state", |ui| {
            fields::vector_rows(ui, "Position", "In the ecliptic J2000 frame, from the system origin", system.position(index), false);
            fields::vector_rows(ui, "Velocity", "In the ecliptic J2000 frame", system.velocity(index), true);
            match system.local_position(index) {
                Some(local) => fields::vector_rows(ui, "Local", "Position relative to the primary", local, false),
                None => fields::text_row(ui, "Local", "Position relative to the primary", NONE, ""),
            }
            fields::text_row(ui, "Clock", "Simulation time these figures are for", &fields::instant(time), &fields::instant_hint(time));
        });
    });

    fields::section(ui, "Rotation", false, |ui| match system.rotation(index) {
        Some(rotation) => rotation_grid(ui, rotation, system.time()),
        None => {
            ui.weak("This body does not rotate.");
        }
    });

    fields::section(ui, "Appearance", false, |ui| {
        fields::grid(ui, "viewer_appearance", |ui| match system.appearance(index) {
            Appearance::Empty => {
                fields::text_row(ui, "Kind", "", "None", "Nothing is drawn for this body");
            }
            Appearance::DebugBall(ball) => {
                fields::text_row(ui, "Kind", "", "Ball", "Wireframe sphere");
                fields::text_row(ui, "Radius", "", &fields::distance(ball.radius), &fields::distance_hint(ball.radius));
                fields::text_row(ui, "Colour", "", &format!("{}, {}, {}", ball.color.r, ball.color.g, ball.color.b), "");
                if !ball.highlight_latitudes.is_empty() {
                    let latitudes: Vec<String> =
                        ball.highlight_latitudes.iter().map(|l| format!("{l:.1}°")).collect();
                    fields::text_row(ui, "Latitudes", "Picked out on the wireframe", &latitudes.join(", "), "");
                }
            }
            Appearance::Star(star) => {
                fields::text_row(ui, "Kind", "", "Star", "Emits light onto the rest of the system");
                fields::text_row(ui, "Radius", "", &fields::distance(star.radius), &fields::distance_hint(star.radius));
                fields::text_row(
                    ui,
                    "Magnitude",
                    "Absolute magnitude; lower is brighter",
                    &format!("{:.2}", star.absolute_magnitude),
                    &format!("{:.3e} lm", star.intensity()),
                );
                fields::text_row(ui, "Colour", "", &format!("{}, {}, {}", star.color.r, star.color.g, star.color.b), "");
            }
        });
    });
}

/// The elements as they stand right now, not as they were stored.
fn orbit_grid(ui: &mut Ui, system: &System, index: BodyIndex, kepler: &KeplerMotive, time: Instant) {
    let mu = system.mu(index);
    let primary_radius = system
        .parent(index)
        .map(|parent| system.radius(parent))
        .unwrap_or(0.0);

    fields::grid(ui, "viewer_orbit", |ui| {
        let a = kepler.semi_major_axis();
        fields::text_row(ui, "Semi-major", "Semi-major axis, a", &fields::distance(a), &fields::distance_hint(a));
        fields::text_row(
            ui,
            "Eccentricity",
            "0 is a circle, 1 parabolic, above 1 hyperbolic",
            &format!("{:.6}", kepler.eccentricity()),
            "",
        );

        let periapsis = kepler.periapsis();
        fields::text_row(
            ui,
            "Periapsis",
            "Closest approach, measured from the primary's centre",
            &fields::distance(periapsis),
            &altitude_hint(periapsis, primary_radius),
        );
        match kepler.apoapsis() {
            Some(apoapsis) => fields::text_row(
                ui,
                "Apoapsis",
                "Furthest point, measured from the primary's centre",
                &fields::distance(apoapsis),
                &altitude_hint(apoapsis, primary_radius),
            ),
            None => fields::text_row(ui, "Apoapsis", "", NONE, "Open orbit — it never comes back"),
        }

        fields::text_row(ui, "Inclination", "Tilt against the ecliptic, i", &fields::angle(kepler.inclination()), "");
        fields::text_row(
            ui,
            "Node",
            "Longitude of the ascending node, Ω",
            &fields::angle(kepler.longitude_of_ascending_node_infallible(time)),
            "",
        );
        fields::text_row(
            ui,
            "Periapsis arg.",
            "Argument of periapsis, ω",
            &fields::angle(kepler.argument_of_periapsis(time)),
            "",
        );
        fields::text_row(
            ui,
            "Mean anomaly",
            "Where the body would be on a circle of the same period, M",
            &fields::angle(kepler.mean_anomaly(time, mu).to_degrees()),
            "",
        );
        fields::text_row(
            ui,
            "True anomaly",
            "Angle from periapsis to the body, ν",
            &fields::angle(kepler.true_anomaly(time, mu).to_degrees()),
            "",
        );

        let period = kepler.period(mu);
        fields::text_row(ui, "Period", "One full orbit", &fields::duration(period), &fields::duration_hint(period));
        fields::text_row(
            ui,
            "Periapsis at",
            "The periapsis passage the current elements are counted from",
            &fields::instant(kepler.time_at_periapsis_passage(mu)),
            "",
        );

        if let Some(radius) = kepler.radius_from_primary_at_time(time, mu) {
            fields::text_row(ui, "Distance", "Current separation from the primary", &fields::distance(radius), &fields::distance_hint(radius));
        }
        if let Some(velocity) = kepler.velocity(time, mu) {
            fields::text_row(ui, "Speed", "Current speed relative to the primary", &fields::speed(velocity.length()), "");
        }
        fields::text_row(
            ui,
            "μ",
            "Gravitational parameter this orbit is solved with",
            &format!("{} m³/s²", crate::util::format::sci_not(mu)),
            "G × (primary mass + this mass), unless the orbit overrides it",
        );
    });

    if kepler.is_precessing() {
        ui.weak("Precessing — the node and periapsis above are for the current time.");
    }
}

fn altitude_hint(radius: f64, primary_radius: f64) -> String {
    if primary_radius > 0.0 && radius > primary_radius {
        format!("{} above the primary's surface", fields::distance(radius - primary_radius))
    } else {
        fields::distance_hint(radius)
    }
}

fn rotation_grid(ui: &mut Ui, rotation: &BodyRotation, time: Instant) {
    let pole = rotation.pole_axis();
    let tilt = pole.normalize_or_zero().z.clamp(-1.0, 1.0).acos().to_degrees();

    fields::grid(ui, "viewer_rotation", |ui| {
        match &rotation.mode {
            RotationMode::Spinning { angular_velocity, epoch, .. } => {
                fields::text_row(ui, "Kind", "", "Spinning", "Turns at a constant rate about a fixed pole");
                let period = if *angular_velocity == 0.0 {
                    None
                } else {
                    Some(TimeDelta::from_seconds(std::f64::consts::TAU / angular_velocity.abs()))
                };
                match period {
                    Some(period) => fields::text_row(ui, "Day", "One rotation about the pole", &fields::duration(period), &fields::duration_hint(period)),
                    None => fields::text_row(ui, "Day", "One rotation about the pole", NONE, "Not turning"),
                }
                fields::text_row(
                    ui,
                    "Direction",
                    "Prograde turns the same way as the orbit",
                    if *angular_velocity >= 0.0 { "Prograde" } else { "Retrograde" },
                    "",
                );
                fields::text_row(
                    ui,
                    "Epoch",
                    "When the stored orientation was measured",
                    &fields::instant(epoch.to_instant()),
                    &fields::instant_hint(epoch.to_instant()),
                );
                if let Some(period) = period {
                    let elapsed = time.to_j2000_seconds() - epoch.to_instant().to_j2000_seconds();
                    let phase = (elapsed / period.to_seconds()).rem_euclid(1.0) * 360.0;
                    fields::text_row(ui, "Phase", "How far through the current rotation", &fields::angle(phase), "");
                }
            }
            RotationMode::TidallyLocked { primary_id, .. } => {
                fields::text_row(ui, "Kind", "", "Tidally locked", "One face always points at the primary");
                fields::text_row(ui, "Faces", "The body this one keeps facing", primary_id, "");
            }
        }
        fields::text_row(ui, "Axial tilt", "Angle between the pole and the ecliptic normal", &fields::angle(tilt), "");
        fields::text_row(ui, "Pole", "Pole direction in simulation coordinates", &fields::vector(pole), "");
    });
}

pub fn kind_name(selection: &MotiveSelection) -> &'static str {
    match selection {
        MotiveSelection::Fixed { .. } => "Fixed",
        MotiveSelection::Newtonian { .. } => "Newtonian",
        MotiveSelection::Keplerian(_) => "Keplerian",
    }
}

pub fn kind_hint(selection: &MotiveSelection) -> &'static str {
    match selection {
        MotiveSelection::Fixed { .. } => "Held at an offset from its primary, or from the origin",
        MotiveSelection::Newtonian { .. } => "Integrated step by step under the pull of every major body",
        MotiveSelection::Keplerian(_) => "Evaluated from orbital elements; any instant can be jumped to",
    }
}

pub fn event_name(event: &TransitionEvent) -> &'static str {
    match event {
        TransitionEvent::Epoch => "Epoch",
        TransitionEvent::SOIChange => "Sphere of influence change",
        TransitionEvent::Impulse => "Impulse",
        TransitionEvent::Release => "Release",
    }
}
