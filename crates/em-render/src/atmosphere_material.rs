//! A rocky world's air, seen against space: a shell a little larger than the body, lit by
//! single scattering. The host supplies `shaders/atmosphere.wgsl`, and the surface shader
//! scatters the same air over the disc, so the shell draws only the limb.

use bevy_mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

/// Scale heights from the surface to the shell. The density there is `e^-6`, a quarter of a
/// per cent: past it nothing scatters enough to see.
pub const TOP_HEIGHTS: f32 = 6.0;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct AtmosphereUniform {
    /// World direction to the star.
    pub to_star: Vec4,
    /// See [`crate::body_surface_material::BodySurfaceUniform::starlight`].
    pub starlight: Vec4,
    /// `(surface_reference, stops, 0, 0)`: the tone map, as the surface evaluates it.
    pub exposure: Vec4,
    /// Each is per display channel, averaged over the bands the current mapping puts there.
    ///
    /// Vertical optical depth of the gas; `w` is its scale height as a share of the body's
    /// radius.
    pub gas: Vec4,
    /// Vertical optical depth of the haze; `w` is the air's vertical depth at ten microns, which
    /// absorbs and does not scatter.
    pub haze: Vec4,
    /// The haze's single-scattering albedo.
    pub albedo: Vec4,
    /// Display light from the air's own heat where it is opaque at ten microns.
    pub glow: Vec4,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct AtmosphereMaterial {
    #[uniform(0)]
    pub uniforms: AtmosphereUniform,
}

impl Material for AtmosphereMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/atmosphere.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/atmosphere.wgsl".into()
    }

    /// Additive, so the fragment returns alpha zero: light added to what is behind it.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Back faces, one fragment a pixel, still there when the camera is inside the shell.
        // Over the disc they are behind the body, which writes depth, so the limb is all that
        // is left.
        descriptor.primitive.cull_mode = Some(Face::Front);
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// The scattering both this and the surface import, as `lightcone::scatter`.
const SCATTER: &str = "shaders/scatter.wgsl";

/// Holds [`SCATTER`] loaded. A module reached only through `#import` is never requested on its
/// own, and a shader whose import is unresolved never finishes compiling: whatever uses it
/// silently draws nothing.
#[derive(Resource)]
struct ScatterShader(#[allow(dead_code)] Handle<Shader>);

/// Loads [`SCATTER`]; every material that imports it adds this, and a second add is a no-op.
pub struct ScatterShaderPlugin;

impl Plugin for ScatterShaderPlugin {
    fn build(&self, app: &mut App) {
        let handle = app.world().resource::<AssetServer>().load::<Shader>(SCATTER);
        app.insert_resource(ScatterShader(handle));
    }

    fn is_unique(&self) -> bool {
        false
    }
}

pub struct AtmosphereMaterialPlugin;

impl Plugin for AtmosphereMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((ScatterShaderPlugin, MaterialPlugin::<AtmosphereMaterial>::default()));
    }
}
