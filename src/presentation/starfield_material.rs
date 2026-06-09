//! Custom material for rendering a starfield background.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::render::storage::ShaderStorageBuffer;
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

/// Material for rendering distant stars as a skybox-like background.
///
/// Stars are stored in a GPU buffer as `vec4(dir.x, dir.y, dir.z, mag)` where
/// `dir` is a pre-computed unit direction vector in Bevy Y-up space.
/// Colors are stored separately as `vec4(r, g, b, 1.0)` in linear RGB.
#[derive(Clone, Debug, ShaderType)]
pub struct StarfieldMaterialUniform {
    pub star_count: u32,
    pub brightness: f32,
    pub _padding: Vec2,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct StarfieldMaterial {
    /// Storage buffer containing star data as `[f32; 4]` per star
    #[storage(0, read_only)]
    pub stars: Handle<ShaderStorageBuffer>,

    /// Storage buffer containing per-star linear RGB colors as `[f32; 4]`
    #[storage(1, read_only)]
    pub colors: Handle<ShaderStorageBuffer>,

    /// Uniform parameters for fragment shading.
    #[uniform(2)]
    pub uniforms: StarfieldMaterialUniform,
}

impl Material for StarfieldMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/starfield.wgsl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/starfield.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Render back faces so we see the inside of the sphere
        descriptor.primitive.cull_mode = None;

        // Don't write to depth buffer so scene geometry renders on top
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }

        Ok(())
    }
}

/// Plugin that registers the StarfieldMaterial with Bevy's rendering system.
pub struct StarfieldMaterialPlugin;

impl Plugin for StarfieldMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<StarfieldMaterial>::default());
    }
}
