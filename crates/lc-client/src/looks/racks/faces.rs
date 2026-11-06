//! A server's face: glass elements on its front, some glowing. A few layouts, each baked in
//! several states of its lights, one layer of a texture array apiece, so a wall of servers shows
//! many faces and repeats none nearby. Each texel holds `(cover, glow, 0, 1)`: how much of it is
//! glass, and how brightly that glass glows.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};

use crate::looks::sprue::tiles::Rng;

/// The face's extent on a server, meters: the server less a margin inside its posts and grooves.
pub const FACE_M: Vec2 = Vec2::new(5.2, 2.4);
/// A meter is about 49 texels each way.
const TEXELS: UVec2 = UVec2::new(256, 128);
/// Down to 2 by 1: a level is half the last, each way.
const LEVELS: u32 = 8;
/// Each layout baked this many times, its lights in other states.
const VARIANTS: u64 = 3;

const LIGHT_M: f32 = 0.05;
const LIGHT_PITCH_M: f32 = 0.16;
const GRID_PITCH_M: f32 = 0.14;
const STATUS_PITCH_M: f32 = 0.18;
const LINE_PITCH_M: f32 = 0.12;
const LINE_M: f32 = 0.05;
const BAY_PITCH_M: f32 = 0.42;
const BAY_M: Vec2 = Vec2::new(0.34, 1.7);
const BAY_LIGHT_M: f32 = 0.035;
/// A screen's and a meter's backlight, under what they show.
const BACKLIGHT: f32 = 0.12;

/// One thing on a face, placed by its lower left corner, meters up and across the face.
#[derive(Clone, Copy, Debug)]
enum Element {
    /// A status screen: a lit pane with lines of text on it.
    Screen { at: Vec2, size: Vec2 },
    /// Three status lights in a row.
    Status { at: Vec2 },
    /// Sixteen lights, four by four, in a pattern.
    Grid { at: Vec2 },
    /// Eight lights in a row showing a byte.
    Byte { at: Vec2 },
    /// A bar lit some way along its length, which is its longer side.
    Meter { at: Vec2, size: Vec2 },
    /// Drive bays: dark glass slots in a row, a light atop each.
    Bays { at: Vec2, count: u32 },
}

use Element::*;

fn v(x: f32, y: f32) -> Vec2 {
    Vec2::new(x, y)
}

fn layouts() -> Vec<Vec<Element>> {
    vec![
        vec![Screen { at: v(0.25, 0.45), size: v(2.0, 1.5) }, Status { at: v(4.4, 1.9) }, Byte { at: v(3.4, 1.0) }, Byte { at: v(3.4, 0.7) }],
        vec![Bays { at: v(0.25, 0.35), count: 10 }, Status { at: v(4.55, 2.0) }],
        vec![Grid { at: v(0.3, 0.8) }, Grid { at: v(1.1, 0.8) }, Grid { at: v(1.9, 0.8) }, Screen { at: v(2.8, 0.5), size: v(2.1, 1.4) }],
        (0..4)
            .flat_map(|k| [Byte { at: v(0.3, 0.5 + 0.35 * k as f32) }, Byte { at: v(1.9, 0.5 + 0.35 * k as f32) }])
            .chain([Meter { at: v(3.6, 0.4), size: v(0.3, 1.6) }, Meter { at: v(4.0, 0.4), size: v(0.3, 1.6) }, Status { at: v(4.4, 2.0) }])
            .collect(),
        (0..6)
            .map(|k| Meter { at: v(0.3 + 0.4 * k as f32, 0.4), size: v(0.25, 1.6) })
            .chain([Screen { at: v(2.9, 1.1), size: v(2.0, 0.9) }, Grid { at: v(3.0, 0.3) }])
            .collect(),
        // Next to blank, so a wall has quiet servers among the busy.
        vec![Status { at: v(4.5, 2.05) }],
        vec![Grid { at: v(0.3, 0.9) }, Grid { at: v(0.95, 0.9) }, Screen { at: v(1.8, 0.4), size: v(1.6, 1.6) }, Grid { at: v(3.75, 0.9) }, Grid { at: v(4.4, 0.9) }],
        vec![
            Bays { at: v(0.25, 0.35), count: 6 },
            Grid { at: v(3.0, 1.5) },
            Meter { at: v(3.0, 0.4), size: v(2.0, 0.25) },
            Meter { at: v(3.0, 0.8), size: v(2.0, 0.25) },
            Status { at: v(4.4, 2.0) },
        ],
    ]
}

/// Glass cover and glow at each texel of one face, row 0 at the face's top.
struct Canvas {
    cover: Vec<f32>,
    glow: Vec<f32>,
}

impl Canvas {
    fn new() -> Self {
        let n = (TEXELS.x * TEXELS.y) as usize;
        Canvas { cover: vec![0.0; n], glow: vec![0.0; n] }
    }

    fn texel_m() -> Vec2 {
        FACE_M / TEXELS.as_vec2()
    }

    /// Calls `shade` with each texel's center in meters over the box `lo..hi`, padded a texel, and
    /// keeps the most cover and glow anything has put there.
    fn paint(&mut self, lo: Vec2, hi: Vec2, shade: impl Fn(Vec2) -> (f32, f32)) {
        let t = Self::texel_m();
        let x0 = ((lo.x / t.x).floor() as i32 - 1).max(0) as u32;
        let x1 = ((hi.x / t.x).ceil() as i32 + 1).min(TEXELS.x as i32) as u32;
        let y0 = ((lo.y / t.y).floor() as i32 - 1).max(0) as u32;
        let y1 = ((hi.y / t.y).ceil() as i32 + 1).min(TEXELS.y as i32) as u32;
        for y in y0..y1 {
            for x in x0..x1 {
                let q = (UVec2::new(x, y).as_vec2() + 0.5) * t;
                let (cover, glow) = shade(q);
                let k = ((TEXELS.y - 1 - y) * TEXELS.x + x) as usize;
                self.cover[k] = self.cover[k].max(cover);
                self.glow[k] = self.glow[k].max(glow);
            }
        }
    }

    /// A rectangle, its edges antialiased over a texel.
    fn rect(&mut self, lo: Vec2, size: Vec2, glow: f32) {
        let (t, center, half) = (Self::texel_m(), lo + 0.5 * size, 0.5 * size);
        self.paint(lo, lo + size, |q| {
            let inside = ((half - (q - center).abs()) / t + 0.5).clamp(Vec2::ZERO, Vec2::ONE);
            let cover = inside.x * inside.y;
            (cover, cover * glow)
        });
    }

    fn light(&mut self, center: Vec2, radius: f32, on: bool) {
        let t = Self::texel_m().min_element();
        let glow = if on { 1.0 } else { 0.0 };
        self.paint(center - radius, center + radius, |q| {
            let cover = ((radius - q.distance(center)) / t + 0.5).clamp(0.0, 1.0);
            (cover, cover * glow)
        });
    }

    fn element(&mut self, element: Element, rng: &mut Rng) {
        match element {
            Screen { at, size } => {
                self.rect(at, size, BACKLIGHT);
                let inset = 0.08;
                let mut top = at.y + size.y - inset - LINE_M;
                while top >= at.y + inset {
                    if !rng.chance(0.2) {
                        let mut x = at.x + inset;
                        let end = at.x + size.x - inset - rng.range(0.0, 0.5 * size.x);
                        while x < end {
                            let word = rng.range(0.08, 0.4).min(end - x);
                            self.rect(v(x, top), v(word, LINE_M), 0.9);
                            x += word + 0.06;
                        }
                    }
                    top -= LINE_PITCH_M;
                }
            }
            Status { at } => {
                for i in 0..3 {
                    let on = rng.chance(0.7);
                    self.light(at + v(0.08 + STATUS_PITCH_M * i as f32, 0.08), LIGHT_M, on);
                }
            }
            Grid { at } => {
                let pattern = rng.below(5);
                let noise = rng.below(1 << 16) as u64;
                for j in 0..4 {
                    for i in 0..4 {
                        let on = match pattern {
                            0 => noise >> (4 * j + i) & 1 == 1,
                            1 => i == j || i == 3 - j,
                            2 => i == 0 || j == 0 || i == 3 || j == 3,
                            3 => (i + j) % 2 == 0,
                            // Bars, as a level meter's.
                            _ => j <= (noise as usize >> (2 * i) & 3),
                        };
                        self.light(at + v(0.07 + GRID_PITCH_M * i as f32, 0.07 + GRID_PITCH_M * j as f32), LIGHT_M, on);
                    }
                }
            }
            Byte { at } => {
                let byte = rng.below(256);
                for i in 0..8 {
                    self.light(at + v(0.08 + LIGHT_PITCH_M * i as f32, 0.08), LIGHT_M, byte >> (7 - i) & 1 == 1);
                }
            }
            Meter { at, size } => {
                self.rect(at, size, BACKLIGHT);
                let fill = rng.range(0.15, 0.95);
                let inset = 0.03;
                let inner = size - 2.0 * inset;
                let lit = if size.y > size.x { v(inner.x, fill * inner.y) } else { v(fill * inner.x, inner.y) };
                self.rect(at + inset, lit, 0.8);
            }
            Bays { at, count } => {
                for k in 0..count {
                    let lo = at + v(BAY_PITCH_M * k as f32, 0.0);
                    self.rect(lo, BAY_M, 0.0);
                    let on = rng.chance(0.75);
                    self.light(lo + v(0.5 * BAY_M.x, BAY_M.y - 0.08), BAY_LIGHT_M, on);
                }
            }
        }
    }

    fn rgba8(&self) -> Vec<u8> {
        let byte = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.cover.iter().zip(&self.glow).flat_map(|(&c, &g)| [byte(c), byte(g), 0, 255]).collect()
    }
}

/// A box filter halving each way.
fn halved(level: &[u8], size: UVec2) -> Vec<u8> {
    let half = size / 2;
    let mut out = vec![0u8; (half.x * half.y * 4) as usize];
    for y in 0..half.y {
        for x in 0..half.x {
            for c in 0..4 {
                let at = |dx: u32, dy: u32| level[(((2 * y + dy) * size.x + 2 * x + dx) * 4 + c) as usize] as u32;
                out[((y * half.x + x) * 4 + c) as usize] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
            }
        }
    }
    out
}

/// How many faces [`bake`] makes.
pub fn count() -> u32 {
    layouts().len() as u32 * VARIANTS as u32
}

/// Every face, a layer apiece, with its mips.
pub fn bake() -> Image {
    let mut data = Vec::new();
    for (l, layout) in layouts().iter().enumerate() {
        for variant in 0..VARIANTS {
            let mut rng = Rng::new(0x5E4E_4E45 ^ (l as u64 * VARIANTS + variant));
            let mut canvas = Canvas::new();
            for &element in layout {
                canvas.element(element, &mut rng);
            }
            let (mut level, mut size) = (canvas.rgba8(), TEXELS);
            for k in 0..LEVELS {
                data.extend_from_slice(&level);
                if k + 1 < LEVELS {
                    level = halved(&level, size);
                    size /= 2;
                }
            }
        }
    }
    let mut image = Image::new(
        Extent3d { width: TEXELS.x, height: TEXELS.y, depth_or_array_layers: count() },
        TextureDimension::D2,
        vec![0; (TEXELS.x * TEXELS.y * 4 * count()) as usize],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = LEVELS;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    image.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::D2Array), ..default() });
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every element lies inside the face, or the bake would clip it at the texture's edge.
    #[test]
    fn every_element_fits_its_face() {
        let extent = |e: &Element| -> (Vec2, Vec2) {
            match *e {
                Screen { at, size } | Meter { at, size } => (at, at + size),
                Status { at } => (at, at + v(0.16 + 2.0 * STATUS_PITCH_M, 0.16)),
                Grid { at } => (at, at + Vec2::splat(0.14 + 3.0 * GRID_PITCH_M)),
                Byte { at } => (at, at + v(0.16 + 7.0 * LIGHT_PITCH_M, 0.16)),
                Bays { at, count } => (at, at + v(BAY_PITCH_M * (count - 1) as f32 + BAY_M.x, BAY_M.y)),
            }
        };
        for (l, layout) in layouts().iter().enumerate() {
            for e in layout {
                let (lo, hi) = extent(e);
                assert!(lo.cmpge(Vec2::ZERO).all() && hi.cmple(FACE_M).all(), "layout {l}: {e:?} reaches {hi}");
            }
        }
    }

    #[test]
    fn the_bake_holds_every_level_of_every_face() {
        let image = bake();
        let per_face: u32 = (0..LEVELS).map(|k| (TEXELS.x >> k) * (TEXELS.y >> k) * 4).sum();
        assert_eq!(image.data.as_ref().unwrap().len() as u32, per_face * count());
    }
}
