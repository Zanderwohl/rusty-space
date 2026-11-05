//! Two ships wearing every look, then every one of [`lc_client::looks`] on boxes of several sizes,
//! with the camera circling each in turn and gliding on to the next, turning to face it as it
//! leaves, forever, while the star turns on its own period: a screensaver that shows each look at
//! every angle of light and view.
//!
//! The ships are the starting form and A Frank Exchange of Views, a player's ship copied off the
//! dev shard into `assets/forms`. They wear the game's hull material, which takes no star spin, so
//! their star holds still where the boxes' was at the start.
//!
//! ```text
//! cargo run -p lc-client --example looks_gallery
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--sizes <m:m:...>` | the boxes' edges, each look wearing every one; 50:100:200 by default |
//! | `--ovoid <m>` | the length of the habitat ovoid visited after the boxes, 10 long to 5 wide to 2 tall; 300 by default |
//! | `--dwell <s>` / `--glide <s>` | seconds circling a box, and gliding to the next; 40 and 8. The camera turns once a box |
//! | `--star-period <s>` | seconds for the star to turn once about the vertical; 120 by default, and best a divisor of an hour, when the shader's clock wraps. 0 holds it still |
//! | `--speed <x>` | the camera's clock against the wall's |
//! | `--time <s>` | where on the camera's clock to start. The star keeps its own |
//! | `--apart` | each of the ships' parts its own mesh, overlapping its neighbors rather than fused with them |
//! | `--hard` | the ships' parts meet as their bare union, no fillet and no mixing of their looks |
//! | `--fullscreen` | borderless, on the current monitor |
//! | `--shot <path>` / `--frames <n>` | photograph after `n` frames, 60 by default, and quit |
//!
//! The right and left arrows cut to the next station or the one before, keeping the camera's angle and the loop's clock. Escape quits.

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{MonitorSelection, WindowMode, WindowResolution};
use em_render::hull_material::{FACES, HullMaterial, HullMaterialPlugin, HullUniform, REGIONS};
use em_render::body_surface_material::BodySurfaceMaterial;
use em_render::population_material::PopulationMaterial;
use em_render::relativistic_starfield_material::RelativisticStarfieldMaterial;
use lc_client::hull_mesh::{ENGINE_FLANK, Finish, HullForm, HullMeshPlugin, Joins, REGION_GRAPHS, region};
use lc_client::looks::kinds::{HullLooksPlugin, living_roof, wear};
use lc_client::procedural::ProceduralTexturesPlugin;
use lc_client::ship_hull::{HullPalettePlugin, Palette, request_palette};
use lc_client::tonemap::ToneMap;
use lc_world::fitting::Balance;
use lc_world::form::sdf::Sdf;
use lc_world::form::{Form, Kind, SparMode};
use lc_client::looks::aztec::{Aztec, AztecMaterial, Windows};
use lc_client::looks::obsidian::{Obsidian, ObsidianMaterial};
use lc_client::looks::outline::RoofOutline;
use lc_client::looks::sprue::{Sprue, SprueMaterial};
use lc_client::looks::{LooksPlugin, direction, spin};

/// How far off a box the camera circles, in its edges.
const STANDOFF: f32 = 2.2;
/// The share of a glide the camera takes to turn from the box it leaves to the one it heads for.
const TURN: f32 = 0.2;
/// `mesh_void`'s hull exposure and lamps, which are placed for a lit albedo of 0.3.
const LIT_WINDOW: f32 = 0.0075;
const LIT_HULL: f32 = 0.3;
const HULL_STOPS: f32 = -1.5;
/// The hull graphs' unit square, meters.
const TILE_M: f32 = 64.0;

const FRANK: &str = include_str!("../assets/forms/a-frank-exchange-of-views.ron");

struct Args {
    sizes: Vec<f32>,
    ovoid: f32,
    dwell: f32,
    glide: f32,
    star_period: f32,
    speed: f32,
    time: f32,
    fullscreen: bool,
    hard: bool,
    apart: bool,
    shot: Option<String>,
    frames: u32,
}

impl Args {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let raw = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let value = |flag: &str| {
            raw(flag).map(|v| v.parse::<f32>().unwrap_or_else(|_| panic!("{flag} takes a number")))
        };
        let sizes = raw("--sizes").map_or(vec![50.0, 100.0, 200.0], |t| {
            t.split(':').map(|c| c.parse().expect("--sizes is meters joined by :")).collect()
        });
        Self {
            sizes,
            ovoid: value("--ovoid").unwrap_or(300.0),
            dwell: value("--dwell").unwrap_or(40.0),
            glide: value("--glide").unwrap_or(8.0),
            star_period: value("--star-period").unwrap_or(120.0),
            speed: value("--speed").unwrap_or(1.0),
            time: value("--time").unwrap_or(0.0),
            fullscreen: args.iter().any(|a| a == "--fullscreen"),
            hard: args.iter().any(|a| a == "--hard"),
            apart: args.iter().any(|a| a == "--apart"),
            shot: raw("--shot"),
            frames: value("--frames").unwrap_or(60.0) as u32,
        }
    }
}

#[derive(Resource)]
struct Setup(Args);

/// The boxes the camera visits, in order.
#[derive(Resource)]
struct Stations(Vec<(Vec3, f32)>);

/// The ships' forms and where each stands, meshed once the hull palette lands.
#[derive(Resource)]
struct Ships {
    waiting: Vec<(Form, Vec3)>,
    to_star: Vec3,
    joins: Joins,
    apart: bool,
}

/// Stations the arrows have moved the loop on by.
#[derive(Resource, Default)]
struct Skipped(i64);

fn main() {
    let args = Args::parse();
    let mode = if args.fullscreen { WindowMode::BorderlessFullscreen(MonitorSelection::Current) } else { WindowMode::Windowed };
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Lightcone Frontier: looks".into(),
                        resolution: WindowResolution::new(1280, 720).with_scale_factor_override(1.0),
                        mode,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .init_asset::<PopulationMaterial>()
        .init_asset::<BodySurfaceMaterial>()
        .init_asset::<RelativisticStarfieldMaterial>()
        .add_plugins((LooksPlugin, ProceduralTexturesPlugin, HullMaterialPlugin, HullMeshPlugin, HullLooksPlugin, HullPalettePlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Setup(args))
        .init_resource::<Skipped>()
        .add_systems(Startup, (spawn, request_palette))
        .add_systems(Update, ((skip, orbit, photograph).chain(), spawn_ships))
        .run();
}

fn spawn(
    mut commands: Commands,
    setup: Res<Setup>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sprues: ResMut<Assets<SprueMaterial>>,
    mut aztecs: ResMut<Assets<AztecMaterial>>,
    mut obsidians: ResMut<Assets<ObsidianMaterial>>,
) {
    let args = &setup.0;
    let to_star = direction(25.0, -60.0);
    let star_spin = spin(args.star_period);

    let sprue = Sprue { to_star, star_spin, ..default() };
    let sprue_layers = sprue.materials(&sprue.bake(), &mut images).map(|m| sprues.add(m));
    let aztec = aztecs.add(Aztec { to_star, star_spin, unit_box: true, ..default() }.material());
    let offices = aztecs.add(Aztec { to_star, star_spin, windows: Some(Windows::default()), unit_box: true, ..default() }.material());
    let mut habitat = |unit_box| aztecs.add(Aztec { to_star, star_spin, unit_box, ..Aztec::habitat() }.material());
    let (habitat_box, habitat_ovoid) = (habitat(true), habitat(false));
    let obsidian = obsidians.add(Obsidian { to_star, star_spin, ..default() }.material());

    // A row a look, a column a size, far enough apart that a box's neighbors stay small behind it.
    let largest = args.sizes.iter().copied().fold(0.0, f32::max);
    // Before the first column, the first ship farthest, so the second glides on to the boxes.
    let ships: Vec<(Form, f32)> = [Form::starting(), frank()]
        .into_iter()
        .map(|form| {
            let (lo, hi) = Sdf::new(&form, &Balance::DEFAULT).expect("the ships are valid forms").bounds();
            // A box's edge from its diagonal, so a ship is framed as a box its size would be.
            let size = (hi - lo).length() as f32 / 3f32.sqrt();
            (form, size)
        })
        .collect();
    let mut ship_x = vec![0.0; ships.len()];
    let (mut x, mut beside) = (0.0, args.sizes.first().copied().unwrap_or(0.0));
    for (k, (_, size)) in ships.iter().enumerate().rev() {
        x -= 2.0 * STANDOFF * (beside + size);
        ship_x[k] = x;
        beside = *size;
    }
    let mut stations: Vec<(Vec3, f32)> = ships.iter().zip(&ship_x).map(|((_, size), &x)| (Vec3::new(x, 0.0, 0.0), *size)).collect();
    commands.insert_resource(Ships { waiting: ships.into_iter().zip(&stations).map(|((form, _), s)| (form, s.0)).collect(), to_star, joins: if args.hard { Joins::Hard } else { Joins::Blended }, apart: args.apart });
    let leftmost = ship_x.first().copied().unwrap_or(0.0);

    let mut columns = Vec::new();
    let mut x = 0.0;
    for (i, &size) in args.sizes.iter().enumerate() {
        if i > 0 {
            x += 2.0 * STANDOFF * (args.sizes[i - 1] + size);
        }
        columns.push(x);
    }
    let rows = [0.0, 1.0, 2.0, 3.0, 4.0].map(|row| row * 3.0 * STANDOFF * largest);
    for (&size, &x) in args.sizes.iter().zip(&columns) {
        for (look, &z) in rows.iter().enumerate() {
            let center = Vec3::new(x, 0.0, z);
            if look == 0 {
                for (k, layer) in sprue_layers.iter().enumerate() {
                    let edge = size + 2.0 * Sprue::lift(k);
                    commands.spawn((
                        Mesh3d(meshes.add(Cuboid::new(edge, edge, edge))),
                        MeshMaterial3d(layer.clone()),
                        Transform::from_translation(center),
                    ));
                }
            } else if look == 4 {
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(size, size, size))),
                    MeshMaterial3d(obsidian.clone()),
                    Transform::from_translation(center),
                ));
            } else {
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
                    MeshMaterial3d([&aztec, &offices, &habitat_box][look - 1].clone()),
                    Transform::from_translation(center).with_scale(Vec3::splat(size)),
                ));
            }
            stations.push((center, size));
        }
    }
    // Past the last column, in the middle of the rows, its long axis along x.
    let length = args.ovoid;
    let ovoid_at = Vec3::new(columns.last().copied().unwrap_or(0.0) + 2.0 * STANDOFF * (largest + length), 0.0, 0.5 * rows[4]);
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(128, 64))),
        MeshMaterial3d(habitat_ovoid),
        RoofOutline,
        Transform::from_translation(ovoid_at).with_scale(Vec3::new(0.5, 0.1, 0.25) * length),
    ));
    stations.push((ovoid_at, 0.6 * length));
    commands.insert_resource(Stations(stations));

    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 40f32.to_radians(),
            near: 0.1,
            far: 10.0 * (ovoid_at.x - leftmost + rows[4] + length),
            ..default()
        }),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
    ));
}

fn frank() -> Form {
    let preset: lc_proto::Preset = ron::from_str(FRANK).expect("assets/forms holds a preset");
    Form::from(&preset.form)
}

/// The ships, under the game's hull material, once its palette has baked.
fn spawn_ships(
    mut commands: Commands,
    mut ships: ResMut<Ships>,
    palette: Res<Palette>,
    mut materials: ResMut<Assets<HullMaterial>>,
) {
    if ships.waiting.is_empty() {
        return;
    }
    let Some((albedo, lights)) = palette.ready() else { return };
    let mut emitted = [Vec4::ZERO; REGIONS];
    for (slot, kind) in emitted.iter_mut().zip(REGION_GRAPHS) {
        *slot = match kind {
            "drone" => LIT_WINDOW * Vec3::new(0.85, 0.92, 1.0),
            "living" => LIT_WINDOW * Vec3::new(1.0, 0.8, 0.55),
            "engine" => 8.0 * LIT_WINDOW * Vec3::new(0.75, 0.85, 1.0),
            "mind" => 0.3 * LIT_WINDOW * Vec3::new(0.6, 0.9, 1.0),
            "bay" => LIT_WINDOW * Vec3::new(1.0, 0.92, 0.8),
            "engine_flank" => LIT_WINDOW * Vec3::new(1.0, 0.72, 0.2),
            _ => Vec3::ZERO,
        }
        .extend(0.0);
    }
    let material = materials.add(HullMaterial {
        uniforms: wear(HullUniform {
            // Enough fill to see an unlit side against the black.
            to_star: ships.to_star.extend(0.08),
            exposure: Vec4::new(LIT_HULL * 2f32.powf(-HULL_STOPS), ToneMap::default().surface_stops, 0.0, 0.0),
            detail: Vec4::new(TILE_M, if ships.joins == Joins::Hard { 1.0 } else { 0.0 }, 0.0, 0.0),
            bolted: 1 << region(Kind::Spar(SparMode::Saddle)),
            emitted,
            open: UVec4::new(region(Kind::Engine), ENGINE_FLANK, 1, 0),
            faces: [(8.0 * LIT_WINDOW * Vec3::new(0.75, 0.85, 1.0)).extend(0.0); FACES],
            ..default()
        }),
        albedo,
        lights,
        roof_map: None,
        roof_outline: default(),
        pipe_tiles: None,
        pipe_light: None,
    });
    // From the ship's frame, z up, to Bevy's, y up.
    let ship_to_world = Quat::from_rotation_x(-FRAC_PI_2);
    let (joins, apart) = (ships.joins, ships.apart);
    for (form, at) in ships.waiting.drain(..) {
        let sdf = Sdf::new(&form, &Balance::DEFAULT).expect("checked at startup");
        let (lo, hi) = sdf.bounds();
        let transform = Transform::from_translation(at - ship_to_world * ((lo + hi) / 2.0).as_vec3()).with_rotation(ship_to_world);
        // A piece is meshed in the ship's frame, so every piece shares the ship's transform.
        let hulls: Vec<HullForm> = if apart {
            lc_client::hull_mesh::apart(&form, &Balance::DEFAULT)
                .expect("checked at startup")
                .into_iter()
                .map(|source| HullForm { source, finish: Finish::Smooth, cells: None, joins })
                .collect()
        } else {
            vec![HullForm { joins, ..HullForm::form(form, Balance::DEFAULT, Finish::Smooth) }]
        };
        for hull in hulls {
            commands.spawn((living_roof(), hull, MeshMaterial3d(material.clone()), transform));
        }
    }
}

fn skip(keys: Res<ButtonInput<KeyCode>>, mut skipped: ResMut<Skipped>, mut exit: MessageWriter<AppExit>) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        skipped.0 += 1;
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        skipped.0 -= 1;
    }
}

fn orbit(
    setup: Res<Setup>,
    stations: Res<Stations>,
    skipped: Res<Skipped>,
    time: Res<Time>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let args = &setup.0;
    let stations = &stations.0;
    let t = args.time + time.elapsed_secs() * args.speed;
    let segment = args.dwell + args.glide;
    let visit = (t / segment).floor();
    let i = (visit as i64 + skipped.0).rem_euclid(stations.len() as i64) as usize;
    let next = (i + 1) % stations.len();
    let into_glide = ((t - visit * segment - args.dwell) / args.glide.max(1.0e-3)).clamp(0.0, 1.0);
    let blend = into_glide * into_glide * (3.0 - 2.0 * into_glide);

    let (from, to) = (stations[i], stations[next]);
    let target = from.0.lerp(to.0, blend);
    let dist = STANDOFF * (from.1 + (to.1 - from.1) * blend);
    let yaw = TAU * t / segment;
    // Off a whole number of turns, so the pitch's swing does not line up with the stations.
    let pitch = (15.0 + 30.0 * (TAU * t / (2.7 * segment)).sin()).to_radians();
    let eye = target + dist * Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos());
    // Looking at `target` mid-glide is looking at the empty space between boxes.
    let turn = (into_glide / TURN).min(1.0);
    let turn = turn * turn * (3.0 - 2.0 * turn);
    let ahead = Transform::from_translation(eye).looking_at(target, Vec3::Y).rotation;
    let onward = Transform::from_translation(eye).looking_at(to.0, Vec3::Y).rotation;
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(eye).with_rotation(ahead.slerp(onward, turn));
    }
}

fn photograph(mut commands: Commands, setup: Res<Setup>, mut drawn: Local<u32>, mut exit: MessageWriter<AppExit>) {
    let args = &setup.0;
    let Some(path) = &args.shot else { return };
    *drawn += 1;
    if *drawn == args.frames {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path.clone()));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if *drawn > args.frames + 30 {
        exit.write(AppExit::Success);
    }
}
