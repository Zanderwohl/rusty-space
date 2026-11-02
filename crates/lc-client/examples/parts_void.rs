//! Photograph every part kind in every primitive, through the mesher and the hull material, and
//! beside them each of [`lc_client::looks`] on the same shapes: a grid, a column a kind or look
//! and a row a primitive. Each is one bare piece, not a form, so nothing has to be a valid ship.
//!
//! ```text
//! cargo run -p lc-client --example parts_void -- /tmp/parts.png
//! ```
//!
//! | flag | for |
//! |---|---|
//! | `--volume <m3>` | every piece's volume, so they are alike in size; 60 000 by default |
//! | `--cells <n>` | the mesher's grid across each piece; 128 by default |
//! | `--yaw <deg>` / `--pitch <deg>` / `--zoom <x>` | the camera about the grid, and its distance as a multiple of what frames it |
//! | `--only <column>` | one column: a kind as `hull_mesh::REGION_GRAPHS` names it, or `aztec`, `offices`, `habitat`, `obsidian` or `sprue` |
//! | `--sun <deg>` | the star's elevation |
//! | `--frames <n>` | frames after every mesh lands |

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
use em_render::population_material::PopulationMaterial;
use em_render::relativistic_starfield_material::RelativisticStarfieldMaterial;
use glam::{DMat3, DVec3};
use lc_client::looks::kinds::{HullLooksPlugin, living_roof, wear};
use lc_client::hull_mesh::{Finish, HullForm, HullMeshPlugin, HullMeshState, HullMeshSystems, HullSource, REGION_GRAPHS, region};
use lc_client::looks::aztec::{Aztec, AztecMaterial, Windows};
use lc_client::looks::sprue::{Sprue, SprueMaterial};
use lc_client::looks::LooksPlugin;
use lc_client::looks::obsidian::{Obsidian, ObsidianMaterial};
use lc_client::looks::outline::RoofOutline;
use lc_client::procedural::{Bakes, ProceduralTexturesPlugin, Shape as Target2d, Target, placeholder};
use lc_client::tonemap::ToneMap;
use lc_world::form::place::{Pose, Side};
use lc_world::form::primitive::Shape;
use lc_world::form::sdf::Piece;
use lc_world::form::{Kind, PartId, Primitive, SparMode};

/// As `hull_void`: the graphs' unit square, and its texels.
const TILE_M: f32 = 64.0;
const TILE_TEXELS: u32 = 512;
const LIT_WINDOW: f32 = 0.0075;
const LIT_HULL: f32 = 0.3;

const FOV_Y: f32 = 0.5;

/// In the order of `REGION_GRAPHS`, less the engine's flank, which no part is.
const KINDS: [Kind; 8] = [
    Kind::Storage,
    Kind::Drone,
    Kind::Living,
    Kind::Engine,
    Kind::Data,
    Kind::Mind,
    Kind::Spar(SparMode::Saddle),
    Kind::Bay,
];

const LOOKS: [&str; 5] = ["aztec", "offices", "habitat", "obsidian", "sprue"];

/// Each primitive in a proportion that shows it, its axis standing up.
const PRIMITIVES: [Primitive; 6] = [
    Primitive::Ellipsoid { axes: DVec3::new(1.4, 1.0, 0.8) },
    Primitive::Capsule { length: 2.0 },
    Primitive::Slab { edges: DVec3::new(1.4, 1.0, 0.7), corner: 0.12 },
    Primitive::Cylinder { length: 2.5 },
    Primitive::Torus { major: 3.0 },
    Primitive::Frustum { length: 2.0, taper: 0.5 },
];

#[derive(Clone, Copy)]
enum Column {
    Kind(Kind),
    Look(&'static str),
}

impl Column {
    fn name(&self) -> &'static str {
        match self {
            Column::Kind(kind) => REGION_GRAPHS[region(*kind) as usize],
            Column::Look(look) => look,
        }
    }
}

struct Args {
    path: String,
    columns: Vec<Column>,
    volume: f64,
    cells: u32,
    yaw_deg: f32,
    pitch_deg: f32,
    zoom: f32,
    sun_deg: f32,
    frames: u32,
}

impl Args {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let text = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
        let value = |flag: &str| text(flag).map(|v| v.parse::<f32>().unwrap_or_else(|_| panic!("{flag} takes a number")));
        let fail = |message: &str| -> ! {
            eprintln!("parts_void: {message}");
            std::process::exit(2);
        };
        let path = args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(|| fail("give a PNG path to write"));
        let all: Vec<Column> = KINDS.map(Column::Kind).into_iter().chain(LOOKS.map(Column::Look)).collect();
        let columns = match text("--only") {
            None => all,
            Some(name) => vec![*all.iter().find(|c| c.name() == name).unwrap_or_else(|| fail(&format!("no column {name}")))],
        };
        Self {
            path,
            columns,
            volume: value("--volume").unwrap_or(6.0e4) as f64,
            cells: value("--cells").unwrap_or(128.0) as u32,
            yaw_deg: value("--yaw").unwrap_or(-25.0),
            pitch_deg: value("--pitch").unwrap_or(20.0),
            zoom: value("--zoom").unwrap_or(1.0),
            sun_deg: value("--sun").unwrap_or(35.0),
            frames: value("--frames").unwrap_or(10.0) as u32,
        }
    }
}

#[derive(Resource)]
struct Scene {
    args: Args,
    albedo: Vec<Handle<Image>>,
    lights: Vec<Handle<Image>>,
    albedo_texels: Vec<Option<Vec<u8>>>,
    light_texels: Vec<Option<Vec<u8>>>,
    spawned: bool,
    /// Frames since every piece first showed its mesh.
    drawn: Option<u32>,
}

fn main() {
    let args = Args::parse();
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin { file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into(), ..default() })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Lightcone Frontier: parts in a void".into(),
                        resolution: WindowResolution::new(1280, 720).with_scale_factor_override(1.0),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .init_asset::<PopulationMaterial>()
        .init_asset::<BodySurfaceMaterial>()
        .init_asset::<RelativisticStarfieldMaterial>()
        .add_plugins((ProceduralTexturesPlugin, HullMaterialPlugin, HullMeshPlugin, LooksPlugin, HullLooksPlugin))
        .insert_resource(ClearColor(Color::BLACK))
        .insert_resource(Scene {
            args,
            albedo: Vec::new(),
            lights: Vec::new(),
            albedo_texels: vec![None; REGION_GRAPHS.len()],
            light_texels: vec![None; REGION_GRAPHS.len()],
            spawned: false,
            drawn: None,
        })
        .add_systems(Startup, request_tiles)
        .add_systems(Update, spawn.before(HullMeshSystems))
        .add_systems(Update, photograph.after(HullMeshSystems))
        .add_systems(Last, take_texels)
        .run();
}

fn request_tiles(mut scene: ResMut<Scene>, assets: Res<AssetServer>, mut images: ResMut<Assets<Image>>, mut bakes: ResMut<Bakes>) {
    let plane = Target::new(Target2d::Plane(TILE_TEXELS));
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

/// The part's axis, its x, along the ship's z, which is up once drawn: towers and a lying ring.
const STANDING: DMat3 = DMat3::from_cols(DVec3::Z, DVec3::X, DVec3::Y);

/// The same shape grown by `h` meters all round: exact for the capsule, slab and torus, and near
/// enough for the rest to stand a sprue's layer off its base.
fn grown(shape: Shape, h: f64) -> Shape {
    match shape {
        Shape::Ellipsoid { semi_axes } => Shape::Ellipsoid { semi_axes: semi_axes + h },
        Shape::Capsule { radius, length } => Shape::Capsule { radius: radius + h, length },
        Shape::Slab { edges, corner } => Shape::Slab { edges: edges + 2.0 * h, corner: corner + h },
        Shape::Cylinder { radius, length } => Shape::Cylinder { radius: radius + h, length: length + 2.0 * h },
        Shape::Torus { major, minor } => Shape::Torus { major, minor: minor + h },
        Shape::Frustum { length, start, end } => Shape::Frustum { length: length + 2.0 * h, start: start + h, end: end + h },
    }
}

/// Farthest any part of `shape` reaches from its center, meters.
fn reach(shape: Shape) -> f64 {
    match shape {
        Shape::Ellipsoid { semi_axes } => semi_axes.max_element(),
        Shape::Capsule { radius, length } => radius + 0.5 * length,
        Shape::Slab { edges, .. } => 0.5 * edges.length(),
        Shape::Cylinder { radius, length } => radius.hypot(0.5 * length),
        Shape::Torus { major, minor } => major + minor,
        Shape::Frustum { length, start, end } => start.max(end).hypot(0.5 * length),
    }
}

fn piece(kind: Kind, shape: Shape) -> Piece {
    Piece { part: PartId(0), side: Side::Original, kind, shape, pose: Pose { position: DVec3::ZERO, rotation: STANDING } }
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
    mut hulls: ResMut<Assets<HullMaterial>>,
    mut aztecs: ResMut<Assets<AztecMaterial>>,
    mut obsidians: ResMut<Assets<ObsidianMaterial>>,
    mut sprues: ResMut<Assets<SprueMaterial>>,
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
            error!("parts_void: a tile settled without landing; see the warnings above");
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
    let hull = hulls.add(HullMaterial {
        uniforms: wear(HullUniform {
            to_star: to_star.extend(0.08),
            exposure: Vec4::new(LIT_HULL * 2f32.powf(1.5), ToneMap::default().surface_stops, 0.0, 0.0),
            detail: Vec4::new(TILE_M, 0.0, 0.0, 0.0),
            bolted: 1 << region(Kind::Spar(SparMode::Saddle)),
            emitted,
            ..default()
        }),
        albedo: images.add(tile_array(&albedo, TILE_TEXELS, Tile::Albedo)),
        lights: images.add(tile_array(&lights, TILE_TEXELS, Tile::Lights)),
        roof_map: None,
        roof_outline: default(),
    });
    let aztec = aztecs.add(Aztec { to_star, ..default() }.material());
    let offices = aztecs.add(Aztec { to_star, windows: Some(Windows::default()), ..default() }.material());
    let habitat = aztecs.add(Aztec { to_star, ..Aztec::habitat() }.material());
    let obsidian = obsidians.add(Obsidian { to_star, ..default() }.material());
    let sprue_layers = if args.columns.iter().any(|c| matches!(c, Column::Look("sprue"))) {
        let sprue = Sprue { to_star, ..default() };
        Some(sprue.materials(&sprue.bake(), &mut images).map(|m| sprues.add(m)))
    } else {
        None
    };

    let shapes: Vec<Shape> = PRIMITIVES.iter().map(|p| p.at(p.scale(args.volume))).collect();
    let slot = 2.4 * shapes.iter().copied().map(reach).fold(0.0, f64::max) as f32;
    // On the plane square to the view, so every slot is seen alike.
    let right = Vec3::Y.cross(toward).normalize();
    let up = toward.cross(right);
    let (columns, rows) = (args.columns.len() as f32, shapes.len() as f32);
    for (c, column) in args.columns.iter().enumerate() {
        for (r, &shape) in shapes.iter().enumerate() {
            let at = right * (c as f32 - (columns - 1.0) / 2.0) * slot + up * ((rows - 1.0) / 2.0 - r as f32) * slot;
            let transform = Transform::from_translation(at).with_rotation(ship_to_world());
            let form = |kind: Kind, shape: Shape| HullForm {
                source: HullSource::Pieces(Arc::from([piece(kind, shape)])),
                finish: Finish::Smooth,
                cells: Some(args.cells),
            };
            match *column {
                Column::Kind(kind) => {
                    commands.spawn((form(kind, shape), MeshMaterial3d(hull.clone()), transform, living_roof()));
                }
                Column::Look("sprue") => {
                    let layers = sprue_layers.as_ref().expect("made when a column is the sprue");
                    for (k, layer) in layers.iter().enumerate() {
                        let lifted = grown(shape, Sprue::lift(k) as f64);
                        commands.spawn((form(Kind::Storage, lifted), MeshMaterial3d(layer.clone()), transform));
                    }
                }
                Column::Look("obsidian") => {
                    commands.spawn((form(Kind::Storage, shape), MeshMaterial3d(obsidian.clone()), transform));
                }
                Column::Look(look) => {
                    let material = match look {
                        "offices" => offices.clone(),
                        "habitat" => habitat.clone(),
                        _ => aztec.clone(),
                    };
                    let piece = commands.spawn((form(Kind::Storage, shape), MeshMaterial3d(material), transform)).id();
                    if look == "habitat" {
                        commands.entity(piece).insert(RoofOutline);
                    }
                }
            }
        }
    }

    let aspect = 16.0 / 9.0;
    let half = (0.5 * rows * slot).max(0.5 * columns * slot / aspect);
    let distance = args.zoom * half / (0.5 * FOV_Y).tan();
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: FOV_Y, near: distance * 0.01, far: distance * 4.0, ..default() }),
        Hdr,
        Bloom::NATURAL,
        Tonemapping::TonyMcMapface,
        Transform::from_translation(toward * distance).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn photograph(
    mut commands: Commands,
    mut scene: ResMut<Scene>,
    pieces: Query<&HullMeshState>,
    mut exit: MessageWriter<AppExit>,
) {
    let all_current = !pieces.is_empty() && pieces.iter().all(|s| s.current());
    let Some(frames) = scene.drawn.map(|n| n + 1).or(all_current.then_some(0)) else { return };
    scene.drawn = Some(frames);
    if frames == scene.args.frames {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(scene.args.path.clone()));
    }
    // The capture is asynchronous; quitting on the same frame loses the file.
    if frames > scene.args.frames + 30 {
        exit.write(AppExit::Success);
    }
}
