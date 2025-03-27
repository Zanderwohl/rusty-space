use std::io::Read;
use bevy::app::{App, Update};
use bevy::math::{DVec3, Vec2};
use bevy::prelude::{in_state, info, Assets, Camera, Camera3d, Commands, Entity, GlobalTransform, Image, IntoSystemSetConfigs, Mesh, NextState, OnExit, Plugin, Query, Res, ResMut, StandardMaterial, SystemSet, Time, Transform, Without};
use bevy::prelude::IntoSystemConfigs;
use bevy::render::camera::ViewportConversionError;
use bevy::utils::HashMap;
use bevy_egui::{egui, EguiContexts};
use lazy_static::lazy_static;
use num_traits::{FloatConst, Pow};
use regex::Regex;
use crate::body::appearance::{Appearance, AssetCache};
use crate::body::universe::save::{UniverseFile, UniversePhysics, ViewSettings};
use crate::body::universe::Universe;
use crate::gui::app::{AppState, PlanetariumCamera};
use crate::gui::menu::{MenuState, TagState, UiState};
use crate::gui::planetarium::time::SimTime;
use crate::gui::settings::{Settings, UiSettings, UiTheme};
use crate::body::{unload_simulation_objects, SimulationObject};
use bevy_flycam::prelude::*;
use crate::body::motive::info::BodyInfo;
use crate::body::motive::kepler_motive::KeplerMotive;
use windows::body_edit::body_edit_window;
use crate::gui::planetarium::windows::settings::settings_window;
use crate::gui::planetarium::windows::spin::spin_window;
use crate::util::mappings;

const J2000_JD: f64 = 2451545.0;

pub mod time;
mod display;
mod windows;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumUISet;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
struct PlanetariumLoadingSet;

pub struct Planetarium;

impl Plugin for Planetarium {
    fn build(&self, app: &mut App) {
        app
            .init_resource::<SimTime>()
            .init_resource::<UniversePhysics>()
            .init_resource::<ViewSettings>()
            .init_resource::<AssetCache>()
            .configure_sets(Update, (
                PlanetariumUISet.run_if(in_state(AppState::Planetarium)),
                PlanetariumLoadingSet.run_if(in_state(AppState::PlanetariumLoading)),
            ))
            .add_systems(Update, (
                (
                    windows::controls::control_window,
                    body_edit_window,
                    settings_window,
                    spin_window,
                    advance_time,
                    calculate_kepler.before(position_bodies),
                    position_bodies,
                    label_bodies,
                ).in_set(PlanetariumUISet),
                (load_assets).in_set(PlanetariumLoadingSet),
            ))
            .add_systems(OnExit(AppState::Planetarium), unload_simulation_objects)
        ;
    }
}

lazy_static! {
    static ref SCI_RE: Regex = Regex::new(r"\d?\.\d+\s?x\s?10\s?\^\s?\d+").unwrap();
}

fn advance_time(mut sim_time: ResMut<SimTime>, time: Res<Time>) {
    if sim_time.playing {
        sim_time.previous_time = sim_time.time_seconds;
        sim_time.time_seconds += sim_time.gui_speed * time.delta_secs_f64();
    }
}

fn calculate_kepler(
    mut sim_time: ResMut<SimTime>,
    mut kepler_bodies: Query<(&KeplerMotive, &mut BodyInfo)>,
    fixed_bodies: Query<(&SimulationObject, &BodyInfo), Without<KeplerMotive>>,
    physics: Res<UniversePhysics>,
) {
    // First collect all body IDs and masses into a HashMap to avoid borrow conflicts
    let mut body_masses: std::collections::HashMap<String, (f64, DVec3)> = std::collections::HashMap::new();
    for (_, info) in fixed_bodies.iter() {
        body_masses.insert(info.id.clone(), (info.mass, info.current_position));
    }
    for (_, info) in kepler_bodies.iter() {
        body_masses.insert(info.id.clone(), (info.mass, info.current_position));
    }

    let time = sim_time.time_seconds;
    for (motive, mut info) in kepler_bodies.iter_mut() {
        let (primary_mass, primary_position) = body_masses.get(&motive.primary_id)
            .copied()
            .expect("Missing body info");
            
        let mu = physics.gravitational_constant * primary_mass;
        let position = motive.displacement(time, mu);
        if let Some(position) = position {
            info.current_position = primary_position + position;
        }
    }
}

fn position_bodies(
    mut sim_time: ResMut<SimTime>,
    mut bodies: Query<(&SimulationObject, &mut Transform, &BodyInfo, &Appearance)>,
    view_settings: Res<ViewSettings>,
) {
    let distance_scale = if view_settings.logarithmic_distance_scale {
        let n = mappings::log_scale(view_settings.distance_scale, view_settings.logarithmic_distance_base) as f32;
        // info!("{} -> {}", view_settings.distance_scale, n);
        n
    } else {
        view_settings.distance_scale as f32
    };

    for (_, mut transform, body_info, appearance) in bodies.iter_mut() {
        // Convert from z-axis-up to y-axis-up coordinate system
        // In z-axis-up: (x, y, z) where z is up
        // In y-axis-up: (x, z, -y) where y is up
        let position = body_info.current_position.as_vec3();
        transform.translation = bevy::math::Vec3::new(
            position.x,
            position.z,
            -position.y
        ) * distance_scale; // Scale factor

        let body_scale = if view_settings.logarithmic_body_scale {
            mappings::log_scale(appearance.radius(), view_settings.logarithmic_body_base) * view_settings.body_scale
        } else {
            (appearance.radius() * view_settings.body_scale)
        } as f32;
        transform.scale = bevy::math::Vec3::splat(body_scale);
    }
}

fn label_bodies(
    view_settings: Res<ViewSettings>,
    mut contexts: EguiContexts,
    cameras: Query<(&Camera, &Camera3d, &PlanetariumCamera, &GlobalTransform)>,
    bodies: Query<(&SimulationObject, &mut Transform, &BodyInfo)>,
) {
    let ctx = contexts.ctx_mut();
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::new("body_labels")));

    for (camera, camera3d, _, camera_transform) in &cameras {
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
