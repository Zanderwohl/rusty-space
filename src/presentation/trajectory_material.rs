//! Custom material for trajectory tube rendering with emissive bloom and alpha fade.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

/// Canonical tube radius baked into trajectory tube meshes.
pub const TRAJECTORY_BASE_TUBE_RADIUS: f32 = 0.05;

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

    /// Canonical tube radius baked into mesh vertices.
    #[uniform(0)]
    pub base_tube_radius: f32,

    /// Desired tube radius for this frame (shader displaces vertices along normals).
    #[uniform(0)]
    pub target_tube_radius: f32,

    /// When > 0.5, trajectory vertex shader computes tube thickness from each
    /// vertex's world-space distance to the origin.
    #[uniform(0)]
    pub dynamic_thickness: f32,

    /// Brightness (0..1) at the front/leading end of the trajectory.
    /// The shader lerps `front -> back` using the per-vertex factor in vertex alpha.
    /// Defaults to an identity range (0..1) so meshes that bake a final brightness
    /// into alpha (markers) render unchanged.
    #[uniform(0)]
    pub front: f32,

    /// Brightness (0..1) at the back/trailing end of the trajectory.
    #[uniform(0)]
    pub back: f32,

    /// Camera exposure; the shader multiplies brightness by 2^-exposure.
    /// Defaults to 0.0 (no adjustment) for meshes that pre-bake exposure.
    #[uniform(0)]
    pub exposure: f32,

    /// Whole-trajectory brightness multiplier (the Glow setting). 1.0 is the
    /// baseline; higher values drive overbright HDR/bloom. Defaults to 1.0.
    #[uniform(0)]
    pub glow_gain: f32,

    /// Alpha blending mode
    pub alpha_mode: AlphaMode,
}

impl Default for TrajectoryMaterial {
    fn default() -> Self {
        Self {
            base_color: LinearRgba::new(0.12, 0.85, 0.45, 1.0), // VFD green with slight blue tint
            brightness_threshold: 0.3,
            emission_strength: 4.0,
            base_tube_radius: TRAJECTORY_BASE_TUBE_RADIUS,
            target_tube_radius: TRAJECTORY_BASE_TUBE_RADIUS,
            dynamic_thickness: 1.0,
            front: 0.0,
            back: 1.0,
            exposure: 0.0,
            glow_gain: 1.0,
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
