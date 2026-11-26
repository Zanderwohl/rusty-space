//! A planet's rings, drawn as a flat annulus whose bands the fragment shader evaluates.
//!
//! The bands are uniform data rather than vertex data, so an edge falls where the data puts it at
//! any zoom and a band narrower than a pixel is averaged over the pixel instead of being missed.
//! The host supplies `shaders/ring.wgsl`.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

/// Bands a ring can carry. Must match `RING_BANDS` in `ring.wgsl`.
pub const RING_BANDS: usize = 16;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct RingUniform {
    pub tint: Vec4,
    /// Per band, inside out: `(inner, outer, depth, icy)`. Radii are in units of the outer edge;
    /// depth is optical depth over the deepest band's. Bands past `params.x` are ignored.
    pub bands: [Vec4; RING_BANDS],
    /// `(band count, opacity, limb gain, 0)`.
    pub params: Vec4,
    /// `(grains per outer radius, strength, seed, 0)`. The grain varies with radius only.
    pub grain: Vec4,
}

impl Default for RingUniform {
    fn default() -> Self {
        Self {
            tint: Vec4::ONE,
            bands: [Vec4::ZERO; RING_BANDS],
            params: Vec4::new(0.0, 1.0, 0.15, 0.0),
            grain: Vec4::new(40.0, 0.35, 0.0, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct RingMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: RingUniform,
    /// The populations' grain volume, read along one line. It must tile.
    #[texture(1, dimension = "3d", visibility(fragment))]
    #[sampler(2, visibility(fragment))]
    pub grain: Handle<Image>,
}

impl Material for RingMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/ring.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/ring.wgsl".into()
    }

    /// Additive, with the fragment returning alpha zero: premultiplied `src + dst * (1 - alpha)`.
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
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        // A sheet is seen from both sides.
        descriptor.primitive.cull_mode = None;
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub struct RingMaterialPlugin;

impl Plugin for RingMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<RingMaterial>::default());
    }
}
