//! Things too small to resolve that are not stars: a quad each, shaded as a sum of blackbodies and
//! lines through the starfield's uniforms, so they are exposed, Doppler-shifted and aberrated as
//! a star is.
//!
//! A star is one blackbody of fixed size. One of these is up to [`TERMS`] terms, each a blackbody
//! at a temperature over a solid angle, a line at a wavelength carrying a flux, or a flux the same
//! in every band, plus an optional **event**: a flash of one term for a while, then a blackbody
//! fading as luminosity falling linearly to nothing, closed form in `globals.time` so nothing is
//! rewritten while it plays.
//!
//! The host supplies `shaders/craft_points.wgsl` and the starfield's band table.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};

use crate::relativistic_starfield_material::{ATTRIBUTE_STAR_CORNER, RelativisticStarfieldUniform};

/// Terms per point, in two attributes of two each.
pub const TERMS: usize = 4;

/// Terms 0 and 1, `(kelvin, sr, kelvin, sr)`. A blackbody's band flux is its band radiance times
/// `sr`; a negative kelvin is a line at that many meters, whose `sr` is its flux, W/m²; and one at
/// or below `-1e38` is the same flux, `sr`, in every band.
pub const ATTRIBUTE_TERMS_A: MeshVertexAttribute =
    MeshVertexAttribute::new("CraftTermsA", 0x4352_4146_0001, VertexFormat::Float32x4);

/// Terms 2 and 3.
pub const ATTRIBUTE_TERMS_B: MeshVertexAttribute =
    MeshVertexAttribute::new("CraftTermsB", 0x4352_4146_0002, VertexFormat::Float32x4);

/// The event's clock: `(start, flash, fade, wrap)`, real seconds as `globals.time` reads them,
/// `start` wrapped at `wrap`. A negative `start` is no event.
pub const ATTRIBUTE_EVENT_CLOCK: MeshVertexAttribute =
    MeshVertexAttribute::new("CraftEventClock", 0x4352_4146_0003, VertexFormat::Float32x4);

/// The event's light: `(flash kelvin, flash sr, fade's first kelvin, fade's sr)`, the flash a term
/// as above. The fade's area is fixed, so its temperature goes as `(1 - t / fade)^¼`.
pub const ATTRIBUTE_EVENT_LIGHT: MeshVertexAttribute =
    MeshVertexAttribute::new("CraftEventLight", 0x4352_4146_0004, VertexFormat::Float32x4);

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct CraftPointMaterial {
    /// The starfield's, so these points share its exposure, glare and pixel sizes.
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: RelativisticStarfieldUniform,
    /// The starfield's table.
    #[texture(1, sample_type = "float", filterable = false, visibility(vertex, fragment))]
    pub band_lut: Handle<Image>,
}

impl Material for CraftPointMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/craft_points.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/craft_points.wgsl".into()
    }

    /// Additive, and the fragment returns alpha zero: see the starfield's.
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
            ATTRIBUTE_STAR_CORNER.at_shader_location(1),
            ATTRIBUTE_TERMS_A.at_shader_location(2),
            ATTRIBUTE_TERMS_B.at_shader_location(3),
            ATTRIBUTE_EVENT_CLOCK.at_shader_location(4),
            ATTRIBUTE_EVENT_LIGHT.at_shader_location(5),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub struct CraftPointMaterialPlugin;

impl Plugin for CraftPointMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<CraftPointMaterial>::default());
    }
}
