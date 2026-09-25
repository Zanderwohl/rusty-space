//! The form's signed distance, in meters in the ship's frame, negative inside.
//!
//! [`Sdf`] resolves poses, shapes, spar neighbors and blend radii once, so [`Sdf::distance`] is
//! arithmetic only. The grid (F6) and the client's mesher both read it. See 29 §Spars conform.
//!
//! The ellipsoid is the Lipschitz-1 bound `(|p/r| − 1) · r_min`, not exact, so a saddle's gap and a
//! strap's depth off it grow away from its shortest axis. The quadratic smooth minimum keeps the
//! whole field Lipschitz 1. 29 §What the server computes has the rest.
//!
//! Only IEEE arithmetic and `sqrt`, like placement, so every machine agrees to the bit.

use std::collections::HashMap;

use glam::{DMat3, DVec2, DVec3};

use super::place::{Pose, Side};
use super::primitive::Shape;
use super::{Form, FormError, Kind, Part, PartId, SparMode};
use crate::fitting::Balance;

/// One copy of one part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub part: PartId,
    pub side: Side,
    pub kind: Kind,
    pub shape: Shape,
    /// From the part's frame to the ship's.
    pub pose: Pose,
}

#[derive(Clone, Debug, PartialEq)]
enum Conform {
    Whole,
    /// Indices of the tree neighbors cut out of it.
    Saddle(Vec<usize>),
    /// `depth` in meters.
    Strap { parent: usize, depth: f64 },
}

/// A form prepared for evaluation.
#[derive(Clone, Debug)]
pub struct Sdf {
    pieces: Vec<Piece>,
    /// Each pose's transpose, taken once.
    inverse: Vec<DMat3>,
    conform: Vec<Conform>,
    /// `(parent, child, radius in meters)` for every joint that is smoothed. Never a spar's.
    blends: Vec<(usize, usize, f64)>,
    spar_gap: f64,
    bounds: (DVec3, DVec3),
}

impl Sdf {
    pub fn new(form: &Form, balance: &Balance) -> Result<Sdf, FormError> {
        let poses = form.place(balance.min_part_m3)?;
        let parts: HashMap<PartId, &Part> = form.parts.iter().map(|p| (p.id, p)).collect();

        let mut index = HashMap::with_capacity(poses.len());
        let mut pieces = Vec::with_capacity(poses.len());
        for (id, side, pose) in poses.iter() {
            let part = parts[&id];
            index.insert((id, side), pieces.len());
            pieces.push(Piece { part: id, side, kind: part.kind, shape: part.shape(balance.min_part_m3), pose: *pose });
        }
        // A mirror that starts at a part hangs its copy from the parent's original.
        let parent: Vec<Option<usize>> = pieces
            .iter()
            .map(|piece| {
                let up = parts[&piece.part].placement?.parent;
                Some(index.get(&(up, piece.side)).copied().unwrap_or_else(|| index[&(up, Side::Original)]))
            })
            .collect();
        let mut children = vec![Vec::new(); pieces.len()];
        for (i, up) in parent.iter().enumerate() {
            if let Some(up) = *up {
                children[up].push(i);
            }
        }

        let is_spar = |i: usize| matches!(pieces[i].kind, Kind::Spar(_));
        let conform = (0..pieces.len())
            .map(|i| match pieces[i].kind {
                Kind::Spar(SparMode::Saddle) => Conform::Saddle(parent[i].into_iter().chain(children[i].clone()).collect()),
                Kind::Spar(SparMode::Strap) => {
                    let up = parent[i].expect("only the Mind is unplaced");
                    Conform::Strap { parent: up, depth: balance.spar_thickness * pieces[up].shape.least_dimension() }
                }
                _ => Conform::Whole,
            })
            .collect();

        let mut blends = Vec::new();
        for (child, up) in parent.iter().enumerate() {
            let Some(up) = *up else { continue };
            if is_spar(child) || is_spar(up) {
                continue;
            }
            let blend = parts[&pieces[child].part].placement.expect("has a parent").blend;
            let smaller = pieces[child].shape.least_dimension().min(pieces[up].shape.least_dimension());
            let radius = blend * smaller;
            // One that overflows is dropped rather than turning the whole field to NaN.
            if radius > 0.0 && radius.is_finite() {
                blends.push((up, child, radius));
            }
        }

        let mut margin = vec![0.0f64; pieces.len()];
        for &(up, child, radius) in &blends {
            margin[up] = margin[up].max(radius / 4.0);
            margin[child] = margin[child].max(radius / 4.0);
        }
        let mut bounds = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for (piece, margin) in pieces.iter().zip(margin) {
            let half = piece.shape.extent(piece.pose.rotation, margin);
            bounds = (bounds.0.min(piece.pose.position - half), bounds.1.max(piece.pose.position + half));
        }

        let inverse = pieces.iter().map(|p| p.pose.rotation.transpose()).collect();
        Ok(Sdf { pieces, inverse, conform, blends, spar_gap: balance.spar_gap, bounds })
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// Contains every point where [`Sdf::distance`] is not positive. Ship frame, `(min, max)`.
    pub fn bounds(&self) -> (DVec3, DVec3) {
        self.bounds
    }

    /// Allocates its scratch; a loop wants [`Sdf::distance_with`].
    pub fn distance(&self, p: DVec3) -> f64 {
        self.distance_with(p, &mut Vec::new())
    }

    /// [`Sdf::distance`] reusing `scratch` across calls, for a loop over a grid.
    pub fn distance_with(&self, p: DVec3, scratch: &mut Vec<f64>) -> f64 {
        let raw = self.raw(p, scratch);
        let hard = (0..raw.len()).map(|i| self.conformed(i, raw)).fold(f64::INFINITY, f64::min);
        // A spar is never blended, so both ends of a joint are their primitives.
        self.blends.iter().fold(hard, |d, &(a, b, radius)| d.min(smooth_min(raw[a], raw[b], radius)))
    }

    /// The piece whose own distance at `p` is least, and that distance. Where two are blended the
    /// fillet goes to whichever is nearer.
    pub fn nearest(&self, p: DVec3) -> (&Piece, f64) {
        self.nearest_with(p, &mut Vec::new())
    }

    pub fn nearest_with(&self, p: DVec3, scratch: &mut Vec<f64>) -> (&Piece, f64) {
        let raw = self.raw(p, scratch);
        let (i, d) = (0..raw.len())
            .map(|i| (i, self.conformed(i, raw)))
            .fold((0, f64::INFINITY), |best, next| if next.1 < best.1 { next } else { best });
        (&self.pieces[i], d)
    }

    /// One piece alone, with a spar cut by its neighbors, and nothing blended. Reads only that
    /// piece and its tree neighbors.
    pub fn piece_distance(&self, piece: usize, p: DVec3) -> f64 {
        let own = self.primitive(piece, p);
        match &self.conform[piece] {
            Conform::Whole => own,
            Conform::Saddle(neighbors) => {
                neighbors.iter().fold(own, |d, &n| d.max(self.spar_gap - self.primitive(n, p)))
            }
            &Conform::Strap { parent, depth } => strap(own, self.primitive(parent, p), depth),
        }
    }

    /// For a spar, the neighbor whose seam with it is nearest `p`, and how near: zero on the line
    /// where the spar's primitive meets a saddle's cut or a strap's parent, and at most the
    /// distance to that line off it. `None` for any other part. 32 §Materials by kind puts a row of
    /// bolts along it.
    pub fn seam(&self, piece: usize, p: DVec3) -> Option<(usize, f64)> {
        let own = self.primitive(piece, p).abs();
        let on = |n: usize, other: f64| (n, own.max(other.abs()));
        let nearer = |a: (usize, f64), b: (usize, f64)| if b.1 < a.1 { b } else { a };
        match &self.conform[piece] {
            Conform::Whole => None,
            Conform::Saddle(neighbors) => neighbors
                .iter()
                .map(|&n| on(n, self.spar_gap - self.primitive(n, p)))
                .reduce(nearer),
            &Conform::Strap { parent, .. } => Some(on(parent, self.primitive(parent, p))),
        }
    }

    /// Distance to one piece's primitive, uncut and unblended.
    pub fn primitive(&self, piece: usize, p: DVec3) -> f64 {
        self.pieces[piece].shape.distance(self.inverse[piece] * (p - self.pieces[piece].pose.position))
    }

    fn raw<'a>(&self, p: DVec3, scratch: &'a mut Vec<f64>) -> &'a [f64] {
        scratch.clear();
        scratch.extend((0..self.pieces.len()).map(|i| self.primitive(i, p)));
        scratch
    }

    fn conformed(&self, i: usize, raw: &[f64]) -> f64 {
        match &self.conform[i] {
            Conform::Whole => raw[i],
            Conform::Saddle(neighbors) => neighbors.iter().fold(raw[i], |d, &n| d.max(self.spar_gap - raw[n])),
            &Conform::Strap { parent, depth } => strap(raw[i], raw[parent], depth),
        }
    }
}

/// Kept only between the parent's surface and `depth` outside it.
fn strap(own: f64, parent: f64, depth: f64) -> f64 {
    own.max(-parent).max(parent - depth)
}

/// At most `radius / 4` below `min(a, b)`, and equal to it once they differ by `radius`.
fn smooth_min(a: f64, b: f64, radius: f64) -> f64 {
    let h = (radius - (a - b).abs()).max(0.0) / radius;
    a.min(b) - h * h * radius / 4.0
}

impl Shape {
    /// Signed distance from `p` in the part's frame. Exact except for the ellipsoid, which is a
    /// bound; see the module.
    pub fn distance(&self, p: DVec3) -> f64 {
        let rho = (p.y * p.y + p.z * p.z).sqrt();
        match *self {
            Shape::Ellipsoid { semi_axes } => ((p / semi_axes).length() - 1.0) * semi_axes.min_element(),
            Shape::Capsule { radius, length } => {
                let h = length / 2.0;
                (p - DVec3::X * p.x.clamp(-h, h)).length() - radius
            }
            Shape::Slab { edges, corner } => {
                let q = p.abs() - (edges / 2.0 - corner);
                q.max(DVec3::ZERO).length() + q.max_element().min(0.0) - corner
            }
            Shape::Cylinder { radius, length } => {
                let q = DVec2::new(p.x.abs() - length / 2.0, rho - radius);
                q.max(DVec2::ZERO).length() + q.max_element().min(0.0)
            }
            Shape::Torus { major, minor } => DVec2::new(rho - major, p.x).length() - minor,
            Shape::Frustum { length, start, end } => frustum(DVec2::new(rho, p.x), length / 2.0, start, end),
        }
    }

    /// The smallest of its extents, in meters: what a strap's depth and a blend's radius are
    /// fractions of.
    pub fn least_dimension(&self) -> f64 {
        match *self {
            Shape::Ellipsoid { semi_axes } => 2.0 * semi_axes.min_element(),
            Shape::Capsule { radius, .. } => 2.0 * radius,
            Shape::Slab { edges, .. } => edges.min_element(),
            Shape::Cylinder { radius, length } => (2.0 * radius).min(length),
            Shape::Torus { minor, .. } => 2.0 * minor,
            Shape::Frustum { length, start, end } => (2.0 * start.max(end)).min(length),
        }
    }

    /// Half the box, along the axes of the frame `rotation` turns the part into, around where
    /// [`Shape::distance`] is at most `margin`. Exact for all but the frustum, which takes its
    /// wider end at both.
    fn extent(&self, rotation: DMat3, margin: f64) -> DVec3 {
        let axis = rotation.x_axis.abs();
        // How far a unit circle across the axis reaches along each outer axis.
        let across = (DVec3::ONE - axis * axis).max(DVec3::ZERO).map(f64::sqrt);
        match *self {
            // The bound's level sets are the ellipsoid scaled, not offset.
            Shape::Ellipsoid { semi_axes } => {
                let s = semi_axes * (1.0 + margin / semi_axes.min_element());
                let m = rotation * DMat3::from_diagonal(s);
                DVec3::from_array([0, 1, 2].map(|i| m.row(i).length()))
            }
            Shape::Capsule { radius, length } => axis * (length / 2.0) + radius + margin,
            Shape::Slab { edges, corner } => {
                let abs = DMat3::from_cols(axis, rotation.y_axis.abs(), rotation.z_axis.abs());
                abs * (edges / 2.0 - corner) + corner + margin
            }
            Shape::Cylinder { radius, length } => axis * (length / 2.0) + across * radius + margin,
            Shape::Torus { major, minor } => across * major + minor + margin,
            Shape::Frustum { length, start, end } => axis * (length / 2.0) + across * start.max(end) + margin,
        }
    }
}

/// `q` is (distance from the axis, x). The nearer of the end disk on the point's side and the
/// slant edge from `(start, −h)` to `(end, h)`, and inside only if inside both.
fn frustum(q: DVec2, h: f64, start: f64, end: f64) -> f64 {
    let cap = if q.y < 0.0 { start } else { end };
    let to_cap = DVec2::new(q.x - q.x.min(cap), q.y.abs() - h);
    let rim = DVec2::new(end, h);
    let slant = DVec2::new(end - start, 2.0 * h);
    let along = ((rim - q).dot(slant) / slant.length_squared()).clamp(0.0, 1.0);
    let to_slant = q - rim + slant * along;
    let sign = if to_slant.x < 0.0 && to_cap.y < 0.0 { -1.0 } else { 1.0 };
    sign * to_cap.length_squared().min(to_slant.length_squared()).sqrt()
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::form::{Mount, Placement, Primitive};

    const B: Balance = Balance::DEFAULT;

    fn shapes() -> Vec<Shape> {
        [
            Primitive::Ellipsoid { axes: DVec3::new(3.0, 1.0, 2.0) },
            Primitive::Ellipsoid { axes: DVec3::ONE },
            Primitive::Capsule { length: 2.0 },
            Primitive::Capsule { length: 0.0 },
            Primitive::Slab { edges: DVec3::new(2.0, 1.0, 3.0), corner: 0.2 },
            Primitive::Slab { edges: DVec3::new(2.0, 1.0, 3.0), corner: 0.0 },
            Primitive::Slab { edges: DVec3::ONE, corner: 0.5 },
            Primitive::Cylinder { length: 3.0 },
            Primitive::Cylinder { length: 0.2 },
            Primitive::Torus { major: 3.0 },
            Primitive::Torus { major: 1.2 },
            Primitive::Frustum { length: 2.0, taper: 0.5 },
            Primitive::Frustum { length: 2.0, taper: 2.5 },
            Primitive::Frustum { length: 3.0, taper: 0.0 },
            Primitive::Frustum { length: 1.0, taper: 1.0 },
        ]
        .map(|p| p.at(1.7))
        .to_vec()
    }

    /// Membership from each primitive's definition, sharing nothing with [`Shape::distance`].
    fn inside(shape: &Shape, p: DVec3) -> bool {
        let rho = p.y.hypot(p.z);
        match *shape {
            Shape::Ellipsoid { semi_axes } => (p / semi_axes).length_squared() < 1.0,
            Shape::Capsule { radius, length } => {
                let x = p.x.clamp(-length / 2.0, length / 2.0);
                (p - DVec3::X * x).length() < radius
            }
            Shape::Slab { edges, corner } => (p.abs() - (edges / 2.0 - corner)).max(DVec3::ZERO).length() < corner
                || (corner == 0.0 && (p.abs() - edges / 2.0).max_element() < 0.0),
            Shape::Cylinder { radius, length } => p.x.abs() < length / 2.0 && rho < radius,
            Shape::Torus { major, minor } => (rho - major).hypot(p.x) < minor,
            Shape::Frustum { length, start, end } => {
                p.x.abs() < length / 2.0 && rho < start + (end - start) * (p.x / length + 0.5)
            }
        }
    }

    /// Evenly spread unit vectors, a Fibonacci sphere.
    fn directions(n: usize) -> Vec<DVec3> {
        let golden = PI * (3.0 - 5f64.sqrt());
        (0..n)
            .map(|k| {
                let z = 1.0 - 2.0 * (k as f64 + 0.5) / n as f64;
                let (s, c) = (golden * k as f64).sin_cos();
                let w = (1.0 - z * z).sqrt();
                DVec3::new(w * c, w * s, z)
            })
            .collect()
    }

    fn lattice(min: DVec3, max: DVec3, n: usize) -> impl Iterator<Item = DVec3> {
        let step = (max - min) / (n - 1) as f64;
        (0..n * n * n).map(move |k| min + step * DVec3::new((k % n) as f64, (k / n % n) as f64, (k / n / n) as f64))
    }

    #[test]
    fn zero_on_every_surface_and_exact_off_it_but_the_ellipsoid() {
        for shape in shapes() {
            let scale = shape.extent(DMat3::IDENTITY, 0.0).max_element();
            let out = 0.02 * shape.least_dimension();
            let exact = !matches!(shape, Shape::Ellipsoid { .. });
            // F2's exit on a sharp box is a double root, good only to about √ε.
            let tolerance = if matches!(shape, Shape::Slab { corner: 0.0, .. }) { 1e-7 } else { 1e-12 };
            for d in directions(400) {
                // F2's exit: a surface point and its normal from closed forms that are not these.
                let exit = shape.exit(d);
                let on = shape.distance(exit.point);
                assert!(on.abs() <= tolerance * scale, "{shape:?} along {d}: {on}");
                let off = shape.distance(exit.point + exit.normal * out);
                if exact {
                    assert!((off - out).abs() <= tolerance * scale, "{shape:?} along {d}: {off} for {out}");
                } else {
                    assert!(off > 0.0 && off <= out * (1.0 + 1e-12), "{shape:?} along {d}: {off} for {out}");
                }
            }
        }
    }

    #[test]
    fn the_sign_is_membership_and_the_slope_is_at_most_one() {
        for shape in shapes() {
            let reach = shape.extent(DMat3::IDENTITY, 0.0) * 1.3;
            let points: Vec<DVec3> = lattice(-reach, reach, 23).collect();
            for &p in &points {
                let d = shape.distance(p);
                if d.abs() > 1e-12 {
                    assert_eq!(d < 0.0, inside(&shape, p), "{shape:?} at {p}: {d}");
                }
            }
            let pairs = points.iter().zip(&points[1..]).chain(points.iter().zip(points.iter().rev().skip(7)));
            for (a, b) in pairs.filter(|(a, b)| a != b) {
                let slope = (shape.distance(*a) - shape.distance(*b)).abs() / (*a - *b).length();
                assert!(slope <= 1.0 + 1e-12, "{shape:?}: {slope} between {a} and {b}");
            }
        }
    }

    #[test]
    fn a_torus_is_exact_inside_the_tube_too() {
        let torus = Shape::Torus { major: 5.0, minor: 2.0 };
        assert_eq!(torus.distance(DVec3::Y * 5.0), -2.0);
        assert_eq!(torus.distance(DVec3::new(0.5, 0.0, 5.0)), -1.5);
        assert_eq!(torus.distance(DVec3::ZERO), 3.0);
        assert!((torus.distance(DVec3::X * 4.0) - (41f64.sqrt() - 2.0)).abs() < 1e-15);
    }

    fn mind() -> Part {
        Part::mind(PartId(0), B.min_part_m3)
    }

    fn part(id: u16, kind: Kind, primitive: Primitive, volume_m3: f64, parent: u16, mount: Mount) -> Part {
        let placement = Placement { parent: PartId(parent), mount, twist: 0.0, tilt: DVec2::ZERO, blend: 0.0, mirror: false };
        Part { id: PartId(id), kind, primitive, volume_m3, placement: Some(placement) }
    }

    fn at(anchor: DVec3, standoff: f64) -> Mount {
        Mount::Attached { anchor, standoff }
    }

    const SPHERE: Primitive = Primitive::Capsule { length: 0.0 };

    /// A sphere around the Mind, a boom off it, and a ball on the boom's end, both embedded so a
    /// saddle has something to cut. The sphere and the ball have exact distances.
    fn boom(mode: SparMode) -> Form {
        Form {
            parts: vec![
                mind(),
                part(1, Kind::Storage, SPHERE, 5e5, 0, Mount::Enclosing),
                part(2, Kind::Spar(mode), Primitive::Cylinder { length: 8.0 }, 2e4, 1, at(DVec3::new(0.3, 1.0, 0.2), -0.3)),
                part(3, Kind::Engine, SPHERE, 1e4, 2, at(DVec3::X, -0.5)),
            ],
        }
    }

    fn index(sdf: &Sdf, id: u16, side: Side) -> usize {
        sdf.pieces().iter().position(|p| p.part == PartId(id) && p.side == side).unwrap()
    }

    /// Where `f` crosses zero between `a` and `b`, which it must.
    fn root(f: impl Fn(DVec3) -> f64, mut a: DVec3, mut b: DVec3) -> DVec3 {
        assert!(f(a).signum() != f(b).signum(), "no crossing: {} and {}", f(a), f(b));
        let below = f(a) < 0.0;
        for _ in 0..100 {
            let mid = (a + b) / 2.0;
            if (f(mid) < 0.0) == below { a = mid } else { b = mid }
        }
        a
    }

    /// Exact distance to a sphere piece, from its pose and radius alone.
    fn off_sphere(piece: &Piece, p: DVec3) -> f64 {
        let Shape::Capsule { radius, length } = piece.shape else { panic!("{:?}", piece.shape) };
        assert_eq!(length, 0.0);
        (p - piece.pose.position).length() - radius
    }

    #[test]
    fn a_saddle_sits_spar_gap_off_each_neighbor() {
        let sdf = Sdf::new(&boom(SparMode::Saddle), &B).unwrap();
        let spar = index(&sdf, 2, Side::Original);
        let pose = sdf.pieces()[spar].pose;
        let reach = sdf.pieces()[spar].shape.reach();
        let hull = &sdf.pieces()[index(&sdf, 1, Side::Original)];
        let ball = &sdf.pieces()[index(&sdf, 3, Side::Original)];
        let own = |p| sdf.piece_distance(spar, p);

        // Along the boom's axis, its two ends are where it stops short of each neighbor.
        let foot = root(own, pose.position, pose.to_outer(-DVec3::X * reach));
        let head = root(own, pose.position, pose.to_outer(DVec3::X * reach));
        assert!((off_sphere(hull, foot) - B.spar_gap).abs() < 1e-9, "{}", off_sphere(hull, foot));
        assert!((off_sphere(ball, head) - B.spar_gap).abs() < 1e-9, "{}", off_sphere(ball, head));

        // And nowhere does any of it come nearer.
        let (min, max) = sdf.bounds();
        let mut solid = 0;
        for p in lattice(min, max, 60).filter(|&p| own(p) <= 0.0) {
            solid += 1;
            assert!(off_sphere(hull, p) >= B.spar_gap - 1e-9 && off_sphere(ball, p) >= B.spar_gap - 1e-9, "{p}");
        }
        assert!(solid > 100, "{solid}");
    }

    #[test]
    fn a_strap_stays_inside_its_shell() {
        // A band around a tank: a torus enclosing a sphere, its tube straddling the surface.
        let tank = part(1, Kind::Storage, SPHERE, 5e5, 0, Mount::Enclosing);
        let r = tank.shape(B.min_part_m3).reach();
        let band = Primitive::Torus { major: 7.0 };
        let volume = band.volume(r / 7.0);
        let strap = part(2, Kind::Spar(SparMode::Strap), band, volume, 1, Mount::Enclosing);
        let form = Form { parts: vec![mind(), tank, strap] };
        let sdf = Sdf::new(&form, &B).unwrap();
        let depth = B.spar_thickness * 2.0 * r;
        let own = |p| sdf.piece_distance(index(&sdf, 2, Side::Original), p);

        let (min, max) = sdf.bounds();
        let mut solid = 0;
        for p in lattice(min, max, 90).filter(|&p| own(p) <= 0.0) {
            solid += 1;
            let above = p.length() - r;
            assert!((-1e-9..=depth + 1e-9).contains(&above), "{p}: {above} above, in {depth}");
        }
        assert!(solid > 50, "{solid}");
        // Its whole depth, where the tube crosses the surface.
        let outward = DVec3::Y;
        let top = root(own, outward * (r + depth / 2.0), outward * (r + 2.0 * depth));
        assert!((top.length() - r - depth).abs() < 1e-9, "{top}");
    }

    #[test]
    fn a_strap_is_as_deep_as_its_parents_least_dimension_says() {
        let flat = Primitive::Slab { edges: DVec3::new(4.0, 4.0, 1.0), corner: 0.1 };
        let mut form = boom(SparMode::Strap);
        form.parts[1].primitive = flat;
        let sdf = Sdf::new(&form, &B).unwrap();
        let slab = sdf.pieces()[index(&sdf, 1, Side::Original)].shape;
        let Shape::Slab { edges, .. } = slab else { panic!() };
        assert_eq!(slab.least_dimension(), edges.z);
        let spar = index(&sdf, 2, Side::Original);
        let depth = B.spar_thickness * edges.z;
        assert_eq!(sdf.conform[spar], Conform::Strap { parent: index(&sdf, 1, Side::Original), depth });
    }

    #[test]
    fn a_seam_is_where_a_spar_meets_what_cuts_it() {
        let sdf = Sdf::new(&boom(SparMode::Saddle), &B).unwrap();
        let [hull, spar, ball] = [1, 2, 3].map(|id| index(&sdf, id, Side::Original));
        let piece = sdf.pieces()[spar];
        let Shape::Cylinder { radius, .. } = piece.shape else { panic!() };
        let reach = piece.shape.reach();
        // Down the boom's side toward the hull, to where the side meets the cut.
        let side = |x: f64| piece.pose.to_outer(DVec3::new(x, 0.0, radius));
        let x = root(|p| B.spar_gap - sdf.primitive(hull, p), side(0.0), side(-reach)).dot(piece.pose.axis())
            - piece.pose.position.dot(piece.pose.axis());
        let (neighbor, d) = sdf.seam(spar, side(x)).unwrap();
        assert_eq!(neighbor, hull);
        assert!(d < 1e-9, "{d}");
        let (neighbor, d) = sdf.seam(spar, side(x + 1.0)).unwrap();
        assert_eq!(neighbor, hull);
        assert!(d > 0.5 && d <= 1.0 + 1e-9, "a meter along the side: {d}");
        let proud = side(x) + piece.pose.rotation.z_axis;
        assert!(sdf.seam(spar, proud).unwrap().1 > 0.5, "a meter off the boom's side");
        let (neighbor, _) = sdf.seam(spar, side(reach * 0.9)).unwrap();
        assert_eq!(neighbor, ball);
        assert_eq!(sdf.seam(hull, side(x)), None);

        // A strap's is its edge on the parent's surface.
        let tank = part(1, Kind::Storage, SPHERE, 5e5, 0, Mount::Enclosing);
        let r = tank.shape(B.min_part_m3).reach();
        let band = Primitive::Torus { major: 7.0 };
        let strap = part(2, Kind::Spar(SparMode::Strap), band, band.volume(r / 7.0), 1, Mount::Enclosing);
        let sdf = Sdf::new(&Form { parts: vec![mind(), tank, strap] }, &B).unwrap();
        let (tank, strap) = (index(&sdf, 1, Side::Original), index(&sdf, 2, Side::Original));
        let tube = |t: f64| DVec3::new((r / 7.0) * t.sin(), r + (r / 7.0) * t.cos(), 0.0);
        // Around the tube's surface, from its outermost point to its innermost.
        let edge = tube(root(|t| sdf.primitive(tank, tube(t.x)), DVec3::ZERO, DVec3::X * PI).x);
        let (neighbor, d) = sdf.seam(strap, edge).unwrap();
        assert_eq!(neighbor, tank);
        assert!(d < 1e-9, "{d}");
        assert!(sdf.seam(strap, tube(0.0)).unwrap().1 > 0.1 * r / 7.0);
    }

    #[test]
    fn only_tree_neighbors_cut_a_spar() {
        let alone = Sdf::new(&boom(SparMode::Saddle), &B).unwrap();
        // A sibling laid right across the boom.
        let mut form = boom(SparMode::Saddle);
        let spar = form.parts[2].placement.unwrap();
        form.parts.push(part(4, Kind::Data, SPHERE, 2e4, 1, spar.mount));
        let crossed = Sdf::new(&form, &B).unwrap();
        let (a, b) = (index(&alone, 2, Side::Original), index(&crossed, 2, Side::Original));
        let sibling = index(&crossed, 4, Side::Original);
        let (min, max) = crossed.bounds();
        let mut overlap = 0;
        for p in lattice(min, max, 40) {
            assert_eq!(alone.piece_distance(a, p), crossed.piece_distance(b, p), "{p}");
            overlap += usize::from(crossed.primitive(sibling, p) < 0.0 && crossed.piece_distance(b, p) < 0.0);
        }
        assert!(overlap > 0);
    }

    fn blended(blend: f64) -> Form {
        let mut form = boom(SparMode::Saddle);
        for part in &mut form.parts[2..] {
            part.placement.as_mut().unwrap().blend = blend;
        }
        let mut pod = part(4, Kind::Living, Primitive::Ellipsoid { axes: DVec3::new(2.0, 1.0, 1.0) }, 5e4, 1, at(-DVec3::Z, -0.2));
        pod.placement.as_mut().unwrap().blend = blend;
        form.parts.push(pod);
        form
    }

    fn hard(sdf: &Sdf, p: DVec3) -> f64 {
        (0..sdf.pieces().len()).map(|i| sdf.piece_distance(i, p)).fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn spars_join_hard_whatever_their_blend() {
        let sdf = Sdf::new(&blended(0.5), &B).unwrap();
        let spar = index(&sdf, 2, Side::Original);
        let pose = sdf.pieces()[spar].pose;
        let reach = sdf.pieces()[spar].shape.reach();
        // Around both of the boom's joints, the union is exactly the hard one.
        for end in [-1.0, 1.0] {
            let joint = pose.to_outer(DVec3::X * end * reach);
            for p in lattice(joint - reach, joint + reach, 25) {
                assert_eq!(sdf.distance(p), hard(&sdf, p), "{p}");
            }
        }
        assert_eq!(sdf.blends.len(), 1, "only the pod's joint with the hull");
    }

    #[test]
    fn a_blend_fills_a_joint_by_its_radius_and_leaves_the_rest() {
        let blend = 0.3;
        let sdf = Sdf::new(&blended(blend), &B).unwrap();
        let (hull, pod) = (index(&sdf, 1, Side::Original), index(&sdf, 4, Side::Original));
        let radius = blend * sdf.pieces()[hull].shape.least_dimension().min(sdf.pieces()[pod].shape.least_dimension());

        // Where the two are equally far the fillet is deepest, a quarter of the radius.
        let (a, b) = (sdf.pieces()[hull].pose.position, sdf.pieces()[pod].pose.position);
        let even = root(|p| sdf.piece_distance(hull, p) - sdf.piece_distance(pod, p), a, b);
        let depth = hard(&sdf, even) - sdf.distance(even);
        assert!((depth - radius / 4.0).abs() < 1e-9 * radius, "{depth} for {radius}");

        let (min, max) = sdf.bounds();
        for p in lattice(min, max, 40) {
            let (d, h) = (sdf.distance(p), hard(&sdf, p));
            assert!(d <= h && d >= h - radius / 4.0 - 1e-12, "{p}");
            let (dh, dp) = (sdf.piece_distance(hull, p), sdf.piece_distance(pod, p));
            if (dh - dp).abs() >= radius {
                assert_eq!(d, h, "{p}");
            }
        }
    }

    #[test]
    fn the_whole_field_is_lipschitz_one() {
        let sdf = Sdf::new(&blended(0.4), &B).unwrap();
        let (min, max) = sdf.bounds();
        let pad = (max - min) * 0.1;
        let points: Vec<DVec3> = lattice(min - pad, max + pad, 30).collect();
        let pairs = points.iter().zip(&points[1..]).chain(points.iter().zip(points.iter().rev().skip(11)));
        for (a, b) in pairs.filter(|(a, b)| a != b) {
            let slope = (sdf.distance(*a) - sdf.distance(*b)).abs() / (*a - *b).length();
            assert!(slope <= 1.0 + 1e-12, "{slope} between {a} and {b}");
        }
    }

    #[test]
    fn a_mirrored_copy_is_in_the_field_and_named() {
        let mut form = blended(0.2);
        form.parts[2].placement.as_mut().unwrap().mirror = true;
        let sdf = Sdf::new(&form, &B).unwrap();
        assert_eq!(sdf.pieces().len(), 7);
        let flip = DVec3::new(1.0, -1.0, 1.0);
        let (min, max) = sdf.bounds();
        assert_eq!(min.y, -max.y);
        for p in lattice(min, max, 30) {
            assert!((sdf.distance(p) - sdf.distance(p * flip)).abs() < 1e-12, "{p}");
        }
        let ball = sdf.pieces()[index(&sdf, 3, Side::Mirror)].pose.position;
        assert!(ball.y < 0.0);
        let (piece, d) = sdf.nearest(ball);
        assert_eq!((piece.part, piece.side, piece.kind), (PartId(3), Side::Mirror, Kind::Engine));
        assert!(d < 0.0 && sdf.distance(ball) < 0.0);
        // The mirrored boom is cut by the hull's one copy and its own ball, not the original's.
        let spar = index(&sdf, 2, Side::Mirror);
        let hull = index(&sdf, 1, Side::Original);
        let mirror_ball = index(&sdf, 3, Side::Mirror);
        assert_eq!(sdf.conform[spar], Conform::Saddle(vec![hull, mirror_ball]));
    }

    #[test]
    fn nearest_names_the_kind_under_a_point() {
        let sdf = Sdf::new(&blended(0.2), &B).unwrap();
        for (id, kind) in [(1, Kind::Storage), (3, Kind::Engine), (4, Kind::Living)] {
            let i = index(&sdf, id, Side::Original);
            let (piece, d) = sdf.nearest(sdf.pieces()[i].pose.position);
            assert_eq!((piece.part, piece.kind), (PartId(id), kind));
            assert_eq!(d, sdf.piece_distance(i, sdf.pieces()[i].pose.position));
        }
    }

    /// Contains every point of the solid, and with nothing blended is no larger than a cell of
    /// sampling. A blend's margin is conservative.
    #[test]
    fn the_bounds_hold_the_solid() {
        for blend in [0.0, 0.5] {
            let mut form = blended(blend);
            let pod = form.parts[4].placement.as_mut().unwrap();
            pod.twist = 0.7;
            pod.tilt = DVec2::new(0.4, -0.3);
            let sdf = Sdf::new(&form, &B).unwrap();
            let (min, max) = sdf.bounds();
            let pad = (max - min) * 0.2;
            let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
            for p in lattice(min - pad, max + pad, 70).filter(|&p| sdf.distance(p) <= 0.0) {
                assert!(p.cmpge(min).all() && p.cmple(max).all(), "{p} outside {min}..{max}");
                (lo, hi) = (lo.min(p), hi.max(p));
            }
            if blend == 0.0 {
                let cell = (max - min + 2.0 * pad) / 69.0;
                assert!((lo - min).cmple(cell).all(), "{lo} against {min}, cells of {cell}");
                assert!((max - hi).cmple(cell).all(), "{hi} against {max}");
            }
        }
    }

    /// Two equal boxes stacked with their sides flush: along the seam both are equally far, so
    /// the fillet stands a quarter of its radius proud of either.
    #[test]
    fn the_bounds_hold_a_fillet_proud_of_both_parts() {
        let slab = |edges| Primitive::Slab { edges, corner: 0.0 };
        let lower = part(1, Kind::Storage, slab(DVec3::new(4.0, 3.0, 1.0)), 5e5, 0, Mount::Enclosing);
        let mut upper = part(2, Kind::Data, slab(DVec3::new(1.0, 3.0, 4.0)), 5e5, 1, at(DVec3::Z, 0.0));
        upper.placement.as_mut().unwrap().blend = 0.5;
        let sdf = Sdf::new(&Form { parts: vec![mind(), lower, upper] }, &B).unwrap();
        let Shape::Slab { edges, .. } = sdf.pieces()[index(&sdf, 1, Side::Original)].shape else { panic!() };
        let radius = sdf.blends[0].2;
        let seam = DVec3::new(edges.x / 2.0 + radius / 8.0, 0.0, edges.z / 2.0);
        assert!(sdf.distance(seam) < 0.0, "the fillet reaches past the flush sides");
        assert!(seam.x <= sdf.bounds().1.x);
    }

    #[test]
    fn least_dimension_is_the_smallest_extent() {
        for shape in shapes() {
            let extents = shape.extent(DMat3::IDENTITY, 0.0) * 2.0;
            assert!((shape.least_dimension() - extents.min_element()).abs() < 1e-12, "{shape:?}");
        }
    }
}
