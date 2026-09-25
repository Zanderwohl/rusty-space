//! Population envelopes: the volumes that stand in for swarms, belts and clouds.
//!
//! A population has no members, so nothing is instanced from it and nothing invents members to
//! instance. The *distribution* is drawn instead: a convex proxy whose fragments march the
//! population's own density field. The three distributions a population carries say where the
//! material is — semi-major axis and eccentricity radially, inclination in latitude — and the
//! field is those two profiles and nothing else, which is why one shape covers a belt and an
//! isotropic swarm with no special case anywhere.
//!
//! This replaced a torus of proxy geometry with the density painted on its surface. That could
//! not say how much material a sightline crossed: from inside a belt every sightline meets the
//! far wall exactly once and nearly face-on, so looking along the belt and looking at the pole
//! came out within four per cent of each other where the material differs by a factor of seven.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::population_material::{
    ATTRIBUTE_SHELL_DENSITY, PROFILE_LATITUDE, PROFILE_RADIAL, PROFILE_SAMPLES,
    PopulationMaterial, PopulationUniform,
};
use em_render::render_space::sim_to_render;
use glam::DVec3;
use lc_world::population::Population;

use crate::system::{M_PER_LY, UNIT_M};

/// Longitude divisions of a ring.
pub const SEGMENTS: usize = 96;

/// Overall gain on the mapped opacity.
///
/// Above one because the compressed values are still small: a belt lands at 0.0013 and would be
/// a single dim pixel of a ring. This is the display decision on top of the physical one.
pub const OPACITY_GAIN: f32 = 8.0;

/// What a shell is dimmed to when the ship is inside it.
///
/// Low, because "inside" spans a lot of ground. At one astronomical unit the Kuiper shell is
/// forty times further out than the ship and reads as an even wash; at Uranus it is barely
/// twice as far, lines of sight through it are oblique, and at a fade that suited the first
/// case it became a gray barrel filling the frame.
///
/// It is a weaker knob than it looks. The exposure meters the whole frame, so inside a shell
/// that fills the sky the meter follows this number and the displayed brightness barely moves:
/// halving it, measured, changed the Kuiper station by nothing at all and the belt by an eighth.
/// What it still does is set the band against the *stars*, which is the comparison that matters.
pub const INSIDE_FADE: f32 = 0.09;

/// Below this covering fraction a population is not drawn at all.
///
/// An Oort cloud covers about 4e-15 of its star's sky. It is photometrically invisible, which is
/// the fact 03-world-model.md builds the shell radius on, and drawing it would say the opposite
/// of what the model says.
pub const FAINTEST: f64 = 1.0e-13;

/// Covering fraction as an opacity.
///
/// A fourth root, and the exponent is the whole decision. The quantity spans fourteen decades —
/// an asteroid belt covers 2.6e-12 of its star's sky, a Kuiper analog 3e-8, a half-built swarm
/// 0.4 — so a linear mapping renders everything natural as exactly zero.
///
/// A logarithm was the first attempt and overcorrected badly: it put the Kuiper belt at 0.46,
/// and since the ship is *inside* that shell the result was a gray haze over the entire sky. The
/// compression has to leave the ordering intact without flattening it. A fourth root gives a
/// belt 0.001, a Kuiper belt 0.013 and a half-built swarm 0.80: a trace, a haze, and a
/// structure, which is the right reading of all three.
///
/// An envelope is a visualization either way. It is an orbit line, not a photograph.
pub fn opacity_of(covering: f64) -> f32 {
    if covering <= FAINTEST {
        return 0.0;
    }
    (covering.powf(0.25) as f32).clamp(0.0, 1.0)
}

/// How opaque a ring system is drawn. Rings are solid and reflective rather than a shadow, so
/// this is what they cover of their own annulus, not of a sphere.
pub fn ring_opacity(rings: &lc_world::rings::RingSystem) -> f32 {
    let annulus = std::f64::consts::PI * rings.outer_m().powi(2);
    opacity_of(rings.cross_section_m2() / annulus.max(f64::MIN_POSITIVE))
}

#[derive(Component)]
pub struct EnvelopeMesh(pub usize);

/// A ring system, which unlike a population is attached to a body and therefore moves.
#[derive(Component)]
pub struct RingMesh {
    /// The body's place in this frame's drawables.
    ///
    /// An index rather than the name it could be found by: `drawables_at` walks the arena in a
    /// fixed order and only ever drops a body, so while the list is the same length it holds
    /// the same bodies in the same places. [`Envelopes::ringed_bodies`] is what makes that
    /// true — a list that changed shape respawns these rather than moving them all by one.
    pub body: usize,
    /// Outer radius in render units.
    pub radius: f32,
}

/// One population, as something to draw.
pub struct Shell {
    pub mesh: Handle<Mesh>,
    pub material: Handle<PopulationMaterial>,
    /// Which of the system's populations this draws, so the gain can reach its material
    /// without rebuilding the filter that chose it.
    pub population: usize,
    /// Radius in render units, and the rotation taking `+Z` to the population's pole.
    pub radius: f32,
    pub orientation: Quat,
    /// Kept because the display gain reaches the material every frame and this does not change.
    pub field: Field,
}

/// Divisions of the proxy sphere.
///
/// Coarse on purpose. It is not the shape — the shape is [`Profile`] — and all it has to do is
/// cover the pixels the material projects to and be convex, so that dropping the face the ray
/// leaves through leaves exactly one fragment per pixel.
pub const PROXY_RINGS: usize = 16;
pub const PROXY_SEGMENTS: usize = 32;

/// Radius of the proxy, so its flat facets stay outside the sphere of material they bound.
///
/// `sec(pi / 2 RINGS)` would be exact; this is comfortably past it, and the excess is free
/// because the march clips analytically to a radius of one before it takes a single step.
const PROXY_MARGIN: f32 = 1.05;

/// The scalars the shader needs to read [`Profile`] — everything about the field that is not
/// the two rows themselves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Field {
    /// Inner edge of the material, with the outer edge at one.
    pub inner: f32,
    /// Sine of the widest inclination: the slab the material lies inside.
    pub slab: f32,
    /// Line integral of the density along a radial ray in the plane. See [`Profile`].
    pub reference: f32,
}

impl Default for Field {
    fn default() -> Self {
        Self { inner: 0.0, slab: 1.0, reference: 1.0 }
    }
}

/// A population's density field, as the shader reads it.
///
/// Two rows and three numbers. The field is separable — radius from the semi-major axis and
/// eccentricity distributions, latitude from the inclination distribution, and nothing depends
/// on longitude — so a row apiece is not a compression of it but the whole of it.
///
/// `reference` is the calibration. A radial ray outward from the star in the population's plane
/// is the sightline whose extinction *is* the covering fraction, which is what the photometry
/// in [`opacity_of`] measures; the shader solves for the extinction coefficient that puts that
/// one ray at `opacity` and every other sightline then follows from the geometry. This is what
/// replaces the old surface shader's area correction, and unlike it there is no factor left to
/// find by looking — a change to the shape cannot change what the population paints, because
/// the shape is what the calibration is solved against.
pub struct Profile {
    /// Density against latitude, sampled at `sin(phi) = slab * i / (N - 1)`, peak at one.
    pub latitude: Vec<f32>,
    /// Density against radius, sampled at `r = inner + (1 - inner) * i / (N - 1)`, peak at one.
    pub radial: Vec<f32>,
    pub field: Field,
}

/// Samples in the eccentric anomaly when one orbit's radial distribution is integrated.
///
/// In `E` rather than in `r`: a Kepler ellipse spends time `(1 - e cos E) dE` uniformly in `E`,
/// and the same distribution written in `r` has an inverse-square-root singularity at each apsis
/// that no amount of sampling in `r` handles.
const ANOMALY_STEPS: usize = 256;

/// Sub-samples across the cell each quadrature node stands for.
///
/// The distributions are quadrature, not a catalog. Nine semi-major axes and five
/// eccentricities are forty-five sharp annuli, and summed straight they read as concentric
/// rings that no belt has. Each node stands for a cell of the distribution it was drawn from,
/// so the profile is integrated across that cell. Twelve is where the residual ripple falls
/// under a per cent, measured on the generated system.
const CELL_SUBDIVISIONS: usize = 12;

/// The population's density field.
pub fn profile_of(population: &Population) -> Profile {
    let Some(extent) = population.extent() else {
        return Profile {
            latitude: vec![1.0; PROFILE_SAMPLES],
            radial: vec![1.0; PROFILE_SAMPLES],
            field: Field::default(),
        };
    };
    let inner = (extent.inner_m / extent.outer_m).clamp(0.0, 0.999) as f32;
    let slab = extent.half_angle_rad.sin().clamp(1.0e-4, 1.0) as f32;

    // Latitude, in units of the widest inclination, so a belt spends the whole row on the
    // twelve degrees it occupies rather than on the eighty-eight it does not.
    let latitude: Vec<f64> = (0..PROFILE_SAMPLES)
        .map(|i| {
            let sin_phi = slab as f64 * i as f64 / (PROFILE_SAMPLES - 1) as f64;
            population.inclination.sky_density(sin_phi.clamp(-1.0, 1.0).asin())
        })
        .collect();
    let latitude = peak_normalized(&latitude);

    let radial = radial_profile(population, extent.outer_m, inner as f64);
    let radial = peak_normalized(&radial);

    // The reference ray: outward from the star in the plane, where the latitude factor is its
    // own peak of one, so this integral is the radial row alone.
    let step = (1.0 - inner) / (PROFILE_SAMPLES - 1) as f32;
    let reference = radial.iter().sum::<f32>() * step;

    Profile { latitude, radial, field: Field { inner, slab, reference } }
}

fn peak_normalized(values: &[f64]) -> Vec<f32> {
    let peak = values.iter().cloned().fold(0.0f64, f64::max).max(f64::MIN_POSITIVE);
    values.iter().map(|v| (v / peak) as f32).collect()
}

/// Time-averaged number density against radius, as a histogram over the profile's samples.
///
/// The mass each orbit deposits, not the density it has at a sample point: an ellipse's radial
/// distribution is singular at both apsides, and a histogram in `E` integrates straight through
/// that where point sampling in `r` would spike on whichever bin the apsis landed in.
fn radial_profile(population: &Population, outer_m: f64, inner: f64) -> Vec<f64> {
    let axes = population.semi_major.nodes();
    let eccentricities = population.eccentricity.nodes();
    let (d_axis, d_ecc) = (node_spacing(axes), node_spacing(eccentricities));

    let width = (1.0 - inner) / PROFILE_SAMPLES as f64;
    let mut mass = vec![0.0f64; PROFILE_SAMPLES];
    let sub = (CELL_SUBDIVISIONS * CELL_SUBDIVISIONS * ANOMALY_STEPS) as f64;
    for &(axis, w_axis) in axes {
        for a in cell(axis, d_axis) {
            if a <= 0.0 {
                continue;
            }
            for &(ecc, w_ecc) in eccentricities {
                for e in cell(ecc, d_ecc) {
                    let e = e.clamp(0.0, 0.95);
                    for k in 0..ANOMALY_STEPS {
                        let anomaly =
                            std::f64::consts::TAU * (k as f64 + 0.5) / ANOMALY_STEPS as f64;
                        let spent = 1.0 - e * anomaly.cos();
                        let radius = a * spent / outer_m;
                        let bin = ((radius - inner) / width) as isize;
                        if bin >= 0 && (bin as usize) < PROFILE_SAMPLES {
                            mass[bin as usize] += w_axis * w_ecc * spent / sub;
                        }
                    }
                }
            }
        }
    }
    mass.iter().map(|m| m / width).collect()
}

/// Spacing of a distribution's nodes, which is the width of the cell each one stands for.
/// Zero for a single node, which is a delta and stands for nothing wider than itself.
fn node_spacing(nodes: &[(f64, f64)]) -> f64 {
    if nodes.len() < 2 {
        return 0.0;
    }
    let lo = nodes.iter().map(|(v, _)| *v).fold(f64::INFINITY, f64::min);
    let hi = nodes.iter().map(|(v, _)| *v).fold(f64::NEG_INFINITY, f64::max);
    (hi - lo) / (nodes.len() - 1) as f64
}

fn cell(center: f64, width: f64) -> Vec<f64> {
    if width <= 0.0 {
        return vec![center];
    }
    (0..CELL_SUBDIVISIONS)
        .map(|k| center - width * 0.5 + width * (k as f64 + 0.5) / CELL_SUBDIVISIONS as f64)
        .collect()
}

/// The profile as the texture the shader binds: latitude on row zero, radius on row one.
///
/// `R32Float` and unfilterable, like the starfield's band table and for the same reason — a
/// 32-bit float texture cannot be sampled with a filtering sampler under WebGPU, so the shader
/// interpolates it itself.
pub fn profile_image(profile: &Profile) -> Image {
    // Placed by the row constants rather than in the order they are written, so the one thing
    // the host and the shader have to agree about is stated once on each side.
    let mut rows = [[0.0f32; PROFILE_SAMPLES]; 2];
    rows[PROFILE_LATITUDE].copy_from_slice(&profile.latitude);
    rows[PROFILE_RADIAL].copy_from_slice(&profile.radial);
    let mut data = Vec::with_capacity(PROFILE_SAMPLES * rows.len() * 4);
    for row in rows {
        for value in row {
            data.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut image = Image::new(
        Extent3d { width: PROFILE_SAMPLES as u32, height: rows.len() as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// The proxy every population is drawn with: a sphere, coarse, slightly oversized.
///
/// One mesh for every population in the system, because it carries nothing about any of them.
/// What used to be a per-population torus — its cross-section from the eccentricity spread, its
/// normals hand-derived because the axes of an ellipse divide, a floor under its half-axes so a
/// degenerate spread did not collapse it into a line — is now three floats and two rows of a
/// texture, and none of those can be a polygon that misses.
pub fn build_proxy() -> Mesh {
    let count = (PROXY_RINGS + 1) * (PROXY_SEGMENTS + 1);
    let mut positions = Vec::with_capacity(count);
    let mut normals = Vec::with_capacity(count);
    let mut density = Vec::with_capacity(count);
    let mut indices = Vec::with_capacity(PROXY_RINGS * PROXY_SEGMENTS * 6);

    for ring in 0..=PROXY_RINGS {
        let polar = std::f32::consts::PI * ring as f32 / PROXY_RINGS as f32;
        let (sp, cp) = polar.sin_cos();
        for segment in 0..=PROXY_SEGMENTS {
            let theta = std::f32::consts::TAU * segment as f32 / PROXY_SEGMENTS as f32;
            let (st, ct) = theta.sin_cos();
            let out = Vec3::new(sp * ct, cp, sp * st);
            positions.push((out * PROXY_MARGIN).to_array());
            normals.push(out.to_array());
            // Rings only; a volume reads its density from the profile.
            density.push(1.0);
        }
    }

    let row = PROXY_SEGMENTS + 1;
    for ring in 0..PROXY_RINGS {
        for segment in 0..PROXY_SEGMENTS {
            let a = (ring * row + segment) as u32;
            let b = a + row as u32;
            indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(ATTRIBUTE_SHELL_DENSITY, density);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Radial divisions of a ring. Enough that the Cassini Division is a gap rather than a hint.
pub const RADIAL: usize = 96;

/// A flat annulus carrying each vertex's optical depth, normalized so the deepest band is one.
///
/// A ring is not a shell: it has radial structure and no latitude. Drawing it as a shell at one
/// radius would put Saturn's rings on a circle, and they span a factor of 1.8 in radius with a
/// division in the middle that is the most recognizable thing about them.
///
/// Unit scale is the outer edge, so the transform is one number.
pub fn build_ring(rings: &lc_world::rings::RingSystem) -> Mesh {
    let (inner, outer) = (rings.inner_m(), rings.outer_m());
    let peak = rings.bands.iter().map(|b| b.optical_depth).fold(0.0f64, f64::max);
    let peak = peak.max(f64::MIN_POSITIVE);

    let mut positions = Vec::with_capacity((RADIAL + 1) * (SEGMENTS + 1));
    let mut normals = Vec::with_capacity(positions.capacity());
    let mut density = Vec::with_capacity(positions.capacity());
    let mut indices = Vec::with_capacity(RADIAL * SEGMENTS * 6);

    let pole = sim_to_render(DVec3::Z).as_vec3().to_array();
    for r in 0..=RADIAL {
        let radius_m = inner + (outer - inner) * r as f64 / RADIAL as f64;
        // Sampled at the midpoint of the step, so a band edge does not fall exactly on a
        // vertex and vanish.
        let at = (rings.depth_at(radius_m + (outer - inner) * 0.5 / RADIAL as f64) / peak) as f32;
        let unit = radius_m / outer;
        for s in 0..=SEGMENTS {
            let theta = std::f64::consts::TAU * s as f64 / SEGMENTS as f64;
            let (st, ct) = theta.sin_cos();
            positions.push(sim_to_render(DVec3::new(unit * ct, unit * st, 0.0)).as_vec3().to_array());
            // Flat: every normal is the pole.
            normals.push(pole);
            density.push(at);
        }
    }

    let row = SEGMENTS + 1;
    for r in 0..RADIAL {
        for s in 0..SEGMENTS {
            let a = (r * row + s) as u32;
            let (b, c, d) = (a + 1, a + row as u32, a + row as u32 + 1);
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(ATTRIBUTE_SHELL_DENSITY, density);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// What the material scatters, as a fraction of what falls on it.
///
/// A single number where a body gets one per surface type, because a population is a
/// distribution and has no surface. Dark, which is what a rubble pile is; the exact value only
/// sets how much of the optical color is the star's rather than the material's own glow, and
/// the display level is fixed separately.
const ALBEDO: f64 = 0.1;

/// What the brightest display channel is drawn at when a sightline is completely full.
///
/// The old flat tint, kept as a level so the change is a change of *color* and not of
/// brightness. It has to be a display decision: a belt's real surface brightness is four
/// decades under a star's and renders as nothing at all in every normalized preset, which is
/// true photometrically and useless as a picture — the same argument [`opacity_of`] settles for
/// the opacity. An envelope is an orbit line, not a photograph.
const DISPLAY_LEVEL: f32 = 0.76;

/// The star a population is lit by, and the instrument looking at it.
#[derive(Clone, Copy)]
pub struct Lighting<'a> {
    pub star_teff_k: f64,
    pub star_radius_m: f64,
    pub star_luminosity_w: f64,
    pub mapping: &'a em_spectra::BandMapping,
}

/// What a fully-filled sightline through the population radiates, per band.
///
/// Two terms, and which one wins is the whole of why a belt looks different in different bands.
/// In the optical it is starlight the material scatters, so a belt is the color of its star. At
/// ten microns it is the material's own two-hundred-kelvin glow, which the star has none of, and
/// a belt goes from a barely-there haze to the brightest thing in the frame. Both scale with
/// `band_response`, which is emissivity and absorptivity at once — Kirchhoff, and the reason one
/// array can serve both this and the extinction.
///
/// The thermal term has no free constant in it: a full column of material at `T` radiates a
/// blackbody at `T`. Only the scattered term carries [`ALBEDO`].
pub fn source_radiance(population: &Population, lighting: Lighting) -> em_spectra::PerBand<f32> {
    let radius = population.thermal_radius();
    if radius <= 0.0 {
        return em_spectra::PerBand::splat(0.0);
    }
    // Surface brightness of the starlight arriving here: the star's own, diluted by how small
    // its disc is from this far out.
    let dilution = (lighting.star_radius_m / radius).powi(2);
    let equilibrium = population.equilibrium_temperature_under(lighting.star_luminosity_w);
    let (star, warm) = (crate::session::spectrum_at(lighting.star_teff_k), crate::session::spectrum_at(equilibrium));
    em_spectra::PerBand::new(std::array::from_fn(|i| {
        let band = em_spectra::Band::ALL[i];
        let scattered = ALBEDO * dilution * star[band] as f64;
        let glow = warm[band] as f64;
        (population.band_response[band] as f64 * (scattered + glow)) as f32
    }))
}

/// The per-band `(source, extinction)` pairs the shader reads, and the mapping's own columns.
///
/// The source is normalized so the brightest display channel lands at [`DISPLAY_LEVEL`]: the
/// bands set the *color* and the march sets the amount, and separating them is what keeps a
/// population legible in a preset its light barely reaches while still saying which preset it
/// is being seen in.
fn band_columns(
    population: &Population,
    lighting: Lighting,
    field: Field,
    opacity: f32,
    fade: f32,
) -> ([Vec4; em_spectra::BANDS], [Vec4; em_spectra::BANDS]) {
    let to_display = crate::starfield::band_columns(lighting.mapping);
    let source = source_radiance(population, lighting);

    let displayed = lighting.mapping.apply(&source);
    let peak = displayed.iter().cloned().fold(0.0f32, f32::max);
    let level = if peak > 0.0 { DISPLAY_LEVEL / peak } else { 0.0 };

    let covering = population.covering_fraction();
    let material = std::array::from_fn(|i| {
        let band = em_spectra::Band::ALL[i];
        // The opacity mapping again, on what this band actually meets. For a population of
        // solid bodies every band gets the same answer, which is right: a meter of rock is a
        // meter of rock from B to 21 cm. Dust is where it separates.
        let response = population.band_response[band].clamp(0.0, 1.0) as f64;
        let seen = (opacity_of(covering * response) * opacity / opacity_of(covering).max(f32::MIN_POSITIVE))
            .clamp(0.0, 1.0);
        // Solve for the coefficient that puts the reference ray at that opacity. Clamped short
        // of one, or a completed swarm asks for an infinite one.
        let wanted = (seen * fade).clamp(0.0, 0.98);
        let sigma = -(1.0 - wanted).ln() / field.reference.max(1e-6);
        Vec4::new(source[band] * level, sigma, 0.0, 0.0)
    });
    (to_display, material)
}

/// The uniforms for one population: what the photometry says is there.
pub fn uniforms(
    population: &Population,
    seed: f32,
    gain: f32,
    inside: bool,
    field: Field,
    lighting: Lighting,
) -> PopulationUniform {
    // No `tint`: the color is the per-band source run through the instrument's own mapping.
    // It used to be one of two hard-coded constants chosen by a reddening test on
    // `band_response`, which was both blind to the sensor and backwards — `extinction::RATIO`
    // is *largest* in B for dust, so the test that meant to catch dust never did.
    let opacity = (opacity_of(population.covering_fraction()) * gain).clamp(0.0, 1.0);
    let fade = if inside { INSIDE_FADE } else { 1.0 };
    let (band_to_display, band_material) =
        band_columns(population, lighting, field, opacity, fade);
    PopulationUniform {
        // Carried rather than assumed by the shader: the pole is +Z in simulation space, and
        // `sim_to_render` is the one place that knows what that is once it is rendered.
        pole: sim_to_render(DVec3::Z).as_vec3().extend(0.0),
        opacity,
        seed,
        inside_fade: fade,
        inner: field.inner,
        slab: field.slab,
        band_to_display,
        band_material,
        ..default()
    }
}

/// Is this population worth drawing at all?
pub fn visible(population: &Population) -> bool {
    opacity_of(population.covering_fraction()) > 0.0 && population.thermal_radius() > 0.0
}

/// Where a shell sits and how large, given where the ship is.
///
/// The camera never translates, so the shell is placed relative to it. One render unit is one
/// astronomical unit, which keeps a belt's radius a number of order ten.
pub fn transform(star_ly: DVec3, ship_ly: DVec3, population: &Population) -> Transform {
    let offset_m = (star_ly - ship_ly) * M_PER_LY;
    let radius = (population.thermal_radius() / UNIT_M) as f32;
    Transform {
        translation: sim_to_render(offset_m / UNIT_M).as_vec3(),
        rotation: orientation(population.pole),
        scale: Vec3::splat(radius),
    }
}

/// The rotation taking the mesh's `+Z` to the population's pole.
pub fn orientation(pole: DVec3) -> Quat {
    let to = sim_to_render(pole.normalize_or_zero()).as_vec3();
    if to.length_squared() <= 0.0 {
        return Quat::IDENTITY;
    }
    Quat::from_rotation_arc(sim_to_render(DVec3::Z).as_vec3(), to.normalize())
}

/// Spawn one entity per drawable population of the system the ship is in.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PopulationMaterial>,
    images: &mut Assets<Image>,
    grain: &Handle<Image>,
    populations: &[Population],
    gain: f32,
    lighting: Lighting,
) -> Vec<Shell> {
    // One proxy for all of them: it carries nothing about any population, so there is nothing
    // to build per population.
    let mesh = meshes.add(build_proxy());
    let mut shells = Vec::new();
    for (i, p) in populations.iter().enumerate().filter(|(_, p)| visible(p)) {
        let profile = profile_of(p);
        let material = materials.add(PopulationMaterial {
            uniforms: uniforms(p, i as f32 * 7.31 + 1.0, gain, true, profile.field, lighting),
            profile: images.add(profile_image(&profile)),
            grain: grain.clone(),
        });
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            // An Oort shell is a hundred thousand units across and the ship is inside it;
            // its bounds say nothing useful about whether it is on screen.
            NoFrustumCulling,
            // Marched at half resolution. See `haze`.
            bevy::camera::visibility::RenderLayers::layer(crate::haze::HAZE_LAYER),
            EnvelopeMesh(shells.len()),
        ));
        shells.push(Shell {
            mesh: mesh.clone(),
            material,
            population: i,
            // The field is normalized to the outer edge, so that is what scales it. The
            // thermal radius is where the light comes from, which is a different number
            // and is what the interface names the band by.
            radius: (p.extent().map(|e| e.outer_m).unwrap_or(0.0) / UNIT_M) as f32,
            orientation: orientation(p.pole),
            field: profile.field,
        });
    }
    shells
}

/// Spawn a ring for every body in the system that has one.
fn spawn_rings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PopulationMaterial>,
    images: &mut Assets<Image>,
    grain: &Handle<Image>,
    drawn: &[crate::system::Drawable],
    gain: f32,
) {
    // A ring takes the surface path and never reads the profile, but the bind group still
    // wants one. Flat, and shared by every ring in the system.
    let unread = images.add(profile_image(&Profile {
        latitude: vec![1.0; PROFILE_SAMPLES],
        radial: vec![1.0; PROFILE_SAMPLES],
        field: Field::default(),
    }));
    for (i, body) in drawn.iter().enumerate() {
        let Some(rings) = body.rings else { continue };
        // No display gain, unlike a population. The gain exists because even a Kuiper belt is
        // a trace and would otherwise be nothing; Saturn's rings cover a third of their own
        // annulus and need no help. Applying it here made Jupiter's rings -- three parts per
        // million, and it took Voyager to find them -- render at a fifth opacity.
        let _ = gain;
        let uniform = PopulationUniform {
            tint: Vec4::new(0.88, 0.84, 0.76, 1.0),
            opacity: ring_opacity(rings.system),
            seed: i as f32 * 3.77 + 0.5,
            // A ring is not a cloud: its texture is banding, not speckle.
            grain_frequency: 12.0,
            grain_strength: 0.35,
            // A sheet has no inside to march.
            volumetric: 0.0,
            ..default()
        };
        commands.spawn((
            Mesh3d(meshes.add(build_ring(rings.system))),
            MeshMaterial3d(
                materials.add(PopulationMaterial {
                    uniforms: uniform,
                    profile: unread.clone(),
                    grain: grain.clone(),
                }),
            ),
            Transform::default(),
            NoFrustumCulling,
            RingMesh {
                body: i,
                radius: (rings.system.outer_m() / UNIT_M) as f32,
            },
        ));
    }
}

/// The star and the instrument, as one population's shading needs them.
fn lighting_of<'a>(
    system: &lc_world::system::LocalSystem,
    session: &'a crate::app::Game,
) -> Lighting<'a> {
    Lighting {
        star_teff_k: system.star_teff_k(),
        star_radius_m: system.star_radius_m(),
        star_luminosity_w: system.star_luminosity_w(),
        // The same mapping the starfield binds, so a belt and the stars through it are seen by
        // one instrument rather than two.
        mapping: &session.mapping,
    }
}

/// The envelopes currently drawn, and which system they belong to.
#[derive(Resource, Default)]
pub struct Envelopes {
    pub star: Option<lc_world::sky::StarId>,
    pub shells: Vec<Shell>,
    /// How many drawables the ring entities' indices were taken against. See [`RingMesh::body`].
    pub ringed_bodies: usize,
}

/// Spawn the envelopes of the system the ship is in, and keep them placed.
pub fn update_envelopes(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    ui: Res<crate::app::Ui>,
    eye: Res<crate::hull::Eye>,
    bodies: Res<crate::starfield::Bodies>,
    mut envelopes: ResMut<Envelopes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PopulationMaterial>>,
    mut images: ResMut<Assets<Image>>,
    grain: Res<crate::procedural::PopulationGrain>,
    existing: Query<Entity, Or<(With<EnvelopeMesh>, With<RingMesh>)>>,
    mut placed: Query<(&mut Transform, &EnvelopeMesh), Without<RingMesh>>,
    mut ringed: Query<(&mut Transform, &RingMesh), Without<EnvelopeMesh>>,
) {
    let here = session.system.as_ref().map(|s| s.star);
    if envelopes.star != here || envelopes.ringed_bodies != bodies.drawn.len() {
        for entity in &existing {
            commands.entity(entity).despawn();
        }
        envelopes.star = here;
        envelopes.ringed_bodies = bodies.drawn.len();
        envelopes.shells = match session.system.as_ref() {
            Some(system) => {
                spawn_rings(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &mut images,
                    &grain.image,
                    &bodies.drawn,
                    ui.envelope_gain,
                );
                spawn(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &mut images,
                    &grain.image,
                    &system.populations,
                    ui.envelope_gain,
                    lighting_of(system, &session),
                )
            }
            None => Vec::new(),
        };
        // Nothing is placed until next frame, when the spawns exist.
        return;
    }

    let Some(system) = session.system.as_ref() else { return };
    for (mut transform, of) in placed.iter_mut() {
        let Some(shell) = envelopes.shells.get(of.0) else { continue };
        transform.translation =
            sim_to_render((system.origin_ly - eye.at_ly) * M_PER_LY / UNIT_M).as_vec3();
        transform.rotation = shell.orientation;
        transform.scale = Vec3::splat(shell.radius);
    }

    // Rings ride their body, so unlike a shell they are placed every frame.
    for (mut transform, ring) in ringed.iter_mut() {
        let Some(body) = bodies.drawn.get(ring.body) else { continue };
        let Some(rings) = body.rings else { continue };
        transform.translation =
            sim_to_render((body.position_ly - eye.at_ly) * M_PER_LY / UNIT_M).as_vec3();
        transform.rotation = orientation(rings.pole);
        transform.scale = Vec3::splat(ring.radius);
    }

    // The display gain is a knob, so it has to reach the material rather than only the spawn.
    for shell in &envelopes.shells {
        let Some(population) = system.populations.get(shell.population) else { continue };
        if let Some(mut material) = materials.get_mut(&shell.material) {
            let inside = eye.at_ly.distance(system.origin_ly) * M_PER_LY
                < population.thermal_radius();
            let next = uniforms(
                population,
                material.uniforms.seed,
                ui.envelope_gain,
                inside,
                shell.field,
                lighting_of(system, &session),
            );
            if material.uniforms != next {
                material.uniforms = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use lc_world::distribution::{Distribution, Inclination};

    use super::*;

    fn population(inclination: Inclination, count: f64) -> Population {
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::normal(3.0e11, 2.0e10, 9),
            eccentricity: Distribution::uniform(0.0, 0.05, 5),
            inclination,
            count,
            cross_section: 3.0e6,
            band_response: em_spectra::PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        }
    }

    /// A sun-like star, and whichever instrument is being asked about.
    fn sunlike(mapping: &em_spectra::BandMapping) -> Lighting<'_> {
        let star = lc_world::star::Star::SOL;
        Lighting {
            star_teff_k: star.teff_k,
            star_radius_m: star.radius_m,
            star_luminosity_w: star.luminosity(),
            mapping,
        }
    }

    /// A dusty population, by the one thing that makes a population dusty: it interacts far
    /// less at long wavelengths. `extinction::RATIO` is a factor of 1.3e10 from B to 21 cm.
    fn dusty(base: Population) -> Population {
        let mut response = em_spectra::PerBand::splat(0.0f32);
        for b in em_spectra::Band::ALL {
            response[b] = em_spectra::extinction::RATIO[b] as f32;
        }
        Population { band_response: response, ..base }
    }

    fn positions(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            bevy_mesh::VertexAttributeValues::Float32x3(v) => v.clone(),
            _ => panic!("positions must be Float32x3"),
        }
    }

    fn normals(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() {
            bevy_mesh::VertexAttributeValues::Float32x3(v) => v.clone(),
            _ => panic!("normals must be Float32x3"),
        }
    }

    fn densities(mesh: &Mesh) -> Vec<f32> {
        match mesh.attribute(ATTRIBUTE_SHELL_DENSITY).unwrap() {
            bevy_mesh::VertexAttributeValues::Float32(v) => v.clone(),
            _ => panic!("density must be Float32"),
        }
    }

    /// Walk a ray through the field the shader marches, and return its optical depth in units
    /// of the extinction coefficient. The shader does this on the GPU; this is the same
    /// integral, so what it measures is the field rather than the rendering.
    fn depth_along(profile: &Profile, from: DVec3, direction: DVec3, steps: usize) -> f64 {
        let direction = direction.normalize();
        // Far enough to leave the unit sphere from anywhere inside it.
        let span = 4.0;
        let step = span / steps as f64;
        let mut total = 0.0;
        for k in 0..steps {
            let at = from + direction * ((k as f64 + 0.5) * step);
            total += sample(profile, at) * step;
        }
        total
    }

    /// The field at a point, without the grain: radius times latitude, peak at one.
    fn sample(profile: &Profile, at: DVec3) -> f64 {
        let radius = at.length();
        let field = profile.field;
        if radius > 1.0 || radius < field.inner as f64 {
            return 0.0;
        }
        let read = |row: &[f32], u: f64| -> f64 {
            let x = u.clamp(0.0, 1.0) * (PROFILE_SAMPLES - 1) as f64;
            let i = x.floor() as usize;
            let j = (i + 1).min(PROFILE_SAMPLES - 1);
            let f = x - i as f64;
            row[i] as f64 * (1.0 - f) + row[j] as f64 * f
        };
        let radial = read(&profile.radial, (radius - field.inner as f64) / (1.0 - field.inner as f64));
        // The pole is +Z in simulation space, and the field is built in those terms.
        let sin_phi = (at.z / radius).abs();
        let latitude = read(&profile.latitude, sin_phi / field.slab as f64);
        radial * latitude
    }

    /// The thing a painted surface could not say, and the reason for the change.
    ///
    /// From inside a belt, looking along it passes through far more material than looking at
    /// the pole. The tube shader put those within four per cent of each other — every sightline
    /// met the far wall exactly once and nearly face-on, so the crossing count and the
    /// incidence angle, which were the only two things it had, were the same for both.
    #[test]
    fn a_long_way_through_a_belt_is_a_long_way_through_a_belt() {
        let belt = population(Inclination::uniform_angle(0.0, 0.2, 12), 1e6);
        let profile = profile_of(&belt);
        let mid = ((profile.field.inner as f64) + 1.0) * 0.5;
        let from = DVec3::new(mid, 0.0, 0.0);

        let along = depth_along(&profile, from, DVec3::Y, 4000);
        let pole = depth_along(&profile, from, DVec3::Z, 4000);
        assert!(
            along > pole * 5.0,
            "along the belt against out of its plane: {along:.4} and {pole:.4}",
        );

        // And it is graded rather than a step: tipping the sightline out of the plane takes it
        // down monotonically.
        let mut last = f64::INFINITY;
        for tenths in 0..=6 {
            let angle = 0.06 * tenths as f64;
            let at = depth_along(&profile, from, DVec3::new(0.0, angle.cos(), angle.sin()), 4000);
            assert!(at <= last + 1e-9, "not monotonic at {angle}: {at} after {last}");
            last = at;
        }
    }

    /// One field for both, and it is the *profile* that says which. A narrow inclination spread
    /// leaves the material in a band near the plane; an isotropic one fills the sphere.
    ///
    /// The torus this replaces could not say the second one. Its tube was as tall as it was
    /// wide for an isotropic population, which sounds right and is not: swept about the pole
    /// that is an apple core, and a swarm the model calls isotropic had nothing at all above
    /// forty-five degrees of latitude.
    #[test]
    fn a_belt_is_flat_and_a_swarm_is_round() {
        let flat = profile_of(&population(Inclination::uniform_angle(0.0, 0.2, 12), 1e6));
        let round = profile_of(&population(Inclination::isotropic(), 1e6));

        assert!(flat.field.slab < 0.25, "a belt is thin: {}", flat.field.slab);
        assert!((round.field.slab - 1.0).abs() < 1e-6, "a cloud is not: {}", round.field.slab);

        // The belt's density has run out well before the pole; the cloud's never does.
        assert!(flat.latitude.last().unwrap() < &0.05, "{:?}", flat.latitude.last());
        assert!(round.latitude.iter().all(|v| (*v - 1.0).abs() < 1e-6), "isotropic is flat");

        // Which the field then delivers: from the center, every direction out of a cloud meets
        // the same material, and that is exactly what the apple core could not do.
        let center = DVec3::ZERO;
        let plane = depth_along(&round, center, DVec3::X, 4000);
        let pole = depth_along(&round, center, DVec3::Z, 4000);
        assert!(plane > 0.0 && (plane / pole - 1.0).abs() < 0.02, "{plane:.4} and {pole:.4}");
    }

    /// The invariant a change of shape has to keep: the same population paints the same light.
    ///
    /// The photometry is in `covering_fraction` and did not change. What the shader solves for
    /// is the extinction coefficient that puts the reference ray — radially outward, in the
    /// plane — at exactly the mapped opacity, so this checks that the reference integral the
    /// host computes is the one that ray actually has. Measured by walking the field rather
    /// than by re-running the sum that built it, so it is a check and not a restatement.
    #[test]
    fn the_reference_ray_is_the_one_the_calibration_names() {
        for inclination in [Inclination::uniform_angle(0.0, 0.2, 12), Inclination::isotropic()] {
            let profile = profile_of(&population(inclination, 1e6));
            let walked = depth_along(&profile, DVec3::ZERO, DVec3::X, 20000);
            let claimed = profile.field.reference as f64;
            assert!(
                (walked / claimed - 1.0).abs() < 0.02,
                "walked {walked:.5} against a claimed {claimed:.5}",
            );
            assert!(claimed > 0.0, "a population with anything in it has a reference ray");
        }
    }

    /// The thing the sphere could not say at all: a belt reaches from an inner radius to an
    /// outer one, and the field is empty either side of them.
    #[test]
    fn the_field_spans_the_radii_the_population_occupies() {
        let belt = population(Inclination::uniform_angle(0.0, 0.2, 12), 1e6);
        let extent = belt.extent().expect("an extent");
        let inner = (extent.inner_m / extent.outer_m) as f32;
        let profile = profile_of(&belt);
        assert!((profile.field.inner - inner).abs() < 1e-4, "{} against {inner}", profile.field.inner);

        assert_eq!(sample(&profile, DVec3::X * 1.01), 0.0, "nothing past the outer edge");
        assert_eq!(sample(&profile, DVec3::X * (inner as f64 * 0.99)), 0.0, "nor inside the hole");
        let mid = ((inner as f64) + 1.0) * 0.5;
        assert!(sample(&profile, DVec3::X * mid) > 0.1, "and something in between");
    }

    /// The nodes are quadrature, not a catalog. Nine semi-major axes summed straight paint
    /// nine annuli, and a belt made of concentric rings is a rendering of the integration
    /// scheme rather than of a belt.
    #[test]
    fn the_radial_profile_is_smooth_enough_not_to_read_as_rings() {
        let profile = profile_of(&population(Inclination::uniform_angle(0.0, 0.2, 12), 1e6));
        let row = &profile.radial;
        let ripple: f32 = (1..row.len() - 1)
            .map(|i| (row[i - 1] - 2.0 * row[i] + row[i + 1]).abs())
            .sum::<f32>()
            / (row.len() - 2) as f32;
        assert!(ripple < 0.01, "the radial profile is lumpy: {ripple}");
        assert!(row.iter().cloned().fold(0.0f32, f32::max) > 0.99, "peak-normalized");
    }

    /// The thing the sensor presets are named for. A meter of rock is a meter of rock from B to
    /// 21 cm, so a belt of solid bodies is equally opaque in every band; dust is not, and at
    /// 21 cm a sightline through it finds almost nothing there.
    ///
    /// Before this, `band_response` reached the renderer only as a choice between two hard-coded
    /// tints, so "dust penetration" penetrated nothing and every preset drew the same gray.
    #[test]
    fn a_band_the_material_barely_meets_is_a_band_it_barely_blocks() {
        let mapping = em_spectra::presets::natural();
        let field = Field { inner: 0.3, slab: 0.2, reference: 0.3 };
        let extinction = |p: &Population| {
            let u = uniforms(p, 0.0, OPACITY_GAIN, false, field, sunlike(&mapping));
            em_spectra::Band::ALL.map(|b| u.band_material[b.index()].y)
        };

        let rock = extinction(&population(Inclination::uniform_angle(0.0, 0.2, 12), 1e9));
        let first = rock[0];
        assert!(first > 0.0, "a belt blocks something: {first}");
        assert!(
            rock.iter().all(|s| (s / first - 1.0).abs() < 1.0e-6),
            "solid bodies are gray across the bands: {rock:?}",
        );

        let dust = extinction(&dusty(population(Inclination::uniform_angle(0.0, 0.2, 12), 1e9)));
        let (blue, radio) = (dust[em_spectra::Band::B.index()], dust[em_spectra::Band::Radio.index()]);
        assert!(blue > 0.0, "dust is opaque in the blue: {blue}");
        assert!(radio < blue * 0.05, "and all but transparent at 21 cm: {radio} against {blue}");
        // Monotonic the whole way out, which is what makes stepping through the sensors read as
        // one cloud thinning rather than as six unrelated pictures.
        for pair in dust.windows(2) {
            assert!(pair[1] <= pair[0] + 1.0e-9, "not monotonic across the bands: {dust:?}");
        }
    }

    /// And the color follows the instrument. A belt shines by scattered starlight in the
    /// optical and by its own two-hundred-kelvin glow at ten microns, so the preset that looks
    /// at ten microns finds something the natural one cannot.
    #[test]
    fn a_warm_belt_reads_as_heat_only_in_the_band_that_can_see_heat() {
        let belt = population(Inclination::uniform_angle(0.0, 0.2, 12), 1e9);
        let field = profile_of(&belt).field;
        // What the shader adds up for a sightline that is completely full: every band's source
        // through that band's column of the mapping.
        let displayed = |mapping: &em_spectra::BandMapping| {
            let u = uniforms(&belt, 0.0, OPACITY_GAIN, false, field, sunlike(mapping));
            let channel = |c: usize| {
                em_spectra::Band::ALL
                    .iter()
                    .map(|b| u.band_to_display[b.index()][c] * u.band_material[b.index()].x)
                    .sum::<f32>()
            };
            [channel(0), channel(1), channel(2)]
        };

        // Natural is scattered starlight across three neighboring optical bands, so a belt
        // comes out near the color of its star rather than any color of its own.
        let natural = displayed(&em_spectra::presets::natural());
        let (hi, lo) = (
            natural.iter().cloned().fold(0.0f32, f32::max),
            natural.iter().cloned().fold(f32::MAX, f32::min),
        );
        assert!(hi / lo < 2.0, "a sun-lit belt is not strongly colored in the optical: {natural:?}");

        // Thermal puts ten microns in red, and a two-hundred-kelvin belt against a sun-like
        // reference has nothing anywhere else.
        let thermal = displayed(&em_spectra::presets::thermal());
        assert!(
            thermal[0] > thermal[1] * 10.0 && thermal[0] > thermal[2] * 10.0,
            "a warm belt should read as heat: {thermal:?}",
        );

        // Whatever the preset, the brightest channel lands at the display level, so a belt is
        // legible in a band its light barely reaches and still says which band that is.
        for (name, mapping) in em_spectra::presets::all() {
            let peak = displayed(&mapping).iter().cloned().fold(0.0f32, f32::max);
            assert!((peak - DISPLAY_LEVEL).abs() < 1.0e-3, "{name} peaks at {peak}");
        }
    }

    /// The mapping is a fourth root because the quantity spans fourteen decades. A logarithm
    /// overcorrected -- it put the Kuiper belt at 0.46, and since the ship is inside that shell
    /// the sky became a gray wash.
    #[test]
    fn the_opacity_mapping_keeps_three_populations_apart() {
        let belt = opacity_of(2.6e-12);
        let kuiper = opacity_of(3.0e-8);
        let swarm = opacity_of(0.4);
        assert!(belt < kuiper && kuiper < swarm, "{belt} {kuiper} {swarm}");
        assert!(belt > 0.0 && belt < 0.01, "a belt is a trace: {belt}");
        assert!(kuiper > 0.005 && kuiper < 0.05, "a Kuiper belt is a haze: {kuiper}");
        assert!(swarm > 0.5, "a swarm is a structure: {swarm}");
    }

    #[test]
    fn an_oort_cloud_is_not_drawn() {
        // Four parts in 1e15, and 03-world-model.md builds the shell radius on it being
        // invisible. Drawing it would say the opposite of what the model says.
        assert_eq!(opacity_of(4.4e-15), 0.0);
        assert!(!visible(&population(Inclination::isotropic(), 1.0)));
    }

    #[test]
    fn a_shell_is_placed_where_the_star_is_and_scaled_to_its_orbit() {
        let p = population(Inclination::uniform_angle(0.0, 0.2, 12), 1e6);
        let star = DVec3::new(1.0, 2.0, -0.5);
        let at_star = transform(star, star, &p);
        assert!(at_star.translation.length() < 1e-6, "the ship at the star sees it centered");
        assert!((at_star.scale.x - (p.thermal_radius() / UNIT_M) as f32).abs() < 1e-3);

        let away = transform(star, star + DVec3::X * 1e-4, &p);
        assert!(away.translation.length() > 0.0, "and moving the ship moves the shell");
    }

    #[test]
    fn the_pole_orients_the_shell() {
        let flat = orientation(DVec3::Z);
        let tipped = orientation(DVec3::X);
        assert!(flat.is_finite() && tipped.is_finite());
        assert!(flat.angle_between(tipped) > 1.0, "a different pole is a different orientation");
        assert_eq!(orientation(DVec3::ZERO), Quat::IDENTITY);
    }

    #[test]
    fn a_shell_the_ship_is_inside_is_dimmed() {
        let p = population(Inclination::uniform_angle(0.0, 0.2, 12), 1e9);
        let field = profile_of(&p).field;
        let mapping = em_spectra::presets::natural();
        let out = uniforms(&p, 0.0, OPACITY_GAIN, false, field, sunlike(&mapping));
        let inside = uniforms(&p, 0.0, OPACITY_GAIN, true, field, sunlike(&mapping));
        assert_eq!(out.inside_fade, 1.0);
        assert!(inside.inside_fade < 0.5, "inside, a shell covers the whole sky");
        assert_eq!(out.opacity, inside.opacity, "only the fade differs, not the physics");
    }

    #[test]
    fn a_ring_mesh_has_the_gaps_the_data_says_it_has() {
        let saturn = lc_world::rings::for_body("Saturn").unwrap();
        let mesh = build_ring(saturn);
        let d = densities(&mesh);
        assert!(d.iter().any(|v| *v > 0.99), "the B ring is the peak");
        assert!(d.iter().any(|v| *v < 0.1 && *v > 0.0), "and the division is thin, not empty");

        // Walking outward, the profile must rise, fall into the division and rise again.
        let row = SEGMENTS + 1;
        let profile: Vec<f32> = (0..=RADIAL).map(|r| d[r * row]).collect();
        let peak = profile.iter().cloned().fold(0.0f32, f32::max);
        let peak_at = profile.iter().position(|v| *v == peak).unwrap();
        let after = &profile[peak_at..];
        let dip = after.iter().cloned().fold(f32::MAX, f32::min);
        assert!(dip < peak / 5.0, "there should be a division past the B ring: {dip} vs {peak}");
        assert!(after.iter().rev().take(10).any(|v| *v > dip * 3.0), "and an A ring past that");
    }

    #[test]
    fn a_ring_is_flat_and_the_proxy_is_not() {
        let saturn = lc_world::rings::for_body("Saturn").unwrap();
        let ring = normals(&build_ring(saturn));
        let proxy = normals(&build_proxy());
        let first = ring[0];
        assert!(ring.iter().all(|n| *n == first), "every ring normal is the pole");
        assert!(proxy.iter().any(|n| *n != proxy[0]), "a sphere's normals point everywhere");
    }

    /// The proxy has to contain the material it stands for, or the march starts inside the
    /// field and the near edge of a population is a polygon.
    #[test]
    fn the_proxy_encloses_the_unit_sphere() {
        let closest = positions(&build_proxy())
            .iter()
            .map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt())
            .fold(f32::MAX, f32::min);
        assert!(closest > 1.0, "a facet cuts inside the material: {closest}");
    }

    /// The four ring systems, in the order a person would rank them by eye.
    #[test]
    fn the_ring_systems_come_out_in_the_right_order() {
        let drawn = |id: &str| ring_opacity(lc_world::rings::for_body(id).unwrap());
        let (saturn, uranus, neptune, jupiter) =
            (drawn("Saturn"), drawn("Uranus"), drawn("Neptune"), drawn("Jupiter"));
        assert!(saturn > 0.5, "Saturn's rings are the thing you see: {saturn}");
        assert!(uranus < saturn * 0.5 && uranus > 0.05, "Uranus's are faint but real: {uranus}");
        assert!(neptune < uranus, "Neptune's fainter still: {neptune}");
        assert!(jupiter < 0.05, "and Jupiter's took Voyager to find: {jupiter}");
    }
}
