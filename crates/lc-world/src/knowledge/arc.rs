//! An orbit from an arc of bearings, with no range in any of them.
//!
//! **What is searched, and why it is the ranges.** A bearing gives two angles and no distance,
//! so the missing quantity is a range. The obvious shortcut is to search the orbit's *plane*
//! instead and cross each ray with it -- and it does not work, because a ship inside a system
//! and the planets it is watching are all within a few degrees of one plane, so every ray lies
//! nearly *in* the candidate plane and crosses it nowhere that is not rounding error. A plane
//! tells you nothing about a body you are coplanar with. Measured before it was abandoned: with
//! the true plane in hand, every ray from a 5 AU orbit to an Earth-like one grazes it.
//!
//! So the ranges are searched, and three of them are almost enough by themselves:
//!
//! 1. Take three looks, spread across the arc. Guess the range at the first two.
//! 2. The third is **not** a guess. An orbit's plane contains the star, so the three positions
//!    and the star are coplanar, `det[r1 r2 r3] = 0`, and that determinant is *linear* in the
//!    third range. One equation, one closed-form solution.
//! 3. Those three positions give the plane outright, and in it a conic with its focus at the
//!    star is a linear solve: `1/r = A + B cos(theta) + C sin(theta)` gives the semi-latus
//!    rectum, the eccentricity and where periapsis is. No mass and no period: pure geometry.
//! 4. Turn each true anomaly into a mean anomaly, unwrap in time order, and regress the times
//!    on them. `t = epoch + M / n` is linear in `1/n`, so the mean motion and the epoch fall
//!    out of a straight line -- and with them the period, **measured** rather than assumed from
//!    a mass.
//! 5. The star's `mu` is then `n^2 a^3`, Kepler's third law read the other way. This is what
//!    the design doc means by the star's mass being refined once an orbit is held, and it is
//!    why no mass has to be supplied to get here.
//!
//! Two numbers are searched and everything after them is closed form. The score is in radians
//! against the bearings themselves: propagate the orbit to each observation time and measure
//! the angle it misses by. See `lightcone/docs/25-system-knowledge.md`.

use em_foundations::kepler;
use glam::{DMat3, DVec3};

use crate::knowledge::astrometry::Bearing;

/// Ranges tried per axis before refining, log-spaced across [`NEAR_AU`, `FAR_AU`].
///
/// Finer is both more accurate *and* faster here, which is not the usual trade: a coarser grid
/// leaves more candidates half-plausible, and those are the ones whose cost is a Kepler solve
/// per look rather than a rejected guard. Halving this to 128 lost a case and took 15% longer.
const RANGES: usize = 256;

/// The band a body of a system can be in, astronomical units. Outside it there is nothing a
/// survey from inside would be looking at.
const NEAR_AU: f64 = 0.02;
const FAR_AU: f64 = 200.0;

/// Rounds of shrinking the search about the best pair, and the factor each round shrinks by.
const REFINEMENTS: usize = 100;

/// Rounds of least-squares settling every candidate gets. The one that wins gets eight times
/// as many, because the comparison has to be even and only the answer has to be exact.
const SETTLINGS: usize = 60;

/// Bearings fewer than this cannot pin an orbit: three looks make the conic fit exact and the
/// timing line nearly so, leaving nothing over to judge a candidate by.
pub const LOOKS_NEEDED: usize = 5;

/// Three positions within this of collinear give a plane that is all rounding.
const DEGENERATE: f64 = 1.0e-6;

/// A fitted body must stay at least this much of the observer's own distance from the star away
/// from the observer.
///
/// Without it a short arc has an attractor at the *ship's own orbit*: put the body on top of
/// the observer and the range goes to zero, the parallax with it, and any orbit explains the
/// bearings. Three of the eight starts on Saturn's three-degree arc landed there, at 5.0000 AU
/// with no eccentricity, which is the 5 AU circle the ship was flying.
const NOT_ABOARD: f64 = 1.0e-2;

/// How closely two separated solutions must agree on the axis and the period to be the same
/// answer rather than two different ones.
const AGREEMENT: f64 = 0.02;

/// A *disagreeing* solution whose miss is within this factor of the best one is a rival, and an
/// arc with a rival does not determine an orbit.
///
/// **The test is rivalry, not the residual.** An earlier rule asked instead that the best fit be
/// near the bearings' own noise, and it threw away the best orbits this makes: 284 degrees of an
/// Earth-like arc fits to a=1.0001 with the runner-up five thousand times worse -- decisive by
/// any reading -- and was refused for sitting three thousand times the noise floor, which is a
/// statement about how far the search converged and not about what the arc supports. Meanwhile
/// three degrees of Saturn's arc gives 2.7, 5.0, 2.6 and 230 AU within a factor of three of each
/// other, which is exactly what having no answer looks like.
const RIVAL: f64 = 10.0;

/// Positive and finite. Written out because `!(x > 0.0)` is what rejects a NaN and `x <= 0.0`
/// is not, and the difference is a silent wrong orbit rather than a refused one.
fn sound(x: f64) -> bool {
    x.is_finite() && x > 0.0
}

/// One look at a body: where from, which way, when, and how well.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub from_m: DVec3,
    pub toward: DVec3,
    pub at_s: f64,
    pub sigma_rad: f64,
}

impl Look {
    /// From a stored sighting, with the star's position as the origin of the frame.
    pub fn of(bearing: &Bearing, at_s: f64, star_ly: DVec3) -> Self {
        Self {
            from_m: (bearing.observer_ly - star_ly) * crate::system::M_PER_LY,
            toward: bearing.toward,
            at_s,
            sigma_rad: bearing.sigma_rad.max(f64::MIN_POSITIVE),
        }
    }
}

/// An orbit as this fit determines it, in the star's frame. Meters, seconds, radians.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fitted {
    pub semi_major_m: f64,
    pub eccentricity: f64,
    pub period_s: f64,
    /// Unit normal of the orbit's plane, signed so the body goes round it counterclockwise.
    pub pole: DVec3,
    /// True anomaly's zero: where periapsis is, measured in the plane's own basis.
    pub periapsis_rad: f64,
    /// Coordinate seconds at which the body is at periapsis.
    pub epoch_s: f64,
    /// `n^2 a^3`: the star's gravitational parameter as this orbit measures it.
    pub mu: f64,
    /// Weighted root-mean-square of what the fit misses the bearings by, radians.
    pub residual_rad: f64,
    pub looks: usize,
}

impl Fitted {
    /// Where the body is at `t`, in the star's frame, meters.
    pub fn at(&self, t_s: f64) -> DVec3 {
        let (u, v) = basis(self.pole);
        let n = std::f64::consts::TAU / self.period_s;
        let mean = (t_s - self.epoch_s) * n;
        let Some(true_anomaly) = kepler::anomaly::true_from_mean(mean, self.eccentricity) else {
            return DVec3::ZERO;
        };
        let angle = self.periapsis_rad + true_anomaly;
        let p = self.semi_major_m * (1.0 - self.eccentricity * self.eccentricity);
        let r = p / (1.0 + self.eccentricity * true_anomaly.cos());
        (u * angle.cos() + v * angle.sin()) * r
    }
}

/// A right-handed basis for the plane whose normal is `pole`.
fn basis(pole: DVec3) -> (DVec3, DVec3) {
    let u = pole.any_orthonormal_vector();
    (u, pole.cross(u))
}

/// The third range that puts the three positions and the star in one plane.
///
/// `det[r1 r2 r3] = 0` expanded in the third range: `r3 = p3 + d3 * rho`, and the determinant is
/// `(r1 x r2) . p3 + rho * (r1 x r2) . d3`, linear in `rho`. `None` when the coefficient
/// vanishes -- the third ray runs along the plane the first two define, and no range on it
/// reaches that plane.
fn coplanar_range(r1: DVec3, r2: DVec3, from_m: DVec3, toward: DVec3) -> Option<f64> {
    let normal = r1.cross(r2);
    let slope = normal.dot(toward);
    if slope.abs() <= f64::MIN_POSITIVE {
        return None;
    }
    let range = -normal.dot(from_m) / slope;
    sound(range).then_some(range)
}

/// `1/r = A + B cos(theta) + C sin(theta)`: a conic with its focus at the star, and the one
/// step here that is a straight linear solve.
///
/// `points` are `(angle in the plane, radius)`. Returns the semi-latus rectum, the
/// eccentricity, and where periapsis is.
fn conic(points: &[(f64, f64)]) -> Option<(f64, f64, f64)> {
    let mut normal = DMat3::ZERO;
    let mut rhs = DVec3::ZERO;
    for (theta, r) in points {
        let row = DVec3::new(1.0, theta.cos(), theta.sin());
        normal += DMat3::from_cols(row * row.x, row * row.y, row * row.z);
        rhs += row / *r;
    }
    let scale = normal.to_cols_array().iter().fold(0.0f64, |m, e| m.max(e.abs()));
    if !sound(normal.determinant().abs() - 1.0e-12 * scale * scale * scale) {
        return None;
    }
    let solved = normal.inverse() * rhs;
    let (a, b, c) = (solved.x, solved.y, solved.z);
    if !sound(a) {
        return None;
    }
    let eccentricity = (b * b + c * c).sqrt() / a;
    // A parabola or a hyperbola is not an orbit this fit is for, and 1 is where the algebra
    // below divides by zero.
    if !sound(0.95 - eccentricity) {
        return None;
    }
    Some((1.0 / a, eccentricity, c.atan2(b)))
}

/// Regress the times on the mean anomalies: `t = epoch + M / n`, linear in `1/n`.
///
/// The anomalies are unwrapped in time order, which assumes consecutive looks are less than an
/// orbit apart. A survey revisiting a body every few game hours is nowhere near that for
/// anything with a period worth fitting.
///
/// Returns seconds per radian of mean anomaly, and the epoch. A negative rate is a prograde
/// orbit seen from the wrong side of its plane, which the caller fixes by flipping the pole.
fn timing(mean: &[(f64, f64)]) -> Option<(f64, f64)> {
    let mut unwrapped: Vec<(f64, f64)> = Vec::with_capacity(mean.len());
    let mut turns = 0.0;
    let mut last = mean.first()?.0;
    for (m, t) in mean {
        if *m + turns < last - std::f64::consts::PI {
            turns += std::f64::consts::TAU;
        }
        last = m + turns;
        unwrapped.push((last, *t));
    }
    let count = unwrapped.len() as f64;
    let mean_m = unwrapped.iter().map(|(m, _)| m).sum::<f64>() / count;
    let mean_t = unwrapped.iter().map(|(_, t)| t).sum::<f64>() / count;
    let mut var = 0.0;
    let mut cov = 0.0;
    for (m, t) in &unwrapped {
        var += (m - mean_m) * (m - mean_m);
        cov += (m - mean_m) * (t - mean_t);
    }
    if !sound(var) {
        return None;
    }
    let per_rad = cov / var;
    sound(per_rad.abs()).then_some((per_rad, mean_t - per_rad * mean_m))
}

/// The orbit through three positions at three times, scored against every look.
///
/// The positions give the plane, the plane gives the conic, the conic gives the anomalies and
/// the anomalies with the times give the period. Nothing here is searched.
fn through(places: [(DVec3, f64); 3], looks: &[Look], bound: f64) -> Option<Fitted> {
    let [(r1, t1), (r2, t2), (r3, t3)] = places;
    let mut pole = (r2 - r1).cross(r3 - r1);
    let scale = r1.length().max(r2.length()).max(r3.length());
    if !sound(pole.length() - DEGENERATE * scale * scale) {
        return None;
    }
    pole = pole.normalize();
    let (u, v) = basis(pole);
    let angle = |r: DVec3| r.dot(v).atan2(r.dot(u));
    let points = [
        (angle(r1), r1.length()),
        (angle(r2), r2.length()),
        (angle(r3), r3.length()),
    ];
    let (semi_latus_rectum, eccentricity, periapsis_rad) = conic(&points)?;
    let semi_major_m = kepler::semi_major_axis::from_eccentricity_and_semi_latus_rectum(
        eccentricity,
        semi_latus_rectum,
    );
    if !sound(semi_major_m) {
        return None;
    }

    let anomaly = |theta: f64| {
        let true_anomaly = kepler::anomaly::wrap_pi(theta - periapsis_rad);
        let e = kepler::anomaly::eccentric_from_true(true_anomaly, eccentricity);
        kepler::anomaly::mean_from_eccentric(e, eccentricity)
    };
    let (per_rad, epoch_s) = timing(&[
        (anomaly(points[0].0), t1),
        (anomaly(points[1].0), t2),
        (anomaly(points[2].0), t3),
    ])?;
    let period_s = per_rad.abs() * std::f64::consts::TAU;
    if !sound(period_s) {
        return None;
    }
    let n = std::f64::consts::TAU / period_s;

    let mut fitted = Fitted {
        semi_major_m,
        eccentricity,
        period_s,
        pole: if per_rad < 0.0 { -pole } else { pole },
        periapsis_rad: if per_rad < 0.0 { -periapsis_rad } else { periapsis_rad },
        epoch_s,
        mu: n * n * semi_major_m * semi_major_m * semi_major_m,
        residual_rad: 0.0,
        looks: looks.len(),
    };
    fitted.residual_rad = residual(&fitted, looks, bound)?;
    Some(fitted)
}

/// Weighted RMS of the angle between where the orbit says the body was and where it was seen.
///
/// `bound` is the best RMS found so far. A candidate that has already missed by more than that
/// cannot win, so the rest of its looks are not worth solving Kepler's equation for -- and most
/// candidates are hopeless within two or three of them. The bound is compared against the same
/// quantity the function returns, so the exit changes the cost and not the answer.
fn residual(fitted: &Fitted, looks: &[Look], bound: f64) -> Option<f64> {
    let total: f64 = looks.iter().map(|l| 1.0 / (l.sigma_rad * l.sigma_rad)).sum();
    if !sound(total) {
        return None;
    }
    let ceiling = bound * bound * total;
    let mut sum = 0.0;
    for look in looks {
        let offset = fitted.at(look.at_s) - look.from_m;
        if !sound(offset.length() - NOT_ABOARD * look.from_m.length()) {
            return None;
        }
        let miss = between(offset.normalize(), look.toward);
        sum += miss * miss / (look.sigma_rad * look.sigma_rad);
        if sum > ceiling {
            return None;
        }
    }
    sum.is_finite().then(|| (sum / total).sqrt())
}

/// The angle between two unit vectors, stable when it is small.
///
/// **Not `DVec3::angle_between`.** That is an `acos` of the dot product, and for an angle of
/// 3e-10 radians the dot product is `1 - 4.5e-20`, which is exactly 1.0 in f64 -- so it returns
/// zero for every bearing this fit is trying to resolve. The objective was blind below about
/// 1e-8 radians, which is thirty times the noise it was meant to be measuring, and every fit
/// plateaued there. `atan2` of the cross product keeps its digits all the way down.
fn between(a: DVec3, b: DVec3) -> f64 {
    a.cross(b).length().atan2(a.dot(b))
}

/// Grid points kept for polishing, rather than only the best.
///
/// A short arc's residual surface is badly multi-modal: fitting three degrees of Saturn's orbit
/// admits wide shallow minima at 5, 55 and 229 AU, all of them thousands of sigma worse than
/// the truth and all of them found before it. So several starts are polished, and they must be
/// **separated** ones.
const STARTS: usize = 8;

/// Grid steps two kept starts must be apart, in the log of either range.
///
/// Without it a finer grid is a *worse* search: every one of the best eight points is a
/// neighbor of the same spurious minimum, and none of them explores anywhere else. Measured,
/// going from a 16% grid to a 2.7% one on Saturn's short arc: the best residual got worse, from
/// 1.1e-5 to 2.3e-5, because the extra resolution bought duplicates instead of diversity.
const APART: usize = 6;

/// Hooke-Jeeves pattern search in the log of each range: probe the eight directions, take the
/// steepest, then keep going the same way while it keeps helping.
///
/// The probe alone is not enough. The two ranges are to the same body a short time apart, so
/// the residual has a long narrow valley along "both larger together", and eight fixed
/// directions walk across such a valley rather than down it. The pattern move is what follows
/// it. Measured, on an Earth-like orbit seen over ten weeks: probing alone settled at 0.57% on
/// the period, and following the valley takes it to 0.03%.
///
/// A step that only ever shrinks is a third failure mode, and the one found first: it cannot
/// recover from starting on the wrong hill, and a grid point is a fifth of a decade from its
/// neighbor. That one settled on a 212-day period for a 365-day orbit.
fn polish(
    mut x: f64,
    mut y: f64,
    mut held: Fitted,
    mut span: f64,
    score: &impl Fn(f64, f64, f64) -> Option<Fitted>,
) -> (f64, f64, Fitted) {
    const WAYS: [(f64, f64); 8] = [
        (1.0, 0.0),
        (-1.0, 0.0),
        (0.0, 1.0),
        (0.0, -1.0),
        (1.0, 1.0),
        (-1.0, -1.0),
        (1.0, -1.0),
        (-1.0, 1.0),
    ];
    for _ in 0..REFINEMENTS {
        let probe = WAYS
            .into_iter()
            .filter_map(|(dx, dy)| {
                let (nx, ny) = (x * (dx * span).exp(), y * (dy * span).exp());
                score(nx, ny, held.residual_rad).map(|found| (dx, dy, nx, ny, found))
            })
            .min_by(|p, q| p.4.residual_rad.total_cmp(&q.4.residual_rad));

        let Some((dx, dy, nx, ny, found)) = probe else {
            span *= 0.5;
            if span < 1.0e-13 {
                break;
            }
            continue;
        };
        (x, y, held) = (nx, ny, found);

        // Along the valley, with the step growing, until it stops paying.
        let mut reach = span * 2.0;
        while reach < 4.0 {
            let (nx, ny) = (x * (dx * reach).exp(), y * (dy * reach).exp());
            match score(nx, ny, held.residual_rad) {
                Some(found) => {
                    (x, y, held) = (nx, ny, found);
                    reach *= 2.0;
                }
                None => break,
            }
        }
        span *= 1.3;
    }
    (x, y, held)
}

/// Least squares over every element, from the three-point solution as its starting guess.
///
/// **Gauss, then least squares**, and the second half is not optional. A three-point solution
/// passes *exactly* through three noisy rays, so it is an interpolation and carries their noise
/// as a systematic. Measured on an Earth-like orbit: the three-point solution sits 420 times its
/// own noise floor, and settling it over all six elements takes it to within a few.
///
/// A pattern search rather than a Gauss-Newton: the derivatives of a Kepler propagation with
/// respect to its elements are a page of algebra to get wrong, and six parameters at a dozen
/// probes a round is cheap enough that the difference does not pay for itself.
fn settle(mut held: Fitted, looks: &[Look], rounds: usize) -> Fitted {
    let mut scale = 1.0;
    for _ in 0..rounds {
        let (u, v) = basis(held.pole);
        let mut moved = false;
        for step in [scale, -scale] {
            /// One element nudged: the orbit, how far, and the plane's own two axes.
            type Nudge = fn(&Fitted, f64, DVec3, DVec3) -> Fitted;
            let ways: [Nudge; 7] = [
                |f, d, _, _| Fitted { semi_major_m: f.semi_major_m * (1.0 + d * 1.0e-3), ..*f },
                |f, d, _, _| Fitted {
                    eccentricity: (f.eccentricity + d * 1.0e-3).clamp(0.0, 0.94),
                    ..*f
                },
                |f, d, u, _| Fitted { pole: (f.pole + u * d * 1.0e-4).normalize(), ..*f },
                |f, d, _, v| Fitted { pole: (f.pole + v * d * 1.0e-4).normalize(), ..*f },
                |f, d, _, _| Fitted { periapsis_rad: f.periapsis_rad + d * 1.0e-4, ..*f },
                |f, d, _, _| Fitted { epoch_s: f.epoch_s + d * f.period_s * 1.0e-5, ..*f },
                |f, d, _, _| Fitted { period_s: f.period_s * (1.0 + d * 1.0e-4), ..*f },
            ];
            for way in ways {
                let tried = way(&held, step, u, v);
                if let Some(found) = residual(&tried, looks, held.residual_rad) {
                    held = Fitted { residual_rad: found, ..tried };
                    moved = true;
                }
            }
        }
        scale *= if moved { 1.3 } else { 0.5 };
        if scale < 1.0e-9 {
            break;
        }
    }
    let n = std::f64::consts::TAU / held.period_s;
    held.mu = n * n * held.semi_major_m * held.semi_major_m * held.semi_major_m;
    held
}

/// The orbit that best explains an arc of bearings, or `None` if they do not support one.
///
/// Two ranges are searched, log-spaced because a body could be anywhere from just off the star
/// to the far edge of the system and a linear grid would spend every point in the outer system.
/// Everything after those two is closed form.
pub fn fit(looks: &[Look]) -> Option<Fitted> {
    if looks.len() < LOOKS_NEEDED {
        return None;
    }
    let mut ordered = looks.to_vec();
    ordered.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
    // Spread across the arc rather than adjacent: three looks minutes apart barely differ, and
    // the plane they define would be whatever the noise says.
    let (a, b, c) = (ordered[0], ordered[ordered.len() / 2], ordered[ordered.len() - 1]);

    let score = |rho_a: f64, rho_b: f64, bound: f64| -> Option<Fitted> {
        if !(sound(rho_a) && sound(rho_b)) {
            return None;
        }
        let r1 = a.from_m + a.toward * rho_a;
        let r2 = b.from_m + b.toward * rho_b;
        let rho_c = coplanar_range(r1, r2, c.from_m, c.toward)?;
        let r3 = c.from_m + c.toward * rho_c;
        through([(r1, a.at_s), (r2, b.at_s), (r3, c.at_s)], &ordered, bound)
    };

    let au = 1.495_978_707e11;
    let step = (FAR_AU / NEAR_AU).ln() / (RANGES - 1) as f64;
    let grid: Vec<f64> = (0..RANGES).map(|i| NEAR_AU * (i as f64 * step).exp() * au).collect();
    // The grid keeps the best few, separated, rather than all of them or the best few
    // outright. No tightening bound here: a candidate worse than the eighth best is still worth
    // keeping if it is somewhere else, and rejecting it early is what destroys the diversity.
    /// A grid point: where it is on the grid, the two ranges, and what they fitted to.
    type Candidate = (usize, usize, f64, f64, Fitted);
    let mut found: Vec<Candidate> = Vec::new();
    for (i, x) in grid.iter().enumerate() {
        for (j, y) in grid.iter().enumerate() {
            let Some(fitted) = score(*x, *y, f64::INFINITY) else { continue };
            let near = found.iter().position(|(a, b, _, _, _)| {
                a.abs_diff(i) < APART && b.abs_diff(j) < APART
            });
            match near {
                Some(k) if fitted.residual_rad < found[k].4.residual_rad => {
                    found[k] = (i, j, *x, *y, fitted);
                }
                Some(_) => {}
                None => found.push((i, j, *x, *y, fitted)),
            }
        }
    }
    found.sort_by(|p, q| p.4.residual_rad.total_cmp(&q.4.residual_rad));
    found.truncate(STARTS);
    let found: Vec<(f64, f64, Fitted)> =
        found.into_iter().map(|(_, _, x, y, fitted)| (x, y, fitted)).collect();

    let mut polished: Vec<Fitted> = found
        .into_iter()
        .map(|(x, y, fitted)| settle(polish(x, y, fitted, step, &score).2, &ordered, SETTLINGS))
        .collect();
    polished.sort_by(|p, q| p.residual_rad.total_cmp(&q.residual_rad));

    // Unrivalled, or nothing. A separated solution that disagrees and explains the bearings
    // nearly as well means the arc admits more than one orbit, and saying so is worth more than
    // reporting whichever of them scored best.
    let best = polished.first().copied()?;
    let agrees = |other: &Fitted| {
        (other.semi_major_m / best.semi_major_m - 1.0).abs() < AGREEMENT
            && (other.period_s / best.period_s - 1.0).abs() < AGREEMENT
    };
    let rivalled = polished
        .iter()
        .skip(1)
        .any(|other| !agrees(other) && other.residual_rad < best.residual_rad * RIVAL);
    // The winner alone is settled properly. Every candidate got the same cheap budget above so
    // that the comparison between them is fair; spending the long budget on all of them costs
    // four times as much and changes which one wins not at all.
    (!rivalled).then(|| settle(best, &ordered, SETTLINGS * 8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng;

    const AU_M: f64 = 1.495_978_707e11;
    const MU_SUN: f64 = 1.327_124_4e20;
    const DAY_S: f64 = 86_400.0;
    const YEAR_S: f64 = 365.25 * DAY_S;

    /// The centroid floor of a ship's telescope in V, `resolution * CENTROID_FLOOR`, which is
    /// as well as a bearing to a bright planet is ever known.
    const SIGMA: f64 = 2.979e-7 * crate::knowledge::astrometry::CENTROID_FLOOR;

    /// A body on a known orbit about a star of known mass at the origin.
    struct Truth {
        semi_major_m: f64,
        eccentricity: f64,
        pole: DVec3,
        mu: f64,
    }

    impl Truth {
        fn period_s(&self) -> f64 {
            kepler::period::third_law(self.semi_major_m, self.mu)
        }

        fn fitted(&self) -> Fitted {
            Fitted {
                semi_major_m: self.semi_major_m,
                eccentricity: self.eccentricity,
                period_s: self.period_s(),
                pole: self.pole.normalize(),
                periapsis_rad: 1.8,
                epoch_s: 4.0e6,
                mu: self.mu,
                residual_rad: 0.0,
                looks: 0,
            }
        }

        fn at(&self, t_s: f64) -> DVec3 {
            self.fitted().at(t_s)
        }
    }

    fn like(au: f64, eccentricity: f64) -> Truth {
        Truth {
            semi_major_m: au * AU_M,
            eccentricity,
            pole: DVec3::new(0.02, -0.03, 1.0),
            mu: MU_SUN,
        }
    }

    /// Bearings from a ship on a circular `ship_au` orbit, `count` of them `every_s` apart,
    /// each nudged by a gaussian of `sigma` radians per axis.
    fn looks(truth: &Truth, ship_au: f64, count: usize, every_s: f64, sigma: f64) -> Vec<Look> {
        let ship_r = ship_au * AU_M;
        let ship_period = kepler::period::third_law(ship_r, truth.mu);
        (0..count)
            .map(|i| {
                let t = i as f64 * every_s;
                let phase = std::f64::consts::TAU * t / ship_period;
                let from = DVec3::new(phase.cos(), phase.sin(), 0.0) * ship_r;
                let toward = (truth.at(t) - from).normalize();
                let (x, y) = toward.any_orthonormal_pair();
                let nudge = x * rng::gaussian(rng::hash(&[i as u64, 1])) * sigma
                    + y * rng::gaussian(rng::hash(&[i as u64, 2])) * sigma;
                Look {
                    from_m: from,
                    toward: (toward + nudge).normalize(),
                    at_s: t,
                    sigma_rad: sigma.max(1.0e-12),
                }
            })
            .collect()
    }

    fn off(a: f64, b: f64) -> f64 {
        (a / b - 1.0).abs()
    }

    /// **An orbit out of angles and nothing else.** No range in any bearing and no mass given:
    /// the plane comes from three positions, the conic from a linear solve in it, and the period
    /// from a straight line through the mean anomalies. Ten weeks of an Earth-like orbit, at the
    /// best bearing a ship's telescope gives.
    #[test]
    fn an_orbit_comes_out_of_bearings_with_no_range_in_them() {
        let truth = like(1.0, 0.0167);
        let seen = looks(&truth, 5.0, 24, 3.0 * DAY_S, SIGMA);
        let fitted = fit(&seen).expect("twenty-four looks over ten weeks");

        let period = off(fitted.period_s, truth.period_s());
        assert!(period < 1.0e-3, "period off by {period}, past the 0.1% the survey is for");
        let axis = off(fitted.semi_major_m, truth.semi_major_m);
        assert!(axis < 1.0e-3, "axis off by {axis}");
        assert!(
            (fitted.eccentricity - truth.eccentricity).abs() < 3.0e-3,
            "eccentricity {} against {}",
            fitted.eccentricity,
            truth.eccentricity
        );
        let tilt = fitted.pole.angle_between(truth.pole.normalize());
        assert!(tilt.to_degrees() < 0.1, "plane out by {} degrees", tilt.to_degrees());

        // It cannot beat the noise it was given, and lands within a few hundred of it. The gap
        // is the three-point solution's interpolation bias, which `settle` reduces and does not
        // remove.
        let floor = residual(&truth.fitted(), &seen, f64::INFINITY).expect("the truth scores");
        assert!(floor < 2.0 * SIGMA, "the floor should be the noise, got {floor}");
        assert!(fitted.residual_rad > floor * 0.5, "it beat the noise it was given");
    }

    /// **The star's mass falls out of the orbit**, which is what lets a survey stop leaning on
    /// a luminosity prior for it. Nothing about `mu` goes into the fit: the period is measured
    /// off the timing line and `mu` is `n^2 a^3`.
    #[test]
    fn the_stars_mass_falls_out_of_an_orbit() {
        for (name, mu) in [("the sun", MU_SUN), ("a heavier star", MU_SUN * 3.0)] {
            let truth = Truth { mu, ..like(1.0, 0.0167) };
            // The same fraction of the orbit whatever the mass, so only the mass differs.
            let every_s = truth.period_s() / 120.0;
            let seen = looks(&truth, 5.0, 24, every_s, SIGMA);
            let fitted = fit(&seen).unwrap_or_else(|| panic!("{name} gives no orbit"));
            assert!(off(fitted.mu, mu) < 0.01, "{name}: mu off by {}", off(fitted.mu, mu));
        }
    }

    /// Mars is the eccentric one of the planets a survey is asked about, and the conic fit is
    /// what carries that rather than a circle with a correction.
    #[test]
    fn an_eccentric_orbit_comes_out_eccentric() {
        let truth = like(1.524, 0.0934);
        let seen = looks(&truth, 5.0, 24, 5.0 * DAY_S, SIGMA);
        let fitted = fit(&seen).expect("fits");
        let period = off(fitted.period_s, truth.period_s());
        assert!(period < 2.0e-3, "period off by {period}");
        assert!(
            (fitted.eccentricity - truth.eccentricity).abs() < 0.01,
            "eccentricity {} against {}",
            fitted.eccentricity,
            truth.eccentricity
        );
    }

    /// **A short arc gives no orbit, and that is the answer.** Three game months is 0.85% of
    /// Saturn's orbit, about three degrees, and three degrees does not determine a twenty-nine
    /// year orbit. The separated starts land on 2.7, 5.0, 2.6 and 230 AU with residuals within
    /// a factor of three of each other, so there is no answer to give -- and [`AGREEMENT`] is
    /// what turns that into a refusal instead of whichever of them scored best.
    ///
    /// This is the design doc's "Saturn's 3 degree arc to about a percent by 15 minutes" being
    /// wrong, and it is why the note beside it now says so.
    #[test]
    fn a_short_arc_gives_no_orbit_rather_than_a_wrong_one() {
        let truth = like(9.537, 0.0565);
        let seen = looks(&truth, 5.0, 24, 0.25 * YEAR_S / 24.0, SIGMA);
        let swept = (0.25 * YEAR_S / truth.period_s()) * 360.0;
        assert!(swept > 2.9 && swept < 3.3, "{swept} degrees is not the arc this is about");
        assert!(fit(&seen).is_none(), "three degrees cannot settle a 29 year orbit");

        // The same orbit over a quarter of its period does settle, which says the refusal is
        // about the arc and not about Saturn.
        let long = looks(&truth, 5.0, 24, truth.period_s() / 96.0, SIGMA);
        let fitted = fit(&long).expect("a quarter of an orbit is enough");
        assert!(off(fitted.period_s, truth.period_s()) < 0.05, "{}", off(fitted.period_s, truth.period_s()));
    }

    #[test]
    fn too_few_bearings_support_no_orbit() {
        let truth = like(1.0, 0.0167);
        for count in 0..LOOKS_NEEDED {
            let seen = looks(&truth, 5.0, count, 3.0 * DAY_S, SIGMA);
            assert!(fit(&seen).is_none(), "{count} looks gave an orbit");
        }
    }

    /// **Arc, not noise, is what an orbit costs.** Doubling the arc is four orders of
    /// magnitude on the period; making the bearings a hundred times worse is nothing at all,
    /// because over a short arc the error is the fit's own convergence and not the measurement.
    /// Worth knowing before anybody buys a better telescope to get a better orbit.
    #[test]
    fn a_longer_arc_buys_far_more_than_a_better_bearing() {
        let truth = like(1.0, 0.0167);
        let period_off = |count: usize, every_s: f64, sigma: f64| {
            let seen = looks(&truth, 5.0, count, every_s, sigma);
            off(fit(&seen).expect("fits at what is tried here").period_s, truth.period_s())
        };

        let short = period_off(24, 3.0 * DAY_S, SIGMA);
        let long = period_off(48, 3.0 * DAY_S, SIGMA);
        assert!(short < 1.0e-3, "seventy degrees of arc gave {short}");
        assert!(long < short / 1000.0, "twice the arc gave {long} against {short}");

        // A hundred times the noise, over the short arc, is lost in that.
        let noisy = period_off(24, 3.0 * DAY_S, SIGMA * 100.0);
        assert!(noisy < short * 3.0, "{noisy} against {short}");
        // A thousand times is not.
        let hopeless = period_off(24, 3.0 * DAY_S, SIGMA * 1000.0);
        assert!(hopeless > short * 2.0, "{hopeless} against {short}");
    }

    /// A body going the other way round is a pole pointing the other way, not a negative
    /// period. The slope of the timing line is what says which.
    #[test]
    fn a_retrograde_orbit_comes_out_with_its_pole_reversed() {
        let mut truth = like(1.0, 0.0167);
        truth.pole = -truth.pole;
        let seen = looks(&truth, 5.0, 24, 3.0 * DAY_S, SIGMA);
        let fitted = fit(&seen).expect("fits");
        assert!(fitted.period_s > 0.0, "a period is never negative");
        let tilt = fitted.pole.angle_between(truth.pole.normalize());
        assert!(tilt.to_degrees() < 0.1, "pole out by {} degrees", tilt.to_degrees());
    }

    /// The angle between two nearly-parallel unit vectors, which `DVec3::angle_between` cannot
    /// give: its `acos` of a dot product that has rounded to exactly 1.0 returns zero. The
    /// whole fit was blind below 1e-8 radians until this replaced it.
    #[test]
    fn a_tiny_angle_is_measurable_at_all() {
        let a = DVec3::X;
        for tiny in [1.0e-8, 1.0e-10, 1.0e-12] {
            let b = (a + DVec3::Y * tiny).normalize();
            assert!(
                (between(a, b) / tiny - 1.0).abs() < 1.0e-6,
                "{tiny} came out as {}",
                between(a, b)
            );
            if tiny < 1.0e-8 {
                assert_eq!(a.angle_between(b), 0.0, "glam still cannot, so this must");
            }
        }
    }
}
