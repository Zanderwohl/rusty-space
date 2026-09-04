//! Custom material for rendering a starfield background.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};

/// Per-star linear RGB color with baked brightness in the alpha channel.
///
/// This is a *custom* attribute on purpose: using the built-in
/// `Mesh::ATTRIBUTE_COLOR` makes Bevy enable the `VERTEX_COLORS` shader def,
/// which injects a `@location(7)` color varying into the auto-generated
/// (pre)pass pipelines and fails validation against our custom vertex layout.
pub const ATTRIBUTE_STAR_COLOR: MeshVertexAttribute =
    MeshVertexAttribute::new("StarColor", 0x5354_4152_0003, VertexFormat::Float32x4);

/// Quad corner offset in `[-1, 1]^2`. Doubles as the radial falloff coordinate
/// in the fragment shader.
pub const ATTRIBUTE_STAR_CORNER: MeshVertexAttribute =
    MeshVertexAttribute::new("StarCorner", 0x5354_4152_0001, VertexFormat::Float32x2);

/// Per-star size factor in `0..1` derived from apparent magnitude (bright stars
/// map to 1.0 -> max radius). Expanded against the radius uniforms in the shader
/// so the size sliders stay live without rebuilding the mesh.
pub const ATTRIBUTE_STAR_SIZE_T: MeshVertexAttribute =
    MeshVertexAttribute::new("StarSizeT", 0x5354_4152_0002, VertexFormat::Float32);

/// Uniform parameters for the starfield shader.
///
/// `star_count` is retained for 16-byte alignment / diagnostics; the shader no
/// longer iterates stars (each is its own quad).
#[derive(Clone, Debug, ShaderType)]
pub struct StarfieldMaterialUniform {
    pub star_count: u32,
    pub brightness: f32,
    /// Angular radius (arcminutes) of the faintest stars.
    pub star_radius_min: f32,
    /// Angular radius (arcminutes) of the brightest stars.
    pub star_radius_max: f32,
}

/// Material for rendering distant stars as camera-facing billboard quads.
///
/// All per-star data (direction, color, baked brightness, size factor) lives in
/// the mesh vertex attributes, so the material only carries the shared uniform.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct StarfieldMaterial {
    /// Uniform parameters for shading.
    #[uniform(0)]
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
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Vertex layout matching the billboard shader's `Vertex` struct.
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTRIBUTE_STAR_COLOR.at_shader_location(1),
            ATTRIBUTE_STAR_CORNER.at_shader_location(2),
            ATTRIBUTE_STAR_SIZE_T.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];

        // Quads are camera-facing; don't cull either winding.
        descriptor.primitive.cull_mode = None;

        // Don't write depth so scene geometry renders on top.
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
