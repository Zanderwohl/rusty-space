//! Least squares over a fit's elements, and the covariance its error bars come from.
//!
//! Levenberg-Marquardt on a finite-difference Jacobian of [`arc::terms`]: two components of each
//! bearing's miss and a standardized range per ranged look. Every term is already in sigmas, so
//! the weights are one and `(J^T J)^-1` is the covariance outright. See
//! `lightcone/docs/25-system-knowledge.md`.
//!
//! **Eccentricity and periapsis are moved as `h = e cos w, k = e sin w`**, and the epoch as the
//! mean longitude `w + M` at the looks' weighted center. At small eccentricity the periapsis is
//! barely defined and the epoch trades against it; in these coordinates that valley is a
//! straight line, a circle is an ordinary point, and the phase a reader wants is one parameter
//! rather than a difference of two.

use std::f64::consts::{PI, TAU};

use glam::DQuat;

use super::arc::{self, Fitted, Look, basis};

/// Log axis, `h`, `k`, the pole's tilt along each vector of [`basis`], mean longitude at the
/// pivot, log period. Seven, because the period stands in for the primary's mass.
const PARAMETERS: usize = 7;
const AXIS: usize = 0;
const H: usize = 1;
const K: usize = 2;
const TILT_U: usize = 3;
const TILT_V: usize = 4;
const LONGITUDE: usize = 5;
const PERIOD: usize = 6;

/// What each parameter is stepped by for its derivative. Small enough to be linear, large
/// enough that the positions it moves are not rounding: at a moon seen from 5 AU a part in a
/// million of its orbit is still a few hundred meters.
const STEP: f64 = 1.0e-6;

/// The most eccentric a step may make an orbit, as `arc::conic` allows.
const MOST_ECCENTRIC: f64 = 0.95;

/// Damping tries per round before the round is given up as converged.
const TRIES: usize = 12;

/// A round that improves chi-square by less than this fraction ends the settle.
const CONVERGED: f64 = 1.0e-10;

/// Singular values below this fraction of the largest are rounding, and are floored there so a
/// direction nothing constrains reads as enormous rather than infinite or NaN.
const CONDITION: f64 = 1.0e-13;

/// The closest to parabolic an error bar may reach.
///
/// Not one: at one the semi-latus rectum is finite and the axis is not, so every element the
/// fit reports goes with it.
pub(super) const PARABOLIC: f64 = 0.999;

/// A fit and the time its mean longitude is measured at, which the parameters are offsets from.
#[derive(Clone, Copy)]
struct About {
    fitted: Fitted,
    pivot_s: f64,
}

impl About {
    /// The orbit `step` away. `None` past [`MOST_ECCENTRIC`].
    fn moved(&self, step: &[f64; PARAMETERS]) -> Option<Fitted> {
        let f = &self.fitted;
        let (u, v) = basis(f.pole);
        let pole = (f.pole + u * step[TILT_U] + v * step[TILT_V]).normalize();
        // In-plane angles are measured from `basis`, which a tilted pole replaces with a basis
        // of its own. Carrying the old one with the tilt keeps an angle naming the same
        // direction, so tilting the plane does not also spin the body round it.
        let carried = DQuat::from_rotation_arc(f.pole, pole) * u;
        let (u2, v2) = basis(pole);
        let turned = carried.dot(v2).atan2(carried.dot(u2));

        let n = TAU / f.period_s;
        let longitude = f.periapsis_rad + n * (self.pivot_s - f.epoch_s) + step[LONGITUDE] + turned;
        let (eccentricity, periapsis_rad) = if f.assumed_circular {
            (0.0, 0.0)
        } else {
            let h = f.eccentricity * f.periapsis_rad.cos() + step[H];
            let k = f.eccentricity * f.periapsis_rad.sin() + step[K];
            let e = h.hypot(k);
            (e, if e > 0.0 { k.atan2(h) } else { f.periapsis_rad } + turned)
        };
        if !(eccentricity < MOST_ECCENTRIC) {
            return None;
        }
        let semi_major_m = f.semi_major_m * step[AXIS].exp();
        let period_s = f.period_s * step[PERIOD].exp();
        let n = TAU / period_s;
        let mean = em_foundations::kepler::anomaly::wrap_pi(longitude - periapsis_rad);
        Some(Fitted {
            semi_major_m,
            eccentricity,
            period_s,
            pole,
            periapsis_rad,
            epoch_s: self.pivot_s - mean / n,
            mu: n * n * semi_major_m * semi_major_m * semi_major_m,
            ..*f
        })
    }

    /// Which parameters move: a circle assumed for want of arc has no `h` or `k`.
    fn free(&self) -> Vec<usize> {
        (0..PARAMETERS).filter(|&j| !(self.fitted.assumed_circular && (j == H || j == K))).collect()
    }

    /// The period's step, shrunk by the turns the arc spans so the phase it moves at the ends
    /// stays as small as every other parameter's.
    fn step(&self, j: usize, looks: &[Look]) -> f64 {
        if j != PERIOD {
            return STEP;
        }
        let (first, last) =
            looks.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), l| (lo.min(l.at_s), hi.max(l.at_s)));
        let turns = ((last - first) / self.fitted.period_s).abs();
        STEP / (1.0 + TAU * if turns.is_finite() { turns } else { 0.0 })
    }

    /// How far along its path each parameter moves the body at the pivot, in radians of mean
    /// anomaly: the phase a reader draws, which is the mean longitude's and also part of the
    /// eccentricity's and the periapsis's. `None` where the body has no pace to move along.
    fn along(&self) -> Option<[f64; PARAMETERS]> {
        let f = &self.fitted;
        let dt = f.period_s * STEP;
        let pace = (f.at(self.pivot_s + dt) - f.at(self.pivot_s - dt)) / (2.0 * dt) * (f.period_s / TAU);
        if !arc::sound(pace.length_squared()) {
            return None;
        }
        let mut out = [0.0; PARAMETERS];
        for j in self.free() {
            let place = |sign: f64| {
                let mut step = [0.0; PARAMETERS];
                step[j] = sign * STEP;
                Some(self.moved(&step)?.at(self.pivot_s))
            };
            let (Some(plus), Some(minus)) = (place(1.0), place(-1.0)) else { continue };
            out[j] = (plus - minus).dot(pace) / (2.0 * STEP * pace.length_squared());
        }
        Some(out)
    }

    /// `d terms / d parameter` for each free parameter, one column each. Central where both
    /// sides are orbits, one-sided where one is not, and zero where neither is.
    fn jacobian(&self, free: &[usize], looks: &[Look], here: &[f64]) -> Vec<Vec<f64>> {
        free.iter()
            .map(|&j| {
                let h = self.step(j, looks);
                let side = |sign: f64| {
                    let mut step = [0.0; PARAMETERS];
                    step[j] = sign * h;
                    arc::terms(&self.moved(&step)?, looks)
                };
                match (side(1.0), side(-1.0)) {
                    (Some(p), Some(m)) => p.iter().zip(&m).map(|(p, m)| (p - m) / (2.0 * h)).collect(),
                    (Some(p), None) => p.iter().zip(here).map(|(p, c)| (p - c) / h).collect(),
                    (None, Some(m)) => here.iter().zip(&m).map(|(c, m)| (c - m) / h).collect(),
                    (None, None) => vec![0.0; here.len()],
                }
            })
            .collect()
    }
}

/// The columns of `a` rotated until they are mutually orthogonal, one-sided Jacobi, and the
/// rotations that did it.
///
/// Afterwards column `i` is `s_i u_i` and column `i` of the returned matrix is `v_i`, the SVD of
/// the original `a` with its singular values in the column norms. On the Jacobian itself
/// rather than `J^T J`, which would square a condition number that a short arc already puts
/// near 1e10.
fn orthogonalize(a: &mut [Vec<f64>]) -> Vec<Vec<f64>> {
    const SWEEPS: usize = 60;
    let p = a.len();
    let mut v: Vec<Vec<f64>> = (0..p).map(|i| (0..p).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    let rotate = |x: &mut [Vec<f64>], i: usize, j: usize, c: f64, s: f64| {
        let (left, right) = x.split_at_mut(j);
        let (Some(xi), Some(xj)) = (left.get_mut(i), right.first_mut()) else { return };
        for (a, b) in xi.iter_mut().zip(xj.iter_mut()) {
            let (ai, bj) = (*a, *b);
            *a = c * ai - s * bj;
            *b = s * ai + c * bj;
        }
    };
    for _ in 0..SWEEPS {
        let mut rotated = false;
        for j in 1..p {
            for i in 0..j {
                let dot = |x: &[f64], y: &[f64]| x.iter().zip(y).map(|(x, y)| x * y).sum::<f64>();
                let (alpha, beta, gamma) = (dot(&a[i], &a[i]), dot(&a[j], &a[j]), dot(&a[i], &a[j]));
                if gamma.abs() <= f64::EPSILON * (alpha * beta).sqrt() {
                    continue;
                }
                rotated = true;
                let zeta = (beta - alpha) / (2.0 * gamma);
                let t = zeta.signum() / (zeta.abs() + (1.0 + zeta * zeta).sqrt());
                let c = 1.0 / (1.0 + t * t).sqrt();
                rotate(a, i, j, c, c * t);
                rotate(&mut v, i, j, c, c * t);
            }
        }
        if !rotated {
            break;
        }
    }
    v
}

fn squared(terms: &[f64]) -> f64 {
    terms.iter().map(|t| t * t).sum()
}

/// The Jacobian at a fit, column-scaled and decomposed: what both a step and a covariance are
/// read from.
struct Linear {
    free: Vec<usize>,
    /// Each column's norm before decomposing, which the answer is divided back out by.
    scales: Vec<f64>,
    /// `s_i^2`, floored at [`CONDITION`].
    singular2: Vec<f64>,
    /// `s_i u_i . terms`.
    projected: Vec<f64>,
    /// Right singular vectors, one per column.
    v: Vec<Vec<f64>>,
}

impl Linear {
    fn at(about: &About, looks: &[Look], here: &[f64]) -> Self {
        let free = about.free();
        let mut columns = about.jacobian(&free, looks, here);
        let scales: Vec<f64> = columns
            .iter_mut()
            .map(|column| {
                let norm = squared(column).sqrt();
                let scale = if arc::sound(norm) { norm } else { 1.0 };
                column.iter_mut().for_each(|x| *x /= scale);
                scale
            })
            .collect();
        let v = orthogonalize(&mut columns);
        let largest = columns.iter().map(|c| squared(c)).fold(0.0, f64::max);
        let singular2 = columns.iter().map(|c| squared(c).max(largest * CONDITION * CONDITION)).collect();
        let projected = columns.iter().map(|c| c.iter().zip(here).map(|(a, b)| a * b).sum()).collect();
        Self { free, scales, singular2, projected, v }
    }

    /// The damped Gauss-Newton step, in parameters: `-(J^T J + d)^-1 J^T r` in the scaled
    /// columns, where `d` is `damping` times the largest singular value squared.
    fn step(&self, damping: f64) -> [f64; PARAMETERS] {
        let largest = self.singular2.iter().copied().fold(0.0, f64::max);
        let mut out = [0.0; PARAMETERS];
        for (row, (&j, scale)) in self.free.iter().zip(&self.scales).enumerate() {
            let scaled: f64 = self
                .v
                .iter()
                .zip(self.singular2.iter().zip(&self.projected))
                .map(|(v, (s2, p))| v[row] * p / (s2 + damping * largest))
                .sum();
            out[j] = -scaled / scale;
        }
        out
    }

    /// `(J^T J)^-1`, over every parameter; zero in the rows and columns of those held.
    fn covariance(&self) -> [[f64; PARAMETERS]; PARAMETERS] {
        let mut out = [[0.0; PARAMETERS]; PARAMETERS];
        for (a, (&i, si)) in self.free.iter().zip(&self.scales).enumerate() {
            for (b, (&j, sj)) in self.free.iter().zip(&self.scales).enumerate() {
                let scaled: f64 = self.v.iter().zip(&self.singular2).map(|(v, s2)| v[a] * v[b] / s2).sum();
                out[i][j] = scaled / (si * sj);
            }
        }
        out
    }
}

/// Least squares over every element the fit has free, from `held` as its starting guess, for
/// at most `rounds` Gauss-Newton rounds.
///
/// Levenberg-Marquardt rather than plain Gauss-Newton because a three-point start can be far
/// enough out that the linearization overshoots; the damping falls away as it converges, and
/// what is left is Gauss-Newton's quadratic finish. The pattern search this replaced stalled
/// hundreds of times the noise over a fifth of an orbit, and a million times over a whole one.
pub(super) fn settle(held: Fitted, looks: &[Look], rounds: usize) -> Fitted {
    let Some(pivot_s) = arc::pivot(looks) else { return held };
    let Some(mut here) = arc::terms(&held, looks) else { return held };
    let mut about = About { fitted: held, pivot_s };
    let mut chi2 = squared(&here);
    let mut damping = 1.0e-3;
    for _ in 0..rounds {
        let linear = Linear::at(&about, looks, &here);
        let mut improved = None;
        for _ in 0..TRIES {
            let tried = about.moved(&linear.step(damping)).and_then(|f| Some((f, arc::terms(&f, looks)?)));
            if let Some((fitted, terms)) = tried.filter(|(_, t)| squared(t) < chi2) {
                improved = Some((fitted, terms));
                damping *= 0.1;
                break;
            }
            damping *= 10.0;
        }
        let Some((fitted, terms)) = improved else { break };
        let before = chi2;
        (about.fitted, here, chi2) = (fitted, terms.clone(), squared(&terms));
        if before - chi2 <= CONVERGED * before {
            break;
        }
    }
    Fitted { residual_rad: (chi2 / arc::total(looks)).sqrt(), ..about.fitted }
}

/// One sigma on each element of a fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub period_s: f64,
    pub semi_major_m: f64,
    pub eccentricity: f64,
    /// Radians the plane's pole can move, the worse of its two directions.
    pub pole_rad: f64,
    /// Seconds: the phase at the pivot, as a time. Where the body is along its path, from
    /// every element that moves it there, and not the epoch's error alone: at small
    /// eccentricity the periapsis is barely defined and the epoch trades against it.
    pub epoch_s: f64,
    /// Correlation of that phase with the period, -1 to 1.
    pub phase_period_rho: f64,
}

impl Spread {
    fn unmeasured(fitted: &Fitted) -> Self {
        Self {
            period_s: f64::INFINITY,
            semi_major_m: f64::INFINITY,
            eccentricity: (PARABOLIC - fitted.eccentricity).max(0.0),
            pole_rad: PI,
            epoch_s: fitted.period_s * 0.5,
            phase_period_rho: 0.0,
        }
    }
}

/// Each element's error bar, from the covariance at the fit.
///
/// Marginal, which is the one to report: the period and the axis trade against each other,
/// and an element's bar with the others held comes out eighty times too small. Scaled by the
/// reduced chi-square where the fit misses by more than the errors allow, because a two-body
/// orbit is not the whole of a body's motion: Earth's center swings 4700 km about the
/// Earth-Moon barycenter.
///
/// A linearization, so it holds where the fit's own error is small against the orbit. Where it
/// is not, the bars are capped where an element stops meaning anything rather than where the
/// data stops constraining it: an eccentricity at parabolic, a pole half a turn from any other,
/// and a phase of half a turn, which is anywhere.
pub fn spread(fitted: &Fitted, looks: &[Look]) -> Spread {
    let (Some(pivot_s), Some(here)) = (arc::pivot(looks), arc::terms(fitted, looks)) else {
        return Spread::unmeasured(fitted);
    };
    // A circle assumed for want of arc is fitted with no eccentricity, but its bars must not be
    // taken with none: the eccentricity is unknown, not zero, and held at zero the rest come out
    // as if it were known. Jupiter's circle over two weeks stated its axis to 0.0009 AU when it
    // was 0.24 AU out.
    let about = About { fitted: Fitted { assumed_circular: false, ..*fitted }, pivot_s };
    let linear = Linear::at(&about, looks, &here);
    let freedom = here.len().saturating_sub(linear.free.len()).max(1) as f64;
    let inflate = (squared(&here) / freedom).max(1.0);
    let c = linear.covariance().map(|row| row.map(|x| x * inflate));
    let sigma = |x: f64| if x.is_finite() && x >= 0.0 { x.sqrt() } else { f64::INFINITY };

    let eccentricity = if fitted.assumed_circular {
        0.0
    } else {
        let (cos, sin) = (fitted.periapsis_rad.cos(), fitted.periapsis_rad.sin());
        sigma(cos * cos * c[H][H] + 2.0 * cos * sin * c[H][K] + sin * sin * c[K][K])
    };
    // A pole's error is two-dimensional, and an arc pins the two directions differently: one
    // seen edge-on fixes the plane's tilt and says almost nothing about its twist.
    let (uu, uv, vv) = (c[TILT_U][TILT_U], c[TILT_U][TILT_V], c[TILT_V][TILT_V]);
    let worse = 0.5 * (uu + vv) + (0.25 * (uu - vv) * (uu - vv) + uv * uv).sqrt();
    let g = about.along().unwrap_or_else(|| {
        let mut only = [0.0; PARAMETERS];
        only[LONGITUDE] = 1.0;
        only
    });
    let along: f64 = (0..PARAMETERS).map(|i| (0..PARAMETERS).map(|j| g[i] * c[i][j] * g[j]).sum::<f64>()).sum();
    let (phase, period) = (sigma(along), sigma(c[PERIOD][PERIOD]));
    // Only the mean longitude's covariance with the period drifts. What the eccentricity adds
    // to the phase is periodic, and carried forward as a drift it made the bars worse than
    // leaving the correlation out: Mars over one and a half orbits, a span before its pivot,
    // went from 1.1 to 1.7 of its bar out.
    let rho = c[LONGITUDE][PERIOD] / (phase * period);
    Spread {
        period_s: period * fitted.period_s,
        semi_major_m: sigma(c[AXIS][AXIS]) * fitted.semi_major_m,
        eccentricity: eccentricity.min((PARABOLIC - fitted.eccentricity).max(0.0)),
        pole_rad: sigma(worse).min(PI),
        epoch_s: (phase / TAU).min(0.5) * fitted.period_s,
        phase_period_rho: if rho.is_finite() { rho.clamp(-1.0, 1.0) } else { 0.0 },
    }
}

/// A circle assumed only because the three anchors could not shape a conic, released where the
/// whole arc can.
///
/// Anchors a whole orbit apart are one point, so a long arc can fall back to a circle however
/// much it bends. Held circular it cannot fit an eccentric orbit, and the pattern search that
/// once tried releasing it stalled. From the circle, with `h` and `k` free, Gauss-Newton moves
/// straight to the eccentricity. Kept only where the arc pins the eccentricity to
/// [`SHAPED`]: a short arc bends too little to say, which is why the circle was assumed.
pub(super) fn released(circle: &Fitted, looks: &[Look], rounds: usize) -> Option<Fitted> {
    let settled = settle(Fitted { assumed_circular: false, ..*circle }, looks, rounds);
    (settled.residual_rad <= circle.residual_rad && spread(&settled, looks).eccentricity < SHAPED)
        .then_some(settled)
}

/// The eccentricity's bar below which an arc shapes a conic.
const SHAPED: f64 = 0.02;

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    /// The decomposition has to reproduce the matrix it was handed, and orthogonal columns.
    #[test]
    fn orthogonalizing_is_an_svd() {
        let a: Vec<Vec<f64>> =
            vec![vec![1.0, 2.0, 3.0, 4.0], vec![2.0, 0.5, -1.0, 0.0], vec![1.0, 2.0, 3.0, 4.000_001]];
        let mut b = a.clone();
        let v = orthogonalize(&mut b);
        for i in 0..3 {
            for j in 0..i {
                let dot: f64 = b[i].iter().zip(&b[j]).map(|(x, y)| x * y).sum();
                assert!(dot.abs() < 1.0e-12, "columns {i} and {j} are not orthogonal: {dot}");
            }
        }
        // `a = b v^T`.
        for (col, original) in a.iter().enumerate() {
            for row in 0..4 {
                let rebuilt: f64 = (0..3).map(|i| b[i][row] * v[i][col]).sum();
                assert!((rebuilt - original[row]).abs() < 1.0e-12, "{rebuilt} against {}", original[row]);
            }
        }
    }

    #[test]
    fn a_nudge_of_nothing_moves_nothing() {
        let fitted = Fitted {
            semi_major_m: 2.0e11,
            eccentricity: 0.1,
            period_s: 5.0e7,
            pole: DVec3::new(0.1, -0.2, 1.0).normalize(),
            periapsis_rad: 2.0,
            epoch_s: -3.0e6,
            mu: 0.0,
            reach_m: f64::INFINITY,
            assumed_circular: false,
            residual_rad: 0.0,
            looks: 0,
        };
        let about = About { fitted, pivot_s: 4.0e6 };
        let moved = about.moved(&[0.0; PARAMETERS]).expect("an orbit");
        for t in [0.0, 1.0e7, -2.0e7] {
            assert!(moved.at(t).distance(fitted.at(t)) < 1.0e-3, "{} m apart at {t}", moved.at(t).distance(fitted.at(t)));
        }
        // And a tilt moves the plane without turning the body round in it.
        let mut step = [0.0; PARAMETERS];
        step[TILT_U] = 1.0e-4;
        let tilted = about.moved(&step).expect("an orbit");
        let (was, now) = (fitted.at(4.0e6), tilted.at(4.0e6));
        let along = (now - was).dot(fitted.pole.cross(was).normalize());
        assert!(along.abs() < 1.0e-6 * was.length(), "a tilt moved the body {along} m along its path");
    }
}
