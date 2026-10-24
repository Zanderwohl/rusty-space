//! Photograph a layered sprue on a box in a void: an opaque plated base under two cutout layers
//! of mazed pipe, each flat and normal-mapped, standing off one another and lit from the gaps by
//! sodium lamps. A look to try, not yet any part's.
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
//! | `--exposure <stops>` | open the exposure |
//! | `--seed <n>` | the tiles' layout |
//! | `--frames <n>` | frames after the box is up |
//! | `--burst <n>` / `--sweep <deg>` | photograph `n` frames, numbered, swinging the yaw through `sweep` across them: parallax |

mod bake;
mod tiles;

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::shader::ShaderRef;
use bevy::window::WindowResolution;
use bevy_mesh::MeshVertexBufferLayoutRef;

use bake::Lamps;
use tiles::{Maze, Rng};

/// What one tile spans. Two to the box's edge by default.
const TILE_M: f32 = 24.0;

/// Some 4.7 cm a texel, geometry and light alike: sixteen variants of three layers at 1024 would
/// be a quarter of a gigabyte.
const TILE_TEXELS: u32 = 512;

/// Two choices a tile edge: a variant is its left + 2 right + 4 bottom + 8 top choice, which
/// sprue.wgsl picks by hash, and any arrangement meets at every seam.
const VARIANTS: usize = 16;

/// Meters over the base: layer 1, layer 2, the deep lamps, and the lamps riding layer 1.
const HEIGHTS: Vec4 = Vec4::new(0.9, 1.9, 0.4, 1.25);

/// High-pressure sodium, linear: yellow washed toward white rather than LPS's pure 589 nm.
const SODIUM: Vec3 = Vec3::new(1.0, 0.72, 0.2);

const CONDUIT: Maze = Maze {
    cells: 8,
    radii: &[0.3, 0.4, 0.5],
    prune: 0.5,
    prune_passes: 2,
    doubled: 0.3,
    boxed: 0.5,
    collar_m: 1.6,
    crossing: 0.45,
};

const CAPILLARY: Maze = Maze {
    cells: 16,
    radii: &[0.1, 0.14, 0.18],
    prune: 0.6,
    prune_passes: 3,
    doubled: 0.25,
    boxed: 0.35,
    collar_m: 0.7,
    crossing: 0.4,
};

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
    exposure: f32,
    seed: u64,
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
            eprintln!("sprue_void: give a PNG path to write");
            std::process::exit(2);
        });
        let sodium = raw("--sodium").map_or(SODIUM, |t| {
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
            star: value("--star").unwrap_or(0.2),
            lamp: value("--lamp").unwrap_or(1.0),
            scatter: value("--scatter").unwrap_or(1.0),
            bulb: value("--bulb").unwrap_or(1.0),
            exposure: value("--exposure").unwrap_or(0.0),
            seed: value("--seed").unwrap_or(7.0) as u64,
            frames: value("--frames").unwrap_or(60.0) as u32,
            burst: value("--burst").unwrap_or(1.0).max(1.0) as u32,
            sweep: value("--sweep").unwrap_or(0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct SprueUniform {
    layer: Vec4,
    heights: Vec4,
    lamp_color: Vec4,
    to_star: Vec4,
    star_color: Vec4,
    ambient: Vec4,
    finish: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
struct SprueMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    uniforms: SprueUniform,
    #[texture(1, dimension = "2d_array", visibility(fragment))]
    #[sampler(2, visibility(fragment))]
    tiles: Handle<Image>,
    #[texture(3, dimension = "2d_array", visibility(fragment))]
    light: Handle<Image>,
}

impl Material for SprueMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/sprue.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/sprue.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        Ok(())
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
        .add_plugins(MaterialPlugin::<SprueMaterial>::default())
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
    let texels = TILE_TEXELS as usize;
    let mut rng = Rng::new(args.seed);
    let conduit = tiles::MazeEdges::new(&CONDUIT, &mut rng);
    let capillary = tiles::MazeEdges::new(&CAPILLARY, &mut rng);
    let lamps_spec = Lamps {
        cell_m: 4.0,
        deep_share: 0.35,
        riding_share: 0.3,
        heights: HEIGHTS,
        scatter: 0.004 * args.scatter,
        bulb_radius_m: 0.09,
        bulb_power: 18.0 * args.bulb,
    };
    let started = std::time::Instant::now();
    let variant = |v: usize| {
        let edges = [v & 1, v >> 1 & 1, v >> 2 & 1, v >> 3 & 1];
        let mut rng = Rng::new(args.seed.wrapping_mul(0x9E37_79B9).wrapping_add(v as u64));
        let layers = vec![
            tiles::base(texels, TILE_M, &mut rng).surface(true),
            tiles::maze(texels, TILE_M, &CONDUIT, &conduit, edges, &mut rng).surface(false),
            tiles::maze(texels, TILE_M, &CAPILLARY, &capillary, edges, &mut rng).surface(false),
        ];
        let lamps = bake::of_variant(&lamps_spec, &layers, args.seed, v);
        let light = bake::bake(&lamps_spec, &layers, &lamps);
        (layers, light, lamps.len())
    };
    let made: Vec<_> = std::thread::scope(|scope| {
        let making: Vec<_> = (0..VARIANTS).map(|v| scope.spawn(move || variant(v))).collect();
        making.into_iter().map(|m| m.join().expect("a variant panicked")).collect()
    });
    info!(
        "sprue_void: made {VARIANTS} variants, {} lamps, in {:.2?}",
        made.iter().map(|m| m.2).sum::<usize>(),
        started.elapsed()
    );
    let layers: Vec<_> = made.iter().flat_map(|m| &m.0).collect();
    let geometry: Vec<Vec<u8>> = layers.iter().map(|l| l.rgba8()).collect();
    let tiles = images.add(tiles::tile_array(&geometry, TILE_TEXELS));
    let light: Vec<Vec<Vec3>> = made.iter().flat_map(|m| m.1.iter().cloned()).collect();
    let light = images.add(bake::light_array(&light, &layers));

    let (sun, sun_yaw) = (args.sun.to_radians(), args.sun_yaw.to_radians());
    let to_star = Vec3::new(sun.cos() * sun_yaw.cos(), sun.sin(), sun.cos() * sun_yaw.sin());
    for (k, lift) in [0.0, HEIGHTS.x, HEIGHTS.y].into_iter().enumerate() {
        let material = materials.add(SprueMaterial {
            uniforms: SprueUniform {
                layer: Vec4::new(k as f32, TILE_M, 0.0, 0.0),
                heights: HEIGHTS,
                lamp_color: (args.sodium * 6.0 * args.lamp).extend(0.0),
                to_star: to_star.extend(0.0),
                star_color: Vec3::splat(args.star).extend(0.0),
                ambient: (args.sodium * 0.015).extend(0.0),
                finish: Vec4::new(2f32.powf(args.exposure), 24.0, 0.35, 0.0),
            },
            tiles: tiles.clone(),
            light: light.clone(),
        });
        let edge = args.size + 2.0 * lift;
        commands.spawn((Mesh3d(meshes.add(Cuboid::new(edge, edge, edge))), MeshMaterial3d(material)));
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
