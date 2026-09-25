//! An emission's cone, drawn as an indicator of where its heat lands, and the aperture it leaves.
//!
//! Two materials for the two parts of one emission, each one draw:
//!
//! - [`ExhaustConeMaterial`] is the cone. A top-hat from a point: uniform inside the half-angle,
//!   nothing outside, flux `P / (Ω d²)` with `Ω = 4π sin²(θ/2)`. It is not light, so its color is
//!   the host's and only its *shape* is physics: each pixel reads the flux at the point where its
//!   view ray comes angularly closest to the axis, as seen from the apex, and maps it between the
//!   faint color at the drawn length and the hot color at the hot flux and nearer. Closed form, no
//!   march.
//! - [`ApertureGlowMaterial`] is the open face, a blackbody at whatever radiance the host works
//!   out, and a short near-field glow off it, both through the exposure.
//!
//! Local space is the proxy's. For the cone the apex is the origin, the axis `+y`, and the drawn
//! length is one, so its proxy is scaled uniformly by the length: the shader measures angles, which
//! a non-uniform scale would bend. For the aperture the face is the disk of radius one at `y = 0`,
//! facing `+y`. The host supplies `shaders/exhaust_cone.wgsl` and `shaders/aperture_glow.wgsl`.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;
use bevy_mesh::MeshVertexBufferLayoutRef;

/// How far the proxies stand off what they hold, as a fraction.
///
/// The proxies are polygons inscribed in their circles, so a flat side sits `1 − cos(π/N)` inside;
/// at [`SIDES`] that is half a percent, and the rest is room for the edge.
const MARGIN: f32 = 1.05;
const SIDES: u32 = 48;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct ExhaustConeUniform {
    /// `(power_w, half_angle_rad, length_m, hot_w_m2)`.
    ///
    /// The flux at `length_m` is where the cone fades out; at `hot_w_m2` and above it is drawn in
    /// [`Self::hot`]. Between the two the mix is logarithmic in flux, which is linear in the log
    /// of distance.
    pub emission: Vec4,
    /// Linear display color at the far end. `w` unused.
    pub faint: Vec4,
    /// Linear display color where the flux reaches `hot_w_m2`. `w` unused.
    pub hot: Vec4,
    /// The camera, in the proxy's own space: lengths from the apex. `w` unused.
    pub eye_local: Vec4,
}

impl Default for ExhaustConeUniform {
    fn default() -> Self {
        Self {
            emission: Vec4::new(1.0, 5f32.to_radians(), 1.0, 100.0),
            faint: Vec4::new(0.02, 0.004, 0.0, 0.0),
            hot: Vec4::new(0.6, 0.12, 0.02, 0.0),
            eye_local: Vec4::new(1.0, 0.5, 0.0, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct ExhaustConeMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: ExhaustConeUniform,
}

impl ExhaustConeMaterial {
    /// The proxy for a cone of `half_angle_rad`: a closed cone, apex at the origin, base at
    /// `y = 1`. Scale it uniformly by the length.
    pub fn proxy(half_angle_rad: f32) -> Mesh {
        cone_proxy(half_angle_rad.tan() * MARGIN)
    }
}

impl Material for ExhaustConeMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/exhaust_cone.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/exhaust_cone.wgsl".into()
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
        back_faces_only(descriptor);
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct ApertureGlowUniform {
    /// The face's radiance, band-mapped to linear display RGB, on the exposure's scale. `w` unused.
    pub face: Vec4,
    /// The near-field glow seen side-on through its middle, on the same scale. `w` unused.
    pub glow: Vec4,
    /// `(reach, width, _, _)`, in aperture radii: how far aft the glow runs and how wide it is.
    pub shape: Vec4,
    /// The camera in the proxy's own space, in aperture radii. `w` unused.
    pub eye_local: Vec4,
    /// `(surface_reference, stops, overflow, _)`, as [`crate::plume_material::PlumeUniform::exposure`]
    /// without the brightness, which is already in the colors.
    pub exposure: Vec4,
}

impl Default for ApertureGlowUniform {
    fn default() -> Self {
        Self {
            face: Vec4::ONE,
            glow: Vec4::ONE,
            shape: Vec4::new(6.0, 1.2, 0.0, 0.0),
            eye_local: Vec4::new(10.0, 3.0, 0.0, 0.0),
            exposure: Vec4::new(1.0, 5.0, 0.5, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct ApertureGlowMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: ApertureGlowUniform,
}

impl ApertureGlowMaterial {
    /// The glow's proxy, for [`ApertureGlowUniform::shape`]: a closed cylinder from the face to
    /// where the glow has run out, in aperture radii. Scale it uniformly by the aperture radius.
    pub fn proxy(reach: f32, width: f32) -> Mesh {
        // Three widths is `e⁻⁹` of the peak, and the glow's own center sits `reach / 2` aft.
        let radius = (width * GLOW_EXTENT).max(1.0) * MARGIN;
        let long = reach * (0.5 + 0.5 * GLOW_EXTENT);
        Cylinder::new(radius, long)
            .mesh()
            .resolution(SIDES)
            .segments(1)
            .build()
            .translated_by(Vec3::Y * long * 0.5)
    }
}

/// How many of its own radii the glow's proxy holds.
const GLOW_EXTENT: f32 = 3.0;

impl Material for ApertureGlowMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/aperture_glow.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/aperture_glow.wgsl".into()
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
        back_faces_only(descriptor);
        Ok(())
    }
}

/// One fragment per pixel, and one still there with the camera inside the proxy: each fragment
/// answers for its whole ray, so both faces would count it twice. See `plume_material`.
fn back_faces_only(descriptor: &mut RenderPipelineDescriptor) {
    descriptor.primitive.cull_mode = Some(Face::Front);
    if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
        depth_stencil.depth_write_enabled = Some(false);
    }
}

/// A closed cone, apex at the origin and base of `radius` at `y = 1`, wound outward.
///
/// Built rather than taken from `Cone`, whose apex is at `+h/2`: this puts the apex exactly on
/// the local origin, where the shader's angles are measured from.
fn cone_proxy(radius: f32) -> Mesh {
    let n = SIDES;
    let mut positions = vec![[0.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    for i in 0..n {
        let a = std::f32::consts::TAU * i as f32 / n as f32;
        positions.push([radius * a.cos(), 1.0, radius * a.sin()]);
    }
    let mut indices = Vec::with_capacity(6 * n as usize);
    for i in 0..n {
        let (a, b) = (2 + i, 2 + (i + 1) % n);
        // Counter-clockwise seen from outside: the side from beside, the base from beyond it.
        indices.extend_from_slice(&[0, a, b, 1, b, a]);
    }
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_indices(Indices::U32(indices))
}

pub struct ExhaustConeMaterialPlugin;

impl Plugin for ExhaustConeMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            MaterialPlugin::<ExhaustConeMaterial>::default(),
            MaterialPlugin::<ApertureGlowMaterial>::default(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every triangle faces away from the axis, so culling front faces leaves the far wall.
    #[test]
    fn the_cone_proxy_is_wound_outward() {
        let mesh = cone_proxy(0.1);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        let Some(Indices::U32(indices)) = mesh.indices() else { panic!("no indices") };
        let center = Vec3::new(0.0, 0.6, 0.0);
        for tri in indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from_array(p[tri[k] as usize]));
            let normal = (b - a).cross(c - a);
            let out = (a + b + c) / 3.0 - center;
            assert!(normal.dot(out) > 0.0, "{tri:?} faces inward");
        }
    }
}
