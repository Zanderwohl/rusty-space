//! What a body probably weighs, from the best evidence held: its own mass where a satellite has
//! weighed it, else its measured size at the density bodies of that size have, else a size read
//! off its brightness.
//!
//! A guess for drawing and ranking, never a measurement: nothing here is filed, and a reader
//! wanting the mass reads [`super::BodyBelief::mass_kg`].

use em_spectra::Band;

use super::{BodyBelief, Knowledge, Placed, Subject};
use crate::sky::generate::disc::EARTH_RADIUS;
use crate::star::Star;

/// Geometric albedo taken for a body nothing has resolved. Rock is darker and cloud and ice are
/// brighter, so a size read through this is good to a factor of two or so in mass, which is the
/// precision a mark's size has anyway.
const ALBEDO: f64 = 0.3;

/// Bulk density by size, Earth radii against kg/m^3, interpolated in log-log: rubble piles, the
/// Moon, the Earth, Neptune, Jupiter.
const DENSITY: [(f64, f64); 5] = [(0.05, 2000.0), (0.27, 3340.0), (1.0, 5510.0), (3.9, 1640.0), (11.2, 1330.0)];

impl Knowledge {
    /// What `belief`'s body probably weighs, kilograms, or `None` with nothing to go on. `star`
    /// is its system's star, whose light a brightness is read against.
    pub fn guessed_mass_kg(&self, belief: &BodyBelief, star: &Star) -> Option<f64> {
        if let Some((kg, _)) = belief.mass_kg.filter(|(kg, _)| kg.is_finite() && *kg > 0.0) {
            return Some(kg);
        }
        let radius_m = belief
            .radius_m
            .map(|(r, _)| r)
            .filter(|r| r.is_finite() && *r > 0.0)
            .or_else(|| self.radius_from_light(belief, star))?;
        Some(mass_at(radius_m))
    }

    /// The radius that reflects the light last seen from the body, at [`ALBEDO`]. Needs where it
    /// was when that light was seen, from its orbit, and where the star is.
    fn radius_from_light(&self, belief: &BodyBelief, star: &Star) -> Option<f64> {
        let Subject::Body { star: star_id, body } = belief.subject else { return None };
        let seen = self
            .file(belief.subject)?
            .sightings()
            .iter()
            .filter(|s| s.flux.is_finite() && s.flux > 0.0)
            .max_by(|a, b| a.observed_s.total_cmp(&b.observed_s))?;
        let Placed::Known { offset_au, .. } = self.body_belief(star_id, body, seen.observed_s)?.position_now else {
            return None;
        };
        let star_ly = self.belief(Subject::Star(star_id))?.distance.position_ly()?;
        let from_star_m = offset_au.length() * crate::navigation::AU;
        let body_ly = star_ly + offset_au * (crate::navigation::AU / crate::system::M_PER_LY);
        let range_m = body_ly.distance(seen.bearing.observer_ly) * crate::system::M_PER_LY;
        radius_reflecting(star, seen.band, seen.flux, from_star_m, range_m)
    }
}

/// The radius of a body at [`ALBEDO`] that returns `flux` from `from_star_m` out, seen from
/// `range_m` away. Full phase: a crescent reads smaller than it is.
fn radius_reflecting(star: &Star, band: Band, flux: f64, from_star_m: f64, range_m: f64) -> Option<f64> {
    let incident = super::survey::flux_from(star, band, from_star_m);
    let radius = range_m * (flux / (ALBEDO * incident)).sqrt();
    (radius.is_finite() && radius > 0.0).then_some(radius)
}

fn mass_at(radius_m: f64) -> f64 {
    4.0 / 3.0 * std::f64::consts::PI * radius_m.powi(3) * density_at(radius_m / EARTH_RADIUS)
}

fn density_at(radius_earths: f64) -> f64 {
    let x = radius_earths.max(f64::MIN_POSITIVE).ln();
    let points = DENSITY.map(|(r, rho)| (r.ln(), rho.ln()));
    let (first, last) = (points[0], points[points.len() - 1]);
    if x <= first.0 {
        return first.1.exp();
    }
    if x >= last.0 {
        return last.1.exp();
    }
    points
        .windows(2)
        .find_map(|pair| {
            let (a, b) = (pair[0], pair[1]);
            (x <= b.0).then(|| (a.1 + (b.1 - a.1) * (x - a.0) / (b.0 - a.0)).exp())
        })
        .unwrap_or(last.1.exp())
}

#[cfg(test)]
mod tests {
    use super::*;

    const AU: f64 = crate::navigation::AU;

    fn within(guess: f64, truth: f64, factor: f64) -> bool {
        guess / truth < factor && truth / guess < factor
    }

    /// A radius alone lands near the real mass across the sizes a survey meets.
    #[test]
    fn a_radius_gives_about_the_mass() {
        for (name, radius_m, kg) in [
            ("Earth", 6.371e6, 5.972e24),
            ("the Moon", 1.737e6, 7.342e22),
            ("Jupiter", 6.991e7, 1.898e27),
            ("Neptune", 2.462e7, 1.024e26),
            ("Ceres", 4.73e5, 9.39e20),
        ] {
            let guess = mass_at(radius_m);
            assert!(within(guess, kg, 1.6), "{name}: {guess:.3e} against {kg:.3e}");
        }
    }

    /// Jupiter's brightness reads as roughly Jupiter. Its real albedo is 0.54, so it reads large.
    #[test]
    fn a_brightness_gives_about_the_size() {
        let jupiter_m = 6.991e7;
        let (from_star_m, range_m) = (5.2 * AU, 5.0 * AU);
        let flux = 0.54 * super::super::survey::flux_from(&Star::SOL, Band::V, from_star_m) * (jupiter_m / range_m).powi(2);
        let radius = radius_reflecting(&Star::SOL, Band::V, flux, from_star_m, range_m).expect("a size");
        assert!(within(radius, jupiter_m, 1.5), "{radius:.3e}");
    }

    /// A mass a satellite weighed beats a radius, and a radius beats nothing at all.
    #[test]
    fn the_best_evidence_held_is_the_one_used() {
        use crate::knowledge::{BodyId, Orientation, Witness};
        let star = crate::sky::StarId::synthesize("mass", 1);
        let body = BodyId::of(star, "x");
        let bare = BodyBelief {
            subject: Subject::Body { star, body },
            body,
            given: None,
            designation: None,
            kind: Vec::new(),
            period_s: None,
            semi_major_au: None,
            orientation: Orientation::Unknown,
            method: None,
            position_now: Placed::Unknown,
            radius_m: None,
            spin_s: None,
            velocity_m_s: None,
            about: None,
            colors: None,
            mass_kg: None,
            stated_by: None,
            hops: 0,
        };
        let k = Knowledge::new(Witness(1));
        assert_eq!(k.guessed_mass_kg(&bare, &Star::SOL), None, "nothing to go on");
        let sized = BodyBelief { radius_m: Some((6.371e6, 1.0e4)), ..bare.clone() };
        assert!(within(k.guessed_mass_kg(&sized, &Star::SOL).unwrap(), 5.972e24, 1.2));
        let weighed = BodyBelief { mass_kg: Some((1.0e20, 1.0e18)), ..sized };
        assert_eq!(k.guessed_mass_kg(&weighed, &Star::SOL), Some(1.0e20), "the weighing, not the size");
    }

    #[test]
    fn nothing_seen_is_no_size() {
        assert!(radius_reflecting(&Star::SOL, Band::V, 0.0, AU, AU).is_none());
    }
}
