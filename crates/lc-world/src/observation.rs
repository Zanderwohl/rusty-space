//! Observing a system from a distance, which is to say observing its past.

use em_spectra::{Band, PerBand, blackbody};
use glam::DVec3;
use lc_spacetime::worldline::Static;
use lc_spacetime::{Coord, Micros, frame::SystemFrame, retarded_times};

use crate::emission::EmissionModel;
use crate::instrument::Instrument;
use crate::rng;
use crate::star::Star;

/// A system, and how its emission has changed over coordinate time.
///
/// The history is what makes a change observable at all: an event does not alter the past, it
/// appends a new emission model from the moment it happened. An observer solving for retarded
/// time therefore reads whichever model was in force when the light left.
pub struct Target {
    pub frame: SystemFrame,
    history: Vec<(Micros, EmissionModel)>,
}

impl Target {
    pub fn new(frame: SystemFrame, initial: EmissionModel) -> Self {
        Self { frame, history: vec![(frame.origin.t, initial)] }
    }

    /// Append a model that takes effect at `from`. Later than every existing entry.
    pub fn changes_at(&mut self, from: Micros, model: EmissionModel) {
        assert!(from >= self.history.last().unwrap().0, "history must be appended in order");
        self.history.push((from, model));
    }

    /// The model in force at a coordinate time.
    pub fn model_at(&self, t: Micros) -> &EmissionModel {
        let i = self.history.partition_point(|(at, _)| *at <= t);
        &self.history[i.saturating_sub(1)].1
    }

    pub fn star(&self) -> &Star {
        &self.history[0].1.star
    }

    fn worldline(&self) -> Static {
        Static::new(self.frame.origin.position())
    }
}

/// One band of one observation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandMeasurement {
    /// Fractional deficit from occultation alone, `[0, 1]`. What the moment inversion wants.
    pub true_deficit: f64,
    /// Thermal re-emission from the populations, as a fraction of the star's band flux. Zero
    /// for a system with nothing warm in it, and larger than one in the thermal infrared for a
    /// substantial swarm.
    pub reradiated: f64,
    /// Net fractional change the instrument recorded, noise included. Signed: positive is a
    /// deficit, negative an excess, and a swarm produces both at once in different bands.
    pub measured_deficit: f64,
    /// One sigma on `measured_deficit`.
    pub uncertainty: f64,
    pub source_photons: f64,
    pub background_photons: f64,
}

impl BandMeasurement {
    /// How many sigma the measured change stands from zero. Negative for an excess.
    pub fn significance(&self) -> f64 {
        if self.uncertainty > 0.0 { self.measured_deficit / self.uncertainty } else { 0.0 }
    }

    /// Flux actually arriving, over what the bare star would give. Above one where re-emission
    /// beats occultation.
    pub fn relative_flux(&self) -> f64 {
        1.0 - self.true_deficit + self.reradiated
    }
}

/// What an observer got, and when the light left.
#[derive(Clone, Debug)]
pub struct Observation {
    /// Coordinate time of emission, microseconds. Everything here describes the system then.
    pub retarded_time: f64,
    /// Light travel time, microseconds, which in these units is also the distance.
    pub light_travel: f64,
    /// Unit vector from the source toward the observer.
    pub direction: DVec3,
    pub bands: PerBand<Option<BandMeasurement>>,
}

impl Observation {
    pub fn age_seconds(&self) -> f64 {
        self.light_travel * 1e-6
    }

    pub fn band(&self, b: Band) -> Option<&BandMeasurement> {
        self.bands[b].as_ref()
    }

    /// Ratio of the deficit in two bands. Unity means a gray occulter; above unity in the
    /// bluer band means something that reddens, which is to say dust.
    pub fn deficit_ratio(&self, numerator: Band, denominator: Band) -> Option<f64> {
        let (n, d) = (self.band(numerator)?, self.band(denominator)?);
        if d.measured_deficit.abs() <= 0.0 {
            return None;
        }
        Some(n.measured_deficit / d.measured_deficit)
    }
}

/// Observe `target` from `observer_at`, with the light delay that implies.
///
/// `None` when no light from the target reaches that coordinate — the observer is outside its
/// future light cone, or inside the system's own past before it existed.
pub fn observe(
    target: &Target,
    observer_at: Coord,
    instrument: &Instrument,
    exposure_s: f64,
    seed: u64,
) -> Option<Observation> {
    let roots = retarded_times(observer_at, &target.worldline());
    let t_r = *roots.first()?;

    let to_observer = observer_at.position() - target.frame.origin.position();
    let light_travel = observer_at.time_f64() - t_r;
    let distance_m = light_travel * lc_spacetime::LIGHT_MICROSECOND_M;
    if distance_m <= 0.0 {
        return None;
    }
    let direction = if to_observer.length_squared() > 0.0 { to_observer.normalize() } else { DVec3::X };

    let model = target.model_at(Micros::new(t_r as i64));
    let local_t = target.frame.local_seconds(t_r);
    let deficit = model.deficit(direction, local_t);
    let reradiated = model.reradiated();
    let star = &model.star;

    // Flux of an unobscured sphere at this distance: L_band / (4 pi d^2) is R^2 pi B / d^2.
    let geometry = std::f64::consts::PI * star.radius_m * star.radius_m / (distance_m * distance_m);

    let bands = PerBand::new(std::array::from_fn(|i| {
        let band = Band::ALL[i];
        if !instrument.sees(band) {
            return None;
        }
        let baseline = blackbody::band_radiance(band, star.teff_k) * geometry;
        let bare = instrument.counts_from_flux(band, baseline, exposure_s);
        if bare <= 0.0 {
            return None;
        }
        // What actually arrives: the star less what is blocked, plus what the populations
        // re-emit. The shot noise is on that, not on the bare star.
        let occulted = deficit[band] as f64;
        let extra = reradiated[band] as f64;
        let source = (bare * (1.0 - occulted + extra)).max(0.0);
        let background = instrument.self_emission_counts(band, exposure_s);
        if source <= 0.0 {
            return None;
        }
        // Photon statistics, expressed as a fraction of the bare star's flux, which is the
        // unit every deficit here is in, and photon statistics alone: a deficit is the star
        // against itself, so the flat field and the gain that floor an absolute flux
        // ([`crate::instrument::PHOTOMETRY_FLOOR`]) are common to both and divide out.
        let uncertainty = (source + background).sqrt() / bare;
        let net = occulted - extra;
        let noise = rng::gaussian(rng::hash(&[seed, band.index() as u64, t_r.to_bits()]));
        Some(BandMeasurement {
            true_deficit: occulted,
            reradiated: extra,
            measured_deficit: net + noise * uncertainty,
            uncertainty,
            source_photons: source,
            background_photons: background,
        })
    }));

    Some(Observation { retarded_time: t_r, light_travel, direction, bands })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distribution::{Distribution, Inclination};
    use crate::emission::{Body, CircularOrbit};
    use crate::occluder::Occluder;
    use crate::population::Population;
    use em_spectra::{BandMask, extinction};
    use lc_spacetime::units::Span;

    const AU: f64 = 1.496e11;
    const LIGHT_YEAR_US: f64 = 3.155_760e13; // one Julian year in microseconds, c = 1
    const YEAR_S: f64 = 3.155_760e7;

    fn frame_at_ly(ly: f64) -> SystemFrame {
        SystemFrame::new(Coord::new(Micros::ORIGIN, (ly * LIGHT_YEAR_US) as i64, 0, 0).unwrap())
    }

    fn swarm(count: f64, response: PerBand<f32>) -> Population {
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::delta(AU),
            eccentricity: Distribution::delta(0.0),
            inclination: Inclination::isotropic(),
            count,
            cross_section: 1e12,
            band_response: response,
            radiating_ratio: Population::SPHERICAL,
        }
    }

    fn observer_at(t_years: f64) -> Coord {
        Coord::new(Micros::new((t_years * LIGHT_YEAR_US) as i64), 0, 0, 0).unwrap()
    }

    fn big_scope() -> Instrument {
        Instrument::BASELINE.with_aperture(1e4).with_bands(BandMask::ALL).cooled_to(40.0)
    }

    /// Steps 1 to 3 of the phase: what arrives describes the system at the retarded time.
    #[test]
    fn an_observation_describes_the_emission_time_not_the_observation_time() {
        let mut model = EmissionModel::new(Star::SOL, 1);
        model.populations.push(swarm(1.5e6, PerBand::splat(1.0)));
        let target = Target::new(frame_at_ly(30.0), model);

        for t in [30.5, 40.0, 100.0] {
            let obs = observe(&target, observer_at(t), &big_scope(), 1e4, 7).unwrap();
            let age = obs.age_seconds() / YEAR_S;
            assert!((age - 30.0).abs() < 1e-6, "light is {age} years old, expected 30");
            let emitted_at = obs.retarded_time / LIGHT_YEAR_US;
            assert!((emitted_at - (t - 30.0)).abs() < 1e-6, "emitted at {emitted_at}");
        }
    }

    /// Step 4, and the phase's go/no-go: a change is invisible until its light arrives.
    #[test]
    fn a_change_is_invisible_until_its_light_arrives_and_visible_after() {
        let star = Star::SOL;
        let bare = EmissionModel::new(star, 1);
        let mut built = EmissionModel::new(star, 1);
        built.populations.push(swarm(1.5e6, PerBand::splat(1.0)));

        let change_at = 10.0; // years of coordinate time
        let mut target = Target::new(frame_at_ly(30.0), bare);
        target.changes_at(Micros::new((change_at * LIGHT_YEAR_US) as i64), built);

        let scope = big_scope();
        let deficit_at = |t: f64| {
            observe(&target, observer_at(t), &scope, 1e6, 11).unwrap().band(Band::V).unwrap().true_deficit
        };

        // Before the light of the change can have arrived, nothing has changed.
        for t in [0.5, 10.0, 20.0, 39.0, 39.9999] {
            assert_eq!(deficit_at(t), 0.0, "the swarm was visible {} years early", 40.0 - t);
        }
        // After, it has.
        for t in [40.001, 45.0, 100.0] {
            assert!(deficit_at(t) > 1e-6, "the swarm should be visible at t={t}");
        }
    }

    #[test]
    fn two_observers_at_different_ranges_disagree_about_the_present() {
        let star = Star::SOL;
        let mut built = EmissionModel::new(star, 1);
        built.populations.push(swarm(1.5e6, PerBand::splat(1.0)));
        let mut target = Target::new(frame_at_ly(30.0), EmissionModel::new(star, 1));
        target.changes_at(Micros::new((10.0 * LIGHT_YEAR_US) as i64), built);

        // A probe five light-years from the star, at the same coordinate time as an observer
        // thirty out. Both are correct; they are looking at different moments.
        let near = Coord::new(
            Micros::new((20.0 * LIGHT_YEAR_US) as i64),
            (25.0 * LIGHT_YEAR_US) as i64,
            0,
            0,
        )
        .unwrap();
        let far = observer_at(20.0);
        let scope = big_scope();
        let n = observe(&target, near, &scope, 1e6, 3).unwrap();
        let f = observe(&target, far, &scope, 1e6, 3).unwrap();
        assert!(n.band(Band::V).unwrap().true_deficit > 0.0, "the probe sees the swarm");
        assert_eq!(f.band(Band::V).unwrap().true_deficit, 0.0, "the distant observer does not");
        assert!(n.age_seconds() < f.age_seconds());
    }

    /// Step 5, corrected. Gray versus reddening is a *ratio* between bands, so one band
    /// cannot do it at any exposure and two can. The stellar-locus degeneracy that K does not
    /// rescue is a different measurement -- a steady obscuration with no baseline to compare
    /// against -- and lives in `em_spectra::extinction`.
    #[test]
    fn one_band_cannot_tell_a_swarm_from_dust_and_two_can() {
        let star = Star::SOL;
        let mut dust_response = PerBand::splat(0.0f32);
        for b in Band::ALL {
            dust_response[b] = extinction::RATIO[b] as f32;
        }

        let mut gray_model = EmissionModel::new(star, 5);
        gray_model.populations.push(swarm(1.5e6, PerBand::splat(1.0)));
        let mut dust_model = EmissionModel::new(star, 5);
        dust_model.populations.push(swarm(1.5e6, dust_response));

        let gray = Target::new(frame_at_ly(30.0), gray_model);
        let dust = Target::new(frame_at_ly(30.0), dust_model);
        let at = observer_at(60.0);

        let v_only = big_scope().with_bands(BandMask::of(&[Band::V]));
        let optical = big_scope().with_bands(BandMask::SILICON);

        let (g1, d1) = (
            observe(&gray, at, &v_only, 1e8, 1).unwrap(),
            observe(&dust, at, &v_only, 1e8, 1).unwrap(),
        );
        // In V the two are the same measurement, and there is no second band to compare.
        let (gv, dv) = (g1.band(Band::V).unwrap(), d1.band(Band::V).unwrap());
        assert!((gv.true_deficit / dv.true_deficit - 1.0).abs() < 1e-12);
        assert!(g1.deficit_ratio(Band::B, Band::V).is_none(), "V alone has no ratio to form");

        let (g2, d2) = (
            observe(&gray, at, &optical, 1e8, 1).unwrap(),
            observe(&dust, at, &optical, 1e8, 1).unwrap(),
        );
        let gray_ratio = g2.deficit_ratio(Band::B, Band::V).unwrap();
        let dust_ratio = d2.deficit_ratio(Band::B, Band::V).unwrap();
        assert!((gray_ratio - 1.0).abs() < 0.05, "a solid occulter is gray: {gray_ratio}");
        assert!((dust_ratio - 1.32).abs() < 0.1, "dust reddens: {dust_ratio}");
    }

    #[test]
    fn a_transiting_planet_appears_in_the_curve_at_the_right_depth() {
        let star = Star::SOL;
        let mut model = EmissionModel::new(star, 2);
        model.bodies.push(Body {
            occluder: Occluder::new(6.371e6),
            motion: Box::new(CircularOrbit { radius_m: AU, pole: DVec3::Z, phase0: 0.0, mu: star.mu }),
        });
        let target = Target::new(frame_at_ly(30.0), model);
        let scope = big_scope();
        let period_us = std::f64::consts::TAU * (AU.powi(3) / star.mu).sqrt() * 1e6;

        let base = 40.0 * LIGHT_YEAR_US;
        let deepest = (0..4000)
            .map(|k| {
                let t = base + k as f64 * period_us / 4000.0;
                let at = Coord::new(Micros::new(t as i64), 0, 0, 0).unwrap();
                observe(&target, at, &scope, 1.0, 4).unwrap().band(Band::V).unwrap().true_deficit
            })
            .fold(0.0, f64::max);
        assert!((deepest - 1.0186e-4).abs() < 5e-6, "transit depth {deepest}");
    }

    #[test]
    fn noise_falls_as_the_square_root_of_exposure() {
        let mut model = EmissionModel::new(Star::SOL, 1);
        model.populations.push(swarm(1.5e6, PerBand::splat(1.0)));
        let target = Target::new(frame_at_ly(10.0), model);
        let at = observer_at(20.0);
        let scope = big_scope();
        let sigma = |exposure: f64| {
            observe(&target, at, &scope, exposure, 1).unwrap().band(Band::V).unwrap().uncertainty
        };
        assert!((sigma(100.0) / sigma(10_000.0) - 10.0).abs() < 0.01);
    }

    #[test]
    fn distance_costs_depth_as_the_inverse_square() {
        let build = |ly: f64| {
            let mut m = EmissionModel::new(Star::SOL, 1);
            m.populations.push(swarm(1.5e6, PerBand::splat(1.0)));
            Target::new(frame_at_ly(ly), m)
        };
        let scope = big_scope();
        let sigma = |ly: f64| {
            let at = Coord::new(Micros::new(((ly + 1.0) * LIGHT_YEAR_US) as i64), 0, 0, 0).unwrap();
            observe(&build(ly), at, &scope, 1e6, 1).unwrap().band(Band::V).unwrap().uncertainty
        };
        // Ten times further is a hundred times fewer photons, so ten times the noise.
        assert!((sigma(100.0) / sigma(10.0) - 10.0).abs() < 0.05);
    }

    #[test]
    fn a_warm_instrument_cannot_use_its_thermal_band() {
        let mut model = EmissionModel::new(Star::SOL, 1);
        model.populations.push(swarm(1.5e6, PerBand::splat(1.0)));
        let target = Target::new(frame_at_ly(10.0), model);
        let at = observer_at(11.0);
        let warm = big_scope().cooled_to(290.0);
        let cold = big_scope().cooled_to(40.0);
        let sigma = |i: &Instrument| {
            observe(&target, at, i, 1e4, 1).unwrap().band(Band::ThermalIr).unwrap().uncertainty
        };
        assert!(sigma(&warm) / sigma(&cold) > 1e3, "self-emission must swamp the signal");
        // And it costs nothing in V.
        let v = |i: &Instrument| observe(&target, at, i, 1e4, 1).unwrap().band(Band::V).unwrap().uncertainty;
        assert!((v(&warm) / v(&cold) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn nothing_is_observed_from_outside_the_future_light_cone() {
        let target = Target::new(frame_at_ly(30.0), EmissionModel::new(Star::SOL, 1));
        // The observer's own worldline starts at the origin of time; light from 30 ly away
        // emitted at t = 0 has not arrived at t = 1 year.
        let too_early = Coord::new(Micros::new(Span::from_seconds(1).get()), 0, 0, 0).unwrap();
        let obs = observe(&target, too_early, &big_scope(), 1.0, 1);
        assert!(obs.is_some(), "a static star has always been emitting");
        assert!(obs.unwrap().retarded_time < 0.0, "so the light on arrival left before t = 0");
    }

    /// A swarm covering `coverage` of the sphere at one astronomical unit.
    ///
    /// The count is derived, not written: an element count is not a covering fraction, and
    /// guessing one produced a swarm seven orders of magnitude thinner than intended.
    fn swarm_of(coverage: f64) -> Population {
        let radius = 1.495_978_707e11;
        let element = 1.0e6;
        Population {
            pole: DVec3::Z,
            semi_major: Distribution::normal(radius, radius * 0.05, 9),
            eccentricity: Distribution::uniform(0.0, 0.02, 3),
            inclination: Inclination::isotropic(),
            count: coverage * 4.0 * std::f64::consts::PI * radius * radius / element,
            cross_section: element,
            band_response: PerBand::splat(1.0),
            radiating_ratio: Population::PANEL,
        }
    }

    /// The exit criterion of phase 6, at the level the instrument sees it: a star half dimmed
    /// in the visible and blazing at ten microns, from one population.
    #[test]
    fn a_swarm_dims_the_visible_and_lights_up_the_thermal_infrared() {
        let mut model = EmissionModel::new(Star::SOL, 7);
        model.populations.push(swarm_of(0.5));

        let extra = model.reradiated();
        let occulted = model.deficit(DVec3::X, 0.0);
        assert!(occulted[Band::V] > 0.3, "V should be well down, got {}", occulted[Band::V]);
        assert!(extra[Band::V] < 1e-9, "and nothing warm emits visible light");
        assert!(extra[Band::ThermalIr] > 50.0,
            "ten microns should be swamped, got {}", extra[Band::ThermalIr]);

        // And the two are the same population: occultation is gray, so V and I agree.
        let ratio = occulted[Band::I] / occulted[Band::V];
        assert!((ratio - 1.0).abs() < 0.02, "solid occultation is gray, got {ratio}");
    }

    /// The diagnostic, not merely the signature: transits move and waste heat does not. A swarm
    /// flickers in the visible while sitting perfectly still at ten microns.
    #[test]
    fn the_thermal_excess_is_steady_while_the_visible_flickers() {
        let mut model = EmissionModel::new(Star::SOL, 11);
        model.populations.push(swarm_of(0.5));

        let steady = model.reradiated()[Band::ThermalIr];
        let mut visible = Vec::new();
        for k in 0..64 {
            let t = k as f64 * 3.0e6;
            assert_eq!(model.reradiated()[Band::ThermalIr], steady, "heat does not flicker");
            visible.push(model.deficit(DVec3::X, t)[Band::V]);
        }
        let mean = visible.iter().sum::<f32>() / visible.len() as f32;
        let spread = visible.iter().map(|v| (v - mean).abs()).fold(0.0f32, f32::max);
        assert!(spread > 0.0, "the visible deficit should move, and it did not");
        assert!(mean > 0.0);
    }

    /// What the instrument records, with noise, through the same path the client uses.
    #[test]
    fn an_observation_carries_the_deficit_and_the_excess_separately() {
        let mut model = EmissionModel::new(Star::SOL, 3);
        model.populations.push(swarm_of(0.5));
        let target = Target::new(SystemFrame::new(Coord::ORIGIN), model);
        let far = Coord::new(Micros::new(400_000_000_000), 300_000_000, 0, 0).unwrap();
        let instrument = Instrument::BASELINE.with_aperture(20.0).with_bands(BandMask::ALL);
        let obs = observe(&target, far, &instrument, 1.0e5, 0x5117).expect("an observation");

        let v = obs.band(Band::V).expect("a V measurement");
        assert!(v.true_deficit > 0.0 && v.reradiated < 1e-9);
        assert!(v.relative_flux() < 1.0, "the visible is a shadow");

        let ir = obs.band(Band::ThermalIr).expect("a thermal measurement");
        assert!(ir.reradiated > 1.0, "the thermal band is a source, got {}", ir.reradiated);
        assert!(ir.relative_flux() > 1.0, "and it arrives brighter than the bare star");
        assert!(ir.measured_deficit < 0.0, "a net excess reads as a negative deficit");
    }
}
