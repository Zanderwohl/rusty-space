//! Pictures of a compiled galaxy without baking a star: the graph's light sampled into a coarse
//! volume, then integrated along parallel rays. See `34-galaxy-authoring.md`, "Signed distances
//! and ray marching".
//!
//! Emission only: the graph carries no dust field yet, so nothing absorbs. Light is linear sRGB,
//! as [`crate::light`] defines it.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use glam::{DVec3, UVec3};

use crate::compiled::{Aabb, Compiled};
use crate::light::{self, Rgb};
use crate::sample::par_map;

/// A worker's progress through a volume or a march, in slabs or rows, and a flag that stops it.
#[derive(Debug, Default)]
pub struct Progress {
    pub done: AtomicU32,
    pub total: AtomicU32,
    pub stop: AtomicBool,
}

impl Progress {
    /// `(done, total)`.
    pub fn read(&self) -> (u32, u32) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Relaxed)
    }
}

/// The graph's light on a grid of cubic voxels, sampled at their centers.
#[derive(Clone, Debug)]
pub struct Volume {
    pub min: DVec3,
    pub dims: UVec3,
    pub voxel_ly: f64,
    /// light per ly³ as linear sRGB, `x` fastest, then `y`, then `z`.
    pub emission: Vec<[f32; 3]>,
}

impl Volume {
    /// Samples `compiled` over its bounds with `resolution` voxels along the longest side.
    ///
    /// A point sample per voxel misses structure thinner than a voxel; at full galaxy scale a
    /// disc is, and the volume will want to average over each voxel then.
    pub fn of(compiled: &Compiled, resolution: usize, threads: usize) -> Self {
        Self::of_until(compiled, resolution, threads, &Progress::default()).expect("never stopped")
    }

    /// [`Volume::of`], counting slabs into `progress` and giving up with `None` once its flag is
    /// set.
    pub fn of_until(compiled: &Compiled, resolution: usize, threads: usize, progress: &Progress) -> Option<Self> {
        let bounds = compiled.bounds();
        if bounds.is_empty() || resolution == 0 {
            return Some(Self { min: DVec3::ZERO, dims: UVec3::ZERO, voxel_ly: 1.0, emission: Vec::new() });
        }
        let voxel_ly = bounds.size().max_element() / resolution as f64;
        let dims = (bounds.size() / voxel_ly).ceil().as_uvec3().max(UVec3::ONE);
        let min = bounds.center() - dims.as_dvec3() * voxel_ly * 0.5;
        let lights = lights(compiled);
        let half_diagonal = voxel_ly * 3f64.sqrt() * 0.5;
        let slabs: Vec<u32> = (0..dims.z).collect();
        progress.total.store(dims.z, Ordering::Relaxed);
        let emission = par_map(&slabs, threads, |&z| {
            if progress.stopped() {
                return None;
            }
            let mut slab = Vec::with_capacity((dims.x * dims.y) as usize);
            for y in 0..dims.y {
                for x in 0..dims.x {
                    let center = min + (DVec3::new(x as f64, y as f64, z as f64) + 0.5) * voxel_ly;
                    if !compiled.may_reach(center, half_diagonal) {
                        slab.push([0.0; 3]);
                        continue;
                    }
                    let e = emission(compiled, &lights, center);
                    slab.push(e.map(|v| v as f32));
                }
            }
            progress.done.fetch_add(1, Ordering::Relaxed);
            Some(slab)
        });
        let emission: Option<Vec<Vec<[f32; 3]>>> = emission.into_iter().collect();
        Some(Self { min, dims, voxel_ly, emission: emission?.into_iter().flatten().collect() })
    }

    pub fn bounds(&self) -> Aabb {
        Aabb { min: self.min, max: self.min + self.dims.as_dvec3() * self.voxel_ly }
    }

    /// Light per ly³ at `p`, trilinear between voxel centers, zero outside.
    pub fn at(&self, p: DVec3) -> Rgb {
        let g = (p - self.min) / self.voxel_ly - 0.5;
        let base = g.floor();
        let f = g - base;
        let mut out = [0.0; 3];
        for corner in 0..8u32 {
            let o = DVec3::new((corner & 1) as f64, ((corner >> 1) & 1) as f64, ((corner >> 2) & 1) as f64);
            let c = base + o;
            if c.cmplt(DVec3::ZERO).any() || c.cmpge(self.dims.as_dvec3()).any() {
                continue;
            }
            let w = (if o.x > 0.0 { f.x } else { 1.0 - f.x }) * (if o.y > 0.0 { f.y } else { 1.0 - f.y }) * (if o.z > 0.0 { f.z } else { 1.0 - f.z });
            let i = (c.x as u32 + self.dims.x * (c.y as u32 + self.dims.y * c.z as u32)) as usize;
            for k in 0..3 {
                out[k] += w * f64::from(self.emission[i][k]);
            }
        }
        out
    }

    /// `∫ emission dV` over the voxels.
    pub fn total(&self) -> Rgb {
        let dv = self.voxel_ly.powi(3);
        let mut out = [0.0; 3];
        for e in &self.emission {
            for k in 0..3 {
                out[k] += f64::from(e[k]) * dv;
            }
        }
        out
    }
}

/// Each population's mean light per star, in `compiled.populations` order.
pub fn lights(compiled: &Compiled) -> Vec<Rgb> {
    compiled.populations.iter().map(|(id, p)| light::mean_light(p, u64::from(id.0))).collect()
}

fn emission(compiled: &Compiled, lights: &[Rgb], p: DVec3) -> Rgb {
    let mut e = [0.0f64; 3];
    for (n, l) in compiled.sample(p).density.iter().zip(lights) {
        for k in 0..3 {
            e[k] += n * l[k];
        }
    }
    e
}

/// [`march`] straight through the graph instead of a volume: each step samples the field
/// itself, so edges are as sharp as the shapes make them rather than blurred over a voxel.
/// Outside every density-raising support it skips ahead by [`Compiled::reach`]. `step_ly` sets
/// the resolution along the ray only; across it, each pixel is exact.
#[allow(clippy::too_many_arguments)]
pub fn march_exact(compiled: &Compiled, lights: &[Rgb], view: &View, width: u32, height: u32, step_ly: f64, threads: usize, progress: &Progress) -> Option<Picture> {
    let (forward, right, up) = view.basis();
    let b = compiled.bounds();
    let rows: Vec<u32> = (0..height).collect();
    progress.total.store(height, Ordering::Relaxed);
    let pixels = par_map(&rows, threads, |&y| {
        if progress.stopped() {
            return None;
        }
        let row = (0..width)
            .map(|x| {
                let off = (f64::from(x) + 0.5 - f64::from(width) * 0.5) * right + (f64::from(height) * 0.5 - f64::from(y) - 0.5) * up;
                let origin = view.target + off * view.ly_per_px;
                let Some((t0, t1)) = through(b, origin, forward) else { return [0.0; 3] };
                let mut sum = [0.0; 3];
                let mut t = t0 + step_ly * 0.5;
                while t < t1 {
                    let p = origin + forward * t;
                    let skip = compiled.reach(p);
                    if skip > step_ly {
                        t += skip;
                        continue;
                    }
                    let e = emission(compiled, lights, p);
                    for k in 0..3 {
                        sum[k] += e[k];
                    }
                    t += step_ly;
                }
                sum.map(|s| (s * step_ly) as f32)
            })
            .collect::<Vec<_>>();
        progress.done.fetch_add(1, Ordering::Relaxed);
        Some(row)
    });
    let pixels: Option<Vec<Vec<[f32; 3]>>> = pixels.into_iter().collect();
    Some(Picture { width, height, pixels: pixels?.into_iter().flatten().collect() })
}

/// The six sides of the galaxy's axes a camera can look from: `+Z` is face-on from galactic
/// north, `-Y` edge-on with +X to the right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Side {
    /// 0, 1, 2 for X, Y, Z.
    pub axis: usize,
    pub positive: bool,
}

impl Side {
    pub const ALL: [Side; 6] = [
        Side { axis: 0, positive: true },
        Side { axis: 1, positive: true },
        Side { axis: 2, positive: true },
        Side { axis: 0, positive: false },
        Side { axis: 1, positive: false },
        Side { axis: 2, positive: false },
    ];

    pub fn label(self) -> String {
        format!("{}{}", if self.positive { '+' } else { '-' }, ["X", "Y", "Z"][self.axis])
    }

    pub fn named(name: &str) -> Option<Self> {
        Side::ALL.into_iter().find(|s| s.label().eq_ignore_ascii_case(name))
    }

    /// Yaw about +Z from +X and pitch above the XY plane of the direction toward the camera. The
    /// side views keep +Z up; the views from ±Z keep +X to the right.
    pub fn angles(self) -> (f64, f64) {
        use std::f64::consts::{FRAC_PI_2, PI};
        match (self.axis, self.positive) {
            (0, true) => (0.0, 0.0),
            (0, false) => (PI, 0.0),
            (1, true) => (FRAC_PI_2, 0.0),
            (1, false) => (-FRAC_PI_2, 0.0),
            (_, positive) => (-FRAC_PI_2, if positive { FRAC_PI_2 } else { -FRAC_PI_2 }),
        }
    }
}

/// An orthographic camera: looking at `target` from the direction `yaw`, `pitch` gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub target: DVec3,
    pub yaw: f64,
    pub pitch: f64,
    pub ly_per_px: f64,
}

impl View {
    /// `forward`, `right` and `up`. The right comes from the yaw alone, so straight down or up is
    /// still defined.
    pub fn basis(&self) -> (DVec3, DVec3, DVec3) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let forward = -DVec3::new(cp * cy, cp * sy, sp);
        let right = DVec3::new(-sy, cy, 0.0);
        (forward, right, right.cross(forward))
    }

    /// From `side`, with all of `bounds` in a `width` × `height` picture.
    pub fn fitting(side: Side, bounds: Aabb, width: u32, height: u32) -> Self {
        let (yaw, pitch) = side.angles();
        let mut v = View { target: bounds.center(), yaw, pitch, ly_per_px: 1.0 };
        v.fit(bounds, width, height);
        v
    }

    /// Centers on `bounds` and scales so all of it fits, keeping the direction.
    pub fn fit(&mut self, bounds: Aabb, width: u32, height: u32) {
        let (_, right, up) = self.basis();
        let half = bounds.size() * 0.5;
        let reach = |axis: DVec3| half.x * axis.x.abs() + half.y * axis.y.abs() + half.z * axis.z.abs();
        self.target = bounds.center();
        self.ly_per_px = (2.0 * reach(right) / f64::from(width.max(1))).max(2.0 * reach(up) / f64::from(height.max(1))) * 1.05;
    }
}

/// Surface brightness, light per ly² as linear sRGB, row by row from the top.
#[derive(Clone, Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<[f32; 3]>,
}

/// Integrates `volume` along each pixel's ray, in steps of half a voxel.
pub fn march(volume: &Volume, view: &View, width: u32, height: u32, threads: usize) -> Picture {
    march_until(volume, view, width, height, threads, &Progress::default()).expect("never stopped")
}

/// [`march`], counting rows into `progress` and giving up with `None` once its flag is set.
pub fn march_until(volume: &Volume, view: &View, width: u32, height: u32, threads: usize, progress: &Progress) -> Option<Picture> {
    let (forward, right, up) = view.basis();
    let b = volume.bounds();
    let step = volume.voxel_ly * 0.5;
    let rows: Vec<u32> = (0..height).collect();
    progress.total.store(height, Ordering::Relaxed);
    let pixels = par_map(&rows, threads, |&y| {
        if progress.stopped() {
            return None;
        }
        let row = (0..width)
            .map(|x| {
                let off = (f64::from(x) + 0.5 - f64::from(width) * 0.5) * right + (f64::from(height) * 0.5 - f64::from(y) - 0.5) * up;
                let Some((t0, t1)) = through(b, view.target + off * view.ly_per_px, forward) else { return [0.0; 3] };
                let origin = view.target + off * view.ly_per_px;
                let mut sum = [0.0; 3];
                let mut t = t0 + step * 0.5;
                while t < t1 {
                    let e = volume.at(origin + forward * t);
                    for k in 0..3 {
                        sum[k] += e[k];
                    }
                    t += step;
                }
                sum.map(|s| (s * step) as f32)
            })
            .collect::<Vec<_>>();
        progress.done.fetch_add(1, Ordering::Relaxed);
        Some(row)
    });
    let pixels: Option<Vec<Vec<[f32; 3]>>> = pixels.into_iter().collect();
    Some(Picture { width, height, pixels: pixels?.into_iter().flatten().collect() })
}

/// Where the line `origin + t·dir` is inside `b`, for any `t`, as `(entry, exit)`.
fn through(b: Aabb, origin: DVec3, dir: DVec3) -> Option<(f64, f64)> {
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for k in 0..3 {
        if dir[k].abs() < 1e-12 {
            if origin[k] < b.min[k] || origin[k] > b.max[k] {
                return None;
            }
            continue;
        }
        let (a, c) = ((b.min[k] - origin[k]) / dir[k], (b.max[k] - origin[k]) / dir[k]);
        t0 = t0.max(a.min(c));
        t1 = t1.min(a.max(c));
    }
    (t1 > t0).then_some((t0, t1))
}

impl Picture {
    /// `∫ brightness dA`: the light the picture caught.
    pub fn total(&self, ly_per_px: f64) -> Rgb {
        let da = ly_per_px * ly_per_px;
        let mut out = [0.0; 3];
        for p in &self.pixels {
            for k in 0..3 {
                out[k] += f64::from(p[k]) * da;
            }
        }
        out
    }

    /// RGBA, sRGB-encoded, mapped in OKLCh. A pixel's lightness is the square root of a level
    /// logarithmic in its luminance over `decades` below the brightest pixels (the 99.9th percentile, so a lone hot pixel does
    /// not darken the rest); its hue is its light's; and its chroma keeps the ratio to lightness
    /// its light has at full brightness, times `saturation`. Mapped in sRGB, a dim warm pixel
    /// came out brown and a dim pale one gray; in OKLCh it stays the color it is, only darker.
    pub fn to_rgba8(&self, decades: f64, saturation: f64) -> Vec<u8> {
        encode(&self.display(decades, saturation).0)
    }

    /// [`Picture::to_rgba8`] before encoding: linear display colors, for drawing stars over, and
    /// the luminance at the top of the window.
    pub fn display(&self, decades: f64, saturation: f64) -> (Vec<[f64; 3]>, f64) {
        use em_spectra::oklab;
        let luminance = |p: &[f32; 3]| 0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]);
        let mut lit: Vec<f64> = self.pixels.iter().map(luminance).filter(|&l| l > 0.0).collect();
        lit.sort_by(f64::total_cmp);
        let peak = lit.get(((lit.len() as f64 * 0.999) as usize).min(lit.len().saturating_sub(1))).copied().unwrap_or(1.0);
        let mut out = Vec::with_capacity(self.pixels.len());
        for p in &self.pixels {
            let y = luminance(p);
            // The square root lifts the middle of the range as sRGB's encoding lifted it before,
            // so a disc an order of magnitude below the arms is not drawn near black.
            let level = if y > 0.0 { (1.0 + (y / peak).log10() / decades).clamp(0.0, 1.0) } else { 0.0 };
            let lightness = level.sqrt();
            let top = f64::from(p.iter().copied().fold(0.0f32, f32::max));
            let rgb = if lightness == 0.0 || top <= 0.0 {
                [0.0; 3]
            } else {
                let [l0, a0, b0] = oklab::from_linear_srgb(p.map(|c| f64::from(c) / top));
                let scale = saturation * lightness / l0.max(1e-6);
                in_gamut([lightness, a0 * scale, b0 * scale])
            };
            out.push(rgb);
        }
        (out, peak)
    }
}

/// RGBA, sRGB-encoded, from linear display colors; above 1 clips to white.
pub fn encode(display: &[[f64; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(display.len() * 4);
    for c in display {
        for v in c {
            out.push((em_spectra::cie::encode_srgb(v.clamp(0.0, 1.0)) * 255.0).round() as u8);
        }
        out.push(255);
    }
    out
}

/// Doc 34's `L_A`, L☉: stars brighter than this are seen from across the galaxy, so a picture
/// draws them as points over the light of the rest.
pub const BRIGHT_LUMINOSITY: f64 = 100.0;

/// A star a picture draws as a point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bright {
    pub position_ly: DVec3,
    pub luminosity_solar: f64,
    pub teff_k: f64,
}

/// Draws `stars` over `display`, a `width` × `height` picture seen through `view`, as the game's
/// sky draws distant stars ([`em_spectra::point::DISTANT`]), in the game's colors
/// ([`em_spectra::presets::natural`]).
///
/// Exposed as the picture is: a star's light falls into one pixel, so it reads as its
/// luminosity over the pixel's area against `peak`, the luminance at the top of the picture's
/// window, shifted by `exposure` stops. The brightest stars are then many stops over and glare,
/// as they do in the sky. `screen_px` is screen pixels per picture pixel, since a sprite's size
/// is in screen pixels.
#[allow(clippy::too_many_arguments)]
pub fn sprinkle(display: &mut [[f64; 3]], width: u32, height: u32, view: &View, stars: &[Bright], peak: f64, exposure: f64, screen_px: f64) {
    use em_spectra::point::DISTANT;
    let mapping = em_spectra::presets::natural();
    let area = view.ly_per_px * view.ly_per_px;
    let shown: Vec<(&Bright, [f32; 3])> = stars
        .iter()
        .filter(|s| s.luminosity_solar >= BRIGHT_LUMINOSITY && s.teff_k > 0.0)
        .map(|s| {
            // Band light per unit of output, times the output: a star's light at any one distance.
            let per_band = em_spectra::blackbody::per_band(s.teff_k);
            let total = em_spectra::blackbody::radiant_exitance(s.teff_k) / std::f64::consts::PI;
            (s, mapping.apply_f64(&per_band).map(|c| (c / total * s.luminosity_solar / area) as f32))
        })
        .collect();
    let reference = (peak * 2f64.powf(-exposure)) as f32;
    let (_, right, up) = view.basis();
    for (star, linear) in shown {
        let Some(sprite) = DISTANT.sprite(linear, reference) else { continue };
        let d = star.position_ly - view.target;
        let (sx, sy) = (f64::from(width) * 0.5 + d.dot(right) / view.ly_per_px, f64::from(height) * 0.5 - d.dot(up) / view.ly_per_px);
        // At least most of a pixel, so a star in a coarse picture still lands somewhere.
        let radius = (f64::from(sprite.radius_px) / screen_px).max(0.7);
        let (x0, x1) = ((sx - radius).floor().max(0.0) as i64, (sx + radius).ceil().min(f64::from(width)) as i64);
        let (y0, y1) = ((sy - radius).floor().max(0.0) as i64, (sy + radius).ceil().min(f64::from(height)) as i64);
        for y in y0..y1 {
            for x in x0..x1 {
                let r = ((x as f64 + 0.5 - sx).hypot(y as f64 + 0.5 - sy) / radius) as f32;
                let w = f64::from(DISTANT.profile(&sprite, r));
                if w > 0.0 {
                    let px = &mut display[(y as u32 * width + x as u32) as usize];
                    for k in 0..3 {
                        px[k] += f64::from(sprite.color[k]) * w;
                    }
                }
            }
        }
    }
}

/// Linear sRGB for an OKLab color, its chroma reduced by bisection until it fits the gamut.
fn in_gamut([l, a, b]: [f64; 3]) -> [f64; 3] {
    let fits = |k: f64| {
        let c = em_spectra::oklab::to_linear_srgb([l, a * k, b * k]);
        c.iter().all(|v| (-1e-9..=1.0 + 1e-9).contains(v)).then_some(c)
    };
    if let Some(c) = fits(1.0) {
        return c;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        if fits(mid).is_some() { lo = mid } else { hi = mid }
    }
    fits(lo).unwrap_or([l.clamp(0.0, 1.0); 3]).map(|v| v.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ray_crosses_a_box_where_it_should() {
        let b = Aabb { min: DVec3::splat(-1.0), max: DVec3::splat(1.0) };
        assert_eq!(through(b, DVec3::new(0.0, 0.0, -5.0), DVec3::Z), Some((4.0, 6.0)));
        assert_eq!(through(b, DVec3::new(3.0, 0.0, -5.0), DVec3::Z), None);
    }

    #[test]
    fn each_side_looks_along_its_axis() {
        for side in Side::ALL {
            let (yaw, pitch) = side.angles();
            let (forward, right, up) = View { target: DVec3::ZERO, yaw, pitch, ly_per_px: 1.0 }.basis();
            let mut toward = DVec3::ZERO;
            toward[side.axis] = if side.positive { -1.0 } else { 1.0 };
            assert!(forward.distance(toward) < 1e-12, "{}: {forward}", side.label());
            assert!((right.length() - 1.0).abs() < 1e-12 && right.dot(forward).abs() < 1e-12 && (up.length() - 1.0).abs() < 1e-12);
        }
    }

    /// A bright star lands at its projected pixel, brighter with exposure, and a star under
    /// [`BRIGHT_LUMINOSITY`] is not drawn.
    #[test]
    fn bright_stars_land_where_they_project() {
        let view = View { target: DVec3::ZERO, yaw: -std::f64::consts::FRAC_PI_2, pitch: std::f64::consts::FRAC_PI_2, ly_per_px: 1.0 };
        let draw = |l: f64, exposure: f64| {
            let mut display = vec![[0.0; 3]; 21 * 21];
            let star = Bright { position_ly: DVec3::new(4.0, 2.0, 0.0), luminosity_solar: l, teff_k: 20_000.0 };
            sprinkle(&mut display, 21, 21, &view, &[star], 100.0, exposure, 1.0);
            display
        };
        let at = |d: &[[f64; 3]], x: usize, y: usize| d[y * 21 + x].iter().sum::<f64>();
        // +X right and +Y up from +Z: (4, 2) is four right of center and two up.
        let (dim, bright) = (draw(1000.0, 0.0), draw(1000.0, 8.0));
        assert!(at(&dim, 14, 8) > 0.0 && at(&dim, 10, 10) == 0.0, "the star is not at its pixel");
        assert!(at(&bright, 14, 8) > at(&dim, 14, 8));
        assert!(draw(50.0, 8.0).iter().all(|p| p.iter().sum::<f64>() == 0.0), "a faint star was drawn");
    }
}
