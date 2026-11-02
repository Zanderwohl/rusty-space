//! Which part kinds wear which look on a real hull, as the hull material's regions take them.

use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use em_render::hull_material::{HullMaterial, HullUniform};
use lc_world::form::Kind;

use super::aztec::{Aztec, Roof, Windows};
use super::obsidian::Obsidian;
use super::outline::{LivingRoof, outline_hull_roofs};
use super::sprue::{HEIGHTS, Sprue, TILE_M};
use crate::hull_mesh::{ENGINE_FLANK, region};

/// Outlines a hull's habitat roof once its mesh lands, which a hull asks for with [`living_roof`],
/// and bakes the engine flanks' sprue once, handing it to every hull material.
pub struct HullLooksPlugin;

impl Plugin for HullLooksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pipes>()
            .add_systems(Startup, bake_pipes)
            .add_systems(PostUpdate, (outline_hull_roofs, fit_pipes));
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

/// What a hull entity carries so its Living roof is walled in under its rim.
pub fn living_roof() -> LivingRoof {
    LivingRoof(region(Kind::Living))
}

/// `uniform` with the Mind knapped in obsidian, Storage in azteced plate, Living a habitat, and an
/// engine's flanks a layered sprue.
pub fn wear(uniform: HullUniform) -> HullUniform {
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
        ..uniform
    }
}
