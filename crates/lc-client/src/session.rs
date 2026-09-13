//! What the client knows and is doing. No engine: the Bevy layer reads this.

use std::collections::HashMap;

use em_spectra::{Band, BandMapping, PerBand, blackbody, presets};
use glam::{DVec3, Vec3};
use lc_spacetime::{Coord, Micros, frame::SystemFrame, units::Span};
use lc_world::instrument::Instrument;
use lc_world::observation::{Observation, Target, observe};
use lc_world::sky::{CatalogueStar, StarId, StarProvider, generate};
use lc_world::star::Star;

use crate::curve::LightCurve;
use crate::tonemap::{Shaded, ToneMap};

/// One in-game Julian year per real hour.
pub const TIME_RATE: f64 = 31_557_600.0 / 3600.0;

/// Light-years per light-microsecond, for reading catalogue positions onto the grid.
const LY_PER_LUS: f64 = 299.792458 / 9.460_730_472_580_8e15;

/// How many nearby stars get a generated system and a full emission model.
///
/// The rest are drawn as bare blackbodies. Starlight is a function rather than an event
/// stream, so a star with nothing around it costs one evaluation and needs no model at all.
pub const MODELLED_STARS: usize = 12;

/// A star as the renderer wants it.
#[derive(Clone, Copy, Debug)]
pub struct SkyStar {
    pub id: StarId,
    /// Ecliptic position, light-years.
    pub position_ly: DVec3,
    pub shaded: Shaded,
    /// Light travel time from it, in seconds. Everything drawn is this stale.
    pub light_age_s: f64,
}

pub struct Session {
    pub stars: Vec<CatalogueStar>,
    pub observer: Coord,
    pub telescope: Instrument,
    pub mapping: BandMapping,
    pub tone: ToneMap,
    pub curve: LightCurve,
    pub pointing: Option<StarId>,
    targets: HashMap<StarId, Target>,
}

impl Session {
    /// Take the nearest stars to the origin and model a few of them.
    pub fn new(provider: &dyn StarProvider, count: usize) -> Self {
        let mut stars: Vec<CatalogueStar> = provider.stars().to_vec();
        stars.sort_by(|a, b| a.position_ly.length().total_cmp(&b.position_ly.length()));
        stars.truncate(count.max(1));

        let mut targets = HashMap::new();
        for star in stars.iter().take(MODELLED_STARS) {
            targets.insert(star.id, build_target(star));
        }

        let mut session = Self {
            stars,
            observer: Coord::ORIGIN,
            telescope: Instrument::BASELINE,
            mapping: presets::natural(),
            tone: ToneMap::default(),
            curve: LightCurve::new(Band::V, 4000),
            pointing: None,
            targets,
        };
        session.auto_expose();
        session
    }

    /// Advance coordinate time by a span of real seconds.
    pub fn advance(&mut self, real_seconds: f64) {
        let micros = (real_seconds * TIME_RATE * 1e6) as i64;
        self.observer = Coord {
            t: self.observer.t + Span::new(micros),
            ..self.observer
        };
    }

    pub fn coordinate_time_s(&self) -> f64 {
        self.observer.t.get() as f64 * 1e-6
    }

    pub fn target(&self, id: StarId) -> Option<&Target> {
        self.targets.get(&id)
    }

    pub fn star(&self, id: StarId) -> Option<&CatalogueStar> {
        self.stars.iter().find(|s| s.id == id)
    }

    /// Point the telescope, clearing whatever it was watching.
    pub fn point_at(&mut self, id: Option<StarId>) {
        if self.pointing != id {
            self.curve.clear();
        }
        self.pointing = id;
    }

    /// Take one measurement of whatever the telescope is on.
    pub fn observe(&mut self, exposure_s: f64) -> Option<Observation> {
        let target = self.targets.get(&self.pointing?)?;
        let obs = observe(target, self.observer, &self.telescope, exposure_s, 0x10c)?;
        self.curve.record(&obs);
        Some(obs)
    }

    /// Band radiance arriving from one star, light delay included where it is modelled.
    pub fn radiance_from(&self, star: &CatalogueStar) -> PerBand<f32> {
        let distance_m = star.position_ly.length() * 9.460_730_472_580_8e15;
        match self.targets.get(&star.id) {
            // A modelled system is evaluated properly, light delay and all.
            Some(target) => observe(target, self.observer, &full_spectrum(), 1.0, 0x5ee)
                .map(|o| received(&o, &star.star, distance_m))
                .unwrap_or_default(),
            // A bare star is a function of its own parameters and nothing else.
            None => bare(&star.star, distance_m),
        }
    }

    /// Every star, shaded for the current band mapping and exposure.
    pub fn sky(&self) -> Vec<SkyStar> {
        self.stars
            .iter()
            .map(|s| {
                let distance_m = s.position_ly.length() * 9.460_730_472_580_8e15;
                SkyStar {
                    id: s.id,
                    position_ly: s.position_ly,
                    shaded: self.tone.shade(&self.radiance_from(s), &self.mapping),
                    light_age_s: distance_m / 299_792_458.0,
                }
            })
            .collect()
    }

    /// Put the brightest thing in the sky at the top of the displayed window.
    ///
    /// There is no absolute reference to use instead. A star's band radiance at
    /// interstellar range is of order 1e-10 in SI units, so any fixed reference is either
    /// thirty stops high or thirty stops low, and the picture is black or white accordingly.
    /// The window has to be placed by the scene, and it moves when the band mapping does —
    /// the thermal preset reads a different band and therefore a different brightness.
    pub fn auto_expose(&mut self) {
        self.expose_to_percentile(0.98);
    }

    /// Place the window so that `fraction` of the sky falls below the top of it.
    ///
    /// A percentile rather than the maximum, because one star can be arbitrarily closer than
    /// the rest — the Sun is in the catalogue at about an astronomical unit — and exposing
    /// for it puts everything else thirty stops under and renders a black sky. Letting the
    /// brightest couple of percent clip is what a star map does anyway.
    pub fn expose_to_percentile(&mut self, fraction: f32) {
        let mut luminances: Vec<f32> = self
            .stars
            .iter()
            .map(|s| self.luminance_from(s))
            .filter(|l| *l > 0.0 && l.is_finite())
            .collect();
        if luminances.is_empty() {
            return;
        }
        luminances.sort_by(f32::total_cmp);
        let at = ((luminances.len() - 1) as f32 * fraction.clamp(0.0, 1.0)).round() as usize;
        self.tone.reference = luminances[at];
    }

    /// Displayed luminance a star would contribute under the current mapping.
    pub fn luminance_from(&self, star: &CatalogueStar) -> f32 {
        let rgb = self.mapping.apply(&self.radiance_from(star));
        rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
    }
}

/// An idealised instrument, for evaluating what leaves a system rather than what an
/// instrument would record of it.
fn full_spectrum() -> Instrument {
    Instrument::BASELINE
        .with_bands(em_spectra::BandMask::ALL)
        .cooled_to(20.0)
}

fn geometry(star: &Star, distance_m: f64) -> f64 {
    std::f64::consts::PI * star.radius_m * star.radius_m / (distance_m * distance_m)
}

fn bare(star: &Star, distance_m: f64) -> PerBand<f32> {
    let g = geometry(star, distance_m);
    PerBand::new(std::array::from_fn(|i| {
        (blackbody::band_radiance(Band::ALL[i], star.teff_k) * g) as f32
    }))
}

fn received(observation: &Observation, star: &Star, distance_m: f64) -> PerBand<f32> {
    let g = geometry(star, distance_m);
    PerBand::new(std::array::from_fn(|i| {
        let band = Band::ALL[i];
        let full = blackbody::band_radiance(band, star.teff_k) * g;
        let deficit = observation.band(band).map(|m| m.true_deficit).unwrap_or(0.0);
        (full * (1.0 - deficit)) as f32
    }))
}

fn build_target(star: &CatalogueStar) -> Target {
    let system = generate::system_for(star);
    let mut model = lc_world::emission::EmissionModel::new(star.star, star.seed());
    model.populations = system.populations;
    for planet in &system.planets {
        model.bodies.push(lc_world::emission::Body {
            occluder: lc_world::occluder::Occluder::new(planet.radius_m),
            motion: Box::new(lc_world::emission::CircularOrbit {
                radius_m: planet.semi_major_m,
                pole: DVec3::Z,
                phase0: planet.mean_anomaly_deg.to_radians(),
                mu: star.star.mu,
            }),
        });
    }
    let grid = star.position_ly / LY_PER_LUS / 9.460_730_472_580_8e15 * 299.792458;
    let origin = Coord::new(
        Micros::ORIGIN,
        (star.position_ly.x / LY_PER_LUS) as i64,
        (star.position_ly.y / LY_PER_LUS) as i64,
        (star.position_ly.z / LY_PER_LUS) as i64,
    )
    .unwrap_or(Coord::ORIGIN);
    let _ = grid;
    Target::new(SystemFrame::new(origin), model)
}

/// Stops of brightness a star field spreads over, beyond the tone map's own window.
pub const POINT_STOPS: f32 = 14.0;

/// How large a star should be drawn: its brightness across [`POINT_STOPS`], plus any glow.
pub fn point_size(shaded: &Shaded, base: f32) -> f32 {
    base * (0.35 + shaded.point_brightness(POINT_STOPS) + shaded.glow.clamp(0.0, 12.0) * 0.3)
}

/// Colour for a star drawn as a point.
pub fn point_colour(shaded: &Shaded) -> Vec3 {
    shaded.point_colour(POINT_STOPS)
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;

    fn session() -> Session {
        Session::new(&AuthoredStars::sample(), 3)
    }

    #[test]
    fn a_session_sorts_its_stars_by_distance_and_models_the_near_ones() {
        let s = session();
        assert_eq!(s.stars.len(), 3);
        let d: Vec<f64> = s.stars.iter().map(|x| x.position_ly.length()).collect();
        assert!(d.windows(2).all(|w| w[0] <= w[1]), "{d:?}");
        assert!(s.target(s.stars[0].id).is_some());
    }

    #[test]
    fn the_clock_runs_a_year_an_hour() {
        let mut s = session();
        s.advance(3600.0);
        let years = s.coordinate_time_s() / 31_557_600.0;
        assert!((years - 1.0).abs() < 1e-6, "an hour should be {years} years");
    }

    #[test]
    fn every_star_in_the_sky_is_shaded_and_stale() {
        let s = session();
        let sky = s.sky();
        assert_eq!(sky.len(), 3);
        for star in &sky {
            assert!(star.shaded.colour().is_finite());
            assert!(star.light_age_s > 0.0, "everything drawn is old");
            // Four light-years is about four years of staleness.
            let years = star.light_age_s / 31_557_600.0;
            assert!(years > 4.0 && years < 30.0, "{years} years");
        }
    }

    /// The bug this exists for: the Sun sits in the catalogue about an astronomical unit
    /// away, and exposing for the brightest star renders everything else black.
    ///
    /// Not an artifact — the Sun at one AU outshines a star four light-years off by some
    /// thirty-six stops, which is why daylight hides the sky. What a percentile buys is that
    /// the outlier clips instead of setting the scale for everyone.
    #[test]
    fn one_very_close_star_does_not_black_out_the_field() {
        let template = AuthoredStars::sample().stars()[1].clone();
        let mut stars = Vec::new();
        for k in 0..200u64 {
            let mut s = template.clone();
            s.provenance.key = k;
            s.id = lc_world::sky::StarId::synthesise("many", k);
            let d = 4.0 + (k % 40) as f64;
            s.position_ly = glam::DVec3::new(d, (k % 7) as f64, (k % 11) as f64).normalize() * d;
            stars.push(s);
        }
        let mut sun = template.clone();
        sun.provenance.key = 9999;
        sun.id = lc_world::sky::StarId::synthesise("many", 9999);
        sun.position_ly = glam::DVec3::new(1.6e-5, 0.0, 0.0); // roughly an AU
        stars.insert(0, sun);

        let s = Session::new(&AuthoredStars::new("many", stars), 201);
        let visible = s.sky().iter().filter(|x| point_colour(&x.shaded).length() > 0.0).count();
        assert!(visible > 150, "only {visible} of 201 stars survived the exposure");

        // Exposing for the maximum instead is the failure: the field goes out.
        let mut naive = Session::new(&AuthoredStars::new("many", s.stars.clone()), 201);
        naive.expose_to_percentile(1.0);
        let left = naive.sky().iter().filter(|x| point_colour(&x.shaded).length() > 0.0).count();
        assert!(left < 5, "{left} stars should have survived exposing for the Sun");
    }

    #[test]
    fn auto_exposure_puts_the_brightest_star_at_the_top_of_the_window() {
        let s = session();
        let sky = s.sky();
        assert!(sky.iter().any(|x| x.shaded.colour().length() > 0.0), "something must be visible");
        let brightest = sky
            .iter()
            .map(|x| x.shaded.colour().max_element())
            .fold(0.0f32, f32::max);
        assert!(brightest > 0.99, "the brightest should fill the window, got {brightest}");
    }

    /// The bug this exists for: a fixed reference of 1.0 is thirty stops above a star's
    /// actual band radiance, and the whole sky renders black.
    #[test]
    fn a_fixed_reference_would_render_nothing() {
        let mut s = session();
        s.tone = ToneMap::default();
        assert!(
            s.sky().iter().all(|x| x.shaded.colour() == Vec3::ZERO),
            "an unexposed scene is black, which is why auto_expose exists"
        );
        s.auto_expose();
        assert!(s.sky().iter().any(|x| x.shaded.colour().length() > 0.0));
    }

    #[test]
    fn pointing_somewhere_new_discards_the_old_curve() {
        let mut s = session();
        let (a, b) = (s.stars[0].id, s.stars[1].id);
        s.point_at(Some(a));
        s.observe(1e4);
        assert!(!s.curve.is_empty());
        s.point_at(Some(b));
        assert!(s.curve.is_empty(), "a curve belongs to one target");
    }

    #[test]
    fn observing_nothing_returns_nothing() {
        let mut s = session();
        s.point_at(None);
        assert!(s.observe(1.0).is_none());
    }

    #[test]
    fn a_modelled_system_accumulates_a_curve_over_time() {
        let mut s = session();
        s.telescope = s.telescope.with_aperture(1e4).with_bands(em_spectra::BandMask::ALL);
        s.point_at(Some(s.stars[0].id));
        for _ in 0..200 {
            s.advance(6.0);
            s.observe(1e3);
        }
        assert_eq!(s.curve.len(), 200);
        let (first, last) = s.curve.span().unwrap();
        assert!(last > first, "the curve should advance through emission time");
    }

    #[test]
    fn switching_the_band_mapping_changes_what_is_drawn() {
        let mut s = session();
        let natural: Vec<Vec3> = s.sky().iter().map(|x| x.shaded.colour()).collect();
        s.mapping = presets::thermal();
        // The window follows the mapping: a different band is a different brightness.
        s.auto_expose();
        let thermal: Vec<Vec3> = s.sky().iter().map(|x| x.shaded.colour()).collect();
        assert_ne!(natural, thermal, "the preset must reach the picture");
    }
}
