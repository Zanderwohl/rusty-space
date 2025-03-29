use std::io::Read;
use bevy::app::{App, Update};
use bevy::math::{DVec3, Vec2};
use bevy::pbr::PointLight;
use bevy::prelude::{in_state, info, Added, Assets, Camera, Camera3d, Changed, Commands, DetectChanges, Entity, GlobalTransform, Image, IntoSystemSetConfigs, Mesh, NextState, OnExit, Or, Plugin, Query, Res, ResMut, StandardMaterial, SystemSet, Time, Transform, Without};
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
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::kepler_motive::KeplerMotive;
use windows::body_edit::body_edit_window;
use crate::body::motive::fixed_motive::FixedMotive;
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
                    adjust_lights,
                    calculate_fixed.before(position_bodies),
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

fn adjust_lights(
    mut lights: Query<(&BodyInfo, &mut PointLight, &Appearance)>,
    view_settings: Res<ViewSettings>,
) {
    if !view_settings.is_changed() {
        return;
    }

    // TODO: better scaling factor, and log scaling
    for (body, mut light, appearance) in lights.iter_mut() {
        match appearance {
            Appearance::Star(star_ball) => {
                light.range = star_ball.intensity * (1e9 * (view_settings.distance_scale as f32)).pow(2.0);
            }
            _ => {} // This probably won't happen but if it does, it's not worth a crash.
        }
    }
}

fn calculate_fixed(
    mut fixed_bodies: Query<(&mut BodyState, &BodyInfo, &FixedMotive),
        (Or<(Changed<FixedMotive>, Added<FixedMotive>)>)>,
) {
    for (mut state, info, motive) in fixed_bodies.iter_mut() {
        state.current_position = motive.position;
        state.last_step_position = motive.position;
    }
}

fn kepler_trajectory(
    mut kepler_bodies: Query<(&mut BodyState, &BodyInfo, &KeplerMotive),
        (Or<(Changed<KeplerMotive>, Added<KeplerMotive>)>)>,
) {
    for (mut state, info, motive) in kepler_bodies.iter_mut() {
        
    }
}

fn calculate_kepler(
    mut sim_time: ResMut<SimTime>,
    mut kepler_bodies: Query<(&mut KeplerMotive, &BodyInfo, &mut BodyState)>,
    fixed_bodies: Query<(&SimulationObject, &BodyInfo, &BodyState), Without<KeplerMotive>>,
    physics: Res<UniversePhysics>,
) {
    // First collect all body IDs and masses into a HashMap to avoid borrow conflicts
    let mut bodies_prev_frame: std::collections::HashMap<String, (f64, DVec3)> = std::collections::HashMap::new();
    for (_, info, state) in fixed_bodies.iter() {
        bodies_prev_frame.insert(info.id.clone(), (info.mass, state.current_position));
    }
    for (_, info, state) in kepler_bodies.iter() {
        bodies_prev_frame.insert(info.id.clone(), (info.mass, state.current_position));
    }

    let time = sim_time.time_seconds;
    for (mut motive, info, mut state) in kepler_bodies.iter_mut() {
        let (primary_mass, primary_position) = bodies_prev_frame.get(&motive.primary_id)
            .copied()
            .expect("Missing body info");
            
        let mu = physics.gravitational_constant * primary_mass;
        let position = motive.displacement(time, mu);
        if let Some(position) = position {
            state.current_position = primary_position + position;
            state.current_local_position = Some(position);
            state.current_primary_position = Some(primary_position);
        }
    }
}

fn position_bodies(
    mut sim_time: ResMut<SimTime>,
    mut bodies: Query<(&SimulationObject, &mut Transform, &BodyInfo, &BodyState, &Appearance)>,
    view_settings: Res<ViewSettings>,
) {
    let distance_scale = if view_settings.logarithmic_distance_scale {
        let n = mappings::log_scale(view_settings.distance_scale, view_settings.logarithmic_distance_base) as f32;
        // info!("{} -> {}", view_settings.distance_scale, n);
        n
    } else {
        view_settings.distance_scale as f32
    };

    for (_, mut transform, info, state, appearance) in bodies.iter_mut() {
        // Convert from z-axis-up to y-axis-up coordinate system
        // In z-axis-up: (x, y, z) where z is up
        // In y-axis-up: (x, z, -y) where y is up
        // TODO: I doubt any of this works for moonmoons.
        if !view_settings.logarithmic_distance_scale || state.current_local_position.is_none() || state.current_primary_position.is_none() {
            let position = state.current_position.as_vec3();  // Use calculated position *unless* we are doing logarithmic distance scale current object has a primary.
            transform.translation = bevy::math::Vec3::new(
                position.x,
                position.z,
                -position.y
            ) * distance_scale; // Scale factor
        } else {
            let local_position = state.current_local_position.unwrap().as_vec3();
            let primary_position = state.current_primary_position.unwrap().as_vec3();
            let adjusted_primary_position = bevy::math::Vec3::new(
                primary_position.x,
                primary_position.z,
                -primary_position.y
            ) * distance_scale; // Scale factor
            let adjusted_local_position = bevy::math::Vec3::new(
                local_position.x,
                local_position.z,
                -local_position.y
            ) * distance_scale;
            transform.translation = adjusted_primary_position + adjusted_local_position;
        };

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
