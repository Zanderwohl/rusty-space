//! Material for the encounter marker drawn at a sphere-of-influence crossing.
//!
//! Field order is load-bearing: `AsBindGroup` packs in declaration order and
//! `shaders/encounter_marker.wgsl` mirrors it. No explicit tail padding — naga_oil rejects
//! identifiers a composable module would have to rewrite, and `encase` and naga agree on
//! the trailing padding unprompted.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

#[derive(Clone, Debug, ShaderType)]
pub struct EncounterMarkerUniform {
    /// The face of the target, render space: two perpendicular axes, both square to the
    /// traveller's velocity, so the craft flies into the face rather than along it.
    pub plane_x: Vec4,
    pub plane_y: Vec4,
    pub base_color: Vec4,
    /// Angular radius of each circle, radians. `w` unused.
    pub ring_radii: Vec4,
    /// Angular radius of the tube the circles are drawn from, radians.
    pub tube_radius: f32,
    pub emission_strength: f32,
    pub alpha: f32,
}

impl Default for EncounterMarkerUniform {
    fn default() -> Self {
        Self {
            plane_x: Vec4::X,
            plane_y: Vec4::Y,
            // Red, as the reticle colour that means "something happens here".
            base_color: Vec4::new(1.0, 0.18, 0.14, 1.0),
            ring_radii: Vec4::new(0.016, 0.026, 0.036, 0.0),
            tube_radius: 0.0012,
            emission_strength: 4.0,
            alpha: 1.0,
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct EncounterMarkerMaterial {
    #[uniform(0)]
    pub uniform: EncounterMarkerUniform,
}

impl Material for EncounterMarkerMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/encounter_marker.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/encounter_marker.wgsl".into()
    }

    /// Additive, like the trajectory and sphere shells: a marker annotates the scene and
    /// must never hide any of it.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    /// POSITION and NORMAL only, so the default vertex layout fits; this is here for the
    /// pipeline state.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;

        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }

        Ok(())
    }
}

pub struct EncounterMarkerMaterialPlugin;

impl Plugin for EncounterMarkerMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<EncounterMarkerMaterial>::default());
    }
}
