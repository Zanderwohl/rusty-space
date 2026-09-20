//! Wireframe geometry: lat/lon grid spheres, great circles, and the tube builder under them.
//!
//! Meshes only, and no systems. Both products draw wireframes and neither draws them the same
//! way — one has a planetarium of bodies with terminators, the other a map of decade rings —
//! so what is shared is the geometry and what is not is every ECS system around it. That split
//! is the one `lightcone/docs/06-crate-layout.md` argues for, and it is why
//! [`build_tube_from_points`] is public: everything here is a tube, and a second tube builder
//! in the other product would be the duplication the extraction exists to avoid.
//!
//! Bevy's **Y-up render axes**, not simulation space: these are meshes, and a mesh is already
//! in the renderer. `render_space` is where the boundary is.
//!
//! Brightness rides in vertex-color **alpha**, which `body_wireframe.wgsl` reads as line
//! weight. Under a plain blended material it reads as opacity instead, which is the same
//! picture for a diagram — a solid equator and a faint grid.

use std::f32::consts::PI;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology, VertexAttributeValues};

/// Number of points per circle/parallel
const POINTS_PER_CIRCLE: u32 = 36;

/// Number of points per meridian (half-circle from pole to pole)
const POINTS_PER_MERIDIAN: u32 = 19;

/// Latitude spacing in degrees
const LAT_SPACING: f32 = 30.0;

/// Longitude spacing in degrees  
const LON_SPACING: f32 = 30.0;

/// Brightness for regular grid lines
const BRIGHTNESS_REGULAR: f32 = 0.6;

/// Brightness for highlight latitudes (tropics, circles)
const BRIGHTNESS_HIGHLIGHT: f32 = 0.85;

/// Brightness for equator and prime meridian
const BRIGHTNESS_PRIMARY: f32 = 1.0;

/// How far the pole skewer extends beyond the sphere (as a multiplier of radius)
const POLE_EXTENSION: f32 = 3.0;


/// Find two perpendicular vectors to form a plane orthogonal to the given direction.
fn perpendicular_vectors(dir: Vec3) -> (Vec3, Vec3) {
    let not_parallel = if dir.x.abs() < 0.9 {
        Vec3::X
    } else {
        Vec3::Y
    };

    let perp1 = dir.cross(not_parallel).normalize_or_zero();
    let perp2 = dir.cross(perp1).normalize_or_zero();

    (perp1, perp2)
}

/// Build tube geometry from a sequence of points.
/// Returns (positions, normals, colors, indices) buffers.
/// If `closed` is true, connects the last point back to the first.
pub fn build_tube_from_points(
    points: &[Vec3],
    brightness: f32,
    tube_radius: f32,
    tube_sides: u32,
    closed: bool,
    index_offset: u32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<u32>) {
    if points.len() < 2 {
        return (vec![], vec![], vec![], vec![]);
    }

    let ring_count = points.len();
    let verts_per_ring = tube_sides as usize;
    let total_verts = ring_count * verts_per_ring;

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total_verts);
    let mut colors: Vec<[f32; 4]> = Vec::with_capacity(total_verts);

    let connection_count = if closed { ring_count } else { ring_count - 1 };
    let mut indices: Vec<u32> = Vec::with_capacity(connection_count * verts_per_ring * 6);

    for (ring_idx, center) in points.iter().enumerate() {
        let tangent = if ring_idx == 0 {
            if closed {
                (points[1] - points[ring_count - 1]).normalize_or_zero()
            } else {
                (points[1] - *center).normalize_or_zero()
            }
        } else if ring_idx == ring_count - 1 {
            if closed {
                (points[0] - points[ring_idx - 1]).normalize_or_zero()
            } else {
                (*center - points[ring_idx - 1]).normalize_or_zero()
            }
        } else {
            (points[ring_idx + 1] - points[ring_idx - 1]).normalize_or_zero()
        };

        let (perp1, perp2) = perpendicular_vectors(tangent);

        for i in 0..tube_sides {
            let angle = (i as f32 / tube_sides as f32) * 2.0 * PI;
            let (sin_a, cos_a) = angle.sin_cos();

            let offset = perp1 * cos_a * tube_radius + perp2 * sin_a * tube_radius;
            let pos = *center + offset;
            positions.push([pos.x, pos.y, pos.z]);

            let normal = offset.normalize_or_zero();
            normals.push([normal.x, normal.y, normal.z]);

            colors.push([1.0, 1.0, 1.0, brightness]);
        }

        let should_connect = if closed {
            true
        } else {
            ring_idx < ring_count - 1
        };

        if should_connect {
            let base = index_offset + (ring_idx * verts_per_ring) as u32;
            let next_ring_idx = if ring_idx == ring_count - 1 { 0 } else { ring_idx + 1 };
            let next_base = index_offset + (next_ring_idx * verts_per_ring) as u32;

            for i in 0..tube_sides {
                let i_next = (i + 1) % tube_sides;

                indices.push(base + i);
                indices.push(next_base + i);
                indices.push(base + i_next);

                indices.push(base + i_next);
                indices.push(next_base + i);
                indices.push(next_base + i_next);
            }
        }
    }

    (positions, normals, colors, indices)
}

/// Build an empty mesh that still declares the vertex layout required by
/// `body_wireframe.wgsl` (`position`, `normal`, `color`).
fn empty_wireframe_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, Vec::<[f32; 3]>::new());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_COLOR,
        VertexAttributeValues::Float32x4(Vec::new()),
    );
    mesh.insert_indices(Indices::U32(Vec::new()));
    mesh
}

/// Convert latitude/longitude (in radians) to a point on the unit sphere.
/// Uses Bevy Y-up convention: +Y = north pole, +X = prime meridian (lon=0).
fn latlon_to_point(lat: f32, lon: f32) -> Vec3 {
    let cos_lat = lat.cos();
    Vec3::new(
        cos_lat * lon.cos(),
        lat.sin(),
        cos_lat * lon.sin(),
    )
}

/// Generate a parallel (latitude circle) as a sequence of points.
fn generate_parallel(lat_deg: f32, num_points: u32) -> Vec<Vec3> {
    let lat = lat_deg.to_radians();
    (0..num_points)
        .map(|i| {
            let lon = (i as f32 / num_points as f32) * 2.0 * PI;
            latlon_to_point(lat, lon)
        })
        .collect()
}

/// Generate a meridian (longitude half-circle from south to north pole) as a sequence of points.
fn generate_meridian(lon_deg: f32, num_points: u32) -> Vec<Vec3> {
    let lon = lon_deg.to_radians();
    (0..num_points)
        .map(|i| {
            let lat = -PI / 2.0 + (i as f32 / (num_points - 1) as f32) * PI;
            latlon_to_point(lat, lon)
        })
        .collect()
}

/// Generate a lat/lon wireframe sphere mesh.
/// `highlight_latitudes` are additional latitudes (in degrees, positive only) to draw with
/// intermediate brightness. Both +lat and -lat are drawn.
pub fn generate_latlon_sphere(highlight_latitudes: &[f64], tube_radius: f32, tube_sides: u32) -> Mesh {
    let mut all_positions: Vec<[f32; 3]> = Vec::new();
    let mut all_normals: Vec<[f32; 3]> = Vec::new();
    let mut all_colors: Vec<[f32; 4]> = Vec::new();
    let mut all_indices: Vec<u32> = Vec::new();

    let mut add_tube = |points: &[Vec3], brightness: f32, closed: bool| {
        let offset = all_positions.len() as u32;
        let (pos, norm, col, idx) = build_tube_from_points(points, brightness, tube_radius, tube_sides, closed, offset);
        all_positions.extend(pos);
        all_normals.extend(norm);
        all_colors.extend(col);
        all_indices.extend(idx);
    };

    // Collect all latitudes to draw
    let mut latitudes: Vec<(f32, f32)> = Vec::new(); // (lat_deg, brightness)

    // Standard parallels at 30-degree intervals (excluding poles)
    let mut lat = -60.0f32;
    while lat <= 60.0 {
        let brightness = if lat.abs() < 0.01 {
            BRIGHTNESS_PRIMARY // Equator
        } else {
            BRIGHTNESS_REGULAR
        };
        latitudes.push((lat, brightness));
        lat += LAT_SPACING;
    }

    // Add highlight latitudes (both positive and negative)
    for &hl in highlight_latitudes {
        let hl = hl as f32;
        if hl > 0.0 {
            // Check if it's not already close to an existing latitude
            let already_exists = latitudes.iter().any(|(l, _)| (l - hl).abs() < 1.0 || (l + hl).abs() < 1.0);
            if !already_exists {
                latitudes.push((hl, BRIGHTNESS_HIGHLIGHT));
                latitudes.push((-hl, BRIGHTNESS_HIGHLIGHT));
            }
        }
    }

    // Sort by latitude for consistent ordering
    latitudes.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Generate parallels
    for (lat_deg, brightness) in &latitudes {
        let points = generate_parallel(*lat_deg, POINTS_PER_CIRCLE);
        add_tube(&points, *brightness, true);
    }

    // Generate meridians
    let num_meridians = (360.0 / LON_SPACING) as u32;
    for i in 0..num_meridians {
        let lon_deg = i as f32 * LON_SPACING;
        let brightness = if lon_deg.abs() < 0.01 {
            BRIGHTNESS_PRIMARY // Prime meridian
        } else {
            BRIGHTNESS_REGULAR
        };
        let points = generate_meridian(lon_deg, POINTS_PER_MERIDIAN);
        add_tube(&points, brightness, false);
    }

    // Generate pole skewer (axis through north and south poles, extending beyond sphere)
    let pole_points = vec![
        Vec3::new(0.0, -POLE_EXTENSION, 0.0), // South extension
        Vec3::new(0.0, -1.0, 0.0),            // South pole
        Vec3::new(0.0, 1.0, 0.0),             // North pole
        Vec3::new(0.0, POLE_EXTENSION, 0.0),  // North extension
    ];
    add_tube(&pole_points, BRIGHTNESS_PRIMARY, false);

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, all_positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, all_normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(all_colors));
    mesh.insert_indices(Indices::U32(all_indices));

    mesh
}

/// Generate a great circle tube mesh perpendicular to the given normal vector.
/// Used for terminator lines.
pub fn generate_great_circle_tube(normal: Vec3, tube_radius: f32, tube_sides: u32) -> Mesh {
    let normal = normal.normalize_or_zero();
    if normal.length_squared() < 0.001 {
        return empty_wireframe_mesh();
    }

    // Find two perpendicular vectors in the great circle plane
    let (perp1, perp2) = perpendicular_vectors(normal);

    // Generate points around the great circle at unit radius
    let points: Vec<Vec3> = (0..POINTS_PER_CIRCLE)
        .map(|i| {
            let angle = (i as f32 / POINTS_PER_CIRCLE as f32) * 2.0 * PI;
            let (sin_a, cos_a) = angle.sin_cos();
            perp1 * cos_a + perp2 * sin_a
        })
        .collect();

    let (positions, normals, colors, indices) =
        build_tube_from_points(&points, BRIGHTNESS_PRIMARY, tube_radius, tube_sides, true, 0);

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(colors));
    mesh.insert_indices(Indices::U32(indices));

    mesh
}


// ---------------------------------------------------------------------------------------
// Map geometry.
//
// All three are **unit-sized** and scaled per entity. A map spans fifteen orders of
// magnitude, so a mesh built at a world size is one mesh per ring per frame; built at unit
// size it is one asset and a transform. The tubes go elliptical under a non-uniform scale,
// which on a four-sided tube is a fraction of a pixel and is the trade being made.
// ---------------------------------------------------------------------------------------

/// A unit-radius ring in the XZ plane, so its normal is `+Y` — Bevy's up, and what a map's
/// reference plane is rotated from.
pub fn ring_tube(segments: u32, tube_radius: f32, tube_sides: u32, brightness: f32) -> Mesh {
    let segments = segments.max(3);
    let points: Vec<Vec3> = (0..segments)
        .map(|i| {
            let angle = (i as f32 / segments as f32) * 2.0 * PI;
            let (sin_a, cos_a) = angle.sin_cos();
            Vec3::new(cos_a, 0.0, sin_a)
        })
        .collect();
    let (positions, normals, colors, indices) =
        build_tube_from_points(&points, brightness, tube_radius, tube_sides, true, 0);
    assemble(positions, normals, colors, indices)
}

/// A **filled** unit disc in the XZ plane, normal `+Y`. The one solid thing a wireframe map
/// draws, and what tells a contact from a body at a size where shape is all there is.
///
/// Wound both ways, so it is visible from either face and no caller has to think about the
/// material's cull mode. It is a fan from a rim vertex rather than from the center, because
/// the wireframe shader takes `normalize(position)` for its day/night term and a vertex at the
/// origin makes that a NaN — dead code at `num_suns: 0`, and not worth leaving loaded.
///
/// Every normal is `+Y`, so the shader's thickness displacement translates the disc along its
/// own normal instead of resizing it. A caller wanting it exactly unit-sized asks for a target
/// radius equal to the material's base.
pub fn disc(segments: u32, brightness: f32) -> Mesh {
    let segments = segments.max(3);
    let positions: Vec<[f32; 3]> = (0..segments)
        .map(|i| {
            let angle = (i as f32 / segments as f32) * 2.0 * PI;
            let (sin_a, cos_a) = angle.sin_cos();
            [cos_a, 0.0, sin_a]
        })
        .collect();
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let colors = vec![[1.0, 1.0, 1.0, brightness]; positions.len()];
    let mut indices = Vec::with_capacity((segments as usize - 2) * 6);
    for i in 1..segments - 1 {
        indices.extend([0, i, i + 1]);
        indices.extend([0, i + 1, i]);
    }
    assemble(positions, normals, colors, indices)
}

/// Radial spokes from the origin out to unit radius, in the XZ plane.
///
/// What makes an edge-on plane read as a plane. Seen from within it the rings are a single
/// line and carry no depth at all; the spokes are what still converge.
/// `inner` is where a spoke starts, as a fraction of its length: every spoke meeting every
/// other one at the origin is a blob rather than a center. It is a parameter because the mesh
/// is scaled, and a caller that stretches the spokes past the edge of the view has to shrink
/// it by the same factor or the hole in the middle grows with them.
pub fn plane_spokes(spokes: u32, inner: f32, tube_radius: f32, tube_sides: u32, brightness: f32)
    -> Mesh {
    let mut buffers = Buffers::default();
    let inner = inner.clamp(f32::MIN_POSITIVE, 0.9);
    for i in 0..spokes {
        let angle = (i as f32 / spokes.max(1) as f32) * 2.0 * PI;
        let (sin_a, cos_a) = angle.sin_cos();
        let out = Vec3::new(cos_a, 0.0, sin_a);
        buffers.add(&[out * inner, out], brightness, tube_radius, tube_sides, false);
    }
    buffers.into_mesh()
}

/// A dashed line of unit height along `+Y`, from the plane up to what hangs above it.
///
/// The mesh is scaled to the drop it is drawn for, so `dashes` sets how long each dash ends up
/// being — and the caller picks it per line to keep the dash itself a constant size. A single
/// count for every line would make a long drop's dashes long and a short one's short, which
/// reads as two different kinds of line rather than one line at two lengths.
pub fn drop_line(dashes: u32, tube_radius: f32, tube_sides: u32, brightness: f32) -> Mesh {
    let dashes = dashes.max(1);
    // `dashes` dashes and `dashes - 1` gaps, all the same length, so the line starts and ends
    // on a dash — an object with a gap under it looks detached from its own marker.
    let step = 1.0 / (2 * dashes - 1) as f32;
    let mut buffers = Buffers::default();
    for i in 0..dashes {
        let from = Vec3::Y * (2 * i) as f32 * step;
        buffers.add(&[from, from + Vec3::Y * step], brightness, tube_radius, tube_sides, false);
    }
    buffers.into_mesh()
}

/// One mesh from a set of polylines, each drawn as a closed or open tube.
///
/// What a population's outline is: two edge circles and four cross-sections, which is six
/// curves and one draw. `em_map::outline` decides the curves and this turns them into
/// geometry.
pub fn tube_curves(curves: &[Vec<Vec3>], tube_radius: f32, tube_sides: u32, brightness: f32)
    -> Mesh {
    let mut buffers = Buffers::default();
    for curve in curves {
        // Already closed by the caller, which repeats its first point — so an open tube, or
        // the joining quad would be laid over the repeat.
        buffers.add(curve, brightness, tube_radius, tube_sides, false);
    }
    buffers.into_mesh()
}

/// Several tubes accumulating into one mesh, keeping the index offset right.
#[derive(Default)]
struct Buffers {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
}

impl Buffers {
    fn add(&mut self, points: &[Vec3], brightness: f32, tube_radius: f32, tube_sides: u32,
        closed: bool) {
        let offset = self.positions.len() as u32;
        let (pos, norm, col, idx) =
            build_tube_from_points(points, brightness, tube_radius, tube_sides, closed, offset);
        self.positions.extend(pos);
        self.normals.extend(norm);
        self.colors.extend(col);
        self.indices.extend(idx);
    }

    fn into_mesh(self) -> Mesh {
        assemble(self.positions, self.normals, self.colors, self.indices)
    }
}

fn assemble(positions: Vec<[f32; 3]>, normals: Vec<[f32; 3]>, colors: Vec<[f32; 4]>,
    indices: Vec<u32>) -> Mesh {
    let mut mesh = empty_wireframe_mesh();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, VertexAttributeValues::Float32x4(colors));
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A disc is solid and two-sided, and has no vertex where the shader would divide by zero.
    #[test]
    fn a_disc_is_filled_from_both_sides() {
        let mesh = disc(16, 1.0);
        let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions")
        };
        assert_eq!(positions.len(), 16, "the rim, and nothing in the middle");
        for p in positions {
            let radius = (p[0] * p[0] + p[2] * p[2]).sqrt();
            assert!((radius - 1.0).abs() < 1.0e-5, "a rim point at {radius} from the center");
            assert!(p[1].abs() < 1.0e-6, "a disc is flat");
        }
        let Some(Indices::U32(indices)) = mesh.indices() else { panic!("no indices") };
        // Fourteen triangles to cover a sixteen-gon, and the same again reversed.
        assert_eq!(indices.len(), 14 * 3 * 2);
        let winding: i32 = indices
            .chunks(3)
            .map(|t| {
                let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize],
                    positions[t[2] as usize]);
                let cross = (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0]);
                cross.signum() as i32
            })
            .sum();
        assert_eq!(winding, 0, "every triangle should have its mirror");
    }

    /// Vertex positions, which is what every assertion here is actually about.
    pub(super) fn positions(mesh: &Mesh) -> Vec<Vec3> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(VertexAttributeValues::Float32x3(v)) => {
                v.iter().map(|p| Vec3::from_array(*p)).collect()
            }
            _ => panic!("a wireframe mesh without positions"),
        }
    }

    fn alphas(mesh: &Mesh) -> Vec<f32> {
        match mesh.attribute(Mesh::ATTRIBUTE_COLOR) {
            Some(VertexAttributeValues::Float32x4(v)) => v.iter().map(|c| c[3]).collect(),
            _ => panic!("a wireframe mesh without colors"),
        }
    }

    fn index_count(mesh: &Mesh) -> usize {
        mesh.indices().map(|i| i.len()).unwrap_or(0)
    }

    /// A grid sphere is a sphere: every vertex sits on the unit surface, give or take the
    /// tube's own thickness. Only the pole skewer sticks out, deliberately.
    #[test]
    fn a_grid_sphere_is_a_unit_sphere() {
        let mesh = generate_latlon_sphere(&[], 0.01, 4);
        let on_surface = positions(&mesh)
            .iter()
            .filter(|p| (p.length() - 1.0).abs() <= 0.02)
            .count();
        let beyond = positions(&mesh).iter().filter(|p| p.length() > 1.05).count();
        assert!(on_surface > 0, "nothing landed on the surface");
        assert!(beyond > 0, "the pole skewer did not extend past the sphere");
        assert!(
            positions(&mesh).iter().all(|p| p.length() <= POLE_EXTENSION + 0.1),
            "something is outside the skewer"
        );
    }

    /// Brightness rides in alpha, and the three weights are what make an equator read as one.
    /// A builder that wrote a single brightness would draw a uniform cage.
    #[test]
    fn the_grid_carries_its_three_line_weights() {
        let mesh = generate_latlon_sphere(&[23.4], 0.01, 4);
        let seen = alphas(&mesh);
        for (name, weight) in [
            ("regular", BRIGHTNESS_REGULAR),
            ("highlight", BRIGHTNESS_HIGHLIGHT),
            ("primary", BRIGHTNESS_PRIMARY),
        ] {
            assert!(
                seen.iter().any(|a| (a - weight).abs() < 1e-6),
                "no {name} lines at alpha {weight}"
            );
        }
    }

    /// A highlight latitude is drawn at both signs — a tropic is two circles, not one.
    #[test]
    fn a_highlight_latitude_is_drawn_north_and_south() {
        let plain = positions(&generate_latlon_sphere(&[], 0.01, 4)).len();
        let tropics = positions(&generate_latlon_sphere(&[23.4], 0.01, 4)).len();
        let one_circle = POINTS_PER_CIRCLE as usize * 4;
        assert_eq!(tropics - plain, 2 * one_circle, "expected two extra parallels");
    }

    /// A great circle lies in the plane its normal names. Getting this wrong puts a
    /// terminator at right angles to the light, which looks like a shading bug.
    #[test]
    fn a_great_circle_is_perpendicular_to_its_normal() {
        for normal in [Vec3::Y, Vec3::X, Vec3::new(1.0, 2.0, -3.0).normalize()] {
            let mesh = generate_great_circle_tube(normal, 0.01, 4);
            for p in positions(&mesh) {
                assert!(p.dot(normal).abs() < 0.02, "{p:?} is off the plane of {normal:?}");
                assert!((p.length() - 1.0).abs() < 0.02, "{p:?} is off the unit circle");
            }
        }
    }

    /// A zero normal names no plane, so there is nothing to draw — and the empty mesh still
    /// has to declare the layout `body_wireframe.wgsl` binds, or the pipeline fails at run
    /// time rather than at build time.
    #[test]
    fn a_circle_with_no_plane_is_empty_but_still_a_wireframe() {
        let mesh = generate_great_circle_tube(Vec3::ZERO, 0.01, 4);
        assert!(positions(&mesh).is_empty());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
        assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
    }

    /// Closing a tube joins the last ring to the first, which is one more quad.
    #[test]
    fn closing_a_tube_adds_the_joining_quad() {
        let points = vec![Vec3::X, Vec3::Y, Vec3::Z];
        let open = build_tube_from_points(&points, 1.0, 0.01, 4, false, 0);
        let closed = build_tube_from_points(&points, 1.0, 0.01, 4, true, 0);
        assert_eq!(closed.3.len() - open.3.len(), 4 * 6, "one ring of quads, four sides");
    }

    /// Fewer than two points is not a tube. It has to come back empty rather than
    /// degenerate: a zero-length tube is geometry with no normal, and the map draws one for
    /// every object that happens to sit in the reference plane.
    #[test]
    fn a_tube_needs_somewhere_to_go() {
        for points in [vec![], vec![Vec3::X]] {
            let (p, n, c, i) = build_tube_from_points(&points, 1.0, 0.01, 4, true, 0);
            assert!(p.is_empty() && n.is_empty() && c.is_empty() && i.is_empty());
        }
    }

    /// The offset is what lets one mesh hold many tubes, and an index that ignores it points
    /// at another tube's vertices — which draws as sheets of triangles across the sphere.
    #[test]
    fn the_index_offset_moves_the_indices() {
        let points = vec![Vec3::X, Vec3::Y];
        let (_, _, _, base) = build_tube_from_points(&points, 1.0, 0.01, 4, false, 0);
        let (_, _, _, moved) = build_tube_from_points(&points, 1.0, 0.01, 4, false, 100);
        assert_eq!(base.len(), moved.len());
        assert!(base.iter().zip(&moved).all(|(a, b)| b - a == 100));
    }

    /// A ring lies in the plane a map's reference plane is rotated from, at unit radius.
    #[test]
    fn a_ring_is_a_unit_circle_in_the_xz_plane() {
        let mesh = ring_tube(64, 0.01, 4, 1.0);
        for p in positions(&mesh) {
            assert!(p.y.abs() < 0.02, "{p:?} is off the XZ plane");
            assert!((Vec2::new(p.x, p.z).length() - 1.0).abs() < 0.02, "{p:?} is off the circle");
        }
    }

    /// And it closes. An open ring has a seam, which at a decade boundary is a gap in the one
    /// thing on screen that is claiming to be a circle.
    #[test]
    fn a_ring_closes_on_itself() {
        let open = build_tube_from_points(
            &(0..64).map(|i| {
                let a = (i as f32 / 64.0) * 2.0 * PI;
                Vec3::new(a.cos(), 0.0, a.sin())
            }).collect::<Vec<_>>(), 1.0, 0.01, 4, false, 0);
        assert_eq!(index_count(&ring_tube(64, 0.01, 4, 1.0)) - open.3.len(), 4 * 6);
    }

    /// Spokes reach the rim and start clear of the middle, where a dozen tubes meeting inside
    /// one tube's radius reads as a blob rather than as a center.
    #[test]
    fn spokes_reach_the_rim_without_piling_up_in_the_middle() {
        let mesh = plane_spokes(12, 0.02, 0.01, 4, 0.5);
        let radii: Vec<f32> = positions(&mesh).iter().map(|p| Vec2::new(p.x, p.z).length())
            .collect();
        assert!(radii.iter().any(|r| *r > 0.97), "no spoke reached the rim");
        assert!(radii.iter().all(|r| *r > 0.005), "a spoke ran through the center");
        assert!(positions(&mesh).iter().all(|p| p.y.abs() < 0.02), "a spoke left the plane");
    }

    /// A drop-line starts and ends on a dash. An object with a gap under it looks detached
    /// from its own marker, and the plane end looks like it is not quite touching.
    #[test]
    fn a_drop_line_starts_and_ends_on_a_dash() {
        for dashes in [1, 2, 5, 9] {
            let heights: Vec<f32> = positions(&drop_line(dashes, 0.01, 4, 1.0))
                .iter().map(|p| p.y).collect();
            let low = heights.iter().copied().fold(f32::INFINITY, f32::min);
            let high = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            assert!(low.abs() < 1e-5, "{dashes} dashes start at {low}, not the plane");
            assert!((high - 1.0).abs() < 1e-5, "{dashes} dashes end at {high}, not the top");
        }
    }

    /// It is dashed, which is to say it is more than one tube and they do not touch.
    #[test]
    fn a_drop_line_has_gaps_in_it() {
        let solid = index_count(&drop_line(1, 0.01, 4, 1.0));
        let dashed = index_count(&drop_line(6, 0.01, 4, 1.0));
        assert_eq!(dashed, solid * 6, "six dashes should be six tubes");

        let mut heights: Vec<f32> = positions(&drop_line(6, 0.01, 4, 1.0))
            .iter().map(|p| p.y).collect();
        heights.sort_by(f32::total_cmp);
        heights.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        assert_eq!(heights.len(), 12, "six dashes have twelve ends");
    }

    /// A set of curves becomes one mesh, and an empty set is still a wireframe rather than a
    /// panic.
    #[test]
    fn curves_become_one_tube_mesh() {
        let square: Vec<Vec3> = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0)]
            .iter()
            .map(|(x, z)| Vec3::new(*x, 0.0, *z))
            .collect();
        let one = tube_curves(std::slice::from_ref(&square), 0.01, 4, 1.0);
        let two = tube_curves(&[square.clone(), square], 0.01, 4, 1.0);
        assert_eq!(positions(&two).len(), 2 * positions(&one).len());
        assert!(
            two.indices().unwrap().iter().all(|i| (i as usize) < positions(&two).len()),
            "the second curve's indices were not offset",
        );

        let empty = tube_curves(&[], 0.01, 4, 1.0);
        assert!(positions(&empty).is_empty());
        assert!(empty.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
    }

    /// Asking for nothing must not produce a mesh that divides by zero or wraps.
    #[test]
    fn degenerate_requests_are_still_meshes() {
        for mesh in [ring_tube(0, 0.01, 4, 1.0), plane_spokes(0, 0.02, 0.01, 4, 1.0),
            drop_line(0, 0.01, 4, 1.0)] {
            assert!(positions(&mesh).iter().all(|p| p.is_finite()), "a NaN got into a buffer");
            assert!(mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some());
        }
    }

    #[test]
    fn a_grid_sphere_is_a_closed_index_buffer() {
        let mesh = generate_latlon_sphere(&[], 0.01, 4);
        assert!(index_count(&mesh) > 0);
        assert_eq!(index_count(&mesh) % 3, 0, "a triangle list with a partial triangle");
        let vertices = positions(&mesh).len() as u32;
        assert!(
            mesh.indices().unwrap().iter().all(|i| (i as u32) < vertices),
            "an index points past the end of the vertex buffer"
        );
    }
}
