//! Every one of [`lc_client::looks`] on boxes of several sizes, with the camera circling each box
//! in turn and gliding on to the next, forever, while the star turns on its own period: a
//! screensaver that shows each look at every angle of light and view.
//!
//! ```text
//! cargo run -p lc-client --example looks_gallery
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--sizes <m:m:...>` | the boxes' edges, each look wearing every one; 50:100:200 by default |
//! | `--dwell <s>` / `--glide <s>` | seconds circling a box, and gliding to the next; 40 and 8. The camera turns once a box |
//! | `--star-period <s>` | seconds for the star to turn once about the vertical; 120 by default, and best a divisor of an hour, when the shader's clock wraps. 0 holds it still |
//! | `--speed <x>` | the camera's clock against the wall's |
//! | `--time <s>` | where on the camera's clock to start. The star keeps its own |
//! | `--fullscreen` | borderless, on the current monitor |
//! | `--shot <path>` / `--frames <n>` | photograph after `n` frames, 60 by default, and quit |
//!
//! The right and left arrows cut to the next box or the one before, the next look at the same
//! size or the next size's first, keeping the camera's angle and the loop's clock. Escape quits.

use std::f32::consts::TAU;

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{MonitorSelection, WindowMode, WindowResolution};
use lc_client::looks::aztec::{Aztec, AztecMaterial, Windows};
use lc_client::looks::sprue::{Sprue, SprueMaterial};
use lc_client::looks::{LooksPlugin, direction, spin};

/// How far off a box the camera circles, in its edges.
const STANDOFF: f32 = 2.2;

struct Args {
    sizes: Vec<f32>,
    dwell: f32,
    glide: f32,
    star_period: f32,
    speed: f32,
    time: f32,
    fullscreen: bool,
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
            dwell: value("--dwell").unwrap_or(40.0),
            glide: value("--glide").unwrap_or(8.0),
            star_period: value("--star-period").unwrap_or(120.0),
            speed: value("--speed").unwrap_or(1.0),
            time: value("--time").unwrap_or(0.0),
            fullscreen: args.iter().any(|a| a == "--fullscreen"),
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

/// Boxes the arrows have moved the loop on by.
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
        .add_plugins(LooksPlugin)
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Setup(args))
        .init_resource::<Skipped>()
        .add_systems(Startup, spawn)
        .add_systems(Update, (skip, orbit, photograph).chain())
        .run();
}

fn spawn(
    mut commands: Commands,
    setup: Res<Setup>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sprues: ResMut<Assets<SprueMaterial>>,
    mut aztecs: ResMut<Assets<AztecMaterial>>,
) {
    let args = &setup.0;
    let to_star = direction(25.0, -60.0);
    let star_spin = spin(args.star_period);

    let sprue = Sprue { to_star, star_spin, ..default() };
    let sprue_layers = sprue.materials(&sprue.bake(), &mut images).map(|m| sprues.add(m));
    let aztec = aztecs.add(Aztec { to_star, star_spin, unit_box: true, ..default() }.material());
    let offices = aztecs.add(Aztec { to_star, star_spin, windows: Some(Windows::default()), unit_box: true, ..default() }.material());

    // A row a look, a column a size, far enough apart that a box's neighbors stay small behind it.
    let largest = args.sizes.iter().copied().fold(0.0, f32::max);
    let mut columns = Vec::new();
    let mut x = 0.0;
    for (i, &size) in args.sizes.iter().enumerate() {
        if i > 0 {
            x += 2.0 * STANDOFF * (args.sizes[i - 1] + size);
        }
        columns.push(x);
    }
    let rows = [0.0, 1.0, 2.0].map(|row| row * 3.0 * STANDOFF * largest);
    let mut stations = Vec::new();
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
            } else {
                commands.spawn((
                    Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
                    MeshMaterial3d(if look == 1 { aztec.clone() } else { offices.clone() }),
                    Transform::from_translation(center).with_scale(Vec3::splat(size)),
                ));
            }
            stations.push((center, size));
        }
    }
    commands.insert_resource(Stations(stations));

    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 40f32.to_radians(),
            near: 0.1,
            far: 10.0 * (columns.last().copied().unwrap_or(0.0) + rows[2] + largest),
            ..default()
        }),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
    ));
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
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
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
