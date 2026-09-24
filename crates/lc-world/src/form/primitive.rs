//! Sizing a primitive: its volume, the scale solved from a volume, and its surface area.
//!
//! A [`Primitive`] is shape without size, and a [`Shape`] is one at a scale, in meters. Volume
//! goes as scale³ and area as scale², so each primitive needs only its value at scale one.
//!
//! Every shape is centered on its part's origin, and a part's axis is its local x: a capsule's,
//! cylinder's or frustum's length and a torus's axis of symmetry run along it, a frustum's first
//! end at −x. A frustum is centered at half its length, not at its centroid.

use std::f64::consts::PI;

use glam::DVec3;

use super::{Kind, Part, PartId, Primitive};

/// Knud Thomsen's exponent. The approximation is exact for a sphere and off by at most about 1%.
const THOMSEN_P: f64 = 1.6075;

/// A primitive at a size. Lengths in meters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Ellipsoid { semi_axes: DVec3 },
    /// `length` is the straight section between the two hemispheres.
    Capsule { radius: f64, length: f64 },
    /// Full edges, with every edge and corner rounded to `corner`, which is at most half the
    /// shortest edge.
    Slab { edges: DVec3, corner: f64 },
    Cylinder { radius: f64, length: f64 },
    /// `major` is from the axis to the center of the tube, `minor` the tube's radius.
    Torus { major: f64, minor: f64 },
    /// `start` is the radius of the end at −x, `end` of the one at +x.
    Frustum { length: f64, start: f64, end: f64 },
}

impl Primitive {
    /// The Mind's primitive: a cube, whatever the form stores for it.
    pub const MIND: Primitive = Primitive::Slab { edges: DVec3::ONE, corner: 0.0 };

    /// `scale` is the ellipsoid's semi-axis along x, the slab's edge along x, the torus's minor
    /// radius, the frustum's radius at −x, and the radius of the rest.
    pub fn at(&self, scale: f64) -> Shape {
        match *self {
            Primitive::Ellipsoid { axes } => Shape::Ellipsoid { semi_axes: axes * (scale / axes.x) },
            Primitive::Capsule { length } => Shape::Capsule { radius: scale, length: length * scale },
            Primitive::Slab { edges, corner } => {
                let edges = edges * (scale / edges.x);
                Shape::Slab { edges, corner: corner * edges.min_element() }
            }
            Primitive::Cylinder { length } => Shape::Cylinder { radius: scale, length: length * scale },
            Primitive::Torus { major } => Shape::Torus { major: major * scale, minor: scale },
            Primitive::Frustum { length, taper } => {
                Shape::Frustum { length: length * scale, start: scale, end: taper * scale }
            }
        }
    }

    pub fn volume(&self, scale: f64) -> f64 {
        self.at(1.0).volume() * scale.powi(3)
    }

    /// Exact to a few ulps, so the volume a client sends is the volume it gets back.
    pub fn scale(&self, volume_m3: f64) -> f64 {
        (volume_m3 / self.at(1.0).volume()).cbrt()
    }

    pub fn area(&self, scale: f64) -> f64 {
        self.at(1.0).area() * scale * scale
    }
}

impl Shape {
    pub fn volume(&self) -> f64 {
        match *self {
            Shape::Ellipsoid { semi_axes: s } => 4.0 / 3.0 * PI * s.x * s.y * s.z,
            Shape::Capsule { radius: r, length } => PI * r * r * (length + 4.0 / 3.0 * r),
            Shape::Slab { edges, corner: r } => {
                let core = edges - 2.0 * r;
                let [x, y, z] = core.to_array();
                x * y * z + 2.0 * r * (x * y + y * z + z * x) + PI * r * r * (x + y + z) + 4.0 / 3.0 * PI * r.powi(3)
            }
            Shape::Cylinder { radius: r, length } => PI * r * r * length,
            Shape::Torus { major, minor } => 2.0 * PI * PI * major * minor * minor,
            Shape::Frustum { length, start: a, end: b } => PI * length * (a * a + a * b + b * b) / 3.0,
        }
    }

    /// The ellipsoid's is Thomsen's approximation; the rest are exact.
    pub fn area(&self) -> f64 {
        match *self {
            Shape::Ellipsoid { semi_axes: s } => {
                let [a, b, c] = s.to_array().map(|x| x.powf(THOMSEN_P));
                4.0 * PI * ((a * b + b * c + c * a) / 3.0).powf(1.0 / THOMSEN_P)
            }
            Shape::Capsule { radius: r, length } => 2.0 * PI * r * (length + 2.0 * r),
            Shape::Slab { edges, corner: r } => {
                let core = edges - 2.0 * r;
                let [x, y, z] = core.to_array();
                2.0 * (x * y + y * z + z * x) + 2.0 * PI * r * (x + y + z) + 4.0 * PI * r * r
            }
            Shape::Cylinder { radius: r, length } => 2.0 * PI * r * (length + r),
            Shape::Torus { major, minor } => 4.0 * PI * PI * major * minor,
            Shape::Frustum { length, start: a, end: b } => {
                PI * ((a + b) * (a - b).hypot(length) + a * a + b * b)
            }
        }
    }
}

impl Part {
    /// `min_part_m3` is a parameter until K2's `Balance` lands; F4 and F5 read it from there.
    pub fn mind(id: PartId, min_part_m3: f64) -> Part {
        Part { id, kind: Kind::Mind, primitive: Primitive::MIND, volume_m3: min_part_m3, placement: None }
    }

    /// The part at its size. A Mind is the cube of `min_part_m3` whatever it stores.
    pub fn shape(&self, min_part_m3: f64) -> Shape {
        match self.kind {
            Kind::Mind => Primitive::MIND.at(Primitive::MIND.scale(min_part_m3)),
            _ => self.primitive.at(self.primitive.scale(self.volume_m3)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spread() -> Vec<Primitive> {
        let mut all = Vec::new();
        for axes in [DVec3::ONE, DVec3::new(3.0, 1.0, 1.0), DVec3::new(0.2, 1.0, 5.0), DVec3::new(1.0, 2.0, 0.5)] {
            all.push(Primitive::Ellipsoid { axes });
        }
        for length in [0.0, 0.5, 4.0, 40.0] {
            all.push(Primitive::Capsule { length });
            if length > 0.0 {
                all.push(Primitive::Cylinder { length });
            }
        }
        for (edges, corner) in [
            (DVec3::ONE, 0.0),
            (DVec3::ONE, 0.5),
            (DVec3::new(4.0, 1.0, 2.0), 0.25),
            (DVec3::new(0.3, 2.0, 1.0), 0.5),
            (DVec3::new(1.0, 1.0, 10.0), 0.1),
        ] {
            all.push(Primitive::Slab { edges, corner });
        }
        for major in [1.0, 1.5, 4.0, 30.0] {
            all.push(Primitive::Torus { major });
        }
        for (length, taper) in [(1.0, 0.0), (2.0, 0.5), (0.3, 1.0), (5.0, 3.0), (10.0, 0.1)] {
            all.push(Primitive::Frustum { length, taper });
        }
        all
    }

    #[test]
    fn scale_solved_from_a_volume_gives_the_volume_back() {
        for primitive in spread() {
            for volume in [1e-3, 1.0, 1_000.0, 7_654_321.0, 3.3e12] {
                let scale = primitive.scale(volume);
                assert!(scale.is_finite() && scale > 0.0, "{primitive:?}");
                let back = primitive.volume(scale);
                assert!(((back - volume) / volume).abs() < 1e-12, "{primitive:?} at {volume}: {back}");
                let shape = primitive.at(scale);
                assert!(((shape.volume() - volume) / volume).abs() < 1e-12, "{primitive:?}");
                assert!(((shape.area() - primitive.area(scale)) / shape.area()).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn volume_goes_as_the_cube_and_area_as_the_square() {
        for primitive in spread() {
            let ratio = primitive.volume(3.0) / primitive.volume(1.0);
            assert!((ratio - 27.0).abs() < 1e-12, "{primitive:?}");
            let ratio = primitive.area(3.0) / primitive.area(1.0);
            assert!((ratio - 9.0).abs() < 1e-12, "{primitive:?}");
        }
    }

    #[test]
    fn the_scale_is_the_dimension_each_primitive_names() {
        let s = 2.5;
        let at = |p: Primitive| p.at(s);
        assert_eq!(at(Primitive::Ellipsoid { axes: DVec3::new(2.0, 1.0, 4.0) }), Shape::Ellipsoid {
            semi_axes: DVec3::new(2.5, 1.25, 5.0)
        });
        assert_eq!(at(Primitive::Capsule { length: 2.0 }), Shape::Capsule { radius: 2.5, length: 5.0 });
        assert_eq!(at(Primitive::Slab { edges: DVec3::new(2.0, 4.0, 1.0), corner: 0.5 }), Shape::Slab {
            edges: DVec3::new(2.5, 5.0, 1.25),
            corner: 0.625
        });
        assert_eq!(at(Primitive::Cylinder { length: 2.0 }), Shape::Cylinder { radius: 2.5, length: 5.0 });
        assert_eq!(at(Primitive::Torus { major: 3.0 }), Shape::Torus { major: 7.5, minor: 2.5 });
        assert_eq!(at(Primitive::Frustum { length: 2.0, taper: 0.5 }), Shape::Frustum {
            length: 5.0,
            start: 2.5,
            end: 1.25
        });
    }

    #[test]
    fn the_mind_is_a_cube_of_min_part_m3_whatever_it_stores() {
        let mind = Part::mind(PartId(0), 1_000.0);
        let Shape::Slab { edges, corner } = mind.shape(1_000.0) else { panic!() };
        assert!((edges - DVec3::splat(10.0)).abs().max_element() < 1e-12);
        assert_eq!(corner, 0.0);

        let bloated = Part { primitive: Primitive::Torus { major: 2.0 }, volume_m3: 1e9, ..mind };
        assert_eq!(bloated.shape(1_000.0), mind.shape(1_000.0));
    }

    #[test]
    fn degenerate_proportions_still_have_a_finite_scale() {
        let sphere = Primitive::Capsule { length: 0.0 };
        assert!((sphere.scale(4.0 / 3.0 * PI) - 1.0).abs() < 1e-15);
        assert!((sphere.area(1.0) - 4.0 * PI).abs() < 1e-14);

        let cone = Primitive::Frustum { length: 3.0, taper: 0.0 };
        assert!((cone.scale(PI) - 1.0).abs() < 1e-15);
        assert!((cone.area(1.0) - PI * (1.0 + 10f64.sqrt())).abs() < 1e-14);

        let horn = Primitive::Torus { major: 1.0 };
        assert!(horn.scale(1.0).is_finite());
        let knife = Primitive::Slab { edges: DVec3::new(1.0, 1e-9, 1.0), corner: 0.5 };
        assert!(knife.scale(1.0).is_finite());
    }

    fn inside(shape: &Shape, p: DVec3) -> bool {
        match *shape {
            Shape::Ellipsoid { semi_axes } => (p / semi_axes).length_squared() <= 1.0,
            Shape::Slab { edges, corner } => (p.abs() - (edges / 2.0 - corner)).max(DVec3::ZERO).length() <= corner,
            _ => unreachable!("solids of revolution are lathed"),
        }
    }

    /// Where the ray from the origin along `d` leaves a convex shape, by bisection on [`inside`],
    /// so nothing here shares a formula with the code under test.
    fn surface(shape: &Shape, d: DVec3) -> DVec3 {
        let (mut lo, mut hi) = (0.0, 1.0);
        while inside(shape, d * hi) {
            lo = hi;
            hi *= 2.0;
        }
        for _ in 0..55 {
            let mid = 0.5 * (lo + hi);
            if inside(shape, d * mid) { lo = mid } else { hi = mid }
        }
        d * lo
    }

    /// Area and volume of a closed mesh given as a grid of points, summing triangles and the
    /// signed tetrahedra they make with the origin.
    fn measure(grid: &[Vec<DVec3>]) -> (f64, f64) {
        let (mut area, mut volume) = (0.0, 0.0);
        for (row, next) in grid.iter().zip(&grid[1..]) {
            for j in 0..row.len() - 1 {
                for [a, b, c] in [[row[j], row[j + 1], next[j + 1]], [row[j], next[j + 1], next[j]]] {
                    let n = (b - a).cross(c - a);
                    area += n.length() / 2.0;
                    volume += a.dot(n) / 6.0;
                }
            }
        }
        (area, volume)
    }

    /// A convex shape meshed over the six faces of a cube of directions, stretched to the shape's
    /// extents so an elongated one is sampled evenly. A box's faces land on the grid exactly.
    fn ray_cast(shape: &Shape, n: usize) -> (f64, f64) {
        let extents = DVec3::new(surface(shape, DVec3::X).x, surface(shape, DVec3::Y).y, surface(shape, DVec3::Z).z);
        let (mut area, mut volume) = (0.0, 0.0);
        for axis in 0..3 {
            for sign in [1.0, -1.0] {
                let grid: Vec<Vec<DVec3>> = (0..=n)
                    .map(|i| {
                        (0..=n)
                            .map(|j| {
                                let mut d = [0.0; 3];
                                d[axis] = sign;
                                d[(axis + 1) % 3] = (PI / 4.0 * (2.0 * i as f64 / n as f64 - 1.0)).tan();
                                d[(axis + 2) % 3] = (PI / 4.0 * (2.0 * j as f64 / n as f64 - 1.0)).tan();
                                surface(shape, (extents * DVec3::from_array(d).normalize()).normalize())
                            })
                            .collect()
                    })
                    .collect();
                let (a, v) = measure(&grid);
                area += a;
                volume += v.abs();
            }
        }
        (area, volume)
    }

    /// A profile of (x, distance from the axis) turned about the x axis. Every rim is a vertex of
    /// the profile, so the only error is the polygon standing in for each circle.
    fn lathe(profile: &[(f64, f64)], turns: usize) -> (f64, f64) {
        let grid: Vec<Vec<DVec3>> = (0..=turns)
            .map(|i| {
                let (sin, cos) = (2.0 * PI * i as f64 / turns as f64).sin_cos();
                profile.iter().map(|&(x, r)| DVec3::new(x, r * cos, r * sin)).collect()
            })
            .collect();
        let (area, volume) = measure(&grid);
        (area, volume.abs())
    }

    fn arc(center: (f64, f64), radius: f64, from: f64, to: f64, steps: usize) -> impl Iterator<Item = (f64, f64)> {
        (0..=steps).map(move |k| {
            let t = from + (to - from) * k as f64 / steps as f64;
            (center.0 + radius * t.cos(), center.1 + radius * t.sin())
        })
    }

    /// At `fine` segments to a circle or a cube face. Every mesh here is smooth between vertices it
    /// places exactly, so its error goes as `1/fine²`.
    fn tessellate(shape: &Shape, fine: usize) -> (f64, f64) {
        match *shape {
            Shape::Ellipsoid { .. } | Shape::Slab { .. } => ray_cast(shape, fine / 4),
            Shape::Capsule { radius: r, length } => {
                let h = length / 2.0;
                let back = arc((-h, 0.0), r, PI, PI / 2.0, fine / 4);
                let front = arc((h, 0.0), r, PI / 2.0, 0.0, fine / 4);
                lathe(&back.chain(front).collect::<Vec<_>>(), fine)
            }
            Shape::Cylinder { radius: r, length } => {
                let h = length / 2.0;
                lathe(&[(-h, 0.0), (-h, r), (h, r), (h, 0.0)], fine)
            }
            Shape::Frustum { length, start, end } => {
                let h = length / 2.0;
                lathe(&[(-h, 0.0), (-h, start), (h, end), (h, 0.0)], fine)
            }
            Shape::Torus { major, minor } => {
                lathe(&arc((0.0, major), minor, 0.0, 2.0 * PI, fine).collect::<Vec<_>>(), fine)
            }
        }
    }

    /// Two meshes, Richardson-extrapolated to cancel the `1/fine²` term.
    fn measured(shape: &Shape) -> (f64, f64) {
        let (a1, v1) = tessellate(shape, 400);
        let (a2, v2) = tessellate(shape, 800);
        ((4.0 * a2 - a1) / 3.0, (4.0 * v2 - v1) / 3.0)
    }

    #[test]
    fn area_and_volume_agree_with_a_fine_tessellation() {
        for primitive in spread() {
            let shape = primitive.at(1.7);
            let (area, volume) = measured(&shape);
            // Thomsen is good to about 1%. A rounded slab's curvature jumps where a face meets its
            // rounding, which spoils the extrapolation.
            let (area_tolerance, volume_tolerance) = match primitive {
                Primitive::Ellipsoid { .. } => (1.5e-2, 1e-8),
                Primitive::Slab { corner, .. } if corner > 0.0 => (1e-4, 1e-4),
                _ => (1e-8, 1e-8),
            };
            let area_err = (shape.area() - area).abs() / area;
            let volume_err = (shape.volume() - volume).abs() / volume;
            assert!(area_err < area_tolerance, "{primitive:?}: area {} against {area}", shape.area());
            assert!(volume_err < volume_tolerance, "{primitive:?}: volume {} against {volume}", shape.volume());
        }
    }
}
