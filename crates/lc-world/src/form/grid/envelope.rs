//! The field's envelope: the smallest ellipsoid on the ship's axes containing every part, grown by
//! `envelope_margin` on each. 29 §What the server computes.
//!
//! The parts are sampled where a containing ellipsoid can touch them: every primitive is an
//! ellipsoid, or points or circles swept by a ball, and only those are extreme. The ellipsoid is
//! then the smallest by volume holding the samples, a convex problem in `w = 1/a²` and `u = w c`
//! solved by a log barrier, over the samples that turn out to matter. Its area is closed form in
//! Carlson's integrals. Only IEEE arithmetic, `sqrt` and `libm`, so every machine agrees to the bit.

use std::f64::consts::PI;

use glam::DVec3;

use crate::fitting::Balance;
use crate::form::primitive::Shape;
use crate::form::sdf::Sdf;

/// Directions sampled over a whole ellipsoid part, and over each ball a point is swept by.
const PART_DIRECTIONS: usize = 4096;
const BALL_DIRECTIONS: usize = 768;
/// Around a circle: a rim, or a torus's axis.
const RIM_STEPS: usize = 512;
const TORUS_STEPS: usize = 128;
/// Samples the barrier starts from; the rest join only if the solution leaves them outside, the
/// worst this many at a time. Only a handful ever touch.
const FIRST_SET: usize = 64;
const JOINING: usize = 32;
/// Samples this far inside at a round's solution are dropped from the next; they rejoin if it
/// leaves them outside.
const KEPT_ABOVE: f64 = 0.9;
/// Rounds of joining before the last is simply scaled to hold every sample.
const ROUNDS: usize = 64;
const NEWTON_STEPS: usize = 50;
const HALVINGS: i32 = 40;
/// Of the log volume: the barrier's duality gap at which it stops.
const GAP: f64 = 1.0e-10;

/// Meters, ship frame. Its axes are the ship's: nose, beam and up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Envelope {
    pub center: DVec3,
    pub semi_axes: DVec3,
    /// What was added to each semi-axis: `envelope_margin` times the cube root of hull volume.
    pub margin_m: f64,
}

impl Envelope {
    pub fn of(sdf: &Sdf, balance: &Balance) -> Envelope {
        // Closed forms, every copy, overlaps counted twice, as F6 has always read it.
        let volume_m3: f64 = sdf.pieces().iter().map(|p| p.shape.volume()).sum();
        let margin_m = balance.envelope_margin * libm::cbrt(volume_m3);
        let (lo, hi) = sdf.bounds();
        let points = extremes(sdf);
        let (center, semi_axes) = smallest(&points, (lo + hi) / 2.0, ((hi - lo) / 2.0).max(DVec3::splat(1e-9)));
        Envelope { center, semi_axes: semi_axes + margin_m, margin_m }
    }

    /// Signed, negative inside, Lipschitz 1: the ellipsoid primitive's bound.
    pub fn distance(&self, p: DVec3) -> f64 {
        Shape::Ellipsoid { semi_axes: self.semi_axes }.distance(p - self.center)
    }

    pub fn volume_m3(&self) -> f64 {
        4.0 / 3.0 * PI * self.semi_axes.element_product()
    }

    /// `3V R_G(a⁻², b⁻², c⁻²)`: DLMF 19.33.1.
    pub fn area_m2(&self) -> f64 {
        let [x, y, z] = self.semi_axes.to_array().map(|a| 1.0 / (a * a));
        3.0 * self.volume_m3() * rg(x, y, z)
    }

    /// The longest diameter: `length_m`.
    pub fn extent_m(&self) -> f64 {
        2.0 * self.semi_axes.max_element()
    }
}

/// Unit vectors on a Fibonacci spiral, evenly spread.
fn spiral(n: usize) -> impl Iterator<Item = DVec3> {
    let golden = PI * (3.0 - libm::sqrt(5.0));
    (0..n).map(move |i| {
        let z = 1.0 - (2 * i + 1) as f64 / n as f64;
        let r = libm::sqrt((1.0 - z * z).max(0.0));
        let theta = golden * i as f64;
        DVec3::new(r * libm::cos(theta), r * libm::sin(theta), z)
    })
}

/// A circle about the part's x axis at `x`, of `radius`.
fn circle(x: f64, radius: f64, steps: usize) -> impl Iterator<Item = DVec3> {
    (0..steps).map(move |j| {
        let theta = 2.0 * PI * j as f64 / steps as f64;
        DVec3::new(x, radius * libm::cos(theta), radius * libm::sin(theta))
    })
}

/// Every point of every part a containing ellipsoid can touch, ship frame.
fn extremes(sdf: &Sdf) -> Vec<DVec3> {
    let mut out = Vec::new();
    let ball: Vec<DVec3> = spiral(BALL_DIRECTIONS).collect();
    for piece in sdf.pieces() {
        let mut local = Vec::new();
        let swept = |local: &mut Vec<DVec3>, cores: &[DVec3], radius: f64| {
            if radius > 0.0 {
                local.extend(cores.iter().flat_map(|c| ball.iter().map(move |d| *c + *d * radius)));
            } else {
                local.extend_from_slice(cores);
            }
        };
        match piece.shape {
            Shape::Ellipsoid { semi_axes } => local.extend(spiral(PART_DIRECTIONS).map(|d| d * semi_axes)),
            Shape::Capsule { radius, length } => swept(&mut local, &[DVec3::X * (-length / 2.0), DVec3::X * (length / 2.0)], radius),
            Shape::Slab { edges, corner } => {
                let h = edges / 2.0 - corner;
                let corners: Vec<DVec3> = (0..8)
                    .map(|c| DVec3::new(if c & 1 == 0 { -h.x } else { h.x }, if c & 2 == 0 { -h.y } else { h.y }, if c & 4 == 0 { -h.z } else { h.z }))
                    .collect();
                swept(&mut local, &corners, corner);
            }
            Shape::Cylinder { radius, length } => {
                local.extend(circle(-length / 2.0, radius, RIM_STEPS).chain(circle(length / 2.0, radius, RIM_STEPS)));
            }
            Shape::Torus { major, minor } => {
                let axis: Vec<DVec3> = circle(0.0, major, TORUS_STEPS).collect();
                swept(&mut local, &axis, minor);
            }
            Shape::Frustum { length, start, end } => {
                local.extend(circle(-length / 2.0, start, RIM_STEPS).chain(circle(length / 2.0, end, RIM_STEPS)));
            }
        }
        out.extend(local.into_iter().map(|p| piece.pose.position + piece.pose.rotation * p));
    }
    out
}

/// The smallest axis-aligned ellipsoid holding `points`, as `(center, semi-axes)`. Solved in the
/// box `center ± half`, where every point is within one of the origin on each axis.
fn smallest(points: &[DVec3], center: DVec3, half: DVec3) -> (DVec3, DVec3) {
    let unit: Vec<DVec3> = points.iter().map(|p| (*p - center) / half).collect();
    if unit.is_empty() {
        return (center, half);
    }
    let stride = unit.len().div_ceil(FIRST_SET);
    let mut working: Vec<DVec3> = unit.iter().step_by(stride).copied().collect();
    // Holds the whole box, so every point, strictly.
    let mut z = [0.3, 0.3, 0.3, 0.0, 0.0, 0.0];
    for _ in 0..ROUNDS {
        z = barrier(&working, z);
        let reach = |p: &DVec3| g(&z, *p);
        let worst = unit.iter().map(reach).fold(0.0, f64::max);
        if worst <= 1.0 + 1.0e-9 {
            break;
        }
        // The worst outside join what still nearly touches, and the start is grown until it holds
        // them all strictly.
        let mut outside: Vec<(f64, DVec3)> = unit.iter().map(|p| (reach(p), *p)).filter(|(g, _)| *g > 1.0).collect();
        outside.sort_by(|a, b| b.0.total_cmp(&a.0));
        working.retain(|p| reach(p) > KEPT_ABOVE);
        working.extend(outside.iter().take(JOINING).map(|(_, p)| *p));
        for w in &mut z {
            *w /= worst * 1.01;
        }
    }
    // Every sample inside to rounding: the barrier stops a hair short of the boundary.
    let worst = unit.iter().map(|p| g(&z, *p)).fold(0.0, f64::max);
    let w = DVec3::new(z[0], z[1], z[2]) / worst;
    let c = DVec3::new(z[3], z[4], z[5]) / DVec3::new(z[0], z[1], z[2]);
    (center + c * half, half / DVec3::new(w.x.sqrt(), w.y.sqrt(), w.z.sqrt()))
}

/// `Σ wᵢ (pᵢ − cᵢ)²` with `c = u / w`: one at the ellipsoid's surface.
fn g(z: &[f64; 6], p: DVec3) -> f64 {
    let p = p.to_array();
    (0..3).map(|i| z[i] * p[i] * p[i] - 2.0 * z[i + 3] * p[i] + z[i + 3] * z[i + 3] / z[i]).sum()
}

/// Maximizes `Σ log wᵢ` subject to every `g < 1`, from a strictly feasible `z`.
fn barrier(points: &[DVec3], mut z: [f64; 6]) -> [f64; 6] {
    let mut t = 1.0;
    let f = |z: &[f64; 6], t: f64| -> Option<f64> {
        if z[..3].iter().any(|w| *w <= 0.0) {
            return None;
        }
        let mut sum = -t * (libm::log(z[0]) + libm::log(z[1]) + libm::log(z[2]));
        for p in points {
            let slack = 1.0 - g(z, *p);
            if slack <= 0.0 {
                return None;
            }
            sum -= libm::log(slack);
        }
        Some(sum)
    };
    while points.len() as f64 / t > GAP {
        for _ in 0..NEWTON_STEPS {
            let (grad, hess) = derivatives(points, &z, t);
            let Some(step) = solve(hess, grad.map(|x| -x)) else { break };
            // The Newton decrement, which is what is left to gain, whatever the barrier's scale.
            let decrement: f64 = (0..6).map(|i| -grad[i] * step[i]).sum();
            if decrement < 1.0e-9 {
                break;
            }
            let here = f(&z, t).expect("the barrier stays feasible");
            let taken = (0..HALVINGS).map(|k| 0.5f64.powi(k)).find_map(|alpha| {
                let next: [f64; 6] = std::array::from_fn(|i| z[i] + alpha * step[i]);
                f(&next, t).is_some_and(|there| there <= here - 0.25 * alpha * decrement).then_some(next)
            });
            // Nothing left that rounding lets it see.
            let Some(next) = taken else { break };
            z = next;
        }
        t *= 16.0;
    }
    z
}

/// The barrier's gradient and Hessian in `(w, u)`, each axis a 2×2 block for `g`.
fn derivatives(points: &[DVec3], z: &[f64; 6], t: f64) -> ([f64; 6], [[f64; 6]; 6]) {
    let mut grad = [0.0; 6];
    let mut hess = [[0.0; 6]; 6];
    for i in 0..3 {
        grad[i] = -t / z[i];
        hess[i][i] = t / (z[i] * z[i]);
    }
    let c: [f64; 3] = std::array::from_fn(|i| z[i + 3] / z[i]);
    for p in points {
        let p = p.to_array();
        let slack = 1.0 - g(z, DVec3::from_array(p));
        let mut dg = [0.0; 6];
        for i in 0..3 {
            dg[i] = p[i] * p[i] - c[i] * c[i];
            dg[i + 3] = -2.0 * (p[i] - c[i]);
        }
        for a in 0..6 {
            grad[a] += dg[a] / slack;
            for b in 0..6 {
                hess[a][b] += dg[a] * dg[b] / (slack * slack);
            }
        }
        for i in 0..3 {
            let w = z[i];
            hess[i][i] += 2.0 * c[i] * c[i] / w / slack;
            hess[i][i + 3] -= 2.0 * c[i] / w / slack;
            hess[i + 3][i] -= 2.0 * c[i] / w / slack;
            hess[i + 3][i + 3] += 2.0 / w / slack;
        }
    }
    (grad, hess)
}

/// Gaussian elimination with partial pivoting.
fn solve(mut a: [[f64; 6]; 6], mut b: [f64; 6]) -> Option<[f64; 6]> {
    for col in 0..6 {
        let pivot = (col..6).fold(col, |best, r| if a[r][col].abs() > a[best][col].abs() { r } else { best });
        if a[pivot][col] == 0.0 || !a[pivot][col].is_finite() {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for r in col + 1..6 {
            let k = a[r][col] / a[col][col];
            for c in col..6 {
                a[r][c] -= k * a[col][c];
            }
            b[r] -= k * b[col];
        }
    }
    let mut x = [0.0; 6];
    for r in (0..6).rev() {
        let tail: f64 = (r + 1..6).map(|c| a[r][c] * x[c]).sum();
        x[r] = (b[r] - tail) / a[r][r];
    }
    Some(x)
}

/// Carlson's `R_F`, by duplication: Numerical Recipes' `rf`, to about 1e-16.
fn rf(x: f64, y: f64, z: f64) -> f64 {
    let (mut x, mut y, mut z) = (x, y, z);
    loop {
        let (sx, sy, sz) = (x.sqrt(), y.sqrt(), z.sqrt());
        let lambda = sx * (sy + sz) + sy * sz;
        x = 0.25 * (x + lambda);
        y = 0.25 * (y + lambda);
        z = 0.25 * (z + lambda);
        let mean = (x + y + z) / 3.0;
        let (dx, dy, dz) = ((mean - x) / mean, (mean - y) / mean, (mean - z) / mean);
        if dx.abs().max(dy.abs()).max(dz.abs()) < 0.0025 {
            let e2 = dx * dy - dz * dz;
            let e3 = dx * dy * dz;
            return (1.0 + (e2 / 24.0 - 0.1 - 3.0 / 44.0 * e3) * e2 + e3 / 14.0) / mean.sqrt();
        }
    }
}

/// Carlson's `R_D`, likewise `rd`.
fn rd(x: f64, y: f64, z: f64) -> f64 {
    const C1: f64 = 3.0 / 14.0;
    const C2: f64 = 1.0 / 6.0;
    const C3: f64 = 9.0 / 22.0;
    const C4: f64 = 3.0 / 26.0;
    let (mut x, mut y, mut z) = (x, y, z);
    let (mut sum, mut fac) = (0.0, 1.0);
    loop {
        let (sx, sy, sz) = (x.sqrt(), y.sqrt(), z.sqrt());
        let lambda = sx * (sy + sz) + sy * sz;
        sum += fac / (sz * (z + lambda));
        fac *= 0.25;
        x = 0.25 * (x + lambda);
        y = 0.25 * (y + lambda);
        z = 0.25 * (z + lambda);
        let mean = 0.2 * (x + y + 3.0 * z);
        let (dx, dy, dz) = ((mean - x) / mean, (mean - y) / mean, (mean - z) / mean);
        if dx.abs().max(dy.abs()).max(dz.abs()) < 0.0015 {
            let ea = dx * dy;
            let eb = dz * dz;
            let ec = ea - eb;
            let ed = ea - 6.0 * eb;
            let ee = ed + ec + ec;
            let series = 1.0 + ed * (-C1 + 0.25 * C3 * ed - 1.5 * C4 * dz * ee) + dz * (C2 * ee + dz * (-C3 * ec + dz * C4 * ea));
            return 3.0 * sum + fac * series / (mean * mean.sqrt());
        }
    }
}

/// Carlson's `R_G`, DLMF 19.21.10, with the middle argument last so its product term is not a
/// difference of large numbers.
fn rg(x: f64, y: f64, z: f64) -> f64 {
    let mut v = [x, y, z];
    v.sort_by(f64::total_cmp);
    let [x, z, y] = v;
    0.5 * (z * rf(x, y, z) - (x - z) * (y - z) * rd(x, y, z) / 3.0 + (x * y / z).sqrt())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(semi_axes: DVec3) -> Envelope {
        Envelope { center: DVec3::ZERO, semi_axes, margin_m: 0.0 }
    }

    fn relative(a: f64, b: f64) -> f64 {
        (a / b - 1.0).abs()
    }

    /// Against the spheroids' elementary forms, which share nothing with Carlson's.
    #[test]
    fn the_area_is_the_spheroids() {
        assert!(relative(envelope(DVec3::splat(3.0)).area_m2(), 4.0 * PI * 9.0) < 1e-14);
        let (a, c) = (5.0f64, 2.0f64);
        let e = (1.0 - c * c / (a * a)).sqrt();
        let prolate = 2.0 * PI * c * c * (1.0 + a / (c * e) * e.asin());
        assert!(relative(envelope(DVec3::new(a, c, c)).area_m2(), prolate) < 1e-13);
        let oblate = 2.0 * PI * a * a * (1.0 + (1.0 - e * e) / e * e.atanh());
        assert!(relative(envelope(DVec3::new(a, a, c)).area_m2(), oblate) < 1e-13);
    }

    /// A triaxial one against a sum over a fine latitude–longitude mesh of the surface.
    #[test]
    fn the_area_of_a_triaxial_ellipsoid_is_its_surfaces() {
        let r = DVec3::new(4.0, 2.5, 1.0);
        let (n, m) = (800, 1600);
        let at = |i: usize, j: usize| {
            let (theta, phi) = (PI * i as f64 / n as f64, 2.0 * PI * j as f64 / m as f64);
            r * DVec3::new(theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos())
        };
        let mut sum = 0.0;
        for i in 0..n {
            for j in 0..m {
                let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1));
                sum += 0.5 * (b - a).cross(c - a).length() + 0.5 * (c - a).cross(d - a).length();
            }
        }
        assert!(relative(envelope(r).area_m2(), sum) < 1e-5, "{} against {sum}", envelope(r).area_m2());
    }

    /// The smallest ellipsoid round points on a known one is that one, wherever it sits.
    #[test]
    fn the_smallest_round_an_ellipsoids_surface_is_itself() {
        let (r, at) = (DVec3::new(7.0, 3.0, 2.0), DVec3::new(1.0, -2.0, 0.5));
        let points: Vec<DVec3> = spiral(2000).map(|d| at + d * r).collect();
        let (center, semi_axes) = smallest(&points, DVec3::ZERO, DVec3::splat(10.0));
        assert!((center - at).length() < 1e-6 * r.x, "{center}");
        assert!(((semi_axes - r) / r).abs().max_element() < 1e-4, "{semi_axes}");
    }

    /// A box's corners: the smallest axis-aligned ellipsoid through them is the box scaled by √3.
    #[test]
    fn the_smallest_round_a_box_goes_through_its_corners() {
        let h = DVec3::new(5.0, 2.0, 1.0);
        let corners: Vec<DVec3> = (0..8)
            .map(|c| DVec3::new(if c & 1 == 0 { -h.x } else { h.x }, if c & 2 == 0 { -h.y } else { h.y }, if c & 4 == 0 { -h.z } else { h.z }))
            .collect();
        let (center, semi_axes) = smallest(&corners, DVec3::ZERO, h);
        assert!(center.length() < 1e-9);
        assert!(((semi_axes / (h * 3f64.sqrt())) - 1.0).abs().max_element() < 1e-6, "{semi_axes}");
    }
}
