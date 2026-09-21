//! The baked emission shell: what a population does to the star's output, per direction.
//!
//! Only statistically steady occluders live here. A shell vertex is a property of the
//! direction, not of the moment, so anything coherent stays on the analytic path.

use glam::DVec3;
use serde::{Deserialize, Serialize};

use em_spectra::{BANDS, Band, PerBand};

use crate::population::Population;
use crate::star::Star;

/// Bumped whenever the channel layout changes. `BANDS` is part of the layout, so going from
/// five bands to seven would have silently reinterpreted every stored shell without it.
pub const FORMAT_VERSION: u16 = 1;

/// `m`, one deficit per band, and the crossing time.
const CHANNELS: usize = 2 + BANDS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellError {
    Version { found: u16, expected: u16 },
    Bands { found: u8, expected: u8 },
    Length { found: usize, expected: usize },
}

impl std::fmt::Display for ShellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version { found, expected } => {
                write!(f, "shell format {found}, expected {expected}")
            }
            Self::Bands { found, expected } => {
                write!(f, "shell has {found} bands, expected {expected}")
            }
            Self::Length { found, expected } => {
                write!(f, "shell has {found} floats, expected {expected}")
            }
        }
    }
}
impl std::error::Error for ShellError {}

/// What a shell holds at one direction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShellSample {
    /// Cone-averaged elements on the disc. Also selects the flicker regime.
    pub mean_count: f64,
    pub deficit: PerBand<f32>,
    pub crossing_time: f64,
}

/// A geodesic sphere of baked values.
///
/// Vertices are stored as one triangular lattice per icosahedron face, **not** deduplicated
/// across shared edges. That costs 9.5% more storage at level 5 and buys O(1) lookup with no
/// hierarchy to walk. Duplicated edge vertices are bit-identical because their direction is
/// computed from the same two corners either way.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Shell {
    version: u16,
    level: u8,
    bands: u8,
    data: Vec<f32>,
}

const PHI: f64 = 1.618_033_988_749_895;

fn icosahedron() -> ([DVec3; 12], [[usize; 3]; 20]) {
    let v = |x: f64, y: f64, z: f64| DVec3::new(x, y, z).normalize();
    let verts = [
        v(-1.0, PHI, 0.0),
        v(1.0, PHI, 0.0),
        v(-1.0, -PHI, 0.0),
        v(1.0, -PHI, 0.0),
        v(0.0, -1.0, PHI),
        v(0.0, 1.0, PHI),
        v(0.0, -1.0, -PHI),
        v(0.0, 1.0, -PHI),
        v(PHI, 0.0, -1.0),
        v(PHI, 0.0, 1.0),
        v(-PHI, 0.0, -1.0),
        v(-PHI, 0.0, 1.0),
    ];
    let faces = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    (verts, faces)
}

impl Shell {
    /// Lattice steps per face edge.
    #[inline]
    fn n(level: u8) -> usize {
        1usize << level
    }

    /// Vertices per icosahedron face.
    #[inline]
    fn per_face(level: u8) -> usize {
        let n = Self::n(level);
        (n + 1) * (n + 2) / 2
    }

    pub fn level(&self) -> u8 {
        self.level
    }

    pub fn vertex_count(&self) -> usize {
        20 * Self::per_face(self.level)
    }

    /// What a deduplicated sphere of this level would hold, for comparison.
    pub fn unique_vertex_count(&self) -> usize {
        10 * 4usize.pow(self.level as u32) + 2
    }

    fn lattice_index(n: usize, i: usize, j: usize) -> usize {
        j * (n + 1) - j * (j.saturating_sub(1)) / 2 + i
    }

    fn direction(corners: [DVec3; 3], n: usize, i: usize, j: usize) -> DVec3 {
        let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
        (corners[0] * (1.0 - u - v) + corners[1] * u + corners[2] * v).normalize()
    }

    /// Evaluate the populations over the sphere and store the result.
    pub fn bake(star: &Star, populations: &[Population], level: u8) -> Self {
        let (verts, faces) = icosahedron();
        let n = Self::n(level);
        let per_face = Self::per_face(level);
        let mut data = vec![0.0f32; 20 * per_face * CHANNELS];

        for (f, face) in faces.iter().enumerate() {
            let corners = [verts[face[0]], verts[face[1]], verts[face[2]]];
            for j in 0..=n {
                for i in 0..=(n - j) {
                    let dir = Self::direction(corners, n, i, j);
                    let at = (f * per_face + Self::lattice_index(n, i, j)) * CHANNELS;

                    let mut count = 0.0;
                    let mut weighted_crossing = 0.0;
                    let mut deficit = PerBand::splat(0.0f32);
                    for p in populations {
                        let m = p.mean_count_cone(dir, star);
                        if m <= 0.0 {
                            continue;
                        }
                        let gray = m * p.single_event_depth(star);
                        for b in Band::ALL {
                            deficit[b] += (gray * p.band_response[b] as f64) as f32;
                        }
                        count += m;
                        weighted_crossing += m * p.crossing_time(star);
                    }
                    data[at] = count as f32;
                    for b in Band::ALL {
                        data[at + 1 + b.index()] = deficit[b];
                    }
                    data[at + 1 + BANDS] = if count > 0.0 {
                        (weighted_crossing / count) as f32
                    } else {
                        0.0
                    };
                }
            }
        }
        Self {
            version: FORMAT_VERSION,
            level,
            bands: BANDS as u8,
            data,
        }
    }

    pub fn check(&self) -> Result<(), ShellError> {
        if self.version != FORMAT_VERSION {
            return Err(ShellError::Version {
                found: self.version,
                expected: FORMAT_VERSION,
            });
        }
        if self.bands as usize != BANDS {
            return Err(ShellError::Bands {
                found: self.bands,
                expected: BANDS as u8,
            });
        }
        let expected = self.vertex_count() * CHANNELS;
        if self.data.len() != expected {
            return Err(ShellError::Length {
                found: self.data.len(),
                expected,
            });
        }
        Ok(())
    }

    fn read(&self, vertex: usize) -> ShellSample {
        let at = vertex * CHANNELS;
        let mut deficit = PerBand::splat(0.0f32);
        for b in Band::ALL {
            deficit[b] = self.data[at + 1 + b.index()];
        }
        ShellSample {
            mean_count: self.data[at] as f64,
            deficit,
            crossing_time: self.data[at + 1 + BANDS] as f64,
        }
    }

    /// Barycentric interpolation at `direction`.
    pub fn sample(&self, direction: DVec3) -> ShellSample {
        let d = direction.normalize();
        let (verts, faces) = icosahedron();
        let n = Self::n(self.level);
        let per_face = Self::per_face(self.level);

        for (f, face) in faces.iter().enumerate() {
            let (a, b, c) = (verts[face[0]], verts[face[1]], verts[face[2]]);
            // Barycentric weights of the ray against the planar triangle.
            let denom = a.dot(b.cross(c));
            if denom.abs() < 1e-12 {
                continue;
            }
            let wa = d.dot(b.cross(c)) / denom;
            let wb = d.dot(c.cross(a)) / denom;
            let wc = d.dot(a.cross(b)) / denom;
            if wa < -1e-9 || wb < -1e-9 || wc < -1e-9 {
                continue;
            }
            let total = wa + wb + wc;
            let (u, v) = (wb / total * n as f64, wc / total * n as f64);

            let fi = (u.floor() as usize).min(n.saturating_sub(1));
            let fj = (v.floor() as usize).min(n.saturating_sub(1));
            let (fu, fv) = (u - fi as f64, v - fj as f64);
            let base = f * per_face;
            let idx =
                |i: usize, j: usize| base + Self::lattice_index(n, i.min(n), j.min(n - i.min(n)));

            let (corners, weights) = if fu + fv <= 1.0 {
                (
                    [idx(fi, fj), idx(fi + 1, fj), idx(fi, fj + 1)],
                    [1.0 - fu - fv, fu, fv],
                )
            } else {
                (
                    [idx(fi + 1, fj), idx(fi, fj + 1), idx(fi + 1, fj + 1)],
                    [1.0 - fv, 1.0 - fu, fu + fv - 1.0],
                )
            };

            let mut out = ShellSample::default();
            for (k, &vertex) in corners.iter().enumerate() {
                let s = self.read(vertex);
                let w = weights[k];
                out.mean_count += w * s.mean_count;
                out.crossing_time += w * s.crossing_time;
                for band in Band::ALL {
                    out.deficit[band] += (w as f32) * s.deficit[band];
                }
            }
            return out;
        }
        ShellSample::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::{Distribution, Inclination};
    use crate::rng;

    const AU: f64 = 1.496e11;

    fn swarm(inc: Inclination) -> Population {
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::delta(AU),
            eccentricity: Distribution::delta(0.0),
            inclination: inc,
            count: 1.5e6,
            cross_section: 1e12,
            band_response: PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        }
    }

    #[test]
    fn vertex_counts_match_the_documented_geometry() {
        for level in 3..=6u8 {
            let s = Shell::bake(&Star::SOL, &[], level);
            assert_eq!(s.vertex_count(), 20 * Shell::per_face(level));
            // The non-deduplicated overhead should stay modest and shrink with level.
            let overhead = s.vertex_count() as f64 / s.unique_vertex_count() as f64 - 1.0;
            assert!(overhead < 0.45, "level {level} overhead {overhead}");
            if level == 5 {
                assert!(
                    (overhead - 0.095).abs() < 0.005,
                    "level 5 overhead is {overhead}"
                );
            }
        }
    }

    #[test]
    fn an_isotropic_swarm_bakes_flat() {
        let pop = swarm(Inclination::isotropic());
        let shell = Shell::bake(&Star::SOL, std::slice::from_ref(&pop), 3);
        let want = pop.mean_deficit(DVec3::X, &Star::SOL) as f32;
        for k in 0..400u64 {
            let d = DVec3::new(
                rng::gaussian(rng::hash(&[k, 1])),
                rng::gaussian(rng::hash(&[k, 2])),
                rng::gaussian(rng::hash(&[k, 3])),
            );
            if d.length() < 1e-6 {
                continue;
            }
            let got = shell.sample(d).deficit[Band::V];
            assert!((got / want - 1.0).abs() < 1e-4, "{d}: {got} vs {want}");
        }
    }

    #[test]
    fn sampling_at_a_vertex_returns_that_vertex() {
        let pop = swarm(Inclination::uniform_angle(0.0, 0.4, 16));
        let shell = Shell::bake(&Star::SOL, std::slice::from_ref(&pop), 4);
        let (verts, faces) = icosahedron();
        let n = Shell::n(4);
        for face in faces.iter().take(4) {
            let corners = [verts[face[0]], verts[face[1]], verts[face[2]]];
            for (i, j) in [(0, 0), (n, 0), (0, n), (n / 2, n / 4)] {
                let dir = Shell::direction(corners, n, i, j);
                let direct = pop.mean_deficit(dir, &Star::SOL) as f32;
                let got = shell.sample(dir).deficit[Band::V];
                assert!(
                    (got - direct).abs() <= direct.abs() * 1e-5,
                    "{got} vs {direct}"
                );
            }
        }
    }

    #[test]
    fn interpolation_is_continuous_across_face_edges() {
        let pop = swarm(Inclination::uniform_angle(0.0, 0.5, 16));
        let shell = Shell::bake(&Star::SOL, std::slice::from_ref(&pop), 4);
        let (verts, faces) = icosahedron();
        // Scale of the field, so the tolerance is not relative to a value that may be zero:
        // a shared edge can run through the population's inclination limit.
        let peak = pop.mean_deficit(DVec3::X, &Star::SOL) as f32;
        assert!(peak > 0.0);

        // Step either side of a shared edge, close enough that the field's own gradient
        // across the step is negligible and any difference left is a seam.
        let c = [verts[faces[0][0]], verts[faces[0][1]], verts[faces[0][2]]];
        for k in 1..20 {
            let t = k as f64 / 20.0;
            let on_edge = (c[0] * (1.0 - t) + c[1] * t).normalize();
            let inward = (on_edge + (c[2] - on_edge) * 1e-7).normalize();
            let outward = (on_edge - (c[2] - on_edge) * 1e-7).normalize();
            let (a, b) = (
                shell.sample(inward).deficit[Band::V],
                shell.sample(outward).deficit[Band::V],
            );
            assert!(
                (a - b).abs() < peak * 1e-5,
                "seam at t={t}: {a} vs {b}, peak {peak}"
            );
        }
    }

    #[test]
    fn a_band_bakes_dark_over_the_pole() {
        let pop = swarm(Inclination::uniform_angle(0.0, 0.2, 16));
        let shell = Shell::bake(&Star::SOL, std::slice::from_ref(&pop), 4);
        assert!(shell.sample(DVec3::X).deficit[Band::V] > 0.0);
        assert_eq!(shell.sample(DVec3::Z).deficit[Band::V], 0.0);
    }

    #[test]
    fn band_response_reddens_a_dust_population() {
        let mut dust = swarm(Inclination::isotropic());
        for b in Band::ALL {
            dust.band_response[b] = em_spectra::extinction::RATIO[b] as f32;
        }
        let shell = Shell::bake(&Star::SOL, std::slice::from_ref(&dust), 3);
        let s = shell.sample(DVec3::X);
        assert!(s.deficit[Band::B] > s.deficit[Band::V]);
        assert!(s.deficit[Band::V] > s.deficit[Band::K]);
        assert!(
            s.deficit[Band::Radio] < s.deficit[Band::V] * 1e-6,
            "radio sees through dust"
        );
    }

    #[test]
    fn the_format_round_trips_and_rejects_a_mismatch() {
        let shell = Shell::bake(&Star::SOL, &[swarm(Inclination::isotropic())], 3);
        assert_eq!(shell.check(), Ok(()));
        let json = serde_json::to_string(&shell).unwrap();
        let back: Shell = serde_json::from_str(&json).unwrap();
        assert_eq!(back, shell);
        assert_eq!(back.check(), Ok(()));

        let mut wrong = shell.clone();
        wrong.version = FORMAT_VERSION + 1;
        assert!(matches!(wrong.check(), Err(ShellError::Version { .. })));
        let mut short = shell;
        short.data.pop();
        assert!(matches!(short.check(), Err(ShellError::Length { .. })));
    }
}
