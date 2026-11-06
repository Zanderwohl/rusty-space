//! Where a point lies on its primitive's own surface, in meters a look can lay a pattern in. Each
//! primitive is cut into a few charts, each flat or wrapping, with the bounds a look needs to keep
//! a pattern clear of its edges:
//!
//! - a slab's six faces, each a [`ChartKind::Rect`]; on its four sides v runs up its z, and its
//!   two faces across z are its ends, z being whichever of its axes [`oriented`] picks;
//! - a cylinder's or frustum's side, a [`ChartKind::Band`] around its axis, and its ends, each a
//!   [`ChartKind::Disc`];
//! - a capsule's side, a band, and its hemispheres, each a [`ChartKind::Dome`] laid out about its
//!   pole;
//! - an ellipsoid's middle, a band within [`BAND_LATITUDE`] of its equator, and its caps, domes,
//!   about whichever of its axes [`oriented`] picks;
//! - a torus, one [`ChartKind::Ring`], wrapping both ways.
//!
//! Every primitive's axis is its own x, and a chart's u cross its v is the surface's outward
//! normal. Round charts use mean radii: a frustum's band is stretched around toward its wide end
//! and pinched toward its narrow, and an ellipsoid's band as its sections are not circles.
//!
//! A look that treats a part's ends apart from its sides, as a roof, reads [`ChartPoint::end`]. A
//! round part lying on its side, its axis nearer level than upright, has no end facing up: its
//! band is marked [`ChartPoint::lying`], with its u measured from the line along its top, so a look
//! that wants a roof takes one from the band's upper part, and its ends are sides.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use glam::{DMat3, DVec2, DVec3};
use lc_world::form::primitive::Shape;

/// Where an ellipsoid's band gives way to its caps, as a latitude on the sphere it is stretched
/// from. Low, so an ovoid habitat's roof runs well down its sides. `charts.wgsl` matches it.
pub const BAND_LATITUDE: f64 = 20.0 * PI / 180.0;

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
    /// A cap laid out about its pole, each point its arc from it measured on a sphere of radius
    /// `bounds.y`; `bounds` are `(radius, sphere_radius, 0, 0)`.
    Dome = 4,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartPoint {
    pub uv: DVec2,
    pub kind: ChartKind,
    pub bounds: [f64; 4],
    /// One of the part's ends rather than its side.
    pub end: bool,
    /// The band of a round part lying on its side, its u 0 along the part's top.
    pub lying: bool,
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
/// `-x` at the near, so either end's chart faces out. On a lying part, whose +y is up, v runs up.
fn across(p: DVec3, far: bool, lying: bool) -> DVec2 {
    match (lying, far) {
        (false, _) => DVec2::new(if far { p.y } else { -p.y }, p.z),
        (true, true) => DVec2::new(-p.z, p.y),
        (true, false) => DVec2::new(p.z, p.y),
    }
}

/// Upright enough that a round part's end is its roof: its axis within 60 degrees of up.
const STANDING_COS: f64 = 0.5;

#[derive(Clone, Copy, Debug)]
pub struct Oriented {
    pub shape: Shape,
    pub turn: DMat3,
    pub lying: bool,
}

/// `shape` in the frame its charts are laid out in: evaluate them at `turn * p` and carry a tangent
/// back by the transpose. An ellipsoid or slab takes as its axis whichever of its own runs most
/// along `up` (part frame), so a roof lands on top however the part is turned; a round part lying
/// on its side is turned about its axis to put +y up. Every turn is a rotation.
pub fn oriented(shape: &Shape, up: DVec3) -> Oriented {
    let up = up.normalize_or_zero();
    let row = |i: usize| DVec3::from_array(std::array::from_fn(|k| if k == i { 1.0 } else { 0.0 }));
    let a = up.abs().max_position();
    let (b, c) = ((a + 1) % 3, (a + 2) % 3);
    match *shape {
        // Its axis is x.
        Shape::Ellipsoid { semi_axes } => Oriented {
            shape: Shape::Ellipsoid { semi_axes: DVec3::new(semi_axes[a], semi_axes[b], semi_axes[c]) },
            turn: DMat3::from_cols(row(a), row(b), row(c)).transpose(),
            lying: false,
        },
        // Its axis is z.
        Shape::Slab { edges, corner } => Oriented {
            shape: Shape::Slab { edges: DVec3::new(edges[b], edges[c], edges[a]), corner },
            turn: DMat3::from_cols(row(b), row(c), row(a)).transpose(),
            lying: false,
        },
        Shape::Cylinder { .. } | Shape::Capsule { .. } | Shape::Frustum { .. } if up.x.abs() < STANDING_COS => {
            Oriented { shape: *shape, turn: DMat3::from_rotation_x(-up.z.atan2(up.y)), lying: true }
        }
        _ => Oriented { shape: *shape, turn: DMat3::IDENTITY, lying: false },
    }
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
        // Latitude as `chart_at` measures it, so an off-surface vertex agrees with the bounds.
        Shape::Ellipsoid { semi_axes } => {
            let q = p / semi_axes;
            if q.x.atan2(q.y.hypot(q.z)).abs() <= BAND_LATITUDE { 0 } else { end(p.x > 0.0) }
        }
        Shape::Torus { .. } => 0,
    }
}

/// Any chart answers for any point, so a triangle can be laid out whole in one.
pub fn chart_at(shape: &Shape, chart: u8, p: DVec3, lying: bool) -> ChartPoint {
    let mut point = laid(shape, chart, p, lying);
    if lying {
        point.lying = point.kind == ChartKind::Band;
        point.end = false;
    }
    point
}

fn laid(shape: &Shape, chart: u8, p: DVec3, lying: bool) -> ChartPoint {
    let theta = around(p);
    let far = chart == 2;
    let across = |p: DVec3, far: bool| across(p, far, lying);
    match *shape {
        Shape::Slab { edges, .. } => {
            let axis = (chart / 2) as usize;
            let sign = if chart % 2 == 1 { 1.0 } else { -1.0 };
            // u across, v up the part's z on a side; across x then y on an end.
            let (b, c, flip) = match axis {
                0 => (1, 2, sign),
                1 => (0, 2, -sign),
                _ => (0, 1, sign),
            };
            let half = edges / 2.0;
            ChartPoint {
                uv: DVec2::new(flip * p[b], p[c]),
                kind: ChartKind::Rect,
                bounds: [-half[b], half[b], -half[c], half[c]],
                end: axis == 2,
                lying: false,
            }
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
                // Each point's arc from the pole, so the rim is a quarter turn out.
                _ => {
                    let polar = (if far { p.x - h } else { -p.x - h }).max(0.0);
                    let arc = radius * p.y.hypot(p.z).atan2(polar);
                    dome(across(p, far).normalize_or_zero() * arc, radius * FRAC_PI_2, radius)
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
                    dome(across(q, far).normalize_or_zero() * arc, (FRAC_PI_2 - BAND_LATITUDE) * meridian, meridian)
                }
            }
        }
        Shape::Torus { major, minor } => {
            let tube = p.x.atan2(p.y.hypot(p.z) - major);
            ChartPoint {
                uv: DVec2::new(theta * major, tube * minor),
                kind: ChartKind::Ring,
                bounds: [TAU * major, TAU * minor, 0.0, 0.0],
                end: false,
                lying: false,
            }
        }
    }
}

fn band(theta: f64, radius: f64, v: f64, half: f64) -> ChartPoint {
    ChartPoint { uv: DVec2::new(theta * radius, v), kind: ChartKind::Band, bounds: [TAU * radius, 0.0, -half, half], end: false, lying: false }
}

fn disc(uv: DVec2, radius: f64) -> ChartPoint {
    ChartPoint { uv, kind: ChartKind::Disc, bounds: [radius, 0.0, 0.0, 0.0], end: true, lying: false }
}

fn dome(uv: DVec2, radius: f64, sphere: f64) -> ChartPoint {
    ChartPoint { uv, kind: ChartKind::Dome, bounds: [radius, sphere, 0.0, 0.0], end: true, lying: false }
}

/// A triangle laid out whole in the chart its middle is in, so a pattern never smears across one.
/// Corners are unwrapped to within half a period of the first; the periods taken tell a corner on
/// one side of a seam from the same corner on the other.
pub fn lay_out(shape: &Shape, corners: [DVec3; 3], lying: bool) -> (u8, [(ChartPoint, [i32; 2]); 3]) {
    let chart = chart_of(shape, (corners[0] + corners[1] + corners[2]) / 3.0);
    let laid = corners.map(|p| chart_at(shape, chart, p, lying));
    let (first, periods) = (laid[0].uv, laid[0].periods());
    let turns = |x: f64, near: f64, period: f64| if period > 0.0 { ((near - x) / period).round() as i32 } else { 0 };
    (
        chart,
        laid.map(|mut here| {
            let k = [turns(here.uv.x, first.x, periods.x), turns(here.uv.y, first.y, periods.y)];
            here.uv += DVec2::new(k[0] as f64 * periods.x, k[1] as f64 * periods.y);
            (here, k)
        }),
    )
}

/// `x` less whole periods, to within half of one of `near`; as is for no period.
pub fn unwrapped(x: f64, near: f64, period: f64) -> f64 {
    if period > 0.0 { x - period * ((x - near) / period).round() } else { x }
}

/// The direction u grows along the surface at `p`, part frame.
pub fn tangent(shape: &Shape, chart: u8, p: DVec3, h: f64, lying: bool) -> DVec3 {
    let axes = [DVec3::X, DVec3::Y, DVec3::Z];
    let here = chart_at(shape, chart, p, lying);
    let period = here.periods().x;
    let du = DVec3::from_array(axes.map(|a| {
        let ahead = unwrapped(chart_at(shape, chart, p + a * h, lying).uv.x, here.uv.x, period);
        let behind = unwrapped(chart_at(shape, chart, p - a * h, lying).uv.x, here.uv.x, period);
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
        for (shape, lying) in shapes().into_iter().flat_map(|s| [(s, false), (s, true)]) {
            for p in surface(&shape) {
                let chart = chart_of(&shape, p);
                let h = 1e-4;
                let t = tangent(&shape, chart, p, h, lying);
                let here = chart_at(&shape, chart, p, lying);
                let periods = here.periods();
                let v = |q: DVec3| unwrapped(chart_at(&shape, chart, q, lying).uv.y, here.uv.y, periods.y);
                let dv = DVec3::from_array([DVec3::X, DVec3::Y, DVec3::Z].map(|a| v(p + a * h) - v(p - a * h)));
                let n = DVec3::from_array([DVec3::X, DVec3::Y, DVec3::Z].map(|a| shape.distance(p + a * h) - shape.distance(p - a * h)));
                if dv.length() > 1e-9 {
                    assert!(t.cross(dv).dot(n) > 0.0, "{shape:?} chart {chart} at {p}: u × v is inward");
                }
            }
        }
    }

    /// A meter on the surface is a meter in the chart, where no mean radius stretches it.
    #[test]
    fn a_meter_on_the_surface_is_a_meter_in_the_chart() {
        for shape in &shapes()[..2] {
            for p in surface(shape) {
                let chart = chart_of(shape, p);
                let t = tangent(shape, chart, p, 1e-4, false);
                let here = chart_at(shape, chart, p, false);
                let ahead = chart_at(shape, chart, p + t * 0.01, false);
                let du = unwrapped(ahead.uv.x, here.uv.x, here.periods().x) - here.uv.x;
                assert!((du / 0.01 - 1.0).abs() < 0.02, "{shape:?} chart {chart}: {} m a meter", du / 0.01);
            }
        }
    }

    /// A flat ovoid with z up is capped top and bottom, not at its tips.
    #[test]
    fn an_ellipsoid_is_capped_about_the_axis_most_up() {
        let lying = Shape::Ellipsoid { semi_axes: DVec3::new(50.0, 25.0, 10.0) };
        let Oriented { shape, turn, .. } = oriented(&lying, DVec3::Z);
        let chart = |p: DVec3| chart_at(&shape, chart_of(&shape, turn * p), turn * p, false);
        assert!(chart(DVec3::new(0.0, 0.0, 10.0)).end, "its top is a cap");
        assert!(chart(DVec3::new(0.0, 0.0, -10.0)).end, "its bottom is a cap");
        assert!(!chart(DVec3::new(50.0, 0.0, 0.0)).end, "its tip is on the band");
        assert!(turn.determinant() > 0.0, "a turn, not a reflection");
    }

    /// A slab mounted with its thin x outward and up: its broad faces across x are its ends.
    #[test]
    fn a_slab_takes_its_ends_across_the_axis_most_up() {
        let flat = Shape::Slab { edges: DVec3::new(0.25, 6.3, 12.5), corner: 0.0 };
        let up = DVec3::new(0.8, 0.0, 0.6);
        let Oriented { shape, turn, .. } = oriented(&flat, up);
        let chart = |p: DVec3| chart_at(&shape, chart_of(&shape, turn * p), turn * p, false);
        assert!(chart(DVec3::new(0.125, 1.0, 2.0)).end, "its broad face is an end");
        assert!(!chart(DVec3::new(0.0, 0.0, 6.25)).end, "its narrow edge is a side");
        assert!(turn.determinant() > 0.0, "a turn, not a reflection");
    }

    /// A lying cylinder's band is measured from its top; its ends are sides with v up.
    #[test]
    fn a_lying_cylinder_measures_its_band_from_its_top() {
        let tube = Shape::Cylinder { radius: 8.0, length: 30.0 };
        let o = oriented(&tube, DVec3::new(0.0, 0.6, 0.8));
        assert!(o.lying);
        let at = |p: DVec3| {
            let q = o.turn * p;
            chart_at(&o.shape, chart_of(&o.shape, q), q, true)
        };
        let top = at(DVec3::new(0.0, 0.6, 0.8) * 8.0);
        assert!(top.lying && top.uv.x.abs() < 1e-9, "{top:?}");
        let end = at(DVec3::new(15.0, 0.0, 0.0));
        assert!(!end.end && !end.lying);
        let higher = at(DVec3::new(15.0, 0.6, 0.8));
        assert!(higher.uv.y > end.uv.y, "an end's v runs up");
        assert!(!oriented(&tube, DVec3::X).lying, "standing, it is not");
    }

    /// A lying part's roof, cut in the shader, stops where an ovoid's does.
    #[test]
    fn the_shader_stops_a_lying_roof_at_the_band_latitude() {
        let wgsl = include_str!("../assets/shaders/charts.wgsl");
        let line = wgsl.lines().find(|l| l.starts_with("const ROOF_ELEVATION")).expect("ROOF_ELEVATION is declared");
        let value: f64 = line.split('=').nth(1).and_then(|v| v.trim().trim_end_matches(';').parse().ok()).expect("a number");
        assert!((value - BAND_LATITUDE).abs() < 1e-6, "{value} against {BAND_LATITUDE}");
    }

    #[test]
    fn a_band_wraps_once_around() {
        let shape = Shape::Cylinder { radius: 8.0, length: 30.0 };
        let a = chart_at(&shape, 0, DVec3::new(0.0, -8.0, 1e-9), false);
        let b = chart_at(&shape, 0, DVec3::new(0.0, -8.0, -1e-9), false);
        assert!((unwrapped(a.uv.x, b.uv.x, a.periods().x) - b.uv.x).abs() < 1e-6);
        assert!((a.periods().x - TAU * 8.0).abs() < 1e-9);
    }
}
