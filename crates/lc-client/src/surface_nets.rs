//! Surface nets over any signed distance field: a closed, manifold mesh of where it is zero.
//! `hull_mesh` paints what it makes of a form, or of bare pieces.
//!
//! **Manifold by construction.** Before extraction the grid is cleaned of every lattice square
//! whose corners alternate in sign, by turning one of its outside corners inside. Every cell
//! face then carries at most one segment of surface, so the crossings in a cell form simple
//! loops, and a vertex a loop (not a cell) leaves every edge shared by exactly two triangles
//! and every vertex with one fan of them. The blocky finish is the same topology with its
//! vertices moved, so it is closed the same way.
//!
//! A feature thinner than a cell, like a strap on a coarse grid, is kept only where a sample
//! lands in it. It may vanish or be holed; diagonal neighbors are joined rather than split, and
//! what is left is closed.

use std::collections::{HashMap, VecDeque};

use glam::DVec3;
use lc_world::form::sdf::Sdf;

/// A signed distance in meters, negative inside, and Lipschitz 1: the grid skips a block on the
/// strength of one sample at its center.
pub trait Field {
    fn distance(&self, p: DVec3, scratch: &mut Vec<f64>) -> f64;
    /// Contains every point where the distance is not positive, `(min, max)`.
    fn bounds(&self) -> (DVec3, DVec3);
}

impl Field for Sdf {
    fn distance(&self, p: DVec3, scratch: &mut Vec<f64>) -> f64 {
        self.distance_with(p, scratch)
    }

    fn bounds(&self) -> (DVec3, DVec3) {
        Sdf::bounds(self)
    }
}

/// Samples of the field on a lattice whose outermost layer lies outside the bounds, so every
/// sample on it is outside and the surface is closed.
pub(crate) struct Grid {
    pub(crate) origin: DVec3,
    pub(crate) step: f64,
    pub(crate) n: [usize; 3],
    pub(crate) values: Vec<f32>,
    /// Blocks that may hold a sign change, as block coordinates.
    pub(crate) near: Vec<[usize; 3]>,
}

/// Samples a side of the blocks the grid is evaluated in.
const BLOCK: usize = 8;

/// `f32` rounds a positive distance below about 1e-45 to zero, which would read as inside.
fn narrow(d: f64) -> f32 {
    let v = d as f32;
    if d > 0.0 && v <= 0.0 { f32::MIN_POSITIVE } else { v }
}

/// Where cleaning puts a sample it turns inside, in cells: far enough in that its crossings
/// are not all at the sample, which would put several vertices on one point.
const DILATED: f64 = 0.1;

pub(crate) const AXES: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z];

/// The two axes across each axis, in the order the cell's edge table indexes them.
const ACROSS: [(usize, usize); 3] = [(1, 2), (0, 2), (0, 1)];

impl Grid {
    pub(crate) fn sample(field: &impl Field, cells: u32) -> Grid {
        let (min, max) = field.bounds();
        let size = max - min;
        let step = size.max_element() / cells.max(1) as f64;
        let n = size.to_array().map(|s| (s / step).ceil() as usize + 3);
        let origin = min - DVec3::splat(step);
        let mut grid = Grid { origin, step, n, values: vec![0.0; n[0] * n[1] * n[2]], near: Vec::new() };

        // Lipschitz 1: a block whose center is farther from the surface than any of its samples
        // or their neighbors two cells out has one sign across all of them, so no crossing
        // touches it and its samples need only the sign.
        let reach = ((BLOCK as f64) * 3f64.sqrt() / 2.0 + 2.0) * step;
        let blocks = n.map(|n| n.div_ceil(BLOCK));
        let mut scratch = Vec::new();
        for bz in 0..blocks[2] {
            for by in 0..blocks[1] {
                for bx in 0..blocks[0] {
                    let lo = [bx, by, bz].map(|b| b * BLOCK);
                    let hi = [0, 1, 2].map(|a| (lo[a] + BLOCK).min(n[a]));
                    let middle = DVec3::from_array([0, 1, 2].map(|a| (lo[a] + hi[a] - 1) as f64 / 2.0));
                    let d = field.distance(origin + middle * step, &mut scratch);
                    let far = d.abs() > reach;
                    if !far {
                        grid.near.push([bx, by, bz]);
                    }
                    for z in lo[2]..hi[2] {
                        for y in lo[1]..hi[1] {
                            for x in lo[0]..hi[0] {
                                let i = grid.index([x, y, z]);
                                let d = if far { d } else { field.distance(grid.position([x, y, z]), &mut scratch) };
                                grid.values[i] = narrow(d);
                            }
                        }
                    }
                }
            }
        }
        grid.clean();
        grid
    }

    pub(crate) fn index(&self, i: [usize; 3]) -> usize {
        i[0] + self.n[0] * (i[1] + self.n[1] * i[2])
    }

    fn position(&self, i: [usize; 3]) -> DVec3 {
        self.origin + DVec3::new(i[0] as f64, i[1] as f64, i[2] as f64) * self.step
    }

    fn value(&self, i: [usize; 3]) -> f32 {
        self.values[self.index(i)]
    }

    fn inside(&self, i: [usize; 3]) -> bool {
        self.value(i) <= 0.0
    }

    fn on_boundary(&self, i: [usize; 3]) -> bool {
        (0..3).any(|a| i[a] == 0 || i[a] == self.n[a] - 1)
    }

    /// Turns inside one outside corner of each lattice square whose corners alternate, until
    /// none does. Each turn makes an outside sample inside, so it ends. The corner turned is
    /// the one nearer the surface. A boundary sample is never one: a square that alternates
    /// has an inside corner diagonal to each outside one, and no boundary sample is inside.
    pub(crate) fn clean(&mut self) {
        let mut queue = VecDeque::new();
        for block in self.near.clone() {
            let samples: Vec<[usize; 3]> = self.block_samples(block).collect();
            for i in samples {
                for axis in 0..3 {
                    self.clean_square(axis, i, &mut queue);
                }
            }
        }
        while let Some((axis, base)) = queue.pop_front() {
            self.clean_square(axis, base, &mut queue);
        }
        self.near.sort_unstable();
        self.near.dedup();
    }

    /// Queues every square the turned sample is a corner of.
    fn clean_square(&mut self, axis: usize, base: [usize; 3], queue: &mut VecDeque<(usize, [usize; 3])>) {
        let Some(corners) = self.square(axis, base) else { return };
        let [a, b, c, d] = corners.map(|i| self.inside(i));
        if !(a == d && b == c && a != b) {
            return;
        }
        let (p, q) = if a { (corners[1], corners[2]) } else { (corners[0], corners[3]) };
        let pick = match (self.on_boundary(p), self.on_boundary(q)) {
            (false, true) => p,
            (true, false) => q,
            (true, true) => return,
            (false, false) if self.value(p) <= self.value(q) => p,
            (false, false) => q,
        };
        let i = self.index(pick);
        self.values[i] = -(DILATED * self.step) as f32;
        self.near.push(pick.map(|x| x / BLOCK));
        for axis in 0..3 {
            let (u, v) = ACROSS[axis];
            for (du, dv) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                if pick[u] >= du && pick[v] >= dv {
                    let mut base = pick;
                    base[u] -= du;
                    base[v] -= dv;
                    queue.push_back((axis, base));
                }
            }
        }
    }

    /// The square across `axis` at `base`, corners in the order `base`, `+u`, `+v`, `+u+v`.
    fn square(&self, axis: usize, base: [usize; 3]) -> Option<[[usize; 3]; 4]> {
        let (u, v) = ACROSS[axis];
        if base[u] + 1 >= self.n[u] || base[v] + 1 >= self.n[v] {
            return None;
        }
        let step = |i: [usize; 3], a: usize| {
            let mut j = i;
            j[a] += 1;
            j
        };
        Some([base, step(base, u), step(base, v), step(step(base, u), v)])
    }

    fn block_samples(&self, block: [usize; 3]) -> impl Iterator<Item = [usize; 3]> + '_ {
        let lo = block.map(|b| b * BLOCK);
        let hi = [0, 1, 2].map(|a| (lo[a] + BLOCK).min(self.n[a]));
        (lo[2]..hi[2]).flat_map(move |z| (lo[1]..hi[1]).flat_map(move |y| (lo[0]..hi[0]).map(move |x| [x, y, z])))
    }

    /// Every sample that is the base of a cell, lattice edge or voxel that may cross the surface.
    /// An edge that crosses has an end in a near block, either because the field crosses there
    /// or because cleaning turned it, and the cells around it are based within two samples of
    /// that end, so in the same block or the next.
    fn candidates(&self) -> impl Iterator<Item = [usize; 3]> + '_ {
        let blocks = self.n.map(|n| n.div_ceil(BLOCK));
        let mut marked = vec![false; blocks[0] * blocks[1] * blocks[2]];
        for &[x, y, z] in &self.near {
            for dz in 0..3 {
                for dy in 0..3 {
                    for dx in 0..3 {
                        let (bx, by, bz) = ((x + dx).wrapping_sub(1), (y + dy).wrapping_sub(1), (z + dz).wrapping_sub(1));
                        if bx < blocks[0] && by < blocks[1] && bz < blocks[2] {
                            marked[bx + blocks[0] * (by + blocks[1] * bz)] = true;
                        }
                    }
                }
            }
        }
        let marked: Vec<[usize; 3]> = (0..marked.len())
            .filter(|&k| marked[k])
            .map(|k| [k % blocks[0], k / blocks[0] % blocks[1], k / blocks[0] / blocks[1]])
            .collect();
        marked.into_iter().flat_map(|block| self.block_samples(block))
    }
}

/// Vertices and outward, counterclockwise triangles over them.
pub(crate) struct Surface {
    pub(crate) vertices: Vec<DVec3>,
    pub(crate) triangles: Vec<[u32; 3]>,
}

impl Surface {
    /// Split along the shorter diagonal, which keeps slivers out of a curved sheet.
    fn quad(&mut self, q: [u32; 4]) {
        let p = q.map(|i| self.vertices[i as usize]);
        if p[0].distance_squared(p[2]) <= p[1].distance_squared(p[3]) {
            self.triangles.extend([[q[0], q[1], q[2]], [q[0], q[2], q[3]]]);
        } else {
            self.triangles.extend([[q[0], q[1], q[3]], [q[1], q[2], q[3]]]);
        }
    }
}

/// A cell's twelve edges as pairs of corners, corner `c` at offset `(c & 1, c >> 1 & 1, c >> 2)`.
/// Edge `4a + bu + 2bv` runs along axis `a` at offsets `bu`, `bv` across it, per [`ACROSS`].
const EDGES: [(usize, usize); 12] =
    [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];

/// Each face's four edges.
const FACES: [[usize; 4]; 6] = [[4, 10, 6, 8], [5, 11, 7, 9], [0, 9, 2, 8], [1, 11, 3, 10], [0, 5, 1, 4], [2, 7, 3, 6]];

const NO_LOOP: u8 = u8::MAX;

/// Which loop of crossings each edge is on, from the cell's corner signs. Each face holds at
/// most two crossings once the grid is clean, so each pairs its two and the pairs chain into
/// loops.
fn loops(inside: [bool; 8]) -> ([u8; 12], u8) {
    let crosses = EDGES.map(|(a, b)| inside[a] != inside[b]);
    // For each edge, its two faces and its partner on each.
    let mut partner = [[usize::MAX; 2]; 12];
    let mut faces_of = [[usize::MAX; 2]; 12];
    for (f, face) in FACES.iter().enumerate() {
        let on: Vec<usize> = face.iter().copied().filter(|&e| crosses[e]).collect();
        debug_assert!(on.len() != 4, "a face with four crossings survived cleaning");
        for pair in on.chunks(2).filter(|p| p.len() == 2) {
            for (e, other) in [(pair[0], pair[1]), (pair[1], pair[0])] {
                let slot = usize::from(faces_of[e][0] != usize::MAX);
                faces_of[e][slot] = f;
                partner[e][slot] = other;
            }
        }
    }
    let mut id = [NO_LOOP; 12];
    let mut count = 0u8;
    for start in 0..12 {
        if !crosses[start] || id[start] != NO_LOOP {
            continue;
        }
        let (mut edge, mut via) = (start, faces_of[start][0]);
        loop {
            id[edge] = count;
            let slot = usize::from(faces_of[edge][0] == via);
            let next = partner[edge][slot];
            via = faces_of[edge][slot];
            edge = next;
            if edge == start || edge == usize::MAX {
                break;
            }
        }
        count += 1;
    }
    (id, count)
}

/// Surface nets: a vertex at the mean of each loop's crossings, and a quad around each lattice
/// edge that crosses, joining the vertices of the four cells around it.
///
/// `blocky` puts each vertex at its cell's center instead. The quad around a lattice edge is then
/// the face between the cubes about the edge's two samples, so this is the occupied samples as
/// cubes, with the loops keeping two cubes that meet only at a corner from sharing a vertex.
pub(crate) fn nets(grid: &Grid, blocky: bool) -> Surface {
    let mut surface = Surface { vertices: Vec::new(), triangles: Vec::new() };
    let mut cells: HashMap<usize, [u32; 12]> = HashMap::new();
    let mut active = Vec::new();
    let is_base = |i: &[usize; 3]| (0..3).all(|a| i[a] + 1 < grid.n[a]);
    let offsets: [usize; 8] = std::array::from_fn(|c| (c & 1) + (c >> 1 & 1) * grid.n[0] + (c >> 2) * grid.n[0] * grid.n[1]);
    for base in grid.candidates().filter(is_base) {
        let at = grid.index(base);
        let mut mask = 0u8;
        for (c, offset) in offsets.iter().enumerate() {
            mask |= u8::from(grid.values[at + offset] <= 0.0) << c;
        }
        if mask == 0 || mask == u8::MAX {
            continue;
        }
        let inside: [bool; 8] = std::array::from_fn(|c| mask >> c & 1 == 1);
        let corner = |c: usize| [base[0] + (c & 1), base[1] + (c >> 1 & 1), base[2] + (c >> 2)];
        let (id, count) = loops(inside);
        let mut sums = vec![(DVec3::ZERO, 0.0); count as usize];
        for (e, &(a, b)) in EDGES.iter().enumerate() {
            if id[e] == NO_LOOP {
                continue;
            }
            let (va, vb) = (grid.value(corner(a)) as f64, grid.value(corner(b)) as f64);
            let t = va / (va - vb);
            let p = grid.position(corner(a)).lerp(grid.position(corner(b)), t);
            let sum = &mut sums[id[e] as usize];
            *sum = (sum.0 + p, sum.1 + 1.0);
        }
        let first = surface.vertices.len() as u32;
        let center = grid.position(base) + DVec3::splat(0.5 * grid.step);
        surface.vertices.extend(sums.iter().map(|(p, n)| if blocky { center } else { *p / *n }));
        cells.insert(at, id.map(|l| if l == NO_LOOP { u32::MAX } else { first + l as u32 }));
        active.push(base);
    }
    for base in active {
        let from = grid.inside(base);
        for axis in 0..3 {
            let mut end = base;
            end[axis] += 1;
            if grid.inside(end) == from {
                continue;
            }
            let (u, v) = ACROSS[axis];
            // Counterclockwise about `u × v`, which is `+axis` but for y.
            let mut quad = [(1, 1), (0, 1), (0, 0), (1, 0)].map(|(du, dv)| {
                let mut cell = base;
                cell[u] -= du;
                cell[v] -= dv;
                cells[&grid.index(cell)][4 * axis + du + 2 * dv]
            });
            // Outward is `+axis` when the inside end is the base.
            if (axis == 1) == from {
                quad.reverse();
            }
            surface.quad(quad);
        }
    }
    surface
}

/// Normals from the field's gradient, by central differences a tenth of a cell wide. Where the
/// gradient vanishes, at a crease the field is flat across, the triangles' own.
pub(crate) fn gradients(field: &impl Field, surface: &Surface, step: f64) -> Vec<DVec3> {
    let h = 0.1 * step;
    let mut scratch = Vec::new();
    let mut normals: Vec<DVec3> = surface
        .vertices
        .iter()
        .map(|&p| {
            let mut d = |q: DVec3| field.distance(q, &mut scratch);
            let g = DVec3::from_array([0, 1, 2].map(|a| d(p + AXES[a] * h) - d(p - AXES[a] * h)));
            g.try_normalize().unwrap_or(DVec3::ZERO)
        })
        .collect();
    let mut faces = vec![DVec3::ZERO; normals.len()];
    for t in &surface.triangles {
        let [a, b, c] = t.map(|i| surface.vertices[i as usize]);
        let n = (b - a).cross(c - a);
        for &i in t {
            faces[i as usize] += n;
        }
    }
    // Or where it points against them: across a gap narrower than a cell, which the grid
    // bridged and the field did not.
    for (n, f) in normals.iter_mut().zip(faces) {
        if n.dot(f) <= 0.0 {
            *n = f.try_normalize().unwrap_or(DVec3::Z);
        }
    }
    normals
}

