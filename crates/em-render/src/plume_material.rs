//! A drive's exhaust, drawn as a volume of glowing gas.
//!
//! A plume is not a surface, so drawing a cone's skin would be drawing a paper cone: the edges
//! would be as bright as the middle and the silhouette would be a hard line. The mesh is a
//! *proxy* instead — a closed cylinder that merely has to contain the gas — and each fragment
//! integrates the density along its own view ray. Feathered edges and a bright core come out
//! of that rather than being painted on, and the proxy's own shape never shows.
//!
//! The gas is not uniform either. A drive burns fuel-rich, and the flow combs whatever leaves
//! the injector unmixed into lengthwise streaks of cooler, sootier gas, so a sample is two gases
//! with two colors rather than one scaled. The streaks travel aft with [`PlumeUniform::churn`],
//! whose phase the host advances on *simulation* time — a stopped clock is a still plume.
//!
//! Local space is the proxy's: the axis is `+y` running from `-0.5` at the nozzle to `+0.5` at
//! the far end, and a radius of one is the proxy's wall. The host supplies
//! `shaders/plume.wgsl`.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

/// Steps taken through the volume per fragment.
///
/// A plume is convex and its density is smooth, so this is about resolving the profile rather
/// than about not missing anything. Two dozen is past where the banding stops being visible.
pub const STEPS: u32 = 24;

/// The churn's period along the flow, in lattice cells of its first octave.
///
/// The shader's hash repeats on it, which is what makes wrapping the phase invisible. Wrapped
/// it must be: the clock reaches tens of millions of times real time and a phase that only grew
/// would leave `f32`'s useful spacing while somebody was still looking at it. `plume.wgsl`
/// carries the same number.
pub const CHURN_PERIOD: f32 = 64.0;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct PlumeUniform {
    /// The exhaust's own light, band-mapped to linear display red, green and blue. `w` unused.
    ///
    /// A blackbody at the plume's temperature, exactly as a hull or a world is — a drive's
    /// output is thermal and this is where it goes. It is the *whole* of the plume's color:
    /// there is nothing out here to reflect and nothing lighting it.
    pub glow: Vec4,
    /// `(throat, mouth, edge, taper)`, the first two as fractions of the proxy's radius.
    ///
    /// `mouth / throat` is the expansion ratio — how much wider the gas is where it ends than
    /// where it leaves the nozzle — and it is the one number that decides a plume's shape.
    /// `edge` is how sharply the density falls off to the side: high is a pencil, low is a
    /// flare. `taper` is how fast it thins along its length as it spreads and cools.
    pub shape: Vec4,
    /// Where the camera is, in the proxy's own space. `w` unused.
    ///
    /// Passed rather than derived: the march needs the ray in local coordinates, and inverting
    /// the model matrix in the shader to get there is both fiddly and wasteful when the answer
    /// is one vector the host already has.
    pub eye_local: Vec4,
    /// `(surface_reference, stops, brightness, overflow)`.
    ///
    /// The same tone map the lit surfaces evaluate, for the same reason: a plume beside a
    /// planet has to sit in one exposure rather than two that agree. `overflow` is what a stop
    /// past the top of the window is worth as HDR value, so the deep middle of the cone leaves
    /// above one and becomes a halo instead of clipping flat — the same bargain the starfield
    /// makes with a star that is twenty stops over.
    pub exposure: Vec4,
    /// What the fuel-rich streaks radiate, on the same scale as [`Self::glow`]. `w` unused.
    ///
    /// A cooler graybody: unmixed fuel burns colder than the core and the soot it leaves is the
    /// one part of a plume that is not optically thin, so it emits less than a blackbody as well
    /// as redder. Both of those are the host's to work out — this is only where the answer goes.
    pub soot: Vec4,
    /// `(phase, across, along, bite)`.
    ///
    /// `phase` slides the pattern aft, in lattice cells, and the host wraps it at
    /// [`CHURN_PERIOD`]. `across` is how many lanes go round the plume and `along` how many
    /// lattice cells span its length — their ratio is how stretched a streak is, and a streak
    /// that is not stretched is a cloud. `bite` is how much of the gas the streaks may claim.
    pub churn: Vec4,
}

impl Default for PlumeUniform {
    fn default() -> Self {
        Self {
            glow: Vec4::ONE,
            shape: Vec4::new(0.12, 0.85, 2.5, 1.5),
            eye_local: Vec4::new(0.0, 0.0, -10.0, 0.0),
            exposure: Vec4::new(1.0, 2.5, 1.0, 0.5),
            soot: Vec4::ZERO,
            churn: Vec4::new(0.0, 5.0, 1.5, 1.0),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct PlumeMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: PlumeUniform,
}

impl Material for PlumeMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/plume.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/plume.wgsl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // **Back faces only, and the proxy must be closed.** Each fragment integrates the whole
        // ray, so one fragment per pixel is the requirement: drawing both faces would run the
        // march twice and double every plume. Back faces rather than front ones because they
        // are the ones still there when the camera is inside the proxy, which happens the
        // moment anybody looks at their own ship from astern.
        descriptor.primitive.cull_mode = Some(Face::Front);
        // Gas does not hide what is behind it.
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

pub struct PlumeMaterialPlugin;

impl Plugin for PlumeMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PlumeMaterial>::default());
    }
}
