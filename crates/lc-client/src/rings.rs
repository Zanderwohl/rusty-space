//! Planetary rings: the annulus each ringed body gets, the bands and light its shader reads, and
//! keeping it on its body.
//!
//! A ring is shaded as its planet is: starlight its particles reflect and heat they radiate, per
//! band, through the current band mapping, and tone-mapped per fragment in the planet's exposure
//! (`ring.wgsl`). On top of that is a display floor, so a ring too thin to see is still faintly
//! there. What the particles cover is the alpha, so a ring hides what is behind it.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::render_space::sim_to_render;
use em_render::ring_material::{RING_BANDS, RingMaterial, RingUniform};
use em_spectra::{BANDS, Band, BandMapping, PerBand};
use glam::DVec3;
use lc_world::ground::Ground;
use lc_world::rings::RingSystem;

use crate::envelope::{opacity_of, orientation};
use crate::system::{Drawable, M_PER_LY, UNIT_M};

/// Longitude divisions of the annulus. The bands are drawn by the shader, so this only has to
/// keep the outline round.
const SEGMENTS: usize = 128;

/// How far the mesh reaches past the material, in units of the outer edge: past the sag of a
/// chord between segments (`1 / cos(pi / SEGMENTS)` is 1.0003), and enough that a pixel on an
/// edge still has a fragment to filter.
const MARGIN: f32 = 0.01;

/// Grains across the outer radius. The grain varies with radius only, so it reads as ringlets.
const GRAIN_FREQUENCY: f32 = 40.0;
const GRAIN_STRENGTH: f32 = 0.35;

/// What a band is drawn at, at least, for its coverage `c` before lighting: `RING_FLOOR c^(1/4)`.
///
/// A display decision, as `envelope::DISPLAY_LEVEL` is for a population: the planet's exposure
/// window is five stops, so by its true brightness a ring under an optical depth of about 0.01 is
/// black beside its planet. The value is linear and the screen's sRGB encoding lifts it, so 0.01
/// shows as about 25 of 255. Tuned on the reference shots: Jupiter's rings (optical depth 3e-6)
/// at about 13 of 255, a generated ring at 4e-5 seen obliquely at about 27, and from about 0.1 up
/// the true brightness is higher and the floor does nothing.
pub const RING_FLOOR: f32 = 0.1;

/// Ring ice in [`Band`] order, as a share of its V reflectance: water ice reddened by the dust
/// mixed through it, which is why Saturn's rings are tan where snow is blue-white, and dark by
/// 2 µm where ice absorbs. Nothing reflects in the emissive bands.
const RING_ICE: [f32; BANDS] = [0.75, 1.0, 1.1, 1.12, 0.25, 0.0, 0.0];

/// Ring ice's emissivity at 10 µm and at 21 cm. Ice is nearly lossless in the radio, so a ring of
/// it radiates a few kelvin there however warm it is; Saturn's do.
const RING_ICE_EMISSIVE: (f32, f32) = (0.97, 0.05);

/// Particles at least this bright are ice. The generator draws rocky rings from 0.03 to 0.1 and
/// icy ones from 0.1 to 0.6, and the measured systems split the same way.
const ICY_ALBEDO: f64 = 0.1;

/// How opaque a ring system was drawn before its light was its own; the beauty shots still
/// choose rings by it (`ring-rendering-plan.md` phase 6). What the rings cover of their own disc.
pub fn ring_opacity(rings: &RingSystem) -> f32 {
    let disc = std::f64::consts::PI * rings.outer_m().powi(2);
    opacity_of(rings.cross_section_m2() / disc.max(f64::MIN_POSITIVE))
}

/// A ring system, which unlike a population is attached to a body and therefore moves.
#[derive(Component)]
#[require(crate::app::InGameOnly = crate::app::in_game_only())]
pub struct RingMesh {
    /// The body's place in this frame's drawables.
    ///
    /// An index rather than the name it could be found by: `drawables_at` walks the arena in a
    /// fixed order and only ever drops a body, so while the list is the same length it holds
    /// the same bodies in the same places. [`Rings::drawn`] is what makes that true — a list
    /// that changed shape respawns these rather than moving them all by one.
    pub body: usize,
    /// Outer radius in render units.
    pub radius: f32,
}

/// Which system the ring entities were spawned for.
#[derive(Resource, Default)]
pub struct Rings {
    pub star: Option<lc_world::sky::StarId>,
    /// How many drawables the ring entities' indices were taken against.
    pub drawn: usize,
}

/// One band before packing: inner and outer radius in meters, and optical depth.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    inner: f64,
    outer: f64,
    depth: f64,
}

impl Span {
    fn area(&self) -> f64 {
        std::f64::consts::PI * (self.outer * self.outer - self.inner * self.inner).max(0.0)
    }
}

pub fn is_icy(rings: &RingSystem) -> bool {
    rings.albedo >= ICY_ALBEDO
}

/// The bands as the shader takes them, inside out: `(inner, outer, optical depth, icy)` with radii
/// in units of the outer edge, and how many there are. Every band takes the system's composition
/// until a band carries its own.
///
/// A system with more bands than the uniform holds loses its narrowest gaps first. No system
/// does today: the generator names at most twelve and the measured ones have four.
pub fn pack(rings: &RingSystem) -> (usize, [Vec4; RING_BANDS]) {
    let spans = fitted(rings.bands.iter().map(|b| Span { inner: b.inner_m, outer: b.outer_m, depth: b.optical_depth }).collect());
    let outer = rings.outer_m().max(f64::MIN_POSITIVE);
    let icy = f32::from(u8::from(is_icy(rings)));
    let mut packed = [Vec4::ZERO; RING_BANDS];
    for (slot, s) in packed.iter_mut().zip(&spans) {
        *slot = Vec4::new((s.inner / outer) as f32, (s.outer / outer) as f32, s.depth as f32, icy);
    }
    (spans.len(), packed)
}

/// `spans`, sorted, with the narrowest gap closed until they fit. A closed gap's band carries the
/// two bands' cross-section over the whole merged area, so the material is kept.
fn fitted(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by(|a, b| a.inner.total_cmp(&b.inner));
    while spans.len() > RING_BANDS {
        let narrowest = (0..spans.len() - 1)
            .min_by(|&i, &j| (spans[i + 1].inner - spans[i].outer).total_cmp(&(spans[j + 1].inner - spans[j].outer)))
            .expect("more than one span");
        let (a, b) = (spans[narrowest], spans.remove(narrowest + 1));
        let mut merged = Span { inner: a.inner, outer: b.outer, depth: 0.0 };
        merged.depth = (a.depth * a.area() + b.depth * b.area()) / merged.area().max(f64::MIN_POSITIVE);
        spans[narrowest] = merged;
    }
    spans
}

/// Packed optical depth averaged over the radii `radius +- width / 2`, as `ring.wgsl` takes each
/// band's share of a pixel.
pub fn depth_over(bands: &[Vec4], count: usize, radius: f32, width: f32) -> f32 {
    let half = 0.5 * width;
    let within = |edge: f32| (edge - radius).clamp(-half, half);
    bands.iter().take(count.min(RING_BANDS)).map(|b| b.z * (within(b.y) - within(b.x))).sum::<f32>() / width
}

/// The alpha `ring.wgsl` draws a pixel at: the share of what is behind it the particles stop,
/// seen at `mu`, averaged over the radii `radius +- width / 2`.
pub fn coverage_over(bands: &[Vec4], count: usize, radius: f32, width: f32, mu: f32) -> f32 {
    let half = 0.5 * width;
    let within = |edge: f32| (edge - radius).clamp(-half, half);
    let sum: f32 = bands.iter().take(count.min(RING_BANDS)).map(|b| (within(b.y) - within(b.x)) * (1.0 - (-b.z / mu).exp())).sum();
    (sum / width).clamp(0.0, 1.0)
}

/// `lit_face` in `ring.wgsl`: the sunlit face of a sheet at optical depth `tau`, seen at `mu` and
/// lit at `mu0`, as a share of a thick sheet lit and seen face-on.
pub fn lit_face(mu: f32, mu0: f32, tau: f32) -> f32 {
    if mu0 <= 0.0 {
        return 0.0;
    }
    2.0 * mu0 / (mu + mu0) * (1.0 - (-tau * (1.0 / mu + 1.0 / mu0)).exp())
}

/// `unlit_face` in `ring.wgsl`: the face away from the star.
pub fn unlit_face(mu: f32, mu0: f32, tau: f32) -> f32 {
    if mu0 <= 0.0 {
        return 0.0;
    }
    if (mu - mu0).abs() < 1.0e-3 {
        return 2.0 * mu0 * tau / (mu * mu) * (-tau / mu).exp();
    }
    2.0 * mu0 / (mu - mu0) * ((-tau / mu).exp() - (-tau / mu0).exp())
}

/// The floor `ring.wgsl` draws a band at, on its sunlit face, seen at `mu`.
pub fn floor_of(tau: f32, mu: f32) -> f32 {
    RING_FLOOR * opacity_of(f64::from(1.0 - (-tau / mu).exp()))
}

/// What a ring's particles are made of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Ice,
    Rock,
}

/// What a ring's particles are, band by band: reflectance scaled so V is the ring's albedo, and
/// emissivity by Kirchhoff where they reflect.
pub struct Particles {
    pub reflectance: [f32; BANDS],
    pub emissivity: [f32; BANDS],
}

pub fn particles(material: Material, albedo: f64) -> Particles {
    let (shape, (thermal, radio)) = match material {
        Material::Ice => (RING_ICE, RING_ICE_EMISSIVE),
        Material::Rock => {
            let e = Ground::Basalt.emissivity();
            (Ground::Basalt.reflectance(), (e[Band::ThermalIr.index()], e[Band::Radio.index()]))
        }
    };
    let scale = albedo as f32 / shape[Band::V.index()].max(f32::MIN_POSITIVE);
    let reflectance = shape.map(|r| (r * scale).clamp(0.0, 1.0));
    let emissivity = std::array::from_fn(|k| match Band::ALL[k] {
        Band::ThermalIr => thermal,
        Band::Radio => radio,
        _ => 1.0 - reflectance[k],
    });
    Particles { reflectance, emissivity }
}

/// A particle's temperature: in equilibrium with the star at the planet's distance, warmed by
/// what it absorbs.
pub fn particle_k(star_luminosity_w: f64, distance_m: f64, albedo: f64) -> f64 {
    lc_world::system::equilibrium_temperature(star_luminosity_w, distance_m) * (1.0 - albedo).max(0.0).powf(0.25)
}

/// The star as a ring at one distance sees it.
#[derive(Clone, Copy)]
pub struct Starlight {
    pub radius_m: f64,
    pub teff_k: f64,
    pub luminosity_w: f64,
    pub distance_m: f64,
}

/// What a filled sheet of these particles reflects and radiates, as linear display light through
/// `mapping`: `(reflected, emitted)`.
pub fn light_of(particles: &Particles, albedo: f64, star: Starlight, mapping: &BandMapping) -> (Vec3, Vec3) {
    let incident = crate::resolved::lit_radiance(1.0, star.radius_m, star.teff_k, star.distance_m);
    let glow = crate::session::spectrum_at(particle_k(star.luminosity_w, star.distance_m, albedo));
    let reflected = PerBand::new(std::array::from_fn(|k| particles.reflectance[k] * incident[Band::ALL[k]]));
    let emitted = PerBand::new(std::array::from_fn(|k| particles.emissivity[k] * glow[Band::ALL[k]]));
    (Vec3::from_array(mapping.apply(&reflected)), Vec3::from_array(mapping.apply(&emitted)))
}

/// Everything the shader reads for one ring this frame.
pub fn uniform(rings: &lc_world::system::Rings, star: Starlight, to_star: DVec3, mapping: &BandMapping, tone: &crate::tonemap::ToneMap, seed: f32) -> RingUniform {
    let (count, bands) = pack(&rings.system);
    let albedo = rings.system.albedo;
    let (reflected_rocky, emitted_rocky) = light_of(&particles(Material::Rock, albedo), albedo, star, mapping);
    let (reflected_icy, emitted_icy) = light_of(&particles(Material::Ice, albedo), albedo, star, mapping);
    RingUniform {
        bands,
        params: Vec4::new(count as f32, RING_FLOOR, 0.0, 0.0),
        reflected_rocky: reflected_rocky.extend(0.0),
        reflected_icy: reflected_icy.extend(0.0),
        emitted_rocky: emitted_rocky.extend(0.0),
        emitted_icy: emitted_icy.extend(0.0),
        to_star: sim_to_render(to_star.normalize_or_zero()).as_vec3().extend(0.0),
        exposure: Vec4::new(tone.surface_reference, tone.surface_stops, 0.0, 0.0),
        grain: Vec4::new(GRAIN_FREQUENCY, GRAIN_STRENGTH, seed, 0.0),
    }
}

/// A flat annulus from `inner` to the outer edge at one, with [`MARGIN`] either side. Every
/// normal is the pole.
pub fn build_annulus(inner: f32) -> Mesh {
    let radii = [(inner - MARGIN).max(0.0), 1.0 + MARGIN];
    let pole = sim_to_render(DVec3::Z).as_vec3().to_array();
    let mut positions = Vec::with_capacity(2 * (SEGMENTS + 1));
    for radius in radii {
        for s in 0..=SEGMENTS {
            let (sin, cos) = (std::f64::consts::TAU * s as f64 / SEGMENTS as f64).sin_cos();
            positions.push(sim_to_render(DVec3::new(cos, sin, 0.0) * f64::from(radius)).as_vec3().to_array());
        }
    }
    let row = (SEGMENTS + 1) as u32;
    let indices = (0..SEGMENTS as u32).flat_map(|s| [s, s + row, s + 1, s + 1, s + row, s + row + 1]).collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![pole; positions.len()]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// Where a body's rings go this frame, seen from `eye_ly`.
fn placement(body: &Drawable, rings: &lc_world::system::Rings, eye_ly: DVec3) -> Transform {
    Transform {
        translation: sim_to_render((body.position_ly - eye_ly) * M_PER_LY / UNIT_M).as_vec3(),
        rotation: orientation(rings.pole),
        scale: Vec3::splat((rings.system.outer_m() / UNIT_M) as f32),
    }
}

fn seed_of(index: usize) -> f32 {
    index as f32 * 3.77 + 0.5
}

/// Spawn a ring for every drawn body that has one, and keep each on its body, on its body's
/// layer, and lit for this frame.
pub fn update_rings(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    eye: Res<crate::hull::Eye>,
    bodies: Res<crate::starfield::Bodies>,
    shot: Res<crate::beauty::ShotBody>,
    camera: Query<(&Projection, &Camera), With<crate::app::SkyCamera>>,
    mut state: ResMut<Rings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RingMaterial>>,
    grain: Res<crate::procedural::PopulationGrain>,
    mut placed: Query<(Entity, &mut Transform, &mut Visibility, &mut RenderLayers, &MeshMaterial3d<RingMaterial>, &RingMesh)>,
) {
    let here = session.system.as_ref().map(|s| s.star);
    if state.star != here || state.drawn != bodies.drawn.len() {
        for (entity, ..) in &placed {
            commands.entity(entity).despawn();
        }
        state.star = here;
        state.drawn = bodies.drawn.len();
        for (i, body) in bodies.drawn.iter().enumerate() {
            let Some(rings) = &body.rings else { continue };
            let inner = (rings.system.inner_m() / rings.system.outer_m().max(f64::MIN_POSITIVE)) as f32;
            // Lit and layered from the next frame, with the rest.
            commands.spawn((
                Mesh3d(meshes.add(build_annulus(inner))),
                MeshMaterial3d(materials.add(RingMaterial { uniforms: RingUniform::default(), grain: grain.image.clone() })),
                placement(body, rings, eye.at_ly),
                Visibility::Hidden,
                RenderLayers::layer(0),
                NoFrustumCulling,
                RingMesh { body: i, radius: (rings.system.outer_m() / UNIT_M) as f32 },
            ));
        }
        return;
    }
    let Some(system) = session.system.as_ref() else { return };
    let rad_per_px = crate::starfield::camera_scale(&camera);
    let star_ly = system.star_position_ly();
    for (_, mut transform, mut visibility, mut layers, material, ring) in &mut placed {
        let Some(body) = bodies.drawn.get(ring.body) else { continue };
        let Some(rings) = &body.rings else { continue };
        *transform = placement(body, rings, eye.at_ly);
        let Some(layer) = crate::resolved::layer_for(body, rings.system.outer_m(), eye.at_ly, rad_per_px, &shot) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        visibility.set_if_neq(Visibility::Inherited);
        layers.set_if_neq(RenderLayers::layer(layer));

        let star = Starlight {
            radius_m: system.star_radius_m(),
            teff_k: system.star_teff_k(),
            luminosity_w: system.star_luminosity_w(),
            distance_m: star_ly.distance(body.position_ly) * M_PER_LY,
        };
        // A ring only the telescope draws is exposed with its planet, which is metered for itself.
        let mut tone = session.tone;
        if layer != 0 {
            let radiance = crate::resolved::surface_radiance(body, star.radius_m, star.teff_k, star.distance_m);
            tone.surface_reference = crate::resolved::metered_for(&tone, &radiance, &session.mapping);
        }
        let next = uniform(rings, star, star_ly - body.position_ly, &session.mapping, &tone, seed_of(ring.body));
        if let Some(mut asset) = materials.get_mut(&material.0)
            && asset.uniforms != next
        {
            asset.uniforms = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_world::rings::for_body;
    use std::borrow::Cow;

    const PIXEL: f32 = 1.0e-3;

    fn packed(id: &str) -> (usize, [Vec4; RING_BANDS]) {
        pack(for_body(id).unwrap())
    }

    /// Saturn walked outward: the B ring is the peak, the Cassini Division a thin dip that is not
    /// empty, and the A ring past it.
    #[test]
    fn saturn_has_the_division_the_data_says_it_has() {
        let (count, bands) = packed("Saturn");
        let at = |r: f32| depth_over(&bands, count, r, PIXEL);
        let r_saturn = 5.8232e7 / 1.36775e8;
        let (b, division, a) = (at(1.8 * r_saturn), at(2.06 * r_saturn), at(2.2 * r_saturn));
        assert!((b - 1.5).abs() < 1e-5, "the B ring at its own depth: {b}");
        assert!(division > 0.0 && division < b / 10.0, "the division is thin, not empty: {division}");
        assert!(a > division * 4.0, "and the A ring is thicker again: {a}");
        assert_eq!(at(1.1 * r_saturn), 0.0, "nothing inside the C ring");
        assert_eq!(at(1.0 + 2.0 * PIXEL), 0.0, "nothing outside the A ring");
    }

    /// Bands come out inside out in units of the outer edge, the last ending at one.
    #[test]
    fn bands_are_packed_inside_out_against_the_outer_edge() {
        for id in ["Saturn", "Jupiter", "Uranus", "Neptune"] {
            let (count, bands) = packed(id);
            assert_eq!(count, for_body(id).unwrap().bands.len());
            assert!(bands[..count].windows(2).all(|w| w[0].y <= w[1].x), "{id}: out of order");
            assert!((bands[count - 1].y - 1.0).abs() < 1e-6, "{id}: the outermost band ends at one");
            let system = for_body(id).unwrap();
            assert!(bands[..count].iter().zip(system.bands.iter()).all(|(p, b)| p.z == b.optical_depth as f32), "{id}: depths as measured");
            let icy = f32::from(u8::from(id == "Saturn"));
            assert!(bands[..count].iter().all(|b| b.w == icy), "{id}: only Saturn's rings are ice");
            assert!(bands[count..].iter().all(|b| *b == Vec4::ZERO), "{id}: unused slots are empty");
        }
    }

    /// Neptune's Adams ring is 70 km wide. The vertex grid this replaced missed it outright; the
    /// filter finds it at any pixel size, dimmed by its share of the pixel.
    #[test]
    fn a_ring_narrower_than_a_pixel_dims_its_pixel_by_its_share() {
        let (count, bands) = packed("Neptune");
        let adams = bands[count - 1];
        let width = adams.y - adams.x;
        let center = 0.5 * (adams.x + adams.y);
        let wide = 10.0 * width;
        let seen = depth_over(&bands, count, center, wide);
        assert!((seen - adams.z * 0.1).abs() < 1e-4, "a tenth of the pixel at full depth: {seen}");
        let resolved = depth_over(&bands, count, center, width * 0.5);
        assert!((resolved / adams.z - 1.0).abs() < 1e-4, "resolved, it is its own depth: {resolved}");
    }

    /// The same for a gap: a gap a tenth of a pixel wide takes a tenth off the pixel.
    #[test]
    fn a_gap_narrower_than_a_pixel_takes_its_share_off() {
        let gap = 1.0e-4;
        let system = RingSystem {
            bands: Cow::Owned(vec![
                lc_world::rings::Band { name: "A", inner_m: 0.5, outer_m: 0.75 - gap / 2.0, optical_depth: 1.0 },
                lc_world::rings::Band { name: "B", inner_m: 0.75 + gap / 2.0, outer_m: 1.0, optical_depth: 1.0 },
            ]),
            albedo: 0.5,
        };
        let (count, bands) = pack(&system);
        let seen = depth_over(&bands, count, 0.75, (10.0 * gap) as f32);
        assert!((seen - 0.9).abs() < 1e-3, "{seen}");
    }

    /// Past the uniform's capacity the narrowest gaps close first, and the material is kept.
    #[test]
    fn too_many_bands_close_their_narrowest_gaps_and_keep_the_material() {
        let bands: Vec<lc_world::rings::Band> = (0..20)
            .map(|i| {
                let inner = 1.0 + i as f64;
                // Every fourth gap is wide, the rest narrow.
                let gap = if i % 4 == 3 { 0.5 } else { 0.01 };
                lc_world::rings::Band { name: "x", inner_m: inner, outer_m: inner + 1.0 - gap, optical_depth: 0.1 }
            })
            .collect();
        let system = RingSystem { bands: Cow::Owned(bands), albedo: 0.5 };
        let spans = fitted(system.bands.iter().map(|b| Span { inner: b.inner_m, outer: b.outer_m, depth: b.optical_depth }).collect());
        assert_eq!(spans.len(), RING_BANDS);
        let cross = |s: &[Span]| s.iter().map(|s| s.depth * s.area()).sum::<f64>();
        let before: Vec<Span> = system.bands.iter().map(|b| Span { inner: b.inner_m, outer: b.outer_m, depth: b.optical_depth }).collect();
        assert!((cross(&spans) / cross(&before) - 1.0).abs() < 1e-9, "the cross-section is kept");
        // The wide gaps survive.
        let wide = spans.windows(2).filter(|w| w[1].inner - w[0].outer > 0.4).count();
        assert_eq!(wide, 4, "{spans:?}");
    }

    /// The annulus covers the material and its margin, and is flat.
    #[test]
    fn the_annulus_covers_the_bands_and_is_flat() {
        let mesh = build_annulus(0.5);
        let bevy_mesh::VertexAttributeValues::Float32x3(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap() else {
            panic!("positions must be Float32x3")
        };
        let radius = |p: &[f32; 3]| (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        let (lo, hi) = positions.iter().map(radius).fold((f32::MAX, 0.0f32), |(lo, hi), r| (lo.min(r), hi.max(r)));
        assert!(lo < 0.5 && hi > 1.0, "{lo}..{hi}");
        // The chord between segments has to stay outside the unit circle.
        assert!(hi * (std::f32::consts::PI / SEGMENTS as f32).cos() > 1.0);
        let bevy_mesh::VertexAttributeValues::Float32x3(normals) = mesh.attribute(Mesh::ATTRIBUTE_NORMAL).unwrap() else {
            panic!("normals must be Float32x3")
        };
        assert!(normals.iter().all(|n| *n == normals[0]), "every normal is the pole");
    }

    /// The four measured systems, in the order a person would rank them by eye.
    #[test]
    fn the_ring_systems_come_out_in_the_right_order() {
        let drawn = |id: &str| ring_opacity(for_body(id).unwrap());
        let (saturn, uranus, neptune, jupiter) =
            (drawn("Saturn"), drawn("Uranus"), drawn("Neptune"), drawn("Jupiter"));
        assert!(saturn > 0.5, "Saturn's rings are the thing you see: {saturn}");
        assert!(uranus < saturn * 0.5 && uranus > 0.05, "Uranus's are faint but real: {uranus}");
        assert!(neptune < uranus, "Neptune's fainter still: {neptune}");
        assert!(jupiter < 0.05, "and Jupiter's took Voyager to find: {jupiter}");
    }

    const AU: f64 = 1.495_978_707e11;

    fn sun_at(au: f64) -> Starlight {
        Starlight { radius_m: 6.957e8, teff_k: 5772.0, luminosity_w: 3.828e26, distance_m: au * AU }
    }

    /// Display light of a sheet through `mapping`, against a gray sheet of the same albedo, per
    /// channel: what the particles' spectrum does to the color, with the star's taken out.
    fn against_gray(material: Material, mapping: &BandMapping) -> Vec3 {
        let star = sun_at(9.5);
        let (ice, _) = light_of(&particles(material, 0.5), 0.5, star, mapping);
        let gray = Particles { reflectance: [0.5; BANDS], emissivity: [0.5; BANDS] };
        let (flat, _) = light_of(&gray, 0.5, star, mapping);
        Vec3::select(flat.cmpgt(Vec3::ZERO), ice / flat, Vec3::ZERO)
    }

    #[test]
    fn particles_are_as_bright_in_v_as_the_albedo_says() {
        for material in [Material::Ice, Material::Rock] {
            for albedo in [0.03, 0.1, 0.5] {
                let p = particles(material, albedo);
                assert!((p.reflectance[Band::V.index()] - albedo as f32).abs() < 1e-6);
                assert!(p.emissivity.iter().all(|e| (0.0..=1.0).contains(e)));
            }
        }
    }

    /// Thermal maps 10 µm, K and V to red, green and blue. Ice is bright in V and dark by 2 µm,
    /// so an icy ring loses green against blue; rock barely changes across them.
    #[test]
    fn an_icy_ring_is_dark_in_k_and_bright_in_v() {
        let thermal = em_spectra::presets::thermal();
        let (ice, rock) = (against_gray(Material::Ice, &thermal), against_gray(Material::Rock, &thermal));
        assert!(ice.y < ice.z * 0.3, "ice in K against V: {ice}");
        assert!(rock.y > rock.z * 0.8, "rock in K against V: {rock}");
    }

    /// Dust penetration maps 21 cm, 10 µm and K. Nothing reflects at the first two, so all a ring
    /// sends there is its own glow; K is all its reflection. Ice barely radiates at 21 cm, so an
    /// icy ring is far fainter there than a rocky one at the same temperature.
    #[test]
    fn under_dust_penetration_a_ring_reflects_only_in_k() {
        let dust = em_spectra::presets::dust_penetration();
        let (reflected, emitted) = light_of(&particles(Material::Ice, 0.5), 0.5, sun_at(9.5), &dust);
        assert_eq!((reflected.x, reflected.y), (0.0, 0.0), "{reflected}");
        assert!(reflected.z > 0.0 && emitted.y > 0.0, "{reflected} {emitted}");
        let (_, rock) = light_of(&particles(Material::Rock, 0.5), 0.5, sun_at(9.5), &dust);
        assert!(emitted.x < rock.x * 0.1, "21 cm, ice against rock: {} {}", emitted.x, rock.x);
    }

    /// A rocky ring at 0.03 AU is hundreds of kelvin and glows at 10 µm; an icy one at 10 AU is
    /// eighty and barely does.
    #[test]
    fn a_hot_rocky_ring_glows_and_a_cold_icy_one_barely_does() {
        let thermal = em_spectra::presets::thermal();
        let (_, hot) = light_of(&particles(Material::Rock, 0.06), 0.06, sun_at(0.03), &thermal);
        let (_, cold) = light_of(&particles(Material::Ice, 0.5), 0.5, sun_at(10.0), &thermal);
        assert!(hot.x > cold.x * 1.0e3, "10 µm: {} against {}", hot.x, cold.x);
        assert!(particle_k(3.828e26, 0.03 * AU, 0.06) > 1000.0);
    }

    /// A thick ring is dark from the side away from the star; a thin one looks about the same
    /// from both.
    #[test]
    fn only_a_thick_ring_is_dark_on_its_unlit_face() {
        let (mu, mu0) = (0.5, 0.3);
        let ratio = |tau: f32| unlit_face(mu, mu0, tau) / lit_face(mu, mu0, tau);
        assert!(ratio(2.0) < 0.1, "thick: {}", ratio(2.0));
        assert!(ratio(0.05) > 0.8, "thin: {}", ratio(0.05));
        // The limit where the two angles meet is continuous with the general formula.
        let near = unlit_face(0.5, 0.5 + 2.0e-3, 0.4);
        assert!((unlit_face(0.5, 0.5, 0.4) / near - 1.0).abs() < 0.02);
        assert_eq!(lit_face(0.5, 0.0, 1.0), 0.0, "with the star in the ring's plane nothing is lit");
    }

    /// Saturn's B ring and Saturn have about the same albedo, and face-on the ring's depth of 1.5
    /// makes it nearly a filled sheet, so the two are about equally bright: the ring is what puts
    /// back the planet's missing brightness in `system.rs`.
    #[test]
    fn saturns_b_ring_is_about_as_bright_as_saturn() {
        let saturn = for_body("Saturn").unwrap();
        let ring = saturn.albedo as f32 * lit_face(1.0, 1.0, 1.5);
        let disc = 0.499;
        assert!((0.5..2.0).contains(&(ring / disc)), "ring over disc: {}", ring / disc);
    }

    /// The floor keeps the measured systems in order by their densest band, and sits where the
    /// tuning says, in linear light: Jupiter's barely, a generated ring at 0.1 plainly.
    #[test]
    fn the_floor_keeps_the_order_and_its_targets() {
        let densest = |id: &str| for_body(id).unwrap().bands.iter().map(|b| b.optical_depth as f32).fold(0.0, f32::max);
        let floor = |id: &str| floor_of(densest(id), 1.0);
        assert!(floor("Saturn") > floor("Uranus") && floor("Uranus") > floor("Neptune") && floor("Neptune") > floor("Jupiter"));
        assert!(floor_of(3.0e-6, 1.0) < 0.005, "Jupiter's main ring: {}", floor_of(3.0e-6, 1.0));
        assert!(floor_of(0.1, 1.0) > 0.05, "a generated ring at 0.1: {}", floor_of(0.1, 1.0));
        assert!(floor_of(100.0, 0.01) <= RING_FLOOR);
        assert_eq!(floor_of(0.0, 1.0), 0.0);
    }

    /// A ring hides what is behind it by what its particles cover: Saturn's B ring most of it, its
    /// Cassini Division little, Jupiter's rings nothing to speak of; and obliquely, more.
    #[test]
    fn a_ring_hides_what_is_behind_it_by_its_coverage() {
        let (count, bands) = packed("Saturn");
        let r_saturn = 5.8232e7 / 1.36775e8;
        let at = |r: f32, mu: f32| coverage_over(&bands, count, r * r_saturn, PIXEL, mu);
        assert!((at(1.8, 1.0) - (1.0 - (-1.5f32).exp())).abs() < 1e-3, "B ring face-on: {}", at(1.8, 1.0));
        assert!(at(2.06, 1.0) < 0.1, "the division: {}", at(2.06, 1.0));
        assert!(at(2.06, 0.1) > at(2.06, 1.0) * 5.0, "obliquely the division closes");
        assert_eq!(at(3.0, 1.0), 0.0);
        let (count, bands) = packed("Jupiter");
        let main = bands[1];
        assert!(coverage_over(&bands, count, 0.5 * (main.x + main.y), PIXEL, 1.0) < 1.0e-5);
    }
}
