//! A starfield shaded from physics rather than from a baked colour.
//!
//! A descendant of [`crate::local_starfield_material`] and of the application's catalogue
//! starfield, sharing their billboard technique: one mesh, four vertices per star, expanded in
//! the vertex stage. It does not replace either. The difference is what is baked. Those bake a
//! colour and a brightness, which is right when the input is a fixed apparent magnitude; this
//! bakes a temperature and a radius, and derives colour, brightness and apparent direction per
//! frame from uniforms, because a moving observer changes all three.
//!
//! The host supplies `shaders/starfield.wgsl` and the band lookup table; see
//! [`RelativisticStarfieldMaterial::band_lut`] for what the table has to contain.

use bevy::pbr::{MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError, VertexFormat,
};
use bevy::shader::ShaderRef;
use bevy_mesh::{MeshVertexAttribute, MeshVertexBufferLayoutRef};

/// Quad corner in `[-1, 1]^2`, which doubles as the radial falloff coordinate.
pub const ATTRIBUTE_STAR_CORNER: MeshVertexAttribute =
    MeshVertexAttribute::new("RelStarCorner", 0x5245_4C53_0001, VertexFormat::Float32x2);

/// Per-star physics: `(effective temperature K, radius m, unused, unused)`.
pub const ATTRIBUTE_STAR_PARAMS: MeshVertexAttribute =
    MeshVertexAttribute::new("RelStarParams", 0x5245_4C53_0002, VertexFormat::Float32x4);

/// What orbits it and is warm: `(temperature K, radiance over the star's disc, grey deficit, -)`.
///
/// All zero for a system with nothing in it, which is almost all of them. Steady rather than
/// per-frame: what a population absorbs is set by how much of the sky around the star it covers,
/// and that does not change as it orbits.
pub const ATTRIBUTE_STAR_WARM: MeshVertexAttribute =
    MeshVertexAttribute::new("RelStarWarm", 0x5245_4C53_0003, VertexFormat::Float32x4);

/// Bands carried from emission to display. Must match `em_spectra::BANDS`.
pub const BANDS: usize = 7;

#[derive(Clone, Debug, ShaderType)]
pub struct RelativisticStarfieldUniform {
    /// Band-to-display matrix by column: entry `b` is band `b`'s contribution to `(r, g, b)`.
    pub band_to_display: [Vec4; BANDS],
    /// Ship velocity as a fraction of `c`, render axes. `w` is unused.
    pub beta: Vec4,
    /// Ship position relative to the mesh's bake origin, light-years, render axes.
    pub ship_offset_ly: Vec4,
    /// Luminance that maps to the top of the displayed window.
    pub reference: f32,
    /// Stops of brightness a star field is spread over, wider than the tone map's window.
    pub point_stops: f32,
    /// Drawn radius of a point source, radians. The host converts these from pixels using the
    /// camera's field of view, so a star is the same size on screen whatever the window is.
    pub min_radius_rad: f32,
    pub max_radius_rad: f32,
    /// Extra radius per stop above the window, in units of `min_radius_rad`.
    pub glow_radius_gain: f32,
    /// Output gain for a source inside the window. Kept near 1 so that what blooms is the
    /// overflow and not simply everything: at 4, an ordinary star was already past the knee and
    /// a star twenty-four stops over was indistinguishable from one barely clipping.
    pub brightness: f32,
    /// HDR value added per stop above the window.
    ///
    /// Without it every source above the window clips to the same white and a star twenty-four
    /// stops over -- which is what one looks like from sixty astronomical units -- renders
    /// identically to one barely clipping. The overflow has to leave the shader as a number
    /// bloom can act on.
    pub overflow_gain: f32,
    /// How much of the output the glare around the source carries, against the source itself.
    pub halo_gain: f32,
    /// Lookup domain: `index = (log2(T) - log_t_min) * log_t_scale`.
    pub log_t_min: f32,
    pub log_t_scale: f32,
    pub lut_samples: f32,
}

impl Default for RelativisticStarfieldUniform {
    fn default() -> Self {
        Self {
            band_to_display: [Vec4::ZERO; BANDS],
            beta: Vec4::ZERO,
            ship_offset_ly: Vec4::ZERO,
            reference: 1.0,
            point_stops: 14.0,
            // Replaced by the host from pixels and the camera's field of view; these are the
            // 90-degree, 1280-wide values so a headless default is not sub-pixel.
            min_radius_rad: 1.4e-3,
            max_radius_rad: 8.0e-3,
            glow_radius_gain: 0.35,
            brightness: 1.2,
            overflow_gain: 1.0,
            halo_gain: 0.3,
            log_t_min: 0.0,
            log_t_scale: 1.0,
            lut_samples: 1.0,
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct RelativisticStarfieldMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: RelativisticStarfieldUniform,
    /// `log2` of band radiance: one column per temperature sample, one row per band.
    ///
    /// Read with `textureLoad` and interpolated in the shader, so it is deliberately not
    /// filterable — a 32-bit float texture cannot be sampled with a filtering sampler under
    /// WebGPU, and interpolating log2 is the right thing to do anyway.
    #[texture(1, sample_type = "float", filterable = false, visibility(vertex, fragment))]
    pub band_lut: Handle<Image>,
}

impl Material for RelativisticStarfieldMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/starfield.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/starfield.wgsl".into()
    }

    /// Point sources sum where they overlap, so the blend is additive rather than ordered.
    ///
    /// **The fragment shader must return alpha zero.** `AlphaMode::Add` is premultiplied
    /// blending, `src + dst * (1 - alpha)`, so an alpha of one is not addition at all -- it is
    /// an overwrite. Returning one made every faint star punch a dark square through the glare
    /// of a bright one, and because two transparent meshes at the same depth are sorted with an
    /// arbitrary tie-break, those squares flickered frame to frame. It looked exactly like
    /// z-fighting and it was blend order.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Add
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Custom attributes rather than Mesh::ATTRIBUTE_COLOR: the built-in one enables the
        // VERTEX_COLORS shader def, which injects a @location(7) varying into the generated
        // prepass pipeline that then fails validation against this layout.
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            ATTRIBUTE_STAR_CORNER.at_shader_location(1),
            ATTRIBUTE_STAR_PARAMS.at_shader_location(2),
            ATTRIBUTE_STAR_WARM.at_shader_location(3),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];
        descriptor.primitive.cull_mode = None;
        if let Some(depth_stencil) = descriptor.depth_stencil.as_mut() {
            depth_stencil.depth_write_enabled = false;
        }
        Ok(())
    }
}

pub struct RelativisticStarfieldMaterialPlugin;

impl Plugin for RelativisticStarfieldMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<RelativisticStarfieldMaterial>::default());
    }
}
