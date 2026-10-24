//! The lamps, placed in tile variants and baked into each layer's light when the tiles are made.
//!
//! A ship hundreds of kilometers long cannot afford to light every texel from every lamp near it
//! each frame, so what does not depend on the eye is done here: each lamp's diffuse light, its
//! shadows through the layers between, the haze it lights, and its bulb. What is left to the
//! shader is the star, which moves as the ship turns, and the view. Lamp specular is lost.
//!
//! Light is in units of the lamps' color, which the shader supplies, so color and power stay
//! live. Stored RGB9E5: four bytes a texel, and bulbs need the range.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::math::{UVec2, Vec2, Vec3, Vec4};
use bevy::prelude::*;
use bevy::render::render_resource::{
    Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};

use crate::tiles::{Rng, Surface};

pub struct Lamps {
    /// One lamp at most a cell, and no farther than this does its light reach.
    pub cell_m: f32,
    pub deep_share: f32,
    /// Of the cells where layer 1 is there to carry one.
    pub riding_share: f32,
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
}

/// Two choices of lamps a tile edge, so a tile is one of [`VARIANTS`]: left + 2 right + 4 bottom
/// + 8 top. The shader picks each tile's edges by hash, and the tiles meet whatever it picks.
pub const VARIANTS: usize = 16;

/// Every tile's lamps, by variant.
///
/// A lamp's light reaches [`Lamps::cell_m`], so the plane is cut into regions no lamp lights
/// across: a strip that far either side of each tile edge, a square at each corner, and the
/// interior. A strip's lamps are its edge's choice, and light both tiles it touches the same way;
/// the corners are one set everywhere; only the interior is the tile's own. A tile is baked from
/// its interior, its four strips and its four corners, so the light is continuous across a seam.
pub fn variants(spec: &Lamps, layers: &[Surface], seed: u64) -> Vec<Vec<Lamp>> {
    let tile_m = layers[0].tile_m;
    let reach = spec.cell_m;
    let inner = ((tile_m - 2.0 * reach) / spec.cell_m).round() as u32;
    assert!(inner >= 1 && (tile_m / spec.cell_m).fract() == 0.0, "a tile is whole cells, wider than two");
    let region = |tag: u64, lo: Vec2, cells: UVec2| {
        let mut rng = Rng::new(seed.wrapping_mul(0x2545_F491_4F6C_DD1D).wrapping_add(tag));
        lamps_in(spec, layers, &mut rng, lo, cells)
    };
    let corner = region(1, Vec2::splat(-reach), UVec2::splat(2));
    let vertical: Vec<_> = (0..2).map(|c| region(2 + c, Vec2::new(-reach, reach), UVec2::new(2, inner))).collect();
    let horizontal: Vec<_> = (0..2).map(|c| region(4 + c, Vec2::new(reach, -reach), UVec2::new(inner, 2))).collect();
    let shifted = |lamps: &[Lamp], by: Vec2| lamps.iter().map(|l| Lamp { at: l.at + by, ..*l }).collect::<Vec<_>>();
    (0..VARIANTS)
        .map(|v| {
            let mut lamps = region(16 + v as u64, Vec2::splat(reach), UVec2::splat(inner));
            lamps.extend(shifted(&vertical[v & 1], Vec2::ZERO));
            lamps.extend(shifted(&vertical[v >> 1 & 1], Vec2::new(tile_m, 0.0)));
            lamps.extend(shifted(&horizontal[v >> 2 & 1], Vec2::ZERO));
            lamps.extend(shifted(&horizontal[v >> 3 & 1], Vec2::new(0.0, tile_m)));
            for at in [Vec2::ZERO, Vec2::new(tile_m, 0.0), Vec2::new(0.0, tile_m), Vec2::splat(tile_m)] {
                lamps.extend(shifted(&corner, at));
            }
            lamps
        })
        .collect()
}

/// Deep lamps over the base, and riding ones on layer 1's pipes, at most one of each a cell.
fn lamps_in(spec: &Lamps, layers: &[Surface], rng: &mut Rng, lo: Vec2, cells: UVec2) -> Vec<Lamp> {
    let mut lamps = Vec::new();
    for y in 0..cells.y {
        for x in 0..cells.x {
            for (riding, share, z) in [(false, spec.deep_share, spec.heights.z), (true, spec.riding_share, spec.heights.w)] {
                let on = rng.chance(share);
                let at = lo + (Vec2::new(x as f32, y as f32) + 0.2 + 0.6 * Vec2::new(rng.unit(), rng.unit())) * spec.cell_m;
                let tint = defect(rng);
                let carried = !riding || sample(&layers[1].cover, layers[1].texels, layers[1].tile_m, at) >= 0.5;
                if on && carried && tint != Vec3::ZERO {
                    lamps.push(Lamp { at, z, tint, carrier: riding as usize });
                }
            }
        }
    }
    lamps
}

/// A lamp's color and power against a sound one's, which is 1.
///
/// Most are a few percent off in each channel, as a batch is. Some came out of a bad batch,
/// greener or pinker. Some are failing: an aging sodium arc runs dim and red. A few are dead.
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

/// Each layer's light at every texel it covers.
pub fn bake(spec: &Lamps, layers: &[Surface], lamps: &[Lamp]) -> Vec<Vec<Vec3>> {
    let n = layers[0].texels;
    let tile_m = layers[0].tile_m;
    let t = tile_m / n as f32;
    let z = [0.0, spec.heights.x, spec.heights.y];
    // Some 10 cm of penumbra.
    let soft: Vec<Vec<f32>> = layers.iter().map(|s| blurred(&s.cover, n, 2)).collect();
    let passed = |p: Vec2, zp: f32, q: Vec2, zq: f32| {
        let mut through = 1.0;
        for k in 1..=2 {
            if (z[k] - zp) * (z[k] - zq) < 0.0 {
                let at = p.lerp(q, (z[k] - zp) / (zq - zp));
                through *= 1.0 - smoothstep(0.35, 0.65, sample(&soft[k], n, tile_m, at));
            }
        }
        through
    };
    // Lamps outside the tile light into it but are not wrapped around: the neighbor's own bake
    // has them on its side.
    let inside = |x: i64, y: i64| (0..n as i64).contains(&x) && (0..n as i64).contains(&y);
    let index = |x: i64, y: i64| y as usize * n + x as usize;
    let texel = |x: i64, y: i64| Vec2::new((x as f32 + 0.5) * t, (y as f32 + 0.5) * t);

    let mut light = vec![vec![Vec3::ZERO; n * n]; layers.len()];
    let reach = spec.cell_m;
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
                    let through = passed(p, z[k], lamp.at, lamp.z);
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

/// Every variant's layers' light as one mipmapped array, variant-major, each level weighted by
/// coverage as the geometry's is.
pub fn light_array(light: &[Vec<Vec3>], layers: &[Surface]) -> Image {
    let texels = layers[0].texels as u32;
    let mut data = Vec::new();
    for (rgb, surface) in light.iter().zip(layers.iter().cycle()) {
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

/// Bilinear, wrapping, at `p` meters.
fn sample(values: &[f32], n: usize, tile_m: f32, p: Vec2) -> f32 {
    let s = p / tile_m * n as f32 - 0.5;
    let (x0, y0) = (s.x.floor(), s.y.floor());
    let (fx, fy) = (s.x - x0, s.y - y0);
    let at = |dx: i64, dy: i64| {
        let (x, y) = ((x0 as i64 + dx).rem_euclid(n as i64), (y0 as i64 + dy).rem_euclid(n as i64));
        values[y as usize * n + x as usize]
    };
    let top = at(0, 0) * (1.0 - fx) + at(1, 0) * fx;
    let bottom = at(0, 1) * (1.0 - fx) + at(1, 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// A wrapping box blur of `radius` texels, separable.
fn blurred(values: &[f32], n: usize, radius: usize) -> Vec<f32> {
    let width = (2 * radius + 1) as f32;
    let pass = |src: &[f32], horizontal: bool| {
        let mut out = vec![0.0; n * n];
        for a in 0..n {
            for b in 0..n {
                let mut sum = 0.0;
                for o in 0..=2 * radius {
                    let c = (b + n + o - radius) % n;
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
