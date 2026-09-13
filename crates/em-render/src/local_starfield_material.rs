//! Custom material for rendering local simulation stars onto the skybox.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};

/// Per-star linear RGBA color (alpha unused for now).
pub const ATTRIBUTE_LOCAL_STAR_COLOR: MeshVertexAttribute =
    MeshVertexAttribute::new("LocalStarColor", 0x4C53_434F_4C4F_5201, VertexFormat::Float32x4);

/// Quad corner offset in `[-1, 1]^2`.
pub const ATTRIBUTE_LOCAL_STAR_CORNER: MeshVertexAttribute =
    MeshVertexAttribute::new("LocalStarCorner", 0x4C53_434F_524E_4552, VertexFormat::Float32x2);

/// Per-star params: (intensity, radius, unused).
pub const ATTRIBUTE_LOCAL_STAR_PARAMS: MeshVertexAttribute =
    MeshVertexAttribute::new("LocalStarParams", 0x4C53_5052_4D53_0001, VertexFormat::Float32x3);

#[derive(Clone, Debug, ShaderType)]
pub struct LocalStarfieldMaterialUniform {
    pub brightness_min: f32,
    pub brightness_max: f32,
    pub min_angular_radius_rad: f32,
    pub max_intensity: f32,
    pub near_distance_bevy: f32,
    pub far_distance_bevy: f32,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct LocalStarfieldMaterial {
    #[uniform(0)]
    pub uniforms: LocalStarfieldMaterialUniform,
}

impl Material for LocalStarfieldMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/local_starfield.wgsl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/local_starfield.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTRIBUTE_LOCAL_STAR_COLOR.at_shader_location(1),
            ATTRIBUTE_LOCAL_STAR_CORNER.at_shader_location(2),
            ATTRIBUTE_LOCAL_STAR_PARAMS.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];

        descriptor.primitive.cull_mode = None;

        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }

        Ok(())
    }
}

pub struct LocalStarfieldMaterialPlugin;

impl Plugin for LocalStarfieldMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<LocalStarfieldMaterial>::default());
    }
}
