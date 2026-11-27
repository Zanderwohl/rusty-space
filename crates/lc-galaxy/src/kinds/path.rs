//! A tube along a Catmull-Rom spline through control points: a stellar stream, a filament, a
//! spur off an arm. Each point carries the tube's radius there.

use std::sync::Arc;

use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::plummer::Contribution;
use super::shape::{self, Field, ShapeKind, Unit, ball_volume, disc_area, require, resolve, taper};
use crate::compiled::{Aabb, Compiled, Shape};
use crate::feature::{FeatureId, FeatureKind, Problem};
use crate::num::Num;

/// Straight pieces per span between control points.
const PIECES_PER_SPAN: usize = 24;

/// The fastest the radius may change per ly along the path. Past 1, one node's sphere swallows
/// the next; well below it, the volume's correction for the taper stays accurate.
pub const MAX_TAPER: f64 = 0.3;

/// One control point, in the path's frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathPoint {
    pub at_ly: (f64, f64, f64),
    pub radius_ly: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PathKind {
    pub frame: FeatureId,
    pub points: Vec<PathPoint>,
    pub edge_ly: Num,
    pub contributes: Vec<Contribution>,
}

/// The curve through `points` as straight pieces, with the radius at each node: uniform
/// Catmull-Rom for the position, so it passes through every point, and linear for the radius.
/// The ends are extended by reflection, so the curve leaves an end toward its neighbor.
pub fn curve(points: &[(DVec3, f64)]) -> Vec<(DVec3, f64)> {
    let n = points.len();
    if n < 2 {
        return points.to_vec();
    }
    let at = |i: isize| -> DVec3 {
        match i {
            -1 => 2.0 * points[0].0 - points[1].0,
            i if i as usize >= n => 2.0 * points[n - 1].0 - points[n - 2].0,
            i => points[i as usize].0,
        }
    };
    let mut out = Vec::with_capacity((n - 1) * PIECES_PER_SPAN + 1);
    for s in 0..n - 1 {
        let (p0, p1, p2, p3) = (at(s as isize - 1), at(s as isize), at(s as isize + 1), at(s as isize + 2));
        for k in 0..PIECES_PER_SPAN {
            let t = k as f64 / PIECES_PER_SPAN as f64;
            let (t2, t3) = (t * t, t * t * t);
            let p = 0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3);
            out.push((p, points[s].1 + (points[s + 1].1 - points[s].1) * t));
        }
    }
    out.push(points[n - 1]);
    out
}

/// The tube, in galaxy coordinates: a frame's rotation does not change a radius.
#[derive(Debug)]
struct Tube {
    nodes: Vec<(DVec3, f64)>,
    edge: f64,
    bounds: Aabb,
    lipschitz: f64,
}

impl Tube {
    fn new(nodes: Vec<(DVec3, f64)>, edge: f64) -> Self {
        let bounds = nodes.iter().fold(Aabb::EMPTY, |b, &(p, r)| b.union(Aabb::around(p, DVec3::splat(r))));
        // The radius changes along a piece at `|Δr| / length`, and the nearest point on it moves
        // no faster than the point measured from.
        let slope = nodes.windows(2).map(|w| (w[1].1 - w[0].1).abs() / w[0].0.distance(w[1].0).max(1e-12)).fold(0.0, f64::max);
        Self { nodes, edge, bounds, lipschitz: 1.0 + slope }
    }

    /// For each piece: the distance from `p` to it and the tube's radius at the nearest point.
    fn pieces(&self, p: DVec3) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.nodes.windows(2).map(move |w| {
            let ((a, ra), (b, rb)) = (w[0], w[1]);
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-24)).clamp(0.0, 1.0);
            (p.distance(a + ab * t), ra + (rb - ra) * t)
        })
    }

    /// Each node's sphere reaches past where the next piece has narrowed, so a tube tapering at
    /// slope `k` is the envelope of those spheres, whose cross-section is wider than the radius
    /// by `1 / √(1 - k²)`. Ignores the overlap inside a bend tighter than the radius.
    fn volume(&self) -> f64 {
        let sides: f64 = self
            .nodes
            .windows(2)
            .map(|w| {
                let (r, length) = (0.5 * (w[0].1 + w[1].1), w[0].0.distance(w[1].0));
                let k = (w[1].1 - w[0].1) / length;
                length * r * r * disc_area(self.edge / r) / (1.0 - k * k)
            })
            .sum();
        let cap = |r: f64| 0.5 * r * r * r * ball_volume(self.edge / r);
        sides + cap(self.nodes[0].1) + cap(self.nodes[self.nodes.len() - 1].1)
    }
}

impl Shape for Tube {
    fn distance(&self, p: DVec3) -> f64 {
        self.pieces(p).map(|(d, r)| d - r).fold(f64::INFINITY, f64::min)
    }

    fn weight(&self, p: DVec3) -> f64 {
        if !self.bounds.contains(p) {
            return 0.0;
        }
        self.pieces(p).map(|(d, r)| taper(d / r, self.edge / r)).fold(0.0, f64::max)
    }

    fn lipschitz(&self) -> f64 {
        self.lipschitz
    }

    fn bounds(&self) -> Aabb {
        self.bounds
    }
}

impl FeatureKind for PathKind {
    const KIND: &'static str = "path";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "points", "edge_ly", "contributes"];

    fn new() -> Self {
        let point = |x: f64, y: f64, radius_ly: f64| PathPoint { at_ly: (x, y, 0.0), radius_ly };
        Self {
            frame: FeatureId(0),
            points: vec![point(-20.0, -10.0, 3.0), point(0.0, 5.0, 2.5), point(20.0, 0.0, 2.0)],
            edge_ly: Num::Lit(1.0),
            contributes: Vec::new(),
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        shape::depends_on(self.frame, &self.contributes)
    }

    fn variables(&self) -> Vec<String> {
        shape::variables(&[&self.edge_ly], &self.contributes)
    }

    fn summary(&self) -> String {
        format!("{} points", self.points.len())
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        let frame = super::frame(out, self.frame)?.clone();
        let edge = resolve(&self.edge_ly, out)?;
        require(self.points.len() >= 2, "needs at least two points")?;
        let smallest = self.points.iter().map(|p| p.radius_ly).fold(f64::INFINITY, f64::min);
        require(smallest > 0.0 && (0.0..=smallest).contains(&edge), "needs positive radii and 0 ≤ edge ≤ the smallest")?;
        let placed: Vec<(DVec3, f64)> = self.points.iter().map(|p| (frame.to_galaxy(DVec3::from(p.at_ly)), p.radius_ly)).collect();
        require(placed.windows(2).all(|w| w[0].0 != w[1].0), "has two points in the same place")?;
        let tube = Tube::new(curve(&placed), edge);
        require(tube.lipschitz - 1.0 <= MAX_TAPER, &format!("changes radius faster than {MAX_TAPER} ly per ly along it"))?;
        let volume = tube.volume();
        shape::contribute(id, out, &self.contributes, vec![(Arc::new(tube), volume)])
    }
}

impl ShapeKind for PathKind {
    fn frame_mut(&mut self) -> &mut FeatureId {
        &mut self.frame
    }

    fn contributes_mut(&mut self) -> &mut Vec<Contribution> {
        &mut self.contributes
    }

    fn fields(&mut self) -> Vec<Field<'_>> {
        vec![Field::Num("edge", &mut self.edge_ly, Unit::Ly)]
    }

    fn points_mut(&mut self) -> Option<&mut Vec<PathPoint>> {
        Some(&mut self.points)
    }
}
