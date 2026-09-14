//! A population's envelope, drawn as a shell.
//!
//! A population is a distribution rather than a roster, so there is nothing to instance from
//! and the renderer must not invent members to instance. It draws the distribution instead:
//! one shell, one draw call, whatever the element count. See `lightcone/docs/07-rendering.md`.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};

/// Sky density at this vertex's latitude, with the population's peak at one.
pub const ATTRIBUTE_SHELL_DENSITY: MeshVertexAttribute =
    MeshVertexAttribute::new("ShellDensity", 0x5348_454C_4C00_0001, VertexFormat::Float32);

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct PopulationUniform {
    pub tint: Vec4,
    /// Covering fraction, scaled for display.
    pub opacity: f32,
    pub grain_frequency: f32,
    pub grain_strength: f32,
    pub seed: f32,
    /// How much the limb brightens. A line of sight along a thin shell passes through far more
    /// of it than one straight through, which is why a ring has a bright edge.
    pub limb_gain: f32,
    /// What the shell is scaled to when the camera is inside it. One outside.
    pub inside_fade: f32,
}

impl Default for PopulationUniform {
    fn default() -> Self {
        Self {
            tint: Vec4::new(0.75, 0.72, 0.66, 1.0),
            opacity: 1.0,
            grain_frequency: 90.0,
            grain_strength: 0.8,
            seed: 0.0,
            limb_gain: 0.15,
            inside_fade: 1.0,
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct PopulationMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: PopulationUniform,
}

impl Material for PopulationMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/population.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/population.wgsl".into()
    }

    /// Additive, and the fragment must return alpha zero: `AlphaMode::Add` is premultiplied
    /// blending, `src + dst * (1 - alpha)`, so an alpha of one overwrites instead of adding.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTRIBUTE_SHELL_DENSITY.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        // Both faces: the ship is usually inside the shell looking out through the far side.
        descriptor.primitive.cull_mode = None;
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }
        Ok(())
    }
}

pub struct PopulationMaterialPlugin;

impl Plugin for PopulationMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PopulationMaterial>::default());
    }
}
