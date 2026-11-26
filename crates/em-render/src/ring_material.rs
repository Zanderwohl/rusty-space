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
    /// Per band, inside out: `(inner, outer, optical depth, icy)`. Radii are in units of the outer
    /// edge; `icy` is the share of the band that is ice rather than rock. Bands past `params.x`
    /// are ignored.
    pub bands: [Vec4; RING_BANDS],
    /// `(band count, display floor, planet radius, 0)`. The floor is the least a band is drawn at
    /// for its coverage, before its lighting; see `ring.wgsl`. The planet's radius is in units of
    /// the outer edge, for its shadow.
    pub params: Vec4,
    /// Starlight a fully filled sheet of rock or ice sends back face-on, as linear display light
    /// through the current band mapping.
    pub reflected_rocky: Vec4,
    pub reflected_icy: Vec4,
    /// What the same sheet radiates at its own temperature, in the same units.
    pub emitted_rocky: Vec4,
    pub emitted_icy: Vec4,
    /// World direction from the planet to the star.
    pub to_star: Vec4,
    /// `(surface_reference, stops, 0, 0)`: the tone map, evaluated per fragment.
    pub exposure: Vec4,
    /// `(grains per outer radius, strength, seed, 0)`. The grain varies with radius only.
    pub grain: Vec4,
}

impl Default for RingUniform {
    fn default() -> Self {
        Self {
            bands: [Vec4::ZERO; RING_BANDS],
            params: Vec4::ZERO,
            reflected_rocky: Vec4::ZERO,
            reflected_icy: Vec4::ZERO,
            emitted_rocky: Vec4::ZERO,
            emitted_icy: Vec4::ZERO,
            to_star: Vec4::ZERO,
            exposure: Vec4::ZERO,
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

    /// Premultiplied, `src + dst * (1 - alpha)`, with alpha the share of the pixel the particles
    /// cover: a ring in front of its planet hides it as well as adding its own light.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    /// Sorted just nearer than anything else centered on the planet, so it draws after the
    /// planet's atmosphere shell, which has the same center and would otherwise tie. In render
    /// units; far smaller than the gap between any two bodies.
    fn depth_bias(&self) -> f32 {
        1.0e-4
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
