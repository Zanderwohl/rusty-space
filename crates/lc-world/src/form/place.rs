//! Where every part sits in the ship's frame, walked down the tree from the Mind.
//!
//! The ship's frame is the Mind's: x is the nose (`motion::facing`) and z is up, so y is port and
//! the port–starboard plane is y = 0. A mirrored part keeps its id; `(PartId, Side)` names a copy.
//!
//! Everything is closed form — IEEE arithmetic, `sqrt`, and `libm` for the trigonometry — so the
//! server and every client place a form to the bit. See 29 §Placement is relative.

use std::collections::BTreeMap;

use glam::{DMat3, DVec2, DVec3};

use super::primitive::Shape;
use super::{Form, FormError, Mount, Part, PartId, Placement};

/// Which copy of a part. Every part has an `Original`; a part whose own `mirror` or any ancestor's
/// is set also has a `Mirror`, reflected through y = 0. A mirror inside a mirrored subtree adds
/// nothing, since reflecting twice is the original.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    Original,
    Mirror,
}

/// A rigid transform from a part's frame to its parent's or the ship's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub position: DVec3,
    /// Columns are the part's x, y and z. Proper even on a mirrored copy: every primitive is
    /// symmetric in its own y, so the reflection is carried as that flip composed with a rotation.
    pub rotation: DMat3,
}

impl Pose {
    pub const IDENTITY: Pose = Pose { position: DVec3::ZERO, rotation: DMat3::IDENTITY };

    pub fn to_outer(&self, local: DVec3) -> DVec3 {
        self.position + self.rotation * local
    }

    pub fn to_local(&self, outer: DVec3) -> DVec3 {
        self.rotation.transpose() * (outer - self.position)
    }

    pub fn axis(&self) -> DVec3 {
        self.rotation.x_axis
    }

    /// `inner` given in this pose's frame, carried out to the frame this pose is in.
    pub fn then(&self, inner: &Pose) -> Pose {
        Pose { position: self.to_outer(inner.position), rotation: self.rotation * inner.rotation }
    }

    /// Only sign flips, so the copy is exact.
    fn mirrored(&self) -> Pose {
        let flip = DVec3::new(1.0, -1.0, 1.0);
        let r = self.rotation;
        Pose {
            position: self.position * flip,
            rotation: DMat3::from_cols(r.x_axis * flip, -(r.y_axis * flip), r.z_axis * flip),
        }
    }
}

/// Every copy of every part, in the ship's frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Poses(BTreeMap<(PartId, Side), Pose>);

impl Poses {
    pub fn get(&self, part: PartId, side: Side) -> Option<&Pose> {
        self.0.get(&(part, side))
    }

    /// By id, the original before its mirror.
    pub fn iter(&self) -> impl Iterator<Item = (PartId, Side, &Pose)> {
        self.0.iter().map(|(&(id, side), pose)| (id, side, pose))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Form {
    pub fn place(&self, min_part_m3: f64) -> Result<Poses, FormError> {
        self.validate()?;
        let mut children: BTreeMap<PartId, Vec<&Part>> = BTreeMap::new();
        let mut root = None;
        for part in &self.parts {
            match part.placement {
                Some(p) => children.entry(p.parent).or_default().push(part),
                None => root = Some(part),
            }
        }
        let mut poses = BTreeMap::new();
        let mut stack = vec![(root.expect("validate found the Mind"), Pose::IDENTITY, false)];
        while let Some((part, pose, mirrored)) = stack.pop() {
            poses.insert((part.id, Side::Original), pose);
            if mirrored {
                poses.insert((part.id, Side::Mirror), pose.mirrored());
            }
            let shape = part.shape(min_part_m3);
            for child in children.get(&part.id).into_iter().flatten() {
                let placement = child.placement.expect("only the Mind is unplaced");
                let relative = relative(&shape, &child.shape(min_part_m3), &placement);
                stack.push((child, pose.then(&relative), mirrored || placement.mirror));
            }
        }
        Ok(Poses(poses))
    }
}

/// A child's pose in its parent's frame.
///
/// Attached, the child's foot — the end of its axis at −x, [`Shape::reach`] from its center — sits
/// on the anchor, displaced along the normal by `standoff` reaches, and tilt pivots it about that
/// foot. So a child grows away from its parent, and follows the anchor out when the parent grows.
pub fn relative(parent: &Shape, child: &Shape, placement: &Placement) -> Pose {
    let turn = twist_tilt(placement.twist, placement.tilt);
    match placement.mount {
        Mount::Enclosing => Pose { position: DVec3::ZERO, rotation: turn },
        Mount::Attached { anchor, standoff } => {
            // Scaled first, so an anchor whose squared length overflows still has a direction.
            let exit = parent.exit((anchor / anchor.abs().max_element()).normalize());
            let rotation = normal_frame(exit.normal) * turn;
            let reach = child.reach();
            let foot = exit.point + exit.normal * (standoff * reach);
            Pose { position: foot + rotation.x_axis * reach, rotation }
        }
    }
}

/// Below this the parent's y lies along the normal to rounding, and what is left of it after
/// projection points nowhere in particular.
const PARALLEL: f64 = 1e-9;

/// Axis along `normal`, y along the parent's y projected across it, or its z where the y is
/// parallel.
pub(super) fn normal_frame(normal: DVec3) -> DMat3 {
    let across = |v: DVec3| v - normal * normal.dot(v);
    let y = across(DVec3::Y);
    let y = if y.length() > PARALLEL { y } else { across(DVec3::Z) }.normalize();
    DMat3::from_cols(normal, y, normal.cross(y))
}

fn twist_tilt(twist: f64, tilt: DVec2) -> DMat3 {
    let (s, c) = libm::sincos(twist);
    let twist = DMat3::from_cols(DVec3::X, DVec3::new(0.0, c, s), DVec3::new(0.0, -s, c));
    twist * rotation_vector(DVec3::new(0.0, tilt.x, tilt.y))
}

/// θ² below which `sin θ / θ` and `(1 − cos θ) / θ²` are taken from their series. The next terms
/// are below 1e-17 there.
const SMALL_ANGLE_SQ: f64 = 1e-8;

/// Rodrigues, in the form that divides by θ nowhere it can be zero.
fn rotation_vector(v: DVec3) -> DMat3 {
    let theta_sq = v.length_squared();
    let (sinc, cosc) = if theta_sq < SMALL_ANGLE_SQ {
        (1.0 - theta_sq / 6.0, 0.5 - theta_sq / 24.0)
    } else {
        let theta = theta_sq.sqrt();
        // 1 − cos θ as 2 sin²(θ/2), which does not cancel at small θ.
        let half = libm::sin(theta / 2.0) / theta;
        (libm::sin(theta) / theta, 2.0 * half * half)
    };
    let k = DMat3::from_cols(DVec3::new(0.0, v.z, -v.y), DVec3::new(-v.z, 0.0, v.x), DVec3::new(v.y, -v.x, 0.0));
    DMat3::IDENTITY + k * sinc + k * k * cosc
}

/// A point on a part's surface and the outward normal there, in the part's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exit {
    pub point: DVec3,
    pub normal: DVec3,
}

impl Shape {
    /// Distance from the center to the end of the axis, and so to the foot an attached part
    /// rests on.
    pub fn reach(&self) -> f64 {
        match *self {
            Shape::Ellipsoid { semi_axes } => semi_axes.x,
            Shape::Capsule { radius, length } => length / 2.0 + radius,
            Shape::Slab { edges, .. } => edges.x / 2.0,
            Shape::Cylinder { length, .. } | Shape::Frustum { length, .. } => length / 2.0,
            Shape::Torus { minor, .. } => minor,
        }
    }

    /// Where a ray from the center along the unit `direction` last leaves the surface.
    ///
    /// Every primitive but the torus is convex about its center, so it has one exit. A ray from a
    /// torus's center lies in a plane through its axis, which cuts the tube in two circles, so its
    /// exit is a quadratic too. A ray that misses the tube takes the point of the tube nearest it,
    /// which meets the exits continuously where the ray grazes.
    pub fn exit(&self, direction: DVec3) -> Exit {
        let d = direction;
        let radial = DVec3::new(0.0, d.y, d.z);
        let w = radial.length();
        // On the axis every radial direction is as good as another; +y picks the one point of a
        // torus's inner equator, all of which is nearest to that ray.
        let out = if w > 0.0 { radial / w } else { DVec3::Y };
        let cap = |h: f64| (h / d.x.abs(), DVec3::X * d.x.signum());
        let (s, normal) = match *self {
            Shape::Ellipsoid { semi_axes: a } => {
                let s = 1.0 / (d / a).length();
                (s, d * s / (a * a))
            }
            Shape::Capsule { radius: r, length } => {
                let h = length / 2.0;
                if r * d.x.abs() <= h * w {
                    (r / w, out)
                } else {
                    let end = DVec3::X * (h * d.x.signum());
                    // r² − h²w², factored so it does not cancel near the rim.
                    let s = h * d.x.abs() + ((r - h * w) * (r + h * w)).sqrt();
                    (s, d * s - end)
                }
            }
            Shape::Cylinder { radius: r, length } => {
                if r * d.x.abs() <= length / 2.0 * w { (r / w, out) } else { cap(length / 2.0) }
            }
            Shape::Frustum { length, start, end } => {
                let (h, mid, slope) = (length / 2.0, (start + end) / 2.0, (end - start) / length);
                let closing = w - slope * d.x;
                if closing > 0.0 && mid * d.x.abs() < h * closing {
                    (mid / closing, out - DVec3::X * slope)
                } else {
                    cap(h)
                }
            }
            Shape::Torus { major, minor } => {
                let reach = major * d.x.abs();
                let disc = (minor - reach) * (minor + reach);
                if disc >= 0.0 {
                    let s = major * w + disc.sqrt();
                    return Exit { point: d * s, normal: (d * s - out * major).normalize() };
                }
                let toward = DVec3::X * (w * d.x.signum()) - out * d.x.abs();
                return Exit { point: out * major + toward * minor, normal: toward };
            }
            Shape::Slab { edges, corner } => slab_exit(edges / 2.0 - corner, corner, d),
        };
        Exit { point: d * s, normal: normal.normalize() }
    }
}

/// A rounded box is the points within `corner` of an inner box of half-extents `inner`. Along the
/// ray the distance to that box is a quadratic between the points where each coordinate passes its
/// half-extent, so walk those at most four pieces and solve the one it crosses `corner` in.
fn slab_exit(inner: DVec3, corner: f64, d: DVec3) -> (f64, DVec3) {
    let a = d.abs().to_array();
    let h = inner.to_array();
    let enters = [0, 1, 2].map(|i| if a[i] > 0.0 { h[i] / a[i] } else { f64::INFINITY });
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| enters[i].total_cmp(&enters[j]).then(i.cmp(&j)));
    let past = |s: f64, set: &[usize]| set.iter().map(|&i| (s * a[i] - h[i]).powi(2)).sum::<f64>();

    let mut beyond: Vec<usize> = Vec::with_capacity(3);
    for k in 0..=3 {
        let until = order.get(k).map_or(f64::INFINITY, |&i| enters[i]);
        if !beyond.is_empty() && (until.is_infinite() || past(until, &beyond) > corner * corner) {
            break;
        }
        beyond.push(order[k]);
    }
    let (mut qa, mut qb, mut qc) = (0.0, 0.0, -corner * corner);
    for &i in &beyond {
        qa += a[i] * a[i];
        qb += a[i] * h[i];
        qc += h[i] * h[i];
    }
    // The larger root, where the distance rises through `corner`. Clamped at zero because a sharp
    // box's root is double and rounding can put the discriminant just below it.
    let s = (qb + (qb * qb - qa * qc).max(0.0).sqrt()) / qa;

    let mut normal = DVec3::ZERO;
    for &i in &beyond {
        normal[i] = (s * a[i] - h[i]).max(0.0) * d[i].signum();
    }
    if normal == DVec3::ZERO {
        // A sharp box, whose root sits where its first face is reached.
        normal[order[0]] = d[order[0]].signum();
    }
    (s, normal)
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use super::*;
    use crate::form::{Kind, Primitive};

    const MIN_PART_M3: f64 = 1_000.0;

    fn primitives() -> [Primitive; 6] {
        [
            Primitive::Ellipsoid { axes: DVec3::new(3.0, 1.0, 2.0) },
            Primitive::Capsule { length: 2.0 },
            Primitive::Slab { edges: DVec3::new(2.0, 1.0, 3.0), corner: 0.2 },
            Primitive::Cylinder { length: 3.0 },
            Primitive::Torus { major: 3.0 },
            Primitive::Frustum { length: 2.0, taper: 0.5 },
        ]
    }

    /// Independent of the code under test: membership, from each primitive's definition.
    fn inside(shape: &Shape, p: DVec3) -> bool {
        let rho = p.y.hypot(p.z);
        match *shape {
            Shape::Ellipsoid { semi_axes } => (p / semi_axes).length_squared() <= 1.0,
            Shape::Capsule { radius, length } => {
                let x = p.x.clamp(-length / 2.0, length / 2.0);
                (p - DVec3::X * x).length() <= radius
            }
            Shape::Slab { edges, corner } => (p.abs() - (edges / 2.0 - corner)).max(DVec3::ZERO).length() <= corner,
            Shape::Cylinder { radius, length } => p.x.abs() <= length / 2.0 && rho <= radius,
            Shape::Torus { major, minor } => (rho - major).hypot(p.x) <= minor,
            Shape::Frustum { length, start, end } => {
                p.x.abs() <= length / 2.0 && rho <= start + (end - start) * (p.x / length + 0.5)
            }
        }
    }

    fn size(shape: &Shape) -> f64 {
        match *shape {
            Shape::Ellipsoid { semi_axes } => semi_axes.max_element(),
            Shape::Capsule { radius, length } => radius + length / 2.0,
            Shape::Slab { edges, .. } => edges.length() / 2.0,
            Shape::Cylinder { radius, length } => radius.hypot(length / 2.0),
            Shape::Torus { major, minor } => major + minor,
            Shape::Frustum { length, start, end } => start.max(end).hypot(length / 2.0),
        }
    }

    /// The last exit, found by marching in from well outside and bisecting, so nothing is shared
    /// with a closed form. `None` where the ray misses.
    fn last_exit(shape: &Shape, d: DVec3) -> Option<f64> {
        let far = 2.0 * size(shape);
        let steps = 20_000;
        let (mut lo, mut hi) = (None, far);
        for k in (0..steps).rev() {
            let s = far * k as f64 / steps as f64;
            if inside(shape, d * s) {
                lo = Some(s);
                break;
            }
            hi = s;
        }
        let mut lo = lo?;
        for _ in 0..80 {
            let mid = 0.5 * (lo + hi);
            if inside(shape, d * mid) { lo = mid } else { hi = mid }
        }
        Some(lo)
    }

    fn anchors() -> Vec<DVec3> {
        [
            DVec3::X,
            -DVec3::X,
            DVec3::Y,
            DVec3::Z,
            -DVec3::Z,
            DVec3::new(1.0, 0.9, 0.8),
            DVec3::new(-1.0, 0.3, -0.7),
            DVec3::new(0.2, -1.0, 0.05),
            DVec3::new(0.05, 0.6, 1.0),
        ]
        .map(DVec3::normalize)
        .to_vec()
    }

    fn part(id: u16, primitive: Primitive, volume_m3: f64, parent: u16, mount: Mount) -> Part {
        Part {
            id: PartId(id),
            kind: Kind::Storage,
            primitive,
            volume_m3,
            placement: Some(Placement {
                parent: PartId(parent),
                mount,
                twist: 0.0,
                tilt: DVec2::ZERO,
                blend: 0.0,
                mirror: false,
            }),
        }
    }

    fn attached(anchor: DVec3, standoff: f64) -> Mount {
        Mount::Attached { anchor, standoff }
    }

    fn turned(mut part: Part, twist: f64, tilt: DVec2) -> Part {
        let placement = part.placement.as_mut().unwrap();
        placement.twist = twist;
        placement.tilt = tilt;
        part
    }

    /// A Mind, a parent enclosing it at an angle, and a child on the parent.
    fn pair(parent: Primitive, child: Part) -> Form {
        let parent = turned(part(1, parent, 5e5, 0, Mount::Enclosing), 0.4, DVec2::new(0.1, -0.2));
        Form { parts: vec![Part::mind(PartId(0), MIN_PART_M3), parent, child] }
    }

    fn close(a: DVec3, b: DVec3, scale: f64) -> bool {
        (a - b).length() <= 1e-9 * scale
    }

    #[test]
    fn the_exit_is_where_the_ray_last_leaves_and_the_normal_is_square_to_the_surface() {
        let mut shapes: Vec<Shape> = primitives().iter().map(|p| p.at(1.7)).collect();
        shapes.push(Primitive::MIND.at(2.0));
        shapes.push(Primitive::Slab { edges: DVec3::new(1.0, 2.0, 1.0), corner: 0.5 }.at(1.0));
        shapes.push(Primitive::Frustum { length: 3.0, taper: 0.0 }.at(1.0));
        shapes.push(Primitive::Torus { major: 1.5 }.at(1.0));
        for shape in &shapes {
            for d in anchors() {
                let exit = shape.exit(d);
                let Some(s) = last_exit(shape, d) else { continue };
                assert!(close(exit.point, d * s, size(shape)), "{shape:?} along {d}: {} against {}", exit.point, d * s);
                assert!((exit.normal.length() - 1.0).abs() < 1e-12);
                assert!(exit.normal.dot(d) > 0.0, "{shape:?} along {d}: normal {}", exit.normal);
                if matches!(shape, Shape::Frustum { end: 0.0, .. }) && d == DVec3::X {
                    continue; // the apex, which has no tangent plane
                }
                // Neighboring exits lie in the tangent plane, to first order.
                let (u, v) = d.any_orthonormal_pair();
                for side in [u, v, -u, -v] {
                    let e = d + side * 1e-6;
                    let near = e.normalize() * last_exit(shape, e.normalize()).unwrap();
                    let chord = near - exit.point;
                    let off_plane = chord.normalize().dot(exit.normal).abs();
                    assert!(off_plane < 1e-3, "{shape:?} along {d}: normal {}", exit.normal);
                }
            }
        }
    }

    #[test]
    fn a_ray_that_misses_the_torus_takes_its_nearest_point_and_meets_the_hits_at_the_graze() {
        let torus = Primitive::Torus { major: 3.0 }.at(1.0);
        let Shape::Torus { major, minor } = torus else { unreachable!() };
        let on_tube = |p: DVec3| ((p.y.hypot(p.z) - major).hypot(p.x) - minor).abs() < 1e-12;
        let up = torus.exit(DVec3::X);
        assert!(on_tube(up.point));
        assert!(close(up.point, DVec3::Y * (major - minor), 1.0), "{}", up.point);
        assert!(close(up.normal, -DVec3::Y, 1.0));

        let graze = (minor / major).asin();
        let at = |a: f64| torus.exit(DVec3::new(a.sin(), 0.0, a.cos()));
        let (hit, miss) = (at(graze - 1e-9), at(graze + 1e-9));
        assert!(on_tube(miss.point));
        assert!((hit.point - miss.point).length() < 1e-3, "{} against {}", hit.point, miss.point);
        assert!((hit.normal - miss.normal).length() < 1e-3);
    }

    #[test]
    fn a_torus_child_hangs_off_the_outer_rim() {
        let ring = Primitive::Torus { major: 4.0 };
        let Shape::Torus { major, minor } = ring.at(ring.scale(5e5)) else { unreachable!() };
        let child = part(2, Primitive::Capsule { length: 1.0 }, 1e3, 1, attached(DVec3::Y, 0.0));
        let form = Form {
            parts: vec![Part::mind(PartId(0), MIN_PART_M3), part(1, ring, 5e5, 0, Mount::Enclosing), child],
        };
        let poses = form.place(MIN_PART_M3).unwrap();
        let child = poses.get(PartId(2), Side::Original).unwrap();
        let foot = child.to_outer(DVec3::NEG_X * child_reach(&form, 2));
        assert!(close(foot, DVec3::Y * (major + minor), major), "{foot}");
        assert!(close(child.axis(), DVec3::Y, 1.0));

        // Off the midplane too, it is the outer half of the tube.
        for lift in [0.1, -0.2] {
            let d = DVec3::new(lift, 1.0, 0.0).normalize();
            let exit = ring.at(ring.scale(5e5)).exit(d);
            assert!(exit.point.y > major, "{}", exit.point);
            assert!(exit.normal.y > 0.0);
        }
    }

    fn child_reach(form: &Form, id: u16) -> f64 {
        form.parts.iter().find(|p| p.id == PartId(id)).unwrap().shape(MIN_PART_M3).reach()
    }

    #[test]
    fn an_attached_child_touches_its_parent_at_the_anchor_for_every_pair() {
        for parent in primitives() {
            let parent_shape = parent.at(parent.scale(5e5));
            for child in primitives() {
                let child_shape = child.at(child.scale(2e4));
                for anchor in anchors() {
                    // A ray that misses the torus is its own test.
                    let Some(s) = last_exit(&parent_shape, anchor) else { continue };
                    for (twist, tilt) in [(0.0, DVec2::ZERO), (1.1, DVec2::ZERO), (-0.3, DVec2::new(0.4, -0.25))] {
                        let hung = turned(part(2, child, 2e4, 1, attached(anchor * 7.0, 0.0)), twist, tilt);
                        let form = pair(parent, hung);
                        let poses = form.place(MIN_PART_M3).unwrap();
                        let outer = *poses.get(PartId(1), Side::Original).unwrap();
                        let pose = *poses.get(PartId(2), Side::Original).unwrap();
                        let scale = size(&parent_shape);
                        let what = format!("{child:?} on {parent:?} at {anchor} turned {twist} {tilt}");

                        let anchor_point = outer.to_outer(anchor * s);
                        let reach = match child_shape {
                            Shape::Torus { minor, .. } => minor,
                            _ => last_exit(&child_shape, DVec3::NEG_X).unwrap(),
                        };
                        let foot = pose.to_outer(DVec3::NEG_X * reach);
                        assert!(close(foot, anchor_point, scale), "{what}: foot {foot} against {anchor_point}");

                        // The foot is on the child's surface, and the child is on the far side of it.
                        let local = pose.to_local(anchor_point);
                        let eps = 1e-7 * scale;
                        if !matches!(child_shape, Shape::Torus { .. }) {
                            assert!(inside(&child_shape, local + DVec3::X * eps), "{what}");
                            assert!(!inside(&child_shape, local - DVec3::X * eps), "{what}");
                        }
                        if tilt == DVec2::ZERO {
                            let normal = outer.rotation * parent_shape.exit(anchor).normal;
                            if let Shape::Torus { major, minor } = child_shape {
                                // Its hole over the anchor, its tube's lowest circle in the plane
                                // tangent there.
                                for k in 0..12 {
                                    let (s, c) = (k as f64 * PI / 6.0).sin_cos();
                                    let rim = pose.to_outer(DVec3::new(-minor, major * c, major * s));
                                    let height = (rim - anchor_point).dot(normal);
                                    assert!(height.abs() <= 1e-9 * scale, "{what}: rim {height} off the plane");
                                }
                            }
                            assert!(close(pose.axis(), normal, 1.0), "{what}");
                            let from_parent = outer.to_local(pose.position);
                            assert!(!inside(&parent_shape, from_parent), "{what}: the child's center is buried");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_child_moves_out_with_its_parent_and_grows_away_from_it() {
        let anchor = DVec3::new(0.3, 1.0, -0.4).normalize();
        for parent in primitives() {
            for child in primitives() {
                let foot_and_center = |parent_m3: f64, child_m3: f64| {
                    let hung = part(2, child, child_m3, 1, attached(anchor, 0.0));
                    let parent = part(1, parent, parent_m3, 0, Mount::Enclosing);
                    let form = Form { parts: vec![Part::mind(PartId(0), MIN_PART_M3), parent, hung] };
                    let pose = *form.place(MIN_PART_M3).unwrap().get(PartId(2), Side::Original).unwrap();
                    (pose.to_outer(DVec3::NEG_X * child_reach(&form, 2)), pose.position)
                };
                let (foot, center) = foot_and_center(5e5, 2e4);
                // Eight times the volume is twice every length, and the anchor ray is from the center.
                let (grown_foot, grown_center) = foot_and_center(4e6, 2e4);
                assert!(close(grown_foot, foot * 2.0, foot.length()), "{child:?} on {parent:?}");
                assert!(close(grown_center - grown_foot, center - foot, foot.length()), "{child:?} on {parent:?}");

                let (big_foot, big_center) = foot_and_center(5e5, 1.6e5);
                assert!(close(big_foot, foot, foot.length()), "{child:?} on {parent:?}");
                assert!(close(big_center - big_foot, (center - foot) * 2.0, foot.length()), "{child:?} on {parent:?}");
            }
        }
    }

    #[test]
    fn standoff_is_along_the_normal_in_the_childs_reach() {
        let sphere = Primitive::Ellipsoid { axes: DVec3::ONE };
        let r = sphere.scale(5e5);
        for (standoff, tilt) in [(1.0, DVec2::ZERO), (-1.0, DVec2::ZERO), (0.5, DVec2::new(0.3, 0.2))] {
            let hung = part(2, Primitive::Cylinder { length: 4.0 }, 2e4, 1, attached(DVec3::Z, standoff));
            let hung = turned(hung, 0.0, tilt);
            let form = pair(sphere, hung);
            let poses = form.place(MIN_PART_M3).unwrap();
            let outer = poses.get(PartId(1), Side::Original).unwrap();
            let pose = poses.get(PartId(2), Side::Original).unwrap();
            let reach = child_reach(&form, 2);
            let foot = outer.to_local(pose.to_outer(DVec3::NEG_X * reach));
            assert!(close(foot, DVec3::Z * (r + standoff * reach), r), "{standoff}: {foot}");
        }
        // Minus one centers the child on the surface.
        let form = pair(sphere, part(2, Primitive::Cylinder { length: 4.0 }, 2e4, 1, attached(DVec3::Z, -1.0)));
        let poses = form.place(MIN_PART_M3).unwrap();
        let parent = poses.get(PartId(1), Side::Original).unwrap();
        let center = parent.to_local(poses.get(PartId(2), Side::Original).unwrap().position);
        assert!(close(center, DVec3::Z * r, r));
    }

    #[test]
    fn the_frame_follows_the_parents_y_and_falls_back_to_its_z() {
        let sphere = Primitive::Ellipsoid { axes: DVec3::ONE };
        let frame = |anchor: DVec3, twist: f64, tilt: DVec2| {
            let hung = turned(part(2, Primitive::Capsule { length: 1.0 }, 2e4, 1, attached(anchor, 0.0)), twist, tilt);
            let form = Form {
                parts: vec![Part::mind(PartId(0), MIN_PART_M3), part(1, sphere, 5e5, 0, Mount::Enclosing), hung],
            };
            form.place(MIN_PART_M3).unwrap().get(PartId(2), Side::Original).unwrap().rotation
        };
        let is = |m: DMat3, x: DVec3, y: DVec3, z: DVec3| {
            close(m.x_axis, x, 1.0) && close(m.y_axis, y, 1.0) && close(m.z_axis, z, 1.0)
        };
        let m = frame(DVec3::Z, 0.0, DVec2::ZERO);
        assert!(is(m, DVec3::Z, DVec3::Y, -DVec3::X), "{m}");
        let m = frame(DVec3::Y, 0.0, DVec2::ZERO);
        assert!(is(m, DVec3::Y, DVec3::Z, DVec3::X), "parallel to y falls back to z: {m}");
        let m = frame(-DVec3::Y, 0.0, DVec2::ZERO);
        assert!(is(m, -DVec3::Y, DVec3::Z, -DVec3::X), "{m}");
        // Nearly parallel is still the projected y, however little of it is left.
        for lean in [0.3, 1e-3, 1e-5] {
            let n = DVec3::new(0.0, 1.0, lean).normalize();
            let y = (DVec3::Y - n * n.y).normalize();
            let m = frame(n, 0.0, DVec2::ZERO);
            assert!(is(m, n, y, n.cross(y)), "{lean}: {m}");
        }
        // Twist turns y toward z; tilt about the twisted y then swings the axis toward −z.
        let m = frame(DVec3::Z, FRAC_PI_2, DVec2::ZERO);
        assert!(is(m, DVec3::Z, -DVec3::X, -DVec3::Y), "{m}");
        let m = frame(DVec3::Z, 0.0, DVec2::new(FRAC_PI_2, 0.0));
        assert!(is(m, DVec3::X, DVec3::Y, DVec3::Z), "{m}");
        let m = frame(DVec3::Z, 0.0, DVec2::new(0.0, FRAC_PI_2));
        assert!(is(m, DVec3::Y, -DVec3::Z, -DVec3::X), "{m}");
    }

    #[test]
    fn a_tilt_near_zero_is_continuous_and_orthonormal() {
        for scale in [0.0, 1e-300, 1e-12, 1e-5, 0.99e-4, 1.01e-4, 1e-3] {
            let v = DVec3::new(0.0, 0.3, -0.2) * scale;
            let m = rotation_vector(v);
            assert!(m.is_finite(), "{scale}");
            assert!((m.transpose() * m - DMat3::IDENTITY).abs().to_cols_array().iter().all(|e| *e < 1e-14), "{scale}");
            assert!((m.determinant() - 1.0).abs() < 1e-14);
            // Against the axis-angle form, which the series must match on both sides of its branch.
            let theta = v.length();
            let axis = DVec3::new(0.0, 0.3, -0.2).normalize();
            let exact = DVec3::X * theta.cos() + axis.cross(DVec3::X) * theta.sin();
            assert!((m * DVec3::X - exact).length() < 1e-16, "{scale}: {}", m * DVec3::X - exact);
        }
        let half_turn = rotation_vector(DVec3::Y * PI);
        assert!(close(half_turn * DVec3::X, -DVec3::X, 1.0));
    }

    #[test]
    fn enclosing_is_centered_on_the_parent_and_along_its_axis() {
        let core = part(1, Primitive::Cylinder { length: 3.0 }, 5e5, 0, attached(DVec3::new(0.2, 1.0, 0.3), 0.0));
        let core = turned(core, 0.3, DVec2::new(0.2, 0.1));
        let shell = part(2, Primitive::Ellipsoid { axes: DVec3::new(4.0, 1.0, 1.0) }, 5e6, 1, Mount::Enclosing);
        let shell = turned(shell, 0.9, DVec2::ZERO);
        let form = Form { parts: vec![Part::mind(PartId(0), MIN_PART_M3), core, shell] };
        let poses = form.place(MIN_PART_M3).unwrap();
        let core = poses.get(PartId(1), Side::Original).unwrap();
        let shell = poses.get(PartId(2), Side::Original).unwrap();
        assert_eq!(shell.position, core.position);
        assert!(close(shell.axis(), core.axis(), 1.0));
        let (s, c) = 0.9f64.sin_cos();
        assert!(close(shell.rotation.y_axis, core.rotation.y_axis * c + core.rotation.z_axis * s, 1.0));
    }

    fn mirrored_form() -> Form {
        let capsule = Primitive::Capsule { length: 6.0 };
        let frustum = Primitive::Frustum { length: 2.0, taper: 0.4 };
        let slab = Primitive::Slab { edges: DVec3::new(1.0, 2.0, 0.5), corner: 0.1 };
        let hull = Primitive::Ellipsoid { axes: DVec3::new(4.0, 1.5, 1.0) };
        let hull = turned(part(1, hull, 5e5, 0, Mount::Enclosing), 0.0, DVec2::new(0.05, 0.1));
        let boom = part(2, capsule, 3e4, 1, attached(DVec3::new(0.3, 1.0, 0.4), 0.2));
        let mut boom = turned(boom, 0.7, DVec2::new(0.3, -0.1));
        boom.placement.as_mut().unwrap().mirror = true;
        let pod = part(3, frustum, 1e4, 2, attached(DVec3::new(1.0, 0.2, -0.5), 0.0));
        let pod = turned(pod, -0.4, DVec2::new(0.1, 0.2));
        let ring = Primitive::Torus { major: 2.5 };
        let ring = turned(part(4, ring, 5e3, 3, Mount::Enclosing), 0.2, DVec2::new(0.0, 0.3));
        let mut inner = part(5, slab, 2e3, 3, attached(DVec3::Z, 0.0));
        inner.placement.as_mut().unwrap().mirror = true;
        let keel = part(6, Primitive::Cylinder { length: 5.0 }, 4e4, 1, attached(-DVec3::Z, 0.0));
        Form { parts: vec![Part::mind(PartId(0), MIN_PART_M3), hull, boom, pod, ring, inner, keel] }
    }

    #[test]
    fn a_mirrored_subtree_is_the_exact_reflection_of_the_original() {
        let form = mirrored_form();
        let poses = form.place(MIN_PART_M3).unwrap();
        let mirrored: Vec<u16> =
            poses.iter().filter(|(_, side, _)| *side == Side::Mirror).map(|(id, ..)| id.0).collect();
        // A mirror inside a mirrored subtree adds nothing.
        assert_eq!(mirrored, [2, 3, 4, 5]);
        assert_eq!(poses.len(), 7 + 4);

        let flip = DVec3::new(1.0, -1.0, 1.0);
        for id in [2, 3, 4, 5] {
            let shape = form.parts.iter().find(|p| p.id == PartId(id)).unwrap().shape(MIN_PART_M3);
            let original = poses.get(PartId(id), Side::Original).unwrap();
            let mirror = poses.get(PartId(id), Side::Mirror).unwrap();
            assert_eq!(mirror.position, original.position * flip);
            assert!((mirror.rotation.determinant() - 1.0).abs() < 1e-12);
            // The copy's own y is flipped, which every primitive is symmetric in.
            let local = DVec3::new(0.3, -0.7, 0.45) * size(&shape);
            assert_eq!(mirror.to_outer(local * flip), original.to_outer(local) * flip);
            let s = size(&shape);
            for k in 0..500 {
                let t = k as f64;
                let p = original.position + DVec3::new((t * 0.37).sin(), (t * 0.71).cos(), (t * 1.13).sin()) * s * 1.2;
                let reflected = inside(&shape, mirror.to_local(p * flip));
                assert_eq!(inside(&shape, original.to_local(p)), reflected, "part {id} at {p}");
            }
        }
    }

    #[test]
    fn placing_twice_agrees_to_the_bit() {
        let form = mirrored_form();
        assert_eq!(form.place(MIN_PART_M3).unwrap(), form.place(MIN_PART_M3).unwrap());
        let mut broken = form.clone();
        broken.parts[2].placement.as_mut().unwrap().parent = PartId(40);
        assert_eq!(broken.place(MIN_PART_M3), Err(FormError::MissingParent { part: PartId(2), parent: PartId(40) }));
    }
}
