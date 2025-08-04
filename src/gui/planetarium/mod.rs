use itertools::Itertools;
use std::collections::HashMap;
use bevy::app::{App, Update};
use bevy::math::DVec3;
use bevy::pbr::PointLight;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use lazy_static::lazy_static;
use num_traits::Pow;
use regex::Regex;
use crate::body::appearance::{Appearance, AssetCache};
use crate::body::universe::save::{UniverseFile, UniversePhysics, ViewSettings};
use crate::body::universe::Universe;
use crate::gui::app::{AppState, PlanetariumCamera};
use crate::gui::menu::{TagState, UiState};
use crate::gui::planetarium::time::SimTime;
use crate::body::{universe, unload_simulation_objects, SimulationObject};
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::{fixed_motive, kepler_motive, newton_motive};
use crate::body::motive::kepler_motive::KeplerMotive;
use crate::gui::planetarium::windows::body_info::{BodyInfoState};
use crate::util::bevystuff::GlamVec;
use crate::util::mappings;

const J2000_JD: f64 = 2451545.0;

pub mod time;
mod windows;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumUISet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumSimulationSet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumLoadingSet;

pub struct PlanetariumUI;

impl Plugin for PlanetariumUI {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SimTime>()
            .init_resource::<UniversePhysics>()
            .init_resource::<ViewSettings>()
            .init_resource::<AssetCache>()
            .init_resource::<BodyInfoState>()
            .configure_sets(Update, (
                PlanetariumUISet.run_if(in_state(AppState::Planetarium)),
                PlanetariumSimulationSet.run_if(in_state(AppState::Planetarium)),
                PlanetariumLoadingSet.run_if(in_state(AppState::PlanetariumLoading)),
            ))
            .add_systems(EguiPrimaryContextPass, (
                (
                    windows::controls::control_window,
                    windows::body_edit::body_edit_window,
                    windows::body_info::body_info_window,
                    windows::settings::settings_window,
                    windows::spin::spin_window,

                    label_bodies,
                    ).run_if(in_state(AppState::Planetarium)),
                ))
            .add_systems(Update, (
                (
                    adjust_lights,
                    position_bodies.after(fixed_motive::calculate).after(kepler_motive::calculate).after(newton_motive::calculate),
                    render_trajectories,
                ).in_set(PlanetariumUISet),
                (
                    universe::advance_time,
                    fixed_motive::calculate.before(position_bodies),
                    kepler_motive::calculate.before(position_bodies),
                    newton_motive::calculate.after(kepler_motive::calculate).after(fixed_motive::calculate).before(position_bodies),
                    kepler_motive::calculate_trajectory,
                ).in_set(PlanetariumSimulationSet),
                (load_assets).in_set(PlanetariumLoadingSet),
            ))
            .add_systems(OnExit(AppState::Planetarium), unload_simulation_objects)
        ;
    }
}

lazy_static! {
    static ref SCI_RE: Regex = Regex::new(r"\d?\.\d+\s?x\s?10\s?\^\s?\d+").unwrap();
}

fn adjust_lights(
    mut lights: Query<(&BodyInfo, &mut PointLight, &Appearance)>,
    view_settings: Res<ViewSettings>,
) {
    if !view_settings.is_changed() {
        return;
    }

    // TODO: better scaling factor, and log scaling
    for (_, mut light, appearance) in lights.iter_mut() {
        match appearance {
            Appearance::Star(star_ball) => {
                light.range = star_ball.intensity * (1e9 * (view_settings.distance_scale as f32)).pow(2.0);
            }
            _ => {} // This probably won't happen but if it does, it's not worth a crash.
        }
    }
}

fn position_bodies(
    mut sim_time: ResMut<SimTime>,
    mut bodies: Query<(&SimulationObject, &mut Transform, &BodyInfo, &BodyState, &Appearance)>,
    view_settings: Res<ViewSettings>,
) {
    // TODO: Move the origin to the main camera.
    let distance_scale = if view_settings.logarithmic_distance_scale {
        let n = mappings::log_scale(view_settings.distance_scale, view_settings.logarithmic_distance_base);
        // info!("{} -> {}", view_settings.distance_scale, n);
        n
    } else {
        view_settings.distance_scale
    };

    for (_, mut transform, _, state, appearance) in bodies.iter_mut() {
        // TODO: I doubt any of this works for moonmoons.
        let global_position: Vec3 = if !view_settings.logarithmic_distance_scale || state.current_local_position.is_none() || state.current_primary_position.is_none() {
            state.current_position.as_bevy_scale(distance_scale)  // Use calculated position *unless* we are doing logarithmic distance scale and current object has a primary.
        } else {
            let local_position = state.current_local_position.unwrap().as_bevy_scale(distance_scale);
            let primary_position = state.current_primary_position.unwrap().as_bevy_scale(distance_scale);
            primary_position + local_position
        };
        transform.translation = global_position;

        let body_scale = if view_settings.logarithmic_body_scale {
            mappings::log_scale(appearance.radius(), view_settings.logarithmic_body_base) * view_settings.body_scale
        } else {
            appearance.radius() * view_settings.body_scale
        } as f32;
        transform.scale = bevy::math::Vec3::splat(body_scale);
    }
}

fn render_trajectories(
    bodies: Query<(&BodyState, &BodyInfo)>,
    mut gizmos: Gizmos,
    view_settings: Res<ViewSettings>,
) {
    let distance_scale = if view_settings.logarithmic_distance_scale {
        let n = mappings::log_scale(view_settings.distance_scale, view_settings.logarithmic_distance_base);
        n
    } else {
        view_settings.distance_scale
    };

    let color = Srgba::new(1.0, 0.0, 0.0, 1.0);
    for (state, info) in bodies.iter() {
        if let Some(trajectory) = &state.trajectory {
            for ((t1, d1), (t2, d2)) in trajectory.iter().tuple_windows() {
                gizmos.line(d1.as_bevy_scale(distance_scale), d2.as_bevy_scale(distance_scale), color);
            }
        }
    }
}

fn label_bodies(
    view_settings: Res<ViewSettings>,
    mut contexts: EguiContexts,
    cameras: Query<(&Camera, &Camera3d, &PlanetariumCamera, &GlobalTransform)>,
    bodies: Query<(&SimulationObject, &mut Transform, &BodyInfo)>,
) {
    let ctx = contexts.ctx_mut();
    if ctx.is_err() { return; }
    let ctx = ctx.unwrap();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("body_labels")));

    for (camera, _, _, camera_transform) in &cameras {
        for (_, mut transform, body_info) in bodies.iter() {
            if !view_settings.show_labels && !view_settings.body_in_any_visible_tag(&body_info.id) {
                continue;
            }

            let position = transform.translation;
            let view_pos = camera.world_to_viewport(camera_transform, position);
            match view_pos {
                Ok(pos) => {
                    painter.text(
                        egui::pos2(pos.x, pos.y),
                        egui::Align2::CENTER_BOTTOM,
                        body_info.display_name(),
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
                }
                Err(_) => {}
            }
        }
    }
}

fn load_assets(
    mut commands: Commands,
    mut ui_state: ResMut<UiState>,
    mut view_settings: ResMut<ViewSettings>,
    mut next_app_state: ResMut<NextState<AppState>>,
    mut cache: ResMut<AssetCache>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut universe: ResMut<Universe>,
    mut physics: ResMut<UniversePhysics>,
    mut sim_time: ResMut<SimTime>,
) {
    if ui_state.current_save.is_none() {
        next_app_state.set(AppState::Planetarium);
        return;
    }

    let save = (ui_state.current_save.clone()).unwrap();
    let path = save.path;

    let universe_file: Option<UniverseFile> = UniverseFile::load_from_path(&path);
    if let Some(universe_file) = universe_file {
        let (new_universe, mut sim_time) = Universe::from_file(&universe_file);
        universe.path = new_universe.path.clone();
        universe.clear_all();
        let version = universe_file.contents.version; // TODO: Support multiple file format versions?

        let time = (universe_file.contents.time.time_julian_days - J2000_JD) * 86400.0; // Convert Julian Days to seconds
        sim_time.time_seconds = time;
        sim_time.playing = false;

        physics.gravitational_constant = universe_file.contents.physics.gravitational_constant;
        view_settings.tags = HashMap::<String, TagState>::new();

        let bodies = universe_file.contents.bodies;
        for body in bodies {
            let id = body.id();
            let name = body.name();
            for tag in body.tags() {
                view_settings.tags.entry(tag.clone()).or_insert(TagState::default()).members.push(id.clone());
            }
            // info!("{:?}", view_settings);
            universe.insert(name, id);
            body.spawn(&mut commands, &mut cache, &mut meshes, &mut materials, &mut images);
        }
    }

    next_app_state.set(AppState::Planetarium);
}
