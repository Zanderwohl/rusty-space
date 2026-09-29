//! What a craft inside a system believes about its star, each value with one sigma. Derived from
//! records the craft already holds and never stored, so a relayed report characterizes a star for
//! a craft that was never there. The routes are in `lightcone/docs/20-solar-power.md` §Parking.

use em_spectra::blackbody::{self, SIGMA};
use em_spectra::stellar::{SOLAR_LUMINOSITY, SOLAR_MU};
use em_spectra::Band;
use glam::DVec3;

use super::record::{Colors, Method};
use super::{Distance, File, Knowledge, Subject};
use crate::sky::StarId;
use crate::system::M_PER_LY;

/// A value and one sigma, in the same unit.
pub type Measured = (f64, f64);

/// The main-sequence mass–luminosity relation's scatter, as a fraction of the mass.
const MAIN_SEQUENCE_SCATTER: f64 = 0.3;
const MASS_LUMINOSITY_POWER: f64 = 3.5;

/// The temperatures a fit searches, K: the generator makes nothing outside them.
const COOLEST_K: f64 = 1_500.0;
const HOTTEST_K: f64 = 60_000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Host {
    /// From where the craft is, m.
    pub distance_m: Option<Measured>,
    pub radius_m: Option<Measured>,
    pub teff_k: Option<Measured>,
    /// Bolometric, W.
    pub luminosity_w: Option<Measured>,
    pub mass_kg: Option<Measured>,
}

impl Host {
    pub fn luminosity_fraction(&self) -> Option<f64> {
        self.luminosity_w.map(|(l, s)| s / l).filter(|f| f.is_finite())
    }

    /// Combined with `4πd²F` for a flux read where the craft is, W/m², whose only error is the
    /// distance's.
    pub fn with_reading(self, flux_w_m2: f64) -> Host {
        let Some((d, sd)) = self.distance_m.filter(|_| flux_w_m2 > 0.0) else { return self };
        let l = 4.0 * std::f64::consts::PI * d * d * flux_w_m2;
        let read = (l, l * 2.0 * sd / d);
        let luminosity_w = combine(&[Some(read), self.luminosity_w].into_iter().flatten().collect::<Vec<_>>());
        Host { luminosity_w: luminosity_w.or(Some(read)), ..self }
    }
}

impl Knowledge {
    pub fn host(&self, star: StarId, here_ly: DVec3) -> Host {
        let file = self.file(Subject::Star(star));
        let distance_m = self.belief(Subject::Star(star)).and_then(|b| match b.distance {
            Distance::Measured { position_ly, sigma_ly } => {
                let d = position_ly.distance(here_ly);
                (d > 0.0).then_some((d * M_PER_LY, sigma_ly * M_PER_LY))
            }
            _ => None,
        });
        let measured_radius = file.and_then(super::body::measured_radius);
        let teff_k = file.and_then(|f| best_colors(f.colors())).and_then(temperature_k);

        let mut routes = Vec::new();
        if let (Some(r), Some(t)) = (measured_radius, teff_k) {
            routes.push(from_radius(r, t));
        }
        if let (Some(t), Some((flux, range))) = (teff_k, file.and_then(ranged_flux)) {
            routes.extend(from_flux(flux, range, t));
        }
        let weighed = self.weighed_star(star);
        let luminosity_w = combine(&routes).or_else(|| weighed.map(luminosity_of_mass));
        let radius_m = measured_radius.or_else(|| {
            let ((l, sl), (t, st)) = (luminosity_w?, teff_k?);
            let r = em_spectra::stellar::radius_from_luminosity(l, t);
            Some((r, r * (0.5 * sl / l).hypot(2.0 * st / t)))
        });
        let mass_kg = weighed.or_else(|| luminosity_w.map(mass_of_luminosity));
        Host { distance_m, radius_m, teff_k, luminosity_w, mass_kg }
    }

    /// The tightest `n²a³` of the planets fitted about the star. Only a fit to bearings counts: a
    /// transit's axis was divided by a mass to begin with, and a claim measured nothing.
    fn weighed_star(&self, star: StarId) -> Option<Measured> {
        self.members(star)
            .filter_map(|(_, file)| {
                let orbit = file
                    .orbits()
                    .iter()
                    .filter(|o| o.about.is_none() && o.method == Method::Astrometric)
                    .max_by(|a, b| a.stated_s.total_cmp(&b.stated_s))?;
                super::body::mass_of(orbit)
            })
            .min_by(|a, b| (a.1 / a.0).total_cmp(&(b.1 / b.0)))
    }
}

/// The digest a fit trusts most: of those with two bands or more, the most visits in its
/// least-visited band.
fn best_colors(colors: &[Colors]) -> Option<&Colors> {
    colors
        .iter()
        .filter_map(|c| {
            let visited: Vec<u32> = Band::ALL.iter().map(|b| c.visits[*b]).filter(|v| *v > 0).collect();
            (visited.len() >= 2).then(|| (visited.into_iter().min().unwrap_or(0), c))
        })
        .max_by_key(|(fewest, _)| *fewest)
        .map(|(_, c)| c)
}

/// The tightest-ranged look's band flux (W/m², sigma, band) and its own range (m, sigma). The range
/// must be the same look's: a flux read against where the craft is now is off by the square of how
/// far it moved.
fn ranged_flux(file: &File) -> Option<((f64, f64, Band), Measured)> {
    file.sightings()
        .iter()
        .filter(|seen| seen.flux > 0.0)
        .filter_map(|seen| Some(((seen.flux, seen.flux_sigma, seen.band), seen.range_m.filter(|(r, _)| *r > 0.0)?)))
        .min_by(|a, b| (a.1.1 / a.1.0).total_cmp(&(b.1.1 / b.1.0)))
}

/// The blackbody best fitting the digest's ratios to [`Colors::REFERENCE`]; one sigma is where `χ²`
/// rises by one.
pub fn temperature_k(colors: &Colors) -> Option<Measured> {
    let ratios: Vec<(Band, f64, f64)> = Band::ALL
        .iter()
        .filter(|b| **b != Colors::REFERENCE)
        .filter_map(|b| colors.against_reference(*b).map(|(r, s)| (*b, r, s)))
        .filter(|(_, r, s)| *r > 0.0 && *s > 0.0)
        .collect();
    if ratios.is_empty() {
        return None;
    }
    let chi2 = |ln_t: f64| {
        let t = ln_t.exp();
        let reference = blackbody::band_radiance(Colors::REFERENCE, t);
        ratios
            .iter()
            .map(|(band, r, s)| ((blackbody::band_radiance(*band, t) / reference - r) / s).powi(2))
            .sum::<f64>()
    };
    let (lo, hi) = (COOLEST_K.ln(), HOTTEST_K.ln());
    let best = golden_min(&chi2, lo, hi);
    let floor = chi2(best) + 1.0;
    // Bisect for where χ² crosses the floor; a side that never does is pinned to the search's end.
    let edge = |outer: f64| {
        if chi2(outer) <= floor {
            return outer;
        }
        let (mut inside, mut outside) = (best, outer);
        for _ in 0..40 {
            let mid = 0.5 * (inside + outside);
            if chi2(mid) <= floor { inside = mid } else { outside = mid }
        }
        0.5 * (inside + outside)
    };
    let t = best.exp();
    Some((t, 0.5 * (edge(hi).exp() - edge(lo).exp())))
}

fn golden_min(f: &impl Fn(f64) -> f64, mut lo: f64, mut hi: f64) -> f64 {
    const INVERSE_PHI: f64 = 0.618_033_988_749_895;
    let (mut a, mut b) = (hi - INVERSE_PHI * (hi - lo), lo + INVERSE_PHI * (hi - lo));
    let (mut fa, mut fb) = (f(a), f(b));
    for _ in 0..60 {
        if fa < fb {
            hi = b;
            (b, fb) = (a, fa);
            a = hi - INVERSE_PHI * (hi - lo);
            fa = f(a);
        } else {
            lo = a;
            (a, fa) = (b, fb);
            b = lo + INVERSE_PHI * (hi - lo);
            fb = f(b);
        }
    }
    0.5 * (lo + hi)
}

/// Stefan–Boltzmann. Independent of the distance, which the radius already carried.
fn from_radius((r, sr): Measured, (t, st): Measured) -> Measured {
    let l = blackbody::luminosity(r, t);
    (l, l * (2.0 * sr / r).hypot(4.0 * st / t))
}

/// A band's flux over that band's share of a blackbody's output. The share's error is taken by
/// difference across one sigma of temperature.
fn from_flux((flux, flux_sigma, band): (f64, f64, Band), (d, sd): Measured, (t, st): Measured) -> Option<Measured> {
    let share = |t: f64| std::f64::consts::PI * blackbody::band_radiance(band, t) / (SIGMA * t.powi(4));
    let at = share(t);
    if !(at > 0.0) {
        return None;
    }
    let l = 4.0 * std::f64::consts::PI * d * d * flux / at;
    let share_part = (share(t + st) - share((t - st).max(COOLEST_K))).abs() / (2.0 * at);
    let part = (2.0 * sd / d).hypot(flux_sigma / flux).hypot(share_part);
    Some((l, l * part))
}

/// Inverse-variance in fractions, which is inverse-variance in the logarithm.
fn combine(routes: &[Measured]) -> Option<Measured> {
    let (mut weight, mut sum) = (0.0, 0.0);
    for (l, s) in routes {
        let part = s / l;
        if !(*l > 0.0 && part > 0.0 && part.is_finite()) {
            continue;
        }
        let w = 1.0 / (part * part);
        weight += w;
        sum += w * l.ln();
    }
    (weight > 0.0).then(|| {
        let l = (sum / weight).exp();
        (l, l / weight.sqrt())
    })
}

fn luminosity_of_mass((m, sm): Measured) -> Measured {
    let solar = m * super::body::GRAVITY / SOLAR_MU;
    let l = SOLAR_LUMINOSITY * solar.powf(MASS_LUMINOSITY_POWER);
    (l, l * MASS_LUMINOSITY_POWER * (sm / m).hypot(MAIN_SEQUENCE_SCATTER))
}

fn mass_of_luminosity((l, sl): Measured) -> Measured {
    let solar = em_spectra::stellar::main_sequence_mass_solar(l / SOLAR_LUMINOSITY);
    let m = solar * SOLAR_MU / super::body::GRAVITY;
    (m, m * (sl / l / MASS_LUMINOSITY_POWER).hypot(MAIN_SEQUENCE_SCATTER))
}

#[cfg(test)]
mod tests {
    use em_spectra::PerBand;
    use em_spectra::stellar::{SOLAR_RADIUS, SOLAR_TEFF};

    use super::*;
    use crate::knowledge::Witness;

    fn sun_colors(noise: f64) -> Colors {
        let mut colors = Colors::new(Witness(1));
        let flux = PerBand::splat(0.0).map(|band, _| blackbody::band_radiance(band, SOLAR_TEFF));
        for visit in 0..20 {
            let wobble = if visit % 2 == 0 { 1.0 + noise } else { 1.0 - noise };
            let row = flux.map(|band, f| match band {
                Band::B | Band::V | Band::R | Band::I => Some((f * if band == Band::V { 1.0 } else { wobble }, f * noise.max(1e-4))),
                _ => None,
            });
            colors.fold(visit as f64, &row);
        }
        colors
    }

    /// Noisier colors must give a wider temperature, around the same one.
    #[test]
    fn colors_give_back_the_temperature_they_were_made_at() {
        let (t, sigma) = temperature_k(&sun_colors(0.01)).expect("a temperature");
        assert!((t / SOLAR_TEFF - 1.0).abs() < 0.02, "{t} ± {sigma}");
        assert!(sigma > 0.0 && sigma < 0.05 * t, "{sigma}");
        let (_, wide) = temperature_k(&sun_colors(0.05)).unwrap();
        assert!(wide > sigma, "{wide} against {sigma}");
    }

    #[test]
    fn one_band_is_not_a_temperature() {
        let mut colors = Colors::new(Witness(1));
        let mut row = PerBand::splat(None);
        row[Band::V] = Some((1.0, 0.01));
        colors.fold(0.0, &row);
        assert_eq!(temperature_k(&colors), None);
    }

    #[test]
    fn the_routes_agree_on_the_sun() {
        let au = crate::system::UNIT_M;
        let t = (SOLAR_TEFF, 30.0);
        let (by_radius, _) = from_radius((SOLAR_RADIUS, SOLAR_RADIUS * 1e-3), t);
        let v = crate::knowledge::survey::flux_at(SOLAR_RADIUS, blackbody::band_radiance(Band::V, SOLAR_TEFF), au);
        let (by_flux, _) = from_flux((v, v * 1e-3, Band::V), (au, au * 1e-4), t).unwrap();
        let sun = blackbody::luminosity(SOLAR_RADIUS, SOLAR_TEFF);
        assert!((by_radius / sun - 1.0).abs() < 1e-9, "{by_radius}");
        assert!((by_flux / sun - 1.0).abs() < 1e-3, "{by_flux}");
    }

    #[test]
    fn combining_weights_by_fractional_error() {
        let (l, s) = combine(&[(100.0, 1.0), (110.0, 11.0)]).unwrap();
        assert!(l > 100.0 && l < 101.0, "{l}");
        assert!(s < 1.0, "{s}");
        assert_eq!(combine(&[]), None);
    }

    /// A flux taken near the star and read against a later, farther look's range would be off by
    /// the square of the ratio.
    #[test]
    fn a_flux_is_read_at_the_range_it_was_taken_from() {
        let au = crate::system::UNIT_M;
        let sun_v = |d: f64| crate::knowledge::survey::flux_at(SOLAR_RADIUS, blackbody::band_radiance(Band::V, SOLAR_TEFF), d);
        let look = |observed_s: f64, d: f64, range_m: Option<(f64, f64)>| crate::knowledge::Sighting {
            witness: Witness(1),
            observed_s,
            bearing: crate::knowledge::Bearing { observer_ly: DVec3::ZERO, toward: DVec3::X, sigma_rad: 1e-6 },
            size: None,
            range_m,
            spin_s: None,
            band: Band::V,
            flux: sun_v(d),
            flux_sigma: sun_v(d) * 1e-3,
            lineage: Vec::new(),
        };
        let mut knowledge = Knowledge::new(Witness(1));
        let star = StarId::synthesize("host", 1);
        knowledge.sighted(star, look(0.0, 0.05 * au, Some((0.05 * au, 0.05 * au * 1e-4))));
        knowledge.sighted(star, look(1.0, 3.0 * au, None));
        let (flux, range) = ranged_flux(knowledge.file(Subject::Star(star)).unwrap()).expect("a ranged look");
        let (l, _) = from_flux(flux, range, (SOLAR_TEFF, 10.0)).unwrap();
        let sun = blackbody::luminosity(SOLAR_RADIUS, SOLAR_TEFF);
        assert!((l / sun - 1.0).abs() < 1e-3, "{}", l / sun);
    }

    #[test]
    fn a_reading_pulls_a_loose_luminosity_to_it() {
        let au = crate::system::UNIT_M;
        let sun = blackbody::luminosity(SOLAR_RADIUS, SOLAR_TEFF);
        let loose = Host { distance_m: Some((au, au * 1e-4)), luminosity_w: Some((1.3 * sun, 0.3 * sun)), ..Host::default() };
        let read = loose.with_reading(sun / (4.0 * std::f64::consts::PI * au * au));
        let (l, s) = read.luminosity_w.unwrap();
        assert!((l / sun - 1.0).abs() < 1e-3, "{}", l / sun);
        assert!(s < 1e-3 * sun, "{s}");
        assert_eq!(Host::default().with_reading(1.0), Host::default(), "no distance, no reading");
    }

    #[test]
    fn the_main_sequence_goes_both_ways() {
        let sun_kg = SOLAR_MU / super::super::body::GRAVITY;
        let (l, sl) = luminosity_of_mass((sun_kg, 0.0));
        assert!((l / SOLAR_LUMINOSITY - 1.0).abs() < 1e-9);
        assert!((sl / l - MASS_LUMINOSITY_POWER * MAIN_SEQUENCE_SCATTER).abs() < 1e-9);
        let (m, _) = mass_of_luminosity((l, 0.0));
        assert!((m / sun_kg - 1.0).abs() < 1e-9);
    }
}
