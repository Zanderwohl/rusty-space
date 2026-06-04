//! Custom material for body wireframe rendering with emissive bloom.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

/// Maximum number of suns supported for day/night lighting.
pub const MAX_SUNS: usize = 4;

/// Custom material for body wireframe spheres.
///
/// Brightness is encoded in vertex color alpha. The shader outputs
/// HDR emissive values scaled by brightness for Bloom effect.
/// Sun directions (in body-local space) modulate brightness:
/// night side gets 80% base, each illuminating sun adds 20%.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct BodyWireframeMaterial {
    #[uniform(0)]
    pub base_color: LinearRgba,

    #[uniform(0)]
    pub emission_strength: f32,

    #[uniform(0)]
    pub num_suns: u32,

    #[uniform(0)]
    pub sun_dir_0: Vec4,

    #[uniform(0)]
    pub sun_dir_1: Vec4,

    #[uniform(0)]
    pub sun_dir_2: Vec4,

    #[uniform(0)]
    pub sun_dir_3: Vec4,

    pub alpha_mode: AlphaMode,
}

impl Default for BodyWireframeMaterial {
    fn default() -> Self {
        Self {
            base_color: LinearRgba::new(0.5, 0.5, 0.5, 1.0),
            emission_strength: 3.0,
            num_suns: 0,
            sun_dir_0: Vec4::ZERO,
            sun_dir_1: Vec4::ZERO,
            sun_dir_2: Vec4::ZERO,
            sun_dir_3: Vec4::ZERO,
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

/// Custom material for body occluder spheres with Lambert day/night shading.
///
/// Renders a solid sphere that is fully dark on the night side and shows
/// a subtle VFD-green Lambert-shaded surface on the day side.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct OccluderMaterial {
    #[uniform(0)]
    pub base_color: LinearRgba,

    #[uniform(0)]
    pub num_suns: u32,

    #[uniform(0)]
    pub sun_dir_0: Vec4,

    #[uniform(0)]
    pub sun_dir_1: Vec4,

    #[uniform(0)]
    pub sun_dir_2: Vec4,

    #[uniform(0)]
    pub sun_dir_3: Vec4,

    pub alpha_mode: AlphaMode,
}

impl Default for OccluderMaterial {
    fn default() -> Self {
        Self {
            base_color: LinearRgba::new(0.015, 0.10, 0.05, 1.0),
            num_suns: 0,
            sun_dir_0: Vec4::ZERO,
            sun_dir_1: Vec4::ZERO,
            sun_dir_2: Vec4::ZERO,
            sun_dir_3: Vec4::ZERO,
            alpha_mode: AlphaMode::Opaque,
        }
    }
}

impl Material for OccluderMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/body_occluder.wgsl".into()
    }

    fn vertex_shader() -> ShaderRef {
        "shaders/body_occluder.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }
}

/// Plugin that registers the OccluderMaterial with Bevy's rendering system.
pub struct OccluderMaterialPlugin;

impl Plugin for OccluderMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<OccluderMaterial>::default());
    }
}
