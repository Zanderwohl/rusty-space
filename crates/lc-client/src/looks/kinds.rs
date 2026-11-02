//! Which part kinds wear which look on a real hull, as the hull material's regions take them.

use em_render::hull_material::HullUniform;
use lc_world::form::Kind;

use super::aztec::Aztec;
use super::obsidian::Obsidian;
use crate::hull_mesh::region;

/// `uniform` with the Mind knapped in obsidian and Storage in azteced plate.
pub fn wear(uniform: HullUniform) -> HullUniform {
    let (glass, glass_finish) = Obsidian::default().hull_glass();
    let (plates, plate_finish, plate_metal) = Aztec::default().hull_plates();
    HullUniform {
        glassy: 1 << region(Kind::Mind),
        glass,
        glass_finish,
        plated: 1 << region(Kind::Storage),
        plates,
        plate_finish,
        plate_metal,
        ..uniform
    }
}
