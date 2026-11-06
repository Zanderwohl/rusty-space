//! Which part kinds wear which look on a real hull, as the hull material's regions take them.

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use em_render::hull_material::{HullMaterial, HullUniform};
use lc_world::form::Kind;

use super::aztec::{Aztec, Roof, Windows};
use super::obsidian::Obsidian;
use super::racks::{HullRacks, Racks};
use super::tubes::Tubes;
use super::sprue::{HEIGHTS, Sprue, TILE_M};
use crate::hull_mesh::{ENGINE_FLANK, region};

const THRUSTER_GRID_M: f32 = 4.0;
const THRUSTER_BAR_M: f32 = 0.5;
const THRUSTER_CELL_M: f32 = 2.5;
const THRUSTER_DEPTH_M: f32 = 2.0;
/// Ionized xenon's blue, linear: a cell's color at its walls.
const XENON: Vec3 = Vec3::new(0.08, 0.3, 1.0);

/// Bakes the engine flanks' sprue and the racks' faces once, handing them to every hull material.
pub struct HullLooksPlugin;

impl Plugin for HullLooksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pipes>()
            .init_resource::<RackFaces>()
            .add_systems(Startup, bake_pipes)
            .add_systems(PostUpdate, (fit_pipes, fit_rack_faces));
    }
}

/// The racks' faces: a fraction of a second, so baked where they are first wanted.
#[derive(Resource, Default)]
struct RackFaces(Option<Handle<Image>>);

/// Hands the racks' faces to each hull material that lacks them.
fn fit_rack_faces(mut faces: ResMut<RackFaces>, mut images: ResMut<Assets<Image>>, mut materials: ResMut<Assets<HullMaterial>>) {
    let lacking: Vec<AssetId<HullMaterial>> =
        materials.iter().filter(|(_, m)| m.rack_faces.is_none()).map(|(id, _)| id).collect();
    if lacking.is_empty() {
        return;
    }
    let baked = faces.0.get_or_insert_with(|| images.add(super::racks::faces::bake())).clone();
    for id in lacking {
        if let Some(mut material) = materials.get_mut(id) {
            material.rack_faces = Some(baked.clone());
        }
    }
}

/// The sprue's tile and light arrays: seconds of work, so baked on the async pool.
#[derive(Resource, Default)]
struct Pipes {
    baking: Option<Task<(Image, Image)>>,
    ready: Option<(Handle<Image>, Handle<Image>)>,
}

fn bake_pipes(mut pipes: ResMut<Pipes>) {
    pipes.baking = Some(AsyncComputeTaskPool::get().spawn(async { Sprue::arrays(&Sprue::default().bake()) }));
}

/// Hands the baked arrays to each hull material that lacks them: once a material, whoever made it.
fn fit_pipes(mut pipes: ResMut<Pipes>, mut images: ResMut<Assets<Image>>, mut materials: ResMut<Assets<HullMaterial>>) {
    if let Some(task) = pipes.baking.as_mut()
        && let Some((tiles, light)) = check_ready(task)
    {
        pipes.baking = None;
        pipes.ready = Some((images.add(tiles), images.add(light)));
    }
    let Some((tiles, light)) = &pipes.ready else { return };
    let lacking: Vec<AssetId<HullMaterial>> =
        materials.iter().filter(|(_, m)| m.pipe_tiles.is_none()).map(|(id, _)| id).collect();
    for id in lacking {
        if let Some(mut material) = materials.get_mut(id) {
            material.pipe_tiles = Some(tiles.clone());
            material.pipe_light = Some(light.clone());
        }
    }
}

/// `uniform` with the Mind knapped in obsidian, Storage in azteced plate, Living a habitat, Data
/// server racks, Drone a hive of launch tubes, an engine's flanks a layered sprue and its open
/// faces an ion thruster's grid over glowing cells.
pub fn wear(uniform: HullUniform) -> HullUniform {
    let HullRacks { rack, lean, hall, face, paint, glow } = Racks::default().hull_racks();
    let (tube_cell, tube_depth, tube_guide) = Tubes::default().hull_tubes();
    let (glass, glass_finish) = Obsidian::default().hull_glass();
    let (plates, plate_finish, plate_metal) = Aztec::default().hull_plates();
    let (windows, roof) = (Windows::default(), Roof::default());
    HullUniform {
        glassy: 1 << region(Kind::Mind),
        glass,
        glass_finish,
        plated: 1 << region(Kind::Storage),
        habitat: 1 << region(Kind::Living),
        plates,
        plate_finish,
        plate_metal,
        deck: Vec4::new(windows.deck_m, windows.pitch_m, windows.sill_m, windows.room_m),
        pane: Vec4::new(windows.width_m, windows.height_m, windows.f0, windows.roughness),
        roof: Vec4::new(roof.hex_m, roof.seam_m, roof.rim_m, roof.forest_m),
        roof_sky: roof.sky.extend(roof.scatter),
        roof_shape: Vec4::new(roof.depth_share, windows.lit, 0.0, 0.0),
        piped: 1 << ENGINE_FLANK,
        pipes: Vec4::new(HEIGHTS.x, HEIGHTS.y, TILE_M, 0.0),
        thruster: Vec4::new(THRUSTER_GRID_M, THRUSTER_BAR_M, THRUSTER_CELL_M, THRUSTER_DEPTH_M),
        thruster_glow: XENON.extend(0.0),
        looks: UVec4::new(1 << region(Kind::Data), 1 << region(Kind::Drone), 0, 0),
        rack,
        rack_lean: lean,
        rack_hall: hall,
        rack_face: face,
        rack_paint: paint,
        rack_glow: glow,
        tube_cell,
        tube_depth,
        tube_guide,
        ..uniform
    }
}
