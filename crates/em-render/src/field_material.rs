//! A radiating field around a craft, drawn on any closed envelope mesh.
//!
//! One mesh, three draws, told apart by [`FieldLayer`]: the far wall, a faint inner rim, and the
//! near wall. The walls are the radiator and the mode's surface; the near wall is the only one
//! that can hide what is behind it, so it is drawn last and the other two are pure addition.
//!
//! Every input is a number the host works out: temperature in kelvin, fill as a fraction of the
//! field's limit, mode, and so on. What those numbers mean for a game is the host's business.
//! So is the band mapping: the host evaluates a blackbody at [`ramp_kelvin`] through whatever
//! bands its observer has and hands the answers over in [`FieldUniform::spectrum`], and the
//! shader interpolates any temperature it needs from them.
//!
//! The mesh wants a uniform scale in its transform and its origin at the envelope's center:
//! the collapse inflates it about that point into a sphere. The host supplies
//! `shaders/field.wgsl`. Design: `lightcone/docs/32-ship-rendering.md` §The field.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::{ShaderDefVal, ShaderRef};
use bevy_mesh::MeshVertexBufferLayoutRef;

/// How many beams can put a hot spot on one field at once.
pub const HOT_SPOTS: usize = 4;

/// Temperatures in [`FieldUniform::spectrum`], log-spaced from [`RAMP_MIN_K`] to [`RAMP_MAX_K`].
pub const RAMP: usize = 32;

/// Below any field that is not dead.
pub const RAMP_MIN_K: f32 = 100.0;

/// A collapse's spike is far hotter than this; in any optical or infrared band it is on the
/// Rayleigh-Jeans side already, so its color has stopped changing.
pub const RAMP_MAX_K: f32 = 1.0e6;

/// Mode values in [`FieldUniform::mode`].
pub const CLEAR: f32 = 0.0;
pub const BLACK: f32 = 1.0;

/// The temperature at ramp entry `i`, kelvin.
pub fn ramp_kelvin(i: usize) -> f32 {
    let t = i as f32 / (RAMP - 1) as f32;
    (RAMP_MIN_K.ln() + t * (RAMP_MAX_K.ln() - RAMP_MIN_K.ln())).exp()
}

/// One ramp entry from linear display light: `log2` per channel.
///
/// Logarithms because a visible band at 400 K is some 10⁻³⁵ of its value at 4 600 K, which is
/// below `f32`'s range as a plain number.
pub fn ramp_entry(linear: [f64; 3]) -> Vec4 {
    let log = |x: f64| x.max(1e-300).log2() as f32;
    Vec4::new(log(linear[0]), log(linear[1]), log(linear[2]), 0.0)
}

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct FieldUniform {
    /// `(kelvin, fill, clear_absorptivity, clock_s)`.
    ///
    /// `fill` is heat as a fraction of the limit, `[0, 1]`; hot spots spread and the surface
    /// flickers as it rises. Black absorbs everything and Clear this fraction; by Kirchhoff that
    /// is also each mode's emissivity. `clock_s` is real seconds, for the shimmer and flicker.
    pub state: Vec4,
    /// `(mode, previous mode, switch progress, reach)`.
    ///
    /// Modes are [`CLEAR`] or [`BLACK`]. The new mode's surface covers everything within
    /// `progress * reach` of [`Self::origin`]; `reach` is in mesh-local units and should be the
    /// farthest the envelope gets from the origin. Progress 1 is a settled field.
    pub mode: Vec4,
    /// Where a switch sweeps out from, mesh-local. `w` unused.
    pub origin: Vec4,
    /// World direction to the star. `w` unused.
    pub to_star: Vec4,
    /// Linear display light a white Lambertian surface facing the star would send. `w` unused.
    pub starlight: Vec4,
    /// `(surface_reference, stops, overflow, 0)`: the tone map, as the lit surfaces evaluate it.
    ///
    /// `overflow` is what a stop past the top of the window is worth as HDR value, as for a
    /// plume, so a field brighter than the window blooms rather than clipping flat.
    pub exposure: Vec4,
    /// Toward each beam, world, with `w` its strength: the extra power it lands on the envelope
    /// as a multiple of what the field radiates. Zero strength is no beam.
    pub hot_spots: [Vec4; HOT_SPOTS],
    /// `(since_s, flash_s, afterglow_s, reach)`. `since_s` negative is a field still standing.
    ///
    /// The envelope turns into a sphere of debris that grows to `reach` times its size by the
    /// end of the afterglow, and is gone after it.
    pub collapse: Vec4,
    /// `(spike_k, limit_k, radius, 0)`: the flash's color temperature, the one the afterglow
    /// cools from, and the sphere the envelope rounds into first, mesh-local.
    pub collapse_k: Vec4,
    /// A blackbody at each [`ramp_kelvin`], through the host's bands: see [`ramp_entry`].
    pub spectrum: [Vec4; RAMP],
}

impl Default for FieldUniform {
    fn default() -> Self {
        Self {
            state: Vec4::new(400.0, 0.0, 0.3, 0.0),
            mode: Vec4::new(CLEAR, CLEAR, 1.0, 1.0),
            origin: Vec4::ZERO,
            to_star: Vec4::Z,
            starlight: Vec4::ZERO,
            exposure: Vec4::new(1.0, 5.0, 0.25, 0.0),
            hot_spots: [Vec4::ZERO; HOT_SPOTS],
            collapse: Vec4::new(-1.0, 0.5, 30.0, 4.0),
            collapse_k: Vec4::new(1.0e6, 4600.0, 1.0, 0.0),
            spectrum: [Vec4::splat(-1000.0); RAMP],
        }
    }
}

/// Which of the three draws a material is. Their order is the order they are drawn in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum FieldLayer {
    /// Back faces: the far side of the radiator, seen through the near side.
    #[default]
    Far,
    /// The clear inner layer: a fresnel rim, both faces.
    Inner,
    /// Front faces: the near side of the radiator, which alone can hide what is behind.
    Near,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FieldMaterialKey {
    layer: FieldLayer,
}

impl From<&FieldMaterial> for FieldMaterialKey {
    fn from(material: &FieldMaterial) -> Self {
        Self { layer: material.layer }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
#[bind_group_data(FieldMaterialKey)]
pub struct FieldMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: FieldUniform,
    pub layer: FieldLayer,
    /// How far apart the three layers sort, in view-space units.
    ///
    /// They share a mesh and so a center, and transparent draws are sorted by center: this is
    /// all that orders them. It must be well above `f32` spacing at the camera's distance and
    /// well below the gap to anything else transparent — a thousandth of the envelope is both.
    pub sort_step: f32,
}

impl FieldMaterial {
    /// The three draws of one field, in [`FieldLayer`] order.
    pub fn layers(uniforms: FieldUniform, envelope_size: f32) -> [Self; 3] {
        let sort_step = envelope_size * 1.0e-3;
        [FieldLayer::Far, FieldLayer::Inner, FieldLayer::Near]
            .map(|layer| Self { uniforms: uniforms.clone(), layer, sort_step })
    }
}

impl Material for FieldMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/field.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/field.wgsl".into()
    }

    /// Only the near wall can hide anything. The far wall is behind it, and the rim is inside
    /// it, so both are added, and the near wall's own alpha then covers them as it covers the
    /// hull.
    fn alpha_mode(&self) -> AlphaMode {
        match self.layer {
            FieldLayer::Near => AlphaMode::Premultiplied,
            FieldLayer::Far | FieldLayer::Inner => AlphaMode::Add,
        }
    }

    fn depth_bias(&self) -> f32 {
        self.layer as u8 as f32 * self.sort_step
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = match key.bind_group_data.layer {
            FieldLayer::Far => Some(Face::Front),
            FieldLayer::Inner => None,
            FieldLayer::Near => Some(Face::Back),
        };
        // Tested against the hull, so the hull hides the far wall; never written, so the walls
        // do not hide each other or the rim.
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = Some(false);
        }
        let layer = key.bind_group_data.layer as u8 as u32;
        // Both stages: they are one file, and the preprocessor refuses an undefined name.
        let def = ShaderDefVal::UInt("FIELD_LAYER".into(), layer);
        descriptor.vertex.shader_defs.push(def.clone());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push(def);
        }
        Ok(())
    }
}

pub struct FieldMaterialPlugin;

impl Plugin for FieldMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<FieldMaterial>::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ramp_spans_its_ends() {
        assert!((ramp_kelvin(0) - RAMP_MIN_K).abs() < 1e-3);
        assert!((ramp_kelvin(RAMP - 1) / RAMP_MAX_K - 1.0).abs() < 1e-4);
    }

    #[test]
    fn the_layers_sort_far_to_near() {
        let [far, inner, near] = FieldMaterial::layers(FieldUniform::default(), 100.0);
        assert!(far.depth_bias() < inner.depth_bias());
        assert!(inner.depth_bias() < near.depth_bias());
    }
}
