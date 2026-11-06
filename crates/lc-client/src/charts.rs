//! Where a point lies on its primitive's own surface, in meters a look can lay a pattern in. Each
//! primitive is cut into a few charts, each flat or wrapping, with the bounds a look needs to keep
//! a pattern clear of its edges:
//!
//! - a slab's six faces, each a [`ChartKind::Rect`];
//! - a cylinder's or frustum's side, a [`ChartKind::Band`] around its axis, and its ends, each a
//!   [`ChartKind::Disc`];
//! - a capsule's side, a band, and its hemispheres, each a disc laid out about its pole;
//! - an ellipsoid's middle, a band within [`BAND_LATITUDE`] of its equator, and its caps, discs;
//! - a torus, one [`ChartKind::Ring`], wrapping both ways.
//!
//! Every primitive's axis is its own x, and a chart's u cross its v is the surface's outward
//! normal. Round charts use mean radii: a frustum's band is stretched around toward its wide end
//! and pinched toward its narrow, and an ellipsoid's band as its sections are not circles.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use glam::{DVec2, DVec3};
use lc_world::form::primitive::Shape;

/// How far from its equator an ellipsoid's band runs, either way.
pub const BAND_LATITUDE: f64 = 35.0 * PI / 180.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartKind {
    /// `bounds` are `(u_lo, u_hi, v_lo, v_hi)`.
    Rect = 0,
    /// About the origin; `bounds` are `(radius, 0, 0, 0)`.
    Disc = 1,
    /// u wraps; `bounds` are `(u_period, 0, v_lo, v_hi)`.
    Band = 2,
    /// u and v both wrap; `bounds` are `(u_period, v_period, 0, 0)`.
    Ring = 3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartPoint {
    pub uv: DVec2,
    pub kind: ChartKind,
    pub bounds: [f64; 4],
}

impl ChartPoint {
    /// How far u and v run before they repeat; 0 for one that does not.
    pub fn periods(&self) -> DVec2 {
        match self.kind {
            ChartKind::Band => DVec2::new(self.bounds[0], 0.0),
            ChartKind::Ring => DVec2::new(self.bounds[0], self.bounds[1]),
            _ => DVec2::ZERO,
        }
    }
}

/// The angle about the axis, from +y toward +z.
fn around(p: DVec3) -> f64 {
    p.z.atan2(p.y)
}

/// A point in the plane across the axis, laid out with u cross v along `+x` at the far end and
/// `-x` at the near, so either end's chart faces out.
fn across(p: DVec3, far: bool) -> DVec2 {
    DVec2::new(if far { p.y } else { -p.y }, p.z)
}

/// Which chart of `shape` holds `p`, in the part's frame: the one whose surface is nearest.
pub fn chart_of(shape: &Shape, p: DVec3) -> u8 {
    let rho = p.y.hypot(p.z);
    let end = |far_side: bool| if far_side { 2 } else { 1 };
    match *shape {
        Shape::Slab { edges, .. } => {
            let off = p.abs() - edges / 2.0;
            let axis = if off.x >= off.y && off.x >= off.z { 0 } else if off.y >= off.z { 1 } else { 2 };
            2 * axis as u8 + u8::from(p[axis] > 0.0)
        }
        Shape::Cylinder { radius, length } => {
            if rho - radius >= p.x.abs() - length / 2.0 { 0 } else { end(p.x > 0.0) }
        }
        Shape::Frustum { length, start, end: far } => {
            let h = length / 2.0;
            let r = start + (far - start) * ((p.x + h) / length).clamp(0.0, 1.0);
            if rho - r >= p.x.abs() - h { 0 } else { end(p.x > 0.0) }
        }
        Shape::Capsule { length, .. } => {
            if p.x.abs() <= length / 2.0 { 0 } else { end(p.x > 0.0) }
        }
        Shape::Ellipsoid { semi_axes } => {
            if (p.x / semi_axes.x).abs() <= BAND_LATITUDE.sin() { 0 } else { end(p.x > 0.0) }
        }
        Shape::Torus { .. } => 0,
    }
}

/// Where `p`, in the part's frame, lies in chart `chart` of `shape`. Any chart answers for any
/// point, so a triangle can be laid out whole in one chart.
pub fn chart_at(shape: &Shape, chart: u8, p: DVec3) -> ChartPoint {
    let theta = around(p);
    let far = chart == 2;
    match *shape {
        Shape::Slab { edges, .. } => {
            let axis = (chart / 2) as usize;
            let sign = if chart % 2 == 1 { 1.0 } else { -1.0 };
            let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
            let half = edges / 2.0;
            ChartPoint { uv: DVec2::new(sign * p[b], p[c]), kind: ChartKind::Rect, bounds: [-half[b], half[b], -half[c], half[c]] }
        }
        Shape::Cylinder { radius, length } => match chart {
            0 => band(theta, radius, p.x, length / 2.0),
            _ => disc(across(p, far), radius),
        },
        Shape::Frustum { length, start, end } => match chart {
            0 => {
                let mean = 0.5 * (start + end);
                let slant = length.hypot(end - start) / length;
                band(theta, mean, p.x * slant, length / 2.0 * slant)
            }
            _ => disc(across(p, far), if far { end } else { start }),
        },
        Shape::Capsule { radius, length } => {
            let h = length / 2.0;
            match chart {
                0 => band(theta, radius, p.x, h),
                // Laid out about the pole, each point its arc from it: so the rim is a quarter turn out.
                _ => {
                    let polar = (if far { p.x - h } else { -p.x - h }).max(0.0);
                    let arc = radius * p.y.hypot(p.z).atan2(polar);
                    disc(across(p, far).normalize_or_zero() * arc, radius * FRAC_PI_2)
                }
            }
        }
        Shape::Ellipsoid { semi_axes } => {
            let equator = 0.5 * (semi_axes.y + semi_axes.z);
            // A meridian's mean radius: what an arc along it is measured by.
            let meridian = (0.5 * (semi_axes.x * semi_axes.x + equator * equator)).sqrt();
            let q = p / semi_axes;
            let latitude = q.x.atan2(q.y.hypot(q.z));
            match chart {
                0 => band(theta, equator, latitude * meridian, BAND_LATITUDE * meridian),
                _ => {
                    let arc = (FRAC_PI_2 - latitude.abs()) * meridian;
                    disc(across(q, far).normalize_or_zero() * arc, (FRAC_PI_2 - BAND_LATITUDE) * meridian)
                }
            }
        }
        Shape::Torus { major, minor } => {
            let tube = p.x.atan2(p.y.hypot(p.z) - major);
            ChartPoint { uv: DVec2::new(theta * major, tube * minor), kind: ChartKind::Ring, bounds: [TAU * major, TAU * minor, 0.0, 0.0] }
        }
    }
}

fn band(theta: f64, radius: f64, v: f64, half: f64) -> ChartPoint {
    ChartPoint { uv: DVec2::new(theta * radius, v), kind: ChartKind::Band, bounds: [TAU * radius, 0.0, -half, half] }
}

fn disc(uv: DVec2, radius: f64) -> ChartPoint {
    ChartPoint { uv, kind: ChartKind::Disc, bounds: [radius, 0.0, 0.0, 0.0] }
}

/// `x` less whole periods, to within half of one of `near`; as is for no period.
pub fn unwrapped(x: f64, near: f64, period: f64) -> f64 {
    if period > 0.0 { x - period * ((x - near) / period).round() } else { x }
}

/// The direction u grows along the surface at `p`, in the part's frame: u's gradient laid into the
/// plane square to the surface's.
pub fn tangent(shape: &Shape, chart: u8, p: DVec3, h: f64) -> DVec3 {
    let axes = [DVec3::X, DVec3::Y, DVec3::Z];
    let here = chart_at(shape, chart, p);
    let period = here.periods().x;
    let du = DVec3::from_array(axes.map(|a| {
        let ahead = unwrapped(chart_at(shape, chart, p + a * h).uv.x, here.uv.x, period);
        let behind = unwrapped(chart_at(shape, chart, p - a * h).uv.x, here.uv.x, period);
        ahead - behind
    }));
    let n = DVec3::from_array(axes.map(|a| shape.distance(p + a * h) - shape.distance(p - a * h))).normalize_or_zero();
    (du - n * n.dot(du)).try_normalize().unwrap_or_else(|| n.any_orthonormal_vector())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shapes() -> Vec<Shape> {
        vec![
            Shape::Slab { edges: DVec3::new(30.0, 20.0, 10.0), corner: 1.0 },
            Shape::Cylinder { radius: 8.0, length: 30.0 },
            Shape::Frustum { length: 30.0, start: 10.0, end: 5.0 },
            Shape::Capsule { radius: 8.0, length: 20.0 },
            Shape::Ellipsoid { semi_axes: DVec3::new(20.0, 12.0, 9.0) },
            Shape::Torus { major: 20.0, minor: 5.0 },
        ]
    }

    /// Points on each shape's surface, along rays all round.
    fn surface(shape: &Shape) -> Vec<DVec3> {
        let mut out = Vec::new();
        for i in 0..24 {
            for j in 1..12 {
                let (a, b) = (TAU * i as f64 / 24.0, PI * j as f64 / 12.0);
                let dir = DVec3::new(b.cos(), b.sin() * a.cos(), b.sin() * a.sin());
                // The torus's surface is off its center, so start from the middle of its tube.
                let from = match *shape {
                    Shape::Torus { major, .. } => DVec3::new(0.0, a.cos(), a.sin()) * major,
                    _ => DVec3::ZERO,
                };
                // Inside at the start, outside far off: bisect for where it crosses.
                let (mut inside, mut outside) = (0.0, 1000.0);
                for _ in 0..80 {
                    let mid = 0.5 * (inside + outside);
                    if shape.distance(from + dir * mid) < 0.0 { inside = mid } else { outside = mid }
                }
                out.push(from + dir * inside);
            }
        }
        out
    }

    /// Every chart is right-handed: u, then v, then out.
    #[test]
    fn u_cross_v_is_outward() {
        for shape in shapes() {
            for p in surface(&shape) {
                let chart = chart_of(&shape, p);
                let h = 1e-4;
                let t = tangent(&shape, chart, p, h);
                let here = chart_at(&shape, chart, p);
                let periods = here.periods();
                let v = |q: DVec3| unwrapped(chart_at(&shape, chart, q).uv.y, here.uv.y, periods.y);
                let dv = DVec3::from_array([DVec3::X, DVec3::Y, DVec3::Z].map(|a| v(p + a * h) - v(p - a * h)));
                let n = DVec3::from_array([DVec3::X, DVec3::Y, DVec3::Z].map(|a| shape.distance(p + a * h) - shape.distance(p - a * h)));
                if dv.length() > 1e-9 {
                    assert!(t.cross(dv).dot(n) > 0.0, "{shape:?} chart {chart} at {p}: u × v is inward");
                }
            }
        }
    }

    /// A meter along the surface is about a meter in the chart, where the chart is not stretched
    /// by design: everywhere on a slab, cylinder or capsule.
    #[test]
    fn a_meter_on_the_surface_is_a_meter_in_the_chart() {
        for shape in &shapes()[..2] {
            for p in surface(shape) {
                let chart = chart_of(shape, p);
                let t = tangent(shape, chart, p, 1e-4);
                let here = chart_at(shape, chart, p);
                let ahead = chart_at(shape, chart, p + t * 0.01);
                let du = unwrapped(ahead.uv.x, here.uv.x, here.periods().x) - here.uv.x;
                assert!((du / 0.01 - 1.0).abs() < 0.02, "{shape:?} chart {chart}: {} m a meter", du / 0.01);
            }
        }
    }

    #[test]
    fn a_band_wraps_once_around() {
        let shape = Shape::Cylinder { radius: 8.0, length: 30.0 };
        let a = chart_at(&shape, 0, DVec3::new(0.0, -8.0, 1e-9));
        let b = chart_at(&shape, 0, DVec3::new(0.0, -8.0, -1e-9));
        assert!((unwrapped(a.uv.x, b.uv.x, a.periods().x) - b.uv.x).abs() < 1e-6);
        assert!((a.periods().x - TAU * 8.0).abs() < 1e-9);
    }
}
