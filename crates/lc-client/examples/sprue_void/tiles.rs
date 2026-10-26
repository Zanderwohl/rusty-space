//! The three layers' tiles, generated on the CPU: a plated base, and two mazes of pipe.
//!
//! Each layer comes in edge-matched variants: what crosses a tile edge is that edge's choice and
//! stamped by both tiles beside it, so every shape is clipped to the tile, never wrapped, and
//! each maze grows inside its tile. A texel is linear RGBA8: the normal's x and y in the face's
//! own frame, albedo, and coverage, which is 1 everywhere on the base.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::Vec2;
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

/// splitmix64: enough for layout, and no dependency.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

/// One layer's surface, meters.
pub struct Field {
    texels: usize,
    tile_m: f32,
    height: Vec<f32>,
    albedo: Vec<f32>,
    /// Signed meters inside the nearest shape's edge; negative outside every shape.
    inside: Vec<f32>,
}

impl Field {
    pub fn new(texels: usize, tile_m: f32, floor_albedo: f32) -> Self {
        let n = texels * texels;
        Field { texels, tile_m, height: vec![0.0; n], albedo: vec![floor_albedo; n], inside: vec![-1.0e3; n] }
    }

    fn texel_m(&self) -> f32 {
        self.tile_m / self.texels as f32
    }

    /// Keeps the taller of what `shape` returns and what is there. Clipped, not wrapped: what
    /// crosses a tile edge is stamped by both tiles beside it.
    fn stamp(&mut self, lo: Vec2, hi: Vec2, mut shape: impl FnMut(Vec2) -> Option<(f32, f32, f32)>) {
        let t = self.texel_m();
        let n = self.texels as i64;
        let (x0, y0) = ((lo.x / t).floor() as i64 - 1, (lo.y / t).floor() as i64 - 1);
        let (x1, y1) = ((hi.x / t).ceil() as i64 + 1, (hi.y / t).ceil() as i64 + 1);
        for y in y0.max(0)..=y1.min(n - 1) {
            for x in x0.max(0)..=x1.min(n - 1) {
                let p = Vec2::new((x as f32 + 0.5) * t, (y as f32 + 0.5) * t);
                let Some((height, albedo, inside)) = shape(p) else { continue };
                let i = (y * n + x) as usize;
                if inside > -t && height >= self.height[i] {
                    self.height[i] = height;
                    self.albedo[i] = albedo;
                }
                self.inside[i] = self.inside[i].max(inside);
            }
        }
    }

    /// Collared every `collar_m`.
    pub fn pipe(&mut self, a: Vec2, b: Vec2, r: f32, albedo: f32, collar_m: f32) {
        let pad = Vec2::splat(r * 1.3);
        let (lo, hi) = (a.min(b) - pad, a.max(b) + pad);
        let ab = b - a;
        let len = ab.length().max(1.0e-6);
        self.stamp(lo, hi, |p| {
            let s = ((p - a).dot(ab) / (len * len)).clamp(0.0, 1.0);
            let d = p.distance(a + ab * s);
            let along = s * len;
            let collared = collar_m > 0.0 && {
                let phase = along / collar_m;
                (phase - phase.round()).abs() * collar_m < 0.6 * r && along > r && along < len - r
            };
            let rr = if collared { 1.22 * r } else { r };
            if d > rr + 0.1 {
                return None;
            }
            let h = (rr * rr - d * d).max(0.0).sqrt();
            Some((h, if collared { albedo * 1.35 } else { albedo }, rr - d))
        });
    }

    /// A box `size` across at `height`, its rim rounded over `bevel`.
    pub fn block(&mut self, center: Vec2, size: Vec2, height: f32, bevel: f32, albedo: f32) {
        let half = size / 2.0;
        self.stamp(center - half, center + half, |p| {
            let q = half - (p - center).abs();
            let edge = q.x.min(q.y);
            if edge < -0.05 {
                return None;
            }
            let x = (edge / bevel).clamp(0.0, 1.0);
            let h = height * (1.0 - (1.0 - x) * (1.0 - x)).sqrt();
            Some((h, albedo, edge))
        });
    }

    fn grille(&mut self, center: Vec2, size: Vec2, pitch: f32, depth: f32, along_x: bool) {
        let half = size / 2.0;
        let t = self.texel_m();
        let n = self.texels as i64;
        let (lo, hi) = (center - half, center + half);
        for y in ((lo.y / t).ceil() as i64).max(0)..((hi.y / t).floor() as i64).min(n) {
            for x in ((lo.x / t).ceil() as i64).max(0)..((hi.x / t).floor() as i64).min(n) {
                let p = Vec2::new((x as f32 + 0.5) * t, (y as f32 + 0.5) * t);
                let c = if along_x { p.y - lo.y } else { p.x - lo.x };
                let phase = (c / pitch).fract();
                let cut = depth * (1.0 - (2.0 * phase - 1.0).abs() * 3.0).clamp(0.0, 1.0);
                self.height[(y * n + x) as usize] -= cut;
            }
        }
    }

    pub fn surface(&self, opaque: bool) -> Surface {
        let n = self.texels;
        let t = self.texel_m();
        let at = |x: usize, y: usize| self.height[y.min(n - 1) * n + x.min(n - 1)];
        let mut normal = Vec::with_capacity(n * n);
        let mut cover = Vec::with_capacity(n * n);
        for y in 0..n {
            for x in 0..n {
                let (xl, yl) = (x.saturating_sub(1), y.saturating_sub(1));
                let dx = (at(x + 1, y) - at(xl, y)) / ((x + 1).min(n - 1) - xl) as f32 / t;
                let dy = (at(x, y + 1) - at(x, yl)) / ((y + 1).min(n - 1) - yl) as f32 / t;
                normal.push(Vec3::new(-dx, -dy, 1.0).normalize());
                let i = y * n + x;
                cover.push(if opaque { 1.0 } else { (self.inside[i] / t + 0.5).clamp(0.0, 1.0) });
            }
        }
        Surface { texels: n, tile_m: self.tile_m, normal, albedo: self.albedo.clone(), cover }
    }
}

/// In the face's frame.
pub struct Surface {
    pub texels: usize,
    pub tile_m: f32,
    pub normal: Vec<Vec3>,
    pub albedo: Vec<f32>,
    pub cover: Vec<f32>,
}

impl Surface {
    /// Linear RGBA8: normal x and y, albedo, coverage.
    pub fn rgba8(&self) -> Vec<u8> {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.normal
            .iter()
            .zip(&self.albedo)
            .zip(&self.cover)
            .flat_map(|((n, a), c)| [byte(n.x * 0.5 + 0.5), byte(n.y * 0.5 + 0.5), byte(*a), byte(*c)])
            .collect()
    }
}

pub fn base(texels: usize, tile_m: f32, rng: &mut Rng) -> Field {
    let mut field = Field::new(texels, tile_m, 0.03);
    let mut stack = vec![(Vec2::ZERO, Vec2::splat(tile_m), 0u32)];
    while let Some((lo, hi, depth)) = stack.pop() {
        let size = hi - lo;
        let split = depth < 2 || (size.max_element() > 2.5 && rng.chance(0.8));
        if split && size.max_element() > 1.6 {
            let along_x = size.x > size.y * rng.range(0.7, 1.4);
            let at = rng.range(0.3, 0.7);
            if along_x {
                let mid = lo.x + size.x * at;
                stack.push((lo, Vec2::new(mid, hi.y), depth + 1));
                stack.push((Vec2::new(mid, lo.y), hi, depth + 1));
            } else {
                let mid = lo.y + size.y * at;
                stack.push((lo, Vec2::new(hi.x, mid), depth + 1));
                stack.push((Vec2::new(lo.x, mid), hi, depth + 1));
            }
            continue;
        }
        let gap = 0.08;
        let center = (lo + hi) / 2.0;
        let plate = size - Vec2::splat(gap);
        let height = [0.08, 0.14, 0.22, 0.32][rng.below(4)];
        let albedo = rng.range(0.07, 0.16);
        field.block(center, plate, height, rng.range(0.04, 0.12), albedo);
        match rng.below(5) {
            0 | 1 if plate.min_element() > 1.0 => {
                let inset = plate * rng.range(0.45, 0.75);
                field.block(center, inset, height + rng.range(-0.08, 0.1), 0.05, albedo * rng.range(0.7, 1.3));
            }
            2 if plate.min_element() > 0.8 => {
                let inset = plate - Vec2::splat(0.3);
                field.grille(center, inset, rng.range(0.12, 0.25), 0.05, plate.x > plate.y);
            }
            _ => {}
        }
    }
    field
}

pub struct Maze {
    pub cells: usize,
    /// Each run's radius is one of these, meters.
    pub radii: &'static [f32],
    /// The share of dead ends trimmed, each pass.
    pub prune: f32,
    pub prune_passes: u32,
    /// The share of runs laid as a pair of thinner pipes.
    pub doubled: f32,
    /// The share of junctions of three or more that carry a box.
    pub boxed: f32,
    pub collar_m: f32,
    /// The share of lanes a tile edge carries a pipe across.
    pub crossing: f32,
}

#[derive(Clone, Copy)]
struct Run {
    r: f32,
    albedo: f32,
    doubled: bool,
}

impl Run {
    fn new(spec: &Maze, rng: &mut Rng) -> Self {
        Run { r: spec.radii[rng.below(spec.radii.len())], albedo: rng.range(0.12, 0.3), doubled: rng.chance(spec.doubled) }
    }

    fn lay(self, field: &mut Field, a: Vec2, b: Vec2, collar_m: f32) {
        if self.doubled {
            let side = (b - a).perp().normalize() * self.r * 0.62;
            field.pipe(a + side, b + side, self.r * 0.5, self.albedo, collar_m);
            field.pipe(a - side, b - side, self.r * 0.5, self.albedo, collar_m);
        } else {
            field.pipe(a, b, self.r, self.albedo, collar_m);
        }
    }
}

#[derive(Clone, Copy)]
struct Crossing {
    run: Run,
    /// Along the edge, in cells.
    at: f32,
}

/// The pipes each tile edge's two choices carry across it. Only the crossing is shared: lanes
/// shared by every tile rule a large hull in a grid of the tile's size, so each tile has its own
/// and a crossing jogs to it.
pub struct MazeEdges {
    /// `[vertical, horizontal][choice][cell along the edge]`.
    crossings: [[Vec<Option<Crossing>>; 2]; 2],
}

impl MazeEdges {
    pub fn new(spec: &Maze, rng: &mut Rng) -> Self {
        let n = spec.cells;
        let mut choice = || {
            (0..n)
                .map(|i| {
                    let on = rng.chance(spec.crossing);
                    let at = i as f32 + 0.5 + rng.range(-0.22, 0.22);
                    on.then(|| Crossing { run: Run::new(spec, rng), at })
                })
                .collect::<Vec<_>>()
        };
        let crossings = [[choice(), choice()], [choice(), choice()]];
        MazeEdges { crossings }
    }
}

/// `edges` are the left, right, bottom and top edges' choices.
pub fn maze(texels: usize, tile_m: f32, spec: &Maze, shared: &MazeEdges, edges: [usize; 4], rng: &mut Rng) -> Field {
    let n = spec.cells;
    let cell = tile_m / n as f32;
    // Off each cell's middle: runs stay straight but the grid is irregular.
    let lane_x: Vec<f32> = (0..n).map(|_| rng.range(-0.22, 0.22)).collect();
    let lane_y: Vec<f32> = (0..n).map(|_| rng.range(-0.22, 0.22)).collect();
    let node = |x: usize, y: usize| Vec2::new((x as f32 + 0.5 + lane_x[x]) * cell, (y as f32 + 0.5 + lane_y[y]) * cell);
    let index = |x: usize, y: usize| y * n + x;
    // Edge to the right of and above each node, inside the tile.
    let mut right = vec![false; n * n];
    let mut up = vec![false; n * n];
    let mut seen = vec![false; n * n];
    let mut stack = vec![(rng.below(n), rng.below(n))];
    seen[index(stack[0].0, stack[0].1)] = true;
    while let Some(&(x, y)) = stack.last() {
        let mut open = Vec::new();
        if x + 1 < n && !seen[index(x + 1, y)] {
            open.push((x + 1, y));
        }
        if x > 0 && !seen[index(x - 1, y)] {
            open.push((x - 1, y));
        }
        if y + 1 < n && !seen[index(x, y + 1)] {
            open.push((x, y + 1));
        }
        if y > 0 && !seen[index(x, y - 1)] {
            open.push((x, y - 1));
        }
        if open.is_empty() {
            stack.pop();
            continue;
        }
        let (nx, ny) = open[rng.below(open.len())];
        if ny == y {
            right[index(x.min(nx), y)] = true;
        } else {
            up[index(x, y.min(ny))] = true;
        }
        seen[index(nx, ny)] = true;
        stack.push((nx, ny));
    }

    let [left, right_edge, bottom, top] =
        [(0, edges[0]), (0, edges[1]), (1, edges[2]), (1, edges[3])].map(|(axis, c)| &shared.crossings[axis][c]);
    let crossing_at = |x: usize, y: usize| {
        (x == 0 && left[y].is_some())
            || (x == n - 1 && right_edge[y].is_some())
            || (y == 0 && bottom[x].is_some())
            || (y == n - 1 && top[x].is_some())
    };
    let inner_degree = |right: &[bool], up: &[bool], x: usize, y: usize| {
        let mut d = 0;
        d += (x + 1 < n && right[index(x, y)]) as usize;
        d += (x > 0 && right[index(x - 1, y)]) as usize;
        d += (y + 1 < n && up[index(x, y)]) as usize;
        d += (y > 0 && up[index(x, y - 1)]) as usize;
        d
    };
    for _ in 0..spec.prune_passes {
        let mut cut = Vec::new();
        for y in 0..n {
            for x in 0..n {
                // A pipe across an edge is the edge's, so the node it lands on stays.
                if !crossing_at(x, y) && inner_degree(&right, &up, x, y) == 1 && rng.chance(spec.prune) {
                    cut.push((x, y));
                }
            }
        }
        for (x, y) in cut {
            right[index(x, y)] = false;
            up[index(x, y)] = false;
            if x > 0 {
                right[index(x - 1, y)] = false;
            }
            if y > 0 {
                up[index(x, y - 1)] = false;
            }
        }
    }

    let mut field = Field::new(texels, tile_m, 0.0);
    // Straight in from the seam, so collars count from it on both sides.
    let cross = |field: &mut Field, crossing: &Option<Crossing>, edge: Vec2, inward: Vec2, to: Vec2| {
        let Some(c) = crossing else { return };
        let along = inward.perp().abs();
        let at = edge + along * c.at * cell;
        let elbow = at + inward * (to - at).dot(inward);
        c.run.lay(field, at, elbow, spec.collar_m);
        c.run.lay(field, elbow, to, spec.collar_m);
    };
    for y in 0..n {
        cross(&mut field, &left[y], Vec2::ZERO, Vec2::X, node(0, y));
        cross(&mut field, &right_edge[y], Vec2::new(tile_m, 0.0), -Vec2::X, node(n - 1, y));
    }
    for x in 0..n {
        cross(&mut field, &bottom[x], Vec2::ZERO, Vec2::Y, node(x, 0));
        cross(&mut field, &top[x], Vec2::new(0.0, tile_m), -Vec2::Y, node(x, n - 1));
    }
    for y in 0..n {
        for x in 0..n {
            let a = node(x, y);
            if x + 1 < n && right[index(x, y)] {
                Run::new(spec, rng).lay(&mut field, a, node(x + 1, y), spec.collar_m);
            }
            if y + 1 < n && up[index(x, y)] {
                Run::new(spec, rng).lay(&mut field, a, node(x, y + 1), spec.collar_m);
            }
            let d = inner_degree(&right, &up, x, y)
                + [x == 0 && left[y].is_some(), x == n - 1 && right_edge[y].is_some(), y == 0 && bottom[x].is_some(), y == n - 1 && top[x].is_some()]
                    .iter()
                    .filter(|&&c| c)
                    .count();
            // Kept off the tile's edge, where the neighbor would not have it.
            let room = 2.0 * (a.min(Vec2::splat(tile_m) - a) - 0.1);
            if d >= 3 && rng.chance(spec.boxed) {
                let r = spec.radii.iter().copied().fold(0.0, f32::max);
                let size = (Vec2::new(rng.range(2.6, 4.0), rng.range(2.6, 4.0)) * r).min(room);
                field.block(a, size, r * rng.range(1.2, 1.7), 0.25 * r, rng.range(0.1, 0.22));
            } else if d == 1 {
                let r = spec.radii[0] * 1.5;
                field.block(a, Vec2::splat(2.0 * r).min(room), r * 1.1, 0.3 * r, 0.22);
            }
        }
    }
    field
}

/// A level's normal and albedo are coverage-weighted, or a pipe's edge fades toward the empty
/// texels' flat normal. Its coverage is rescaled to keep the top level's share past the cutout,
/// or thin pipes erode away with distance.
pub fn tile_array(layers: &[Vec<u8>], texels: u32) -> Image {
    let mut data = Vec::new();
    for top in layers {
        let target = covered_share(top, 1.0);
        let mut level = top.clone();
        let mut side = texels as usize;
        loop {
            data.extend_from_slice(&level);
            if side == 1 {
                break;
            }
            level = halved(&level, side);
            side /= 2;
            preserve_coverage(&mut level, target);
        }
    }
    let mut image = Image::new(
        Extent3d { width: texels, height: texels, depth_or_array_layers: layers.len() as u32 },
        TextureDimension::D2,
        vec![0; (texels * texels) as usize * 4 * layers.len()],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = texels.trailing_zeros() + 1;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    image.texture_view_descriptor =
        Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image
}

fn halved(level: &[u8], side: usize) -> Vec<u8> {
    let half = side / 2;
    let mut out = vec![0u8; half * half * 4];
    for y in 0..half {
        for x in 0..half {
            let at = |dx: usize, dy: usize, c: usize| level[((2 * y + dy) * side + 2 * x + dx) * 4 + c] as f32;
            let four = [(0, 0), (1, 0), (0, 1), (1, 1)];
            let alpha: f32 = four.iter().map(|&(dx, dy)| at(dx, dy, 3)).sum();
            let o = (y * half + x) * 4;
            for c in 0..3 {
                let flat = if c == 2 { 0.0 } else { 127.5 };
                let v = if alpha > 0.0 {
                    four.iter().map(|&(dx, dy)| at(dx, dy, c) * at(dx, dy, 3)).sum::<f32>() / alpha
                } else {
                    flat
                };
                out[o + c] = v.round() as u8;
            }
            out[o + 3] = (alpha / 4.0).round() as u8;
        }
    }
    out
}

fn covered_share(level: &[u8], scale: f32) -> f32 {
    let texels = level.len() / 4;
    level.chunks_exact(4).filter(|t| t[3] as f32 * scale >= 127.5).count() as f32 / texels as f32
}

fn preserve_coverage(level: &mut [u8], target: f32) {
    if target >= 1.0 || target <= 0.0 {
        return;
    }
    let (mut lo, mut hi) = (0.25f32, 8.0f32);
    for _ in 0..16 {
        let mid = (lo * hi).sqrt();
        if covered_share(level, mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    for t in level.chunks_exact_mut(4) {
        t[3] = (t[3] as f32 * hi).min(255.0).round() as u8;
    }
}
