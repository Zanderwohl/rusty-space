//! The form sampled at the centers of a fixed grid, and what the server reads off it: the shadow in
//! every direction, broadside, the envelope, the inertia tensor and the extent. See 29 §What the
//! server computes from a form.
//!
//! A cell is filled where [`Sdf::distance`] is not positive. The envelope is not sampled: it is
//! an ellipsoid, [`Envelope`], and read in closed form.
//!
//! Every loop runs in a fixed order over IEEE arithmetic, `sqrt` and `libm`, so the server's
//! `Fitted` and a client's preview agree to the bit.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use glam::{DMat3, DVec2, DVec3};

use super::capacity::mass_fraction;
use super::sdf::Sdf;
use super::{Form, FormError};
use crate::fitting::Balance;

mod envelope;
pub use envelope::Envelope;

/// Cells along the longest side of the padded box, whatever the ship's size.
pub const FORM_GRID: usize = 64;

/// Vertices of a twice-subdivided icosahedron.
pub const SHADOW_DIRECTIONS: usize = 162;

/// Cells of padding beyond the hull's bounds, so the outermost samples are outside it.
const EDGE_CELLS: f64 = 2.0;

/// Room round the hull's bounds, over the cube root of its volume: the box, and so the cell, the
/// broadside and inertia anchors were solved in.
const PAD: f64 = 0.2625;

/// Below this the field's slope across a cell is not a surface's: a part thinner than two cells,
/// or a crease. Well under the bound's least slope off an ellipsoid, its axis ratio.
const GRADIENT_FLOOR: f64 = 0.1;

/// Pixels per cell along each side of the shadow's raster.
const PIXELS_PER_CELL: f64 = 2.0;

/// Broadside's quadratic is fitted to the vertices within about 0.6 rad of the largest: two rings,
/// nineteen of them.
const FIT_COS: f64 = 0.825;
/// Past this, in the fit's gnomonic plane, the peak is the fit's extrapolation and the best vertex
/// stands.
const FIT_REACH: f64 = 0.3;

const ROLL_SAMPLES: usize = 72;
/// Projections either side of the coarse roll, a sample apart, that its parabola is fitted to.
const ROLL_STENCIL: usize = 4;

#[derive(Clone, Debug)]
pub struct FormGrid {
    sdf: Sdf,
    /// Center of cell `[0, 0, 0]`, ship frame.
    origin: DVec3,
    cell_m: f64,
    dims: [usize; 3],
    /// [`Sdf::distance`] at each cell center, x fastest, then y, then z.
    hull: Vec<f64>,
    envelope: Envelope,
    shadow: Shadow,
    broadside: DVec3,
    broadside_m2: f64,
    roll_rad: f64,
    inertia: Inertia,
}

/// The moments of the filled cells, each weighted by the density of the nearest part's kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inertia {
    /// Ship frame, meters.
    pub center_of_mass: DVec3,
    /// About the center of mass, per kilogram: m². Symmetric.
    pub per_kg: DMat3,
    /// What the filled cells weigh: contents only.
    pub cells_kg: f64,
}

impl Inertia {
    /// The tensor for a ship weighing `mass_kg`, kg·m². Everything the cells do not carry — hull
    /// structure, stored energy, heat — is taken to lie where the contents do, so slew answers to
    /// the same mass the drive pushes. [`dry_mass_kg`] is the mass of a ship with nothing stored.
    ///
    /// [`dry_mass_kg`]: super::capacity::dry_mass_kg
    pub fn tensor_kg_m2(&self, mass_kg: f64) -> DMat3 {
        self.per_kg * mass_kg
    }
}

/// Projected area toward each of [`Shadow::directions`], interpolated between them.
#[derive(Clone, Debug, PartialEq)]
pub struct Shadow {
    m2: [f64; SHADOW_DIRECTIONS],
}

impl Shadow {
    /// The wire carries the table in this order.
    pub fn from_m2(m2: [f64; SHADOW_DIRECTIONS]) -> Shadow {
        Shadow { m2 }
    }

    pub fn m2(&self) -> &[f64; SHADOW_DIRECTIONS] {
        &self.m2
    }

    /// Unit vectors, ship frame. Antipodes are exact negatives of each other.
    pub fn directions() -> &'static [DVec3] {
        &icosphere().vertices
    }

    /// m² along `direction`, which need not be normalized: linear across the face of the
    /// icosahedron it passes through. Zero for a zero vector.
    pub fn along(&self, direction: DVec3) -> f64 {
        let s = direction.normalize_or_zero();
        if s == DVec3::ZERO {
            return 0.0;
        }
        let ico = icosphere();
        let nearest = (0..ico.vertices.len())
            .fold((0, f64::NEG_INFINITY), |best, v| {
                let dot = ico.vertices[v].dot(s);
                if dot > best.1 { (v, dot) } else { best }
            })
            .0;
        let (mut face, mut weights) = ico.weights(ico.around[nearest][0], s);
        for &f in &ico.around[nearest][1..] {
            let w = ico.weights(f, s);
            if w.1.min_element() > weights.min_element() {
                (face, weights) = w;
            }
        }
        // The nearest vertex is a corner of the face it lies in on this mesh; searched anyway.
        if weights.min_element() < -1e-12 {
            for f in 0..ico.faces.len() {
                let w = ico.weights(f, s);
                if w.1.min_element() > weights.min_element() {
                    (face, weights) = w;
                }
            }
        }
        let [a, b, c] = ico.faces[face];
        (weights.x * self.m2[a] + weights.y * self.m2[b] + weights.z * self.m2[c]) / weights.element_sum()
    }
}

impl FormGrid {
    pub fn new(form: &Form, balance: &Balance) -> Result<FormGrid, FormError> {
        let sdf = Sdf::new(form, balance)?;
        let envelope = Envelope::of(&sdf, balance);
        let volume_m3: f64 = sdf.pieces().iter().map(|p| p.shape.volume()).sum();
        let (lo, hi) = sdf.bounds();
        let center = (lo + hi) / 2.0;
        let size = hi - lo + 2.0 * PAD * libm::cbrt(volume_m3);
        let cell_m = size.max_element() / (FORM_GRID as f64 - 2.0 * EDGE_CELLS);
        let dims = size.to_array().map(|s| ((s / cell_m).ceil() as usize + 2 * EDGE_CELLS as usize).min(FORM_GRID));
        let mut grid = FormGrid {
            sdf,
            origin: center - DVec3::from_array(dims.map(|n| (n - 1) as f64)) * (cell_m / 2.0),
            cell_m,
            dims,
            hull: Vec::new(),
            envelope,
            shadow: Shadow { m2: [0.0; SHADOW_DIRECTIONS] },
            broadside: DVec3::Z,
            broadside_m2: 0.0,
            roll_rad: 0.0,
            inertia: Inertia { center_of_mass: center, per_kg: DMat3::ZERO, cells_kg: 0.0 },
        };
        grid.sample(balance);
        let silhouette = grid.silhouette();
        grid.fill_shadow(silhouette);
        Ok(grid)
    }

    pub fn sdf(&self) -> &Sdf {
        &self.sdf
    }

    pub fn dims(&self) -> [usize; 3] {
        self.dims
    }

    pub fn cell_m(&self) -> f64 {
        self.cell_m
    }

    /// Ship frame.
    pub fn cell_center(&self, i: usize, j: usize, k: usize) -> DVec3 {
        self.origin + DVec3::new(i as f64, j as f64, k as f64) * self.cell_m
    }

    fn index(&self, i: usize, j: usize, k: usize) -> usize {
        i + self.dims[0] * (j + self.dims[1] * k)
    }

    /// [`Sdf::distance`] at every cell center, x fastest, then y, then z.
    pub fn hull_samples(&self) -> &[f64] {
        &self.hull
    }

    pub fn filled(&self, i: usize, j: usize, k: usize) -> bool {
        self.hull[self.index(i, j, k)] <= 0.0
    }

    pub fn envelope(&self) -> &Envelope {
        &self.envelope
    }

    /// The envelope's signed distance at any point, ship frame, meters, negative inside.
    pub fn envelope_at(&self, p: DVec3) -> f64 {
        self.envelope.distance(p)
    }

    pub fn envelope_area_m2(&self) -> f64 {
        self.envelope.area_m2()
    }

    pub fn envelope_volume_m3(&self) -> f64 {
        self.envelope.volume_m3()
    }

    /// The envelope's longest dimension: `length_m`.
    pub fn extent_m(&self) -> f64 {
        self.envelope.extent_m()
    }

    pub fn shadow(&self) -> &Shadow {
        &self.shadow
    }

    /// The direction of largest shadow, ship frame, signed so its largest component is positive:
    /// the shadow is the same from either side.
    pub fn broadside(&self) -> DVec3 {
        self.broadside
    }

    pub fn broadside_m2(&self) -> f64 {
        self.broadside_m2
    }

    /// Radians in `[−π/2, π/2)`, right-handed about the nose: the roll that carries the ship's +z
    /// onto the direction across the nose with the largest shadow. A ship holding its nose square to
    /// its star presents the most it can by rolling this far from z-to-the-star.
    pub fn roll_rad(&self) -> f64 {
        self.roll_rad
    }

    pub fn inertia(&self) -> &Inertia {
        &self.inertia
    }

    /// Radius of gyration about the axis across the nose the form turns slowest about, m: the
    /// larger eigenvalue of the tensor's block across the nose, per kilogram. What slew goes by.
    pub fn gyration_m(&self) -> f64 {
        let i = self.inertia.per_kg;
        let (a, c, b) = (i.y_axis.y, i.z_axis.z, i.z_axis.y);
        (0.5 * (a + c) + (0.25 * (a - c) * (a - c) + b * b).sqrt()).sqrt()
    }

    /// What `Fitted` states, for a ship weighing `mass_kg`.
    pub fn geometry(&self, mass_kg: f64) -> lc_proto::form::Geometry {
        let t = self.inertia.tensor_kg_m2(mass_kg);
        lc_proto::form::Geometry {
            shadow_m2: self.shadow.m2.to_vec(),
            broadside: self.broadside.to_array(),
            broadside_roll_rad: self.roll_rad,
            envelope_area_m2: self.envelope_area_m2(),
            envelope_volume_m3: self.envelope_volume_m3(),
            inertia_kg_m2: [t.x_axis.x, t.y_axis.y, t.z_axis.z, t.y_axis.x, t.z_axis.x, t.z_axis.y],
            extent_m: self.extent_m(),
        }
    }

    /// The hull's field at every center, and the moments of the filled cells.
    fn sample(&mut self, balance: &Balance) {
        let [nx, ny, nz] = self.dims;
        let n = nx * ny * nz;
        let mut hull = Vec::with_capacity(n);
        let (mut scratch, mut each) = (Vec::new(), Vec::new());
        let density: Vec<f64> = self
            .sdf
            .pieces()
            .iter()
            .map(|p| balance.module_density_kg_m3 * mass_fraction(p.kind, balance))
            .collect();

        // Moments about the grid's center, where offsets are small, and moved to the center of
        // mass only at the end: about the origin they would cancel.
        let center = self.origin + DVec3::from_array(self.dims.map(|n| (n - 1) as f64)) * (self.cell_m / 2.0);
        let cell_m3 = self.cell_m.powi(3);
        let (mut mass, mut first, mut second) = (0.0, DVec3::ZERO, DMat3::ZERO);
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let p = self.cell_center(i, j, k);
                    let d = self.sdf.distance_with(p, &mut scratch);
                    hull.push(d);
                    if d <= 0.0 {
                        // The same pieces as `Sdf::nearest`, ranked by the truer distance.
                        self.sdf.estimate_each_with(p, &mut scratch, &mut each);
                        let nearest = (0..each.len()).fold(0, |best, i| if each[i] < each[best] { i } else { best });
                        let m = density[nearest] * cell_m3;
                        let r = p - center;
                        mass += m;
                        first += r * m;
                        second += outer(r, r) * m;
                    }
                }
            }
        }
        self.hull = hull;

        if mass > 0.0 {
            let c = first / mass;
            let spread = second - outer(c, c) * mass;
            // Each cell is a cube, not a point: h²/6 about each axis.
            let own = mass * self.cell_m * self.cell_m / 6.0;
            let tensor = DMat3::from_diagonal(DVec3::splat(spread.x_axis.x + spread.y_axis.y + spread.z_axis.z + own)) - spread;
            self.inertia = Inertia { center_of_mass: center + c, per_kg: tensor * (1.0 / mass), cells_kg: mass };
        }
    }

    /// The cells on either side of the surface, each filled cell whole unless its field says
    /// where the surface crosses it. A line through the solid leaves it through one of these, so
    /// they cast the whole shadow. Whole cubes alone stand out past the rim by up to a cell's
    /// half-diagonal, which on a hull a few cells thick is a tenth of its shadow.
    fn silhouette(&self) -> Silhouette {
        let [nx, ny, nz] = self.dims;
        let h = self.cell_m;
        let center = DVec3::from_array(self.dims.map(|n| (n - 1) as f64)) / 2.0;
        let at = |i: usize, j: usize, k: usize| self.hull[self.index(i, j, k)];
        let mut silhouette = Silhouette { cubes: Vec::new(), cut: Vec::new(), cell_m: h, raster: Vec::new() };
        // The padding keeps every filled cell off the grid's faces.
        for k in 1..nz - 1 {
            for j in 1..ny - 1 {
                for i in 1..nx - 1 {
                    let d = at(i, j, k);
                    let neighbors = [at(i - 1, j, k), at(i + 1, j, k), at(i, j - 1, k), at(i, j + 1, k), at(i, j, k - 1), at(i, j, k + 1)];
                    let filled = d <= 0.0;
                    if neighbors.iter().all(|&n| (n <= 0.0) == filled) {
                        continue;
                    }
                    let c = (DVec3::new(i as f64, j as f64, k as f64) - center) * h;
                    let gradient =
                        DVec3::new(neighbors[1] - neighbors[0], neighbors[3] - neighbors[2], neighbors[5] - neighbors[4]) / (2.0 * h);
                    let slope = gradient.length();
                    // Across a part thinner than two cells or a crease the difference says nothing.
                    if slope < GRADIENT_FLOOR {
                        if filled {
                            silhouette.cubes.push(c);
                        }
                        continue;
                    }
                    let normal = gradient / slope;
                    // To first order the distance to the zero set along the normal, even where the
                    // field is a bound.
                    let offset = -d / slope;
                    let half = h / 2.0 * normal.abs().element_sum();
                    if offset >= half {
                        silhouette.cubes.push(c);
                    } else if offset > -half {
                        silhouette.cut.push((c, normal, offset));
                    }
                }
            }
        }
        silhouette
    }

    fn fill_shadow(&mut self, mut silhouette: Silhouette) {
        let ico = icosphere();
        let mut m2 = [0.0; SHADOW_DIRECTIONS];
        for v in 0..SHADOW_DIRECTIONS {
            let a = ico.antipode[v];
            // Exactly symmetric, and half the work.
            if a < v {
                m2[v] = m2[a];
            } else {
                m2[v] = silhouette.area(ico.vertices[v]);
            }
        }
        self.shadow = Shadow { m2 };

        // A projection is noisy to a few parts in a thousand at the rim's pixels, and the shadow
        // is flatter than that near its peak, so the peak is fitted over a wide stencil rather
        // than climbed to.
        let top = (0..SHADOW_DIRECTIONS).fold(0, |best, v| if m2[v] > m2[best] { v } else { best });
        let b = ico.vertices[top];
        let (e1, e2) = across(b);
        let stencil = (0..SHADOW_DIRECTIONS).filter(|&v| ico.vertices[v].dot(b) > FIT_COS).map(|v| {
            // Gnomonic, so every point of the plane is a direction.
            let t = ico.vertices[v] / ico.vertices[v].dot(b) - b;
            let (x, y) = (t.dot(e1), t.dot(e2));
            ([1.0, x, y, x * x, x * y, y * y], m2[v] / m2[top])
        });
        let peak = least_squares(stencil).and_then(|[_, c1, c2, c3, c4, c5]| {
            let det = 4.0 * c3 * c5 - c4 * c4;
            if !(c3 < 0.0 && det > 0.0) {
                return None;
            }
            let (x, y) = (-(2.0 * c5 * c1 - c4 * c2) / det, -(2.0 * c3 * c2 - c4 * c1) / det);
            (x * x + y * y < FIT_REACH * FIT_REACH).then(|| (b + e1 * x + e2 * y).normalize())
        });
        let broadside = peak.unwrap_or(b);
        let largest = broadside.abs().max_element();
        let first = broadside.to_array().into_iter().find(|c| c.abs() == largest).unwrap_or(1.0);
        self.broadside = if first < 0.0 { -broadside } else { broadside };
        self.broadside_m2 = silhouette.area(self.broadside);

        let across_nose = |phi: f64| {
            let (s, c) = libm::sincos(phi);
            DVec3::new(0.0, -s, c)
        };
        let half = std::f64::consts::FRAC_PI_2;
        let spacing = std::f64::consts::PI / ROLL_SAMPLES as f64;
        let coarse = (0..ROLL_SAMPLES)
            .map(|k| -half + k as f64 * spacing)
            .fold((0.0, f64::NEG_INFINITY), |best, phi| {
                let area = self.shadow.along(across_nose(phi));
                if area > best.1 { (phi, area) } else { best }
            })
            .0;
        let reach = ROLL_STENCIL as isize;
        let samples: Vec<([f64; 3], f64)> = (-reach..=reach)
            .map(|k| {
                let x = k as f64;
                ([1.0, x, x * x], silhouette.area(across_nose(coarse + x * spacing)))
            })
            .collect();
        let scale = samples[ROLL_STENCIL].1;
        let fitted = least_squares(samples.into_iter().map(|(x, a)| (x, a / scale))).and_then(|[_, c1, c2]| {
            let x = -c1 / (2.0 * c2);
            (c2 < 0.0 && x.abs() <= reach as f64).then_some(x)
        });
        let roll = coarse + fitted.unwrap_or(0.0) * spacing;
        // Half a turn presents the same shadow.
        let pi = std::f64::consts::PI;
        self.roll_rad = if roll < -half { roll + pi } else if roll >= half { roll - pi } else { roll };
    }

}

/// Normal equations, by elimination with partial pivoting. `None` when they are singular.
fn least_squares<const N: usize>(rows: impl Iterator<Item = ([f64; N], f64)>) -> Option<[f64; N]> {
    let mut m = [[0.0; N]; N];
    let mut rhs = [0.0; N];
    for (x, f) in rows {
        for i in 0..N {
            for j in 0..N {
                m[i][j] += x[i] * x[j];
            }
            rhs[i] += x[i] * f;
        }
    }
    for col in 0..N {
        let pivot = (col..N).fold(col, |best, r| if m[r][col].abs() > m[best][col].abs() { r } else { best });
        if m[pivot][col].abs() < 1e-12 * m[0][0].abs() {
            return None;
        }
        m.swap(col, pivot);
        rhs.swap(col, pivot);
        for r in col + 1..N {
            let k = m[r][col] / m[col][col];
            for c in col..N {
                m[r][c] -= k * m[col][c];
            }
            rhs[r] -= k * rhs[col];
        }
    }
    let mut x = [0.0; N];
    for r in (0..N).rev() {
        let tail: f64 = (r + 1..N).map(|c| m[r][c] * x[c]).sum();
        x[r] = (rhs[r] - tail) / m[r][r];
    }
    Some(x)
}

fn outer(a: DVec3, b: DVec3) -> DMat3 {
    DMat3::from_cols(a * b.x, a * b.y, a * b.z)
}

/// Two unit vectors square to `s` and each other. Crossed with whichever axis is least along `s`,
/// so it never degenerates.
fn across(s: DVec3) -> (DVec3, DVec3) {
    let a = s.abs();
    let axis = if a.x <= a.y && a.x <= a.z {
        DVec3::X
    } else if a.y <= a.z {
        DVec3::Y
    } else {
        DVec3::Z
    };
    let u = s.cross(axis).normalize();
    (u, s.cross(u))
}

/// The solid a shadow is cast from, centered on the grid.
struct Silhouette {
    cubes: Vec<DVec3>,
    /// A cell's center, a unit normal and an offset along it: the part of the cube where
    /// `normal · (x − center) ≤ offset`.
    cut: Vec<(DVec3, DVec3, f64)>,
    cell_m: f64,
    raster: Vec<u8>,
}

impl Silhouette {
    /// The area of the solid projected along the unit `s`, point-sampled on a raster of
    /// [`PIXELS_PER_CELL`]. A cube projects to a hexagon, the sum of its three edges' images; a
    /// cut one is tested by clipping the line through each pixel.
    fn area(&mut self, s: DVec3) -> f64 {
        let (u, v) = across(s);
        let h = self.cell_m;
        let edges = [DVec2::new(u.x, v.x) * h, DVec2::new(u.y, v.y) * h, DVec2::new(u.z, v.z) * h];
        // Square to each edge's image, with the hexagon's half-width along it.
        let normals = edges.map(|e| DVec2::new(-e.y, e.x));
        let widths: [f64; 3] = std::array::from_fn(|i| {
            (normals[i].dot(edges[(i + 1) % 3]).abs() + normals[i].dot(edges[(i + 2) % 3]).abs()) / 2.0
        });
        let reach = DVec2::new(
            edges.iter().map(|e| e.x.abs()).sum::<f64>() / 2.0,
            edges.iter().map(|e| e.y.abs()).sum::<f64>() / 2.0,
        );
        let flat = |c: DVec3| DVec2::new(c.dot(u), c.dot(v));

        let (lo, hi) = self
            .cubes
            .iter()
            .chain(self.cut.iter().map(|(c, _, _)| c))
            .fold((DVec2::INFINITY, DVec2::NEG_INFINITY), |(lo, hi), &c| (lo.min(flat(c)), hi.max(flat(c))));
        if lo.x > hi.x {
            return 0.0;
        }
        let px = h / PIXELS_PER_CELL;
        let lo = lo - reach;
        let size = ((hi + reach - lo) / px).ceil();
        let (w, rows) = (size.x as usize + 1, size.y as usize + 1);
        self.raster.clear();
        self.raster.resize(w * rows, 0);

        let raster = &mut self.raster;
        // Pixel centers sit at `lo + (index + ½) px`.
        let mut stamp = |c: DVec2, covers: &dyn Fn(DVec2) -> bool| {
            let first = ((c - reach - lo) / px - 0.5).ceil().max(DVec2::ZERO);
            let last = ((c + reach - lo) / px - 0.5).floor();
            let (x1, y1) = ((last.x as usize).min(w - 1), (last.y as usize).min(rows - 1));
            for y in first.y as usize..=y1 {
                for x in first.x as usize..=x1 {
                    let r = lo + DVec2::new(x as f64 + 0.5, y as f64 + 0.5) * px - c;
                    if raster[x + w * y] == 0 && covers(r) {
                        raster[x + w * y] = 1;
                    }
                }
            }
        };
        let in_hexagon = |r: DVec2| (0..3).all(|i| normals[i].dot(r).abs() <= widths[i]);
        for &c in &self.cubes {
            stamp(flat(c), &in_hexagon);
        }
        for &(c, normal, offset) in &self.cut {
            let clipped = |r: DVec2| {
                if !in_hexagon(r) {
                    return false;
                }
                // The line `c + r.x u + r.y v + t s`, clipped to each slab and then the plane.
                let p = u * r.x + v * r.y;
                let (mut near, mut far) = (f64::NEG_INFINITY, f64::INFINITY);
                for (pi, si) in p.to_array().into_iter().zip(s.to_array()) {
                    if si != 0.0 {
                        let (a, b) = ((-h / 2.0 - pi) / si, (h / 2.0 - pi) / si);
                        near = near.max(a.min(b));
                        far = far.min(a.max(b));
                    }
                }
                let (along, rest) = (normal.dot(s), offset - normal.dot(p));
                if along > 0.0 {
                    far = far.min(rest / along);
                } else if along < 0.0 {
                    near = near.max(rest / along);
                } else if rest < 0.0 {
                    return false;
                }
                near <= far
            };
            stamp(flat(c), &clipped);
        }
        self.raster.iter().filter(|&&b| b != 0).count() as f64 * px * px
    }
}

struct Icosphere {
    vertices: Vec<DVec3>,
    faces: Vec<[usize; 3]>,
    /// Faces meeting at each vertex.
    around: Vec<Vec<usize>>,
    antipode: Vec<usize>,
}

impl Icosphere {
    /// Barycentric weights of `s` in the face, from the cone over it: all positive inside it.
    fn weights(&self, face: usize, s: DVec3) -> (usize, DVec3) {
        let [a, b, c] = self.faces[face].map(|v| self.vertices[v]);
        let det = a.dot(b.cross(c));
        (face, DVec3::new(s.dot(b.cross(c)), a.dot(s.cross(c)), a.dot(b.cross(s))) / det)
    }
}

/// Twelve vertices, each face then split in four twice, midpoints pushed out to the sphere and
/// numbered in the order the faces first reach them.
fn icosphere() -> &'static Icosphere {
    static ICOSPHERE: OnceLock<Icosphere> = OnceLock::new();
    ICOSPHERE.get_or_init(|| {
        let phi = (1.0 + 5f64.sqrt()) / 2.0;
        let mut vertices: Vec<DVec3> = [
            (-1.0, phi, 0.0),
            (1.0, phi, 0.0),
            (-1.0, -phi, 0.0),
            (1.0, -phi, 0.0),
            (0.0, -1.0, phi),
            (0.0, 1.0, phi),
            (0.0, -1.0, -phi),
            (0.0, 1.0, -phi),
            (phi, 0.0, -1.0),
            (phi, 0.0, 1.0),
            (-phi, 0.0, -1.0),
            (-phi, 0.0, 1.0),
        ]
        .into_iter()
        .map(|(x, y, z)| DVec3::new(x, y, z).normalize())
        .collect();
        let mut faces: Vec<[usize; 3]> = vec![
            [0, 11, 5], [0, 5, 1], [0, 1, 7], [0, 7, 10], [0, 10, 11],
            [1, 5, 9], [5, 11, 4], [11, 10, 2], [10, 7, 6], [7, 1, 8],
            [3, 9, 4], [3, 4, 2], [3, 2, 6], [3, 6, 8], [3, 8, 9],
            [4, 9, 5], [2, 4, 11], [6, 2, 10], [8, 6, 7], [9, 8, 1],
        ];
        for _ in 0..2 {
            let mut midpoints = BTreeMap::new();
            let mut split = Vec::with_capacity(faces.len() * 4);
            for [a, b, c] in faces {
                // Antipodal edges sum to exact negatives, so antipodes stay exact.
                let mut mid = |p: usize, q: usize| {
                    *midpoints.entry((p.min(q), p.max(q))).or_insert_with(|| {
                        vertices.push((vertices[p] + vertices[q]).normalize());
                        vertices.len() - 1
                    })
                };
                let (ab, bc, ca) = (mid(a, b), mid(b, c), mid(c, a));
                split.extend([[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
            }
            faces = split;
        }
        let mut around = vec![Vec::new(); vertices.len()];
        for (f, face) in faces.iter().enumerate() {
            for &v in face {
                around[v].push(f);
            }
        }
        let antipode = vertices
            .iter()
            .map(|&p| vertices.iter().position(|&q| q == -p).expect("an icosahedron is centrally symmetric"))
            .collect();
        Icosphere { vertices, faces, around, antipode }
    })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use super::*;
    use crate::form::presets::Builtin;
    use crate::form::{Kind, Mount, Part, PartId, Placement, Primitive};

    const B: Balance = Balance::DEFAULT;

    fn hang(id: u16, kind: Kind, primitive: Primitive, volume_m3: f64, parent: u16, mount: Mount) -> Part {
        let placement = Placement { parent: PartId(parent), mount, twist: 0.0, tilt: DVec2::ZERO, blend: 0.0, mirror: false };
        Part { id: PartId(id), kind, primitive, volume_m3, placement: Some(placement) }
    }

    fn alone(primitive: Primitive, volume_m3: f64) -> Form {
        Form { parts: vec![Part::mind(PartId(0), B.min_part_m3), hang(1, Kind::Storage, primitive, volume_m3, 0, Mount::Enclosing)] }
    }

    const HULL: Primitive = Primitive::Ellipsoid { axes: DVec3::new(5.0, 3.0, 1.0) };
    const HULL_M3: f64 = 2.4e6;

    fn semi_axes(grid: &FormGrid) -> DVec3 {
        match grid.sdf.pieces()[1].shape {
            crate::form::primitive::Shape::Ellipsoid { semi_axes } => semi_axes,
            _ => unreachable!(),
        }
    }

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

    /// 20's `A(ŝ)`, and the perimeter of the same shadow by Ramanujan, from the ellipse the
    /// ellipsoid projects to.
    fn analytic(r: DVec3, s: DVec3) -> (f64, f64) {
        let area = PI * ((r.y * r.z * s.x).powi(2) + (r.x * r.z * s.y).powi(2) + (r.x * r.y * s.z).powi(2)).sqrt();
        let (u, v) = across(s);
        let m = DMat3::from_diagonal(r * r);
        let (a, b, c) = (u.dot(m * u), u.dot(m * v), v.dot(m * v));
        let mean = (a + c) / 2.0;
        let spread = (((a - c) / 2.0).powi(2) + b * b).sqrt();
        let (p, q) = ((mean + spread).sqrt(), (mean - spread).max(0.0).sqrt());
        let t = ((p - q) / (p + q)).powi(2);
        (area, PI * (p + q) * (1.0 + 3.0 * t / (10.0 + (4.0 - 3.0 * t).sqrt())))
    }

    /// Over many directions, between the table's vertices as well as on them. A cell's worth of
    /// rim is the grid's resolution; the voxels' staircase stands out by about half that.
    #[test]
    fn one_ellipsoid_reproduces_the_analytic_shadow() {
        let grid = FormGrid::new(&alone(HULL, HULL_M3), &B).unwrap();
        let r = semi_axes(&grid);
        let mut worst: f64 = 0.0;
        for s in directions(500) {
            let (area, rim) = analytic(r, s);
            let got = grid.shadow().along(s);
            let cells = (got - area) / (rim * grid.cell_m());
            worst = worst.max(cells.abs());
            assert!(cells.abs() < 1.0, "{s}: {got:.4e} against {area:.4e}, {cells:.2} cells of rim");
        }
        eprintln!("worst {worst:.3} cells of rim");
        let broadside = PI * r.x * r.y;
        assert!(((grid.broadside_m2() - broadside) / broadside).abs() < 0.02, "{} against {broadside}", grid.broadside_m2());
        assert!(grid.broadside().dot(DVec3::Z) > 0.999, "{}", grid.broadside());
        assert!(grid.roll_rad().abs() < 0.03, "{}", grid.roll_rad());
    }

    /// Against `solar` itself, at its own proportions, so the two cannot drift apart.
    #[test]
    fn one_ellipsoid_casts_what_solar_says() {
        use crate::craft::{BEAM_PER_LENGTH, HEIGHT_PER_LENGTH};
        let hull = Primitive::Ellipsoid { axes: DVec3::new(1.0, BEAM_PER_LENGTH, HEIGHT_PER_LENGTH) };
        let grid = FormGrid::new(&alone(hull, HULL_M3), &B).unwrap();
        let r = semi_axes(&grid);
        for s in directions(200) {
            let want = crate::solar::silhouette_m2(2.0 * r.x, s);
            let (_, rim) = analytic(r, s);
            let cells = (grid.shadow().along(s) - want) / (rim * grid.cell_m());
            assert!(cells.abs() < 1.0, "{s}: {cells:.2} cells of rim");
        }
    }

    #[test]
    fn one_part_keeps_most_of_the_grid() {
        let grid = FormGrid::new(&alone(HULL, HULL_M3), &B).unwrap();
        let span = 2.0 * semi_axes(&grid).x / grid.cell_m();
        assert!(span > 45.0, "{span:.1} cells");
    }

    #[test]
    fn the_table_is_its_directions_and_symmetric() {
        let grid = FormGrid::new(&Form::starting(), &B).unwrap();
        let ico = icosphere();
        assert_eq!(ico.vertices.len(), SHADOW_DIRECTIONS);
        assert_eq!(ico.faces.len(), 320);
        for (v, &d) in Shadow::directions().iter().enumerate() {
            assert!((d.length() - 1.0).abs() < 1e-15);
            let at = grid.shadow().m2()[v];
            assert!((grid.shadow().along(d) - at).abs() < 1e-12 * at, "{v}");
            assert_eq!(grid.shadow().m2()[ico.antipode[v]], grid.shadow().m2()[v]);
        }
        assert!(grid.shadow().m2().iter().all(|&a| a > 0.0));
    }

    /// Three plates stacked with gaps shade one another from above and are three times one plate
    /// edge on. Summing each cell's own shadow would make them three plates from above too.
    #[test]
    fn a_stack_of_plates_shades_itself() {
        let volume = 5.0e5;
        let flat = Primitive::Slab { edges: DVec3::new(9.0, 5.0, 0.5), corner: 0.1 };
        // Hung from a face, a child's x is the face's normal and its z the parent's −x.
        let stacked = Primitive::Slab { edges: DVec3::new(0.5, 5.0, 9.0), corner: 0.1 };
        let one = alone(flat, volume);
        let mut three = one.clone();
        three.parts.push(hang(2, Kind::Storage, stacked, volume, 1, Mount::Attached { anchor: DVec3::Z, standoff: 3.0 }));
        three.parts.push(hang(3, Kind::Storage, stacked, volume, 2, Mount::Attached { anchor: DVec3::X, standoff: 3.0 }));
        let (one, three) = (FormGrid::new(&one, &B).unwrap(), FormGrid::new(&three, &B).unwrap());

        let above = three.shadow().along(DVec3::Z) / one.shadow().along(DVec3::Z);
        assert!((above - 1.0).abs() < 0.05, "from above the stack casts {above:.3} of one plate");
        let edge = three.shadow().along(DVec3::Y) / one.shadow().along(DVec3::Y);
        assert!(edge > 2.5, "edge on the stack casts {edge:.3} of one plate");
    }

    #[test]
    fn the_envelope_of_a_sphere_is_a_sphere() {
        let volume = 4.0e6;
        let grid = FormGrid::new(&alone(Primitive::Ellipsoid { axes: DVec3::ONE }, volume), &B).unwrap();
        let expected = B.envelope_margin * libm::cbrt(volume + B.min_part_m3);
        let envelope = grid.envelope();
        assert!(relative(envelope.margin_m, expected) < 1e-12);
        let r = libm::cbrt(volume * 3.0 / (4.0 * PI)) + envelope.margin_m;
        assert!(((envelope.semi_axes - r) / r).abs().max_element() < 1e-4, "{} against {r}", envelope.semi_axes);
        assert!(relative(grid.envelope_area_m2(), 4.0 * PI * r * r) < 2e-4);
        assert!(relative(grid.envelope_volume_m3(), 4.0 / 3.0 * PI * r.powi(3)) < 3e-4);
        assert!(relative(grid.extent_m(), 2.0 * r) < 1e-4);
    }

    /// An ellipsoid hull's envelope is itself, each semi-axis the margin longer.
    #[test]
    fn the_envelope_of_an_ellipsoid_is_it_grown_on_each_axis() {
        let grid = FormGrid::new(&alone(HULL, HULL_M3), &B).unwrap();
        let (r, envelope) = (semi_axes(&grid), grid.envelope());
        let want = r + envelope.margin_m;
        assert!(((envelope.semi_axes - want) / want).abs().max_element() < 1e-4, "{} against {want}", envelope.semi_axes);
        let tip = grid.envelope_at(envelope.center + DVec3::X * envelope.semi_axes.x);
        assert!(tip.abs() < 1e-9, "{tip}");
        assert_eq!(grid.extent_m(), 2.0 * envelope.semi_axes.max_element());
    }

    /// Every preset's field holds every part with the margin to spare, and on each axis nothing
    /// smaller would: shrunk along any one of them, some part pokes through.
    #[test]
    fn the_envelope_holds_every_part_and_meets_it_on_each_axis() {
        let mut all = vec![("starting", Form::starting())];
        all.extend(Builtin::ALL.map(|b| (b.name(), b.form())));
        let directions = spiral(40_000);
        for (name, form) in all {
            let grid = FormGrid::new(&form, &B).unwrap();
            let envelope = grid.envelope();
            let inner = envelope.semi_axes - envelope.margin_m;
            let pokes = |axes: DVec3| directions.iter().any(|d| grid.sdf.distance(envelope.center + *d * axes) <= 0.0);
            assert!(!pokes(inner * 1.001), "{name}: a part pokes out");
            for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
                assert!(pokes(inner * (DVec3::ONE - axis * 0.01)), "{name}: loose along {axis}");
            }
        }
    }

    fn spiral(n: usize) -> Vec<DVec3> {
        let golden = PI * (3.0 - 5f64.sqrt());
        (0..n)
            .map(|i| {
                let z = 1.0 - (2 * i + 1) as f64 / n as f64;
                let r = (1.0 - z * z).sqrt();
                let theta = golden * i as f64;
                DVec3::new(r * theta.cos(), r * theta.sin(), z)
            })
            .collect()
    }

    fn relative(got: f64, want: f64) -> f64 {
        ((got - want) / want).abs()
    }

    /// A solid ellipsoid's moments are `m (b² + c²)/5` and round; the cells get them to a few
    /// parts in a thousand.
    #[test]
    fn an_ellipsoid_turns_as_one() {
        let grid = FormGrid::new(&alone(HULL, HULL_M3), &B).unwrap();
        let r = semi_axes(&grid);
        let mass = 3.0e9;
        let t = grid.inertia().tensor_kg_m2(mass);
        let want = DVec3::new(r.y * r.y + r.z * r.z, r.x * r.x + r.z * r.z, r.x * r.x + r.y * r.y) * (mass / 5.0);
        let got = DVec3::new(t.x_axis.x, t.y_axis.y, t.z_axis.z);
        assert!(((got - want) / want).abs().max_element() < 0.02, "{got} against {want}");
        for off in [t.y_axis.x, t.z_axis.x, t.z_axis.y] {
            assert!(off.abs() < 1e-3 * want.min_element(), "{off}");
        }
        assert!(grid.inertia().center_of_mass.length() < 0.01 * grid.cell_m());
    }

    /// Storage and a bay the same size end to end: the center of mass sits toward the storage by
    /// the ratio of their densities.
    #[test]
    fn denser_parts_weigh_more() {
        let pod = Primitive::Capsule { length: 2.0 };
        let mut form = alone(pod, 1.0e5);
        form.parts.push(hang(2, Kind::Bay, pod, 1.0e5, 1, Mount::Attached { anchor: DVec3::X, standoff: 0.0 }));
        let grid = FormGrid::new(&form, &B).unwrap();
        let storage = grid.sdf.pieces().iter().find(|p| p.part == PartId(1)).unwrap().pose.position;
        let bay = grid.sdf.pieces().iter().find(|p| p.part == PartId(2)).unwrap().pose.position;
        let f = B.bay_mass_fraction;
        let want = (storage + bay * f) / (1.0 + f);
        let got = grid.inertia().center_of_mass;
        assert!((got - want).length() < 0.02 * (bay - storage).length(), "{got} against {want}");
    }

    /// Nothing is symmetric top to bottom, so the xz product is not zero; port to starboard is.
    #[test]
    fn the_starting_form_has_an_xz_product() {
        let grid = FormGrid::new(&Form::starting(), &B).unwrap();
        let t = grid.inertia().tensor_kg_m2(1.0e9);
        assert!(t.z_axis.x.abs() > 1e-3 * t.x_axis.x, "{t}");
        assert!(t.y_axis.x.abs() < 1e-9 * t.x_axis.x && t.z_axis.y.abs() < 1e-9 * t.x_axis.x, "{t}");
        assert_eq!(t, t.transpose());
    }

    /// A hull twisted about its nose presents its flat side at that roll.
    #[test]
    fn the_roll_presents_the_broadside() {
        let mut form = alone(HULL, HULL_M3);
        form.parts[1].placement.as_mut().unwrap().twist = 0.4;
        let grid = FormGrid::new(&form, &B).unwrap();
        let z = grid.sdf.pieces()[1].pose.rotation.z_axis;
        let want = libm::atan2(-z.y, z.z);
        assert!((grid.roll_rad() - want).abs() < 0.02, "{} against {want}", grid.roll_rad());
        assert!(grid.broadside().dot(z).abs() > 0.999, "{} against {z}", grid.broadside());
    }

    #[test]
    fn every_preset_fills_its_grid() {
        let mut all = vec![("starting", Form::starting())];
        all.extend(Builtin::ALL.map(|b| (b.name(), b.form())));
        for (name, form) in all {
            let grid = FormGrid::new(&form, &B).unwrap();
            assert_eq!(grid.dims().into_iter().max(), Some(FORM_GRID), "{name}");
            assert!(grid.envelope_area_m2() > 0.0 && grid.extent_m() > 0.0, "{name}");
            let (lo, hi) = grid.sdf.bounds();
            eprintln!(
                "{name}: hull spans {:.1} of {:?} cells of {:.2} m, extent {:.1} m, envelope {:.3e} m² {:.3e} m³, broadside {:.3e} m² along {:.3} roll {:.3}",
                (hi - lo).max_element() / grid.cell_m(),
                grid.dims(),
                grid.cell_m(),
                grid.extent_m(),
                grid.envelope_area_m2(),
                grid.envelope_volume_m3(),
                grid.broadside_m2(),
                grid.broadside(),
                grid.roll_rad()
            );
        }
    }

    #[test]
    fn two_builds_agree_to_the_bit() {
        let form = Builtin::Cluster.form();
        let (a, b) = (FormGrid::new(&form, &B).unwrap(), FormGrid::new(&form, &B).unwrap());
        assert_eq!(a.geometry(1e9), b.geometry(1e9));
        assert_eq!(a.envelope(), b.envelope());
    }
}
