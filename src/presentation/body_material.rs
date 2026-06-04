//! Custom material for body wireframe rendering with emissive bloom.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

/// Custom material for body wireframe spheres.
///
/// Brightness is encoded in vertex color alpha. The shader outputs
/// HDR emissive values scaled by brightness for Bloom effect.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct BodyWireframeMaterial {
    /// Base color for the wireframe lines
    #[uniform(0)]
    pub base_color: LinearRgba,

    /// Emission multiplier (controls bloom intensity)
    #[uniform(0)]
    pub emission_strength: f32,

    /// Alpha blending mode
    pub alpha_mode: AlphaMode,
}

impl Default for BodyWireframeMaterial {
    fn default() -> Self {
        Self {
            base_color: LinearRgba::new(0.5, 0.5, 0.5, 1.0),
            emission_strength: 3.0,
            alpha_mode: AlphaMode::Opaque,
        }
    }
}

impl Material for BodyWireframeMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/body_wireframe.wgsl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/body_wireframe.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}

/// Plugin that registers the BodyWireframeMaterial with Bevy's rendering system.
pub struct BodyWireframeMaterialPlugin;

impl Plugin for BodyWireframeMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<BodyWireframeMaterial>::default());
    }
}
