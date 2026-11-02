//! Photograph azteced sheet metal on a box in a void, lit by a star and a planet: one of
//! [`lc_client::looks`].
//!
//! ```text
//! cargo run -p lc-client --example aztec_void -- /tmp/aztec.png --yaw 35 --pitch 20 --dist 110
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--size <m>` | the box's edge; 48 by default |
//! | `--yaw <deg>` / `--pitch <deg>` / `--dist <m>` | the camera, orbiting `--target` |
//! | `--target <x:y:z>` | what the camera looks at, meters from the box's center |
//! | `--fov <deg>` | vertical field of view; 40 by default |
//! | `--sun <deg>` / `--sun-yaw <deg>` / `--star <x>` | the star's direction, and its power |
//! | `--planet-at <deg:deg>` / `--planet-deg <deg>` / `--planet <x>` | the planet's elevation and yaw, its angular radius, and its radiance; 0 is no planet |
//! | `--cell <m>` / `--min-panel <m>` / `--snap <m>` | the staggered grid before splitting, the smallest panel side, and the grid panel edges snap to |
//! | `--chamfer <m>` / `--lean <deg>` | each wall of a seam's trench: its width across the face, and how far its normal leans toward the seam; 0.15 and 30 |
//! | `--tilt <deg>` | how far a panel's normal may tip off the face's; 5 by default |
//! | `--brown <share>` | of panels a little browner; 0.2 by default |
//! | `--roughness <x>` / `--metallic <x>` | the finish, shared by every panel |
//! | `--windows` | office windows on the sides, in decks along Y |
//! | `--exposure <stops>` | open the exposure |
//! | `--seed <n>` | the layout |
//! | `--frames <n>` | frames before the photograph |
//! | `--burst <n>` / `--sweep <deg>` | photograph `n` frames, numbered, swinging the yaw through `sweep` across them: how the panels flash |

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::WindowResolution;
use lc_client::looks::aztec::{Aztec, AztecMaterial, Windows};
use lc_client::looks::{LooksPlugin, direction};

struct Args {
    path: String,
    size: f32,
    yaw: f32,
    pitch: f32,
    dist: f32,
    target: Vec3,
    fov: f32,
    sun: f32,
    sun_yaw: f32,
    star: f32,
    planet_at: Vec2,
    planet_deg: f32,
    planet: f32,
    cell: f32,
    min_panel: f32,
    snap: f32,
    chamfer: f32,
    lean: f32,
    tilt: f32,
    brown: f32,
    roughness: f32,
    metallic: f32,
    exposure: f32,
    seed: u32,
    windows: bool,
    frames: u32,
    burst: u32,
    sweep: f32,
}

impl Args {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let raw = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let value = |flag: &str| {
            raw(flag).map(|v| v.parse::<f32>().unwrap_or_else(|_| panic!("{flag} takes a number")))
        };
        let path = args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| {
            eprintln!("aztec_void: give a PNG path to write");
            std::process::exit(2);
        });
        let triple = |flag: &str| {
            raw(flag).map(|t| t.split(':').map(|c| c.parse::<f32>().unwrap_or_else(|_| panic!("{flag} takes numbers joined by :"))).collect::<Vec<_>>())
        };
        let target = triple("--target").map_or(Vec3::ZERO, |v| Vec3::new(v[0], v[1], v[2]));
        let planet_at = triple("--planet-at").map_or(Vec2::new(-15.0, 150.0), |v| Vec2::new(v[0], v[1]));
        let d = Aztec::default();
        Self {
            path,
            size: value("--size").unwrap_or(48.0),
            yaw: value("--yaw").unwrap_or(35.0),
            pitch: value("--pitch").unwrap_or(20.0),
            dist: value("--dist").unwrap_or(110.0),
            target,
            fov: value("--fov").unwrap_or(40.0),
            sun: value("--sun").unwrap_or(30.0),
            sun_yaw: value("--sun-yaw").unwrap_or(-50.0),
            star: value("--star").unwrap_or(d.star),
            planet_at,
            planet_deg: value("--planet-deg").unwrap_or(d.planet_deg),
            planet: value("--planet").unwrap_or(d.planet),
            cell: value("--cell").unwrap_or(d.cell_m),
            min_panel: value("--min-panel").unwrap_or(d.min_panel_m),
            snap: value("--snap").unwrap_or(d.snap_m),
            chamfer: value("--chamfer").unwrap_or(d.chamfer_m),
            lean: value("--lean").unwrap_or(d.lean_deg),
            tilt: value("--tilt").unwrap_or(d.tilt_deg),
            brown: value("--brown").unwrap_or(d.brown),
            roughness: value("--roughness").unwrap_or(d.roughness),
            metallic: value("--metallic").unwrap_or(d.metallic),
            exposure: value("--exposure").unwrap_or(0.0),
            windows: args.iter().any(|a| a == "--windows"),
            seed: raw("--seed").map_or(d.seed, |v| v.parse().expect("--seed takes a whole number")),
            frames: value("--frames").unwrap_or(30.0) as u32,
            burst: value("--burst").unwrap_or(1.0).max(1.0) as u32,
            sweep: value("--sweep").unwrap_or(0.0),
        }
    }
}

fn main() {
    let args = Args::parse();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Lightcone Frontier: azteced plate in a void".into(),
                        resolution: WindowResolution::new(1280, 720).with_scale_factor_override(1.0),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .add_plugins(LooksPlugin)
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Setup(args))
        .add_systems(Startup, spawn)
        .add_systems(Update, (orbit, photograph).chain())
        .run();
}

#[derive(Resource)]
struct Setup(Args);

fn spawn(
    mut commands: Commands,
    setup: Res<Setup>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<AztecMaterial>>,
) {
    let args = &setup.0;
    let material = materials.add(
        Aztec {
            cell_m: args.cell,
            min_panel_m: args.min_panel,
            snap_m: args.snap,
            chamfer_m: args.chamfer,
            lean_deg: args.lean,
            tilt_deg: args.tilt,
            brown: args.brown,
            roughness: args.roughness,
            metallic: args.metallic,
            to_star: direction(args.sun, args.sun_yaw),
            star: args.star,
            star_spin: 0.0,
            to_planet: direction(args.planet_at.x, args.planet_at.y),
            planet_deg: args.planet_deg,
            planet: args.planet,
            exposure_stops: args.exposure,
            seed: args.seed,
            windows: args.windows.then(Windows::default),
            unit_box: true,
        }
        .material(),
    );
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(material),
        Transform::from_scale(Vec3::splat(args.size)),
    ));

    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: args.fov.to_radians(),
            near: 0.1,
            far: 10.0 * (args.dist + args.size),
            ..default()
        }),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
    ));
}

/// Which of the burst's photographs frame `drawn` is, held at either end.
fn burst_index(args: &Args, drawn: u32) -> u32 {
    drawn.saturating_sub(args.frames).min(args.burst - 1)
}

fn orbit(setup: Res<Setup>, mut drawn: Local<u32>, mut cameras: Query<&mut Transform, With<Camera3d>>) {
    let args = &setup.0;
    *drawn += 1;
    let share = if args.burst > 1 { burst_index(args, *drawn) as f32 / (args.burst - 1) as f32 - 0.5 } else { 0.0 };
    let (yaw, pitch) = ((args.yaw + share * args.sweep).to_radians(), args.pitch.to_radians());
    let eye = args.target + args.dist * Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos());
    for mut transform in &mut cameras {
        *transform = Transform::from_translation(eye).looking_at(args.target, Vec3::Y);
    }
}

fn photograph(mut commands: Commands, setup: Res<Setup>, mut drawn: Local<u32>, mut exit: MessageWriter<AppExit>) {
    let args = &setup.0;
    *drawn += 1;
    if (args.frames..args.frames + args.burst).contains(&*drawn) {
        let path = match (args.burst > 1, args.path.rsplit_once('.')) {
            (false, _) => args.path.clone(),
            (true, Some((stem, extension))) => format!("{stem}.{:03}.{extension}", burst_index(args, *drawn)),
            (true, None) => format!("{}.{:03}", args.path, burst_index(args, *drawn)),
        };
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if *drawn > args.frames + args.burst + 30 {
        exit.write(AppExit::Success);
    }
}
