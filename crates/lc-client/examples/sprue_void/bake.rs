//! Lamps, placed in tile variants and baked into each layer's light when the tiles are made.
//!
//! Lighting every texel from every nearby lamp each frame costs too much on a ship hundreds of
//! kilometers long, so lamp light, shadows, haze and bulbs are baked and the shader keeps only the
//! star. Lamp specular is lost. Light is in units of the lamps' color, so color and power stay
//! live; RGB9E5 because bulbs need the range.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{UVec2, Vec2, Vec3, Vec4};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

use crate::tiles::{Rng, Surface};

pub struct Lamps {
    /// At most one lamp of each kind a cell, in each variant.
    pub cell_m: f32,
    /// How far a lamp's light goes. Twice it, the width of the strip along a tile edge, is whole
    /// cells.
    pub reach_m: f32,
    /// The slice of its circle a lamp lights, aimed at random; a full turn is a bare bulb.
    pub beam_rad: f32,
    pub deep_share: f32,
    /// Of the cells where layer 1 is there to carry one.
    pub riding_share: f32,
    /// The share of texels, over all variants, where layer 1 could carry a riding lamp. Strips
    /// have no riding lamps and take this many more deep ones, or each tile edge is a dim lane.
    pub carried: f32,
    /// Meters over the base: layer 1, layer 2, the deep lamps, the lamps riding layer 1.
    pub heights: Vec4,
    /// The share of a lamp's light that lights the haze, whatever the surface faces.
    pub scatter: f32,
    pub bulb_radius_m: f32,
    pub bulb_power: f32,
}

#[derive(Clone, Copy)]
pub struct Lamp {
    at: Vec2,
    z: f32,
    tint: Vec3,
    /// The layer its bulb is drawn on.
    carrier: usize,
    /// Unit, along the surface: where a floodlight points.
    aim: Vec2,
}

/// Variant `v`'s lamps.
///
/// No light goes farther than [`Lamps::reach_m`], so a strip that wide either side of a tile edge
/// belongs to the edge: both tiles beside it bake the same strip lamps, and light is continuous
/// across the seam. `interior` is this variant's of [`interiors`]. Corners would be the same on
/// every tile, and so repeat; they have no lamps. Riding lamps stay in the interior, since a strip
/// lamp may stand on the neighbor's pipes.
pub fn of_variant(spec: &Lamps, layers: &[Surface], seed: u64, v: usize, interior: &[Lamp]) -> Vec<Lamp> {
    let tile_m = layers[0].tile_m;
    let reach = spec.reach_m;
    let (strip, inner) = cells(spec, tile_m);
    let strip_deep = spec.deep_share + spec.riding_share * spec.carried;
    let strip_of = |tag: u64, c: usize, lo: Vec2, cells: UVec2| {
        spread(spec, &mut region_rng(seed, tag), lo, cells, 2, [strip_deep, 0.0]).swap_remove(c)
    };
    let shifted = |lamps: Vec<Lamp>, by: Vec2| lamps.into_iter().map(move |l| Lamp { at: l.at + by, ..l });
    let vertical = |c: usize| strip_of(2, c, Vec2::new(-reach, reach), UVec2::new(strip, inner));
    let horizontal = |c: usize| strip_of(4, c, Vec2::new(reach, -reach), UVec2::new(inner, strip));
    let layer_1 = &layers[1];
    let mut lamps: Vec<Lamp> = interior
        .iter()
        .filter(|l| {
            l.carrier == 0
                || sample(&layer_1.cover, layer_1.texels, layer_1.tile_m, l.at).is_some_and(|c| c >= 0.5)
        })
        .copied()
        .collect();
    lamps.extend(vertical(v & 1));
    lamps.extend(shifted(vertical(v >> 1 & 1), Vec2::new(tile_m, 0.0)));
    lamps.extend(horizontal(v >> 2 & 1));
    lamps.extend(shifted(horizontal(v >> 3 & 1), Vec2::new(0.0, tile_m)));
    lamps.retain(|l| l.tint != Vec3::ZERO);
    lamps
}

/// Every variant's interior lamps, spread among them together. Riding ones are candidates: a
/// variant keeps those its own layer 1 is there to carry.
pub fn interiors(spec: &Lamps, tile_m: f32, seed: u64) -> Vec<Vec<Lamp>> {
    let (_, inner) = cells(spec, tile_m);
    let lo = Vec2::splat(spec.reach_m);
    spread(spec, &mut region_rng(seed, 16), lo, UVec2::splat(inner), crate::VARIANTS, [spec.deep_share, spec.riding_share])
}

/// Cells across a strip, and along the interior.
fn cells(spec: &Lamps, tile_m: f32) -> (u32, u32) {
    let strip = 2.0 * spec.reach_m / spec.cell_m;
    let inner = (tile_m - 2.0 * spec.reach_m) / spec.cell_m;
    assert!(
        strip.fract() == 0.0 && inner.fract() == 0.0 && inner >= 1.0,
        "twice the reach, and the tile less it, are whole cells"
    );
    (strip as u32, inner as u32)
}

/// One per region, not per variant, so both tiles beside a strip lay out the same lamps.
fn region_rng(seed: u64, tag: u64) -> Rng {
    Rng::new(seed.wrapping_mul(0x2545_F491_4F6C_DD1D).wrapping_add(tag))
}

/// Lamps for each of `choices`, laid out together and spread evenly among them.
///
/// Tiles are drawn at random, so from far off a hull is the mean of its variants, and lamps
/// placed independently leave that mean lumpy: a pattern repeating every tile. In each cell a kind
/// of lamp goes to its share of the choices, each to its own row and column of a grid as fine as
/// there are lamps, with their aims splitting the circle.
fn spread(spec: &Lamps, rng: &mut Rng, lo: Vec2, cells: UVec2, choices: usize, shares: [f32; 2]) -> Vec<Vec<Lamp>> {
    let mut out = vec![Vec::new(); choices];
    let shuffled = |rng: &mut Rng, n: usize| {
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            order.swap(i, rng.below(i + 1));
        }
        order
    };
    for y in 0..cells.y {
        for x in 0..cells.x {
            let cell = lo + Vec2::new(x as f32, y as f32) * spec.cell_m;
            for (riding, share) in shares.into_iter().enumerate() {
                let count = ((share * choices as f32 + rng.unit()).floor() as usize).min(choices);
                if count == 0 {
                    continue;
                }
                let who = shuffled(rng, choices);
                let (column, row) = (shuffled(rng, count), shuffled(rng, count));
                let turn = rng.unit();
                for i in 0..count {
                    let slot = Vec2::new(column[i] as f32 + rng.unit(), row[i] as f32 + rng.unit()) / count as f32;
                    out[who[i]].push(Lamp {
                        at: cell + slot * spec.cell_m,
                        z: if riding == 1 { spec.heights.w } else { spec.heights.z },
                        tint: defect(rng),
                        carrier: riding,
                        aim: Vec2::from_angle((i as f32 + turn) / count as f32 * std::f32::consts::TAU),
                    });
                }
            }
        }
    }
    out
}

/// Color and power, 1 for a sound lamp: a few percent off by batch, some greener or pinker, some
/// failing dim and red, a few dead.
fn defect(rng: &mut Rng) -> Vec3 {
    let batch = Vec3::new(rng.range(0.96, 1.04), rng.range(0.95, 1.05), rng.range(0.92, 1.08));
    let roll = rng.unit();
    if roll < 0.03 {
        Vec3::ZERO
    } else if roll < 0.11 {
        batch * Vec3::new(1.0, 0.62, 0.42) * rng.range(0.12, 0.5)
    } else if roll < 0.17 {
        batch * Vec3::new(0.86, 1.06, 0.78)
    } else if roll < 0.22 {
        batch * Vec3::new(1.06, 0.84, 1.02)
    } else {
        batch
    }
}

/// What [`Lamps::carried`] is, over every variant's layer 1.
pub fn carried(layer_1s: &[&Surface]) -> f32 {
    let (on, all) = layer_1s
        .iter()
        .fold((0, 0), |(on, all), s| (on + s.cover.iter().filter(|&&c| c >= 0.5).count(), all + s.cover.len()));
    on as f32 / all as f32
}

/// Each layer's light at every texel it covers.
pub fn bake(spec: &Lamps, layers: &[Surface], lamps: &[Lamp]) -> Vec<Vec<Vec3>> {
    let n = layers[0].texels;
    let tile_m = layers[0].tile_m;
    let t = tile_m / n as f32;
    let z = [0.0, spec.heights.x, spec.heights.y];
    // Some 10 cm of penumbra.
    let soft: Vec<Vec<f32>> = layers.iter().map(|s| blurred(&s.cover, n, 2)).collect();
    // Past the tile's edge is a neighbor of unknown variant, so a layer there is its average.
    let mean: Vec<f32> = layers.iter().map(|s| s.cover.iter().sum::<f32>() / s.cover.len() as f32).collect();
    let passed = |p: Vec2, zp: f32, q: Vec2, zq: f32| {
        let mut through = 1.0;
        for k in 1..=2 {
            if (z[k] - zp) * (z[k] - zq) < 0.0 {
                let at = p.lerp(q, (z[k] - zp) / (zq - zp));
                let cover = sample(&soft[k], n, tile_m, at).unwrap_or(mean[k]);
                through *= 1.0 - smoothstep(0.35, 0.65, cover);
            }
        }
        through
    };
    // Not wrapped: the neighbor's own bake has the far side.
    let inside = |x: i64, y: i64| (0..n as i64).contains(&x) && (0..n as i64).contains(&y);
    let index = |x: i64, y: i64| y as usize * n + x as usize;
    let texel = |x: i64, y: i64| Vec2::new((x as f32 + 0.5) * t, (y as f32 + 0.5) * t);

    let mut light = vec![vec![Vec3::ZERO; n * n]; layers.len()];
    let reach = spec.reach_m;
    // With a few degrees of soft edge.
    let beam = |lamp: &Lamp, p: Vec2| {
        let half = 0.5 * spec.beam_rad;
        let away = p - lamp.at;
        if half >= std::f32::consts::PI || away.length_squared() < 1.0e-6 {
            return 1.0;
        }
        let soft = 4f32.to_radians();
        smoothstep((half + soft).min(std::f32::consts::PI).cos(), (half - soft).max(0.0).cos(), away.normalize().dot(lamp.aim))
    };
    let span = (reach / t).ceil() as i64;
    let bulb_span = (6.5 * spec.bulb_radius_m / t).ceil() as i64;
    for lamp in lamps {
        let (cx, cy) = ((lamp.at.x / t).floor() as i64, (lamp.at.y / t).floor() as i64);
        for (k, surface) in layers.iter().enumerate() {
            for y in cy - span..=cy + span {
                for x in cx - span..=cx + span {
                    if !inside(x, y) {
                        continue;
                    }
                    let i = index(x, y);
                    if surface.cover[i] <= 0.0 {
                        continue;
                    }
                    let p = texel(x, y);
                    let d = (lamp.at - p).extend(lamp.z - z[k]);
                    let d2 = d.length_squared();
                    let window = (1.0 - d2 / (reach * reach)).max(0.0).powi(2);
                    if window <= 0.0 {
                        continue;
                    }
                    let through = passed(p, z[k], lamp.at, lamp.z) * beam(lamp, p);
                    if through <= 0.0 {
                        continue;
                    }
                    let ndl = surface.normal[i].dot(d / d2.sqrt()).max(0.0);
                    let power = lamp.tint * window * through / (d2 + 0.04);
                    light[k][i] += power * (surface.albedo[i] * ndl + spec.scatter);
                }
            }
        }
        // Averaged over each texel, since a bulb's core is narrower than one.
        const SUB: i32 = 4;
        let r2 = spec.bulb_radius_m * spec.bulb_radius_m;
        for y in cy - bulb_span..=cy + bulb_span {
            for x in cx - bulb_span..=cx + bulb_span {
                if !inside(x, y) {
                    continue;
                }
                let mut glow = 0.0;
                for sy in 0..SUB {
                    for sx in 0..SUB {
                        let at = Vec2::new(x as f32, y as f32) + (Vec2::new(sx as f32, sy as f32) + 0.5) / SUB as f32;
                        let q = (at * t).distance_squared(lamp.at) / r2;
                        glow += (-4.0 * q).exp() + 0.04 * (-0.15 * q).exp();
                    }
                }
                light[lamp.carrier][index(x, y)] += lamp.tint * spec.bulb_power * glow / (SUB * SUB) as f32;
            }
        }
    }
    light
}

/// Each level weighted by `layers`' coverage, as the geometry's is.
pub fn light_array(light: &[Vec<Vec3>], layers: &[&Surface]) -> Image {
    assert_eq!(light.len(), layers.len());
    let texels = layers[0].texels as u32;
    let mut data = Vec::new();
    for (rgb, surface) in light.iter().zip(layers) {
        let mut level: Vec<Vec3> = rgb.clone();
        let mut cover = surface.cover.clone();
        let mut side = texels as usize;
        loop {
            data.extend(level.iter().flat_map(|c| rgb9e5(*c).to_le_bytes()));
            if side == 1 {
                break;
            }
            let half = side / 2;
            let (mut next, mut next_cover) = (vec![Vec3::ZERO; half * half], vec![0.0; half * half]);
            for y in 0..half {
                for x in 0..half {
                    let four = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| (2 * y + dy) * side + 2 * x + dx);
                    let weight: f32 = four.iter().map(|&i| cover[i]).sum();
                    if weight > 0.0 {
                        next[y * half + x] = four.iter().map(|&i| level[i] * cover[i]).sum::<Vec3>() / weight;
                    }
                    next_cover[y * half + x] = weight / 4.0;
                }
            }
            (level, cover, side) = (next, next_cover, half);
        }
    }
    let mut image = Image::new(
        Extent3d { width: texels, height: texels, depth_or_array_layers: light.len() as u32 },
        TextureDimension::D2,
        vec![0; (texels * texels) as usize * 4 * light.len()],
        TextureFormat::Rgb9e5Ufloat,
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

/// Shared-exponent RGB: nine bits of mantissa each, five of exponent, biased 15.
fn rgb9e5(c: Vec3) -> u32 {
    const MAX: f32 = 65408.0;
    let c = c.clamp(Vec3::ZERO, Vec3::splat(MAX));
    let peak = c.max_element();
    let mut exponent = (peak.log2().floor().max(-16.0) as i32) + 16;
    let mut step = 2f32.powi(exponent - 24);
    if (peak / step + 0.5).floor() >= 512.0 {
        exponent += 1;
        step *= 2.0;
    }
    let m = |v: f32| ((v / step + 0.5).floor() as u32).min(511);
    m(c.x) | m(c.y) << 9 | m(c.z) << 18 | (exponent as u32) << 27
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bilinear at `p` meters, clamped at the edge; `None` off the tile.
fn sample(values: &[f32], n: usize, tile_m: f32, p: Vec2) -> Option<f32> {
    if p.min_element() < 0.0 || p.max_element() >= tile_m {
        return None;
    }
    let s = p / tile_m * n as f32 - 0.5;
    let (x0, y0) = (s.x.floor(), s.y.floor());
    let (fx, fy) = (s.x - x0, s.y - y0);
    let at = |dx: i64, dy: i64| {
        let (x, y) = ((x0 as i64 + dx).clamp(0, n as i64 - 1), (y0 as i64 + dy).clamp(0, n as i64 - 1));
        values[y as usize * n + x as usize]
    };
    let top = at(0, 0) * (1.0 - fx) + at(1, 0) * fx;
    let bottom = at(0, 1) * (1.0 - fx) + at(1, 1) * fx;
    Some(top * (1.0 - fy) + bottom * fy)
}

/// A box blur of `radius` texels, separable, clamped at the edge.
fn blurred(values: &[f32], n: usize, radius: usize) -> Vec<f32> {
    let width = (2 * radius + 1) as f32;
    let pass = |src: &[f32], horizontal: bool| {
        let mut out = vec![0.0; n * n];
        for a in 0..n {
            for b in 0..n {
                let mut sum = 0.0;
                for o in 0..=2 * radius {
                    let c = (b + o).saturating_sub(radius).min(n - 1);
                    sum += if horizontal { src[a * n + c] } else { src[c * n + a] };
                }
                let i = if horizontal { a * n + b } else { b * n + a };
                out[i] = sum / width;
            }
        }
        out
    };
    pass(&pass(values, true), false)
}
