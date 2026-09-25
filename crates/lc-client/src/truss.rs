//! The truss a refit step stands up: a lattice of girders at a fixed pitch in meters, square to
//! the ship's frame, kept where it falls in the sliver's shell. 32 §A build step is a frontier.
//!
//! Whole girders, so the cost goes with what is kept; past [`MAX_GIRDERS`] the hull shader draws
//! the lattice on the sliver's surface instead.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::hull_material::ATTRIBUTE_HULL_GIRDER;
use glam::DVec3;
use lc_world::form::sdf::Piece;

/// One lattice for every hull, so the truss is the same size in meters on any of them. Also the
/// plating's panel, so panels close over whole cells.
pub const PITCH_M: f64 = 8.0;
pub const GIRDER_RADIUS_M: f64 = 0.35;
/// Past this the truss is drawn by the hull shader alone. About a million vertices.
pub const MAX_GIRDERS: usize = 60_000;
/// Nodes of the sliver's box worth looking at before giving up on a mesh.
const MAX_NODES: f64 = 4.0e6;
/// How far out of the finished surface the scaffold stands, and how deep into the part the truss
/// reaches, in pitches.
const SCAFFOLD_OUT: f64 = 1.0;
const DEPTH: f64 = 2.0;
/// Layers of girders a line of sight into the sliver crosses.
pub const LAYERS: f64 = SCAFFOLD_OUT + DEPTH;
/// A node's octahedron, in girder radii: enough to cover the open ends of the tubes meeting in it.
const KNUCKLE: f64 = 1.6;
const SIDES: usize = 6;

#[derive(Clone, Debug, Default)]
pub struct TrussBuffers {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// [`ATTRIBUTE_HULL_GIRDER`].
    pub girders: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    pub count: usize,
}

impl TrussBuffers {
    pub fn into_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        mesh.insert_attribute(ATTRIBUTE_HULL_GIRDER, self.girders);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

/// The girder from `node` one pitch along `axis`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Girder {
    pub node: [i64; 3],
    pub axis: usize,
}

impl Girder {
    pub fn ends(&self) -> (DVec3, DVec3) {
        let a = node_at(self.node);
        let mut e = DVec3::ZERO;
        e[self.axis] = PITCH_M;
        (a, a + e)
    }

    /// When it goes up, as the share of its band that must have passed. Hashed, so girders go up
    /// one at a time rather than as a line.
    pub fn threshold(&self) -> f64 {
        let mut h = 0x9e37_79b9_7f4a_7c15u64 ^ self.axis as u64;
        for x in self.node {
            h = mix(h ^ x as u64);
        }
        // Never 0 or 1, so nothing stands before its band starts or after it ends.
        ((h >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn node_at(i: [i64; 3]) -> DVec3 {
    DVec3::new(i[0] as f64, i[1] as f64, i[2] as f64) * PITCH_M
}

/// The girders of `copy`'s sliver: each whose two nodes lie within [`DEPTH`] pitches inside the
/// copy's surface and [`SCAFFOLD_OUT`] outside it, and outside everything `standing` measures,
/// which is the ship the step leaves alone. `None` when there are too many to mesh. Each is
/// paired with whether it is scaffold, standing outside the finished surface.
pub fn girders(copy: &Piece, standing: &dyn Fn(DVec3) -> f64) -> Option<Vec<(Girder, bool)>> {
    let inverse = copy.pose.rotation.transpose();
    let outer = |p: DVec3| copy.shape.distance(inverse * (p - copy.pose.position));
    let half = copy.shape.extent(copy.pose.rotation, SCAFFOLD_OUT * PITCH_M);
    let lo = ((copy.pose.position - half) / PITCH_M).floor().as_i64vec3().to_array();
    let hi = ((copy.pose.position + half) / PITCH_M).ceil().as_i64vec3().to_array();
    let n = [0, 1, 2].map(|a| (hi[a] - lo[a] + 1) as usize);
    if (n[0] as f64) * (n[1] as f64) * (n[2] as f64) > MAX_NODES {
        return None;
    }
    let mut inside: HashMap<[i64; 3], bool> = HashMap::new();
    let mut within = |i: [i64; 3]| {
        *inside.entry(i).or_insert_with(|| {
            let p = node_at(i);
            let d = outer(p);
            (-DEPTH * PITCH_M..=SCAFFOLD_OUT * PITCH_M).contains(&d) && standing(p) >= 0.0
        })
    };
    let mut out = Vec::new();
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                let node = [x, y, z];
                if !within(node) {
                    continue;
                }
                for axis in 0..3 {
                    let mut next = node;
                    next[axis] += 1;
                    if within(next) {
                        let girder = Girder { node, axis };
                        let (a, b) = girder.ends();
                        out.push((girder, outer((a + b) / 2.0) > 0.0));
                        if out.len() > MAX_GIRDERS {
                            return None;
                        }
                    }
                }
            }
        }
    }
    Some(out)
}

/// The girders as tubes, with an octahedron at each node they meet in. `joint` is where the
/// step's bands sweep from, and `span_m` how far they go.
pub fn mesh(girders: &[(Girder, bool)], joint: DVec3, span_m: f64) -> TrussBuffers {
    let mut out = TrussBuffers { count: girders.len(), ..default() };
    let r = GIRDER_RADIUS_M;
    let x_of = |p: DVec3| (p.distance(joint).min(span_m)) as f32;
    // Per node: the earliest girder's threshold and where it reads its band, and whether all of
    // them are scaffold, so the knuckle stands exactly while one of its girders does.
    let mut nodes: HashMap<[i64; 3], (f64, f32, bool)> = HashMap::new();
    for &(g, scaffold) in girders {
        let (a, b) = g.ends();
        let attribute = [x_of((a + b) / 2.0), g.threshold() as f32, if scaffold { 1.0 } else { 0.0 }, 0.0];
        let (u, v) = ((g.axis + 1) % 3, (g.axis + 2) % 3);
        let base = out.positions.len() as u32;
        for k in 0..SIDES {
            let angle = std::f64::consts::TAU * k as f64 / SIDES as f64;
            let mut n = DVec3::ZERO;
            n[u] = angle.cos();
            n[v] = angle.sin();
            for end in [a, b] {
                out.positions.push((end + n * r).as_vec3().to_array());
                out.normals.push(n.as_vec3().to_array());
                out.girders.push(attribute);
            }
        }
        for k in 0..SIDES as u32 {
            let (i0, i1) = (base + 2 * k, base + 2 * ((k + 1) % SIDES as u32));
            // Outward: `u`, then `v`, then along the axis is right-handed.
            out.indices.extend([i0, i1, i0 + 1, i1, i1 + 1, i0 + 1]);
        }
        let mut next = g.node;
        next[g.axis] += 1;
        for node in [g.node, next] {
            let slot = nodes.entry(node).or_insert((1.0, attribute[0], true));
            if g.threshold() < slot.0 {
                (slot.0, slot.1) = (g.threshold(), attribute[0]);
            }
            slot.2 &= scaffold;
        }
    }
    let mut nodes: Vec<_> = nodes.into_iter().collect();
    nodes.sort_by_key(|(node, _)| *node);
    for (node, (threshold, x, scaffold)) in nodes {
        let at = node_at(node);
        let attribute = [x, threshold as f32, if scaffold { 1.0 } else { 0.0 }, 0.0];
        let base = out.positions.len() as u32;
        let tips = [DVec3::X, DVec3::NEG_X, DVec3::Y, DVec3::NEG_Y, DVec3::Z, DVec3::NEG_Z];
        for t in tips {
            out.positions.push((at + t * KNUCKLE * r).as_vec3().to_array());
            out.normals.push(t.as_vec3().to_array());
            out.girders.push(attribute);
        }
        for (sx, sy, sz) in [(0, 2, 4), (2, 1, 4), (1, 3, 4), (3, 0, 4), (2, 0, 5), (1, 2, 5), (3, 1, 5), (0, 3, 5)] {
            out.indices.extend([base + sx, base + sy, base + sz]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_world::form::place::{Pose, Side};
    use lc_world::form::primitive::Shape;
    use lc_world::form::{Kind, PartId};

    fn ball(radius: f64, at: DVec3) -> Piece {
        Piece {
            part: PartId(1),
            side: Side::Original,
            kind: Kind::Storage,
            shape: Shape::Ellipsoid { semi_axes: DVec3::splat(radius) },
            pose: Pose { position: at, rotation: glam::DMat3::IDENTITY },
        }
    }

    /// A sliver around a ball: every girder is in the shell, none inside what stands, and the
    /// scaffold is exactly the girders outside the finished surface.
    #[test]
    fn girders_fill_the_shell_and_nothing_inside_it() {
        let copy = ball(40.0, DVec3::ZERO);
        let inner = |p: DVec3| p.length() - 30.0;
        let kept = girders(&copy, &inner).expect("a small sliver meshes");
        assert!(!kept.is_empty());
        let mut scaffold = 0;
        for (g, is_scaffold) in &kept {
            let (a, b) = g.ends();
            for end in [a, b] {
                assert!(inner(end) >= 0.0, "{g:?} inside what stands");
                assert!(end.length() - 40.0 <= SCAFFOLD_OUT * PITCH_M + 1e-9, "{g:?} too far out");
            }
            assert_eq!(*is_scaffold, (a + b).length() / 2.0 > 40.0);
            scaffold += usize::from(*is_scaffold);
        }
        assert!(scaffold > 0 && scaffold < kept.len(), "{scaffold} of {}", kept.len());
    }

    /// The pitch is meters, not a share of the part: a ball five times larger has girders just as
    /// long, and about twenty-five times as many over a shell as thick.
    #[test]
    fn the_pitch_is_the_same_in_meters_on_any_size() {
        let (small, large) = (ball(30.0, DVec3::ZERO), ball(150.0, DVec3::ZERO));
        let shell = |r: f64| move |p: DVec3| p.length() - (r - 6.0);
        let a = girders(&small, &shell(30.0)).unwrap();
        let b = girders(&large, &shell(150.0)).unwrap();
        for (g, _) in a.iter().chain(&b) {
            let (p, q) = g.ends();
            assert!((p.distance(q) - PITCH_M).abs() < 1e-9);
        }
        let ratio = b.len() as f64 / a.len() as f64;
        assert!((10.0..60.0).contains(&ratio), "{ratio}");
    }

    /// A GSV's sliver is refused rather than meshed, which leaves it to the shader.
    #[test]
    fn a_fifty_kilometer_sliver_is_left_to_the_shader() {
        let copy = ball(20_000.0, DVec3::ZERO);
        assert!(girders(&copy, &|p: DVec3| p.length() - 18_000.0).is_none());
    }

    #[test]
    fn thresholds_are_spread_and_never_at_the_ends() {
        let mut sum = 0.0;
        let n = 2000;
        for i in 0..n {
            let t = Girder { node: [i, -i / 3, 7], axis: (i % 3) as usize }.threshold();
            assert!(t > 0.0 && t < 1.0);
            sum += t;
        }
        assert!((sum / n as f64 - 0.5).abs() < 0.03, "{}", sum / n as f64);
    }

    /// Every tube faces out: its normals point away from the girder's axis, and each triangle
    /// winds counterclockwise seen from outside.
    #[test]
    fn tubes_and_knuckles_face_out() {
        let g = Girder { node: [0, 0, 0], axis: 1 };
        let m = mesh(&[(g, false)], DVec3::ZERO, 100.0);
        assert_eq!(m.positions.len(), 2 * SIDES + 2 * 6);
        for t in m.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from_array(m.positions[t[k] as usize]));
            let n = Vec3::from_array(m.normals[t[0] as usize]) + Vec3::from_array(m.normals[t[1] as usize]) + Vec3::from_array(m.normals[t[2] as usize]);
            assert!((b - a).cross(c - a).dot(n) > 0.0, "{t:?} winds inward");
        }
        assert!(m.girders.iter().all(|g| g[2] == 0.0 && g[0] >= 0.0));
    }
}
