//! Obsidian: knapped volcanic glass, blue-gray-black, cut by flake scars that meet in ridges and
//! peaks. Fully procedural, on world position in 3D, so it takes any shape without a seam; see
//! `assets/shaders/obsidian.wgsl`.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

pub struct Obsidian {
    /// The larger scars' spacing; the finer are a third of it.
    pub scar_m: f32,
    /// How far a scar's dish leans the surface toward its middle at its rim, as a tangent.
    pub dish: f32,
    /// How far each scar may tilt as a whole.
    pub tilt_deg: f32,
    /// The ripple rings' lean, as a tangent.
    pub ripple: f32,
    /// The glass's color, linear: almost black.
    pub color: Vec3,
    pub roughness: f32,
    pub to_star: Vec3,
    pub star: f32,
    /// The star's turn about world Y, rad/s; see [`super::spin`].
    pub star_spin: f32,
    pub to_planet: Vec3,
    /// The planet's angular radius.
    pub planet_deg: f32,
    pub planet: f32,
    /// What the glass mirrors of the sky, linear.
    pub sky: Vec3,
    pub exposure_stops: f32,
    /// Below 2^24: it reaches the shader as an f32.
    pub seed: u32,
}

impl Default for Obsidian {
    fn default() -> Self {
        Self {
            scar_m: 5.0,
            dish: 0.16,
            tilt_deg: 12.0,
            ripple: 0.015,
            color: Vec3::new(0.022, 0.026, 0.034),
            roughness: 0.08,
            to_star: super::direction(30.0, -50.0),
            star: 3.0,
            star_spin: 0.0,
            to_planet: super::direction(-15.0, 150.0),
            planet_deg: 20.0,
            planet: 0.12,
            sky: Vec3::new(0.012, 0.016, 0.024),
            exposure_stops: 0.0,
            seed: 7,
        }
    }
}

impl Obsidian {
    pub fn material(&self) -> ObsidianMaterial {
        ObsidianMaterial {
            uniforms: ObsidianUniform {
                scar: Vec4::new(self.scar_m, self.dish, self.tilt_deg.to_radians().tan(), self.ripple),
                glass: self.color.extend(self.roughness),
                to_star: self.to_star.extend(self.star_spin),
                star_color: Vec3::splat(self.star).extend(0.0),
                to_planet: self.to_planet.extend(self.planet_deg.to_radians()),
                planet_color: (Vec3::new(0.55, 0.7, 1.0) * self.planet).extend(0.0),
                sky: self.sky.extend(0.0),
                finish: Vec4::new(2f32.powf(self.exposure_stops), self.seed as f32, 0.0, 0.0),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct ObsidianUniform {
    scar: Vec4,
    glass: Vec4,
    to_star: Vec4,
    star_color: Vec4,
    to_planet: Vec4,
    planet_color: Vec4,
    sky: Vec4,
    finish: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct ObsidianMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    uniforms: ObsidianUniform,
}

impl Material for ObsidianMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/obsidian.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/obsidian.wgsl".into()
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
