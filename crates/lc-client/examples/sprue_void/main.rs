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

use tiles::{Maze, Rng};

/// What one tile spans. Two to the box's edge by default.
const TILE_M: f32 = 24.0;

/// Some 2.3 cm a texel.
const TILE_TEXELS: u32 = 1024;

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
    offset: 0.0,
};

const CAPILLARY: Maze = Maze {
    cells: 16,
    radii: &[0.1, 0.14, 0.18],
    prune: 0.6,
    prune_passes: 3,
    doubled: 0.25,
    boxed: 0.35,
    collar_m: 0.7,
    offset: 0.5,
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
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
struct SprueUniform {
    layer: Vec4,
    heights: Vec4,
    lamps: Vec4,
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

#[derive(Resource)]
struct Shot {
    path: String,
    frames: u32,
    drawn: u32,
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
        .insert_resource(Shot { path: args.path.clone(), frames: args.frames, drawn: 0 })
        .insert_resource(Setup(args))
        .add_systems(Startup, spawn)
        .add_systems(Update, photograph)
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
    let layers = [
        tiles::base(texels, TILE_M, &mut rng).texels(true),
        tiles::maze(texels, TILE_M, &CONDUIT, &mut rng).texels(false),
        tiles::maze(texels, TILE_M, &CAPILLARY, &mut rng).texels(false),
    ];
    let tiles = images.add(tiles::tile_array(&layers, TILE_TEXELS));

    let (sun, sun_yaw) = (args.sun.to_radians(), args.sun_yaw.to_radians());
    let to_star = Vec3::new(sun.cos() * sun_yaw.cos(), sun.sin(), sun.cos() * sun_yaw.sin());
    for (k, lift) in [0.0, HEIGHTS.x, HEIGHTS.y].into_iter().enumerate() {
        let material = materials.add(SprueMaterial {
            uniforms: SprueUniform {
                layer: Vec4::new(k as f32, TILE_M, 0.09, 18.0 * args.bulb),
                heights: HEIGHTS,
                lamps: Vec4::new(4.0, 0.35, 0.3, 0.004 * args.scatter),
                lamp_color: (args.sodium * 6.0 * args.lamp).extend(0.0),
                to_star: to_star.extend(0.0),
                star_color: Vec3::splat(args.star).extend(0.0),
                ambient: (args.sodium * 0.015).extend(0.0),
                finish: Vec4::new(2f32.powf(args.exposure), 24.0, 0.35, 0.0),
            },
            tiles: tiles.clone(),
        });
        let edge = args.size + 2.0 * lift;
        commands.spawn((Mesh3d(meshes.add(Cuboid::new(edge, edge, edge))), MeshMaterial3d(material)));
    }

    let (yaw, pitch) = (args.yaw.to_radians(), args.pitch.to_radians());
    let eye = args.target + args.dist * Vec3::new(pitch.cos() * yaw.sin(), pitch.sin(), pitch.cos() * yaw.cos());
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
        Transform::from_translation(eye).looking_at(args.target, Vec3::Y),
    ));
}

fn photograph(mut commands: Commands, mut shot: ResMut<Shot>, mut exit: MessageWriter<AppExit>) {
    shot.drawn += 1;
    if shot.drawn == shot.frames {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(shot.path.clone()));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if shot.drawn > shot.frames + 30 {
        exit.write(AppExit::Success);
    }
}
