//! The sky as geometry: one quad per star, fed by [`crate::session::Session`].
//!
//! The material and the shader live in `em-render`. This is the half that cannot be shared —
//! building the mesh, generating the band table, and keeping the uniforms current.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::relativistic_starfield_material::{
    ATTRIBUTE_STAR_CORNER, ATTRIBUTE_STAR_PARAMS, ATTRIBUTE_STAR_WARM, BANDS,
    RelativisticStarfieldMaterial, RelativisticStarfieldUniform,
};
use em_render::render_space::sim_to_render;
use em_spectra::{Band, BandMapping, blackbody};
use glam::DVec3;
use lc_world::sky::{CatalogueStar, generate};

use crate::session::{POINT_STOPS, Session};

/// Temperature samples in the band table, over [`LOG_T_MIN`] to [`LOG_T_MAX`].
pub const LUT_SAMPLES: usize = 2048;

/// The table's domain, as `log2` of kelvin: 16 K to about 4 million K.
///
/// Far wider than any star, because the shader looks the table up at the *Doppler-shifted*
/// temperature. At the drive's 0.999c cap the shift factor is 44.7 either way, so a 2000 K red
/// dwarf astern arrives at 45 K and a 50 000 K O star ahead arrives at 2.2 million. Both ends
/// have to be in the table or the sky clips to a flat colour at speed.
pub const LOG_T_MIN: f32 = 4.0;
pub const LOG_T_MAX: f32 = 22.0;

/// How far the ship travels before the mesh is rebuilt about its new position, light-years.
///
/// Star positions are baked relative to a bake origin and the ship's offset from that origin
/// is a uniform, so the shader differences two small numbers instead of two interstellar ones.
/// The offset only has to stay small enough that `f32` resolves it; a light-year is three
/// orders inside that, and at the rates a crossing runs it means a rebuild every few seconds.
pub const REBAKE_LY: f64 = 1.0;

/// Quad corners, counter-clockwise.
const QUAD: [[f32; 2]; 4] = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];

/// Where the current mesh was baked, and what is drawing it.
#[derive(Resource)]
pub struct Starfield {
    pub origin_ly: DVec3,
    pub mesh: Handle<Mesh>,
    pub material: Handle<RelativisticStarfieldMaterial>,
}

#[derive(Component)]
pub struct SkyMesh;

/// `log2` of band radiance against temperature: one column per sample, one row per band.
///
/// Generated from `em_spectra::blackbody`, which is the integral the telescope samples. The
/// starfield and the instrument agreeing is not a convention to maintain — there is one
/// Planck integral and this is a tabulation of it.
///
/// Stored as `log2` because the raw value moves by decades between adjacent samples on the
/// Wien side, where a linear blend of the numbers themselves would be wrong by most of the
/// interval. `R32Float` because a 32-bit float texture is not filterable under WebGPU, so the
/// shader interpolates it itself.
pub fn band_lut() -> Image {
    let mut data: Vec<u8> = Vec::with_capacity(LUT_SAMPLES * BANDS * 4);
    for band in Band::ALL {
        for i in 0..LUT_SAMPLES {
            let log_t = LOG_T_MIN + (LOG_T_MAX - LOG_T_MIN) * i as f32 / (LUT_SAMPLES - 1) as f32;
            let radiance = blackbody::band_radiance(band, 2f64.powf(log_t as f64));
            // Far enough below anything representable to read as dark, and finite.
            let value = if radiance > 0.0 { radiance.log2() as f32 } else { -300.0 };
            data.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut image = Image::new(
        Extent3d { width: LUT_SAMPLES as u32, height: BANDS as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::R32Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// Four vertices per star, positions relative to `origin_ly`, in render axes.
pub fn build_mesh(stars: &[CatalogueStar], origin_ly: DVec3) -> Mesh {
    let n = stars.len();
    let mut positions = Vec::with_capacity(n * 4);
    let mut corners = Vec::with_capacity(n * 4);
    let mut params = Vec::with_capacity(n * 4);
    let mut warm = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);

    for star in stars {
        // Differenced in f64 and narrowed after, which is the whole point of the bake origin.
        let at = sim_to_render(star.position_ly - origin_ly).as_vec3().to_array();
        let physics = [star.star.teff_k as f32, star.star.radius_m as f32, 0.0, 0.0];
        let heat = warm_params(star);
        let base = positions.len() as u32;
        for corner in QUAD {
            positions.push(at);
            corners.push(corner);
            params.push(physics);
            warm.push(heat);
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(ATTRIBUTE_STAR_CORNER, corners);
    mesh.insert_attribute(ATTRIBUTE_STAR_PARAMS, params);
    mesh.insert_attribute(ATTRIBUTE_STAR_WARM, warm);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// A star's swarm, as the shader wants it: temperature, radiance against the stellar disc, and
/// the grey deficit it removes. Zeros where there is nothing.
///
/// Drawn from the same generator the telescope reads, so a star that measures as engineered
/// also looks engineered. The swarm is separable from the rest of the system on purpose: this
/// runs for every star in the sky, and generating a full planetary system for each would not.
pub fn warm_params(star: &CatalogueStar) -> [f32; 4] {
    let Some(swarm) = generate::swarm_for(star) else { return [0.0; 4] };
    let r = star.star.radius_m;
    [
        swarm.equilibrium_temperature(&star.star) as f32,
        // The same ratio `reradiated_radiance` applies, and for the same reason: expressed
        // against the stellar disc, the shader's existing geometry term carries it.
        (swarm.radiating_area() / (4.0 * std::f64::consts::PI * r * r)) as f32,
        // Isotropic, so the deficit along any line of sight is the absorbed fraction.
        swarm.absorbed_fraction() as f32,
        0.0,
    ]
}

/// The uniforms that change: where the ship is, how fast, and how it is looking.
pub fn uniforms(session: &Session, origin_ly: DVec3, lut_scale: f32) -> RelativisticStarfieldUniform {
    let mapping = &session.mapping;
    RelativisticStarfieldUniform {
        band_to_display: band_columns(mapping),
        beta: sim_to_render(session.beta).as_vec3().extend(0.0),
        ship_offset_ly: sim_to_render(session.position_ly - origin_ly).as_vec3().extend(0.0),
        reference: session.tone.reference,
        point_stops: POINT_STOPS,
        log_t_min: LOG_T_MIN,
        log_t_scale: lut_scale,
        lut_samples: LUT_SAMPLES as f32,
        ..default()
    }
}

/// The mapping by column: entry `b` is band `b`'s contribution to red, green and blue.
///
/// Unavailable bands are zeroed here rather than in the shader, matching `BandMapping::apply`:
/// an instrument that cannot sense a band contributes nothing through it.
fn band_columns(mapping: &BandMapping) -> [Vec4; BANDS] {
    std::array::from_fn(|b| {
        let band = Band::ALL[b];
        if !mapping.available.contains(band) {
            return Vec4::ZERO;
        }
        Vec4::new(mapping.matrix[0][b], mapping.matrix[1][b], mapping.matrix[2][b], 0.0)
    })
}

/// Samples of the table per stop of temperature.
pub fn lut_scale() -> f32 {
    (LUT_SAMPLES - 1) as f32 / (LOG_T_MAX - LOG_T_MIN)
}

pub fn spawn_sky(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RelativisticStarfieldMaterial>>,
    mut images: ResMut<Assets<Image>>,
    existing: Query<Entity, With<SkyMesh>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let origin_ly = session.position_ly;
    let mesh = meshes.add(build_mesh(&session.stars, origin_ly));
    let material = materials.add(RelativisticStarfieldMaterial {
        uniforms: uniforms(&session.0, origin_ly, lut_scale()),
        band_lut: images.add(band_lut()),
    });
    commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        // Positions are light-year offsets that the shader never uses for placement, so the
        // mesh's bounds say nothing about where it lands on screen.
        NoFrustumCulling,
        SkyMesh,
    ));
    commands.insert_resource(Starfield { origin_ly, mesh, material });
}

/// Push this frame's uniforms, and re-bake if the ship has outrun the origin.
pub fn update_sky(
    session: Res<crate::app::Game>,
    mut sky: ResMut<Starfield>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RelativisticStarfieldMaterial>>,
) {
    if session.position_ly.distance(sky.origin_ly) > REBAKE_LY {
        sky.origin_ly = session.position_ly;
        if let Some(mesh) = meshes.get_mut(&sky.mesh) {
            *mesh = build_mesh(&session.stars, sky.origin_ly);
        }
    }
    if let Some(material) = materials.get_mut(&sky.material) {
        material.uniforms = uniforms(&session.0, sky.origin_ly, lut_scale());
    }
}

#[cfg(test)]
mod tests {
    use em_spectra::presets;
    use lc_world::sky::{AuthoredStars, StarProvider};

    use super::*;

    /// The shader's table read, mirrored on the CPU so the tabulation can be checked against
    /// the function it tabulates.
    fn lookup(lut: &Image, band: Band, teff: f64) -> f64 {
        let last = (LUT_SAMPLES - 1) as f32;
        let at = ((teff.log2() as f32 - LOG_T_MIN) * lut_scale()).clamp(0.0, last);
        let (i, j) = (at.floor() as usize, (at.floor() as usize + 1).min(LUT_SAMPLES - 1));
        let row = band.index() * LUT_SAMPLES;
        let texel = |k: usize| {
            let at = (row + k) * 4;
            f32::from_le_bytes(lut.data.as_ref().unwrap()[at..at + 4].try_into().unwrap())
        };
        let frac = at - at.floor();
        let log2 = texel(i) + (texel(j) - texel(i)) * frac;
        2f64.powf(log2 as f64)
    }

    #[test]
    fn the_table_is_the_shape_the_shader_indexes() {
        let lut = band_lut();
        assert_eq!(lut.width(), LUT_SAMPLES as u32);
        assert_eq!(lut.height(), BANDS as u32);
        assert_eq!(lut.data.as_ref().unwrap().len(), LUT_SAMPLES * BANDS * 4);
    }

    /// The claim the whole design rests on: the starfield and the telescope are one
    /// implementation. The shader reads a table; the table has to be the function.
    #[test]
    fn the_table_reproduces_the_planck_integral_the_telescope_samples() {
        let lut = band_lut();
        let mut worst: f64 = 0.0;
        for band in Band::ALL {
            for k in 0..=120 {
                let span = (LOG_T_MAX - LOG_T_MIN) as f64;
                let teff = 2f64.powf(LOG_T_MIN as f64 + span * k as f64 / 120.0);
                let want = blackbody::band_radiance(band, teff);
                if want <= 0.0 || !want.is_finite() {
                    continue;
                }
                let got = lookup(&lut, band, teff);
                worst = worst.max((got - want).abs() / want);
            }
        }
        assert!(worst < 0.005, "table is off the integral by {:.3}%", worst * 100.0);
    }

    /// Why the table holds log2 rather than the radiance: on the Wien side the value moves by
    /// decades between samples, and blending the raw numbers is wrong by most of the interval.
    #[test]
    fn interpolating_the_raw_radiance_would_be_far_worse() {
        let lut = band_lut();
        // A cool star in B: 3000 K is deep on the Wien side of a 445 nm band, which is where
        // the curve is steepest and where any interpolation is hardest.
        let band = Band::B;
        let step = 1.0 / lut_scale() as f64;
        let below = ((3000f64.log2() - LOG_T_MIN as f64) / step).floor();
        let teff = 2f64.powf(LOG_T_MIN as f64 + (below + 0.5) * step);
        let want = blackbody::band_radiance(band, teff);
        assert!(want > 0.0, "the comparison needs a band that is actually emitting");

        let lo = blackbody::band_radiance(band, 2f64.powf(LOG_T_MIN as f64 + below * step));
        let hi = blackbody::band_radiance(band, 2f64.powf(LOG_T_MIN as f64 + (below + 1.0) * step));
        let naive = 0.5 * (lo + hi);

        let ours = (lookup(&lut, band, teff) - want).abs() / want;
        let theirs = (naive - want).abs() / want;
        assert!(ours < theirs, "log2 {ours} should beat linear {theirs}");
    }

    /// The table has to cover the shifted temperature, not the emitted one.
    #[test]
    fn the_table_spans_what_a_relativistic_shift_can_reach() {
        let max_beta = crate::flight::Drive::DEFAULT.max_beta;
        let gamma = (1.0 - max_beta * max_beta).sqrt().recip();
        let toward = gamma * (1.0 + max_beta);
        let coolest = 2000.0 / toward;
        let hottest = 50_000.0 * toward;
        assert!(2f64.powf(LOG_T_MIN as f64) < coolest, "{coolest} K falls off the cold end");
        assert!(2f64.powf(LOG_T_MAX as f64) > hottest, "{hottest} K falls off the hot end");
    }

    /// The columns the shader multiplies must reproduce what `BandMapping::apply` computes,
    /// including the masking: an unavailable band is not dark, it contributes nothing.
    #[test]
    fn the_band_columns_reproduce_the_mapping() {
        for (_, mapping) in presets::all() {
            let radiance = em_spectra::PerBand::new(std::array::from_fn(|i| 1.0 + i as f32));
            let want = mapping.apply(&radiance);
            let columns = band_columns(&mapping);
            let mut got = Vec3::ZERO;
            for b in 0..BANDS {
                got += columns[b].truncate() * radiance[Band::ALL[b]];
            }
            assert!((got - Vec3::from_array(want)).length() < 1e-5, "{got:?} vs {want:?}");
        }
    }

    #[test]
    fn an_unavailable_band_contributes_nothing() {
        let mut mapping = presets::natural();
        mapping.available = em_spectra::BandMask::EMPTY;
        assert_eq!(band_columns(&mapping), [Vec4::ZERO; BANDS]);
    }

    fn sky() -> Session {
        Session::new(&AuthoredStars::sample(), 3)
    }

    #[test]
    fn the_mesh_is_one_quad_per_star() {
        let s = sky();
        let mesh = build_mesh(&s.stars, DVec3::ZERO);
        assert_eq!(mesh.count_vertices(), s.stars.len() * 4);
        assert_eq!(mesh.indices().unwrap().len(), s.stars.len() * 6);
    }

    /// The bake origin is the point of the whole arrangement: positions come out small and
    /// the shader differences two small numbers rather than two interstellar ones.
    #[test]
    fn positions_are_relative_to_the_bake_origin() {
        let s = sky();
        let star = &s.stars[0];
        let far = build_mesh(&s.stars, DVec3::ZERO);
        let near = build_mesh(&s.stars, star.position_ly);
        let first = |m: &Mesh| match m.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() {
            bevy_mesh::VertexAttributeValues::Float32x3(v) => Vec3::from_array(v[0]),
            _ => panic!("positions must be Float32x3"),
        };
        assert!((first(&far).length() as f64 - star.position_ly.length()).abs() < 1e-3);
        assert!(first(&near).length() < 1e-3, "baking at the star puts it at the origin");
    }

    #[test]
    fn the_uniforms_follow_the_ship() {
        let mut s = sky();
        let origin = s.position_ly;
        assert_eq!(uniforms(&s, origin, lut_scale()).ship_offset_ly, Vec4::ZERO);
        s.fly_to(s.stars[0].id);
        s.advance(8_000.0);
        let u = uniforms(&s, origin, lut_scale());
        assert!(u.ship_offset_ly.truncate().length() > 0.0, "the ship moved and the uniform did not");
        assert!(u.beta.truncate().length() > 0.5, "and it is moving fast");
        assert!(u.beta.truncate().length() < 1.0, "but not at or above c");
    }

    /// A rebake has to leave the sky where it was: the offset absorbs exactly what the
    /// positions gave up.
    #[test]
    fn rebaking_about_a_new_origin_does_not_move_the_sky() {
        let s = sky();
        let star = &s.stars[0];
        let ship = DVec3::new(0.3, -0.2, 0.1);
        let seen = |origin: DVec3| {
            let mesh_pos = sim_to_render(star.position_ly - origin).as_vec3();
            let offset = sim_to_render(ship - origin).as_vec3();
            (mesh_pos - offset).normalize()
        };
        let before = seen(DVec3::ZERO);
        let after = seen(ship);
        assert!((before - after).length() < 1e-6, "{before:?} vs {after:?}");
    }

    /// The renderer and the instrument have to agree about a swarm, or a star measures as
    /// engineered and looks ordinary.
    #[test]
    fn the_warm_attribute_matches_what_the_emission_model_computes() {
        let stars = lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv");
        let Ok(provider) = stars else { return };
        let swarmed: Vec<_> = provider
            .stars()
            .iter()
            .filter(|s| generate::swarm_for(s).is_some())
            .take(8)
            .collect();
        assert!(!swarmed.is_empty(), "the catalogue should hold some swarms");

        for star in swarmed {
            let [teff, scale, deficit, _] = warm_params(star);
            let swarm = generate::swarm_for(star).unwrap();
            assert!(teff > 0.0 && scale > 0.0, "{teff} K, scale {scale}");
            assert!((0.0..=1.0).contains(&deficit), "a deficit must be a fraction: {deficit}");

            // The shader computes band_radiance(b, teff) * scale; the model computes the same
            // thing through reradiated_radiance. They must be one number.
            let want = swarm.reradiated_radiance(&star.star)[Band::ThermalIr];
            let got = blackbody::band_radiance(Band::ThermalIr, teff as f64) * scale as f64;
            assert!((got / want - 1.0).abs() < 1e-5, "{got} against {want}");
        }
    }

    #[test]
    fn a_star_with_nothing_around_it_carries_no_heat() {
        let s = sky();
        for star in &s.stars {
            if generate::swarm_for(star).is_none() {
                assert_eq!(warm_params(star), [0.0; 4]);
            }
        }
    }
}
