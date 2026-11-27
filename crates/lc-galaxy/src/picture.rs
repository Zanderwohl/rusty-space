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
        let lights: Vec<Rgb> = compiled.populations.iter().map(|(id, p)| light::mean_light(p, u64::from(id.0))).collect();
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
                    let density = compiled.sample(center).density;
                    let mut e = [0.0f64; 3];
                    for (n, l) in density.iter().zip(&lights) {
                        for k in 0..3 {
                            e[k] += n * l[k];
                        }
                    }
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

    /// RGBA, sRGB-encoded. Brightness is logarithmic over `decades` below the brightest pixels
    /// (the 99.9th percentile, so a lone hot pixel does not darken the rest), and each pixel keeps
    /// its own color by dividing by its brightest channel, as the starfield does.
    pub fn to_rgba8(&self, decades: f64) -> Vec<u8> {
        let lum = |p: &[f32; 3]| f64::from(p[0] + p[1] + p[2]);
        let mut lit: Vec<f64> = self.pixels.iter().map(lum).filter(|&l| l > 0.0).collect();
        lit.sort_by(f64::total_cmp);
        let peak = lit.get(((lit.len() as f64 * 0.999) as usize).min(lit.len().saturating_sub(1))).copied().unwrap_or(1.0);
        let mut out = Vec::with_capacity(self.pixels.len() * 4);
        for p in &self.pixels {
            let l = lum(p);
            let level = if l > 0.0 { (1.0 + (l / peak).log10() / decades).clamp(0.0, 1.0) } else { 0.0 };
            let top = p.iter().copied().fold(0.0f32, f32::max).max(f32::MIN_POSITIVE);
            for &c in p {
                out.push((em_spectra::cie::encode_srgb(f64::from(c / top) * level) * 255.0).round() as u8);
            }
            out.push(255);
        }
        out
    }
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
}
