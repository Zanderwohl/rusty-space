//! A population's envelope, drawn as a volume.
//!
//! A population is a distribution rather than a roster, so there is nothing to instance from
//! and the renderer must not invent members to instance. It draws the distribution instead:
//! a convex proxy whose fragments march the population's own density field, one draw call
//! whatever the element count. See `lightcone/docs/07-rendering.md`.
//!
//! Rings share the material and take the surface path — a ring is a sheet, not a volume, and
//! `volumetric` is which.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};

use crate::relativistic_starfield_material::BANDS;

/// Optical depth at this vertex, with the deepest band at one. Rings only: a volume reads its
/// density from [`PopulationMaterial::profile`] instead.
pub const ATTRIBUTE_SHELL_DENSITY: MeshVertexAttribute =
    MeshVertexAttribute::new("ShellDensity", 0x5348_454C_4C00_0001, VertexFormat::Float32);

/// Samples across each row of [`PopulationMaterial::profile`].
pub const PROFILE_SAMPLES: usize = 128;

/// Row of the profile texture holding density against latitude, in units of the widest
/// inclination, so the whole row describes the part of the sky the population reaches.
pub const PROFILE_LATITUDE: usize = 0;

/// Row holding density against radius, from the inner edge to the outer one.
pub const PROFILE_RADIAL: usize = 1;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct PopulationUniform {
    pub tint: Vec4,
    /// The population's pole in mesh space, `w` unused.
    ///
    /// Carried rather than assumed: it is `render_space::sim_to_render` of the simulation pole,
    /// which keeps the Z-up to Y-up swizzle in the one module that owns it.
    pub pole: Vec4,
    /// Covering fraction, scaled for display. What the photometry says is there.
    pub opacity: f32,
    /// Grains per unit of the shell's outer radius. Low: a grain has to be something the
    /// sightline passes *through*, or it averages out over the march and the volume comes back
    /// smooth.
    pub grain_frequency: f32,
    /// How much of the density the granularity moves about. Mean-preserving in the shader.
    pub grain_strength: f32,
    /// Per-population, so two belts are not the same speckle.
    pub seed: f32,
    /// How much the limb brightens. Surface path only; a volume gets this from the geometry.
    pub limb_gain: f32,
    /// What the shell is scaled to when the camera is inside it. One outside.
    pub inside_fade: f32,
    /// Inner edge of the material, with the outer edge at one.
    pub inner: f32,
    /// Sine of the widest inclination: the slab the material lies inside. One for a cloud.
    ///
    /// The march clips to it, which is most of why a belt costs what a belt should: a sightline
    /// crossing a thin belt spends its steps in the belt rather than in the empty sphere
    /// around it.
    pub slab: f32,
    /// One for a population, zero for a ring.
    pub volumetric: f32,
    /// Band `b`'s contribution to display red, green and blue. `w` unused.
    ///
    /// The same columns the starfield binds, from the same `BandMapping`, so a population and
    /// the stars behind it are looking through one instrument rather than two.
    pub band_to_display: [Vec4; BANDS],
    /// Per band: `x` the material's source radiance, `y` its extinction coefficient. `zw` unused.
    ///
    /// Two numbers because a band changes *both* things about a population and they are not the
    /// same change. A dust cloud is thirteen decades more transparent at 21 cm than in B —
    /// `em_spectra::extinction::RATIO` — so at 21 cm the sightline finds almost nothing there;
    /// that is `y`, and it is what makes the dust-penetration preset do what it is named for.
    /// What the material that *is* there looks like is `x`, and for a belt it is scattered
    /// starlight in the optical and its own 200 K glow at ten microns.
    pub band_material: [Vec4; BANDS],
}

impl Default for PopulationUniform {
    fn default() -> Self {
        Self {
            tint: Vec4::new(0.75, 0.72, 0.66, 1.0),
            pole: Vec4::new(0.0, 1.0, 0.0, 0.0),
            opacity: 1.0,
            grain_frequency: 6.0,
            grain_strength: 0.85,
            seed: 0.0,
            limb_gain: 0.15,
            inside_fade: 1.0,
            inner: 0.0,
            slab: 1.0,
            volumetric: 1.0,
            band_to_display: [Vec4::ZERO; BANDS],
            band_material: [Vec4::ZERO; BANDS],
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct PopulationMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: PopulationUniform,
    /// Two rows of density: latitude, then radius. See [`PROFILE_LATITUDE`].
    ///
    /// Read with `textureLoad` and interpolated in the shader, so it is deliberately not
    /// filterable — a 32-bit float texture cannot be sampled with a filtering sampler under
    /// WebGPU, and the profile has decades in it where eight bits would band.
    #[texture(1, sample_type = "float", filterable = false, visibility(fragment))]
    pub profile: Handle<Image>,
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
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            ATTRIBUTE_SHELL_DENSITY.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        // Both faces: a ring is a sheet seen from either side, and the ship is usually inside
        // a population's proxy looking out through the far side of it. The volume path drops
        // its own front faces in the fragment stage, which it has to do in the shader rather
        // than here because one pipeline serves both.
        descriptor.primitive.cull_mode = None;
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = Some(false);
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
