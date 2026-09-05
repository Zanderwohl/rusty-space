//! The Editor: the same body as the Viewer, with every stored value writable.
//!
//! Anything the file format can hold has a control here, including the alternative
//! spellings of an orbit — apsides against axis-and-eccentricity, precessing angles,
//! the four ways of pinning an epoch. Switching spelling converts rather than resets, so
//! the body does not jump when you change how you would rather describe it.

use bevy::math::DVec3;
use bevy::prelude::*;
use bevy_egui::egui::Ui;
use bevy_egui::{egui, EguiContexts};
use em_foundations::kepler::state;
use em_foundations::time::{Instant, TimeDelta};
use em_sim::appearance::Appearance;
use em_sim::body::{BodyInfo, BodyRotation, RotationEpoch, RotationMode};
use em_sim::id::BodyIndex;
use em_sim::motive::kepler::{
    Apsides, EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerFlatAngles, KeplerMotive,
    KeplerPrecessingEulerAngles, KeplerRotation, KeplerShape, MeanAnomalyAtEpoch,
    MeanAnomalyAtJ2000, TrueAnomalyAtEpoch,
};
use em_sim::motive::{Motive, MotiveSelection};
use em_sim::system::System;

use crate::body::universe::Universe;
use crate::gui::menu::escape::UnsavedChanges;
use crate::gui::planetarium::FocusedBodyState;
use crate::gui::settings::{Settings, UiTheme};
use crate::sim::world::SimSystem;
use crate::sim::{BodySelection, CalculateTrajectory};

use super::fields;
use super::picker::{body_picker, breadcrumb, BodyPickerState};

/// A change the user asked for that has to be applied outside the borrow that raised it.
enum Action {
    Kind(Kind),
    Shape(Shape),
    Angles(Angles),
    Epoch(Epoch),
    Rotation(Option<Rotation>),
}

#[derive(PartialEq, Clone, Copy)]
enum Kind {
    Fixed,
    Newtonian,
    Keplerian,
}

#[derive(PartialEq, Clone, Copy)]
enum Shape {
    AxisEccentricity,
    Apsides,
}

#[derive(PartialEq, Clone, Copy)]
enum Angles {
    Euler,
    Flat,
    Precessing,
}

#[derive(PartialEq, Clone, Copy)]
enum Epoch {
    MeanAnomaly,
    TrueAnomaly,
    PeriapsisPassage,
    J2000,
}

#[derive(PartialEq, Clone, Copy)]
enum Rotation {
    Spinning,
    TidallyLocked,
}

/// What changed, and therefore how much has to be recomputed.
#[derive(Default)]
struct Dirty {
    /// This body's own path through space.
    orbit: bool,
    /// Something every other body may depend on: a mass, a primary, a motive kind.
    system: bool,
    /// Saved, but nothing has to be recomputed: a radius, a tag, a rotation.
    cosmetic: bool,
}

impl Dirty {
    fn any(&self) -> bool {
        self.orbit || self.system || self.cosmetic
    }

    fn needs_trajectories(&self) -> bool {
        self.orbit || self.system
    }
}

pub fn editor_window(
    settings: Res<Settings>,
    mut contexts: EguiContexts,
    mut universe: ResMut<Universe>,
    mut focused: ResMut<FocusedBodyState>,
    mut picker: ResMut<BodyPickerState>,
    mut system: ResMut<SimSystem>,
    mut calc: MessageWriter<CalculateTrajectory>,
    mut unsaved: ResMut<UnsavedChanges>,
) {
    if !settings.windows.body_edit {
        return;
    }
    let Ok(ctx) = contexts.ctx_mut() else { return };

    match settings.ui.theme {
        UiTheme::Light => ctx.set_visuals(egui::Visuals::light()),
        UiTheme::Dark => ctx.set_visuals(egui::Visuals::dark()),
    }

    let mut dirty = Dirty::default();

    egui::Window::new("Editor")
        .default_width(340.0)
        .vscroll(true)
        .show(ctx, |ui| {
            body_picker(ui, "editor", &system.0, &mut focused, &mut picker);

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
            ui.separator();

            identity_section(ui, &mut system.0, &mut universe, index, &mut dirty);
            physical_section(ui, &mut system.0, index, &mut dirty);
            motion_section(ui, &mut system.0, index, &mut dirty);
            rotation_section(ui, &mut system.0, index, &mut dirty);
        });

    if dirty.any() {
        unsaved.0 = true;
    }
    if dirty.needs_trajectories() {
        let selection = if dirty.system {
            // A mass or a primary reaches every orbit that hangs off this body.
            BodySelection::All
        } else {
            let index = focused
                .current_body_id
                .as_deref()
                .and_then(|id| system.0.by_name(id));
            match index {
                Some(i) => BodySelection::IDs(vec![system.0.info(i).id.clone()]),
                None => BodySelection::All,
            }
        };
        calc.write(CalculateTrajectory { selection });
    }
}

// ------------------------------------------------------------------ identity

fn identity_section(
    ui: &mut Ui,
    system: &mut System,
    universe: &mut Universe,
    index: BodyIndex,
    dirty: &mut Dirty,
) {
    let mut changed = false;
    fields::section(ui, "Identity", true, |ui| {
        let id = system.info(index).id.clone();
        let info = system.info_mut(index);

        fields::grid(ui, "editor_identity", |ui| {
            let renamed = fields::row(ui, "Name", "What this body is called on screen", |ui| {
                let mut name = info.name.clone().unwrap_or_default();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text(id.as_str())
                        .desired_width(180.0),
                );
                if response.changed() {
                    info.name = if name.trim().is_empty() { None } else { Some(name) };
                    true
                } else {
                    false
                }
            });
            if renamed {
                // The name map is what labels and saves read; it has to follow.
                universe.remove_by_id(&id);
                universe.insert(info.display_name().to_string(), id.clone());
                changed = true;
            }

            fields::row(ui, "ID", "Fixed at creation: other bodies name this one by it", |ui| {
                ui.add_enabled(false, egui::Label::new(id.as_str()));
            });

            changed |= fields::row(ui, "Designation", "Catalogue designation, if it has one", |ui| {
                let mut designation = info.designation.clone().unwrap_or_default();
                if ui
                    .add(egui::TextEdit::singleline(&mut designation).desired_width(180.0))
                    .changed()
                {
                    info.designation =
                        if designation.trim().is_empty() { None } else { Some(designation) };
                    true
                } else {
                    false
                }
            });

            changed |= fields::row(ui, "Major", "Major bodies pull on Newtonian bodies", |ui| {
                ui.checkbox(&mut info.major, "").changed()
            });
        });

        changed |= tags_editor(ui, info);
    });
    // A name, a tag or the major flag: only the last changes any motion, and it is rare
    // enough that recomputing everything is the honest answer.
    dirty.cosmetic |= changed;
    dirty.system |= changed;
}

/// Tags drive the show/hide panel, so they are worth editing in place.
fn tags_editor(ui: &mut Ui, info: &mut BodyInfo) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label("Tags").on_hover_text("Groups the show/hide panel switches on");
        let mut remove = None;
        for (position, tag) in info.tags.iter().enumerate() {
            if ui.small_button(format!("{tag} ×")).on_hover_text("Remove").clicked() {
                remove = Some(position);
            }
        }
        if let Some(position) = remove {
            info.tags.remove(position);
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        let id = ui.make_persistent_id(("editor_new_tag", info.id.as_str()));
        let mut draft: String = ui.data(|d| d.get_temp(id).unwrap_or_default());
        let response = ui.add(
            egui::TextEdit::singleline(&mut draft)
                .hint_text("Add a tag")
                .desired_width(140.0),
        );
        let submitted = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let clicked = ui.small_button("+").clicked();
        if (submitted || clicked) && !draft.trim().is_empty() {
            let tag = draft.trim().to_string();
            if !info.tags.contains(&tag) {
                info.tags.push(tag);
                changed = true;
            }
            draft.clear();
        }
        ui.data_mut(|d| d.insert_temp(id, draft));
    });
    changed
}

// ------------------------------------------------------------------ physical

fn physical_section(ui: &mut Ui, system: &mut System, index: BodyIndex, dirty: &mut Dirty) {
    fields::section(ui, "Physical", true, |ui| {
        fields::grid(ui, "editor_physical", |ui| {
            let mass = system.info(index).mass;
            let mut edited = mass;
            let mass_changed = fields::row(ui, "Mass", "", |ui| fields::sci_drag(ui, &mut edited, "kg"));
            if mass_changed {
                system.info_mut(index).mass = edited;
                dirty.system = true;
            }
            fields::row(ui, "", "", |ui| {
                ui.weak(format!(
                    "{:.4} Earth · {:.6} Solar",
                    edited / fields::EARTH_MASS_KG,
                    edited / fields::SUN_MASS_KG,
                ));
            });

            match system.appearance_mut(index) {
                Appearance::Empty => {
                    fields::row(ui, "Radius", "", |ui| {
                        ui.weak("no appearance").on_hover_text("Give this body a shape to set a radius");
                    });
                }
                Appearance::DebugBall(ball) => {
                    dirty.cosmetic |=
                        fields::row(ui, "Radius", "", |ui| fields::sci_drag(ui, &mut ball.radius, "m"));
                }
                Appearance::Star(star) => {
                    dirty.cosmetic |=
                        fields::row(ui, "Radius", "", |ui| fields::sci_drag(ui, &mut star.radius, "m"));
                    dirty.cosmetic |=
                        fields::row(ui, "Magnitude", "Absolute magnitude; lower is brighter", |ui| {
                            let mut magnitude = star.absolute_magnitude as f64;
                            if fields::unit_drag(ui, &mut magnitude, 0.01, -30.0..=30.0, "") {
                                star.absolute_magnitude = magnitude as f32;
                                return true;
                            }
                            false
                        });
                }
            }
        });
    });
}

// ------------------------------------------------------------------ motion

fn motion_section(ui: &mut Ui, system: &mut System, index: BodyIndex, dirty: &mut Dirty) {
    let time = system.time();
    let mut action: Option<Action> = None;

    fields::section(ui, "Motion", true, |ui| {
        let current = motive_kind(&system.motive(index).motive_at(time).1);

        ui.horizontal(|ui| {
            ui.label("Kind").on_hover_text("How this body's position is worked out");
            for kind in [Kind::Fixed, Kind::Newtonian, Kind::Keplerian] {
                let selected = current == kind;
                if ui
                    .selectable_label(selected, kind_label(kind))
                    .on_hover_text(kind_conversion_hint(kind))
                    .clicked()
                    && !selected
                {
                    action = Some(Action::Kind(kind));
                }
            }
        });

        let options = primary_options(system, index);
        dirty.system |= primary_picker(ui, system, index, &options);

        // Captured before the mutable borrow: the override defaults below have to fall
        // back to the values the orbit is already using, not to zero.
        let mu = system.mu(index);
        let derived_period = keplerian(system, index, time).map(|kepler| kepler.period(mu));

        let selection_time = time;
        match system.motive_mut(index).motive_at_mut(selection_time) {
            Some(MotiveSelection::Fixed { position, .. }) => {
                fields::grid(ui, "editor_fixed", |ui| {
                    dirty.orbit |= fields::vector_edit(ui, "Offset", "Displacement from the primary", position, "m");
                });
            }
            Some(MotiveSelection::Newtonian { position, velocity }) => {
                fields::grid(ui, "editor_newtonian", |ui| {
                    dirty.orbit |= fields::vector_edit(ui, "Position", "Where the integration starts", position, "m");
                    dirty.orbit |= fields::vector_edit(ui, "Velocity", "The velocity it starts with", velocity, "m/s");
                });
            }
            Some(MotiveSelection::Keplerian(kepler)) => {
                let defaults = Overrides { mu, period: derived_period.unwrap_or(TimeDelta::ZERO) };
                kepler_editor(ui, kepler, &defaults, &mut action, dirty);
            }
            None => {
                ui.weak("This body has no motive.");
            }
        }
    });

    if let Some(action) = action {
        apply(system, index, action, dirty);
    }
}

/// The primary is stored in the motive, so changing it re-parents the body.
fn primary_picker(ui: &mut Ui, system: &mut System, index: BodyIndex, options: &[(String, String)]) -> bool {
    let time = system.time();
    let allows_origin = matches!(system.motive(index).motive_at(time).1, MotiveSelection::Fixed { .. });
    let current = system
        .motive(index)
        .motive_at(time)
        .1
        .primary_id()
        .map(|id| id.to_string());

    if !allows_origin && current.is_none() {
        return false; // Newtonian bodies answer to every major body, not to one primary.
    }

    let label = current
        .as_deref()
        .and_then(|id| options.iter().find(|(candidate, _)| candidate == id))
        .map(|(_, name)| name.clone())
        .or_else(|| current.clone())
        .unwrap_or_else(|| "(origin)".to_string());

    let mut chosen: Option<Option<String>> = None;
    ui.horizontal(|ui| {
        ui.label("Primary").on_hover_text("The body this motion is measured against");
        egui::ComboBox::from_id_salt("editor_primary")
            .selected_text(label)
            .show_ui(ui, |ui| {
                if allows_origin && ui.selectable_label(current.is_none(), "(origin)").clicked() {
                    chosen = Some(None);
                }
                for (id, name) in options {
                    if ui.selectable_label(current.as_deref() == Some(id), name).clicked() {
                        chosen = Some(Some(id.clone()));
                    }
                }
            });
    });

    let Some(chosen) = chosen else { return false };
    if chosen == current {
        return false;
    }

    match system.motive_mut(index).motive_at_mut(time) {
        Some(MotiveSelection::Fixed { primary_id, .. }) => *primary_id = chosen,
        Some(MotiveSelection::Keplerian(kepler)) => match chosen {
            Some(id) => kepler.primary_id = id,
            None => return false, // a Keplerian orbit has to orbit something
        },
        _ => return false,
    }
    true
}

/// Every other body, by name, for the primary picker.
fn primary_options(system: &System, index: BodyIndex) -> Vec<(String, String)> {
    let mut options: Vec<(String, String)> = system
        .indices()
        .filter(|i| *i != index)
        .map(|i| (system.info(i).id.clone(), system.info(i).display_name().to_string()))
        .collect();
    options.sort_by(|a, b| a.1.cmp(&b.1));
    options
}

/// What an override falls back to when it is switched on: whatever the orbit already
/// uses, so ticking the box does not move the body.
struct Overrides {
    mu: f64,
    period: TimeDelta,
}

fn kepler_editor(
    ui: &mut Ui,
    kepler: &mut KeplerMotive,
    defaults: &Overrides,
    action: &mut Option<Action>,
    dirty: &mut Dirty,
) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Shape").on_hover_text("Two ways of saying the same ellipse");
        let current = match kepler.shape {
            KeplerShape::EccentricitySMA(_) => Shape::AxisEccentricity,
            KeplerShape::Apsides(_) => Shape::Apsides,
        };
        for (shape, label, hint) in [
            (Shape::AxisEccentricity, "a, e", "Semi-major axis and eccentricity"),
            (Shape::Apsides, "rp, ra", "Periapsis and apoapsis radii"),
        ] {
            if ui.selectable_label(current == shape, label).on_hover_text(hint).clicked() && current != shape {
                *action = Some(Action::Shape(shape));
            }
        }
    });

    fields::grid(ui, "editor_shape", |ui| match &mut kepler.shape {
        KeplerShape::EccentricitySMA(shape) => {
            dirty.orbit |= fields::row(ui, "Semi-major", "Semi-major axis, a", |ui| {
                fields::sci_drag(ui, &mut shape.semi_major_axis, "m")
            });
            dirty.orbit |= fields::row(ui, "Eccentricity", "0 is a circle, 1 parabolic, above 1 hyperbolic", |ui| {
                fields::unit_drag(ui, &mut shape.eccentricity, 0.001, 0.0..=3.0, "")
            });
            fields::row(ui, "", "", |ui| {
                ui.horizontal(|ui| {
                    if ui.small_button("Circular").clicked() {
                        shape.eccentricity = 0.0;
                        dirty.orbit = true;
                    }
                    if ui.small_button("Parabolic").on_hover_text("Exactly escape velocity").clicked() {
                        shape.eccentricity = 1.0;
                        dirty.orbit = true;
                    }
                });
            });
        }
        KeplerShape::Apsides(shape) => {
            dirty.orbit |= fields::row(ui, "Periapsis", "Closest approach, from the primary's centre", |ui| {
                fields::sci_drag(ui, &mut shape.periapsis, "m")
            });
            dirty.orbit |= fields::row(ui, "Apoapsis", "Furthest point, from the primary's centre", |ui| {
                fields::sci_drag(ui, &mut shape.apoapsis, "m")
            });
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Angles").on_hover_text("How the orbit is oriented in space");
        let current = match kepler.rotation {
            KeplerRotation::EulerAngles(_) => Angles::Euler,
            KeplerRotation::FlatAngles(_) => Angles::Flat,
            KeplerRotation::PrecessingEulerAngles(_) => Angles::Precessing,
        };
        for (angles, label, hint) in [
            (Angles::Euler, "Euler", "Inclination, node and periapsis argument"),
            (Angles::Flat, "Flat", "In the ecliptic: only the longitude of periapsis"),
            (Angles::Precessing, "Precessing", "Euler angles that turn over time"),
        ] {
            if ui.selectable_label(current == angles, label).on_hover_text(hint).clicked() && current != angles {
                *action = Some(Action::Angles(angles));
            }
        }
    });

    fields::grid(ui, "editor_angles", |ui| match &mut kepler.rotation {
        KeplerRotation::EulerAngles(angles) => {
            dirty.orbit |= fields::row(ui, "Inclination", "Tilt against the ecliptic, i", |ui| {
                fields::angle_drag(ui, &mut angles.inclination)
            });
            dirty.orbit |= fields::row(ui, "Node", "Longitude of the ascending node, Ω", |ui| {
                fields::angle_drag(ui, &mut angles.longitude_of_ascending_node)
            });
            dirty.orbit |= fields::row(ui, "Periapsis arg.", "Argument of periapsis, ω", |ui| {
                fields::angle_drag(ui, &mut angles.argument_of_periapsis)
            });
        }
        KeplerRotation::FlatAngles(angles) => {
            dirty.orbit |= fields::row(ui, "Periapsis long.", "Longitude of periapsis, ϖ", |ui| {
                fields::angle_drag(ui, &mut angles.longitude_of_periapsis)
            });
        }
        KeplerRotation::PrecessingEulerAngles(angles) => {
            dirty.orbit |= fields::row(ui, "Inclination", "Tilt against the ecliptic, i", |ui| {
                fields::angle_drag(ui, &mut angles.inclination)
            });
            dirty.orbit |= fields::row(ui, "Node", "Longitude of the ascending node at epoch, Ω", |ui| {
                fields::angle_drag(ui, &mut angles.longitude_of_ascending_node)
            });
            dirty.orbit |= fields::row(ui, "Periapsis arg.", "Argument of periapsis at epoch, ω", |ui| {
                fields::angle_drag(ui, &mut angles.argument_of_periapsis)
            });
            dirty.orbit |= fields::row(ui, "Apsidal", "One full turn of the periapsis; positive is prograde", |ui| {
                seconds_drag(ui, &mut angles.apsidal_precession_period)
            });
            dirty.orbit |= fields::row(ui, "Nodal", "One full turn of the node; usually negative", |ui| {
                seconds_drag(ui, &mut angles.nodal_precession_period)
            });
        }
    });

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.label("Epoch").on_hover_text("Where on the orbit the body is, and when");
        let current = match kepler.epoch {
            KeplerEpoch::MeanAnomaly(_) => Epoch::MeanAnomaly,
            KeplerEpoch::TrueAnomaly(_) => Epoch::TrueAnomaly,
            KeplerEpoch::TimeAtPeriapsisPassage(_) => Epoch::PeriapsisPassage,
            KeplerEpoch::J2000(_) => Epoch::J2000,
        };
        for (epoch, label, hint) in [
            (Epoch::MeanAnomaly, "M at t", "Mean anomaly at a stated epoch"),
            (Epoch::TrueAnomaly, "ν at t", "True anomaly at a stated epoch"),
            (Epoch::PeriapsisPassage, "Periapsis", "The moment of a periapsis passage"),
            (Epoch::J2000, "M at J2000", "Mean anomaly at J2000"),
        ] {
            if ui.selectable_label(current == epoch, label).on_hover_text(hint).clicked() && current != epoch {
                *action = Some(Action::Epoch(epoch));
            }
        }
    });

    fields::grid(ui, "editor_epoch", |ui| match &mut kepler.epoch {
        KeplerEpoch::MeanAnomaly(epoch) => {
            dirty.orbit |= fields::row(ui, "Mean anomaly", "M at the epoch below", |ui| {
                fields::angle_drag(ui, &mut epoch.mean_anomaly)
            });
            dirty.orbit |= fields::row(ui, "Epoch", "", |ui| julian_day_drag(ui, &mut epoch.epoch));
        }
        KeplerEpoch::TrueAnomaly(epoch) => {
            dirty.orbit |= fields::row(ui, "True anomaly", "ν at the epoch below", |ui| {
                fields::angle_drag(ui, &mut epoch.true_anomaly)
            });
            dirty.orbit |= fields::row(ui, "Epoch", "", |ui| julian_day_drag(ui, &mut epoch.epoch));
        }
        KeplerEpoch::TimeAtPeriapsisPassage(instant) => {
            dirty.orbit |= fields::row(ui, "Passage", "A moment the body is at periapsis", |ui| {
                julian_day_drag(ui, instant)
            });
        }
        KeplerEpoch::J2000(epoch) => {
            dirty.orbit |= fields::row(ui, "Mean anomaly", "M at J2000", |ui| {
                fields::angle_drag(ui, &mut epoch.mean_anomaly)
            });
        }
    });

    fields::section(ui, "Overrides", false, |ui| {
        ui.weak("Fitted elements set these; leave them off for an ideal two-body orbit.");
        fields::grid(ui, "editor_overrides", |ui| {
            dirty.orbit |= fields::row(ui, "Period", "Anomalistic period, overriding Kepler's third law", |ui| {
                optional_seconds(ui, &mut kepler.anomalistic_period, defaults.period)
            });
            dirty.orbit |= fields::row(ui, "μ", "Explicit gravitational parameter, m³/s²", |ui| {
                let mut present = kepler.gravitational_parameter.is_some();
                let mut changed = ui.checkbox(&mut present, "").changed();
                match (present, kepler.gravitational_parameter) {
                    (true, None) => kepler.gravitational_parameter = Some(defaults.mu),
                    (false, Some(_)) => kepler.gravitational_parameter = None,
                    _ => {}
                }
                if let Some(mu) = kepler.gravitational_parameter.as_mut() {
                    changed |= fields::sci_drag(ui, mu, "");
                }
                changed
            });
        });
    });
}

// ------------------------------------------------------------------ rotation

fn rotation_section(ui: &mut Ui, system: &mut System, index: BodyIndex, dirty: &mut Dirty) {
    let mut action: Option<Action> = None;
    let mut changed = false;

    fields::section(ui, "Rotation", false, |ui| {
        let current = system.rotation(index).map(|rotation| match rotation.mode {
            RotationMode::Spinning { .. } => Rotation::Spinning,
            RotationMode::TidallyLocked { .. } => Rotation::TidallyLocked,
        });

        ui.horizontal(|ui| {
            ui.label("Kind");
            if ui.selectable_label(current.is_none(), "None").clicked() && current.is_some() {
                action = Some(Action::Rotation(None));
            }
            for (mode, label, hint) in [
                (Rotation::Spinning, "Spinning", "Turns at a constant rate about a fixed pole"),
                (Rotation::TidallyLocked, "Locked", "One face always points at the primary"),
            ] {
                if ui.selectable_label(current == Some(mode), label).on_hover_text(hint).clicked()
                    && current != Some(mode)
                {
                    action = Some(Action::Rotation(Some(mode)));
                }
            }
        });

        let Some(rotation) = system.rotation_mut(index) else { return };
        changed |= fields::grid(ui, "editor_rotation", |ui| {
            let mut changed = false;
            match &mut rotation.mode {
                RotationMode::Spinning { orientation_at_epoch, angular_velocity, epoch } => {
                    changed |= fields::row(ui, "Day", "One rotation about the pole", |ui| {
                        let mut period = if *angular_velocity == 0.0 {
                            0.0
                        } else {
                            std::f64::consts::TAU / angular_velocity.abs()
                        };
                        let retrograde = *angular_velocity < 0.0;
                        if fields::sci_drag(ui, &mut period, "s") && period != 0.0 {
                            let magnitude = std::f64::consts::TAU / period.abs();
                            *angular_velocity = if retrograde { -magnitude } else { magnitude };
                            return true;
                        }
                        false
                    });
                    changed |= fields::row(ui, "Direction", "Prograde turns the same way as the orbit", |ui| {
                        let mut retrograde = *angular_velocity < 0.0;
                        if ui.checkbox(&mut retrograde, "Retrograde").changed() {
                            *angular_velocity = angular_velocity.abs() * if retrograde { -1.0 } else { 1.0 };
                            return true;
                        }
                        false
                    });
                    changed |= fields::row(ui, "Tilt", "Angle between the pole and the ecliptic normal", |ui| {
                        let pole = *orientation_at_epoch * DVec3::Z;
                        let mut tilt = pole.normalize_or_zero().z.clamp(-1.0, 1.0).acos().to_degrees();
                        if fields::angle_drag(ui, &mut tilt) {
                            // Tilt about +X, which keeps the prime meridian where it was.
                            *orientation_at_epoch =
                                bevy::math::DQuat::from_rotation_x(tilt.to_radians());
                            return true;
                        }
                        false
                    });
                    changed |= fields::row(ui, "Epoch", "When the orientation above was measured", |ui| {
                        let mut julian_day = match epoch {
                            RotationEpoch::J2000 => Instant::J2000.to_julian_day(),
                            RotationEpoch::JulianDay(day) => *day,
                        };
                        if fields::unit_drag(ui, &mut julian_day, 0.01, f64::MIN..=f64::MAX, "JD") {
                            *epoch = RotationEpoch::JulianDay(julian_day);
                            return true;
                        }
                        false
                    });
                }
                RotationMode::TidallyLocked { primary_id, pole } => {
                    changed |= fields::row(ui, "Faces", "The body this one keeps facing", |ui| {
                        ui.add(egui::TextEdit::singleline(primary_id).desired_width(160.0)).changed()
                    });
                    changed |= fields::row(ui, "Tilt", "Angle between the pole and the ecliptic normal", |ui| {
                        let mut tilt = pole.normalize_or_zero().z.clamp(-1.0, 1.0).acos().to_degrees();
                        if fields::angle_drag(ui, &mut tilt) {
                            let radians = tilt.to_radians();
                            *pole = DVec3::new(radians.sin(), 0.0, radians.cos());
                            return true;
                        }
                        false
                    });
                }
            }
            changed
        });
    });
    dirty.cosmetic |= changed;

    if let Some(Action::Rotation(mode)) = action {
        let rotation = match mode {
            None => None,
            // One turn a day, upright: a starting point to drag away from.
            Some(Rotation::Spinning) => Some(BodyRotation::spinning(
                bevy::math::DQuat::IDENTITY,
                std::f64::consts::TAU / 86_400.0,
                RotationEpoch::J2000,
            )),
            Some(Rotation::TidallyLocked) => {
                let primary = system
                    .parent(index)
                    .map(|parent| system.info(parent).id.clone())
                    .unwrap_or_default();
                Some(BodyRotation::tidally_locked(primary, DVec3::Z))
            }
        };
        system.set_rotation(index, rotation);
        dirty.cosmetic = true;
    }
}

// ------------------------------------------------------------------ conversions

/// Apply a change of representation, carrying the current values across.
fn apply(system: &mut System, index: BodyIndex, action: Action, dirty: &mut Dirty) {
    let time = system.time();
    let mu = system.mu(index);

    match action {
        Action::Kind(kind) => {
            if let Some(motive) = converted_motive(system, index, kind) {
                *system.motive_mut(index) = motive;
                dirty.system = true;
            }
        }
        Action::Shape(shape) => {
            let Some(kepler) = keplerian_mut(system, index, time) else { return };
            let a = kepler.semi_major_axis();
            let e = kepler.eccentricity();
            kepler.shape = match shape {
                Shape::AxisEccentricity => KeplerShape::EccentricitySMA(EccentricitySMA {
                    eccentricity: e,
                    semi_major_axis: a,
                }),
                Shape::Apsides => KeplerShape::Apsides(Apsides {
                    periapsis: a * (1.0 - e),
                    apoapsis: a * (1.0 + e),
                }),
            };
            dirty.orbit = true;
        }
        Action::Angles(target) => {
            let Some(kepler) = keplerian_mut(system, index, time) else { return };
            let elapsed = TimeDelta::ZERO; // convert at the orbit's own epoch, not the clock
            let inclination = kepler.rotation.inclination();
            let node = kepler.rotation.longitude_of_ascending_node_infallible(elapsed);
            let periapsis_arg = kepler.rotation.argument_of_periapsis(elapsed);
            kepler.rotation = match target {
                Angles::Euler => KeplerRotation::EulerAngles(KeplerEulerAngles {
                    inclination,
                    longitude_of_ascending_node: node,
                    argument_of_periapsis: periapsis_arg,
                }),
                Angles::Flat => KeplerRotation::FlatAngles(KeplerFlatAngles {
                    longitude_of_periapsis: node + periapsis_arg,
                }),
                Angles::Precessing => {
                    KeplerRotation::PrecessingEulerAngles(KeplerPrecessingEulerAngles {
                        inclination,
                        longitude_of_ascending_node: node,
                        argument_of_periapsis: periapsis_arg,
                        apsidal_precession_period: TimeDelta::ZERO,
                        nodal_precession_period: TimeDelta::ZERO,
                    })
                }
            };
            dirty.orbit = true;
        }
        Action::Epoch(target) => {
            // Read the anomalies off the orbit as it stands, then restate them.
            let Some(current) = keplerian(system, index, time).cloned() else { return };
            let at = current.epoch.epoch();
            let epoch = match target {
                Epoch::MeanAnomaly => KeplerEpoch::MeanAnomaly(MeanAnomalyAtEpoch {
                    epoch: at,
                    mean_anomaly: current.mean_anomaly(at, mu).to_degrees(),
                }),
                Epoch::TrueAnomaly => KeplerEpoch::TrueAnomaly(TrueAnomalyAtEpoch {
                    epoch: at,
                    true_anomaly: current.true_anomaly(at, mu).to_degrees(),
                }),
                Epoch::PeriapsisPassage => {
                    KeplerEpoch::TimeAtPeriapsisPassage(current.time_at_periapsis_passage(mu))
                }
                Epoch::J2000 => KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
                    mean_anomaly: current.mean_anomaly(Instant::J2000, mu).to_degrees(),
                }),
            };
            if let Some(kepler) = keplerian_mut(system, index, time) {
                kepler.epoch = epoch;
                dirty.orbit = true;
            }
        }
        Action::Rotation(_) => {}
    }
}

/// Build a motive of `kind` that starts the body where it is now.
fn converted_motive(system: &System, index: BodyIndex, kind: Kind) -> Option<Motive> {
    let position = system.position(index);
    let velocity = system.velocity(index);
    let parent = system.parent(index);
    let parent_id = parent.map(|parent| system.info(parent).id.clone());

    match kind {
        Kind::Fixed => {
            let offset = system.local_position(index).unwrap_or(position);
            Some(Motive::fixed_with_parent(parent_id, offset))
        }
        Kind::Newtonian => Some(Motive::newtonian(position, velocity)),
        Kind::Keplerian => {
            // An orbit needs something to orbit, and elements need a state relative to it.
            // A Newtonian body has no primary at all, so fall back to the heaviest thing
            // it could be orbiting; the primary picker appears once the switch has landed.
            let parent = parent.or_else(|| heaviest_other(system, index))?;
            let local_position = position - system.position(parent);
            let local_velocity = velocity - system.velocity(parent);
            let mu = system.gravitational_constant()
                * (system.info(parent).mass + system.info(index).mass);
            let elements = state::from_state(mu, local_position, local_velocity)?;
            if !elements.semi_major_axis.is_finite() {
                return None;
            }
            Some(Motive::from_keplerian(KeplerMotive {
                primary_id: system.info(parent).id.clone(),
                shape: KeplerShape::EccentricitySMA(EccentricitySMA {
                    eccentricity: elements.eccentricity,
                    semi_major_axis: elements.semi_major_axis,
                }),
                rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
                    inclination: elements.inclination.to_degrees(),
                    longitude_of_ascending_node: elements
                        .longitude_of_ascending_node
                        .to_degrees()
                        .rem_euclid(360.0),
                    argument_of_periapsis: elements
                        .argument_of_periapsis
                        .to_degrees()
                        .rem_euclid(360.0),
                }),
                epoch: KeplerEpoch::TrueAnomaly(TrueAnomalyAtEpoch {
                    epoch: system.time(),
                    true_anomaly: elements.true_anomaly.to_degrees().rem_euclid(360.0),
                }),
                anomalistic_period: None,
                gravitational_parameter: None,
            }))
        }
    }
}

/// The most massive body that is not this one — a starting guess at a primary.
fn heaviest_other(system: &System, index: BodyIndex) -> Option<BodyIndex> {
    system
        .indices()
        .filter(|i| *i != index)
        .max_by(|a, b| system.mass(*a).total_cmp(&system.mass(*b)))
}

fn keplerian(system: &System, index: BodyIndex, time: Instant) -> Option<&KeplerMotive> {
    match &system.motive(index).motive_at(time).1 {
        MotiveSelection::Keplerian(kepler) => Some(kepler),
        _ => None,
    }
}

fn keplerian_mut(system: &mut System, index: BodyIndex, time: Instant) -> Option<&mut KeplerMotive> {
    match system.motive_mut(index).motive_at_mut(time) {
        Some(MotiveSelection::Keplerian(kepler)) => Some(kepler),
        _ => None,
    }
}

fn motive_kind(selection: &MotiveSelection) -> Kind {
    match selection {
        MotiveSelection::Fixed { .. } => Kind::Fixed,
        MotiveSelection::Newtonian { .. } => Kind::Newtonian,
        MotiveSelection::Keplerian(_) => Kind::Keplerian,
    }
}

fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Fixed => "Fixed",
        Kind::Newtonian => "Newtonian",
        Kind::Keplerian => "Keplerian",
    }
}

fn kind_conversion_hint(kind: Kind) -> &'static str {
    match kind {
        Kind::Fixed => "Held at an offset from its primary. Converting keeps the current position.",
        Kind::Newtonian => {
            "Integrated under the pull of every major body. Converting seeds it with the current \
             position and velocity."
        }
        Kind::Keplerian => {
            "Evaluated from orbital elements. Converting fits elements to the current state about \
             the primary; a body with no primary cannot convert."
        }
    }
}

// ------------------------------------------------------------------ small widgets

fn seconds_drag(ui: &mut Ui, delta: &mut TimeDelta) -> bool {
    let mut seconds = delta.to_seconds();
    if fields::sci_drag(ui, &mut seconds, "s") {
        *delta = TimeDelta::from_seconds(seconds);
        return true;
    }
    false
}

fn julian_day_drag(ui: &mut Ui, instant: &mut Instant) -> bool {
    let mut julian_day = instant.to_julian_day();
    if fields::unit_drag(ui, &mut julian_day, 0.01, f64::MIN..=f64::MAX, "JD") {
        *instant = Instant::from_julian_day(julian_day);
        return true;
    }
    false
}

fn optional_seconds(ui: &mut Ui, value: &mut Option<TimeDelta>, default: TimeDelta) -> bool {
    let mut present = value.is_some();
    let mut changed = ui.checkbox(&mut present, "").changed();
    match (present, value.as_ref()) {
        (true, None) => *value = Some(default),
        (false, Some(_)) => *value = None,
        _ => {}
    }
    if let Some(delta) = value.as_mut() {
        changed |= seconds_drag(ui, delta);
    }
    changed
}
