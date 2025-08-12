use std::collections::HashMap;
use bevy::app::{App, Update};
use bevy::math::DVec3;
use bevy::pbr::PointLight;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPrimaryContextPass};
use lazy_static::lazy_static;
use regex::Regex;
use gizmoids::trajectory;
use crate::body::appearance::{Appearance, AssetCache};
use crate::body::universe::save::{UniverseFile, UniversePhysics, ViewSettings};
use crate::body::universe::Universe;
use crate::gui::app::AppState;
use crate::gui::menu::{TagState, UiState};
use crate::gui::planetarium::time::SimTime;
use crate::body::{universe, unload_simulation_objects, SimulationObject};
use crate::body::motive::info::{BodyInfo, BodyState};
use crate::body::motive::{fixed_motive, kepler_motive, newton_motive};
pub(crate) use crate::gui::planetarium::camera::{PlanetariumCamera, PlanetariumCameraPlugin};
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::gui::util::freecam::{Freecam};
use crate::util::bevystuff::GlamVec;
use crate::util::jd::{J2000_JD, JD_SECONDS};
use crate::util::mappings;

pub mod time;
mod windows;
pub(crate) mod camera;
mod gizmoids;

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
            .add_plugins(PlanetariumCameraPlugin)
            .add_systems(EguiPrimaryContextPass, (
                (
                    windows::controls::control_window,
                    windows::body_edit::body_edit_window,
                    windows::body_info::body_info_window,
                    windows::settings::settings_window,
                    windows::spin::spin_window,
                    windows::camera::camera_window,

                    label_bodies,
                    ).run_if(in_state(AppState::Planetarium)),
                ))
            .add_systems(Update, (
                (
                    adjust_lights,
                    // scale_distant_objects.after(position_bodies),
                    position_bodies.after(fixed_motive::calculate).after(kepler_motive::calculate).after(newton_motive::calculate),
                    trajectory::render_trajectories,
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

    let distance_scale = view_settings.distance_factor();

    // Calculate the scaled solar system edge distance (1e14m * distance_scale)
    let scaled_solar_system_edge = 1e14 * distance_scale;
    
    for (_, mut light, appearance) in lights.iter_mut() {
        match appearance {
            Appearance::Star(star_ball) => {
                // Set range to reach the scaled solar system edge
                light.range = scaled_solar_system_edge as f32;
                
                // Scale intensity to maintain consistent illumination at the solar system edge
                // Using inverse square law: to maintain same illumination when distance scales by factor S,
                // intensity must scale by S^2
                let intensity_scale_factor = distance_scale * distance_scale;
                light.intensity = star_ball.intensity() * (intensity_scale_factor as f32);
            }
            _ => {} // This probably won't happen but if it does, it's not worth a crash.
        }
    }
}

fn scale_distant_objects(
    camera: Query<&mut Freecam, With<Camera>>,
    mut stars: Query<(&mut Transform, &Appearance)>,
    view_settings: Res<ViewSettings>,
) {
    const MIN_ANGULAR_SIZE: f64 = f64::to_radians(0.1);

    let camera = camera.single().unwrap();
    for (mut transform, appearance) in stars.iter_mut() {
        let star_pos = transform.translation;
        let cam_pos = camera.bevy_pos.as_vec3();
        let dist = star_pos.distance(cam_pos) as f64;

        match appearance {
            Appearance::Star(star_ball) => {
                let rad = star_ball.radius * view_settings.body_scale;
                let angular_size = (rad * 2.0) / dist;

                if angular_size < MIN_ANGULAR_SIZE {
                    let ratio = MIN_ANGULAR_SIZE / angular_size;
                    transform.scale *= ratio as f32;
                }
            }
            Appearance::DebugBall(debug_ball) => {
                let rad = debug_ball.radius * view_settings.body_scale;
                let angular_size = (rad * 2.0) / dist;

                if angular_size < MIN_ANGULAR_SIZE {
                    let ratio = MIN_ANGULAR_SIZE / angular_size;
                    transform.scale *= ratio as f32;
                }
            }
            Appearance::Empty => {}
        }
    }
}

fn position_bodies(
    mut bodies: Query<(&SimulationObject, &mut Transform, &BodyInfo, &BodyState, &Appearance)>,
    camera: Query<&Freecam, With<PlanetariumCamera>>,
    view_settings: Res<ViewSettings>,
) {
    // TODO: Move the origin to the main camera.
    let distance_scale = view_settings.distance_factor();

    let freecam = camera.single().unwrap();

    for (_, mut transform, _, state, appearance) in bodies.iter_mut() {
        // TODO: I doubt any of this works for moonmoons.
        let global_position: DVec3 = if !view_settings.logarithmic_distance_scale || state.current_local_position.is_none() || state.current_primary_position.is_none() {
            state.current_position  // Use calculated position *unless* we are doing logarithmic distance scale and current object has a primary.
        } else {
            let local_position = state.current_local_position.unwrap();
            let primary_position = state.current_primary_position.unwrap();
            primary_position + local_position
        };
        transform.translation = global_position.as_bevy_scaled_cheated(distance_scale, freecam.bevy_pos);

        //let body_scale = view_settings.body_scale_factor(appearance.radius());
        //transform.scale = Vec3::splat(body_scale);
        let body_scale = if view_settings.logarithmic_body_scale {
            mappings::log_scale(appearance.radius(), view_settings.logarithmic_body_base) * view_settings.body_scale
        } else {
            appearance.radius() * view_settings.body_scale
        } as f32;
        transform.scale = Vec3::splat(body_scale);
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
        for (_, transform, body_info) in bodies.iter() {
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

        let time = (universe_file.contents.time.time_julian_days - J2000_JD) * JD_SECONDS; // Convert Julian Days to seconds
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
