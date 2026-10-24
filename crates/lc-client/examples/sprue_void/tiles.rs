//! The three layers' tiles, generated on the CPU: a plated base, and two mazes of pipe.
//!
//! Each tile repeats, so every shape is stamped with wraparound and every maze is grown on a
//! torus. A texel is linear RGBA8: the normal's x and y in the face's own frame, albedo, and
//! coverage, which is 1 everywhere on the base.


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

    /// Calls `shape` with the meter position of every texel center within `radius` of the box
    /// `lo..hi`, unwrapped, and keeps what it returns where it stands taller.
    fn stamp(&mut self, lo: Vec2, hi: Vec2, mut shape: impl FnMut(Vec2) -> Option<(f32, f32, f32)>) {
        let t = self.texel_m();
        let n = self.texels as i64;
        let (x0, y0) = ((lo.x / t).floor() as i64 - 1, (lo.y / t).floor() as i64 - 1);
        let (x1, y1) = ((hi.x / t).ceil() as i64 + 1, (hi.y / t).ceil() as i64 + 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = Vec2::new((x as f32 + 0.5) * t, (y as f32 + 0.5) * t);
                let Some((height, albedo, inside)) = shape(p) else { continue };
                let i = (y.rem_euclid(n) * n + x.rem_euclid(n)) as usize;
                if inside > -t && height >= self.height[i] {
                    self.height[i] = height;
                    self.albedo[i] = albedo;
                }
                self.inside[i] = self.inside[i].max(inside);
            }
        }
    }

    /// A pipe from `a` to `b` of radius `r`, with a collar every `collar_m` along it.
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

    /// Grooves cut across a box, `pitch` apart, along x or y.
    fn grille(&mut self, center: Vec2, size: Vec2, pitch: f32, depth: f32, along_x: bool) {
        let half = size / 2.0;
        let t = self.texel_m();
        let n = self.texels as i64;
        let (lo, hi) = (center - half, center + half);
        for y in ((lo.y / t).ceil() as i64)..((hi.y / t).floor() as i64) {
            for x in ((lo.x / t).ceil() as i64)..((hi.x / t).floor() as i64) {
                let p = Vec2::new((x as f32 + 0.5) * t, (y as f32 + 0.5) * t);
                let c = if along_x { p.y - lo.y } else { p.x - lo.x };
                let phase = (c / pitch).fract();
                let cut = depth * (1.0 - (2.0 * phase - 1.0).abs() * 3.0).clamp(0.0, 1.0);
                let i = (y.rem_euclid(n) * n + x.rem_euclid(n)) as usize;
                self.height[i] -= cut;
            }
        }
    }

    /// Normals from the height's slope, and coverage antialiased over a texel.
    pub fn surface(&self, opaque: bool) -> Surface {
        let n = self.texels;
        let t = self.texel_m();
        let at = |x: usize, y: usize| self.height[(y % n) * n + x % n];
        let mut normal = Vec::with_capacity(n * n);
        let mut cover = Vec::with_capacity(n * n);
        for y in 0..n {
            for x in 0..n {
                let dx = (at(x + 1, y) - at(x + n - 1, y)) / (2.0 * t);
                let dy = (at(x, y + 1) - at(x, y + n - 1)) / (2.0 * t);
                normal.push(Vec3::new(-dx, -dy, 1.0).normalize());
                let i = y * n + x;
                cover.push(if opaque { 1.0 } else { (self.inside[i] / t + 0.5).clamp(0.0, 1.0) });
            }
        }
        Surface { texels: n, tile_m: self.tile_m, normal, albedo: self.albedo.clone(), cover }
    }
}

/// One layer as the bake and the GPU read it, in the face's frame.
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

/// Plates by recursive subdivision, some with a raised inset or a grille.
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

/// How one maze layer is built.
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
    /// Where the lattice starts, in cells, so two layers do not line up.
    pub offset: f32,
}

/// A randomized depth-first maze on a torus, pruned, drawn as pipes along straight lanes.
pub fn maze(texels: usize, tile_m: f32, spec: &Maze, rng: &mut Rng) -> Field {
    let n = spec.cells;
    let cell = tile_m / n as f32;
    // Each lane is nudged off its cell's middle, keeping runs straight but the grid irregular.
    let lane_x: Vec<f32> = (0..n).map(|_| rng.range(-0.22, 0.22)).collect();
    let lane_y: Vec<f32> = (0..n).map(|_| rng.range(-0.22, 0.22)).collect();
    let node = |x: usize, y: usize| {
        Vec2::new(
            (x as f32 + 0.5 + spec.offset + lane_x[x]) * cell,
            (y as f32 + 0.5 + spec.offset + lane_y[y]) * cell,
        )
    };
    let index = |x: usize, y: usize| y * n + x;
    // Edge to the right of and above each node.
    let mut right = vec![false; n * n];
    let mut up = vec![false; n * n];
    let mut seen = vec![false; n * n];
    let mut stack = vec![(rng.below(n), rng.below(n))];
    seen[index(stack[0].0, stack[0].1)] = true;
    while let Some(&(x, y)) = stack.last() {
        let neighbors = [
            ((x + 1) % n, y, 0u8),
            ((x + n - 1) % n, y, 1),
            (x, (y + 1) % n, 2),
            (x, (y + n - 1) % n, 3),
        ];
        let open: Vec<_> = neighbors.iter().filter(|(nx, ny, _)| !seen[index(*nx, *ny)]).collect();
        if open.is_empty() {
            stack.pop();
            continue;
        }
        let &&(nx, ny, dir) = open.get(rng.below(open.len())).unwrap();
        match dir {
            0 => right[index(x, y)] = true,
            1 => right[index(nx, ny)] = true,
            2 => up[index(x, y)] = true,
            _ => up[index(nx, ny)] = true,
        }
        seen[index(nx, ny)] = true;
        stack.push((nx, ny));
    }

    let degree = |right: &[bool], up: &[bool], x: usize, y: usize| {
        [right[index(x, y)], right[index((x + n - 1) % n, y)], up[index(x, y)], up[index(x, (y + n - 1) % n)]]
            .iter()
            .filter(|&&e| e)
            .count()
    };
    for _ in 0..spec.prune_passes {
        let mut cut = Vec::new();
        for y in 0..n {
            for x in 0..n {
                if degree(&right, &up, x, y) == 1 && rng.chance(spec.prune) {
                    cut.push((x, y));
                }
            }
        }
        for (x, y) in cut {
            right[index(x, y)] = false;
            right[index((x + n - 1) % n, y)] = false;
            up[index(x, y)] = false;
            up[index(x, (y + n - 1) % n)] = false;
        }
    }

    let mut field = Field::new(texels, tile_m, 0.0);
    for y in 0..n {
        for x in 0..n {
            let a = node(x, y);
            for (on, b) in [
                (right[index(x, y)], node((x + 1) % n, y) + Vec2::new(if x + 1 == n { tile_m } else { 0.0 }, 0.0)),
                (up[index(x, y)], node(x, (y + 1) % n) + Vec2::new(0.0, if y + 1 == n { tile_m } else { 0.0 })),
            ] {
                if !on {
                    continue;
                }
                let r = spec.radii[rng.below(spec.radii.len())];
                let albedo = rng.range(0.12, 0.3);
                if rng.chance(spec.doubled) {
                    let side = (b - a).perp().normalize() * r * 0.62;
                    field.pipe(a + side, b + side, r * 0.5, albedo, spec.collar_m);
                    field.pipe(a - side, b - side, r * 0.5, albedo, spec.collar_m);
                } else {
                    field.pipe(a, b, r, albedo, spec.collar_m);
                }
            }
            let d = degree(&right, &up, x, y);
            if d >= 3 && rng.chance(spec.boxed) {
                let r = spec.radii.iter().copied().fold(0.0, f32::max);
                let size = Vec2::new(rng.range(2.6, 4.0), rng.range(2.6, 4.0)) * r;
                field.block(a, size, r * rng.range(1.2, 1.7), 0.25 * r, rng.range(0.1, 0.22));
            } else if d == 1 {
                // A dead end is capped, not cut off.
                let r = spec.radii[0] * 1.5;
                field.block(a, Vec2::splat(2.0 * r), r * 1.1, 0.3 * r, 0.22);
            }
        }
    }
    field
}

/// The three layers as a repeating mipmapped array.
///
/// A level's normal and albedo are coverage-weighted, so a pipe's edge does not fade toward the
/// empty texels' flat normal; its coverage is rescaled so the share of texels past the cutout
/// stays the top level's, or thin pipes would erode away with distance.
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
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
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
