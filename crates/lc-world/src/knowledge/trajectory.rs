//! The path a craft was seen on, fitted from where it was seen, so later sightings that keep to
//! it need hold only their times. See `lightcone/docs/22-provenance.md` §Where craft are.
//!
//! Fitted from positions and times and nothing about how the craft is flown: a conic of any
//! eccentricity, or a straight line. A body's `G m` is public, so a conic about it is six numbers,
//! a position and a velocity at one instant, and least squares over every place finds them. A fit
//! is accepted only if every place it was made from lies on it.
//!
//! **Dynamical, not geometric.** The plane-and-`1/r` solve the body fitter uses (`knowledge::arc`)
//! needs the arc to bend; a craft coasting round a star moves a few millionths of a turn between
//! places, the shape comes out wrong, and the conic falls off within days. Propagating a state by
//! Kepler's equation and fitting it to the places works on any arc.
//!
//! **The primary is found, not assumed.** A coast keeps the body it was solved about until it is
//! repatched, which need not be the body whose sphere it is in now; a conic about the wrong one
//! holds for a while and then drifts off. So the body holding the latest place is tried, then each
//! body it goes round, out to the star.

use em_foundations::kepler::state;
use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::BodyId;
use crate::coast::{Coast, PARABOLIC_TOLERANCE};
use crate::flight::JULIAN_YEAR_S;
use crate::system::{LocalSystem, M_PER_LY};

/// Places a fit needs.
pub const FIT_PLACES: usize = 5;

/// Gauss-Newton steps before a fit is given up.
const MOST_STEPS: usize = 30;

/// Where a craft was seen: coordinate seconds the light left it, and light-years.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub at_s: f64,
    pub position_ly: DVec3,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Trajectory {
    /// A conic about `about`: where the craft was relative to it at `epoch_s`, meters, and how fast,
    /// meters a second. `mu` is the body's `G m`. Any eccentricity.
    Conic { about: BodyId, mu: f64, position_m: DVec3, velocity_m_s: DVec3, epoch_s: f64 },
    /// A straight line, light-years and a fraction of c.
    Line { position_ly: DVec3, beta: DVec3, epoch_s: f64 },
}

impl Trajectory {
    /// The path through `places`, oldest first, if they share one: a conic about a body of the
    /// system where there is one, else a line. A craft drifting in a system lies on the line and
    /// no conic. `seed` is a trajectory the places are thought to lie near, which a refit over a
    /// long run needs: a velocity differenced across places hours apart is no start for one. `None`
    /// with fewer than [`FIT_PLACES`], or when no candidate has every place within `tolerance_ly`,
    /// which is what a craft under thrust gives.
    pub fn fit(system: Option<&LocalSystem>, places: &[Place], tolerance_ly: f64, seed: Option<&Trajectory>) -> Option<Trajectory> {
        if places.len() < FIT_PLACES {
            return None;
        }
        let holds = |fitted: &Trajectory| {
            places.iter().all(|p| fitted.place_at(system, p.at_s).is_some_and(|at| at.distance(p.position_ly) <= tolerance_ly))
        };
        let conics = system.into_iter().flat_map(|system| {
            let held = places.last().map(|p| system.holding(p.position_ly, p.at_s));
            std::iter::successors(held, |&k| system.sim().parent(k)).flat_map(move |about| {
                let seeded = seed.and_then(|seed| conic(system, about, places, Some(seed)));
                seeded.into_iter().chain(std::iter::once_with(move || conic(system, about, places, None)).flatten())
            })
        });
        conics.chain(line(places)).find(|fitted| holds(fitted))
    }

    /// Where the path puts the craft at coordinate time `at_s`, light-years. A conic needs the
    /// system it is in; `None` without it, or when its body is not there.
    pub fn place_at(&self, system: Option<&LocalSystem>, at_s: f64) -> Option<DVec3> {
        match *self {
            Trajectory::Line { position_ly, beta, epoch_s } => Some(position_ly + beta * ((at_s - epoch_s) / JULIAN_YEAR_S)),
            Trajectory::Conic { about, mu, position_m, velocity_m_s, epoch_s } => {
                let system = system?;
                let index = system.sim().indices().find(|&k| BodyId::of(system.star, system.sim().name(k)) == about)?;
                let (center_m, _) = system.body_state_at(index, at_s)?;
                let local = propagate(mu, position_m, velocity_m_s, at_s - epoch_s)?;
                Some(system.origin_ly + (center_m + local) / M_PER_LY)
            }
        }
    }
}

/// Where a craft on the conic through `position_m` and `velocity_m_s` is `dt_s` later, meters
/// from the body.
fn propagate(mu: f64, position_m: DVec3, velocity_m_s: DVec3, dt_s: f64) -> Option<DVec3> {
    let mut elements = state::from_state(mu, position_m, velocity_m_s)?;
    let e = elements.eccentricity;
    if (e - 1.0).abs() < PARABOLIC_TOLERANCE {
        elements.eccentricity = 1.0 + PARABOLIC_TOLERANCE.copysign(e - 1.0);
    }
    let coast = Coast { primary: String::new(), mu, elements, epoch_s: 0.0 };
    coast.local_state_at(dt_s).map(|(at, _)| at)
}

/// Least squares over the state at the middle place, by Gauss-Newton with a forward-difference
/// Jacobian. Seeded from `seed` carried to that instant, or from the places either side of it.
fn conic(system: &LocalSystem, about: em_sim::id::BodyIndex, places: &[Place], seed: Option<&Trajectory>) -> Option<Trajectory> {
    let mu = system.sim().gravitational_constant() * system.sim().mass(about);
    if mu <= 0.0 || !mu.is_finite() {
        return None;
    }
    let mut local = Vec::with_capacity(places.len());
    for p in places {
        let (center_m, _) = system.body_state_at(about, p.at_s)?;
        local.push(((p.position_ly - system.origin_ly) * M_PER_LY - center_m, p.at_s));
    }
    let mid = local.len() / 2;
    let epoch_s = local.get(mid)?.1;
    let start = match seed {
        Some(seed) => {
            let ahead = 1.0;
            let at = |t: f64| Some((seed.place_at(Some(system), t)? - system.origin_ly) * M_PER_LY - system.body_state_at(about, t)?.0);
            let here = at(epoch_s)?;
            [here, (at(epoch_s + ahead)? - here) / ahead]
        }
        None => {
            let (before, here, after) = (local.get(mid.checked_sub(1)?)?, local.get(mid)?, local.get(mid + 1)?);
            [here.0, (after.0 - before.0) / (after.1 - before.1)]
        }
    };
    let misses = |x: &[DVec3; 2]| -> Option<Vec<DVec3>> {
        local.iter().map(|(r, t)| Some(propagate(mu, x[0], x[1], t - epoch_s)? - *r)).collect()
    };
    let mut x = start;
    let mut cost: f64 = misses(&x)?.iter().map(|m| m.length_squared()).sum();
    for _ in 0..MOST_STEPS {
        let res = misses(&x)?;
        let mut columns = Vec::with_capacity(6);
        for k in 0..6 {
            let step = (x.get(k / 3)?.length() * 1e-7).max(if k < 3 { 1e-3 } else { 1e-6 });
            let mut nudged = x;
            *component(&mut nudged, k)? += step;
            let moved = misses(&nudged)?;
            columns.push(moved.iter().zip(&res).map(|(m, r)| (*m - *r) / step).collect::<Vec<DVec3>>());
        }
        let mut normal = [[0.0; 6]; 6];
        let mut rhs = [0.0; 6];
        for ((row, rhs), ci) in normal.iter_mut().zip(rhs.iter_mut()).zip(&columns) {
            for (n, cj) in row.iter_mut().zip(&columns) {
                *n = ci.iter().zip(cj).map(|(a, b)| a.dot(*b)).sum();
            }
            *rhs = -ci.iter().zip(&res).map(|(a, r)| a.dot(*r)).sum::<f64>();
        }
        let delta = solve6(normal, rhs)?;
        let mut next = x;
        for (k, d) in delta.iter().enumerate() {
            *component(&mut next, k)? += d;
        }
        let Some(after) = misses(&next).map(|m| m.iter().map(|m| m.length_squared()).sum::<f64>()) else { break };
        if !(after < cost) {
            break;
        }
        let done = cost - after <= 1e-12 * cost;
        (x, cost) = (next, after);
        if done {
            break;
        }
    }
    Some(Trajectory::Conic { about: BodyId::of(system.star, system.sim().name(about)), mu, position_m: x[0], velocity_m_s: x[1], epoch_s })
}

/// Element `k` of a position and velocity read as one six-vector.
fn component(x: &mut [DVec3; 2], k: usize) -> Option<&mut f64> {
    x.get_mut(k / 3)?.as_mut().get_mut(k % 3)
}

/// `a x = b` by Gaussian elimination with partial pivoting. `None` when `a` is singular.
#[allow(clippy::indexing_slicing)] // every index runs over 0..6 into [_; 6] arrays
fn solve6(mut a: [[f64; 6]; 6], mut b: [f64; 6]) -> Option<[f64; 6]> {
    for col in 0..6 {
        let pivot = (col..6).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if !(a[pivot][col].abs() > 0.0) {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in col + 1..6 {
            let f = a[row][col] / a[col][col];
            for k in col..6 {
                a[row][k] -= f * a[col][k];
            }
            b[row] -= f * b[col];
        }
    }
    let mut x = [0.0; 6];
    for row in (0..6).rev() {
        let tail: f64 = (row + 1..6).map(|k| a[row][k] * x[k]).sum();
        x[row] = (b[row] - tail) / a[row][row];
    }
    x.iter().all(|v| v.is_finite()).then_some(x)
}

/// Least squares of position on time.
fn line(places: &[Place]) -> Option<Trajectory> {
    let count = places.len() as f64;
    let epoch_s = places.iter().map(|p| p.at_s).sum::<f64>() / count;
    let mean = places.iter().map(|p| p.position_ly).sum::<DVec3>() / count;
    let spread: f64 = places.iter().map(|p| (p.at_s - epoch_s).powi(2)).sum();
    if !(spread > 0.0) {
        return None;
    }
    let per_s = places.iter().map(|p| (p.position_ly - mean) * (p.at_s - epoch_s)).sum::<DVec3>() / spread;
    Some(Trajectory::Line { position_ly: mean, beta: per_s * JULIAN_YEAR_S, epoch_s })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coast::Coast;
    use crate::navigation::{Course, Plane};

    fn sol() -> LocalSystem {
        let provider = crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv").expect("the catalog");
        let sun = crate::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))
            .expect("the Sun")
            .clone();
        let mut system = LocalSystem::for_star(&sun).expect("the solar system");
        system.advance_to(0.0);
        system
    }

    fn m(a: DVec3, b: DVec3) -> f64 {
        a.distance(b) * M_PER_LY
    }

    const KM_LY: f64 = 1.0e3 / M_PER_LY;

    /// A held orbit of Jupiter, five places twenty minutes apart, followed for three days.
    #[test]
    fn five_places_on_a_held_orbit_give_a_conic_that_keeps_to_it() {
        let system = sol();
        let station = Course::Orbit { body: "Jupiter".into(), altitude_radii: 0.5, plane: Plane::Equatorial }
            .resolve(&system, DVec3::ZERO, 0.0)
            .expect("an orbit");
        let places: Vec<Place> = (0..5)
            .map(|k| {
                let at_s = 1.0e5 + 1200.0 * k as f64;
                Place { at_s, position_ly: station.place_at(&system, at_s).unwrap() }
            })
            .collect();
        let fitted = Trajectory::fit(Some(&system), &places, KM_LY, None).expect("a conic");
        assert!(matches!(fitted, Trajectory::Conic { .. }), "{fitted:?}");
        let later = 1.0e5 + 3.0 * 86_400.0;
        let off = m(fitted.place_at(Some(&system), later).unwrap(), station.place_at(&system, later).unwrap());
        assert!(off < 1.0e4, "off by {off} m after three days");
    }

    /// An eccentric arc and an escape: nothing here assumes the path is a circle.
    #[test]
    fn an_ellipse_and_a_hyperbola_are_fitted_as_what_they_are() {
        let system = sol();
        let earth = system.body_named("Earth").unwrap();
        let (center, carried) = system.body_state_at(earth, 0.0).unwrap();
        let mu = system.sim().gravitational_constant() * system.sim().mass(earth);
        let r0 = 7.0e6;
        for (speed, hyperbolic) in [(1.25, false), (1.6, true)] {
            let circular = (mu / r0).sqrt();
            let at_ly = system.origin_ly + (center + DVec3::X * r0) / M_PER_LY;
            let velocity = carried + DVec3::Y * circular * speed;
            let coast = Coast::from_state(&system, at_ly, velocity, 0.0).expect("a coast");
            let places: Vec<Place> = (0..6)
                .map(|k| {
                    let at_s = 300.0 * k as f64;
                    Place { at_s, position_ly: coast.at(&system, at_s).unwrap().0 }
                })
                .collect();
            let fitted = Trajectory::fit(Some(&system), &places, KM_LY, None).expect("a conic");
            let Trajectory::Conic { mu, position_m, velocity_m_s, .. } = fitted else { panic!("{fitted:?}") };
            let eccentricity = state::from_state(mu, position_m, velocity_m_s).unwrap().eccentricity;
            assert_eq!(eccentricity > 1.0, hyperbolic, "e = {eccentricity}");
            let later = 4.0 * 3600.0;
            let off = m(fitted.place_at(Some(&system), later).unwrap(), coast.at(&system, later).unwrap().0);
            assert!(off < 1.0e4, "speed {speed}: off by {off} m four hours on");
        }
    }

    /// Craft 7 on the dev shard: coasting round the star at 1.8 AU, a few millionths of a turn
    /// between places. A short arc's fit holds for a day; refitted over three days of refiles,
    /// seeded with it, it holds for a month.
    #[test]
    fn a_coast_round_the_star_is_held_from_a_short_arc_and_then_a_run() {
        let system = sol();
        let sun = system.primary();
        let mu = system.sim().gravitational_constant() * system.sim().mass(sun);
        let (center, carried) = system.body_state_at(sun, 0.0).unwrap();
        let r0 = 1.8 * 1.495_978_707e11;
        let at_ly = system.origin_ly + (center + DVec3::new(r0, 0.0, 0.0)) / M_PER_LY;
        let velocity = carried + DVec3::new(2.0e3, (mu / r0).sqrt() * 0.97, 300.0);
        let coast = Coast::about(&system, sun, at_ly, velocity, 0.0).expect("a coast about the Sun");
        let seen = |t: f64| Place { at_s: t, position_ly: coast.at(&system, t).unwrap().0 };
        let truth = |t: f64| coast.at(&system, t).unwrap().0;

        let short: Vec<Place> = (0..5).map(|k| seen(600.0 * k as f64)).collect();
        let first = Trajectory::fit(Some(&system), &short, KM_LY, None).expect("a conic from forty minutes");
        let day = 86_400.0;
        let off = m(first.place_at(Some(&system), day).unwrap(), truth(day));
        assert!(off < 1.0e3, "the short arc is off by {off} m a day on");

        let run: Vec<Place> = short.iter().copied().chain((1..=12).map(|k| seen(21_600.0 * k as f64))).collect();
        let refit = Trajectory::fit(Some(&system), &run, KM_LY, Some(&first)).expect("a conic over the run");
        let month = 30.0 * day;
        let off = m(refit.place_at(Some(&system), month).unwrap(), truth(month));
        assert!(off < 1.0e3, "the run is off by {off} m a month on");
    }

    /// Places under thrust lie on no conic, and are refused rather than fitted loosely.
    #[test]
    fn places_under_thrust_fit_nothing() {
        let system = sol();
        let earth = system.body_named("Earth").unwrap();
        let places: Vec<Place> = (0..6)
            .map(|k| {
                let at_s = 300.0 * k as f64;
                let (center, _) = system.body_state_at(earth, at_s).unwrap();
                let pushed = DVec3::X * (7.0e6 + 0.5 * 10.0 * at_s * at_s);
                Place { at_s, position_ly: system.origin_ly + (center + pushed) / M_PER_LY }
            })
            .collect();
        assert_eq!(Trajectory::fit(Some(&system), &places, KM_LY, None), None);
    }

    /// A coast solved about Jupiter, seen from inside Io's sphere: the conic about Io, which
    /// holds the craft by sphere, drifts off it; the one about Jupiter is the path.
    #[test]
    fn a_coast_is_fitted_about_the_body_it_goes_round_not_the_sphere_it_is_in() {
        let system = sol();
        let jupiter = system.body_named("Jupiter").unwrap();
        let io = system.body_named("Io").unwrap();
        let (io_at, io_v) = system.body_state_at(io, 0.0).unwrap();
        let (j_at, j_v) = system.body_state_at(jupiter, 0.0).unwrap();
        let mu = system.sim().gravitational_constant() * system.sim().mass(jupiter);
        let r = io_at - j_at;
        let at_ly = system.origin_ly + (io_at + r.normalize() * 2.0e6) / M_PER_LY;
        let speed = (mu / (r.length() + 2.0e6)).sqrt();
        let velocity = j_v + (io_v - j_v).normalize() * speed * 1.02;
        let coast = Coast::about(&system, jupiter, at_ly, velocity, 0.0).expect("a coast about Jupiter");
        let places: Vec<Place> =
            (0..6).map(|k| Place { at_s: 600.0 * k as f64, position_ly: coast.at(&system, 600.0 * k as f64).unwrap().0 }).collect();
        assert_eq!(system.holding(places[5].position_ly, 3000.0), io, "premise: Io's sphere holds it");
        let fitted = Trajectory::fit(Some(&system), &places, KM_LY, None).expect("a conic");
        let Trajectory::Conic { about, .. } = fitted else { panic!("{fitted:?}") };
        assert_eq!(about, BodyId::of(system.star, "Jupiter"));
    }

    /// Drifting is a straight line wherever it happens, a system included. Over minutes a conic
    /// can pass for a line; over days only the line holds.
    #[test]
    fn a_craft_drifting_in_a_system_is_fitted_with_a_line() {
        let system = sol();
        let start = system.origin_ly + DVec3::new(3.0e11, 0.0, 0.0) / M_PER_LY;
        let places: Vec<Place> =
            (0..5).map(|k| Place { at_s: 86_400.0 * k as f64, position_ly: start + DVec3::Y * 1e-7 * k as f64 }).collect();
        let fitted = Trajectory::fit(Some(&system), &places, KM_LY, None).expect("a line");
        assert!(matches!(fitted, Trajectory::Line { .. }), "{fitted:?}");
    }

    #[test]
    fn between_systems_the_path_is_a_line() {
        let places: Vec<Place> =
            (0..5).map(|k| Place { at_s: 1.0e6 * k as f64, position_ly: DVec3::new(3.0, 0.0, 0.0) + DVec3::Y * 1e-4 * k as f64 }).collect();
        let fitted = Trajectory::fit(None, &places, 1e-12, None).expect("a line");
        let at = fitted.place_at(None, 1.0e7).unwrap();
        assert!(at.distance(DVec3::new(3.0, 1e-3, 0.0)) < 1e-12, "{at}");
        assert!(Trajectory::fit(None, &places[..4], 1e-12, None).is_none(), "four places are not enough");
    }
}
