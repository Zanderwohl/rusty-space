//! Whether a new look lands where the held orbit said it would.
//!
//! A fit's covariance only knows the noise in the looks it was fitted to. An orbit that is not
//! a fixed ellipse -- the arena's elements precess, Luna's node by 19 degrees a year -- agrees
//! with a Kepler fit to the noise over a short arc and leaves it later, and nothing in the fit
//! can see that. Each new look can: the orbit predicts where the body will be and how well, and
//! a look many sigma from that says the orbit is wrong, however confident it is. See
//! `lightcone/docs/25-system-knowledge.md`.

use glam::{DMat2, DMat3, DVec2, DVec3};

use super::{Placed, Sighting, Subject};

/// Sigma past which a look is taken as the orbit being wrong. Two degrees of freedom, so an
/// honest orbit lands this far out about once in 270,000 looks.
pub const SURPRISING: f64 = 5.0;

/// How far a look landed from where a placement said, in sigma, from the placement's error and
/// the bearing's together. `None` where the placement cannot say: it has no known place, or its
/// phase is anywhere on the orbit.
///
/// `from_m` is the observer from the placement's origin, and `origin_m2` that origin's own
/// covariance, meters squared: a place is only as good as the star it is measured from. The two
/// components across the line of sight are tested together, so this is a Mahalanobis distance:
/// an orbit known well across its path and poorly along it is not surprised by a look that is
/// off along it.
pub fn surprise(placed: Placed, from_m: DVec3, origin_m2: DMat3, toward: DVec3, sigma_rad: f64) -> Option<f64> {
    let Placed::Known { offset_au, error } = placed else { return None };
    if error.anywhere_on_orbit() {
        return None;
    }
    let au = crate::navigation::AU;
    let offset = offset_au * au - from_m;
    let range = offset.length();
    if !super::arc::sound(range) {
        return None;
    }
    // The phase linearized: fine until the along bar is a good part of a turn, where the
    // prediction is a banana and this is only an estimate of it.
    let place = error.whole_au2() * (au * au) + origin_m2;
    let (x, y) = toward.any_orthonormal_pair();
    let seen = |a: DVec3, b: DVec3| a.dot(place * b) / (range * range);
    let bearing = sigma_rad * sigma_rad;
    let s = DMat2::from_cols(
        DVec2::new(seen(x, x) + bearing, seen(x, y)),
        DVec2::new(seen(x, y), seen(y, y) + bearing),
    );
    if !super::arc::sound(s.determinant()) {
        return None;
    }
    let predicted = offset / range;
    let miss = DVec2::new(predicted.dot(x), predicted.dot(y));
    let d2 = miss.dot(s.inverse() * miss);
    d2.is_finite().then(|| d2.max(0.0).sqrt())
}

impl super::Knowledge {
    /// How far `seen` lands from where this craft's beliefs put its body, in sigma. `None` for
    /// anything but a body, or a body or star not placed.
    pub fn surprised_by(&self, subject: Subject, seen: &Sighting) -> Option<f64> {
        let Subject::Body { star, body } = subject else { return None };
        let super::Distance::Measured { position_ly: star_ly, sigma_ly } = self.belief(Subject::Star(star))?.distance
        else {
            return None;
        };
        let placed = self.body_belief(star, body, seen.observed_s)?.position_now;
        let from_m = (seen.bearing.observer_ly - star_ly) * crate::system::M_PER_LY;
        // The star is re-measured as the survey goes, and every orbit about it moves with it.
        // Its parallax fixes it well across the line of sight and poorly along it: from 5 AU a
        // few hundred kilometers, which left without made a right orbit of Mercury's hundreds of
        // sigma out.
        let depth = from_m.normalize_or_zero() * sigma_ly * crate::system::M_PER_LY;
        let origin = DMat3::from_cols(depth * depth.x, depth * depth.y, depth * depth.z);
        surprise(placed, from_m, origin, seen.bearing.toward, seen.bearing.sigma_rad)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::arc::{self, Fitted, Look};
    use crate::knowledge::placed::placed_at;
    use crate::rng;
    use em_foundations::kepler;
    use glam::DQuat;

    const AU_M: f64 = 1.495_978_707e11;
    const MU_SUN: f64 = 1.327_124_4e20;
    const DAY_S: f64 = 86_400.0;
    const YEAR_S: f64 = 365.25 * DAY_S;
    const SIGMA: f64 = 2.979e-7 * crate::knowledge::astrometry::CENTROID_FLOOR;

    /// A Mars-like orbit whose node and periapsis turn at the given rates, radians a second.
    struct Precessing {
        at_rest: Fitted,
        node_rate: f64,
        apsidal_rate: f64,
    }

    impl Precessing {
        fn at(&self, t: f64) -> DVec3 {
            let turned = Fitted { periapsis_rad: self.at_rest.periapsis_rad + self.apsidal_rate * t, ..self.at_rest };
            DQuat::from_rotation_z(self.node_rate * t) * turned.at(t)
        }
    }

    fn mars(node_period_s: f64, apsidal_period_s: f64) -> Precessing {
        let semi_major_m = 1.524 * AU_M;
        let at_rest = Fitted {
            semi_major_m,
            eccentricity: 0.0934,
            period_s: kepler::period::third_law(semi_major_m, MU_SUN),
            pole: DVec3::new(0.0, (5.0f64).to_radians().sin(), (5.0f64).to_radians().cos()),
            periapsis_rad: 0.0,
            epoch_s: 4.0e6,
            mu: MU_SUN,
            reach_m: f64::INFINITY,
            assumed_circular: false,
            residual_rad: 0.0,
            looks: 0,
        };
        let at_rest = Fitted {
            periapsis_rad: crate::knowledge::arc::tests::fixed_periapsis(at_rest.pole, 1.8),
            ..at_rest
        };
        let rate = |period: f64| if period.is_finite() { std::f64::consts::TAU / period } else { 0.0 };
        Precessing { at_rest, node_rate: rate(node_period_s), apsidal_rate: rate(apsidal_period_s) }
    }

    /// Looks from a ship on a 5 AU circle at each of `times`.
    fn looks(place: &dyn Fn(f64) -> DVec3, times: &[f64]) -> Vec<Look> {
        seeded(place, times, 0, true)
    }

    /// `orbiting` false holds the ship still at 5 AU, as a survey from a station does: nothing
    /// then gives the depth but the orbit's own curvature.
    fn seeded(place: &dyn Fn(f64) -> DVec3, times: &[f64], seed: u64, orbiting: bool) -> Vec<Look> {
        let ship_period = if orbiting { kepler::period::third_law(5.0 * AU_M, MU_SUN) } else { f64::INFINITY };
        times
            .iter()
            .enumerate()
            .map(|(i, &t)| {
                let phase = std::f64::consts::TAU * t / ship_period;
                let from = DVec3::new(phase.cos(), phase.sin(), 0.0) * 5.0 * AU_M;
                let toward = (place(t) - from).normalize();
                let (x, y) = toward.any_orthonormal_pair();
                let nudge = x * rng::gaussian(rng::hash(&[seed, i as u64, 61])) * SIGMA
                    + y * rng::gaussian(rng::hash(&[seed, i as u64, 62])) * SIGMA;
                Look { from_m: from, toward: (toward + nudge).normalize(), at_s: t, sigma_rad: SIGMA, range_m: None }
            })
            .collect()
    }

    /// The largest surprise over `later` looks against an orbit fitted to `first` looks.
    fn worst(truth: &Precessing, first: &[f64], later: &[f64]) -> f64 {
        let fitted = arc::fit(&looks(&|t| truth.at(t), first)).expect("the first arc fits");
        let orbit = fitted.stated(crate::knowledge::Witness(1), None, &looks(&|t| truth.at(t), first), 0.0);
        looks(&|t| truth.at(t), later)
            .iter()
            .filter_map(|look| surprise(placed_at(&orbit, look.at_s), look.from_m, DMat3::ZERO, look.toward, look.sigma_rad))
            .fold(0.0, f64::max)
    }

    fn every(from_s: f64, to_s: f64, count: usize) -> Vec<f64> {
        (0..count).map(|i| from_s + (to_s - from_s) * i as f64 / (count - 1) as f64).collect()
    }

    /// **An orbit that is right is never surprised.** A Kepler orbit, fitted over a fifth of
    /// itself and tested at forty looks over the next two years, stays under [`SURPRISING`].
    #[test]
    fn a_kepler_orbit_is_never_surprised() {
        let truth = mars(f64::INFINITY, f64::INFINITY);
        let span = truth.at_rest.period_s * 0.2;
        let worst = worst(&truth, &every(0.0, span, 24), &every(span, span + 2.0 * YEAR_S, 40));
        assert!(worst < SURPRISING, "a right orbit was {worst} sigma out");
    }

    /// **The surprise is a chi-square, so the threshold means what it says.** Over seeded noise,
    /// later looks at a right orbit land at a squared surprise averaging two, the two degrees of
    /// freedom across the line of sight. Too wide an error and they average far less, which is
    /// a test that never fires; too narrow and it fires on orbits that are right.
    ///
    /// From a ship holding still, which is where each element's own sigma failed: the depth is
    /// poorly known, and those sigmas, treated as independent, spread it across the line of
    /// sight too. On Sol that left 1,246 looks at 35 bodies all under one sigma.
    #[test]
    fn a_right_orbit_is_surprised_as_the_noise_says() {
        use crate::knowledge::settle::settle;
        let truth = mars(f64::INFINITY, f64::INFINITY);
        let span = truth.at_rest.period_s * 0.2;
        let mut squares = Vec::new();
        for seed in 0..40 {
            let first = seeded(&|t| truth.at_rest.at(t), &every(0.0, span, 24), seed, false);
            let fitted = settle(truth.at_rest, &first, 64);
            let orbit = fitted.stated(crate::knowledge::Witness(1), None, &first, 0.0);
            let later = seeded(&|t| truth.at_rest.at(t), &every(span, span + YEAR_S, 6), seed + 1000, false);
            for look in later {
                let d = surprise(placed_at(&orbit, look.at_s), look.from_m, DMat3::ZERO, look.toward, look.sigma_rad).expect("tested");
                squares.push(d * d);
            }
        }
        let mean = squares.iter().sum::<f64>() / squares.len() as f64;
        assert!((1.4..2.8).contains(&mean), "a right orbit's squared surprise averages {mean}, not two");
    }

    /// **An orbit that precesses is caught.** Its node turning once in a hundred thousand years:
    /// at the telescope's precision that is far from nothing, and the fit over a fifth of the
    /// orbit already misses by seventy times the noise, which its bars take in. What they cannot
    /// take in is where the precession goes next, and within half a year a look is past
    /// [`SURPRISING`]. Without precession the same fit, tested the same way, is not.
    #[test]
    fn a_precessing_orbit_is_caught() {
        let truth = mars(1.0e5 * YEAR_S, 7.0e4 * YEAR_S);
        let span = truth.at_rest.period_s * 0.2;
        let worst = worst(&truth, &every(0.0, span, 24), &every(span + 150.0 * DAY_S, span + 4.0 * YEAR_S, 10));
        assert!(worst > SURPRISING, "a precessing orbit was only {worst} sigma out");
    }

    /// **A fit that cannot fit is not sure of itself.** A Kepler fit to an orbit that precesses
    /// sits seventy times the noise, and that misfit is the model's: smooth from look to look,
    /// so it does not average down over them as noise does. Its bars cover where the body is
    /// when it was fitted, which with the misfit treated as noise they missed by eight. Where
    /// the precession takes it later is the innovation test's to catch.
    #[test]
    fn a_misfit_widens_the_bars_as_a_model_error() {
        let truth = mars(1.0e5 * YEAR_S, 7.0e4 * YEAR_S);
        let span = truth.at_rest.period_s * 0.2;
        let first = looks(&|t| truth.at(t), &every(0.0, span, 24));
        let fitted = arc::fit(&first).expect("fits");
        assert!(fitted.residual_rad > 20.0 * SIGMA, "premise: the model cannot fit it");
        let orbit = fitted.stated(crate::knowledge::Witness(1), None, &first, 0.0);
        let Placed::Known { offset_au, error } = placed_at(&orbit, span) else { panic!("placed") };
        let au = crate::navigation::AU;
        let miss = offset_au * au - truth.at(span);
        let z = miss.dot((error.whole_au2() * (au * au)).inverse() * miss).sqrt();
        assert!(z < 3.0, "the end of the arc is {z} of its bars out");
    }

    /// **A place is only as good as the star it is measured from.** The survey re-measures the
    /// star as it goes, and its parallax fixes it poorly along the line of sight: on Sol from
    /// 5 AU, 3,800 km. A look displaced by twice that alone is no surprise to a right orbit once
    /// the star's error is counted, and hundreds of sigma without it, which is what refitted a
    /// right orbit of Mercury until a search from scratch put it about Earth.
    #[test]
    fn the_stars_own_error_is_no_surprise() {
        let truth = mars(f64::INFINITY, f64::INFINITY);
        let span = truth.at_rest.period_s * 0.2;
        let first = looks(&|t| truth.at_rest.at(t), &every(0.0, span, 24));
        let orbit = arc::fit(&first).expect("fits").stated(crate::knowledge::Witness(1), None, &first, 0.0);
        let sigma_m = 3.8e6;
        // The star believed `2 sigma` further off along the ship's line of sight to it.
        let from = DVec3::new(5.0 * AU_M, 0.0, 0.0);
        let shift = -from.normalize() * 2.0 * sigma_m;
        let t = span + 10.0 * DAY_S;
        let toward = (truth.at_rest.at(t) + shift - from).normalize();
        let depth = from.normalize() * sigma_m;
        let origin = DMat3::from_cols(depth * depth.x, depth * depth.y, depth * depth.z);
        let placed = placed_at(&orbit, t);
        let counted = surprise(placed, from, origin, toward, SIGMA).expect("tested");
        let ignored = surprise(placed, from, DMat3::ZERO, toward, SIGMA).expect("tested");
        assert!(counted < SURPRISING, "the star's own error surprised a right orbit: {counted}");
        assert!(ignored > SURPRISING, "premise: without it the look is a surprise, got {ignored}");
    }

    /// Along the path an orbit is known worst, so a look off along it is less surprising than
    /// one off across it by the same angle.
    #[test]
    fn a_miss_along_the_path_is_the_least_surprising() {
        let truth = mars(f64::INFINITY, f64::INFINITY);
        let first = looks(&|t| truth.at_rest.at(t), &every(0.0, truth.at_rest.period_s * 0.2, 24));
        let orbit = arc::fit(&first).expect("fits").stated(crate::knowledge::Witness(1), None, &first, 0.0);
        let t = truth.at_rest.period_s * 0.6;
        let placed = placed_at(&orbit, t);
        let Placed::Known { offset_au, error } = placed else { panic!("placed") };
        let from = DVec3::new(5.0 * AU_M, 0.0, 0.0);
        let at = offset_au * crate::navigation::AU;
        let toward = (at - from).normalize();
        let angle = 1.0e-7;
        let off = |direction: DVec3| {
            let moved = (at + direction.normalize() * angle * (at - from).length() - from).normalize();
            surprise(placed, from, DMat3::ZERO, moved, SIGMA).expect("tested")
        };
        let along = error.pace_au.normalize();
        let across = error.pole;
        assert!(toward.dot(along).abs() < 0.9 && toward.dot(across).abs() < 0.9, "premise: seen side on");
        assert!(off(along) < off(across), "along {} against across {}", off(along), off(across));
    }

    /// **A surprise puts a body back in the fit queue at once.**
    /// The whole chain: bearings filed, an orbit fitted and filed, and then a look that agrees
    /// with it, which leaves the body waiting for its arc to grow, and one that does not, which
    /// does not wait.
    #[test]
    fn a_surprise_refits_at_once() {
        use crate::knowledge::{Bearing, BodyId, Knowledge, Sighting, Witness};
        use crate::sky::StarId;

        let star = StarId::synthesize("innovation", 1);
        let star_ly = DVec3::new(3.0, -1.0, 0.5);
        let body = BodyId::of(star, "Kettle");
        let subject = Subject::Body { star, body };
        let mut k = Knowledge::new(Witness(7));
        let sighting = |look: &Look| Sighting {
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
        };
        // The star from round the ship's orbit, which is what places it.
        for i in 0..8 {
            let from = DQuat::from_rotation_z(i as f64 * 0.7) * DVec3::X * 5.0 * AU_M;
            let look = Look { from_m: from, toward: -from.normalize(), at_s: i as f64, sigma_rad: 1.0e-9, range_m: None };
            k.sighted(Subject::Star(star), sighting(&look));
        }
        assert!(k.belief(Subject::Star(star)).and_then(|b| b.distance.position_ly()).is_some(), "premise: the star is placed");

        let truth = mars(f64::INFINITY, f64::INFINITY);
        let span = truth.at_rest.period_s * 0.2;
        for look in looks(&|t| truth.at_rest.at(t), &every(0.0, span, 24)) {
            k.sighted(subject, sighting(&look));
        }
        assert!(k.fit_orbit(subject, star_ly, span), "the arc fits");
        assert_eq!(k.unfitted(star), None, "premise: fitted, and nothing due");

        // A look where the orbit says: nothing is due.
        let later = span + 30.0 * DAY_S;
        let agreeing = looks(&|t| truth.at_rest.at(t), &[later])[0];
        let seen = sighting(&agreeing);
        assert!(k.surprised_by(subject, &seen).expect("tested") < SURPRISING);
        k.sighted(subject, seen);
        assert_eq!(k.unfitted(star), None, "an agreeing look re-armed the fit");

        // A look a thousandth of an AU off the orbit, which it is sure of: due now.
        let moved = |t: f64| truth.at_rest.at(t) + DVec3::Z * 1.0e-3 * AU_M;
        let disagreeing = looks(&moved, &[later + DAY_S])[0];
        assert!(k.surprised_by(subject, &sighting(&disagreeing)).expect("tested") > SURPRISING);
        k.sighted(subject, sighting(&disagreeing));
        assert_eq!(k.unfitted(star), Some(subject), "a surprise waited for the arc to grow");
        k.fit_job(subject, star_ly, later + DAY_S).expect("a job");
        assert_eq!(k.unfitted(star), None, "and the surprise is spent on that one attempt");
    }
}
