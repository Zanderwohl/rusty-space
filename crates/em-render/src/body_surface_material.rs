//! A resolved body's surface: a pattern the host bakes, colored by a palette the body's class
//! gives, or a color map of its own, and optionally a cloud deck over either. The host supplies
//! `shaders/body_surface.wgsl`.
//!
//! A cloud deck evolves. The host bakes its weather as a series of keyframes, each an
//! independent draw of the same noise, and the shader blends two neighbors with weights whose
//! squares sum to one, so the blend has the same contrast as either end. Coverage is taken from
//! the blend rather than blended, which is what makes clouds grow and part instead of
//! cross-dissolving. A third slot is where the host bakes the next keyframe while the other two
//! are drawn.

use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct BodySurfaceUniform {
    pub dark: Vec4,
    pub light: Vec4,
    /// World direction to the star; `w` is the ambient floor on the night side.
    pub to_star: Vec4,
    /// `(color, contrast, clouds, unused)`. `color` is 1 where [`BodySurfaceMaterial::color`]
    /// replaces the pattern and palette, and `clouds` is 1 where a cloud deck is drawn over the
    /// surface.
    pub params: Vec4,
    /// Starlight the surface reflects, as linear display light before the tone map. `w` unused.
    pub reflected: Vec4,
    /// Light the body makes itself, in the same units; `w` is how far the pattern inverts in it.
    ///
    /// Separate from the reflected term rather than summed into it, because only the reflected
    /// half is Lambert-shaded. A body radiates at its own temperature whichever way it is
    /// turned, which is why a gas giant's night side is as bright at ten microns as its day
    /// side. In the optical this is simply zero, so nothing about a sunlit planet changes.
    pub emitted: Vec4,
    /// `(surface_reference, stops, 0, 0)`: the tone map, for the shader to evaluate itself.
    ///
    /// Per fragment rather than per body, because the mix of reflected and emitted light
    /// changes across the disc and the tone map is logarithmic — mapping the two separately and
    /// adding the results put Jupiter's day side twice its night side at ten microns where the
    /// true ratio is 1.14. The star field already evaluates the same curve per star.
    pub exposure: Vec4,
    /// The weight of each weather slot, and in `w` the weather's mean over the sphere, about
    /// which the blend is taken. The squares of the weights sum to one.
    pub weather: Vec4,
    /// How far the equator's easterlies have carried each slot's weather westward, radians. The
    /// shader shapes it by latitude, and it is negative before the slot's keyframe.
    pub drift: Vec4,
}

impl Default for BodySurfaceUniform {
    fn default() -> Self {
        Self {
            dark: Vec4::new(0.13, 0.12, 0.11, 1.0),
            light: Vec4::new(0.34, 0.32, 0.29, 1.0),
            to_star: Vec4::new(0.0, 0.0, 1.0, 0.015),
            params: Vec4::new(0.0, 0.9, 0.0, 0.0),
            reflected: Vec4::ONE,
            emitted: Vec4::ZERO,
            exposure: Vec4::new(1.0, 2.5, 0.0, 0.0),
            weather: Vec4::ZERO,
            drift: Vec4::ZERO,
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct BodySurfaceMaterial {
    #[uniform(0, visibility(vertex, fragment))]
    pub uniforms: BodySurfaceUniform,
    /// Where on the palette each direction of the body sits, `[0, 1]`: a single-channel cubemap
    /// sampled at the body-fixed direction, so the pattern turns with the body.
    #[texture(1, dimension = "cube", visibility(fragment))]
    #[sampler(2, visibility(fragment))]
    pub pattern: Handle<Image>,
    /// The surface's own albedo, an sRGB cubemap, when `params.x` says so. Sampled with the
    /// pattern's sampler.
    #[texture(3, dimension = "cube", visibility(fragment))]
    pub color: Handle<Image>,
    /// The cloud deck's weather, one keyframe a slot: unbounded single-channel cubemaps, drawn
    /// when `params.z` says so.
    #[texture(4, dimension = "cube", visibility(fragment))]
    pub weather_0: Handle<Image>,
    #[texture(5, dimension = "cube", visibility(fragment))]
    pub weather_1: Handle<Image>,
    #[texture(6, dimension = "cube", visibility(fragment))]
    pub weather_2: Handle<Image>,
    /// What the deck's weather is added to and does not change: its belts.
    #[texture(7, dimension = "cube", visibility(fragment))]
    pub climate: Handle<Image>,
}

impl Material for BodySurfaceMaterial {
    fn vertex_shader() -> ShaderRef {
        "shaders/body_surface.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "shaders/body_surface.wgsl".into()
    }

    /// Opaque, and the only thing in this renderer that writes depth. That is what puts the far
    /// half of a ring behind its planet.
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }
}

pub struct BodySurfaceMaterialPlugin;

impl Plugin for BodySurfaceMaterialPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<BodySurfaceMaterial>::default());
    }
}
