use std::collections::HashMap;
use bevy::app::{App, Update};
use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;
use crate::body::appearance::AssetCache;
use crate::body::universe::save::{UniverseFile, UniversePhysics, ViewSettings};
use crate::body::universe::Universe;
use crate::gui::app::AppState;
use crate::gui::menu::UiState;
use crate::body::universe::save::TagState;
use crate::sim::{SimTime, unload_simulation_objects, CalculateTrajectory, BodySelection};
use crate::body::universe;
use crate::body::motive::calculate_body_positions::{self, PhysicsGraph, PositionCache, SimulationPerformanceMetrics};
use crate::body::motive::kepler_motive;
use crate::foundations::time::{Instant, J2000_JD, JD_SECONDS_PER_JULIAN_DAY};
pub(crate) use crate::camera::{PlanetariumCamera, PlanetariumCameraPlugin};
use crate::gui::planetarium::windows::body_info::BodyInfoState;
use crate::presentation::{self, TrajectoryMaterialPlugin, BodyWireframeMaterialPlugin};

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
            .init_resource::<PhysicsGraph>()
            .init_resource::<PositionCache>()
            .init_resource::<SimulationPerformanceMetrics>()
            .add_message::<CalculateTrajectory>()
            .configure_sets(Update, (
                PlanetariumUISet.run_if(in_state(AppState::Planetarium)),
                PlanetariumSimulationSet.run_if(in_state(AppState::Planetarium)),
                PlanetariumLoadingSet.run_if(in_state(AppState::PlanetariumLoading)),
            ))
            .add_plugins(PlanetariumCameraPlugin)
            .add_plugins(TrajectoryMaterialPlugin)
            .add_plugins(BodyWireframeMaterialPlugin)
            .add_systems(EguiPrimaryContextPass, (
                (
                    windows::controls::control_window,
                    windows::body_edit::body_edit_window,
                    windows::body_info::body_info_window,
                    windows::settings::settings_window,
                    windows::spin::spin_window,
                    windows::camera::camera_window,

                    presentation::label_bodies,
                    ).run_if(in_state(AppState::Planetarium)),
                ))
            .add_systems(Update, (
                (
                    presentation::adjust_lights,
                    calculate_body_positions::calculate_body_positions
                        .after(universe::advance_time),
                    kepler_motive::calculate_trajectory,
                    presentation::position_bodies.after(calculate_body_positions::calculate_body_positions),
                    presentation::orient_bodies.after(presentation::position_bodies),
                    // Trajectory mesh systems
                    presentation::spawn_trajectory_meshes_for_bodies,
                    presentation::refresh_precessing_trajectories
                        .before(kepler_motive::calculate_trajectory),
                    presentation::rebuild_trajectory_caches
                        .after(kepler_motive::calculate_trajectory),
                    presentation::build_trajectory_meshes
                        .after(presentation::position_bodies)
                        .after(presentation::rebuild_trajectory_caches),
                    presentation::cleanup_orphaned_trajectory_meshes,
                    // Body wireframe mesh systems
                    presentation::spawn_body_wireframe_meshes,
                    presentation::spawn_terminator_meshes,
                    presentation::update_terminator_meshes
                        .after(presentation::orient_bodies),
                    presentation::cleanup_orphaned_body_wireframes,
                ).in_set(PlanetariumUISet),
                (
                    universe::advance_time,
                ).in_set(PlanetariumSimulationSet),
                (load_assets).in_set(PlanetariumLoadingSet),
            ))
            .add_systems(OnExit(AppState::PlanetariumLoading), initial_trajectories)
            .add_systems(OnExit(AppState::Planetarium), unload_simulation_objects)
        ;


    }
}

fn initial_trajectories(mut calcs: MessageWriter<CalculateTrajectory>) {
    calcs.write(CalculateTrajectory { selection: BodySelection::All });
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

        let time = (universe_file.contents.time.time_julian_days - J2000_JD) * JD_SECONDS_PER_JULIAN_DAY; // Convert Julian Days to seconds
        sim_time.time = Instant::from_seconds_since_j2000(time);
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
