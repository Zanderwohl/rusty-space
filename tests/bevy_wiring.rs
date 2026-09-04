//! The Bevy side of Phase 6, exercised without a window.
//!
//! The library suite covers `em-sim` thoroughly, but nothing in it runs a Bevy schedule.
//! These tests build a real `App`, register the sync systems, and run frames — so a
//! conflicting system parameter, a missing resource, or a sync that reads the wrong body
//! fails here rather than the first time someone opens the planetarium.
//!
//! Spawning is not covered: `sync_body_entities` builds meshes and materials, which needs
//! a GPU. Entities are created directly instead, which is what that system would have
//! produced.

use bevy::prelude::*;
use exotic_matters::body::universe::save::ViewSettings;
use exotic_matters::camera::{Freecam, PlanetariumCamera};
use exotic_matters::gui::settings::Settings;
use exotic_matters::sim::world::{advance_simulation, calculate_trajectories, sync_rotations,
                                 sync_transforms, BodyRef, SimMetrics, SimSystem, Trajectories};
use exotic_matters::sim::{BodySelection, CalculateTrajectory, SimTime};
use em_foundations::time::Instant;
use em_sim::presets::solar_system;
use em_sim::system::System;

/// An app with the simulation loaded and one entity viewing each body.
fn app_with_system() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    let system = System::from_contents(&solar_system()).expect("bundled system");
    let ids: Vec<_> = system.iter_ids().collect();

    app.insert_resource(SimSystem(system))
        .init_resource::<SimTime>()
        .init_resource::<SimMetrics>()
        .init_resource::<ViewSettings>()
        .init_resource::<Settings>()
        .init_resource::<Trajectories>()
        .add_message::<CalculateTrajectory>();

    // A camera, which `sync_transforms` needs to make positions relative to.
    app.world_mut().spawn((
        PlanetariumCamera::new(),
        Freecam { bevy_pos: bevy::math::DVec3::ZERO },
        Transform::default(),
    ));
    for id in ids {
        app.world_mut().spawn((BodyRef(id), Transform::default()));
    }

    app.add_systems(Update, (
        advance_simulation,
        calculate_trajectories.after(advance_simulation),
        sync_transforms.after(advance_simulation),
        sync_rotations.after(sync_transforms),
    ));
    app
}

/// The schedule has to actually run. Bevy validates system parameters when it does, so a
/// conflicting query or an uninitialised resource surfaces here.
#[test]
fn the_planetarium_schedule_runs() {
    let mut app = app_with_system();
    for _ in 0..5 {
        app.update();
    }
    let metrics = app.world().resource::<SimMetrics>();
    assert!(metrics.bodies > 200, "expected the full system, got {}", metrics.bodies);
}

/// With nothing to integrate the whole system is analytic, so it should evaluate directly
/// rather than walking there — that is what makes scrubbing cheap.
#[test]
fn a_fully_analytic_system_jumps_rather_than_stepping() {
    let mut app = app_with_system();
    app.world_mut().resource_mut::<SimTime>().time =
        Instant::from_julian_day(2460676.5);
    app.update();

    let metrics = app.world().resource::<SimMetrics>();
    assert!(metrics.analytic_jump, "a system with no Newtonian bodies should not step");
    assert_eq!(metrics.steps, 0);
    assert_eq!(metrics.newtonian_bodies, 0);
}

/// Every entity must end up at its own body's position, not some other body's. A
/// mis-resolved `BodyRef` would leave transforms plausible but wrong.
#[test]
fn transforms_track_the_right_body() {
    use exotic_matters::presentation::render_space::ToRender;

    let mut app = app_with_system();
    app.world_mut().resource_mut::<SimTime>().time = Instant::from_julian_day(2460676.5);
    app.update();

    let scale = app.world().resource::<ViewSettings>().distance_factor();

    // Snapshot what each entity is showing, then compare against the arena.
    let mut query = app.world_mut().query::<(&BodyRef, &Transform)>();
    let seen: Vec<(BodyRef, Vec3)> = query
        .iter(app.world())
        .map(|(body, transform)| (*body, transform.translation))
        .collect();

    let system = &app.world().resource::<SimSystem>().0;
    for (body, translation) in &seen {
        let i = system.index_of(body.0).expect("every ref must resolve");
        let expected = system.position(i).to_render_relative(scale, bevy::math::DVec3::ZERO);
        let delta = (*translation - expected).length();
        assert!(delta < 1.0e-3, "{} is at {translation:?}, expected {expected:?}", system.name(i));
    }
    assert!(seen.len() > 200, "only checked {} entities", seen.len());
}

/// Advancing the clock must move the bodies.
#[test]
fn advancing_the_clock_moves_things() {
    let mut app = app_with_system();
    app.update();

    let earth = {
        let s = &app.world().resource::<SimSystem>().0;
        s.by_name("Earth").unwrap()
    };
    let before = app.world().resource::<SimSystem>().0.position(earth);

    app.world_mut().resource_mut::<SimTime>().time = Instant::from_julian_day(2451545.0 + 180.0);
    app.update();

    let after = app.world().resource::<SimSystem>().0.position(earth);
    let moved = (after - before).length();
    // Half a year: Earth should be most of an orbit diameter away.
    assert!(moved > 2.0e11, "Earth moved only {moved:e} m in half a year");
}

/// A trajectory request must produce a drawable path for the requested body.
#[test]
fn trajectories_are_produced_on_request() {
    let mut app = app_with_system();
    app.update();

    app.world_mut().write_message(CalculateTrajectory {
        selection: BodySelection::IDs(vec!["Earth".into()]),
    });
    app.update();

    let id = em_sim::id::BodyId::from_name("Earth");
    let trajectories = app.world().resource::<Trajectories>();
    let path = trajectories.0.get(&id).expect("Earth should have a trajectory");
    assert!(path.len() > 8, "trajectory has only {} samples", path.len());
    assert!(path.periodicity().is_some(), "a closed orbit should be marked periodic");
}
