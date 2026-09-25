//! Photograph meshed forms in a void, through the real render pipeline and R3's hull material.
//!
//! Nothing in the game draws a form yet. This meshes the fixture forms with
//! `lc_client::hull_mesh` on the async compute pool, at a resolution set by how many pixels each
//! spans unless `--cells` fixes it. See `lightcone/docs/32-ship-rendering.md`.
//!
//! ```text
//! cargo run -p lc-client --example mesh_void -- /tmp/forms.png
//! cargo run -p lc-client --example mesh_void -- /tmp/strap.png --form strap --cells 32 --finish blocky
//! cargo run -p lc-client --example mesh_void -- /tmp/remesh.png --form cluster --remesh 256 --burst 40
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--form <name>` | `starting`, `plate`, `spindle`, `cluster`, `saddle`, `strap`, or `all` side by side (the default) |
//! | `--finish <smooth\|faceted\|blocky>` | the finish; smooth by default |
//! | `--cells <n>` | a fixed resolution instead of one from pixels on screen |
//! | `--scale <x>` | every length times this, so `--scale 100` makes a 50 km starting ship |
//! | `--yaw <deg>` / `--pitch <deg>` | where the camera stands, about the forms |
//! | `--zoom <x>` | the camera's distance, as a multiple of what frames everything |
//! | `--aim <part>` | look at this part's center instead; with one form only. With `--zoom 0.3` it frames a spar's seams closely enough to see the bolts |
//! | `--exposure <stops>` | open the exposure, in stops from `hull_void`'s, which is placed for a lit albedo of 0.3 and so clips anything lighter; `-1.5` by default |
//! | `--sun <deg>` | the star's elevation; negative is night, where the living lights show |
//! | `--frames <n>` / `--burst <n>` | frames after every mesh lands, and consecutive shots from there |
//! | `--remesh <cells>` | on the first shot, ask every hull for this resolution. Each frame's wall time is logged until every new mesh has landed and been drawn, and a burst photographs the old meshes staying up until the new ones swap in |

use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::WindowResolution;
use em_render::body_surface_material::BodySurfaceMaterial;
use em_render::hull_material::{HullMaterial, HullMaterialPlugin, HullUniform, REGIONS, Tile, tile_array};
use em_render::plume_material::PlumeMaterial;
use em_render::population_material::PopulationMaterial;
use em_render::relativistic_starfield_material::RelativisticStarfieldMaterial;
use lc_client::hull_mesh::{
    Finish, HullForm, HullMeshPlugin, HullMeshState, HullMeshSystems, REGION_GRAPHS, region, spar_fixture,
};
use lc_client::procedural::{Bakes, ProceduralTexturesPlugin, Shape, Target, placeholder};
use lc_client::tonemap::ToneMap;
use lc_world::fitting::Balance;
use lc_world::form::presets::Builtin;
use lc_world::form::sdf::Sdf;
use lc_world::form::{Form, Kind, SparMode};

const FORMS: [&str; 6] = ["starting", "plate", "spindle", "cluster", "saddle", "strap"];

/// As `hull_void`: the graphs' unit square, and its texels.
const TILE_M: f32 = 64.0;
const TILE_TEXELS: u32 = 512;
const LIT_WINDOW: f32 = 0.0075;
const LIT_HULL: f32 = 0.3;

const FOV_Y: f32 = 0.5;

struct Args {
    path: String,
    forms: Vec<&'static str>,
    finish: Finish,
    cells: Option<u32>,
    scale: f64,
    yaw_deg: f32,
    pitch_deg: f32,
    zoom: f32,
    sun_deg: f32,
    exposure: f32,
    frames: u32,
    burst: u32,
    remesh: Option<u32>,
    aim: Option<u16>,
}

impl Args {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let text = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let value = |flag: &str| text(flag).map(|v| v.parse::<f32>().unwrap_or_else(|_| panic!("{flag} takes a number")));
        let fail = |message: &str| -> ! {
            eprintln!("mesh_void: {message}");
            std::process::exit(2);
        };
        let path = args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| fail("give a PNG path to write"));
        let forms = match text("--form").as_deref() {
            None | Some("all") => FORMS.to_vec(),
            Some(name) => vec![*FORMS.iter().find(|f| **f == name).unwrap_or_else(|| fail(&format!("--form is one of {FORMS:?} or all")))],
        };
        let finish = match text("--finish").as_deref() {
            None | Some("smooth") => Finish::Smooth,
            Some("faceted") => Finish::Faceted,
            Some("blocky") => Finish::Blocky,
            Some(other) => fail(&format!("no finish {other}")),
        };
        Self {
            path,
            forms,
            finish,
            cells: value("--cells").map(|c| c as u32),
            scale: value("--scale").unwrap_or(1.0) as f64,
            yaw_deg: value("--yaw").unwrap_or(-35.0),
            pitch_deg: value("--pitch").unwrap_or(25.0),
            zoom: value("--zoom").unwrap_or(1.0),
            sun_deg: value("--sun").unwrap_or(35.0),
            exposure: value("--exposure").unwrap_or(-1.5),
            frames: value("--frames").unwrap_or(10.0) as u32,
            burst: value("--burst").unwrap_or(1.0).max(1.0) as u32,
            remesh: value("--remesh").map(|c| c as u32),
            aim: value("--aim").map(|id| id as u16),
        }
    }
}

fn fixture(name: &str) -> Form {
    match name {
        "starting" => Form::starting(),
        "plate" => Builtin::Plate.form(),
        "spindle" => Builtin::Spindle.form(),
        "cluster" => Builtin::Cluster.form(),
        "saddle" => spar_fixture(SparMode::Saddle),
        "strap" => spar_fixture(SparMode::Strap),
        _ => unreachable!("checked when parsed"),
    }
}

/// Every length times `k`. The Mind's size is `min_part_m3`'s, so that scales too.
fn scaled(form: Form, balance: &mut Balance, k: f64) -> Form {
    let k3 = k * k * k;
    balance.min_part_m3 *= k3;
    Form {
        parts: form
            .parts
            .into_iter()
            .map(|mut p| {
                p.volume_m3 *= k3;
                p
            })
            .collect(),
    }
}

#[derive(Component)]
struct Hull(&'static str);

#[derive(Resource)]
struct Scene {
    args: Args,
    albedo: Vec<Handle<Image>>,
    lights: Vec<Handle<Image>>,
    albedo_texels: Vec<Option<Vec<u8>>>,
    light_texels: Vec<Option<Vec<u8>>>,
    spawned: bool,
    /// Frames since every hull first showed its mesh.
    drawn: Option<u32>,
    /// Frames since the remesh was asked for, and whether it has landed everywhere.
    remeshing: Option<(u32, bool)>,
    /// Frames the log has run past the landing.
    after: u32,
}

fn main() {
    let args = Args::parse();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(bevy::log::LogPlugin {
                    filter: format!("{},lc_client::hull_mesh=debug", bevy::log::DEFAULT_FILTER),
                    ..default()
                })
                .set(AssetPlugin { file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into(), ..default() })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Lightcone Frontier: forms in a void".into(),
                        resolution: WindowResolution::new(1280, 720).with_scale_factor_override(1.0),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .init_asset::<PopulationMaterial>()
        .init_asset::<BodySurfaceMaterial>()
        .init_asset::<RelativisticStarfieldMaterial>()
        .init_asset::<PlumeMaterial>()
        .add_plugins((ProceduralTexturesPlugin, HullMaterialPlugin, HullMeshPlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Scene {
            args,
            albedo: Vec::new(),
            lights: Vec::new(),
            albedo_texels: vec![None; REGION_GRAPHS.len()],
            light_texels: vec![None; REGION_GRAPHS.len()],
            spawned: false,
            drawn: None,
            remeshing: None,
            after: 0,
        })
        .add_systems(Startup, request_tiles)
        .add_systems(Update, spawn.before(HullMeshSystems))
        .add_systems(Update, photograph.after(HullMeshSystems))
        .add_systems(Last, take_texels)
        .run();
}

fn request_tiles(mut scene: ResMut<Scene>, assets: Res<AssetServer>, mut images: ResMut<Assets<Image>>, mut bakes: ResMut<Bakes>) {
    let plane = Target::new(Shape::Plane(TILE_TEXELS));
    let scene = &mut *scene;
    for kind in REGION_GRAPHS {
        let graph = assets.load(format!("textures/hull/{kind}.tgraph"));
        for (target, into) in [(plane.color(), &mut scene.albedo), (plane.layer("lights"), &mut scene.lights)] {
            let image = images.add(placeholder(target));
            bakes.request(graph.clone(), 1, target, image.clone());
            into.push(image);
        }
    }
}

/// In `Last`, after the bake lands and before extraction takes it away.
fn take_texels(mut scene: ResMut<Scene>, images: Res<Assets<Image>>) {
    let scene = &mut *scene;
    for (handles, texels) in [(&scene.albedo, &mut scene.albedo_texels), (&scene.lights, &mut scene.light_texels)] {
        for (handle, slot) in handles.iter().zip(texels.iter_mut()) {
            let Some(image) = images.get(handle) else { continue };
            if slot.is_none() && image.width() == TILE_TEXELS {
                *slot = image.data.clone();
            }
        }
    }
}

/// From the ship's frame, z up, to Bevy's, y up.
fn ship_to_world() -> Quat {
    Quat::from_rotation_x(-FRAC_PI_2)
}

fn camera_direction(args: &Args) -> Vec3 {
    let (yaw, pitch) = (args.yaw_deg.to_radians(), args.pitch_deg.to_radians());
    Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos())
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    bakes: Res<Bakes>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<HullMaterial>>,
    mut exit: MessageWriter<AppExit>,
    mut waited: Local<u32>,
) {
    if scene.spawned || !scene.albedo.iter().chain(&scene.lights).all(|h| bakes.settled(h)) {
        return;
    }
    let (Some(albedo), Some(lights)) = (
        scene.albedo_texels.iter().cloned().collect::<Option<Vec<_>>>(),
        scene.light_texels.iter().cloned().collect::<Option<Vec<_>>>(),
    ) else {
        *waited += 1;
        if *waited >= 3 {
            error!("mesh_void: a tile settled without landing; see the warnings above");
            exit.write(AppExit::error());
        }
        return;
    };
    scene.spawned = true;
    let args = &scene.args;

    let toward = camera_direction(args);
    let sun = args.sun_deg.to_radians();
    let side = Quat::from_rotation_y(0.6) * Vec3::new(toward.x, 0.0, toward.z).normalize();
    let to_star = (side * sun.cos() + Vec3::Y * sun.sin()).normalize();
    let mut emitted = [Vec4::ZERO; REGIONS];
    for (slot, kind) in emitted.iter_mut().zip(REGION_GRAPHS) {
        *slot = match kind {
            "drone" => LIT_WINDOW * Vec3::new(0.85, 0.92, 1.0),
            "living" => LIT_WINDOW * Vec3::new(1.0, 0.8, 0.55),
            "engine" => 8.0 * LIT_WINDOW * Vec3::new(0.75, 0.85, 1.0),
            "mind" => 0.3 * LIT_WINDOW * Vec3::new(0.6, 0.9, 1.0),
            "bay" => LIT_WINDOW * Vec3::new(1.0, 0.92, 0.8),
            _ => Vec3::ZERO,
        }
        .extend(0.0);
    }
    let material = materials.add(HullMaterial {
        uniforms: HullUniform {
            // Enough fill to see an unlit side against the black.
            to_star: to_star.extend(0.08),
            exposure: Vec4::new(LIT_HULL * 2f32.powf(-args.exposure), ToneMap::default().surface_stops, 0.0, 0.0),
            detail: Vec4::new(TILE_M, 0.0, 0.0, 0.0),
            bolted: 1 << region(Kind::Spar(SparMode::Saddle)),
            emitted,
            ..default()
        },
        albedo: images.add(tile_array(&albedo, TILE_TEXELS, Tile::Albedo)),
        lights: images.add(tile_array(&lights, TILE_TEXELS, Tile::Lights)),
    });

    // Side by side in slots as wide as the largest, three to a row.
    let placed: Vec<(&'static str, Form, Balance, DVec3Pair)> = args
        .forms
        .iter()
        .map(|&name| {
            let mut balance = Balance::DEFAULT;
            let form = scaled(fixture(name), &mut balance, args.scale);
            let bounds = Sdf::new(&form, &balance).expect("fixtures are valid").bounds();
            (name, form, balance, bounds)
        })
        .collect();
    let aim = match (args.aim, placed.as_slice()) {
        (None, _) => None,
        (Some(id), [(_, form, balance, (lo, hi))]) => {
            let sdf = Sdf::new(form, balance).expect("fixtures are valid");
            let piece = sdf.pieces().iter().find(|p| p.part.0 == id).unwrap_or_else(|| panic!("--aim: no part {id}"));
            Some(ship_to_world() * (piece.pose.position - (*lo + *hi) / 2.0).as_vec3())
        }
        _ => panic!("--aim wants one --form"),
    };
    let slot = placed.iter().map(|(.., (lo, hi))| (*hi - *lo).length() as f32).fold(0.0, f32::max);
    let columns = placed.len().min(3);
    let rows = placed.len().div_ceil(3);
    // On the plane square to the view, so every slot is seen alike.
    let right = Vec3::Y.cross(toward).normalize();
    let up = toward.cross(right);
    for (k, (name, form, balance, (lo, hi))) in placed.into_iter().enumerate() {
        let (column, row) = ((k % 3) as f32, (k / 3) as f32);
        let at = right * (column - (columns as f32 - 1.0) / 2.0) * slot + up * ((rows as f32 - 1.0) / 2.0 - row) * slot;
        let center = ((lo + hi) / 2.0).as_vec3();
        commands.spawn((
            Hull(name),
            HullForm { form: Arc::new(form), balance, finish: args.finish, cells: args.cells },
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at - ship_to_world() * center).with_rotation(ship_to_world()),
        ));
    }

    // A slot is its form's bounding diagonal, so a form fits in one from any side.
    let aspect = 16.0 / 9.0;
    let half = (0.5 * rows as f32 * slot).max(0.5 * columns as f32 * slot / aspect);
    let distance = args.zoom * half / (0.5 * FOV_Y).tan();
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: FOV_Y,
            near: distance * 0.01,
            far: distance * 4.0,
            ..default()
        }),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
        Transform::from_translation(aim.unwrap_or_default() + toward * distance).looking_at(aim.unwrap_or_default(), Vec3::Y),
    ));
}

type DVec3Pair = (glam::DVec3, glam::DVec3);

fn photograph(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    time: Res<Time<Real>>,
    mut hulls: Query<(&Hull, &mut HullForm, &HullMeshState)>,
    mut exit: MessageWriter<AppExit>,
) {
    let all_current = !hulls.is_empty() && hulls.iter().all(|(.., s)| s.current());
    let Some(frames) = scene.drawn.map(|n| n + 1).or(all_current.then_some(0)) else { return };
    if frames == 0 {
        for (hull, _, state) in &hulls {
            info!("mesh_void: {} at {} cells, {:.0} m across", hull.0, state.cells().unwrap_or(0), state.extent_m());
        }
    }
    scene.drawn = Some(frames);
    let (after, burst) = (scene.args.frames, scene.args.burst);

    if frames == after
        && let Some(cells) = scene.args.remesh
    {
        for (_, mut form, _) in &mut hulls {
            form.cells = Some(cells);
        }
        scene.remeshing = Some((0, false));
    }
    if let Some((n, landed)) = scene.remeshing {
        let current = hulls.iter().filter(|(.., s)| s.current()).count();
        info!(
            "mesh_void: remesh frame {n}: {:.1} ms, {current}/{} hulls on the new mesh",
            time.delta_secs() * 1000.0,
            hulls.iter().count()
        );
        scene.remeshing = Some((n + 1, landed || (n > 0 && current == hulls.iter().count())));
    }

    if (after..after + burst).contains(&frames) {
        let index = frames - after;
        let path = scene.args.path.clone();
        let at = match (burst > 1, path.rsplit_once('.')) {
            (false, _) => path,
            (true, Some((stem, extension))) => format!("{stem}.{index}.{extension}"),
            (true, None) => format!("{path}.{index}"),
        };
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(at));
    }
    let settled = match scene.remeshing {
        Some((_, true)) => {
            scene.after += 1;
            scene.after > 5
        }
        Some((_, false)) => false,
        None => true,
    };
    // The capture is asynchronous; quitting on the same frame loses the file.
    if settled && frames > after + burst + 30 {
        exit.write(AppExit::Success);
    }
}
