//! A form's hull as a mesh: surface nets over `lc_world::form::sdf`, in the ship's frame in
//! meters, carrying the region weights and seam coordinates `em_render::hull_material` reads.
//! See `lightcone/docs/32-ship-rendering.md` §From distance field to mesh.
//!
//! [`mesh_form`] is the pure core, form and resolution in and buffers out. [`HullMeshPlugin`] runs
//! it on the async compute pool, caches the meshes by [`form_hash`], and swaps each into its
//! entity when it lands, so a remesh never holds up a frame.
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

use std::collections::{HashMap, HashSet, VecDeque};
use std::f64::consts::TAU;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use bevy_mesh::{Indices, PrimitiveTopology};
use em_render::hull_material::{ATTRIBUTE_HULL_SEAM, insert_region_weights, region_weights};
use glam::DVec3;
use lc_world::fitting::Balance;
use lc_world::form::primitive::Shape;
use lc_world::form::sdf::Sdf;
use lc_world::form::{Form, FormError, Kind, Mount, Part, PartId, Placement, Primitive, SparMode};

/// The palette's order: region `i` is drawn with `textures/hull/{REGION_GRAPHS[i]}.tgraph`.
pub const REGION_GRAPHS: [&str; 8] = ["storage", "drone", "living", "engine", "data", "mind", "spar", "bay"];

pub fn region(kind: Kind) -> u32 {
    match kind {
        Kind::Storage => 0,
        Kind::Drone => 1,
        Kind::Living => 2,
        Kind::Engine => 3,
        Kind::Data => 4,
        Kind::Mind => 5,
        Kind::Spar(_) => 6,
        Kind::Bay => 7,
    }
}

pub const MIN_CELLS: u32 = 16;
pub const MAX_CELLS: u32 = 256;

const PIXELS_PER_CELL: f32 = 4.0;

/// One cell per four pixels along the ship's longest side, as a power of two from 16 to 256.
/// `current` is kept until the ideal is more than three quarters of a doubling from it, so a
/// ship held near a boundary does not remesh back and forth.
pub fn cells_for(pixels: f32, current: Option<u32>) -> u32 {
    let ideal = (pixels / PIXELS_PER_CELL).max(1.0).log2();
    let (lo, hi) = (MIN_CELLS.ilog2() as f32, MAX_CELLS.ilog2() as f32);
    if let Some(c) = current
        && (ideal.clamp(lo, hi) - (c as f32).log2()).abs() <= 0.75
    {
        return c;
    }
    1 << ideal.round().clamp(lo, hi) as u32
}

/// Pixels spanned by something `extent_m` long at `distance_m` through a perspective camera
/// `fov_y` radians tall on a viewport `height_px` tall. Never more than it would span from
/// half its own length, where the camera would be inside it.
pub fn pixels_across(extent_m: f32, distance_m: f32, fov_y: f32, height_px: f32) -> f32 {
    let distance = distance_m.max(0.5 * extent_m).max(f32::MIN_POSITIVE);
    extent_m / (2.0 * distance * (0.5 * fov_y).tan()) * height_px
}

/// How a hull is drawn. Changes extraction, never the shape: every finish reads the same
/// cleaned samples.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Finish {
    /// Surface nets as they come, with normals from the field's gradient.
    #[default]
    Smooth,
    /// The same triangles with flat normals.
    Faceted,
    /// Each inside sample drawn as a cube a cell across.
    Blocky,
}

/// Mesh buffers in the ship's frame, meters.
#[derive(Clone, Debug, Default)]
pub struct HullBuffers {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub regions: Vec<[[u8; 4]; 4]>,
    /// [`ATTRIBUTE_HULL_SEAM`]'s `(across, along)`.
    pub seams: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    /// Meters between samples.
    pub step: f64,
}

impl HullBuffers {
    pub fn into_mesh(self) -> Mesh {
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        insert_region_weights(&mut mesh, &self.regions);
        mesh.insert_attribute(ATTRIBUTE_HULL_SEAM, self.seams);
        mesh.insert_indices(Indices::U32(self.indices));
        mesh
    }
}

/// `cells` along the longest side of the form's bounds.
pub fn mesh_form(form: &Form, balance: &Balance, cells: u32, finish: Finish) -> Result<HullBuffers, FormError> {
    let sdf = Sdf::new(form, balance)?;
    let grid = Grid::sample(&sdf, cells);
    let surface = nets(&grid, finish == Finish::Blocky);
    let paint = Paint::new(&sdf, form, balance, grid.step);
    let mut scratch = Vec::new();
    let mut painted: Vec<Painted> = surface.vertices.iter().map(|&p| paint.at(p, &mut scratch)).collect();
    along_seams(&mut painted);
    let normals = (finish == Finish::Smooth).then(|| gradients(&sdf, &surface, grid.step));
    Ok(buffers(&surface, &painted, normals.as_deref(), grid.step))
}

/// FNV-1a over a canonical encoding of the form, each part's solved shape, and what of
/// `balance` shapes it. Parts in id order, and `-0.0` as `0.0`, so two forms that mesh alike
/// hash alike.
pub fn form_hash(form: &Form, balance: &Balance) -> u64 {
    let mut h = Fnv::default();
    for x in [balance.min_part_m3, balance.spar_gap, balance.spar_thickness] {
        h.f64(x);
    }
    let mut parts: Vec<&Part> = form.parts.iter().collect();
    parts.sort_by_key(|p| p.id);
    for part in parts {
        h.u64(part.id.0 as u64);
        h.u64(match part.kind {
            Kind::Mind => 0,
            Kind::Storage => 1,
            Kind::Drone => 2,
            Kind::Engine => 3,
            Kind::Living => 4,
            Kind::Data => 5,
            Kind::Bay => 6,
            Kind::Spar(SparMode::Saddle) => 7,
            Kind::Spar(SparMode::Strap) => 8,
        });
        h.f64(part.volume_m3);
        // The solved shape covers the primitive's proportions and its scale.
        let shape = part.shape(balance.min_part_m3);
        let dimensions: Vec<f64> = match shape {
            Shape::Ellipsoid { semi_axes } => semi_axes.to_array().to_vec(),
            Shape::Capsule { radius, length } | Shape::Cylinder { radius, length } => vec![radius, length],
            Shape::Slab { edges, corner } => vec![edges.x, edges.y, edges.z, corner],
            Shape::Torus { major, minor } => vec![major, minor],
            Shape::Frustum { length, start, end } => vec![length, start, end],
        };
        for x in dimensions {
            h.f64(x);
        }
        match part.placement {
            None => h.u64(0),
            Some(Placement { parent, mount, twist, tilt, blend, mirror }) => {
                h.u64(1 + parent.0 as u64);
                match mount {
                    Mount::Enclosing => h.u64(0),
                    Mount::Attached { anchor, standoff } => {
                        h.u64(1);
                        for x in anchor.to_array() {
                            h.f64(x);
                        }
                        h.f64(standoff);
                    }
                }
                for x in [twist, tilt.x, tilt.y, blend] {
                    h.f64(x);
                }
                h.u64(mirror as u64);
            }
        }
    }
    h.0
}

fn mesh_key(form: u64, cells: u32, finish: Finish) -> u64 {
    let mut h = Fnv(form);
    h.u64(cells as u64);
    h.u64(finish as u64);
    h.0
}

struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn u64(&mut self, x: u64) {
        self.bytes(&x.to_le_bytes());
    }

    /// `x + 0.0` is `0.0` for either zero and `x` otherwise.
    fn f64(&mut self, x: f64) {
        self.u64((x + 0.0).to_bits());
    }
}

/// Samples of the field on a lattice whose outermost layer lies outside the bounds, so every
/// sample on it is outside and the surface is closed.
struct Grid {
    origin: DVec3,
    step: f64,
    n: [usize; 3],
    values: Vec<f32>,
    /// Blocks that may hold a sign change, as block coordinates.
    near: Vec<[usize; 3]>,
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

const AXES: [DVec3; 3] = [DVec3::X, DVec3::Y, DVec3::Z];

/// The two axes across each axis, in the order the cell's edge table indexes them.
const ACROSS: [(usize, usize); 3] = [(1, 2), (0, 2), (0, 1)];

impl Grid {
    fn sample(sdf: &Sdf, cells: u32) -> Grid {
        let (min, max) = sdf.bounds();
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
                    let d = sdf.distance_with(origin + middle * step, &mut scratch);
                    let far = d.abs() > reach;
                    if !far {
                        grid.near.push([bx, by, bz]);
                    }
                    for z in lo[2]..hi[2] {
                        for y in lo[1]..hi[1] {
                            for x in lo[0]..hi[0] {
                                let i = grid.index([x, y, z]);
                                let d = if far { d } else { sdf.distance_with(grid.position([x, y, z]), &mut scratch) };
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

    fn index(&self, i: [usize; 3]) -> usize {
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
    fn clean(&mut self) {
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
struct Surface {
    vertices: Vec<DVec3>,
    triangles: Vec<[u32; 3]>,
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
fn nets(grid: &Grid, blocky: bool) -> Surface {
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
fn gradients(sdf: &Sdf, surface: &Surface, step: f64) -> Vec<DVec3> {
    let h = 0.1 * step;
    let mut scratch = Vec::new();
    let mut normals: Vec<DVec3> = surface
        .vertices
        .iter()
        .map(|&p| {
            let mut d = |q: DVec3| sdf.distance_with(q, &mut scratch);
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

/// What a vertex carries to the material.
#[derive(Clone, Copy, Debug)]
struct Painted {
    regions: [[u8; 4]; 4],
    across: f32,
    along: f64,
    /// The spar whose seam `along` is measured about, the vertex's angle about its axis, and
    /// what `along` gains a radian, to unwrap the angle where a triangle straddles its cut.
    about: Option<(usize, f64, f64)>,
    seam: Option<SeamAt>,
}

/// Where a vertex is about the spar whose seam is nearest, in the spar's frame.
#[derive(Clone, Copy, Debug)]
struct SeamAt {
    spar: usize,
    neighbor: usize,
    theta: f64,
    rho: f64,
    x: f64,
    /// How far the seam runs around the axis and along it here, as the parts of a unit tangent.
    /// Only where the vertex is within reach of the seam.
    runs: Option<(f64, f64)>,
}

/// Classes each seam, a spar and the neighbor it meets, as a ring about the spar's axis or a
/// line along it, by which way it runs on the whole, and writes `along` as meters around or
/// along for all of it. Chosen per vertex it would shear the heads where the choice changes, and
/// weights that vary would add their own slope, times tens of meters, to `along`. A seam off its
/// class spreads its heads by the cosine of how far it strays.
///
/// A ring's radius is one for the seam, its vertices' mean: the angle reaches π, so a tenth of a
/// meter between two neighbors' own radii would put a third of a meter between their `along`s.
fn along_seams(painted: &mut [Painted]) {
    #[derive(Default)]
    struct Total {
        around: f64,
        axial: f64,
        rho: f64,
        n: f64,
    }
    let mut totals: HashMap<(usize, usize), Total> = HashMap::new();
    for seam in painted.iter().filter_map(|p| p.seam) {
        if let Some((around, axial)) = seam.runs {
            let total = totals.entry((seam.spar, seam.neighbor)).or_default();
            total.around += around;
            total.axial += axial;
            total.rho += seam.rho;
            total.n += 1.0;
        }
    }
    for paint in painted.iter_mut() {
        let Some(seam) = paint.seam else { continue };
        (paint.along, paint.about) = match totals.get(&(seam.spar, seam.neighbor)) {
            Some(t) if t.axial > t.around => (seam.x, None),
            Some(t) => {
                let rho = t.rho / t.n;
                (seam.theta * rho, Some((seam.spar, seam.theta, rho)))
            }
            None => (seam.theta * seam.rho, Some((seam.spar, seam.theta, seam.rho))),
        };
    }
}

/// Past this from every seam, `across` is written as [`SEAM_FAR`] with its sign kept.
const SEAM_REACH_M: f64 = 4.0;

/// Large enough that the shader's derivative of it spreads any bolt to nothing. Signed as the
/// vertex's side, so it never crosses zero away from a seam, where a row would be drawn.
const SEAM_FAR: f64 = 1.0e4;

/// Reads region weights and seams off the per-part distances.
struct Paint<'a> {
    sdf: &'a Sdf,
    region: Vec<u32>,
    /// Per piece, meters: the largest fillet at any of its joints.
    fillet: Vec<f64>,
    spars: Vec<usize>,
    step: f64,
}

impl<'a> Paint<'a> {
    fn new(sdf: &'a Sdf, form: &Form, balance: &Balance, step: f64) -> Self {
        let pieces = sdf.pieces();
        let parts: HashMap<PartId, &Part> = form.parts.iter().map(|p| (p.id, p)).collect();
        let least = |id: PartId| parts[&id].shape(balance.min_part_m3).least_dimension();
        let is_spar = |id: PartId| matches!(parts[&id].kind, Kind::Spar(_));
        let mut by_part: HashMap<PartId, f64> = HashMap::new();
        for part in &form.parts {
            let Some(placement) = part.placement else { continue };
            if is_spar(part.id) || is_spar(placement.parent) || placement.blend <= 0.0 {
                continue;
            }
            // As `Sdf::new` sizes a blend.
            let radius = placement.blend * least(part.id).min(least(placement.parent));
            for id in [part.id, placement.parent] {
                let r = by_part.entry(id).or_insert(0.0);
                *r = r.max(radius);
            }
        }
        Paint {
            sdf,
            region: pieces.iter().map(|p| region(p.kind)).collect(),
            fillet: pieces.iter().map(|p| by_part.get(&p.part).copied().unwrap_or(0.0)).collect(),
            spars: (0..pieces.len()).filter(|&i| matches!(pieces[i].kind, Kind::Spar(_))).collect(),
            step,
        }
    }

    fn at(&self, p: DVec3, distances: &mut Vec<f64>) -> Painted {
        distances.clear();
        distances.extend((0..self.region.len()).map(|i| self.sdf.piece_distance(i, p)));
        let regions = self.regions(distances);
        let Some((spar, (neighbor, off))) =
            self.spars.iter().filter_map(|&s| Some((s, self.sdf.seam(s, p)?))).min_by(|a, b| a.1.1.total_cmp(&b.1.1))
        else {
            return Painted { regions, across: SEAM_FAR as f32, along: 0.0, about: None, seam: None };
        };
        let on_spar = distances[spar] <= distances[neighbor];
        let (mine, theirs) = if on_spar { (spar, neighbor) } else { (neighbor, spar) };
        // Positive on the lower-numbered region's side; between two spars, the nearer spar's.
        let positive = self.region[mine] < self.region[theirs] || (self.region[mine] == self.region[theirs] && on_spar);
        let sign = if positive { 1.0 } else { -1.0 };
        let near = off <= SEAM_REACH_M.max(2.0 * self.step);
        let across = sign * if near { off } else { SEAM_FAR };
        let pose = self.sdf.pieces()[spar].pose;
        let local = pose.to_local(p);
        let (theta, rho) = (local.z.atan2(local.y), local.y.hypot(local.z));
        let h = 1e-3 * self.step;
        let grad = |i: usize| {
            DVec3::from_array([0, 1, 2].map(|a| self.sdf.primitive(i, p + AXES[a] * h) - self.sdf.primitive(i, p - AXES[a] * h)))
        };
        let runs = near
            .then(|| (pose.rotation.transpose() * grad(spar).cross(grad(neighbor))).try_normalize())
            .flatten()
            .filter(|_| rho > 0.0)
            .map(|t| (t.dot(DVec3::new(0.0, -local.z, local.y) / rho).abs(), t.x.abs()));
        let seam = SeamAt { spar, neighbor, theta, rho, x: local.x, runs };
        Painted { regions, across: across as f32, along: 0.0, about: None, seam: Some(seam) }
    }

    /// The nearest region and the next, blended across the fillet between them, or across a
    /// cell where they meet hard so the edge is not a stair of whole triangles.
    fn regions(&self, distances: &[f64]) -> [[u8; 4]; 4] {
        let mut best = [(f64::INFINITY, usize::MAX); REGION_GRAPHS.len()];
        for (i, &d) in distances.iter().enumerate() {
            let slot = &mut best[self.region[i] as usize];
            if d < slot.0 {
                *slot = (d, i);
            }
        }
        let mut order: Vec<usize> = (0..best.len()).filter(|&r| best[r].1 != usize::MAX).collect();
        order.sort_by(|&a, &b| best[a].0.total_cmp(&best[b].0));
        let first = order[0];
        let Some(&second) = order.get(1) else { return region_weights(first as u32, first as u32, 0.0) };
        let (d1, i1) = best[first];
        let (d2, i2) = best[second];
        let width = self.step.max(self.fillet[i1].min(self.fillet[i2]));
        let x = ((d2 - d1) / width).clamp(0.0, 1.0);
        let share = 0.5 * (1.0 - x * x * (3.0 - 2.0 * x));
        region_weights(first as u32, second as u32, share as f32)
    }
}

/// Lays the surface out as buffers. Shared vertices stay shared where `normals` is given;
/// otherwise each triangle gets its own three with its flat normal. A triangle straddling the
/// cut of a seam's angle takes copies of its corners on one side with a turn added, so `along`
/// never runs backwards across one triangle.
fn buffers(surface: &Surface, painted: &[Painted], normals: Option<&[DVec3]>, step: f64) -> HullBuffers {
    let mut out = HullBuffers { step, ..default() };
    let mut shared: HashMap<(u32, bool), u32> = HashMap::new();
    for t in &surface.triangles {
        let about = t.map(|i| painted[i as usize].about);
        let unwrap = match about {
            [Some((s, a, _)), Some((s2, b, _)), Some((s3, c, _))] if s == s2 && s == s3 => {
                a.max(b).max(c) - a.min(b).min(c) > std::f64::consts::PI
            }
            _ => false,
        };
        let [a, b, c] = t.map(|i| surface.vertices[i as usize]);
        let flat = (b - a).cross(c - a).normalize_or_zero();
        for &v in t {
            let paint = painted[v as usize];
            let wrapped = unwrap && paint.about.is_some_and(|(_, theta, _)| theta < 0.0);
            let push = |out: &mut HullBuffers| {
                let along = paint.along + if wrapped { TAU * paint.about.map_or(0.0, |a| a.2) } else { 0.0 };
                out.positions.push(surface.vertices[v as usize].as_vec3().to_array());
                out.normals.push(normals.map_or(flat, |n| n[v as usize]).as_vec3().to_array());
                out.regions.push(paint.regions);
                out.seams.push([paint.across, along as f32]);
                out.positions.len() as u32 - 1
            };
            let index = match normals {
                Some(_) => match shared.get(&(v, wrapped)) {
                    Some(&i) => i,
                    None => {
                        let i = push(&mut out);
                        shared.insert((v, wrapped), i);
                        i
                    }
                },
                None => push(&mut out),
            };
            out.indices.push(index);
        }
    }
    out
}

/// A storage sphere with one spar. [`SparMode::Saddle`]: a boom off it with an engine ball on
/// its end, embedded at both joints so there is something to cut. [`SparMode::Strap`]: a band
/// around its equator, a torus whose tube straddles the surface. For tests and the void.
pub fn spar_fixture(mode: SparMode) -> Form {
    let balance = Balance::DEFAULT;
    let hang = |id: u16, kind: Kind, primitive: Primitive, volume_m3: f64, parent: u16, mount: Mount| {
        let placement = Placement { parent: PartId(parent), mount, twist: 0.0, tilt: glam::DVec2::ZERO, blend: 0.0, mirror: false };
        Part { id: PartId(id), kind, primitive, volume_m3, placement: Some(placement) }
    };
    let sphere = Primitive::Capsule { length: 0.0 };
    let tank = hang(1, Kind::Storage, sphere, 5.0e5, 0, Mount::Enclosing);
    let spar = match mode {
        SparMode::Saddle => {
            let anchor = Mount::Attached { anchor: DVec3::new(0.3, 1.0, 0.2), standoff: -0.3 };
            hang(2, Kind::Spar(mode), Primitive::Cylinder { length: 8.0 }, 2.0e4, 1, anchor)
        }
        SparMode::Strap => {
            let r = tank.shape(balance.min_part_m3).reach();
            let band = Primitive::Torus { major: 7.0 };
            hang(2, Kind::Spar(mode), band, band.volume(r / 7.0), 1, Mount::Enclosing)
        }
    };
    let mut parts = vec![Part::mind(PartId(0), balance.min_part_m3), tank, spar];
    if mode == SparMode::Saddle {
        parts.push(hang(3, Kind::Engine, sphere, 1.0e4, 2, Mount::Attached { anchor: DVec3::X, standoff: -0.5 }));
    }
    Form { parts }
}

/// What an entity draws as its hull. A mesh arrives as its `Mesh3d` once meshed; give it a
/// `MeshMaterial3d<HullMaterial>` yourself. Its grid is sized by [`measure`].
#[derive(Component, Clone)]
#[require(HullPixels, HullMeshState)]
pub struct HullForm {
    pub form: Arc<Form>,
    pub balance: Balance,
    pub finish: Finish,
    /// A fixed resolution instead of one from [`HullPixels`].
    pub cells: Option<u32>,
}

/// Pixels the hull spans on screen along its longest side, written by [`measure`].
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct HullPixels(pub f32);

#[derive(Component, Debug, Default)]
pub struct HullMeshState {
    form: u64,
    extent_m: f64,
    cells: Option<u32>,
    wanted: Option<u64>,
    shown: Option<u64>,
}

impl HullMeshState {
    /// The longest side of the form's bounds, for [`pixels_across`].
    pub fn extent_m(&self) -> f64 {
        self.extent_m
    }

    pub fn cells(&self) -> Option<u32> {
        self.cells
    }

    /// Whether what is drawn is what is wanted.
    pub fn current(&self) -> bool {
        self.wanted.is_some() && self.wanted == self.shown
    }
}

/// Meshes kept after nothing wants them, so zooming back is free.
const CACHED: usize = 32;

#[derive(Resource, Default)]
pub struct HullMeshes {
    ready: HashMap<u64, Option<Handle<Mesh>>>,
    order: VecDeque<u64>,
    pending: HashMap<u64, Task<Option<Mesh>>>,
}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub struct HullMeshSystems;

pub struct HullMeshPlugin;

impl Plugin for HullMeshPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HullMeshes>().add_systems(Update, (measure, want, land).chain().in_set(HullMeshSystems));
    }
}

/// Each hull's pixels on screen, the most it spans through any active perspective camera, so it
/// is meshed for its nearest view. A frame behind, as the transforms are.
pub fn measure(
    cameras: Query<(&Camera, &Projection, &GlobalTransform)>,
    mut hulls: Query<(&GlobalTransform, &HullMeshState, &mut HullPixels)>,
) {
    for (at, state, mut pixels) in &mut hulls {
        pixels.0 = cameras
            .iter()
            .filter(|(camera, ..)| camera.is_active)
            .filter_map(|(camera, projection, eye)| {
                let Projection::Perspective(perspective) = projection else { return None };
                let height = camera.physical_viewport_size()?.y as f32;
                let distance = eye.translation().distance(at.translation());
                Some(pixels_across(state.extent_m as f32, distance, perspective.fov, height))
            })
            .fold(0.0, f32::max);
    }
}

fn want(mut meshes: ResMut<HullMeshes>, mut hulls: Query<(Ref<HullForm>, &HullPixels, &mut HullMeshState)>) {
    for (hull, pixels, mut state) in &mut hulls {
        if hull.is_changed() {
            state.form = form_hash(&hull.form, &hull.balance);
            state.extent_m = match Sdf::new(&hull.form, &hull.balance) {
                Ok(sdf) => {
                    let (min, max) = sdf.bounds();
                    (max - min).max_element()
                }
                Err(_) => 0.0,
            };
        }
        let cells = hull.cells.unwrap_or_else(|| cells_for(pixels.0, state.cells));
        let key = mesh_key(state.form, cells, hull.finish);
        if state.wanted == Some(key) {
            continue;
        }
        state.cells = Some(cells);
        state.wanted = Some(key);
        if meshes.ready.contains_key(&key) || meshes.pending.contains_key(&key) {
            continue;
        }
        let (form, balance, finish) = (hull.form.clone(), hull.balance, hull.finish);
        let task = AsyncComputeTaskPool::get().spawn(async move {
            let started = bevy::platform::time::Instant::now();
            match mesh_form(&form, &balance, cells, finish) {
                Ok(buffers) => {
                    let (vertices, ms) = (buffers.positions.len(), started.elapsed().as_secs_f64() * 1e3);
                    debug!("hull_mesh: {cells} cells {finish:?}, {vertices} vertices in {ms:.0} ms");
                    Some(buffers.into_mesh())
                }
                Err(error) => {
                    warn!("hull_mesh: {error}");
                    None
                }
            }
        });
        meshes.pending.insert(key, task);
    }
}

fn land(
    mut commands: Commands,
    mut meshes: ResMut<HullMeshes>,
    mut assets: ResMut<Assets<Mesh>>,
    mut hulls: Query<(Entity, &mut HullMeshState, Option<&mut Mesh3d>)>,
) {
    let meshes = &mut *meshes;
    let mut landed = Vec::new();
    meshes.pending.retain(|&key, task| match check_ready(task) {
        Some(mesh) => {
            landed.push((key, mesh));
            false
        }
        None => true,
    });
    for (key, mesh) in landed {
        meshes.ready.insert(key, mesh.map(|m| assets.add(m)));
        meshes.order.push_back(key);
    }
    for (entity, mut state, mesh) in &mut hulls {
        let Some(wanted) = state.wanted else { continue };
        if state.shown == Some(wanted) {
            continue;
        }
        let Some(ready) = meshes.ready.get(&wanted) else { continue };
        state.shown = Some(wanted);
        let Some(handle) = ready.clone() else { continue };
        match mesh {
            Some(mut mesh) => mesh.0 = handle,
            None => {
                commands.entity(entity).insert(Mesh3d(handle));
            }
        }
    }
    // After the handing out, and never one still wanted: `want` asks for a key only once.
    let wanted: HashSet<u64> = hulls.iter().filter_map(|(_, state, _)| state.wanted).collect();
    let mut excess = meshes.order.len().saturating_sub(CACHED);
    let ready = &mut meshes.ready;
    meshes.order.retain(|key| {
        if excess == 0 || wanted.contains(key) {
            return true;
        }
        ready.remove(key);
        excess -= 1;
        false
    });
}

#[cfg(test)]
mod tests {
    use lc_world::form::presets::Builtin;

    use super::*;

    const B: Balance = Balance::DEFAULT;

    fn surface(form: &Form, cells: u32, finish: Finish) -> (Surface, Sdf, f64) {
        let sdf = Sdf::new(form, &B).unwrap();
        let grid = Grid::sample(&sdf, cells);
        let step = grid.step;
        let surface = nets(&grid, finish == Finish::Blocky);
        (surface, sdf, step)
    }

    /// Closed, manifold and consistently wound: every edge is used once in each direction.
    /// Returns the Euler characteristic and the number of connected pieces.
    fn check_closed(s: &Surface, what: &str) -> (i64, i64) {
        assert!(!s.triangles.is_empty(), "{what}: empty");
        let mut directed: HashMap<(u32, u32), u32> = HashMap::new();
        for t in &s.triangles {
            assert!(t[0] != t[1] && t[1] != t[2] && t[0] != t[2], "{what}: degenerate {t:?}");
            for k in 0..3 {
                *directed.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        for (&(a, b), &n) in &directed {
            assert_eq!(n, 1, "{what}: edge {a}->{b} used {n} times");
            assert_eq!(directed.get(&(b, a)), Some(&1), "{what}: edge {a}-{b} has one face");
        }
        // Around each vertex, its triangles form one fan: two cones meeting at a point do not.
        let mut fans: HashMap<u32, HashMap<u32, u32>> = HashMap::new();
        for t in &s.triangles {
            for k in 0..3 {
                fans.entry(t[k]).or_default().insert(t[(k + 1) % 3], t[(k + 2) % 3]);
            }
        }
        for (v, fan) in &fans {
            let start = *fan.keys().next().unwrap();
            let (mut at, mut steps) = (start, 0);
            loop {
                at = fan[&at];
                steps += 1;
                if at == start {
                    break;
                }
            }
            assert_eq!(steps, fan.len(), "{what}: vertex {v} is where {} fans meet", fan.len() as f64 / steps as f64);
        }
        let used: HashSet<u32> = s.triangles.iter().flatten().copied().collect();
        let (v, e, f) = (used.len() as i64, directed.len() as i64 / 2, s.triangles.len() as i64);

        let mut parent: HashMap<u32, u32> = used.iter().map(|&i| (i, i)).collect();
        fn root(parent: &mut HashMap<u32, u32>, mut i: u32) -> u32 {
            while parent[&i] != i {
                let up = parent[&parent[&i]];
                parent.insert(i, up);
                i = up;
            }
            i
        }
        for t in &s.triangles {
            for k in 1..3 {
                let (a, b) = (root(&mut parent, t[0]), root(&mut parent, t[k]));
                parent.insert(a, b);
            }
        }
        let pieces = used.iter().filter(|&&i| root(&mut parent, i) == i).count() as i64;
        (v - e + f, pieces)
    }

    fn volume(s: &Surface) -> f64 {
        s.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| s.vertices[i as usize]);
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    fn mind() -> Part {
        Part::mind(PartId(0), B.min_part_m3)
    }

    fn alone(primitive: Primitive, volume_m3: f64) -> Form {
        let placement =
            Placement { parent: PartId(0), mount: Mount::Enclosing, twist: 0.0, tilt: glam::DVec2::ZERO, blend: 0.0, mirror: false };
        let part = Part { id: PartId(1), kind: Kind::Storage, primitive, volume_m3, placement: Some(placement) };
        Form { parts: vec![mind(), part] }
    }

    /// Each primitive around the Mind, and how many handles it has.
    fn primitives() -> Vec<(Primitive, i64)> {
        vec![
            (Primitive::Ellipsoid { axes: DVec3::new(3.0, 1.0, 2.0) }, 0),
            (Primitive::Capsule { length: 2.0 }, 0),
            (Primitive::Capsule { length: 0.0 }, 0),
            (Primitive::Slab { edges: DVec3::new(2.0, 1.0, 3.0), corner: 0.2 }, 0),
            (Primitive::Slab { edges: DVec3::new(2.0, 1.0, 3.0), corner: 0.0 }, 0),
            (Primitive::Cylinder { length: 3.0 }, 0),
            (Primitive::Torus { major: 3.0 }, 1),
            (Primitive::Frustum { length: 2.0, taper: 0.5 }, 0),
            (Primitive::Frustum { length: 2.0, taper: 0.0 }, 0),
            (Primitive::Frustum { length: 2.0, taper: 2.5 }, 0),
        ]
    }

    const RESOLUTIONS: [u32; 3] = [16, 40, 96];

    /// Faceted is Smooth's triangles with other normals, so two topologies cover the three finishes.
    #[test]
    fn every_primitive_is_closed_at_three_resolutions_in_both_topologies() {
        for (primitive, handles) in primitives() {
            let form = alone(primitive, 5.0e5);
            let truth = form.parts[1].shape(B.min_part_m3).volume();
            for cells in RESOLUTIONS {
                for finish in [Finish::Smooth, Finish::Blocky] {
                    let what = format!("{primitive:?} at {cells} {finish:?}");
                    let (s, sdf, step) = surface(&form, cells, finish);
                    let (chi, pieces) = check_closed(&s, &what);
                    // Pieces that are all spheres but one torus: no holes, and none made.
                    assert_eq!(chi, 2 * pieces - 2 * handles, "{what}: χ {chi} over {pieces} pieces");
                    let v = volume(&s);
                    assert!(v > 0.0, "{what}: wound inward, {v}");
                    if cells == 96 && finish == Finish::Smooth {
                        // The Mind is inside everything but the torus, whose hole it sits in.
                        let mind = if handles == 1 { B.min_part_m3 } else { 0.0 };
                        assert!((v / (truth + mind) - 1.0).abs() < 0.03, "{what}: {v} against {truth}");
                    }
                    if finish == Finish::Smooth {
                        let bound = 2.0 * 3f64.sqrt() * step;
                        for &p in &s.vertices {
                            assert!(sdf.distance(p).abs() <= bound, "{what}: {p} is {} off", sdf.distance(p));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn spar_saddle_and_strap_are_closed_at_three_resolutions() {
        for mode in [SparMode::Saddle, SparMode::Strap] {
            let form = spar_fixture(mode);
            for cells in RESOLUTIONS {
                for finish in [Finish::Smooth, Finish::Blocky] {
                    let what = format!("{mode:?} at {cells} {finish:?}");
                    let (s, _, _) = surface(&form, cells, finish);
                    let (chi, pieces) = check_closed(&s, &what);
                    assert_eq!(chi, 2 * pieces, "{what}: χ {chi} over {pieces} pieces");
                }
            }
        }
    }

    /// A strap about two meters deep on a grid of about seven: kept where a sample lands in it
    /// and nowhere else, and closed either way.
    #[test]
    fn a_strap_thinner_than_a_cell_is_holed_not_broken() {
        let form = spar_fixture(SparMode::Strap);
        let r = form.parts[1].shape(B.min_part_m3).reach();
        let depth = B.spar_thickness * 2.0 * r;
        for finish in [Finish::Smooth, Finish::Blocky] {
            let (s, _, step) = surface(&form, 16, finish);
            assert!(step > 3.0 * depth, "{step} against {depth}");
            let (chi, pieces) = check_closed(&s, "coarse strap");
            assert_eq!(chi, 2 * pieces);
        }
    }

    #[test]
    fn the_presets_are_closed() {
        let forms = [Form::starting()].into_iter().chain(Builtin::ALL.map(Builtin::form));
        for (k, form) in forms.enumerate() {
            for finish in [Finish::Smooth, Finish::Blocky] {
                let (s, ..) = surface(&form, 64, finish);
                check_closed(&s, &format!("preset {k} {finish:?}"));
            }
        }
    }

    /// Fifty kilometers across, with positions in `f32`: a millimeter or two at the rim.
    #[test]
    fn a_fifty_kilometer_hull_is_closed_and_placed_to_the_centimeter() {
        let radius: f64 = 25_000.0;
        let form = alone(Primitive::Capsule { length: 0.0 }, 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3));
        let (s, sdf, step) = surface(&form, 64, Finish::Smooth);
        let (chi, pieces) = check_closed(&s, "50 km");
        assert_eq!((chi, pieces), (2, 1));
        for &p in &s.vertices {
            let rounded = p.as_vec3().as_dvec3();
            assert!((rounded - p).length() < 0.01, "{p}");
            assert!(sdf.distance(rounded).abs() < step, "{p}");
        }
    }

    #[test]
    fn buffers_keep_the_surface_and_carry_every_attribute() {
        let form = Builtin::Cluster.form();
        for finish in [Finish::Smooth, Finish::Faceted, Finish::Blocky] {
            let b = mesh_form(&form, &B, 32, finish).unwrap();
            let n = b.positions.len();
            assert!(n > 0 && b.indices.len() % 3 == 0);
            assert_eq!((b.normals.len(), b.regions.len(), b.seams.len()), (n, n, n));
            assert!(b.indices.iter().all(|&i| (i as usize) < n));
            // Flat normals are their face's; a gradient disagrees with its face only at a crease.
            let against = b
                .indices
                .chunks(3)
                .filter(|t| {
                    let [p, q, r] = [0, 1, 2].map(|k| Vec3::from(b.positions[t[k] as usize]));
                    let face = (q - p).cross(r - p);
                    t.iter().any(|&i| face.dot(Vec3::from(b.normals[i as usize])) < -1e-3 * face.length())
                })
                .count();
            let allowed = if finish == Finish::Smooth { b.indices.len() / 3 / 100 } else { 0 };
            assert!(against <= allowed, "{finish:?}: {against} triangles face against their normals");
        }
    }

    #[test]
    fn regions_follow_the_nearest_part_and_seams_ring_the_spars() {
        let form = spar_fixture(SparMode::Saddle);
        let sdf = Sdf::new(&form, &B).unwrap();
        let b = mesh_form(&form, &B, 128, Finish::Smooth).unwrap();
        let mut near_seam = 0;
        for k in 0..b.positions.len() {
            let p = Vec3::from(b.positions[k]).as_dvec3();
            let (piece, _) = sdf.nearest(p);
            let weights = b.regions[k];
            let heaviest = (0..16).max_by_key(|&r| weights[r / 4][r % 4]).unwrap();
            let own = weights[heaviest / 4][heaviest % 4];
            // In a fillet two regions weigh about the same; outside one the nearest is heaviest.
            if own > 200 {
                assert_eq!(heaviest as u32, region(piece.kind), "at {p}");
            }
            let [across, _] = b.seams[k];
            if across.abs() < 1.0 {
                near_seam += 1;
            }
            assert!(across.abs() <= SEAM_REACH_M.max(2.0 * b.step) as f32 || across.abs() == SEAM_FAR as f32);
        }
        assert!(near_seam > 20, "{near_seam}");
    }

    #[test]
    fn a_seams_along_does_not_run_backwards_across_a_triangle() {
        let form = spar_fixture(SparMode::Saddle);
        let b = mesh_form(&form, &B, 128, Finish::Smooth).unwrap();
        let spar = Sdf::new(&form, &B).unwrap().pieces()[2];
        let rho = match spar.shape {
            lc_world::form::primitive::Shape::Cylinder { radius, .. } => radius,
            other => panic!("{other:?}"),
        };
        for t in b.indices.chunks(3) {
            let seams = [0, 1, 2].map(|k| b.seams[t[k] as usize]);
            if seams.iter().all(|s| s[0].abs() < 1.0) {
                let along = seams.map(|s| s[1]);
                let spread = along.iter().copied().fold(f32::MIN, f32::max) - along.iter().copied().fold(f32::MAX, f32::min);
                assert!((spread as f64) < rho, "a triangle spans {spread} m of seam");
            }
        }
    }

    #[test]
    fn a_sphere_and_a_strap_are_closed_at_the_finest_grid() {
        for form in [alone(Primitive::Capsule { length: 0.0 }, 5.0e5), spar_fixture(SparMode::Strap)] {
            let (s, ..) = surface(&form, MAX_CELLS, Finish::Smooth);
            let (chi, pieces) = check_closed(&s, "at MAX_CELLS");
            assert_eq!(chi, 2 * pieces);
        }
    }

    /// The saddle's boom tilted, so its end meets the hull obliquely and each seam climbs on one
    /// side of the boom and dips on the other.
    fn tilted_boom() -> Form {
        let mut form = spar_fixture(SparMode::Saddle);
        form.parts[2].placement.as_mut().unwrap().tilt = glam::DVec2::new(0.45, 0.3);
        form
    }

    /// Along a seam, `along` advances a meter a meter whichever way the seam runs.
    #[test]
    fn along_is_meters_along_a_seam_that_climbs() {
        let form = tilted_boom();
        let sdf = Sdf::new(&form, &B).unwrap();
        let b = mesh_form(&form, &B, 128, Finish::Smooth).unwrap();
        let spar = 2;
        let h = 1e-4;
        let grad = |i: usize, p: DVec3| DVec3::from_array([0, 1, 2].map(|a| sdf.primitive(i, p + AXES[a] * h) - sdf.primitive(i, p - AXES[a] * h)));
        let mut checked = 0;
        for t in b.indices.chunks(3) {
            for k in 0..3 {
                let (i, j) = (t[k] as usize, t[(k + 1) % 3] as usize);
                if b.seams[i][0].abs() > 1.5 || b.seams[j][0].abs() > 1.5 {
                    continue;
                }
                let (p, q) = (Vec3::from(b.positions[i]).as_dvec3(), Vec3::from(b.positions[j]).as_dvec3());
                let mid = (p + q) / 2.0;
                let Some((neighbor, _)) = sdf.seam(spar, mid) else { continue };
                let Some(tangent) = grad(spar, mid).cross(grad(neighbor, mid)).try_normalize() else { continue };
                let edge = q - p;
                if edge.normalize().dot(tangent).abs() < 0.95 {
                    continue;
                }
                let ratio = ((b.seams[j][1] - b.seams[i][1]) as f64).abs() / edge.length();
                assert!((0.7..=1.3).contains(&ratio), "{ratio} m of along a meter at {mid}");
                checked += 1;
            }
        }
        assert!(checked > 50, "{checked}");
    }

    fn headless() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default())).init_asset::<Mesh>().add_plugins(HullMeshPlugin);
        app
    }

    fn step_until(app: &mut App, what: &str, done: impl Fn(&mut World) -> bool) {
        for _ in 0..20_000 {
            app.update();
            if done(app.world_mut()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        panic!("never {what}");
    }

    fn hull(form: Form, cells: u32) -> HullForm {
        HullForm { form: Arc::new(form), balance: B, finish: Finish::Smooth, cells: Some(cells) }
    }

    fn shown(world: &World, e: Entity) -> Option<Handle<Mesh>> {
        world.get::<Mesh3d>(e).map(|m| m.0.clone())
    }

    /// The old mesh stays up through the frame that asks for a new one and every frame until it
    /// lands, and the new one is swapped in once it has.
    #[test]
    fn a_remesh_never_holds_up_a_frame() {
        let mut app = headless();
        let e = app.world_mut().spawn(hull(Builtin::Cluster.form(), 16)).id();
        step_until(&mut app, "meshed", |w| w.get::<HullMeshState>(e).unwrap().current());
        let old = shown(app.world(), e).expect("a mesh once current");

        app.world_mut().get_mut::<HullForm>(e).unwrap().cells = Some(128);
        app.update();
        assert_eq!(shown(app.world(), e), Some(old.clone()), "the frame that asked still draws the old mesh");
        assert!(!app.world().get::<HullMeshState>(e).unwrap().current());
        assert_eq!(app.world().resource::<HullMeshes>().pending.len(), 1, "meshing elsewhere");

        let mut frames = 0;
        loop {
            let pending = !app.world().resource::<HullMeshes>().pending.is_empty();
            app.update();
            frames += 1;
            let now = shown(app.world(), e);
            if now != Some(old.clone()) {
                assert!(pending || frames == 1, "swapped before the task finished");
                assert!(app.world().resource::<HullMeshes>().pending.is_empty());
                assert!(app.world().get::<HullMeshState>(e).unwrap().current());
                break;
            }
            assert!(frames < 20_000, "the new mesh never landed");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(frames > 1, "landed on the first frame after asking: not meshed off the frame");
    }

    /// More meshes landing in one frame than the cache keeps: every hull still gets its own.
    #[test]
    fn a_crowd_landing_at_once_is_all_drawn() {
        let mut app = headless();
        let crowd: Vec<Entity> = (0..CACHED + 1)
            .map(|k| {
                let form = alone(Primitive::Capsule { length: 0.0 }, 5.0e5 * (1.0 + k as f64 / 100.0));
                app.world_mut().spawn(hull(form, 16)).id()
            })
            .collect();
        app.update();
        let tasks = |w: &mut World| w.resource::<HullMeshes>().pending.values().all(|t| t.is_finished());
        while !tasks(app.world_mut()) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        app.update();
        app.update();
        for e in crowd {
            assert!(shown(app.world(), e).is_some(), "a hull left without its mesh");
            assert!(app.world().get::<HullMeshState>(e).unwrap().current());
        }
    }

    #[test]
    fn a_hash_names_the_shape() {
        let a = Form::starting();
        let mut b = a.clone();
        b.parts.reverse();
        assert_eq!(form_hash(&a, &B), form_hash(&b, &B), "the order parts are listed in");
        let mut c = a.clone();
        c.parts[2].volume_m3 *= 1.0 + 1e-12;
        assert_ne!(form_hash(&a, &B), form_hash(&c, &B), "a part's size");
        let mut d = a.clone();
        d.parts[4].placement.as_mut().unwrap().twist = -0.0;
        assert_eq!(form_hash(&a, &B), form_hash(&d, &B), "the sign of a zero");
        let gap = Balance { spar_gap: 1.0, ..B };
        assert_ne!(form_hash(&a, &B), form_hash(&a, &gap));
    }

    #[test]
    fn cells_track_pixels_in_doublings_and_hold_near_a_boundary() {
        assert_eq!(cells_for(10.0, None), MIN_CELLS);
        assert_eq!(cells_for(10_000.0, None), MAX_CELLS);
        assert_eq!(cells_for(256.0, None), 64);
        assert_eq!(cells_for(256.0 * 1.6, Some(64)), 64);
        assert_eq!(cells_for(256.0 * 1.8, Some(64)), 128);
        assert_eq!(cells_for(10_000.0, Some(MAX_CELLS)), MAX_CELLS);
        assert!(pixels_across(500.0, 1000.0, 1.0, 720.0) > pixels_across(500.0, 2000.0, 1.0, 720.0));
    }

    /// Break the cleaning on purpose: without it, alternating squares survive and the mesh has
    /// edges with four faces.
    #[test]
    fn without_cleaning_a_checkerboard_is_not_manifold() {
        let mut grid = Grid { origin: DVec3::ZERO, step: 1.0, n: [4, 4, 4], values: vec![1.0; 64], near: vec![[0, 0, 0]] };
        for i in [[1, 1, 1], [2, 2, 1], [1, 1, 2], [2, 2, 2]] {
            let k = grid.index(i);
            grid.values[k] = -1.0;
        }
        let dirty = std::panic::catch_unwind(|| {
            let s = nets(&grid, false);
            check_closed(&s, "dirty");
        });
        assert!(dirty.is_err(), "an uncleaned checkerboard came out closed");
        grid.clean();
        let s = nets(&grid, false);
        assert_eq!(check_closed(&s, "clean"), (2, 1));
        let s = nets(&grid, true);
        assert_eq!(check_closed(&s, "clean voxels"), (2, 1));
    }
}
