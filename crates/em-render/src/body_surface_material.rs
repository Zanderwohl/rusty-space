//! A resolved body's surface: a pattern the host bakes, colored by a palette the body's class
//! gives, or a color map of its own, and optionally a cloud deck over either. The host supplies
//! `shaders/body_surface.wgsl`.
//!
//! A cloud deck is keyframes of weather, blended by the shader, over a climate that is fixed.

use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

#[derive(Clone, Debug, PartialEq, ShaderType)]
pub struct BodySurfaceUniform {
    pub dark: Vec4,
    pub light: Vec4,
    /// World direction to the star; `w` is the ambient floor on the night side.
    pub to_star: Vec4,
    /// `(color, contrast, clouds, grounds)`. `color` is 1 where [`BodySurfaceMaterial::color`]
    /// replaces the pattern and palette, `clouds` is 1 where a cloud deck is drawn over the
    /// surface, and `grounds` is 1 where the masks and [`Self::ground`] say what each band sees.
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
    /// Each weather slot's weight, the squares summing to one; `w` is the weather's mean.
    pub weather: Vec4,
    /// Each slot's westward drift at the equator, radians; negative before its keyframe.
    pub drift: Vec4,
    /// `(cover, opacity, 0, 0)`: added to the deck's drive, and what share of its alpha is drawn.
    /// An opacity above one closes the gaps.
    pub deck: Vec4,
    /// Multiplies the deck's gray, linear. White for water cloud.
    pub deck_tint: Vec4,
    /// What an atmosphere scatters, as linear display light: the starlight a white surface of
    /// this body facing the star would send.
    pub starlight: Vec4,
    /// See [`crate::atmosphere_material::AtmosphereUniform`]. Zero for a body without air.
    pub air_gas: Vec4,
    pub air_haze: Vec4,
    pub air_albedo: Vec4,
    pub air_glow: Vec4,
    /// Each kind of ground's albedo through the current band mapping, as display channels:
    /// water, ice, growth, sand, rock and cloud.
    pub ground: [Vec4; GROUNDS],
    /// The same through the natural mapping, which is what the color cubemap was painted in.
    /// The color is scaled by the ratio of the two, so in the natural mapping nothing changes.
    pub ground_natural: [Vec4; GROUNDS],
    /// Per band, what it adds to the display channels from a blackbody at `thermal.x`, and in
    /// `w` its center wavelength in microns. The shader scales each by the Planck ratio at the
    /// temperature it works out, so at `thermal.x` with unit emissivity this sums to `emitted`.
    pub bands: [Vec4; BANDS],
    /// `(mean temperature K, how far the air evens day and night, 0, on)`. With `on`, the shader
    /// works out each fragment's own temperature and emission in place of `emitted`.
    pub thermal: Vec4,
    /// Each ground's emissivity, two to a ground: `(B, V, R, I)` then `(K, 10um, radio,
    /// inertia)`, in [`Self::ground`]'s order.
    pub emissivity: [Vec4; 2 * GROUNDS],
}

/// em_spectra's band count, which this crate does not depend on for one number.
pub const BANDS: usize = 7;

/// Kinds of ground a surface mixes: see [`BodySurfaceUniform::ground`].
pub const GROUNDS: usize = 6;

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
            deck: Vec4::new(0.0, 1.0, 0.0, 0.0),
            deck_tint: Vec4::ONE,
            starlight: Vec4::ZERO,
            air_gas: Vec4::ZERO,
            air_haze: Vec4::ZERO,
            air_albedo: Vec4::ZERO,
            air_glow: Vec4::ZERO,
            ground: [Vec4::ONE; GROUNDS],
            ground_natural: [Vec4::ONE; GROUNDS],
            bands: [Vec4::ZERO; BANDS],
            thermal: Vec4::ZERO,
            emissivity: [Vec4::ONE; 2 * GROUNDS],
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
    /// One weather keyframe a slot, single-channel.
    #[texture(4, dimension = "cube", visibility(fragment))]
    pub weather_0: Handle<Image>,
    #[texture(5, dimension = "cube", visibility(fragment))]
    pub weather_1: Handle<Image>,
    #[texture(6, dimension = "cube", visibility(fragment))]
    pub weather_2: Handle<Image>,
    /// The deck's fixed belts.
    #[texture(7, dimension = "cube", visibility(fragment))]
    pub climate: Handle<Image>,
    /// How much of each texel is land, ice, growth and sand, single-channel: what the color
    /// cubemap's ground is made of.
    #[texture(8, dimension = "cube", visibility(fragment))]
    pub land: Handle<Image>,
    #[texture(9, dimension = "cube", visibility(fragment))]
    pub ice: Handle<Image>,
    #[texture(10, dimension = "cube", visibility(fragment))]
    pub growth: Handle<Image>,
    #[texture(11, dimension = "cube", visibility(fragment))]
    pub sand: Handle<Image>,
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
        app.add_plugins((
            crate::atmosphere_material::ScatterShaderPlugin,
            MaterialPlugin::<BodySurfaceMaterial>::default(),
        ));
    }
}
