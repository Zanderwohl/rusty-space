//! Custom material for distant body point rendering with emissive bloom.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

/// Custom material for distant body point sprites.
///
/// Brightness is pre-computed on the CPU from phase angles to all stars.
/// The shader outputs HDR emissive values for Bloom effect.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct BodyPointMaterial {
    /// Base color (body albedo from AppearanceColor)
    #[uniform(0)]
    pub base_color: LinearRgba,

    /// Pre-computed brightness from phase angle calculations (0.0 to 1.0)
    #[uniform(0)]
    pub brightness: f32,

    /// Emission multiplier (controls bloom intensity)
    #[uniform(0)]
    pub emission_strength: f32,

    /// Alpha blending mode
    pub alpha_mode: AlphaMode,
}

impl Default for BodyPointMaterial {
    fn default() -> Self {
        Self {
            base_color: LinearRgba::new(0.5, 0.5, 0.5, 1.0),
            brightness: 1.0,
            emission_strength: 5.0,
            alpha_mode: AlphaMode::Blend,
        }
    }
}

impl Material for BodyPointMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/body_point.wgsl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/body_point.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}

/// Plugin that registers the BodyPointMaterial with Bevy's rendering system.
pub struct BodyPointMaterialPlugin;

impl Plugin for BodyPointMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<BodyPointMaterial>::default());
    }
}
