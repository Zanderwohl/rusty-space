//! The lamps, placed once a tile and baked into each layer's light when the tile is made.
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
use bevy::math::{Vec2, Vec3, Vec4};
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

pub struct Lamp {
    at: Vec2,
    z: f32,
    tint: Vec3,
    /// The layer its bulb is drawn on.
    carrier: usize,
}

/// Lamps on a tile: deep ones over the base, and riding ones on layer 1's pipes.
pub fn place(spec: &Lamps, layers: &[Surface], rng: &mut Rng) -> Vec<Lamp> {
    let tile_m = layers[0].tile_m;
    let cells = (tile_m / spec.cell_m).round() as usize;
    let mut lamps = Vec::new();
    for y in 0..cells {
        for x in 0..cells {
            for (riding, share, z) in [(false, spec.deep_share, spec.heights.z), (true, spec.riding_share, spec.heights.w)] {
                let on = rng.chance(share);
                let at = (Vec2::new(x as f32, y as f32) + 0.2 + 0.6 * Vec2::new(rng.unit(), rng.unit())) * spec.cell_m;
                // Most are high-pressure sodium; a few run oranger or paler, as a yard's lamps do with age.
                let age = rng.unit();
                if !on || (riding && sample(&layers[1].cover, layers[1].texels, tile_m, at) < 0.5) {
                    continue;
                }
                let tint = Vec3::new(1.0, 0.85, 0.6).lerp(Vec3::new(1.0, 1.08, 1.3), age * age);
                lamps.push(Lamp { at, z, tint, carrier: riding as usize });
            }
        }
    }
    lamps
}

/// Each layer's light at every texel it covers.
pub fn bake(spec: &Lamps, layers: &[Surface], lamps: &[Lamp]) -> Vec<Vec<Vec3>> {
    let n = layers[0].texels;
    let tile_m = layers[0].tile_m;
    let t = tile_m / n as f32;
    let z = [0.0, spec.heights.x, spec.heights.y];
    // Some three texels' penumbra, as the shader's sample a mip and a half down was.
    let soft: Vec<Vec<f32>> = layers.iter().map(|s| blurred(&s.cover, n, 3)).collect();
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
    let wrap = |x: i64, y: i64| (y.rem_euclid(n as i64) * n as i64 + x.rem_euclid(n as i64)) as usize;
    let texel = |x: i64, y: i64| Vec2::new((x as f32 + 0.5) * t, (y as f32 + 0.5) * t);

    let mut light = vec![vec![Vec3::ZERO; n * n]; layers.len()];
    let reach = spec.cell_m;
    let span = (reach / t).ceil() as i64;
    let bulb_span = (6.5 * spec.bulb_radius_m / t).ceil() as i64;
    for lamp in lamps {
        let (cx, cy) = ((lamp.at.x / t) as i64, (lamp.at.y / t) as i64);
        for (k, surface) in layers.iter().enumerate() {
            for y in cy - span..=cy + span {
                for x in cx - span..=cx + span {
                    let i = wrap(x, y);
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
        let r2 = spec.bulb_radius_m * spec.bulb_radius_m;
        for y in cy - bulb_span..=cy + bulb_span {
            for x in cx - bulb_span..=cx + bulb_span {
                let q = texel(x, y).distance_squared(lamp.at) / r2;
                let glow = (-4.0 * q).exp() + 0.04 * (-0.15 * q).exp();
                light[lamp.carrier][wrap(x, y)] += lamp.tint * spec.bulb_power * glow;
            }
        }
    }
    light
}

/// The layers' light as a repeating mipmapped array, each level weighted by coverage as the
/// geometry's is.
pub fn light_array(light: &[Vec<Vec3>], layers: &[Surface]) -> Image {
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
