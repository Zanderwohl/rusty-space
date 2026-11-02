//! Photograph a layered sprue on a box in a void: an opaque plated base under two cutout layers
//! of mazed pipe, each flat and normal-mapped, standing off one another and lit from the gaps by
//! sodium lamps: one of [`lc_client::looks`].
//!
//! ```text
//! cargo run -p lc-client --example sprue_void -- /tmp/sprue.png --yaw 35 --pitch 20 --dist 110
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--size <m>` | the box's edge; 48 by default, two tiles |
//! | `--yaw <deg>` / `--pitch <deg>` / `--dist <m>` | the camera, orbiting `--target` |
//! | `--target <x:y:z>` | what the camera looks at, meters from the box's center |
//! | `--fov <deg>` | vertical field of view; 40 by default |
//! | `--sun <deg>` / `--sun-yaw <deg>` / `--star <x>` | the star's direction, and its power; 0 is no star |
//! | `--sodium <r:g:b>` | the lamps' linear color |
//! | `--lamp <x>` / `--scatter <x>` / `--bulb <x>` | the lamps' power, the haze they light, and their bulbs' |
//! | `--deep <share>` / `--riding <share>` | of 4 m cells with a lamp over the base, and riding layer 1; 0.35 and 0.3 |
//! | `--reach <m>` | how far a lamp's light goes, 2 by default. Twice it, and the tile less twice it, are whole 4 m cells |
//! | `--beam <deg>` | light only this slice of each lamp's circle, aimed at random: floodlights. 70 by default; 360 is a bare bulb |
//! | `--exposure <stops>` | open the exposure |
//! | `--seed <n>` | the tiles' layout |
//! | `--frames <n>` | frames after the box is up |
//! | `--dump <dir>` | also write each variant's layers there as PNGs: normal x, normal y, albedo, coverage |
//! | `--burst <n>` / `--sweep <deg>` | photograph `n` frames, numbered, swinging the yaw through `sweep` across them: parallax |

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::WindowResolution;
use lc_client::looks::sprue::{Sprue, SprueMaterial, TILE_TEXELS, VARIANTS, tiles};
use lc_client::looks::{LooksPlugin, direction};

struct Args {
    path: String,
    size: f32,
    yaw: f32,
    pitch: f32,
    dist: f32,
    target: Vec3,
    sodium: Vec3,
    fov: f32,
    sun: f32,
    sun_yaw: f32,
    star: f32,
    lamp: f32,
    scatter: f32,
    bulb: f32,
    deep: f32,
    riding: f32,
    reach: f32,
    beam: f32,
    exposure: f32,
    seed: u64,
    frames: u32,
    burst: u32,
    sweep: f32,
    dump: Option<String>,
}

impl Args {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let raw = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let value = |flag: &str| {
            raw(flag).map(|v| v.parse::<f32>().unwrap_or_else(|_| panic!("{flag} takes a number")))
        };
        let path = args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| {
            eprintln!("sprue_void: give a PNG path to write");
            std::process::exit(2);
        });
        let d = Sprue::default();
        let sodium = raw("--sodium").map_or(d.sodium, |t| {
            let v: Vec<f32> = t.split(':').map(|c| c.parse().expect("--sodium is r:g:b")).collect();
            Vec3::new(v[0], v[1], v[2])
        });
        let target = raw("--target").map_or(Vec3::ZERO, |t| {
            let v: Vec<f32> = t.split(':').map(|c| c.parse().expect("--target is x:y:z")).collect();
            Vec3::new(v[0], v[1], v[2])
        });
        Self {
            path,
            size: value("--size").unwrap_or(48.0),
            yaw: value("--yaw").unwrap_or(35.0),
            pitch: value("--pitch").unwrap_or(20.0),
            dist: value("--dist").unwrap_or(110.0),
            target,
            sodium,
            fov: value("--fov").unwrap_or(40.0),
            sun: value("--sun").unwrap_or(25.0),
            sun_yaw: value("--sun-yaw").unwrap_or(-60.0),
            star: value("--star").unwrap_or(d.star),
            lamp: value("--lamp").unwrap_or(d.lamp),
            scatter: value("--scatter").unwrap_or(d.scatter),
            bulb: value("--bulb").unwrap_or(d.bulb),
            deep: value("--deep").unwrap_or(d.deep),
            riding: value("--riding").unwrap_or(d.riding),
            reach: value("--reach").unwrap_or(d.reach_m),
            beam: value("--beam").unwrap_or(d.beam_deg),
            exposure: value("--exposure").unwrap_or(0.0),
            seed: raw("--seed").map_or(d.seed, |v| v.parse().expect("--seed takes a whole number")),
            frames: value("--frames").unwrap_or(60.0) as u32,
            burst: value("--burst").unwrap_or(1.0).max(1.0) as u32,
            sweep: value("--sweep").unwrap_or(0.0),
            dump: raw("--dump"),
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
                        title: "Lightcone Frontier: sprue in a void".into(),
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
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SprueMaterial>>,
) {
    let args = &setup.0;
    let sprue = Sprue {
        sodium: args.sodium,
        to_star: direction(args.sun, args.sun_yaw),
        star: args.star,
        star_spin: 0.0,
        lamp: args.lamp,
        scatter: args.scatter,
        bulb: args.bulb,
        deep: args.deep,
        riding: args.riding,
        reach_m: args.reach,
        beam_deg: args.beam,
        exposure_stops: args.exposure,
        seed: args.seed,
    };
    let started = std::time::Instant::now();
    let baked = sprue.bake();
    info!(
        "sprue_void: made {VARIANTS} variants, {} lamps, layer 1 carries {:.0}%, in {:.2?}",
        baked.lamps,
        100.0 * baked.carried,
        started.elapsed()
    );
    if let Some(dir) = &args.dump {
        std::fs::create_dir_all(dir).expect("--dump makes its directory");
        for (i, layer) in baked.shaped.iter().flatten().enumerate() {
            let path = format!("{dir}/v{:02}-layer{}.png", i / 3, i % 3);
            let image = image::RgbaImage::from_raw(TILE_TEXELS, TILE_TEXELS, layer.rgba8()).expect("a whole tile");
            image.save(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        }
        dump_light(dir, &baked.shaped, &baked.light);
    }
    for (k, material) in sprue.materials(&baked, &mut images).into_iter().enumerate() {
        let edge = args.size + 2.0 * Sprue::lift(k);
        commands.spawn((Mesh3d(meshes.add(Cuboid::new(edge, edge, edge))), MeshMaterial3d(materials.add(material))));
    }

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

/// Rec. 709 luma, for a light's brightness.
const LUMA: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

/// Each layer's light through the coverage above it, averaged over every variant: a tile from far off.
fn dump_light(dir: &str, shaped: &[Vec<tiles::Surface>], light: &[Vec<Vec3>]) {
    let n = shaped[0][0].texels;
    let (bands, side) = (16, 64);
    let mut profile = vec![0.0f64; bands];
    let mut map = vec![0.0f64; side * side];
    for (v, layers) in shaped.iter().enumerate() {
        for i in 0..n * n {
            let (x, y) = (i % n, i / n);
            let (mut seen, mut open) = (0.0, 1.0);
            for k in (0..3).rev() {
                let cover = layers[k].cover[i];
                seen += open * cover * light[v * 3 + k][i].dot(LUMA);
                open *= 1.0 - cover;
            }
            profile[x * bands / n] += seen as f64;
            map[(y * side / n) * side + x * side / n] += seen as f64;
        }
    }
    let peak = map.iter().copied().fold(0.0, f64::max);
    let bytes: Vec<u8> = map.iter().map(|m| (255.0 * (m / peak).sqrt()) as u8).collect();
    let path = format!("{dir}/light-mean.png");
    image::GrayImage::from_raw(side as u32, side as u32, bytes)
        .expect("a whole map")
        .save(&path)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let mean = profile.iter().sum::<f64>() / bands as f64;
    let shares: Vec<i32> = profile.iter().map(|p| (p / mean * 100.0).round() as i32).collect();
    info!("sprue_void: light across the tile, against its mean: {shares:?}");
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
