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
    assert!(path.points.len() > 8, "trajectory has only {} samples", path.points.len());
    assert!(path.closed, "a closed orbit should be marked closed");
    assert!(path.period.to_seconds() > 0.0, "a closed orbit needs a period");
}

/// The bundled save must load through the real file path and into the arena.
///
/// `System::from_contents` is covered above against the generated preset, which is
/// built in memory and so proves nothing about SQLite. This drives the path the app
/// actually takes — `.em` on disk, through the migrations and the row decoders, into
/// the arena — and then propagates it, because elements that decode but do not
/// propagate are the failure mode that matters.
#[test]
fn the_bundled_save_loads_and_propagates() {
    use exotic_matters::body::universe::save::UniverseFile;
    use std::path::PathBuf;

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/systems/solar_system.em");
    let file = UniverseFile::load_from_path(&path).expect("the bundled save should load");
    let mut system = System::from_contents(&file.contents).expect("and build a system");

    assert!(system.len() > 150, "only {} bodies came back", system.len());

    // Every body must resolve a finite position a decade out, not just at the epoch.
    em_sim::propagate::evaluate_at(&mut system, Instant::from_julian_day(2451545.0 + 3652.5));
    for i in system.indices() {
        let p = system.position(i);
        assert!(p.is_finite(), "{} propagated to {p:?}", system.name(i));
    }

    // And the hierarchy has to be real: Luna must stay near Earth, not near the Sun.
    let earth = system.by_name("Earth").expect("Earth");
    let luna = system.by_name("Luna").expect("Luna");
    let separation = (system.position(luna) - system.position(earth)).length();
    assert!(
        (3.0e8..5.0e8).contains(&separation),
        "Luna is {separation:e} m from Earth"
    );
}

/// A body added to the arena must acquire an entity, and that bare entity must then be
/// dressed by the presentation systems.
///
/// This is the contract `sync_body_entities` relies on: it attaches nothing but a
/// `BodyRef`, on the understanding that anything keying off `BodyRef` and the absence of
/// its own link component will pick the body up. If that stopped holding, the app would
/// compile and run and simply draw nothing.
#[test]
fn bodies_get_entities_and_then_get_dressed() {
    use bevy::asset::AssetPlugin;
    use exotic_matters::presentation::{
        spawn_body_point_meshes, spawn_body_wireframe_meshes, BodyPointLink, BodyPointMaterial,
        BodyWireframeLink, BodyWireframeMaterial,
    };
    use exotic_matters::sim::world::{sync_body_entities, BodyEntities};

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Mesh>()
        .init_asset::<BodyWireframeMaterial>()
        .init_asset::<BodyPointMaterial>();

    let system = System::from_contents(&solar_system()).expect("bundled system");
    let expected = system.len();
    app.insert_resource(SimSystem(system))
        .init_resource::<BodyEntities>();

    app.add_systems(Update, (
        sync_body_entities,
        spawn_body_wireframe_meshes.after(sync_body_entities),
        spawn_body_point_meshes.after(sync_body_entities),
    ));

    // Two frames: the first spawns the entities, the second dresses them, because
    // `Commands` do not apply until the sync point.
    app.update();
    app.update();

    let tracked = app.world().resource::<BodyEntities>().map.len();
    assert_eq!(tracked, expected, "every body should own exactly one entity");

    let mut refs = app.world_mut().query::<&BodyRef>();
    assert_eq!(refs.iter(app.world()).count(), expected);

    // Every DebugBall body must have picked up both a wireframe and a point sprite.
    let ball_count = {
        let system = &app.world().resource::<SimSystem>().0;
        system.indices()
            .filter(|i| matches!(system.appearance(*i), em_sim::appearance::Appearance::DebugBall(_)))
            .count()
    };
    assert!(ball_count > 100, "only {ball_count} DebugBall bodies to dress");

    let mut wireframed = app.world_mut().query::<(&BodyRef, &BodyWireframeLink)>();
    assert_eq!(wireframed.iter(app.world()).count(), ball_count);
    let mut pointed = app.world_mut().query::<(&BodyRef, &BodyPointLink)>();
    assert_eq!(pointed.iter(app.world()).count(), ball_count);

    // Running again must not dress anything twice — the link component is the guard.
    app.update();
    let mut wireframed = app.world_mut().query::<(&BodyRef, &BodyWireframeLink)>();
    assert_eq!(wireframed.iter(app.world()).count(), ball_count, "dressed twice");
}
