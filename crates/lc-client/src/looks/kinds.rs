//! Which part kinds wear which look on a real hull, as the hull material's regions take them.

use bevy::prelude::*;
use em_render::hull_material::HullUniform;
use lc_world::form::Kind;

use super::aztec::{Aztec, Roof, Windows};
use super::obsidian::Obsidian;
use super::outline::{LivingRoof, outline_hull_roofs};
use crate::hull_mesh::region;

/// Outlines a hull's habitat roof once its mesh lands; a hull asks with [`living_roof`].
pub struct HullLooksPlugin;

impl Plugin for HullLooksPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, outline_hull_roofs);
    }
}

/// What a hull entity carries so its Living roof is walled in under its rim.
pub fn living_roof() -> LivingRoof {
    LivingRoof(region(Kind::Living))
}

/// `uniform` with the Mind knapped in obsidian, Storage in azteced plate, and Living a habitat.
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
        ..uniform
    }
}
