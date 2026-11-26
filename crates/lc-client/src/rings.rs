//! Planetary rings: the annulus each ringed body gets, the bands its shader reads, and keeping it
//! on its body.
//!
//! The band list goes to the shader whole (`ring.wgsl`), so this module only packs it and places
//! the mesh. Brightness is still the old display mapping, [`ring_opacity`]; see
//! `ring-rendering-plan.md` phase 2 for what replaces it.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::render_space::sim_to_render;
use em_render::ring_material::{RING_BANDS, RingMaterial, RingUniform};
use glam::DVec3;
use lc_world::rings::RingSystem;

use crate::envelope::{opacity_of, orientation};
use crate::system::{M_PER_LY, UNIT_M};

/// Longitude divisions of the annulus. The bands are drawn by the shader, so this only has to
/// keep the outline round.
const SEGMENTS: usize = 128;

/// How far the mesh reaches past the material, in units of the outer edge: past the sag of a
/// chord between segments (`1 / cos(pi / SEGMENTS)` is 1.0003), and enough that a pixel on an
/// edge still has a fragment to filter.
const MARGIN: f32 = 0.01;

const TINT: Vec4 = Vec4::new(0.88, 0.84, 0.76, 1.0);

/// Grains across the outer radius. The grain varies with radius only, so it reads as ringlets.
const GRAIN_FREQUENCY: f32 = 40.0;
const GRAIN_STRENGTH: f32 = 0.35;

/// How opaque a ring system is drawn. Rings are solid and reflective rather than a shadow, so
/// this is what they cover of their own disc, not of a sphere.
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

/// The bands as the shader takes them, inside out: `(inner, outer, depth, icy)` in units of the
/// outer edge with depth over the deepest band's, and how many there are.
///
/// A system with more bands than the uniform holds loses its narrowest gaps first. No system
/// does today: the generator names at most twelve and the measured ones have four.
pub fn pack(rings: &RingSystem) -> (usize, [Vec4; RING_BANDS]) {
    let spans = fitted(rings.bands.iter().map(|b| Span { inner: b.inner_m, outer: b.outer_m, depth: b.optical_depth }).collect());
    let outer = rings.outer_m().max(f64::MIN_POSITIVE);
    let peak = spans.iter().map(|s| s.depth).fold(0.0, f64::max).max(f64::MIN_POSITIVE);
    let mut packed = [Vec4::ZERO; RING_BANDS];
    for (slot, s) in packed.iter_mut().zip(&spans) {
        *slot = Vec4::new((s.inner / outer) as f32, (s.outer / outer) as f32, (s.depth / peak) as f32, 0.0);
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

/// `depth_over` in `ring.wgsl`: packed depth averaged over the radii `radius +- width / 2`.
pub fn depth_over(bands: &[Vec4], count: usize, radius: f32, width: f32) -> f32 {
    let half = 0.5 * width;
    let within = |edge: f32| (edge - radius).clamp(-half, half);
    bands.iter().take(count.min(RING_BANDS)).map(|b| b.z * (within(b.y) - within(b.x))).sum::<f32>() / width
}

pub fn uniform(rings: &RingSystem, seed: f32) -> RingUniform {
    let (count, bands) = pack(rings);
    RingUniform {
        tint: TINT,
        bands,
        params: Vec4::new(count as f32, ring_opacity(rings), RingUniform::default().params.z, 0.0),
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
fn placement(body: &crate::system::Drawable, rings: &lc_world::system::Rings, eye_ly: DVec3) -> Transform {
    Transform {
        translation: sim_to_render((body.position_ly - eye_ly) * M_PER_LY / UNIT_M).as_vec3(),
        rotation: orientation(rings.pole),
        scale: Vec3::splat((rings.system.outer_m() / UNIT_M) as f32),
    }
}

/// Spawn a ring for every drawn body that has one, and keep each on its body.
pub fn update_rings(
    mut commands: Commands,
    session: Res<crate::app::Game>,
    eye: Res<crate::hull::Eye>,
    bodies: Res<crate::starfield::Bodies>,
    mut state: ResMut<Rings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RingMaterial>>,
    grain: Res<crate::procedural::PopulationGrain>,
    mut placed: Query<(Entity, &mut Transform, &RingMesh)>,
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
            commands.spawn((
                Mesh3d(meshes.add(build_annulus(inner))),
                MeshMaterial3d(materials.add(RingMaterial {
                    uniforms: uniform(&rings.system, i as f32 * 3.77 + 0.5),
                    grain: grain.image.clone(),
                })),
                placement(body, rings, eye.at_ly),
                NoFrustumCulling,
                RingMesh { body: i, radius: (rings.system.outer_m() / UNIT_M) as f32 },
            ));
        }
        return;
    }
    for (_, mut transform, ring) in &mut placed {
        let Some(body) = bodies.drawn.get(ring.body) else { continue };
        let Some(rings) = &body.rings else { continue };
        *transform = placement(body, rings, eye.at_ly);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_world::rings::{Band, for_body};
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
        assert!((b - 1.0).abs() < 1e-5, "the B ring is the peak: {b}");
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
            assert!(bands[..count].iter().all(|b| b.z > 0.0 && b.z <= 1.0), "{id}: depths over the peak");
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
                Band { name: "A", inner_m: 0.5, outer_m: 0.75 - gap / 2.0, optical_depth: 1.0 },
                Band { name: "B", inner_m: 0.75 + gap / 2.0, outer_m: 1.0, optical_depth: 1.0 },
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
        let bands: Vec<Band> = (0..20)
            .map(|i| {
                let inner = 1.0 + i as f64;
                // Every fourth gap is wide, the rest narrow.
                let gap = if i % 4 == 3 { 0.5 } else { 0.01 };
                Band { name: "x", inner_m: inner, outer_m: inner + 1.0 - gap, optical_depth: 0.1 }
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
}
