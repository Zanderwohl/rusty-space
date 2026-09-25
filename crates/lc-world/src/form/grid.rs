//! The form sampled at the centers of a fixed grid, and what the server reads off it: the shadow in
//! every direction, broadside, the envelope, the inertia tensor and the extent. See 29 §What the
//! server computes from a form.
//!
//! A cell is filled where [`Sdf::distance`] is not positive. The envelope is sampled at the same
//! centers from [`Sdf::estimate_each_with`], because an offset of the bound stretches along an
//! ellipsoid's long axes by its axis ratio.
//!
//! Every loop runs in a fixed order over IEEE arithmetic, `sqrt` and `libm`, so the server's
//! `Fitted` and a client's preview agree to the bit.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use glam::{DMat3, DVec2, DVec3};

use super::capacity::mass_fraction;
use super::sdf::{smooth_min, Sdf};
use super::{Form, FormError, Mount};
use crate::fitting::Balance;

/// Cells along the longest side of the padded box, whatever the ship's size.
pub const FORM_GRID: usize = 64;

/// The envelope's blend radius over the cube root of hull volume. The quadratic blend adds at most
/// a quarter of it anywhere, which bounds how far past the offset the envelope can reach.
pub const ENVELOPE_BLEND: f64 = 0.5;

/// Vertices of a twice-subdivided icosahedron.
pub const SHADOW_DIRECTIONS: usize = 162;

/// Cells of padding beyond the envelope's reach, so the outermost samples are outside it.
const EDGE_CELLS: f64 = 2.0;

/// Over the envelope's worst case, since [`Sdf::estimate_each_with`] is not a bound off an
/// ellipsoid's axes.
const PAD_SAFETY: f64 = 1.5;

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
    /// Pieces the envelope's blend reads: all but one something encloses, which lies inside its
    /// encloser and would only swell the envelope around it.
    blended: Vec<usize>,
    /// Center of cell `[0, 0, 0]`, ship frame.
    origin: DVec3,
    cell_m: f64,
    dims: [usize; 3],
    /// [`Sdf::distance`] at each cell center, x fastest, then y, then z.
    hull: Vec<f64>,
    /// The envelope's field at each cell center, in the same order.
    envelope: Vec<f64>,
    offset_m: f64,
    blend_m: f64,
    shadow: Shadow,
    broadside: DVec3,
    broadside_m2: f64,
    roll_rad: f64,
    envelope_area_m2: f64,
    envelope_volume_m3: f64,
    extent_m: f64,
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

        // Closed forms, every copy, overlaps counted twice: the grid's own volume would move the
        // offset with the resolution, and the box has to be sized before there is a grid.
        let volume_m3: f64 = sdf.pieces().iter().map(|p| p.shape.volume()).sum();
        let root = libm::cbrt(volume_m3);
        let offset_m = balance.envelope_margin * root;
        let blend_m = ENVELOPE_BLEND * root;

        let enclosed: Vec<_> = form
            .parts
            .iter()
            .filter_map(|p| p.placement.filter(|pl| matches!(pl.mount, Mount::Enclosing)).map(|pl| pl.parent))
            .collect();
        let blended = (0..sdf.pieces().len()).filter(|&i| !enclosed.contains(&sdf.pieces()[i].part)).collect();

        let (lo, hi) = sdf.bounds();
        let pad = PAD_SAFETY * (offset_m + blend_m / 4.0);
        let size = hi - lo + 2.0 * pad;
        let cell_m = size.max_element() / (FORM_GRID as f64 - 2.0 * EDGE_CELLS);
        let dims = size.to_array().map(|s| ((s / cell_m).ceil() as usize + 2 * EDGE_CELLS as usize).min(FORM_GRID));
        let center = (lo + hi) / 2.0;
        let origin = center - DVec3::from_array(dims.map(|n| (n - 1) as f64)) * (cell_m / 2.0);

        let mut grid = FormGrid {
            sdf,
            blended,
            origin,
            cell_m,
            dims,
            hull: Vec::new(),
            envelope: Vec::new(),
            offset_m,
            blend_m,
            shadow: Shadow { m2: [0.0; SHADOW_DIRECTIONS] },
            broadside: DVec3::Z,
            broadside_m2: 0.0,
            roll_rad: 0.0,
            envelope_area_m2: 0.0,
            envelope_volume_m3: 0.0,
            extent_m: 0.0,
            inertia: Inertia { center_of_mass: center, per_kg: DMat3::ZERO, cells_kg: 0.0 },
        };
        grid.sample(balance);
        let silhouette = grid.silhouette();
        grid.fill_shadow(silhouette);
        grid.envelope_surface();
        Ok(grid)
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

    /// The envelope's field at every cell center, in [`FormGrid::hull_samples`]' order. Every
    /// sample on the grid's faces is positive, so its zero set closes inside the grid.
    pub fn envelope_samples(&self) -> &[f64] {
        &self.envelope
    }

    /// The envelope's field at any point, ship frame, meters, negative inside: the union's
    /// distance offset by [`FormGrid::envelope_offset_m`], and its two nearest parts blended over
    /// [`FormGrid::envelope_blend_m`].
    pub fn envelope_at(&self, p: DVec3) -> f64 {
        self.envelope_with(p, &mut Vec::new(), &mut Vec::new())
    }

    fn envelope_with(&self, p: DVec3, scratch: &mut Vec<f64>, each: &mut Vec<f64>) -> f64 {
        let union = self.sdf.estimate_each_with(p, scratch, each);
        let (mut first, mut second) = (f64::INFINITY, f64::INFINITY);
        for &i in &self.blended {
            let d = each[i];
            if d < first {
                (first, second) = (d, first);
            } else if d < second {
                second = d;
            }
        }
        let blended = if second.is_finite() { smooth_min(first, second, self.blend_m) } else { first };
        union.min(blended) - self.offset_m
    }

    pub fn envelope_offset_m(&self) -> f64 {
        self.offset_m
    }

    pub fn envelope_blend_m(&self) -> f64 {
        self.blend_m
    }

    pub fn envelope_area_m2(&self) -> f64 {
        self.envelope_area_m2
    }

    pub fn envelope_volume_m3(&self) -> f64 {
        self.envelope_volume_m3
    }

    /// The longest side of the envelope's box along the ship's axes: `length_m`.
    pub fn extent_m(&self) -> f64 {
        self.extent_m
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

    /// What `Fitted` states, for a ship weighing `mass_kg`.
    pub fn geometry(&self, mass_kg: f64) -> lc_proto::form::Geometry {
        let t = self.inertia.tensor_kg_m2(mass_kg);
        lc_proto::form::Geometry {
            shadow_m2: self.shadow.m2.to_vec(),
            broadside: self.broadside.to_array(),
            broadside_roll_rad: self.roll_rad,
            envelope_area_m2: self.envelope_area_m2,
            envelope_volume_m3: self.envelope_volume_m3,
            inertia_kg_m2: [t.x_axis.x, t.y_axis.y, t.z_axis.z, t.y_axis.x, t.z_axis.x, t.z_axis.y],
            extent_m: self.extent_m,
        }
    }

    /// Both fields at every center, and the moments of the filled cells.
    fn sample(&mut self, balance: &Balance) {
        let [nx, ny, nz] = self.dims;
        let n = nx * ny * nz;
        let (mut hull, mut envelope) = (Vec::with_capacity(n), Vec::with_capacity(n));
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
                    envelope.push(self.envelope_with(p, &mut scratch, &mut each));
                    if d <= 0.0 {
                        let (piece, _) = self.sdf.nearest_with(p, &mut scratch);
                        let index = self.sdf.pieces().iter().position(|q| std::ptr::eq(q, piece)).expect("its own piece");
                        let m = density[index] * cell_m3;
                        let r = p - center;
                        mass += m;
                        first += r * m;
                        second += outer(r, r) * m;
                    }
                }
            }
        }
        self.hull = hull;
        self.envelope = envelope;
        debug_assert!(self.envelope_clear_of_faces(), "the envelope reaches the grid's faces");

        if mass > 0.0 {
            let c = first / mass;
            let spread = second - outer(c, c) * mass;
            // Each cell is a cube, not a point: h²/6 about each axis.
            let own = mass * self.cell_m * self.cell_m / 6.0;
            let tensor = DMat3::from_diagonal(DVec3::splat(spread.x_axis.x + spread.y_axis.y + spread.z_axis.z + own)) - spread;
            self.inertia = Inertia { center_of_mass: center + c, per_kg: tensor * (1.0 / mass), cells_kg: mass };
        }
    }

    fn envelope_clear_of_faces(&self) -> bool {
        let [nx, ny, nz] = self.dims;
        (0..nz).all(|k| {
            (0..ny).all(|j| {
                (0..nx).all(|i| {
                    let face = i == 0 || j == 0 || k == 0 || i == nx - 1 || j == ny - 1 || k == nz - 1;
                    !face || self.envelope[self.index(i, j, k)] > 0.0
                })
            })
        })
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

    /// Marching tetrahedra over the envelope's samples, six to each cube between eight centers:
    /// the zero set's area and the volume inside it, from the field's linear interpolant. Counting
    /// cell faces would overstate a sphere's area by half, and H2 anchors the field on this area.
    fn envelope_surface(&mut self) {
        // Each is a path from corner 0 to corner 7 along the axes; bits are x, y and z.
        const TETS: [[usize; 4]; 6] = [[0, 1, 3, 7], [0, 1, 5, 7], [0, 2, 3, 7], [0, 2, 6, 7], [0, 4, 5, 7], [0, 4, 6, 7]];
        let [nx, ny, nz] = self.dims;
        let h = self.cell_m;
        let tet_volume = h * h * h / 6.0;
        let (mut area, mut volume) = (0.0, 0.0);
        let (mut lo, mut hi) = (DVec3::INFINITY, DVec3::NEG_INFINITY);
        for k in 0..nz - 1 {
            for j in 0..ny - 1 {
                for i in 0..nx - 1 {
                    let corner = |c: usize| (i + (c & 1), j + ((c >> 1) & 1), k + ((c >> 2) & 1));
                    let values: [f64; 8] = std::array::from_fn(|c| {
                        let (a, b, d) = corner(c);
                        self.envelope[self.index(a, b, d)]
                    });
                    let inside = values.iter().filter(|&&v| v < 0.0).count();
                    if inside == 0 {
                        continue;
                    }
                    if inside == 8 {
                        volume += h * h * h;
                        continue;
                    }
                    // Relative to the cube's corner 0, so the arithmetic stays at the cell's scale.
                    let base = self.cell_center(i, j, k);
                    let points: [DVec3; 8] =
                        std::array::from_fn(|c| DVec3::new((c & 1) as f64, ((c >> 1) & 1) as f64, ((c >> 2) & 1) as f64) * h);
                    for tet in TETS {
                        let (a, v) = tetrahedron(tet.map(|c| (points[c], values[c])), tet_volume, &mut |p| {
                            lo = lo.min(base + p);
                            hi = hi.max(base + p);
                        });
                        area += a;
                        volume += v;
                    }
                }
            }
        }
        self.envelope_area_m2 = area;
        self.envelope_volume_m3 = volume;
        self.extent_m = if lo.x <= hi.x { (hi - lo).max_element() } else { 0.0 };
    }
}

/// The zero set's area in one tetrahedron and the volume where the linear interpolant is negative.
/// `crossing` sees every point where the zero set meets an edge.
fn tetrahedron(corners: [(DVec3, f64); 4], whole: f64, crossing: &mut impl FnMut(DVec3)) -> (f64, f64) {
    let mut inside = [0usize; 4];
    let mut outside = [0usize; 4];
    let (mut ni, mut no) = (0, 0);
    for (c, &(_, v)) in corners.iter().enumerate() {
        if v < 0.0 {
            inside[ni] = c;
            ni += 1;
        } else {
            outside[no] = c;
            no += 1;
        }
    }
    // Inside is strictly negative and outside not, so the denominator is never zero.
    let mut cut = |a: usize, b: usize| {
        let ((pa, va), (pb, vb)) = (corners[a], corners[b]);
        let p = pa + (pb - pa) * (va / (va - vb));
        crossing(p);
        p
    };
    let tet = |a: DVec3, b: DVec3, c: DVec3, d: DVec3| (b - a).dot((c - a).cross(d - a)).abs() / 6.0;
    let triangle = |a: DVec3, b: DVec3, c: DVec3| (b - a).cross(c - a).length() / 2.0;
    match ni {
        0 => (0.0, 0.0),
        4 => (0.0, whole),
        1 | 3 => {
            let (apex, others) = if ni == 1 { (inside[0], &outside[..3]) } else { (outside[0], &inside[..3]) };
            let p = [cut_pair(&mut cut, apex, others[0], ni), cut_pair(&mut cut, apex, others[1], ni), cut_pair(&mut cut, apex, others[2], ni)];
            let corner = tet(corners[apex].0, p[0], p[1], p[2]);
            (triangle(p[0], p[1], p[2]), if ni == 1 { corner } else { whole - corner })
        }
        _ => {
            let [a, b] = [inside[0], inside[1]];
            let [c, d] = [outside[0], outside[1]];
            let (ac, ad, bc, bd) = (cut(a, c), cut(a, d), cut(b, c), cut(b, d));
            let (pa, pb) = (corners[a].0, corners[b].0);
            // A prism from face (a, ac, ad) to (b, bc, bd), in three.
            let volume = tet(pa, ac, ad, pb) + tet(ac, ad, pb, bc) + tet(ad, pb, bc, bd);
            // ac, ad, bd, bc go round the quad.
            ((bd - ac).cross(bc - ad).length() / 2.0, volume)
        }
    }
}

/// `cut` wants the inside corner first.
fn cut_pair(cut: &mut impl FnMut(usize, usize) -> DVec3, apex: usize, other: usize, inside: usize) -> DVec3 {
    if inside == 1 { cut(apex, other) } else { cut(other, apex) }
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
    use crate::form::{Kind, Part, PartId, Placement, Primitive};

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

    /// Counting cell faces would give half as much again.
    #[test]
    fn the_envelope_of_a_sphere_is_a_sphere() {
        let volume = 4.0e6;
        let grid = FormGrid::new(&alone(Primitive::Ellipsoid { axes: DVec3::ONE }, volume), &B).unwrap();
        let r = libm::cbrt(volume * 3.0 / (4.0 * PI)) + grid.envelope_offset_m();
        let (area, inside) = (4.0 * PI * r * r, 4.0 / 3.0 * PI * r.powi(3));
        let relative = |got: f64, want: f64| ((got - want) / want).abs();
        assert!(relative(grid.envelope_area_m2(), area) < 0.01, "{} against {area}", grid.envelope_area_m2());
        assert!(relative(grid.envelope_volume_m3(), inside) < 0.01, "{} against {inside}", grid.envelope_volume_m3());
        assert!((grid.extent_m() - 2.0 * r).abs() < grid.cell_m(), "{} against {}", grid.extent_m(), 2.0 * r);
        let expected = B.envelope_margin * libm::cbrt(volume + B.min_part_m3);
        assert!(relative(grid.envelope_offset_m(), expected) < 1e-12);
    }

    /// Offsetting the ellipsoid's bound would put the tips `margin · a/c` out, five times too far
    /// on this hull.
    #[test]
    fn the_envelope_is_offset_evenly_round_an_ellipsoid() {
        let grid = FormGrid::new(&alone(HULL, HULL_M3), &B).unwrap();
        let r = semi_axes(&grid);
        let want = 2.0 * (r.x + grid.envelope_offset_m());
        assert!((grid.extent_m() - want).abs() < grid.cell_m(), "{} against {want}", grid.extent_m());
        let tip = grid.envelope_at(DVec3::X * (r.x + grid.envelope_offset_m()));
        let side = grid.envelope_at(DVec3::Z * (r.z + grid.envelope_offset_m()));
        assert!(tip.abs() < 1e-9 && side.abs() < 1e-9, "{tip} {side}");
    }

    /// The Mind sits inside a plate thinner than twice its own distance to the plate's faces:
    /// blended with its encloser it would raise a blister over the plate's middle.
    #[test]
    fn an_enclosed_part_does_not_swell_the_envelope() {
        let grid = FormGrid::new(&alone(Primitive::Slab { edges: DVec3::new(9.0, 5.0, 0.5), corner: 0.1 }, HULL_M3), &B).unwrap();
        let crate::form::primitive::Shape::Slab { edges, .. } = grid.sdf.pieces()[1].shape else { unreachable!() };
        let face = DVec3::Z * (edges.z / 2.0 + grid.envelope_offset_m());
        assert!(grid.envelope_at(face).abs() < 1e-9, "{}", grid.envelope_at(face));
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
    fn every_preset_closes_inside_its_grid() {
        let mut all = vec![("starting", Form::starting())];
        all.extend(Builtin::ALL.map(|b| (b.name(), b.form())));
        for (name, form) in all {
            let grid = FormGrid::new(&form, &B).unwrap();
            assert!(grid.envelope_clear_of_faces(), "{name}");
            assert_eq!(grid.dims().into_iter().max(), Some(FORM_GRID), "{name}");
            assert!(grid.envelope_area_m2() > 0.0 && grid.extent_m() > 0.0, "{name}");
            eprintln!(
                "{name}: {:?} cells of {:.2} m, extent {:.1} m, envelope {:.3e} m² {:.3e} m³, broadside {:.3e} m² along {:.3} roll {:.3}",
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
        assert_eq!(a.envelope_samples(), b.envelope_samples());
    }
}
