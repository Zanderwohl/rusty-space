//! Materials for the sphere-of-influence shell.
//!
//! Both the limb ring and the point cloud carry the same [`SoiShapeUniform`] block, and
//! both read it through the one WGSL function in `shaders/soi_shape.wgsl`. Keeping the
//! shape in a single struct on both sides is what makes a new SOI model one match arm
//! rather than an edit in four places.
//!
//! Every field order here is load-bearing: `AsBindGroup` packs in declaration order and
//! the WGSL structs mirror it line for line. There is deliberately no explicit tail
//! padding — naga_oil refuses identifiers a composable module would have to rewrite (which
//! `pad0` is), and `encase` and naga agree on the trailing padding without being told.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};

/// Quad corner offset in `[-1, 1]^2`, for the point cloud's billboards.
pub const ATTRIBUTE_SOI_CORNER: MeshVertexAttribute =
    MeshVertexAttribute::new("SoiCorner", 0x534F_495F_434F_524E, VertexFormat::Float32x2);

/// The shared shape function, imported by both SOI shaders via `#define_import_path`.
const SOI_SHAPE_SHADER: &str = "shaders/soi_shape.wgsl";

/// Keeps the shared shape module alive.
///
/// A WGSL module reached only through `#import` is never requested by the asset server on
/// its own, and a shader whose import is unresolved does not fail loudly — it simply never
/// finishes compiling, so meshes using it silently draw nothing. Holding a handle forces
/// the module to load before either material's pipeline specialises.
#[derive(Resource)]
struct SoiShapeShader(#[allow(dead_code)] Handle<Shader>);

/// Loads [`SOI_SHAPE_SHADER`]. Added by both material plugins; adding it twice is a no-op.
pub struct SoiShapeShaderPlugin;

impl Plugin for SoiShapeShaderPlugin {
    fn build(&self, app: &mut App) {
        let handle = app.world().resource::<AssetServer>().load::<Shader>(SOI_SHAPE_SHADER);
        app.insert_resource(SoiShapeShader(handle));
    }
}

/// Both SOI materials need the shared module; whichever plugin is added first loads it.
fn ensure_shape_shader(app: &mut App) {
    if !app.is_plugin_added::<SoiShapeShaderPlugin>() {
        app.add_plugins(SoiShapeShaderPlugin);
    }
}

/// The shape of one sphere of influence, as the shaders see it.
///
/// Radii are in render units — metres times the view's distance factor — and
/// `primary_dir` is in render space. Mirrors `SoiShape` in `shaders/soi_shape.wgsl`.
#[derive(Clone, Debug, Default, ShaderType)]
pub struct SoiShapeUniform {
    /// xyz: unit vector body -> primary, render space. w unused.
    pub primary_dir: Vec4,
    /// The `em_sim::influence::SoiModel` discriminant, in declaration order.
    pub model: u32,
    /// The widest radius. An isotropic model returns this for every direction.
    pub radius: f32,
    pub bounding_radius: f32,
    pub min_radius: f32,
    /// `> 0.5` when the limb ring must solve iteratively rather than in closed form.
    pub anisotropic: f32,
    /// Screen-size LOD fade, 0..1.
    pub fade: f32,
}

// === Point cloud ===

#[derive(Clone, Debug, ShaderType)]
pub struct SoiPointsUniform {
    pub shape: SoiShapeUniform,
    pub base_color: Vec4,
    /// Angular radius of one dot, radians. Constant on screen, so a dot is the same few
    /// pixels whether the shell is Luna's or the Sun's.
    pub point_angular_radius: f32,
    /// How sharply the cloud concentrates at the limb. Higher clears the middle more.
    pub limb_power: f32,
    /// Alpha for a dot facing the camera head-on, where the shell reads as interior.
    pub interior_alpha: f32,
    pub brightness: f32,
    pub emission_strength: f32,
}

impl Default for SoiPointsUniform {
    fn default() -> Self {
        Self {
            shape: SoiShapeUniform::default(),
            base_color: Vec4::new(0.35, 0.75, 1.0, 1.0),
            point_angular_radius: 0.0035,
            limb_power: 2.5,
            interior_alpha: 0.22,
            brightness: 1.0,
            emission_strength: 1.6,
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct SoiPointsMaterial {
    #[uniform(0)]
    pub uniform: SoiPointsUniform,
}

impl Material for SoiPointsMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/soi_points.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/soi_points.wgsl".into()
    }

    /// Additive, and paired with the disabled depth write below. Between them a shell
    /// cannot occlude anything — not the body inside it, not a nested child's shell — by
    /// construction rather than by getting a draw order right. Additive is also
    /// order-independent, so overlapping shells simply read brighter.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTRIBUTE_SOI_CORNER.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];

        // Billboards face the camera, so there is no meaningful facing to cull.
        descriptor.primitive.cull_mode = None;

        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }

        Ok(())
    }
}

pub struct SoiPointsMaterialPlugin;

impl Plugin for SoiPointsMaterialPlugin {
    fn build(&self, app: &mut App) {
        ensure_shape_shader(app);
        app.add_plugins(MaterialPlugin::<SoiPointsMaterial>::default());
    }
}

// === Limb ring ===

#[derive(Clone, Debug, ShaderType)]
pub struct SoiRingUniform {
    pub shape: SoiShapeUniform,
    pub base_color: Vec4,
    /// Half-thickness of the tube, in render units. Fed from
    /// [`super::trajectory::calculate_tube_radius`] so SOI rings carry the same screen
    /// weight as trajectory lines.
    pub tube_radius: f32,
    pub emission_strength: f32,
    pub ring_alpha: f32,
}

impl Default for SoiRingUniform {
    fn default() -> Self {
        Self {
            shape: SoiShapeUniform::default(),
            base_color: Vec4::new(0.35, 0.75, 1.0, 1.0),
            tube_radius: 0.05,
            emission_strength: 4.0,
            ring_alpha: 0.9,
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct SoiRingMaterial {
    #[uniform(0)]
    pub uniform: SoiRingUniform,
}

impl Material for SoiRingMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/soi_ring.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/soi_ring.wgsl".into()
    }

    /// Additive and depth-write-free, for the same reason as the point cloud: a shell
    /// must never occlude what it encloses.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    /// The ring uses POSITION and NORMAL, so the default vertex layout already fits and
    /// this exists only for the pipeline state.
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The silhouette curve can double back toward the viewer, so the tube passes
        // through itself and both facings need to draw.
        descriptor.primitive.cull_mode = None;

        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }

        Ok(())
    }
}

pub struct SoiRingMaterialPlugin;

impl Plugin for SoiRingMaterialPlugin {
    fn build(&self, app: &mut App) {
        ensure_shape_shader(app);
        app.add_plugins(MaterialPlugin::<SoiRingMaterial>::default());
    }
}
