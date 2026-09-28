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
//! Two numbers are searched and everything after them is closed form, until least squares over
//! every element settles the answer and says how well it is known: `knowledge::settle`. The
//! score is in radians against the bearings themselves: propagate the orbit to each observation
//! time and measure the angle it misses by. See `lightcone/docs/25-system-knowledge.md`.

use em_foundations::kepler;

use super::settle::{self, settle};
use super::turns::unwrappings;
use glam::{DMat3, DVec3};

/// Ranges tried per axis before refining, log-spaced across [`NEAR_AU`, `FAR_AU`].
///
/// Finer is both more accurate *and* faster here, which is not the usual trade: a coarser grid
/// leaves more candidates half-plausible, and those are the ones whose cost is a Kepler solve
/// per look rather than a rejected guard. Halving this to 128 lost a case and took 15% longer.
const RANGES: usize = 256;

/// How many times the orbit's apparent scale the range search allows it to be.
///
/// **The geometry hands over the band, and it has to come from every look.** A ray's nearest
/// point to the primary is at `-from . toward`, and how far it misses by there is the smallest
/// the orbit can be. The largest of those misses across the arc is the orbit's scale, and the
/// body is somewhere within a few times it.
///
/// Across the arc and not per ray: a body seen near opposition has a ray that passes almost
/// through its primary and misses by nothing, and a band built on that one look collapses to a
/// point.
///
/// This is what makes a satellite tractable at all: Io's scale comes out at 0.003 AU against an
/// observer 9.9 AU off, and the whole-system grid it replaces runs 0.02 to 200 AU in 2.7%
/// steps, of which Io's entire orbit is a twentieth of one.
const SPAN: f64 = 4.0;

/// How many times its own scale a body must sit from its primary before the narrow band is
/// used instead of the whole system's.
///
/// **The scale is a lower bound, and a weak one when the geometry is unkind.** A body near
/// conjunction has rays that pass close to its primary however big its orbit is, so the miss
/// says little. Two bodies at 5 and 5.2 AU have nearly the same period and so sit in near
/// permanent conjunction: measured, Jupiter watched from 5 AU gives a scale of 0.46 AU for a
/// 5.2 AU orbit, and a band built on that misses the truth entirely.
///
/// So the narrow band is used only where the reading is unambiguous. The two cases are three
/// orders of magnitude apart and nothing sits between them: that same Jupiter reads 11 and Io
/// about Jupiter reads 3500.
const SATELLITE: f64 = 100.0;

/// The band a body of a system can be in, astronomical units. Outside it there is nothing a
/// survey from inside would be looking at.
///
/// A bound on the answer as well as on the search. `1/r = A + B cos + C sin` gives the
/// semi-latus rectum as `1/A`, and three points nearly collinear in `(cos, sin)` put `A` near
/// zero and the axis anywhere: a generated system fitted over nine hours produced an orbit of
/// 2.9e16 AU with a plausible-looking 158 day period, and nothing else would have caught it.
const NEAR_AU: f64 = 0.02;
const FAR_AU: f64 = 200.0;

/// Meters in an astronomical unit.
const AU_M: f64 = 1.495_978_707e11;

/// The heaviest a thing at a focus may be, as multiples of the Sun.
///
/// The axis and the period each pass the band above and still imply a nonsense mass between
/// them: nine hours of a generated system fitted to 188 AU with a 187 day period, which is a
/// star of twenty-six million suns. Bounding the mass is not circular even though the mass is
/// one of the answers -- what is being fitted is an orbit about a *star*, and the range of
/// stars is a fact about stars and not about this orbit.
const MU_SUN: f64 = 1.327_124_4e20;
const HEAVIEST: f64 = 300.0;

/// Rounds of shrinking the search about the best pair, and the factor each round shrinks by.
const REFINEMENTS: usize = 100;

/// Gauss-Newton rounds every candidate gets. The one that wins gets eight times as many,
/// because the comparison has to be even and only the answer has to be exact.
const SETTLINGS: usize = 8;

/// Bearings fewer than this cannot pin an orbit: three looks make the conic fit exact and the
/// timing line nearly so, leaving nothing over to judge a candidate by.
pub const LOOKS_NEEDED: usize = 5;

/// Ranged looks that make a fit a solution rather than a search: three positions are an orbit.
pub const RANGED_NEEDED: usize = 3;

/// Three positions within this of collinear give a plane that is all rounding.
const DEGENERATE: f64 = 1.0e-6;

/// A fitted body must stay at least this much of the observer's own distance from the star away
/// from the observer.
///
/// Without it a short arc has an attractor at the *ship's own orbit*: put the body on top of
/// the observer and the range goes to zero, the parallax with it, and any orbit explains the
/// bearings. Three of the eight starts on Saturn's three-degree arc landed there, at 5.0000 AU
/// with no eccentricity, which is the 5 AU circle the ship was flying.
///
/// Not for a ranged look: its range is scored, which rules the attractor out, and a body that
/// close is what ranging is for. From low Earth orbit this bound is 1.5 million km.
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
pub(super) fn sound(x: f64) -> bool {
    x.is_finite() && x > 0.0
}

/// One look at a body: where from, which way, when, and how well.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub from_m: DVec3,
    pub toward: DVec3,
    pub at_s: f64,
    pub sigma_rad: f64,
    /// Meters and one sigma, where the look was close enough to range the body. A bearing with
    /// a range is a *position*, so three of them make this no longer a search: see [`fit`].
    ///
    /// The sigma is kept because the range is scored: see [`residual`]. Dropping it left a
    /// measured distance seeding the search and then constraining nothing, so the settle was
    /// free to slide the size and the depth away from the very solution the range had found.
    pub range_m: Option<(f64, f64)>,
}

impl Look {
    /// From a stored sighting, in the frame of whatever the body is being tested against.
    ///
    /// `primary_ly` is where that primary was **at this look's own time**, not now: a moon's
    /// planet moves between one look and the next, and a frame that ignored that would be
    /// fitting the planet's orbit and the moon's at once.
    pub fn of(seen: &crate::knowledge::Sighting, primary_ly: DVec3) -> Self {
        Self {
            from_m: (seen.bearing.observer_ly - primary_ly) * crate::system::M_PER_LY,
            toward: seen.bearing.toward,
            at_s: seen.observed_s,
            sigma_rad: seen.bearing.sigma_rad.max(f64::MIN_POSITIVE),
            range_m: seen.range_m,
        }
    }

    /// Where the body was, for a look that ranged it.
    fn place(&self) -> Option<DVec3> {
        self.range_m.map(|(range, _)| self.from_m + self.toward * range)
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
    /// Furthest from the primary this fit was ever allowed to put the body, meters.
    ///
    /// Carried on the fit because `residual` is the one place every candidate is scored, and a
    /// plausibility bound written in astronomical units is a bound about *stars*: Io's orbit is
    /// 0.0028 AU and Jupiter is a thousandth of a solar mass, so a guard tuned to planets
    /// throws away every satellite there is. The band the ranges were searched in is the honest
    /// bound, it is already computed, and it means the same thing at every level.
    pub reach_m: f64,
    /// The arc was too short to say anything about the eccentricity, so a circle was assumed.
    ///
    /// Not a claim that the orbit is circular. `1/r = A + B cos + C sin` needs the arc to bend
    /// to separate those three, and over a few degrees it does not: the matrix is singular and
    /// the size is all that survives. A record made from this states its eccentricity as
    /// `None`, which doc 25's rule 4 distinguishes from stating a circle.
    ///
    /// Three anchors can also fail to shape a conic on an arc that bends plenty, by falling at
    /// one phase; `settle::released` takes that circle back where the whole arc shapes one.
    pub assumed_circular: bool,
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
///
/// Written out rather than `any_orthonormal_vector`, because a stated orbit's covariance is in
/// this basis (`knowledge::record::Covariance`) and a library's choice of basis is free to
/// change under an upgrade. The coordinate axis least along the pole, so the projection never
/// degenerates.
pub(super) fn basis(pole: DVec3) -> (DVec3, DVec3) {
    let a = pole.abs();
    let axis = if a.x <= a.y && a.x <= a.z {
        DVec3::X
    } else if a.y <= a.z {
        DVec3::Y
    } else {
        DVec3::Z
    };
    let u = (axis - pole * pole.dot(axis)).normalize();
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

/// The orbit through a sequence of positions at their times, scored against every look.
///
/// Nothing here is searched. The positions give the plane, the plane gives the conic, the conic
/// gives the anomalies, and the anomalies with the times give the period.
///
/// The pole is the sum of `r_i x r_{i+1}`, which is twice the area each step sweeps and so
/// points along the orbit's own normal. Not `(r2 - r1) x (r3 - r1)`: for positions on a short
/// arc that cross product is the arc's *curvature*, a part in ten thousand of the same
/// magnitudes, and it fell under the degeneracy guard for every three-degree arc -- which is
/// exactly the case ranging exists to rescue.
///
/// `held_s` is an orbit already fitted, as seconds per radian, to count turns by: see
/// [`unwrappings`].
fn through(
    places: &[(DVec3, f64)],
    looks: &[Look],
    reach_m: f64,
    bound: f64,
    held_s: Option<f64>,
) -> Option<Fitted> {
    if places.len() < 3 {
        return None;
    }
    let mut pole = DVec3::ZERO;
    let mut scale = 0.0f64;
    for pair in places.windows(2) {
        let (Some((from, _)), Some((to, _))) = (pair.first(), pair.last()) else { continue };
        pole += from.cross(*to);
        scale = scale.max(from.length()).max(to.length());
    }
    if !sound(pole.length() - DEGENERATE * scale * scale) {
        return None;
    }
    let pole = pole.normalize();
    let (u, v) = basis(pole);
    let points: Vec<(f64, f64)> = places
        .iter()
        .map(|(r, _)| (r.dot(v).atan2(r.dot(u)), r.length()))
        .collect();
    // A circle where the conic is singular, which over a few degrees of arc it is: the rows
    // `(1, cos, sin)` are then nearly the same row three times over. The size still comes out,
    // and saying only that is better than solving a singular system for a shape.
    let (semi_latus_rectum, eccentricity, periapsis_rad, assumed_circular) = match conic(&points) {
        Some((p, e, w)) => (p, e, w, false),
        None => {
            let mean = points.iter().map(|(_, r)| r).sum::<f64>() / points.len() as f64;
            if !sound(mean) {
                return None;
            }
            (mean, 0.0, 0.0, true)
        }
    };
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
    let means: Vec<(f64, f64)> = points
        .iter()
        .zip(places)
        .map(|((theta, _), (_, t))| (anomaly(*theta), *t))
        .collect();
    // The anomalies may be saying more than one thing; the bearings say which. See
    // [`unwrappings`].
    let mut best: Option<Fitted> = None;
    for (per_rad, epoch_s) in unwrappings(&means, held_s) {
        let period_s = per_rad.abs() * std::f64::consts::TAU;
        if !sound(period_s) {
            continue;
        }
        let n = std::f64::consts::TAU / period_s;
        let against = best.as_ref().map_or(bound, |held| held.residual_rad.min(bound));
        let mut fitted = Fitted {
            semi_major_m,
            eccentricity,
            period_s,
            // A negative rate is a prograde orbit seen from the wrong side of its plane.
            pole: if per_rad < 0.0 { -pole } else { pole },
            periapsis_rad: if per_rad < 0.0 { -periapsis_rad } else { periapsis_rad },
            epoch_s,
            mu: n * n * semi_major_m * semi_major_m * semi_major_m,
            reach_m,
            assumed_circular,
            residual_rad: 0.0,
            looks: looks.len(),
        };
        let Some(found) = residual(&fitted, looks, against) else { continue };
        fitted.residual_rad = found;
        best = Some(fitted);
    }
    best
}

/// Weighted RMS of the angle between where the orbit says the body was and where it was seen.
///
/// `bound` is the best RMS found so far. A candidate that has already missed by more than that
/// cannot win, so the rest of its looks are not worth solving Kepler's equation for -- and most
/// candidates are hopeless within two or three of them. The bound is compared against the same
/// quantity the function returns, so the exit changes the cost and not the answer.
pub(super) fn residual(fitted: &Fitted, looks: &[Look], bound: f64) -> Option<f64> {
    if !admissible(fitted) {
        return None;
    }
    let total = total(looks);
    if !sound(total) {
        return None;
    }
    let ceiling = bound * bound * total;
    let mut sum = 0.0;
    for look in looks {
        let offset = offset(fitted, look)?;
        let miss = between(offset.normalize(), look.toward);
        sum += miss * miss / (look.sigma_rad * look.sigma_rad);
        if let Some(out) = ranged_out(look, offset) {
            sum += out * out;
        }
        if sum > ceiling {
            return None;
        }
    }
    sum.is_finite().then(|| (sum / total).sqrt())
}

/// What [`residual`] sums, one standardized residual each: a bearing's miss as its two
/// components across the line of sight, and a ranged look's range. For the least squares in
/// `knowledge::settle`, which needs the direction of each miss and not only its size.
pub(super) fn terms(fitted: &Fitted, looks: &[Look]) -> Option<Vec<f64>> {
    if !admissible(fitted) {
        return None;
    }
    let mut out = Vec::with_capacity(looks.len() * 3);
    for look in looks {
        let offset = offset(fitted, look)?;
        let seen = offset.normalize();
        let across = seen - look.toward * seen.dot(look.toward);
        // Scaled to the angle rather than its sine, so the two square to exactly what
        // `residual` adds, and a body behind the observer does not read as a small miss.
        let scale = between(seen, look.toward) / look.sigma_rad / across.length().max(f64::MIN_POSITIVE);
        let (x, y) = look.toward.any_orthonormal_pair();
        out.push(across.dot(x) * scale);
        out.push(across.dot(y) * scale);
        out.extend(ranged_out(look, offset));
    }
    out.iter().all(|t| t.is_finite()).then_some(out)
}

/// The weight [`residual`] divides by, which leaves its answer in radians.
///
/// A ranged look is two measurements rather than one, so it carries its bearing's weight
/// twice. Every term is a squared standardized residual -- how many sigma out -- so they add in
/// one metric although one is an angle and the other a distance.
pub(super) fn total(looks: &[Look]) -> f64 {
    looks
        .iter()
        .map(|l| if l.range_m.is_some() { 2.0 } else { 1.0 } / (l.sigma_rad * l.sigma_rad))
        .sum()
}

/// Whether an orbit is one this fit may answer with.
///
/// Every candidate in this file is scored through here and nowhere else, which is why the band
/// of [`FAR_AU`] is enforced here rather than where the three-point solution lands: a fit
/// settles its way out of the band, so checking only the starting point misses it.
fn admissible(fitted: &Fitted) -> bool {
    if !sound(fitted.semi_major_m) || !sound(fitted.period_s) || fitted.semi_major_m > fitted.reach_m {
        return false;
    }
    // And the thing at the focus has to be something: a star at the top of the chain, a planet
    // under one, a moon under that. Only the ceiling is a real statement, since a primary can
    // be as light as a rock.
    let n = std::f64::consts::TAU / fitted.period_s;
    let implied = n * n * fitted.semi_major_m.powi(3) / MU_SUN;
    implied > 0.0 && implied <= HEAVIEST
}

/// From the observer to where the orbit puts the body at the look's time. `None` for a
/// bearing-only look that puts it aboard: see [`NOT_ABOARD`].
fn offset(fitted: &Fitted, look: &Look) -> Option<DVec3> {
    let offset = fitted.at(look.at_s) - look.from_m;
    (look.range_m.is_some() || sound(offset.length() - NOT_ABOARD * look.from_m.length())).then_some(offset)
}

/// How many sigma a ranged look's range is out.
///
/// **Proximity is the instrument.** A range measured on a close pass is the one thing that
/// fixes the size of an orbit rather than its shape, and a fit that does not score it can walk
/// away from it: see `lightcone/docs/25-system-knowledge.md`.
fn ranged_out(look: &Look, offset: DVec3) -> Option<f64> {
    let (range, sigma) = look.range_m?;
    Some((offset.length() - range) / sigma.max(range * super::survey::RANGE_FLOOR))
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

impl Fitted {
    /// The fit as a record, in the elements [`crate::knowledge::Orientation`] is defined in.
    ///
    /// The plane's basis here is [`basis`], which is whatever `any_orthonormal_vector` returns
    /// and so is not a frame anything else shares. The record wants the standard pair instead:
    /// the ascending node's longitude in simulation axes, and periapsis measured round from
    /// that node. `knowledge::placed::placed_at` reads them straight into
    /// `em_foundations::kepler::state::Elements`, so a wrong convention here is a body drawn in
    /// the wrong place and nothing that complains.
    pub fn stated(
        &self,
        witness: crate::knowledge::Witness,
        about: Option<crate::knowledge::BodyId>,
        looks: &[Look],
        stated_s: f64,
    ) -> crate::knowledge::Orbit {
        let spread = settle::spread(self, looks);
        let node_dir = DVec3::Z.cross(self.pole).normalize_or(DVec3::X);
        let (u, v) = basis(self.pole);
        let periapsis_dir = u * self.periapsis_rad.cos() + v * self.periapsis_rad.sin();
        let periapsis = node_dir
            .cross(periapsis_dir)
            .dot(self.pole)
            .atan2(node_dir.dot(periapsis_dir));
        let au = self.semi_major_m / crate::navigation::AU;
        crate::knowledge::Orbit {
            witness,
            about,
            period_s: (self.period_s, spread.period_s),
            semi_major_au: (au, spread.semi_major_m / crate::navigation::AU),
            eccentricity: (!self.assumed_circular).then_some((self.eccentricity, spread.eccentricity)),
            orientation: crate::knowledge::Orientation::Known {
                pole: self.pole,
                sigma_rad: spread.pole_rad,
                node: node_dir.y.atan2(node_dir.x),
                periapsis,
            },
            epoch_s: Some((self.epoch_s, spread.epoch_s)),
            pivot_s: pivot(looks),
            covariance: spread.covariance,
            method: crate::knowledge::Method::Astrometric,
            stated_s,
            lineage: Vec::new(),
        }
    }
}

impl Fitted {
    /// A stated orbit back in this fit's terms, for reading its covariance: the inverse of
    /// [`Fitted::stated`]. `None` without a full orientation, an epoch and a sane size.
    pub(super) fn of(orbit: &crate::knowledge::Orbit) -> Option<Self> {
        let crate::knowledge::Orientation::Known { pole, node, periapsis, .. } = orbit.orientation else {
            return None;
        };
        let (epoch_s, _) = orbit.epoch_s?;
        let semi_major_m = orbit.semi_major_au.0 * crate::navigation::AU;
        let period_s = orbit.period_s.0;
        if !sound(semi_major_m) || !sound(period_s) {
            return None;
        }
        let node_dir = DVec3::new(node.cos(), node.sin(), 0.0);
        let toward = node_dir * periapsis.cos() + pole.cross(node_dir) * periapsis.sin();
        let (u, v) = basis(pole);
        let n = std::f64::consts::TAU / period_s;
        Some(Fitted {
            semi_major_m,
            eccentricity: orbit.eccentricity.map_or(0.0, |(e, _)| e),
            period_s,
            pole,
            periapsis_rad: toward.dot(v).atan2(toward.dot(u)),
            epoch_s,
            mu: n * n * semi_major_m * semi_major_m * semi_major_m,
            reach_m: f64::INFINITY,
            assumed_circular: orbit.eccentricity.is_none(),
            residual_rad: 0.0,
            looks: 0,
        })
    }
}

/// The weighted center of the looks' times, where a fit's phase is stated. `None` for no looks.
pub(super) fn pivot(looks: &[Look]) -> Option<f64> {
    let (sum, weight) = looks.iter().fold((0.0, 0.0), |(sum, weight), look| {
        let w = 1.0 / (look.sigma_rad * look.sigma_rad);
        (sum + w * look.at_s, weight + w)
    });
    sound(weight).then(|| sum / weight)
}

/// How much worse than it was an orbit may explain a longer arc and still be carried onto it
/// by [`refit`] rather than searched for again.
const STILL_AGREES: f64 = 3.0;

/// An orbit already fitted, carried onto a longer arc of the same body. `None` when the arc no
/// longer agrees with it, which is the caller's cue to [`fit`] from scratch.
///
/// **This is most of the fitting a survey does.** A body is refitted each time its arc grows by
/// half, and the grid is sixty-five thousand range pairs. The grid is what finds an orbit, not
/// what improves one.
///
/// Settled from the held elements first. Only where that does not land cleanly is the search
/// seeded with where the held orbit puts the body, polished in the ranges and settled, as a
/// search's candidates are, and the better of the two kept. The seeded search can recover a
/// held orbit too far out for a settle, and a settle can beat it: over fifteen days of Sol, 13
/// of 46 refits settled cleanly, and 3 more settled better than the search did. A refit costs
/// milliseconds either way, so this is for the answer and not the time.
///
/// A circle assumed for want of arc is never carried: a longer arc is exactly what may shape
/// the conic it could not.
pub fn refit(held: &Fitted, looks: &[Look]) -> Option<Fitted> {
    if held.assumed_circular {
        return None;
    }
    let settled = Some(settle(*held, looks, SETTLINGS * 8))
        .filter(|f| !f.assumed_circular && residual(f, looks, f64::INFINITY).is_some());
    if settled.is_some_and(|f| f.residual_rad <= held.residual_rad * SETTLED_CLEANLY) {
        return settled;
    }
    // Agreeing is not enough on its own: a settle that stops in a nearby minimum three times
    // worse still agrees, and on Sol that carried Mars from a fifth of its bar out to five.
    let searched = fit_from(looks, Some(held));
    [searched, settled.filter(|f| still_agrees(f, held))]
        .into_iter()
        .flatten()
        .min_by(|a, b| a.residual_rad.total_cmp(&b.residual_rad))
}

/// How much worse than the orbit it carries a settle may explain the longer arc and be taken
/// without the seeded search behind it. About the scatter of a noise-limited chi-square over
/// sixteen looks, so a settle at the noise passes and one in a nearby minimum does not.
const SETTLED_CLEANLY: f64 = 1.5;

/// Whether a fit carried onto a longer arc explains it nearly as well as `held` did its own.
fn still_agrees(carried: &Fitted, held: &Fitted) -> bool {
    !carried.assumed_circular && carried.residual_rad <= held.residual_rad * STILL_AGREES
}

// Per thread, so tests running side by side do not count each other's searches.
#[cfg(test)]
thread_local! {
    static SCORED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The orbit that best explains an arc of bearings, or `None` if they do not support one.
///
/// Two ranges are searched, log-spaced because a body could be anywhere from just off the star
/// to the far edge of the system and a linear grid would spend every point in the outer system.
/// Everything after those two is closed form.
pub fn fit(looks: &[Look]) -> Option<Fitted> {
    fit_from(looks, None)
}

fn fit_from(looks: &[Look], seed: Option<&Fitted>) -> Option<Fitted> {
    if looks.len() < LOOKS_NEEDED {
        return None;
    }
    let mut ordered = looks.to_vec();
    ordered.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
    // Spread across the arc rather than adjacent: three looks minutes apart barely differ, and
    // the plane they define would be whatever the noise says. Ranged looks first where there
    // are three of them, since those skip the search entirely.
    let ranged: Vec<Look> = ordered.iter().copied().filter(|l| l.range_m.is_some()).collect();
    let held_s = seed.map(|held| held.period_s / std::f64::consts::TAU);
    let anchors = if ranged.len() >= 3 { &ranged } else { &ordered };
    let (a, c) = (*anchors.first()?, *anchors.last()?);
    // The middle one is whichever look points furthest from both ends, not whichever sits in
    // the middle of the list. A survey revisits on a fixed cadence and a satellite has a short
    // period, so the two can resonate: twenty-four looks over exactly two of Io's orbits put
    // the first and the middle at the *same orbital phase*, which is two coincident points and
    // a conic through them that is singular however much the arc bends.
    let apart = |look: &Look| {
        between(look.toward, a.toward).min(between(look.toward, c.toward))
    };
    let b = *anchors
        .iter()
        .max_by(|p, q| apart(p).total_cmp(&apart(q)))?;

    // How big the orbit can be, from the whole arc: the largest distance from the primary that
    // any ray is forced to pass at. Then each anchor's band is the stretch of its own ray that
    // stays inside that, which is a quadratic and exact.
    let scale = ordered
        .iter()
        .map(|look| {
            let nearest = -look.from_m.dot(look.toward);
            (look.from_m + look.toward * nearest.max(0.0)).length()
        })
        .fold(0.0, f64::max);
    let nearest = ordered.iter().map(|l| l.from_m.length()).fold(f64::INFINITY, f64::min);
    let satellite = scale > 0.0 && nearest > scale * SATELLITE;
    // A satellite is bounded by its own apparent scale; anything else by the system.
    let reach = if satellite { (SPAN * scale).min(FAR_AU * AU_M) } else { FAR_AU * AU_M };

    // Ranged looks are positions, and three positions are an orbit outright: no grid, no
    // polish, nothing searched. What proximity buys is not a better search but no search.
    // Every ranged look, not three of them: the plane a short arc gives is only as good as the
    // number of positions defining it.
    //
    // Two ways from positions to an orbit, the better kept: the conic assumes a circle over a
    // short arc, and the body's motion (`super::state`) holds only while a cubic describes it.
    if ranged.len() >= RANGED_NEEDED {
        let places: Vec<(DVec3, f64)> =
            ranged.iter().filter_map(|l| Some((l.place()?, l.at_s))).collect();
        let best = [
            through(&places, &ordered, reach, f64::INFINITY, held_s),
            super::state::from_motion(&places, &ordered, reach),
        ]
            .into_iter()
            .flatten()
            .map(|found| settle(found, &ordered, SETTLINGS * 8))
            .min_by(|a, b| a.residual_rad.total_cmp(&b.residual_rad));
        if best.is_some() {
            return best;
        }
    }

    let score = |rho_a: f64, rho_b: f64, bound: f64| -> Option<Fitted> {
        #[cfg(test)]
        SCORED.with(|n| n.set(n.get() + 1));
        if !(sound(rho_a) && sound(rho_b)) {
            return None;
        }
        let r1 = a.from_m + a.toward * rho_a;
        let r2 = b.from_m + b.toward * rho_b;
        let rho_c = coplanar_range(r1, r2, c.from_m, c.toward)?;
        let r3 = c.from_m + c.toward * rho_c;
        through(&[(r1, a.at_s), (r2, b.at_s), (r3, c.at_s)], &ordered, reach, bound, held_s)
    };

    // A satellite's band is the stretch of its own ray that stays within `reach` of the
    // primary, which is a quadratic and exact. Anything else gets the whole system, log-spaced
    // because a body could be anywhere in it.
    let whole = (FAR_AU / NEAR_AU).ln() / (RANGES - 1) as f64;
    let over = |look: &Look| -> Vec<f64> {
        if !satellite {
            return (0..RANGES).map(|i| NEAR_AU * (i as f64 * whole).exp() * AU_M).collect();
        }
        let along = look.from_m.dot(look.toward);
        let discriminant = along * along - look.from_m.length_squared() + reach * reach;
        if !sound(discriminant) {
            return Vec::new();
        }
        let root = discriminant.sqrt();
        let (low, high) = ((-along - root).max(reach * 1.0e-6), -along + root);
        if !sound(high - low) {
            return Vec::new();
        }
        let step = (high - low) / (RANGES - 1) as f64;
        (0..RANGES).map(|i| low + i as f64 * step).collect()
    };
    let (first, second) = (over(&a), over(&b));
    if first.is_empty() || second.is_empty() {
        return None;
    }
    // A step of the first band, as a fraction, for the polish to start from.
    let step = (first.get(1)?.max(f64::MIN_POSITIVE) / first.first()?.max(f64::MIN_POSITIVE)).ln().abs().max(1.0e-9);

    if let Some(held) = seed {
        // The range along each anchor's own ray to where the held orbit puts the body then.
        let along = |look: &Look| (held.at(look.at_s) - look.from_m).dot(look.toward);
        let (x, y) = (along(&a), along(&b));
        let start = score(x, y, f64::INFINITY)?;
        let polished = settle(polish(x, y, start, step, &score).2, &ordered, SETTLINGS);
        let mut settled = settle(polished, &ordered, SETTLINGS * 8);
        if settled.assumed_circular {
            settled = settle::released(&settled, &ordered, SETTLINGS * 8).unwrap_or(settled);
        }
        let circle = settled.assumed_circular && ranged.len() < 3;
        return (!circle && settled.residual_rad <= held.residual_rad * STILL_AGREES).then_some(settled);
    }
    // The grid keeps the best few, separated, rather than all of them or the best few
    // outright. No tightening bound here: a candidate worse than the eighth best is still worth
    // keeping if it is somewhere else, and rejecting it early is what destroys the diversity.
    /// A grid point: where it is on the grid, the two ranges, and what they fitted to.
    type Candidate = (usize, usize, f64, f64, Fitted);
    let mut found: Vec<Candidate> = Vec::new();
    for (i, x) in first.iter().enumerate() {
        for (j, y) in second.iter().enumerate() {
            let Some(fitted) = score(*x, *y, f64::INFINITY) else { continue };
            let near = found.iter().position(|(a, b, _, _, _)| {
                a.abs_diff(i) < APART && b.abs_diff(j) < APART
            });

            match near.and_then(|k| found.get_mut(k)) {
                Some(held) if fitted.residual_rad < held.4.residual_rad => {
                    *held = (i, j, *x, *y, fitted);
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
        .map(|(x, y, fitted)| {
            let settled = settle(polish(x, y, fitted, step, &score).2, &ordered, SETTLINGS);
            // Before the rivalry, which a circle held on a conic it cannot fit would lose.
            match settled.assumed_circular {
                true => settle::released(&settled, &ordered, SETTLINGS).unwrap_or(settled),
                false => settled,
            }
        })
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
    if rivalled {
        return None;
    }
    // The winner alone is settled properly. Every candidate got the same cheap budget above so
    // that the comparison between them is fair; spending the long budget on all of them costs
    // four times as much and changes which one wins not at all.
    let best = settle(best, &ordered, SETTLINGS * 8);
    if !best.assumed_circular {
        return Some(best);
    }
    // A circle is only an answer when the positions were known. Assuming one drops two
    // parameters, so an arc too short to shape a conic can still be *fitted* by a circle -- and
    // from bearings alone that circle can be anywhere, because nothing pins the range.
    // Measured on the shard: ten hours of bearings on a moon at 0.0097 AU fitted a circle at 63
    // AU, agreed with by every separated start, implying a star of 299 suns and so squeaking
    // past the mass bound by a hair. If the arc cannot shape a conic and no look ranged it,
    // there is no orbit here -- unless the anchors were only unlucky, and the whole arc shapes
    // the conic they could not.
    settle::released(&best, &ordered, SETTLINGS * 8).or((ranged.len() >= 3).then_some(best))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use super::settle::{PARABOLIC, spread};
    use std::f64::consts::PI;
    use crate::knowledge::astrometry::Bearing;
    use crate::knowledge::Subject;
    use crate::rng;
    use crate::sky::StarId;

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
                periapsis_rad: fixed_periapsis(self.pole.normalize(), 1.8),
                epoch_s: 4.0e6,
                mu: self.mu,
                reach_m: f64::INFINITY,
                assumed_circular: false,
                residual_rad: 0.0,
                looks: 0,
            }
        }

        fn at(&self, t_s: f64) -> DVec3 {
            self.fitted().at(t_s)
        }
    }

    /// `angle` from the basis these fixtures were written against, as an angle in [`basis`], so
    /// that changing the basis does not move every orbit the tests are about.
    pub(in crate::knowledge) fn fixed_periapsis(pole: DVec3, angle: f64) -> f64 {
        let u = pole.any_orthonormal_vector();
        let toward = u * angle.cos() + pole.cross(u) * angle.sin();
        let (u, v) = basis(pole);
        toward.dot(v).atan2(toward.dot(u))
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
    /// each nudged by a gaussian of `sigma` radians per axis. No ranges: a telescope across a
    /// system does not get one. [`ranged`] adds them.
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
                    range_m: None,
                }
            })
            .collect()
    }

    /// A ship on a circular `ship_au` orbit watching whatever `place` says, `count` looks
    /// `every_s` apart.
    ///
    /// On an orbit and not adrift. A slow straight drift is a poor baseline and the fit says so:
    /// the same 72 degrees of Jupiter, watched from a ship drifting 1.5 AU in a line, gives two
    /// candidates 7% apart in period whose residuals differ by 1.5%, both fifty thousand times
    /// the noise floor -- so it refuses, correctly. From a 5 AU orbit, which sweeps six AU of
    /// arc in the same time, it lands on the truth at the floor.
    fn watched(
        place: &dyn Fn(f64) -> DVec3,
        mu: f64,
        ship_au: f64,
        count: usize,
        every_s: f64,
        sigma: f64,
    ) -> Vec<Look> {
        let ship_r = ship_au * AU_M;
        let ship_period = kepler::period::third_law(ship_r, mu);
        (0..count)
            .map(|i| {
                let t = i as f64 * every_s;
                let phase = std::f64::consts::TAU * t / ship_period;
                let from = DVec3::new(phase.cos(), phase.sin(), 0.0) * ship_r;
                let toward = (place(t) - from).normalize();
                let (x, y) = toward.any_orthonormal_pair();
                let nudge = x * rng::gaussian(rng::hash(&[i as u64, 11])) * sigma
                    + y * rng::gaussian(rng::hash(&[i as u64, 12])) * sigma;
                Look {
                    from_m: from,
                    toward: (toward + nudge).normalize(),
                    at_s: t,
                    sigma_rad: sigma.max(1.0e-12),
                    range_m: None,
                }
            })
            .collect()
    }

    /// The same looks, with a range on each to the fraction a close pass would give.
    fn ranged(truth: &Truth, seen: &[Look], fraction: f64) -> Vec<Look> {
        seen.iter()
            .enumerate()
            .map(|(i, look)| {
                let truth_range = (truth.at(look.at_s) - look.from_m).length();
                let slip = rng::gaussian(rng::hash(&[i as u64, 3])) * fraction * truth_range;
                Look { range_m: Some((truth_range + slip, fraction * truth_range)), ..*look }
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

        // At the noise it was given: a least-squares answer explains the bearings at least as
        // well as the truth does, and a fit cannot beat noise by much.
        let floor = residual(&truth.fitted(), &seen, f64::INFINITY).expect("the truth scores");
        assert!(floor < 2.0 * SIGMA, "the floor should be the noise, got {floor}");
        assert!(fitted.residual_rad <= floor, "{} against a floor of {floor}", fitted.residual_rad);
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

    /// **A refit carries an orbit onto a longer arc and lands where a search would.** Half the
    /// arc fitted from scratch, then the whole of it once by search and once carried: the two
    /// answers agree to the fit's own accuracy, and the carried one cost a settle.
    #[test]
    fn a_refit_lands_where_a_search_would() {
        let truth = like(1.524, 0.0934);
        let seen = looks(&truth, 5.0, 48, 5.0 * DAY_S, SIGMA);
        let half = fit(&seen[..24]).expect("half the arc fits");
        let searched = fit(&seen).expect("the whole arc fits");
        let carried = refit(&half, &seen).expect("the longer arc agrees with the shorter one's orbit");
        let period = off(carried.period_s, searched.period_s);
        assert!(period < 2.0e-3, "carried period off the searched one by {period}");
        assert!(off(carried.semi_major_m, searched.semi_major_m) < 2.0e-3);
        assert!(off(carried.period_s, truth.period_s()) < 2.0e-3);
    }

    /// A refit counts whole turns from the period it carries, which a search cannot do over
    /// sixteen looks spread across forty orbits.
    #[test]
    fn a_refit_counts_the_turns_a_search_cannot() {
        let truth = like(0.387, 0.2056);
        let period = truth.period_s();
        let dense = looks(&truth, 5.0, 24, period / 20.0, SIGMA);
        let held = fit(&dense).expect("an orbit and a fifth, well sampled, fits");
        assert!(off(held.period_s, period) < 1.0e-3, "premise: the short arc fits");

        // Sixteen looks over forty orbits, each 2.63 orbits apart.
        let long = looks(&truth, 5.0, 16, period * 2.63, SIGMA);
        let searched = fit(&long).map_or(f64::INFINITY, |f| off(f.period_s, period));
        assert!(searched > 1.0e-2, "premise: a search finds the period to {searched}");
        let carried = refit(&held, &long).expect("the held period says how many turns");
        assert!(off(carried.period_s, period) < 1.0e-5, "period off by {}", off(carried.period_s, period));
        assert!(off(carried.semi_major_m, truth.semi_major_m) < 1.0e-3);
    }

    /// **A refit is a settle where the held orbit is good,** and searches nothing: the seeded
    /// search behind it costs three times as much and lands in the same place.
    #[test]
    fn a_good_orbit_is_carried_by_a_settle() {
        let truth = like(1.524, 0.0934);
        let seen = looks(&truth, 5.0, 48, 5.0 * DAY_S, SIGMA);
        let half = fit(&seen[..24]).expect("half the arc fits");
        SCORED.with(|n| n.set(0));
        let carried = refit(&half, &seen).expect("the longer arc agrees");
        assert_eq!(SCORED.with(|n| n.get()), 0, "a good orbit was searched for again");
        let floor = residual(&truth.fitted(), &seen, f64::INFINITY).expect("the truth scores");
        assert!(carried.residual_rad <= floor, "{} against a floor of {floor}", carried.residual_rad);
    }

    /// **A held period that miscounts the turns is not settled into a wrong count.** Two
    /// percent out over forty orbits is most of a turn by the end, which a settle cannot climb
    /// out of, so it disagrees and the seeded search takes over. From half a percent out the
    /// settle lands on the truth where the seeded search alone took an alias 3.2 times the
    /// period: sixteen looks 2.63 orbits apart fit several periods at the noise, and the search
    /// counts turns from whatever period it is handed. That alias is still open behind the
    /// settle, so this asserts only that the settle makes nothing worse.
    #[test]
    fn a_period_that_miscounts_the_turns_is_not_settled_into_a_wrong_count() {
        let truth = like(0.387, 0.2056);
        let period = truth.period_s();
        let held = fit(&looks(&truth, 5.0, 24, period / 20.0, SIGMA)).expect("an orbit and a fifth fits");
        let long = looks(&truth, 5.0, 16, period * 2.63, SIGMA);
        for factor in [1.005, 1.02, 0.98, 1.1] {
            let wrong = Fitted { period_s: held.period_s * factor, ..held };
            let carried = refit(&wrong, &long);
            let right = carried.is_some_and(|f| off(f.period_s, period) < 1.0e-5);
            assert!(right || carried == fit_from(&long, Some(&wrong)), "{factor}: {carried:?}");
            if factor == 1.005 {
                assert!(right, "half a percent out is what a settle recovers: {carried:?}");
            }
        }
    }

    /// **And never carries a wrong orbit forward**: seeded from one, it either finds its way to
    /// the truth or refuses, and a refusal sends the caller back to the full search.
    #[test]
    fn a_refit_never_carries_a_wrong_orbit() {
        let truth = like(1.524, 0.0934);
        let seen = looks(&truth, 5.0, 24, 5.0 * DAY_S, SIGMA);
        let right = fit(&seen).expect("fits");
        let wrong = Fitted { period_s: right.period_s * 1.3, semi_major_m: right.semi_major_m * 1.2, ..right };
        if let Some(carried) = refit(&wrong, &seen) {
            let period = off(carried.period_s, truth.period_s());
            assert!(period < 2.0e-3, "carried a wrong orbit: period off by {period}");
        }
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

    /// **Arc buys far more than a better bearing.** At the noise floor the period's error is
    /// the noise's, so it scales with the bearing: ten times worse is ten times worse. Doubling
    /// the arc is sixty times better. Worth knowing before anybody buys a better telescope to
    /// get a better orbit.
    #[test]
    fn a_longer_arc_buys_far_more_than_a_better_bearing() {
        let truth = like(1.0, 0.0167);
        let period = |count: usize, sigma: f64| {
            let seen = looks(&truth, 5.0, count, 3.0 * DAY_S, sigma);
            let fitted = fit(&seen).expect("fits at what is tried here");
            let bar = spread(&fitted, &seen).period_s;
            let miss = (fitted.period_s - truth.period_s()).abs();
            assert!(miss < 4.0 * bar, "{count} looks at {sigma:e}: {miss} s out, sigma {bar} s");
            bar
        };

        let short = period(24, SIGMA);
        let long = period(48, SIGMA);
        assert!(long < short / 30.0, "twice the arc gave {long} against {short}");
        let noisy = period(24, SIGMA * 10.0);
        assert!((noisy / short - 10.0).abs() < 1.0, "ten times the noise gave {noisy} against {short}");
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

    /// **The elements have to mean what the reader thinks they mean.** `Fitted` keeps its plane
    /// in a basis nothing else shares; the record keeps a node and an argument of periapsis. A
    /// wrong conversion is a body drawn in the wrong place and nothing that complains, so this
    /// puts the orbit through the reader that draws it and checks it lands where the fit says.
    #[test]
    fn a_stated_orbit_places_the_body_where_the_fit_does() {
        for (au, e, pole) in [
            (1.0, 0.0167, DVec3::new(0.02, -0.03, 1.0)),
            (2.5, 0.31, DVec3::new(0.4, 0.2, 1.0)),
            (0.7, 0.0, DVec3::new(-0.1, 0.5, 1.0)),
        ] {
            let truth = Truth { pole, ..like(au, e) };
            let fitted = truth.fitted();
            let seen = looks(&truth, 5.0, 12, truth.period_s() / 60.0, SIGMA);
            let orbit = fitted.stated(crate::knowledge::Witness(1), None, &seen, 0.0);

            for step in 0..8 {
                let now = fitted.epoch_s + truth.period_s() * step as f64 / 8.0;
                let mine = fitted.at(now) / crate::navigation::AU;
                let theirs = crate::knowledge::body::placed_for_test(&orbit, now);
                let miss = mine.distance(theirs);
                assert!(
                    miss < au * 1.0e-6,
                    "at {step}/8 round a {au} AU orbit the reader is {miss} AU off {mine}",
                );
            }
        }
    }

    /// **A stated orbit reads back as the fit it came from,** which the covariance depends on:
    /// it is in the fit's own terms, so a reader that rebuilt the elements differently would
    /// read every number as something else.
    #[test]
    fn a_stated_orbit_reads_back_as_its_fit() {
        for (au, e, pole) in [
            (1.0, 0.0167, DVec3::new(0.02, -0.03, 1.0)),
            (2.5, 0.31, DVec3::new(0.4, 0.2, 1.0)),
            (0.7, 0.2, DVec3::new(-0.9, 0.5, -0.1)),
        ] {
            let fitted = Truth { pole, ..like(au, e) }.fitted();
            let seen = looks(&Truth { pole, ..like(au, e) }, 5.0, 12, fitted.period_s / 60.0, SIGMA);
            let back = Fitted::of(&fitted.stated(crate::knowledge::Witness(1), None, &seen, 0.0)).expect("reads back");
            assert!(back.pole.distance(fitted.pole) < 1.0e-15);
            assert!(kepler::anomaly::wrap_pi(back.periapsis_rad - fitted.periapsis_rad).abs() < 1.0e-12);
            for t in [0.0, 1.0e7, -3.0e7] {
                assert!(back.at(t).distance(fitted.at(t)) < 1.0, "{} m apart", back.at(t).distance(fitted.at(t)));
            }
        }
    }

    /// A fit states the phase's error at its pivot, and a reader grows it from there.
    #[test]
    fn a_fit_states_how_far_round_its_body_is() {
        use crate::knowledge::placed::{Placed, placed_at};

        for (au, e) in [(1.0, 0.0167), (1.524, 0.0934), (0.7, 0.0)] {
            let truth = like(au, e);
            let seen = looks(&truth, 5.0, 24, truth.period_s() / 120.0, SIGMA);
            let fitted = fit(&seen).expect("fits");
            let orbit = fitted.stated(crate::knowledge::Witness(1), None, &seen, 0.0);
            let (_, sigma_s) = orbit.epoch_s.expect("an epoch");
            assert!(sigma_s > 0.0 && sigma_s < truth.period_s() * 0.5, "{sigma_s} s");

            let pivot = orbit.pivot_s.expect("a fit has a pivot");
            let along = |t: f64| match placed_at(&orbit, t) {
                Placed::Known { error, .. } => error.along_rad,
                other => panic!("{other:?}"),
            };
            let at_pivot = std::f64::consts::TAU * sigma_s / fitted.period_s;
            assert!((along(pivot) - at_pivot).abs() < 1.0e-12 * at_pivot.max(1.0));
            assert!(along(pivot + truth.period_s()) > at_pivot, "a year on it is no worse");
        }
    }

    /// Looks as [`looks`] takes them, with the noise drawn from `seed`.
    fn seeded(truth: &Truth, count: usize, every_s: f64, seed: u64) -> Vec<Look> {
        looks(truth, 5.0, count, every_s, SIGMA)
            .into_iter()
            .enumerate()
            .map(|(i, look)| {
                let toward = (truth.at(look.at_s) - look.from_m).normalize();
                let (x, y) = toward.any_orthonormal_pair();
                let nudge = x * rng::gaussian(rng::hash(&[seed, i as u64, 51])) * SIGMA
                    + y * rng::gaussian(rng::hash(&[seed, i as u64, 52])) * SIGMA;
                Look { toward: (toward + nudge).normalize(), ..look }
            })
            .collect()
    }

    /// How far round its orbit `orbit` puts the body at `t` past where it truly is, radians of
    /// mean anomaly, and the sigma it states for that.
    fn along_miss(truth: &Truth, orbit: &crate::knowledge::Orbit, t: f64) -> (f64, f64) {
        let crate::knowledge::Placed::Known { offset_au, error } = crate::knowledge::placed::placed_at(orbit, t) else {
            panic!("placed")
        };
        let miss = offset_au - truth.at(t) / crate::navigation::AU;
        (miss.dot(error.pace_au) / error.pace_au.length_squared(), error.along_rad)
    }

    /// **Least squares lands at the noise, whatever the shape and however long the arc.** The
    /// pattern search it replaced stalled 800 times the noise over a fifth of an Earth-like
    /// orbit and a million times over a whole one, and refused outright from 0.9 orbits up,
    /// where the three anchors fall at one phase and the conic falls back to a circle.
    #[test]
    fn fits_land_at_the_noise_floor() {
        for (name, truth, orbits) in [
            ("Earth, a fifth", like(1.0, 0.0167), 0.2),
            ("Mars, a fifth", like(1.524, 0.0934), 0.2),
            ("a circle, a fifth", like(0.7, 0.0), 0.2),
            ("Earth, 0.9", like(1.0, 0.0167), 0.9),
            ("Mars, 1.5", like(1.524, 0.0934), 1.5),
            ("Earth, 2", like(1.0, 0.0167), 2.0),
            ("Mercury, 3", like(0.387, 0.2056), 3.0),
        ] {
            let seen = looks(&truth, 5.0, 24, truth.period_s() * orbits / 23.0, SIGMA);
            let fitted = fit(&seen).unwrap_or_else(|| panic!("{name}: no orbit"));
            let floor = residual(&truth.fitted(), &seen, f64::INFINITY).expect("the truth scores");
            let over = fitted.residual_rad / floor;
            assert!((0.7..=1.0).contains(&over), "{name}: {over} times the noise floor");
            assert!(!fitted.assumed_circular, "{name}: a circle held on a conic");
            let bar = spread(&fitted, &seen);
            let z = (fitted.period_s - truth.period_s()) / bar.period_s;
            assert!(z.abs() < 4.0, "{name}: period {z} sigma out");
            let z = (fitted.eccentricity - truth.eccentricity) / bar.eccentricity;
            assert!(z.abs() < 4.0, "{name}: eccentricity {z} sigma out");
        }
    }

    /// **The bars cover the truth as often as they say.** Over seeded noise, each element's
    /// truth lands within one sigma about two times in three and within three nearly always.
    /// The walked bars this replaced put an Earth-like fit 4.5 sigma out on its phase alone.
    ///
    /// Settled from the truth, so what is tested is the covariance and not the search.
    #[test]
    fn the_bars_cover_the_truth_as_often_as_they_say() {
        const SEEDS: u64 = 60;
        for (name, truth, orbits) in [("Earth, a fifth", like(1.0, 0.0167), 0.2), ("Mars, 1.5", like(1.524, 0.0934), 1.5)] {
            let mut z: Vec<f64> = Vec::new();
            let mut poles = 0;
            for seed in 0..SEEDS {
                let seen = seeded(&truth, 24, truth.period_s() * orbits / 23.0, seed);
                let fitted = settle(truth.fitted(), &seen, SETTLINGS * 8);
                let bar = spread(&fitted, &seen);
                let orbit = fitted.stated(crate::knowledge::Witness(1), None, &seen, 0.0);
                let (phase, sigma) = along_miss(&truth, &orbit, orbit.pivot_s.expect("a pivot"));
                z.extend([
                    (fitted.period_s - truth.period_s()) / bar.period_s,
                    (fitted.semi_major_m - truth.semi_major_m) / bar.semi_major_m,
                    (fitted.eccentricity - truth.eccentricity) / bar.eccentricity,
                    phase / sigma,
                ]);
                // The pole's bar is its worse direction, so it covers at least as often.
                if fitted.pole.angle_between(truth.pole.normalize()) > 3.0 * bar.pole_rad {
                    poles += 1;
                }
            }
            let within = |k: f64| z.iter().filter(|z| z.abs() < k).count() as f64 / z.len() as f64;
            assert!((0.55..0.8).contains(&within(1.0)), "{name}: {} within one sigma", within(1.0));
            assert!(within(3.0) > 0.97, "{name}: {} within three", within(3.0));
            assert!(poles <= 2, "{name}: the pole was {poles} of {SEEDS} times outside three sigma");
        }
    }

    /// **The phase's error carried to any time matches the scatter.** A body's place along its
    /// orbit, from a span before its pivot to ten spans after, against where it truly is: the
    /// misses in their own sigma have an RMS of about one at every one of those times.
    ///
    /// Carried by the elements' whole covariance. Carried by each element's own sigma and the
    /// phase's correlation with the period, the bar within a span of a short arc was ten times
    /// too wide, because there the period's error is cancelled by the eccentricity's.
    #[test]
    fn the_phase_error_carried_forward_matches_the_scatter() {
        const SEEDS: u64 = 60;
        for (name, truth, orbits) in [("Earth, a fifth", like(1.0, 0.0167), 0.2), ("Mars, 1.5", like(1.524, 0.0934), 1.5)] {
            let span = truth.period_s() * orbits;
            let spans = [-1.0, 0.0, 0.5, 1.0, 2.0, 4.0, 10.0];
            let mut z = vec![Vec::new(); spans.len()];
            for seed in 0..SEEDS {
                let seen = seeded(&truth, 24, span / 23.0, seed);
                let fitted = settle(truth.fitted(), &seen, SETTLINGS * 8);
                let orbit = fitted.stated(crate::knowledge::Witness(1), None, &seen, 0.0);
                let pivot = orbit.pivot_s.expect("a fit has a pivot");
                for (k, n) in spans.iter().enumerate() {
                    let (miss, sigma) = along_miss(&truth, &orbit, pivot + n * span);
                    z[k].push(miss / sigma);
                }
            }
            for (n, z) in spans.iter().zip(&z) {
                let rms = (z.iter().map(|z| z * z).sum::<f64>() / z.len() as f64).sqrt();
                assert!(rms < 1.5, "{name}, {n} spans on: overconfident, the misses are {rms} sigma");
                assert!(rms > 0.7, "{name}, {n} spans on: the misses are only {rms} sigma");
            }
        }
    }

    /// An error bar has to stay inside what the element means. An eccentricity walked past one
    /// is a hyperbola, and a pole is at most half a turn from any other, so a bar that runs off
    /// to infinity in either is reporting a shape the fit does not describe.
    #[test]
    fn an_error_bar_stays_inside_what_the_element_means() {
        // Noisy, so the elements are loosely constrained and the walks run long.
        let mut checked = 0;
        for (au, e) in [(1.0, 0.0167), (5.2, 0.049), (0.4, 0.7)] {
            let truth = like(au, e);
            let seen = looks(&truth, 5.0, 48, 3.0 * DAY_S, SIGMA * 6.0);
            let Some(fitted) = fit(&seen) else { continue };
            checked += 1;
            let spread = spread(&fitted, &seen);
            assert!(
                spread.eccentricity <= PARABOLIC + 1.0e-12,
                "e bar of {} at {au} AU",
                spread.eccentricity
            );
            assert!(fitted.eccentricity + spread.eccentricity <= 1.0, "the bar reaches a hyperbola");
            assert!(spread.pole_rad <= PI + 1.0e-12, "pole bar of {} rad", spread.pole_rad);
        }
        assert!(checked > 0, "no arc fitted, so nothing was checked");
    }

    /// An element's error bar is the marginal one, everything else free to take up the slack.
    /// Held fixed instead, the period's comes out far too small, because the period and the axis
    /// trade against each other.
    #[test]
    fn the_error_bars_are_the_marginal_ones() {
        let truth = like(1.0, 0.0167);
        let seen = looks(&truth, 5.0, 48, 3.0 * DAY_S, SIGMA);
        let fitted = fit(&seen).expect("fits");
        let spread = spread(&fitted, &seen);

        assert!(spread.period_s > 0.0 && spread.period_s.is_finite(), "{:?}", spread);
        assert!(spread.semi_major_m > 0.0 && spread.eccentricity > 0.0 && spread.pole_rad > 0.0);

        let period_miss = (fitted.period_s - truth.period_s()).abs();
        assert!(
            period_miss < 3.0 * spread.period_s,
            "{period_miss} s out with a sigma of {} s",
            spread.period_s
        );
        let axis_miss = (fitted.semi_major_m - truth.semi_major_m).abs();
        assert!(axis_miss < 3.0 * spread.semi_major_m, "{axis_miss} m out of {}", spread.semi_major_m);
        // And not absurdly wide either, or it would say nothing.
        assert!(spread.period_s / fitted.period_s < 1.0e-3, "{}", spread.period_s / fitted.period_s);

        // Holding the other elements fixed is what this exists to avoid, and the difference is
        // two orders of magnitude.
        let conditional = {
            let worse = (fitted.residual_rad * fitted.residual_rad
                + 1.0 / seen.iter().map(|l| 1.0 / (l.sigma_rad * l.sigma_rad)).sum::<f64>())
            .sqrt();
            let mut step = 1.0e-12;
            loop {
                let tried = Fitted { period_s: fitted.period_s * (1.0 + step), ..fitted };
                match residual(&tried, &seen, f64::INFINITY) {
                    Some(r) if r > worse => break step * fitted.period_s,
                    _ => step *= 1.6,
                }
            }
        };
        assert!(
            spread.period_s > conditional * 5.0,
            "marginal {} against conditional {conditional}",
            spread.period_s
        );
    }

    /// **The whole chain, from filed bearings to a believed orbit.** `looks_at` puts them in
    /// the star's frame, `fit` solves, `stated` writes the elements, and `body_belief` reads
    /// them back into a place. The fit itself is tested above; this is the plumbing around it,
    /// which is where a frame or a unit goes wrong.
    #[test]
    fn a_filed_arc_becomes_a_believed_orbit() {
        use crate::knowledge::{Knowledge, Witness};

        let star = StarId::synthesize("arc", 1);
        let star_ly = DVec3::new(3.0, -1.0, 0.5);
        let body = crate::knowledge::BodyId::of(star, "Kettle");
        let subject = Subject::Body { star, body };

        let truth = like(1.0, 0.0167);
        let seen = looks(&truth, 5.0, 48, 3.0 * DAY_S, SIGMA);
        let mut k = Knowledge::new(Witness(7));
        for look in &seen {
            k.sighted(
                subject,
                crate::knowledge::Sighting {
                    witness: Witness(7),
                    observed_s: look.at_s,
                    bearing: Bearing {
                        observer_ly: star_ly + look.from_m / crate::system::M_PER_LY,
                        toward: look.toward,
                        sigma_rad: look.sigma_rad,
                    },
                    size: None,
                    range_m: None,
                    spin_s: None,
                    band: em_spectra::Band::V,
                    flux: 1.0e-9,
                    flux_sigma: 1.0e-12,
                    lineage: Vec::new(),
                },
            );
        }

        // Only as many as the file keeps, which is what the fit will really be given.
        let held = k.looks_at(subject, &|_| Some(star_ly));
        assert_eq!(held.len(), crate::knowledge::BEARINGS_KEPT.min(seen.len()));

        assert_eq!(k.unfitted(star), Some(subject), "it has bearings and no orbit");
        let now = 1.0e9;
        assert!(k.fit_orbit(subject, star_ly, now), "the arc supports an orbit");
        assert_eq!(k.unfitted(star), None, "and nothing is due once it is fitted");

        let belief = k.body_belief(star, body, truth.fitted().epoch_s).expect("held");
        assert_eq!(belief.method, Some(crate::knowledge::Method::Astrometric));
        let (period, sigma) = belief.period_s.expect("a period");
        assert!(off(period, truth.period_s()) < 0.02, "period off by {}", off(period, truth.period_s()));
        assert!(sigma > 0.0 && sigma.is_finite());

        // And it places the body, which is the point of carrying an orientation and an epoch.
        let crate::knowledge::Placed::Known { offset_au, error } = belief.position_now else {
            panic!("a full orientation should place it, got {:?}", belief.position_now)
        };
        let want = truth.at(truth.fitted().epoch_s) / crate::navigation::AU;
        assert!(
            offset_au.distance(want) < 0.05,
            "placed at {offset_au} against {want}, sigma {}", error.total_au()
        );
    }

    /// **A moon finds its planet, and weighing the planet falls out of it.** Nothing here is a
    /// moon special case: a Keplerian orbit puts its primary at a focus, so the candidate that
    /// works as a focus is the primary, and the same code finds the star for a planet and the
    /// planet for its moon. The planet's mass is then `4 pi^2 a^3 / P^2` of the moon's orbit,
    /// which is the only way anything here weighs anything.
    #[test]
    fn a_moon_finds_its_planet_and_the_planet_is_weighed_by_it() {
        use crate::knowledge::{Knowledge, Witness};

        let star = StarId::synthesize("arc", 3);
        let star_ly = DVec3::new(3.0, -1.0, 0.5);
        let mut k = Knowledge::new(Witness(7));

        // A Jupiter at 5.2 AU and an Io about it: a thousandth of the star's mass at a four
        // hundredth of the distance, so the two orbits share nothing but a plane.
        let planet = like(5.2, 0.048);
        let planet_id = crate::knowledge::BodyId::of(star, "Jupiter");
        let moon_mu = MU_SUN * 9.54e-4;
        let moon_r = 4.217e8;
        let moon_period = kepler::period::third_law(moon_r, moon_mu);
        let (u, v) = planet.pole.normalize().any_orthonormal_pair();
        let moon_at = |t: f64| {
            let turn = std::f64::consts::TAU * t / moon_period;
            planet.at(t) + (u * turn.cos() + v * turn.sin()) * moon_r
        };

        // Each over its own arc, which is what a survey really gets: one revisit cadence is a
        // fifth of Io's orbit and a thousandth of Jupiter's, so by the time the planet has swept
        // enough sky the moon has gone round hundreds of times.
        let planet_span = planet.period_s() * 0.2;
        let moon_span = moon_period * 2.37;
        // Io is about a two-hundredth of Jupiter's brightness, and a primary has to outshine
        // what goes round it.
        let file_ranged = |k: &mut Knowledge, subject: Subject, seen: &[Look], flux: f64| {
            for look in seen {
                k.sighted(
                    subject,
                    crate::knowledge::Sighting {
                        witness: Witness(7),
                        observed_s: look.at_s,
                        bearing: Bearing {
                            observer_ly: star_ly + look.from_m / crate::system::M_PER_LY,
                            toward: look.toward,
                            sigma_rad: look.sigma_rad,
                        },
                        size: None,
                        range_m: look.range_m,
                        spin_s: None,
                        band: em_spectra::Band::V,
                        flux,
                        flux_sigma: 1.0e-12,
                        lineage: Vec::new(),
                    },
                );
            }
        };
        let planet_subject = Subject::Body { star, body: planet_id };
        let moon_id = crate::knowledge::BodyId::of(star, "Io");
        let moon_subject = Subject::Body { star, body: moon_id };
        let planet_looks = watched(&|t| planet.at(t), MU_SUN, 5.0, 24, planet_span / 24.0, SIGMA);
        file_ranged(&mut k, planet_subject, &planet_looks, 1.0e-9);
        // Ranged, which is to say visited. A moon's orbit from bearings alone at survey range
        // is the piece doc 25 records as open: the depth is observable at nine hundred sigma
        // but its basin is thirty times narrower than a step of the range grid, so the search
        // cannot land in it. From a close pass the ranges are measured and there is no search.
        let moon_looks = watched(&moon_at, MU_SUN, 5.0, 24, moon_span / 24.0, SIGMA);
        let moon_ranged: Vec<Look> = moon_looks
            .iter()
            .enumerate()
            .map(|(i, look)| {
                let truth_range = (moon_at(look.at_s) - look.from_m).length();
                let slip = rng::gaussian(rng::hash(&[i as u64, 21])) * 1.0e-6 * truth_range;
                Look { range_m: Some((truth_range + slip, 1.0e-6 * truth_range)), ..*look }
            })
            .collect();
        file_ranged(&mut k, moon_subject, &moon_ranged, 5.0e-12);

        // The planet first, because a moon cannot be placed against a planet nobody has placed.
        assert!(k.fit_orbit(planet_subject, star_ly, planet_span), "the planet fits");
        assert_eq!(
            k.body_belief(star, planet_id, planet_span).expect("held").about,
            None,
            "a planet goes round the star"
        );

        assert!(k.fit_orbit(moon_subject, star_ly, planet_span), "the moon fits");
        let moon = k.body_belief(star, moon_id, planet_span).expect("held");
        assert_eq!(moon.about, Some(planet_id), "the moon goes round the planet, not the star");
        let (au, _) = moon.semi_major_au.expect("an orbit");
        assert!(
            (au * crate::navigation::AU / moon_r - 1.0).abs() < 0.1,
            "{:e} m against {moon_r:e}",
            au * crate::navigation::AU
        );

        // The moon is placed from the star even so, by adding its planet's place to its own.
        let crate::knowledge::Placed::Known { offset_au, .. } = moon.position_now else {
            panic!("a moon with a placed planet is placed, got {:?}", moon.position_now)
        };
        let want = moon_at(planet_span) / crate::navigation::AU;
        assert!(
            offset_au.distance(want) < 0.5,
            "placed at {offset_au} against {want}"
        );

        // And the planet now has a mass, which nothing else in this file could have given it.
        let planet_now = k.body_belief(star, planet_id, planet_span).expect("held");
        let (kg, sigma) = planet_now.mass_kg.expect("a moon weighs its planet");
        let truth_kg = moon_mu / 6.674_30e-11;
        assert!((kg / truth_kg - 1.0).abs() < 0.3, "{kg:e} kg against {truth_kg:e} +/- {sigma:e}");
        assert!(sigma > 0.0 && sigma.is_finite());

        // The moon has no satellite of its own, so nothing weighs it.
        assert_eq!(moon.mass_kg, None, "nothing goes round the moon");
    }

    /// A moon is fitted against where its planet was seen: a planet's orbit three hours late, a
    /// third of Io's orbit out, is taken out of the moon's frame by the planet's own bearings.
    #[test]
    fn a_moon_is_fitted_against_where_its_planet_was_seen() {
        use crate::knowledge::{BodyId, Knowledge, Sighting, Witness};

        let star = StarId::synthesize("arc", 5);
        let star_ly = DVec3::new(3.0, -1.0, 0.5);
        let mut k = Knowledge::new(Witness(7));
        let file = |k: &mut Knowledge, subject: Subject, seen: &[Look], flux: f64| {
            for look in seen {
                let bearing = Bearing {
                    observer_ly: star_ly + look.from_m / crate::system::M_PER_LY,
                    toward: look.toward,
                    sigma_rad: look.sigma_rad,
                };
                k.sighted(
                    subject,
                    Sighting {
                        witness: Witness(7),
                        observed_s: look.at_s,
                        bearing,
                        size: None,
                        range_m: None,
                        spin_s: None,
                        band: em_spectra::Band::V,
                        flux,
                        flux_sigma: flux * 1.0e-3,
                        lineage: Vec::new(),
                    },
                );
            }
        };

        let planet = like(5.2, 0.048);
        let planet_id = BodyId::of(star, "Jupiter");
        let planet_subject = Subject::Body { star, body: planet_id };
        let moon_mu = MU_SUN * 9.54e-4;
        let moon_r = 4.217e8;
        let moon_period = kepler::period::third_law(moon_r, moon_mu);
        let (u, v) = planet.pole.normalize().any_orthonormal_pair();
        let moon_at = |t: f64| {
            let turn = std::f64::consts::TAU * t / moon_period;
            planet.at(t) + (u * turn.cos() + v * turn.sin()) * moon_r
        };

        // The planet seen every six hours across the moon's run, and believed three hours late.
        let hour = 3600.0;
        let planet_looks = watched(&|t| planet.at(t), MU_SUN, 5.0, 24, 6.0 * hour, SIGMA);
        file(&mut k, planet_subject, &planet_looks, 1.0e-9);
        let late = Fitted { epoch_s: planet.fitted().epoch_s + 3.0 * hour, ..planet.fitted() };
        k.orbits(planet_subject, late.stated(Witness(7), None, &planet_looks, 0.0));

        // The moon on a follow-up run, as far as its fit has got: 0, 1, 2, 4 ... 32 hours,
        // three quarters of Io's orbit.
        let moon_subject = Subject::Body { star, body: BodyId::of(star, "Io") };
        let ship_period = kepler::period::third_law(5.0 * AU_M, MU_SUN);
        let times: Vec<f64> = std::iter::once(0.0).chain((0..6).map(|n| hour * 2f64.powi(n))).collect();
        let moon_looks: Vec<Look> = times
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                let phase = std::f64::consts::TAU * t / ship_period;
                let from = DVec3::new(phase.cos(), phase.sin(), 0.0) * 5.0 * AU_M;
                let toward = (moon_at(t) - from).normalize();
                let (x, y) = toward.any_orthonormal_pair();
                let nudge = x * rng::gaussian(rng::hash(&[i as u64, 41])) * SIGMA
                    + y * rng::gaussian(rng::hash(&[i as u64, 42])) * SIGMA;
                Look { from_m: from, toward: (toward + nudge).normalize(), at_s: t, sigma_rad: SIGMA, range_m: None }
            })
            .collect();
        file(&mut k, moon_subject, &moon_looks, 5.0e-12);

        // Premise: in the frame of the believed orbit alone, the moon comes out wrong or not
        // at all.
        let believed = k.looks_at(moon_subject, &|t| Some(star_ly + k.placed(star, planet_id, t)? / crate::system::M_PER_LY));
        let alone = fit(&believed).map_or(f64::INFINITY, |f| off(f.semi_major_m, moon_r));
        assert!(alone > 0.01, "premise: the believed frame alone fits the moon to {alone}");

        let end = *times.last().expect("looks");
        assert!(k.fit_orbit(moon_subject, star_ly, end), "the moon fits");
        let moon = k.body_belief(star, BodyId::of(star, "Io"), end).expect("held");
        assert_eq!(moon.about, Some(planet_id));
        let (au, sigma_au) = moon.semi_major_au.expect("an orbit");
        let axis = off(au * crate::navigation::AU, moon_r);
        assert!(axis < 1.0e-3, "axis off by {axis}, against {alone} in the believed frame");
        let miss = (au * crate::navigation::AU - moon_r).abs() / crate::navigation::AU;
        assert!(miss < 3.0 * sigma_au, "{miss:e} AU out against a bar of {sigma_au:e}");
        let period = off(moon.period_s.expect("a period").0, moon_period);
        assert!(period < 1.0e-3, "period off by {period}");
    }

    /// A body is fitted when its bearings have outgrown its orbit, oldest statement first, and
    /// never before it has enough of them to judge a candidate by.
    #[test]
    fn only_a_body_whose_bearings_have_outgrown_its_orbit_is_due() {
        use crate::knowledge::{Knowledge, Witness};

        let star = StarId::synthesize("arc", 2);
        let mut k = Knowledge::new(Witness(7));
        let subject = |n: u64| Subject::Body { star, body: crate::knowledge::BodyId::of(star, &format!("b{n}")) };
        let sighting = |at_s: f64| crate::knowledge::Sighting {
            witness: Witness(7),
            observed_s: at_s,
            bearing: Bearing { observer_ly: DVec3::X, toward: DVec3::Y, sigma_rad: 1.0e-9 },
            size: None,
            range_m: None,
            spin_s: None,
            band: em_spectra::Band::V,
            flux: 1.0e-9,
            flux_sigma: 1.0e-12,
            lineage: Vec::new(),
        };

        // Too few looks: not due, however long it has been held.
        for i in 0..(LOOKS_NEEDED as u64 - 1) {
            k.sighted(subject(1), sighting(i as f64));
        }
        assert_eq!(k.unfitted(star), None, "{LOOKS_NEEDED} looks are needed");

        k.sighted(subject(1), sighting(99.0));
        assert_eq!(k.unfitted(star), Some(subject(1)), "now it has enough");

        // A second body with more recent bearings waits its turn behind the first, since
        // neither has ever been fitted and the order is by how long that has been true.
        for i in 0..LOOKS_NEEDED as u64 {
            k.sighted(subject(2), sighting(1000.0 + i as f64));
        }
        assert!(matches!(k.unfitted(star), Some(_)), "one of them is due");

        // An orbit stated after the newest bearing settles that body.
        let orbit = crate::knowledge::Orbit {
            witness: Witness(7),
            about: None,
            period_s: (1.0, 0.1),
            semi_major_au: (1.0, 0.1),
            eccentricity: None,
            orientation: crate::knowledge::Orientation::Unknown,
            epoch_s: None,
            pivot_s: None,
            covariance: None,
            method: crate::knowledge::Method::Astrometric,
            stated_s: 1.0e6,
            lineage: Vec::new(),
        };
        k.orbits(subject(1), orbit.clone());
        k.orbits(subject(2), orbit);
        assert_eq!(k.unfitted(star), None, "both are up to date");

        // A transit's orbit is somebody else's method and does not count as having fitted one.
        let mut fresh = Knowledge::new(Witness(7));
        for i in 0..LOOKS_NEEDED as u64 {
            fresh.sighted(subject(3), sighting(i as f64));
        }
        fresh.orbits(
            subject(3),
            crate::knowledge::Orbit { method: crate::knowledge::Method::Transit, stated_s: 1.0e6, ..orbit_of() },
        );
        assert_eq!(fresh.unfitted(star), Some(subject(3)), "a transit is not an astrometric fit");
    }

    /// **A transit and a fit are two statements, not one.**
    ///
    /// A craft that has watched a body transit and then fitted its arc holds both, and neither
    /// is a correction of the other. Held one per witness they overwrote each other every
    /// pass: a fit replaced by a transit's mass-prior shell, which made the body look unfitted,
    /// which refitted it, which the next log re-read undid.
    #[test]
    fn a_transit_and_a_fit_are_both_kept() {
        use crate::knowledge::{Knowledge, Method, Witness};

        let star = StarId::synthesize("arc", 4);
        let subject = Subject::Body { star, body: crate::knowledge::BodyId::of(star, "one") };
        let mut k = Knowledge::new(Witness(7));
        let stated = |method, at_s: f64| crate::knowledge::Orbit {
            method,
            stated_s: at_s,
            semi_major_au: (if method == Method::Transit { 1.2 } else { 1.0 }, 0.1),
            ..orbit_of()
        };

        k.orbits(subject, stated(Method::Transit, 10.0));
        k.orbits(subject, stated(Method::Astrometric, 20.0));
        assert_eq!(k.file(subject).unwrap().orbits().len(), 2, "both statements are held");

        // A later transit does not take the fit away, and the belief keeps reading the fit.
        k.orbits(subject, stated(Method::Transit, 30.0));
        let held = k.file(subject).unwrap().orbits();
        assert_eq!(held.len(), 2, "a later transit replaces the earlier transit only");
        assert_eq!(held.iter().filter(|o| o.method == Method::Astrometric).count(), 1);

        let belief = k.body_belief(star, crate::knowledge::BodyId::of(star, "one"), 100.0).unwrap();
        assert_eq!(belief.method, Some(Method::Astrometric), "a fit stands above a transit shell");
        assert_eq!(belief.semi_major_au.map(|(a, _)| a), Some(1.0));

        // And a newer fit still wins over itself.
        k.orbits(subject, crate::knowledge::Orbit { semi_major_au: (1.1, 0.01), ..stated(Method::Astrometric, 40.0) });
        let belief = k.body_belief(star, crate::knowledge::BodyId::of(star, "one"), 100.0).unwrap();
        assert_eq!(belief.semi_major_au.map(|(a, _)| a), Some(1.1));
    }

    /// **A body that cannot be fitted must not hold the queue.**
    ///
    /// An arc too short to shape an orbit states nothing, so a queue ranked by what has been
    /// stated hands the same body every fit slot forever and nothing else in the system is
    /// ever fitted at all. The attempt is what has to be recorded.
    #[test]
    fn a_failed_fit_goes_to_the_back_of_the_queue() {
        use crate::knowledge::{Knowledge, Witness};

        let star = StarId::synthesize("arc", 3);
        let mut k = Knowledge::new(Witness(7));
        let subject = |n: u64| Subject::Body { star, body: crate::knowledge::BodyId::of(star, &format!("b{n}")) };
        // All pointing one way from one place: no parallax, no curvature, nothing to fit.
        let sighting = |at_s: f64| crate::knowledge::Sighting {
            witness: Witness(7),
            observed_s: at_s,
            bearing: Bearing { observer_ly: DVec3::X, toward: DVec3::Y, sigma_rad: 1.0e-9 },
            size: None,
            range_m: None,
            spin_s: None,
            band: em_spectra::Band::V,
            flux: 1.0e-9,
            flux_sigma: 1.0e-12,
            lineage: Vec::new(),
        };
        for n in 1..=3u64 {
            for i in 0..LOOKS_NEEDED as u64 {
                k.sighted(subject(n), sighting(i as f64));
            }
        }

        // Every body is served in turn, although not one of them can be fitted.
        let mut served = Vec::new();
        for tick in 0..3 {
            let due = k.unfitted(star).expect("each body is due once");
            served.push(due);
            assert!(!k.fit_orbit(due, DVec3::X, 100.0 + tick as f64), "none of these can be fitted");
        }
        for n in 1..=3u64 {
            assert!(served.contains(&subject(n)), "{:?} never got a turn: {served:?}", subject(n));
        }

        // Then nothing, however many ticks go by: the same arc fails the same way, and
        // retrying it every tick is what held a whole shard's tick for as long as anyone
        // surveyed.
        assert_eq!(k.unfitted(star), None, "a failed fit is offered again with nothing new");

        // A look that barely lengthens the arc is not enough either.
        let span = (LOOKS_NEEDED - 1) as f64;
        k.sighted(subject(2), sighting(span * 1.2));
        assert_eq!(k.unfitted(star), None, "a fifth longer is not a new arc");

        // A longer one is.
        k.sighted(subject(2), sighting(span * 2.0));
        assert_eq!(k.unfitted(star), Some(subject(2)), "the arc grew and it gets another turn");
    }

    fn orbit_of() -> crate::knowledge::Orbit {
        crate::knowledge::Orbit {
            witness: crate::knowledge::Witness(7),
            about: None,
            period_s: (1.0, 0.1),
            semi_major_au: (1.0, 0.1),
            eccentricity: None,
            orientation: crate::knowledge::Orientation::Unknown,
            epoch_s: None,
            pivot_s: None,
            covariance: None,
            method: crate::knowledge::Method::Astrometric,
            stated_s: 0.0,
            lineage: Vec::new(),
        }
    }

    /// **Ranges settle the arc that bearings could not, and say what they still cannot.** Three
    /// degrees of Saturn's orbit is refused from bearings alone, correctly, because four
    /// different orbits explain it. Ranged looks are positions, so the same three degrees gives
    /// the plane and the size -- but *not* the eccentricity: `1/r = A + B cos + C sin` needs the
    /// arc to bend to separate those three, and over three degrees the matrix is singular
    /// whatever the ranges are worth. So it reports a circle and states its eccentricity as
    /// unconstrained, and what it calls the axis is really the radius it is at.
    #[test]
    fn a_range_settles_an_arc_that_bearings_cannot() {
        let truth = like(9.537, 0.0565);
        let bearings = looks(&truth, 5.0, 24, 0.25 * YEAR_S / 24.0, SIGMA);
        assert!(fit(&bearings).is_none(), "bearings alone cannot, which is the point");

        // A part in a thousand on each range, well short of what a close pass gives.
        let close = ranged(&truth, &bearings, 1.0e-3);
        let fitted = fit(&close).expect("positions over three degrees are an orbit");
        assert!(fitted.assumed_circular, "three degrees cannot shape a conic");

        // The size it reports is where the body is, which for an eccentric orbit is not the
        // axis: Saturn at 0.0565 runs from 9.0 to 10.1 AU and this arc is near the near end.
        let here = (truth.at(bearings[0].at_s).length() / AU_M, fitted.semi_major_m / AU_M);
        assert!((here.1 / here.0 - 1.0).abs() < 0.02, "{:?} AU: it is at {}", here, here.0);
        let tilt = fitted.pole.angle_between(truth.pole.normalize());
        assert!(tilt.to_degrees() < 1.0, "plane out by {} degrees", tilt.to_degrees());
        // And the period to about a tenth, from three degrees of it.
        let period = off(fitted.period_s, truth.period_s());
        assert!(period < 0.15, "period off by {period}");

        // The record says the eccentricity is unconstrained rather than saying it is zero,
        // which doc 25's rule 4 is about, and its bars say so too: the true axis is inside the
        // axis's bar, although the circle's size is the radius the body is at.
        let orbit = fitted.stated(crate::knowledge::Witness(1), None, &close, 0.0);
        assert_eq!(orbit.eccentricity, None);
        let (au, sigma_au) = orbit.semi_major_au;
        let z = (au - truth.semi_major_m / AU_M) / sigma_au;
        assert!(z.abs() < 3.0, "the axis is {z} sigma out, a bar taken as if the circle were known");
        let (period, sigma) = orbit.period_s;
        let z = (period - truth.period_s()) / sigma;
        assert!(z.abs() < 3.0, "the period is {z} sigma out");

        // A quarter of the orbit, ranged, does shape the conic and gets everything.
        let long = ranged(&truth, &looks(&truth, 5.0, 24, truth.period_s() / 96.0, SIGMA), 1.0e-3);
        let better = fit(&long).expect("a quarter of an orbit is enough");
        assert!(!better.assumed_circular, "a quarter of an orbit bends plenty");
        assert!(off(better.semi_major_m, truth.semi_major_m) < 0.02, "{}", off(better.semi_major_m, truth.semi_major_m));
        assert!(off(better.period_s, truth.period_s()) < 0.05, "{}", off(better.period_s, truth.period_s()));
        assert!(
            (better.eccentricity - truth.eccentricity).abs() < 0.03,
            "eccentricity {} against {}",
            better.eccentricity,
            truth.eccentricity
        );
    }

    /// **A circle is released only where the arc can shape a conic.** Ten hours of bearings on a
    /// moon once fitted a circle at 63 AU, agreed with by every start, because a circle from
    /// bearings alone can be anywhere. Releasing the circle must not bring that back: over a
    /// twentieth of Ganymede's orbit the eccentricity is not pinned, so there is no orbit.
    #[test]
    fn a_short_arc_keeps_its_circle_refused() {
        let mu = MU_SUN * 9.54e-4;
        let truth = Truth { semi_major_m: 1.07e9, eccentricity: 0.0013, pole: DVec3::new(0.02, -0.03, 1.0), mu };
        let seen = watched(&|t| truth.at(t), MU_SUN, 5.0, 24, 10.0 * 3600.0 / 23.0, SIGMA);
        if let Some(fitted) = fit(&seen) {
            assert!(!fitted.assumed_circular, "a circle from bearings alone");
            let axis = off(fitted.semi_major_m, truth.semi_major_m);
            assert!(axis < 0.1, "an orbit {axis} out of its axis");
        }
    }

    /// **A circle assumed is a circle kept.** Where the arc cannot shape a conic the fit says
    /// so and reports no eccentricity -- but the settle was still free to walk one off zero,
    /// and `stated` then dropped it while keeping the periapsis and the epoch that had been
    /// fitted *with* it. The body was drawn up to two eccentricities of arc from where it was
    /// seen, which for Saturn on a three-month arc is most of an astronomical unit.
    #[test]
    fn a_circle_assumed_is_a_circle_fitted() {
        let truth = like(9.537, 0.0565);
        let bearings = looks(&truth, 5.0, 24, 0.25 * YEAR_S / 24.0, SIGMA);
        let close = ranged(&truth, &bearings, 1.0e-3);
        let fitted = fit(&close).expect("positions over three degrees are an orbit");
        assert!(fitted.assumed_circular);
        assert_eq!(fitted.eccentricity, 0.0, "a circle has no eccentricity to fit");
        assert_eq!(fitted.periapsis_rad, 0.0, "nor a periapsis to put it at");

        // And what is reported places the body where it was actually seen. This is the whole
        // consequence: a shape that is not reported must not be a shape that was used.
        let orbit = fitted.stated(crate::knowledge::Witness(1), None, &close, 0.0);
        assert_eq!(orbit.eccentricity, None);
        for look in &close {
            let (range, _) = look.range_m.expect("these were ranged");
            let offset = fitted.at(look.at_s) - look.from_m;
            let out = (offset.length() - range).abs() / range;
            assert!(out < 0.01, "drawn {out} of the way off its measured range");
            // Not at the noise, and it cannot be: a circle fitted through an arc of an
            // eccentricity-0.057 ellipse is the wrong shape by construction, and what it buys
            // for that is a size and a plane it can state. Microradians, not milliradians, and
            // under a percent on the range, which least squares gives up first because each
            // range is worth millions of times less than a bearing.
            let miss = between(offset.normalize(), look.toward);
            assert!(miss < 1.0e-5, "drawn {miss} rad from where it was seen");
        }
    }

    /// **Proximity is the instrument, so a measured range has to be scored.** It seeded the
    /// search and then constrained nothing, which left the settle free to slide the size and
    /// the depth away from the very positions that had found them.
    ///
    /// The objective is what changed, so the objective is what is tested: a range the fit
    /// disagrees with has to cost it, and one it agrees with has to cost nothing.
    #[test]
    fn a_range_the_fit_disagrees_with_costs_it() {
        let truth = like(5.203, 0.0489);
        let bearings = looks(&truth, 5.0, 24, truth.period_s() / 96.0, SIGMA);
        let right = truth.fitted();
        // A part in ten thousand, which is far looser than a close pass gives.
        let with = |scale: f64| -> Vec<Look> {
            bearings
                .iter()
                .map(|l| {
                    let d = (right.at(l.at_s) - l.from_m).length();
                    Look { range_m: Some((d * scale, d * 1.0e-4)), ..*l }
                })
                .collect()
        };

        let blind = residual(&right, &bearings, f64::INFINITY).expect("the truth fits itself");
        let agreeing = residual(&right, &with(1.0), f64::INFINITY).expect("and agrees with its own ranges");
        let disagreeing = residual(&right, &with(0.99), f64::INFINITY).expect("this one it does not");

        // A hundredth out on each range is a hundred sigma, and the objective has to see it.
        assert!(
            disagreeing > 10.0 * agreeing,
            "a violated range cost {disagreeing} against {agreeing}, which is nothing"
        );
        // A range that agrees adds a measurement and no miss, so the weighted answer falls.
        assert!(agreeing < blind, "{agreeing} against {blind}");
        assert!(agreeing > blind * 0.5, "and it is still the same fit: {agreeing} against {blind}");
    }

    /// There is nothing to search: the three positions are the answer and the rest is settling
    /// it. Counted rather than timed, since a timing loses to a loaded machine.
    #[test]
    fn a_ranged_fit_does_no_searching() {
        let truth = like(1.0, 0.0167);
        let bearings = looks(&truth, 5.0, 24, 3.0 * DAY_S, SIGMA);
        let close = ranged(&truth, &bearings, 1.0e-4);
        let scored = |looks: &[Look]| {
            SCORED.with(|n| n.set(0));
            fit(looks).expect("fits");
            SCORED.with(|n| n.get())
        };
        // Otherwise a counter that never counts would pass.
        assert!(scored(&bearings) > 0, "bearings alone are searched");
        assert_eq!(scored(&close), 0, "ranged looks are not");
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
