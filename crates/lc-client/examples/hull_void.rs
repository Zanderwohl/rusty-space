//! Photograph the hull material on a sphere in a void, through the real render pipeline.
//!
//! Nothing in the game draws a hull this way yet. The sphere is eight regions, one a kind,
//! seeded around the living one under the camera, with fillets between them, which is what a
//! form's mesher will hand the material. See `lightcone/docs/32-ship-rendering.md`.
//!
//! ```text
//! cargo run -p lc-client --example hull_void -- /tmp/hull.png --radius 50000 --standoff 300
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--radius <m>` | the sphere; 500 by default |
//! | `--standoff <m>` | the camera's height over the surface; the whole disc by default |
//! | `--tilt <deg>` | turn the view from straight down toward the horizon |
//! | `--sun <deg>` | the star's elevation over the point beneath the camera; negative is night |
//! | `--exposure <stops>` | open the exposure from one placed for a lit hull |
//! | `--frames <n>` / `--burst <n>` | as the client's: frames after the tiles land, and consecutive shots |

use std::f32::consts::TAU;

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::WindowResolution;
use em_render::body_surface_material::BodySurfaceMaterial;
use em_render::hull_material::{
    ATTRIBUTE_HULL_REGION, HullMaterial, HullMaterialPlugin, HullUniform, REGIONS, Tile, region,
    tile_array,
};
use em_render::plume_material::PlumeMaterial;
use em_render::population_material::PopulationMaterial;
use em_render::relativistic_starfield_material::RelativisticStarfieldMaterial;
use lc_client::procedural::{Bakes, ProceduralTexturesPlugin, Shape, Target, placeholder};
use lc_client::tonemap::ToneMap;

const KINDS: [&str; 8] = ["storage", "drone", "living", "engine", "data", "mind", "spar", "bay"];
const LIVING: usize = 2;

/// What every graph's unit square spans. The graphs are written to it.
const TILE_M: f32 = 64.0;

/// Eight centimeters a texel, so a 1.4 m window is some eleven texels tall.
const TILE_TEXELS: u32 = 512;

/// A lit window against a white surface in full sun at 1 AU, by luminance: some 300 cd/m²
/// against 40 000. Everything else here is in units of that white.
const LIT_WINDOW: f32 = 0.0075;

/// The albedo the exposure is placed for, lit face-on.
const LIT_HULL: f32 = 0.3;

/// How far off the living region's center the ring of other regions sits.
const RING_DEG: f32 = 35.0;

/// A fillet's width, as a share of the radius: a form's fillets scale with its parts.
const FILLET: f32 = 0.05;

struct Args {
    path: String,
    radius: f32,
    standoff: f32,
    tilt_deg: f32,
    sun_deg: f32,
    exposure: f32,
    frames: u32,
    burst: u32,
}

impl Args {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let value = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .map(|v| v.parse::<f32>().unwrap_or_else(|_| panic!("{flag} takes a number")))
        };
        let path = args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| {
            eprintln!("hull_void: give a PNG path to write");
            std::process::exit(2);
        });
        let radius = value("--radius").unwrap_or(500.0);
        Self {
            path,
            radius,
            standoff: value("--standoff").unwrap_or(2.5 * radius),
            tilt_deg: value("--tilt").unwrap_or(0.0),
            sun_deg: value("--sun").unwrap_or(30.0),
            exposure: value("--exposure").unwrap_or(0.0),
            frames: value("--frames").unwrap_or(30.0) as u32,
            burst: value("--burst").unwrap_or(1.0).max(1.0) as u32,
        }
    }
}

#[derive(Resource)]
struct Scene {
    args: Args,
    albedo: Vec<Handle<Image>>,
    lights: Vec<Handle<Image>>,
    /// Each bake's texels, taken as it lands: the image leaves the main world once extracted.
    albedo_texels: Vec<Option<Vec<u8>>>,
    light_texels: Vec<Option<Vec<u8>>>,
    /// Frames since the hull was spawned.
    drawn: Option<u32>,
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
                        title: "Lightcone Frontier: hull in a void".into(),
                        resolution: WindowResolution::new(1280, 720).with_scale_factor_override(1.0),
                        ..default()
                    }),
                    ..default()
                }),
        )
        // The bake marks these changed when it lands, so their stores must exist.
        .init_asset::<PopulationMaterial>()
        .init_asset::<BodySurfaceMaterial>()
        .init_asset::<RelativisticStarfieldMaterial>()
        .init_asset::<PlumeMaterial>()
        .add_plugins((ProceduralTexturesPlugin, HullMaterialPlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Scene {
            args,
            albedo: Vec::new(),
            lights: Vec::new(),
            albedo_texels: vec![None; KINDS.len()],
            light_texels: vec![None; KINDS.len()],
            drawn: None,
        })
        .add_systems(Startup, request_tiles)
        .add_systems(Update, (spawn_hull, photograph).chain())
        .add_systems(Last, take_texels)
        .run();
}

fn request_tiles(
    mut scene: ResMut<Scene>,
    assets: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
    mut bakes: ResMut<Bakes>,
) {
    let plane = Target::new(Shape::Plane(TILE_TEXELS));
    for kind in KINDS {
        let graph = assets.load(format!("textures/hull/{kind}.tgraph"));
        for (target, into) in [(plane.color(), &mut scene.albedo), (plane.layer("lights"), &mut scene.lights)] {
            let image = images.add(placeholder(target));
            bakes.request(graph.clone(), 1, target, image.clone());
            into.push(image);
        }
    }
}

/// In `Last`, which is after the bake lands in `Update` and before extraction takes it away.
fn take_texels(mut scene: ResMut<Scene>, images: Res<Assets<Image>>) {
    let scene = &mut *scene;
    for (handles, texels) in [
        (&scene.albedo, &mut scene.albedo_texels),
        (&scene.lights, &mut scene.light_texels),
    ] {
        for (handle, slot) in handles.iter().zip(texels.iter_mut()) {
            let Some(image) = images.get(handle) else { continue };
            if slot.is_none() && image.width() == TILE_TEXELS {
                *slot = image.data.clone();
            }
        }
    }
}

fn spawn_hull(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    bakes: Res<Bakes>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HullMaterial>>,
    mut exit: MessageWriter<AppExit>,
) {
    if scene.drawn.is_some() {
        return;
    }
    if !scene.albedo.iter().chain(&scene.lights).all(|h| bakes.settled(h)) {
        return;
    }
    let (Some(albedo), Some(lights)) = (
        scene.albedo_texels.iter().cloned().collect::<Option<Vec<_>>>(),
        scene.light_texels.iter().cloned().collect::<Option<Vec<_>>>(),
    ) else {
        // A bake that failed has settled without landing.
        let missing: Vec<_> = KINDS
            .iter()
            .zip(scene.albedo_texels.iter().zip(&scene.light_texels))
            .filter(|(_, (a, l))| a.is_none() || l.is_none())
            .map(|(k, _)| *k)
            .collect();
        error!("hull_void: no tile for {missing:?}; see the warnings above");
        exit.write(AppExit::error());
        return;
    };

    let args = &scene.args;
    let (living, e1, e2) = frame();
    let to_star = (args.sun_deg.to_radians().sin() * living
        - args.sun_deg.to_radians().cos() * e1)
        .normalize();

    let mut emitted = [Vec4::ZERO; REGIONS];
    let power = |k: &str| match k {
        "drone" => LIT_WINDOW * Vec3::new(0.85, 0.92, 1.0),
        "living" => LIT_WINDOW * Vec3::new(1.0, 0.8, 0.55),
        // A stand-in for the exhaust's power, which nothing here has.
        "engine" => 8.0 * LIT_WINDOW * Vec3::new(0.75, 0.85, 1.0),
        "mind" => 0.3 * LIT_WINDOW * Vec3::new(0.6, 0.9, 1.0),
        "bay" => LIT_WINDOW * Vec3::new(1.0, 0.92, 0.8),
        _ => Vec3::ZERO,
    };
    for (slot, kind) in emitted.iter_mut().zip(KINDS) {
        *slot = power(kind).extend(0.0);
    }
    let tone = ToneMap::default();
    let material = materials.add(HullMaterial {
        uniforms: HullUniform {
            to_star: to_star.extend(0.0),
            reflected: Vec4::ONE,
            exposure: Vec4::new(LIT_HULL * 2f32.powf(-args.exposure), tone.surface_stops, 0.0, 0.0),
            detail: Vec4::new(TILE_M, 0.0, 0.0, 0.0),
            emitted,
            ..default()
        },
        albedo: images.add(tile_array(&albedo, TILE_TEXELS, Tile::Albedo)),
        lights: images.add(tile_array(&lights, TILE_TEXELS, Tile::Lights)),
    });
    commands.spawn((Mesh3d(meshes.add(sphere(args.radius))), MeshMaterial3d(material)));

    let eye = living * (args.radius + args.standoff);
    let tilt = args.tilt_deg.to_radians();
    let look = (-living * tilt.cos() + e1 * tilt.sin()).normalize();
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            near: 1.0,
            far: 4.0 * (args.radius + args.standoff),
            ..default()
        }),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
        Transform::from_translation(eye).looking_to(look, e2),
    ));
    info!(
        "hull_void: {} m sphere from {} m, star at {}°",
        args.radius, args.standoff, args.sun_deg
    );
    scene.drawn = Some(0);
}

/// The living region's center, and two directions square to it.
fn frame() -> (Vec3, Vec3, Vec3) {
    let living = Vec3::new(0.3, 0.5, 0.8).normalize();
    let e1 = living.cross(Vec3::Y).normalize();
    let e2 = e1.cross(living);
    (living, e1, e2)
}

/// Each kind's seed: living at the center, the rest in a ring about it.
fn seeds() -> [Vec3; KINDS.len()] {
    let (living, e1, e2) = frame();
    let ring = RING_DEG.to_radians();
    let mut out = [living; KINDS.len()];
    let others: Vec<usize> = (0..KINDS.len()).filter(|&k| k != LIVING).collect();
    for (i, &k) in others.iter().enumerate() {
        let around = TAU * i as f32 / others.len() as f32;
        let side = e1 * around.cos() + e2 * around.sin();
        out[k] = living * ring.cos() + side * ring.sin();
    }
    out
}

/// A sphere of `radius` meters in regions: each vertex takes its nearest seed, blending into
/// the second nearest over [`FILLET`], as a mesher would from per-part distances.
fn sphere(radius: f32) -> Mesh {
    let mut mesh = Sphere::new(radius).mesh().ico(7).expect("seven subdivisions is allowed");
    let seeds = seeds();
    let fillet = FILLET * radius;
    let positions = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|a| a.as_float3())
        .expect("a sphere has positions")
        .to_vec();
    let regions: Vec<[f32; 3]> = positions
        .iter()
        .map(|p| {
            let dir = Vec3::from(*p).normalize();
            let mut by_distance: Vec<(f32, u32)> = seeds
                .iter()
                .enumerate()
                .map(|(k, s)| (radius * dir.dot(*s).clamp(-1.0, 1.0).acos(), k as u32))
                .collect();
            by_distance.sort_by(|a, b| a.0.total_cmp(&b.0));
            let ((d1, nearest), (d2, second)) = (by_distance[0], by_distance[1]);
            let x = ((d2 - d1) / fillet).clamp(0.0, 1.0);
            let share = 0.5 * (1.0 - x * x * (3.0 - 2.0 * x));
            region(nearest, second, share)
        })
        .collect();
    mesh.insert_attribute(ATTRIBUTE_HULL_REGION, regions);
    mesh
}

/// As the client's `--shot` and `--burst`.
fn photograph(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(frames) = scene.drawn.as_mut() else { return };
    *frames += 1;
    let (after, burst, path) = (scene.args.frames, scene.args.burst, scene.args.path.clone());
    let frames = *frames;
    if (after..after + burst).contains(&frames) {
        let index = frames - after;
        let at = match (burst > 1, path.rsplit_once('.')) {
            (false, _) => path,
            (true, Some((stem, extension))) => format!("{stem}.{index}.{extension}"),
            (true, None) => format!("{path}.{index}"),
        };
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(at));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if frames > after + burst + 30 {
        exit.write(AppExit::Success);
    }
}
