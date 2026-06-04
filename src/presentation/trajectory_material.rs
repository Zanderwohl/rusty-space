//! Custom material for trajectory tube rendering with emissive bloom and alpha fade.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

/// Custom material for trajectory tubes.
/// 
/// Brightness is encoded in vertex color alpha. The shader outputs:
/// - HDR emissive values (> 1.0) for bright segments, triggering Bloom
/// - Alpha-blended output for dim segments, producing a fade effect
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct TrajectoryMaterial {
    /// Base color multiplied with vertex color RGB
    #[uniform(0)]
    pub base_color: LinearRgba,
    
    /// Brightness threshold: above this, emit HDR; below, fade alpha
    #[uniform(0)]
    pub brightness_threshold: f32,
    
    /// Emission multiplier for bright segments (controls bloom intensity)
    #[uniform(0)]
    pub emission_strength: f32,
    
    /// Alpha blending mode
    pub alpha_mode: AlphaMode,
}

impl Default for TrajectoryMaterial {
    fn default() -> Self {
        Self {
            base_color: LinearRgba::new(0.12, 0.85, 0.45, 1.0), // VFD green with slight blue tint
            brightness_threshold: 0.3,
            emission_strength: 4.0,
            alpha_mode: AlphaMode::Blend,
        }
    }
}

impl Material for TrajectoryMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/trajectory.wgsl".into()
    }
    
    fn vertex_shader() -> ShaderRef {
        "shaders/trajectory.wgsl".into()
    }
    
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}

/// Plugin that registers the TrajectoryMaterial with Bevy's rendering system.
pub struct TrajectoryMaterialPlugin;

impl Plugin for TrajectoryMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<TrajectoryMaterial>::default());
    }
}
