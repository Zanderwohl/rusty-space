//! A light curve as it accumulates.

use std::collections::VecDeque;

use em_spectra::Band;
use lc_world::observation::Observation;

/// A rolling window of measurements in one band.
#[derive(Clone, Debug)]
pub struct LightCurve {
    pub band: Band,
    capacity: usize,
    samples: VecDeque<(f64, f64)>,
    uncertainty: f64,
}

impl LightCurve {
    pub fn new(band: Band, capacity: usize) -> Self {
        Self { band, capacity: capacity.max(2), samples: VecDeque::new(), uncertainty: 0.0 }
    }

    /// Record a measurement, dropping the oldest once full.
    ///
    /// Time is the **emission** time, not the observation time: a curve describes the system
    /// that produced it, and plotting it against arrival would smear anything that moved.
    pub fn record(&mut self, observation: &Observation) {
        let Some(m) = observation.band(self.band) else { return };
        while self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back((observation.retarded_time * 1e-6, m.measured_deficit));
        self.uncertainty = m.uncertainty;
    }

    /// Samples as a contiguous slice, for a plot.
    pub fn samples(&mut self) -> &[(f64, f64)] {
        self.samples.make_contiguous();
        self.samples.as_slices().0
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// One sigma on the most recent measurement.
    pub fn uncertainty(&self) -> f64 {
        self.uncertainty
    }

    /// Emission times covered, in seconds.
    pub fn span(&self) -> Option<(f64, f64)> {
        Some((self.samples.front()?.0, self.samples.back()?.0))
    }

    /// Deepest measured deficit, which is what a transit search is looking for.
    pub fn deepest(&self) -> f64 {
        self.samples.iter().map(|(_, d)| *d).fold(0.0, f64::max)
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

#[cfg(test)]
mod tests {
    use em_spectra::{BandMask, PerBand};
    use glam::DVec3;
    use lc_spacetime::{Coord, Micros, frame::SystemFrame};
    use lc_world::distribution::{Distribution, Inclination};
    use lc_world::emission::{Body, CircularOrbit, EmissionModel};
    use lc_world::instrument::Instrument;
    use lc_world::occluder::Occluder;
    use lc_world::observation::{Target, observe};
    use lc_world::population::Population;
    use lc_world::star::Star;

    use super::*;

    const AU: f64 = 1.496e11;
    const LY_US: f64 = 3.155_760e13;

    fn scope() -> Instrument {
        Instrument::BASELINE.with_aperture(1e4).with_bands(BandMask::ALL).cooled_to(40.0)
    }

    fn target_with_planet() -> Target {
        let star = Star::SOL;
        let mut model = EmissionModel::new(star, 1);
        model.bodies.push(Body {
            occluder: Occluder::new(6.371e6),
            motion: Box::new(CircularOrbit { radius_m: AU, pole: DVec3::Z, phase0: 0.0, mu: star.mu }),
        });
        let frame = SystemFrame::new(Coord::new(Micros::ORIGIN, (30.0 * LY_US) as i64, 0, 0).unwrap());
        Target::new(frame, model)
    }

    #[test]
    fn a_curve_keeps_a_rolling_window() {
        let target = target_with_planet();
        let mut curve = LightCurve::new(Band::V, 10);
        for k in 0..50 {
            let t = (40.0 * LY_US) as i64 + k * 86_400_000_000;
            let at = Coord::new(Micros::new(t), 0, 0, 0).unwrap();
            curve.record(&observe(&target, at, &scope(), 1.0, 2).unwrap());
        }
        assert_eq!(curve.len(), 10, "the window should not grow");
        let (first, last) = curve.span().unwrap();
        assert!(last > first, "and it should hold the most recent samples");
    }

    #[test]
    fn a_transit_shows_up_in_the_recorded_curve() {
        let star = Star::SOL;
        let target = target_with_planet();
        let period_us = std::f64::consts::TAU * (AU.powi(3) / star.mu).sqrt() * 1e6;
        let mut curve = LightCurve::new(Band::V, 4000);
        for k in 0..4000 {
            let t = 40.0 * LY_US + k as f64 * period_us / 4000.0;
            let at = Coord::new(Micros::new(t as i64), 0, 0, 0).unwrap();
            curve.record(&observe(&target, at, &scope(), 1.0, 2).unwrap());
        }
        assert_eq!(curve.len(), 4000);
        assert!((curve.deepest() - 1.0186e-4).abs() < 2e-5, "deepest {}", curve.deepest());
        assert!(curve.uncertainty() > 0.0);
    }

    #[test]
    fn a_curve_plots_against_emission_time_not_arrival() {
        let target = target_with_planet();
        let mut curve = LightCurve::new(Band::V, 8);
        let at = Coord::new(Micros::new((40.0 * LY_US) as i64), 0, 0, 0).unwrap();
        let obs = observe(&target, at, &scope(), 1.0, 2).unwrap();
        curve.record(&obs);
        let (t, _) = curve.span().unwrap();
        assert!((t - obs.retarded_time * 1e-6).abs() < 1e-6);
        // Which is thirty years before the observation.
        assert!(t < at.time_f64() * 1e-6 - 9e8);
    }

    #[test]
    fn a_band_the_instrument_lacks_records_nothing() {
        let target = target_with_planet();
        let mut curve = LightCurve::new(Band::Radio, 8);
        let silicon = scope().with_bands(BandMask::SILICON);
        let at = Coord::new(Micros::new((40.0 * LY_US) as i64), 0, 0, 0).unwrap();
        curve.record(&observe(&target, at, &silicon, 1.0, 2).unwrap());
        assert!(curve.is_empty());
    }

    #[test]
    fn a_swarm_registers_where_a_planet_would_not() {
        let star = Star::SOL;
        let mut model = EmissionModel::new(star, 5);
        model.populations.push(Population {
            pole: DVec3::Z,
            semi_major: Distribution::delta(AU),
            eccentricity: Distribution::delta(0.0),
            inclination: Inclination::isotropic(),
            count: 1.5e6,
            cross_section: 1e12,
            band_response: PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        });
        let frame = SystemFrame::new(Coord::new(Micros::ORIGIN, (30.0 * LY_US) as i64, 0, 0).unwrap());
        let target = Target::new(frame, model);
        let mut curve = LightCurve::new(Band::V, 500);
        for k in 0..500 {
            let t = 40.0 * LY_US + k as f64 * 3.6e9;
            let at = Coord::new(Micros::new(t as i64), 0, 0, 0).unwrap();
            curve.record(&observe(&target, at, &scope(), 1e5, 9).unwrap());
        }
        assert!(curve.deepest() > 1e-6, "the swarm should be measurable: {}", curve.deepest());
    }
}
