//! Azteced sheet metal: rectangular panels, each its own shade and tipped a few degrees, with
//! chamfered trenches between. Fully procedural; see `assets/shaders/aztec.wgsl`.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

pub struct Aztec {
    /// The staggered grid before splitting, the smallest panel side, and the grid panel edges
    /// snap to, meters.
    pub cell_m: f32,
    pub min_panel_m: f32,
    pub snap_m: f32,
    /// Each wall of a seam's trench: its width across the face, and how far its normal leans
    /// toward the seam.
    pub chamfer_m: f32,
    pub lean_deg: f32,
    /// How far a panel's normal may tip off the face's.
    pub tilt_deg: f32,
    /// Of panels a little browner.
    pub brown: f32,
    pub roughness: f32,
    pub metallic: f32,
    pub to_star: Vec3,
    pub star: f32,
    pub star_spin: f32,
    pub to_planet: Vec3,
    pub planet_deg: f32,
    pub planet: f32,
    pub exposure_stops: f32,
    pub seed: u32,
}

impl Default for Aztec {
    fn default() -> Self {
        Self {
            cell_m: 24.0,
            min_panel_m: 3.0,
            snap_m: 0.25,
            chamfer_m: 0.15,
            lean_deg: 30.0,
            tilt_deg: 5.0,
            brown: 0.2,
            roughness: 0.35,
            metallic: 0.6,
            to_star: super::direction(30.0, -50.0),
            star: 3.0,
            star_spin: 0.0,
            to_planet: super::direction(-15.0, 150.0),
            planet_deg: 20.0,
            planet: 0.12,
            exposure_stops: 0.0,
            seed: 7,
        }
    }
}

impl Aztec {
    pub fn material(&self) -> AztecMaterial {
        AztecMaterial {
            uniforms: AztecUniform {
                grid: Vec4::new(self.cell_m, self.min_panel_m, self.snap_m, 0.0),
                chamfer: Vec4::new(self.chamfer_m, self.lean_deg.to_radians(), 0.0, 0.0),
                panel: Vec4::new(self.tilt_deg.to_radians(), self.brown, self.roughness, self.metallic),
                to_star: self.to_star.extend(self.star_spin),
                star_color: Vec3::splat(self.star).extend(0.0),
                to_planet: self.to_planet.extend(self.planet_deg.to_radians()),
                planet_color: (Vec3::new(0.55, 0.7, 1.0) * self.planet).extend(0.0),
                ambient: Vec3::splat(0.004).extend(0.0),
                finish: Vec4::new(2f32.powf(self.exposure_stops), self.seed as f32, 0.0, 0.0),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct AztecUniform {
    grid: Vec4,
    chamfer: Vec4,
    panel: Vec4,
    to_star: Vec4,
    star_color: Vec4,
    to_planet: Vec4,
    planet_color: Vec4,
    ambient: Vec4,
    finish: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct AztecMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    uniforms: AztecUniform,
}

impl Material for AztecMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/aztec.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/aztec.wgsl".into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        super::position_and_normal(descriptor, layout)
    }
}
