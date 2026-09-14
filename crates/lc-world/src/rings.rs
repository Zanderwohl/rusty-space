//! Ring systems, as measured.
//!
//! Radii are from the IAU and Cassini/Voyager results; optical depths are area-averaged over
//! each band and are the roughest number here, because a real ring's depth varies by an order
//! of magnitude across its own width. What the values are for is the *ordering*: Saturn's rings
//! are a structure, Uranus's are threads, Jupiter's are dust you would never see.
//!
//! Poles are not here. `em-sim`'s presets already carry every body's IAU rotation, and a second
//! copy of a pole is a second chance to have it wrong.

use std::f64::consts::PI;

use glam::DVec3;

use crate::distribution::{Distribution, Inclination};
use crate::population::Population;

/// One annulus of roughly uniform opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub name: &'static str,
    pub inner_m: f64,
    pub outer_m: f64,
    /// Normal optical depth, area-averaged across the band.
    pub optical_depth: f64,
}

impl Band {
    /// Area of the annulus, m^2.
    pub fn area_m2(&self) -> f64 {
        PI * (self.outer_m * self.outer_m - self.inner_m * self.inner_m).max(0.0)
    }

    /// Cross-section actually blocking light: the area times what fraction of it is filled.
    ///
    /// `1 - exp(-tau)` rather than `tau`, for the same reason the populations use it: past a
    /// depth of a few tenths the particles start shadowing each other, and Saturn's B ring is
    /// well past that.
    pub fn cross_section_m2(&self) -> f64 {
        self.area_m2() * (1.0 - (-self.optical_depth).exp())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RingSystem {
    /// The `em-sim` body id this belongs to.
    pub body_id: &'static str,
    pub bands: &'static [Band],
    /// Geometric albedo of the particles. Saturn's are water ice and bright; everything else in
    /// the solar system is dark dust and rubble.
    pub albedo: f64,
}

impl RingSystem {
    pub fn inner_m(&self) -> f64 {
        self.bands.iter().map(|b| b.inner_m).fold(f64::INFINITY, f64::min)
    }

    pub fn outer_m(&self) -> f64 {
        self.bands.iter().map(|b| b.outer_m).fold(0.0, f64::max)
    }

    /// Optical depth at a radius, zero in the gaps and outside.
    pub fn depth_at(&self, radius_m: f64) -> f64 {
        self.bands
            .iter()
            .find(|b| radius_m >= b.inner_m && radius_m < b.outer_m)
            .map(|b| b.optical_depth)
            .unwrap_or(0.0)
    }

    /// Total light-blocking cross-section, m^2. Seen face-on this is what reflects.
    pub fn cross_section_m2(&self) -> f64 {
        self.bands.iter().map(|b| b.cross_section_m2()).sum()
    }

    /// As a population, for the parts of the model that take one.
    ///
    /// Rings are flat to about a part in ten million — metres thick across a hundred thousand
    /// kilometres — so the inclination spread is nominal rather than measured. It exists so the
    /// distribution is not degenerate.
    pub fn population(&self, pole: DVec3) -> Population {
        let (inner, outer) = (self.inner_m(), self.outer_m());
        // One square metre a particle, so the count is the cross-section. Real ring particles
        // run from centimetres to metres and nothing here depends on which.
        let cross_section = 1.0;
        Population {
            pole,
            semi_major: Distribution::uniform(inner, outer, 9),
            eccentricity: Distribution::uniform(0.0, 0.002, 3),
            inclination: Inclination::uniform_angle(0.0, 1.0e-4, 4),
            count: self.cross_section_m2() / cross_section,
            cross_section,
            band_response: em_spectra::PerBand::splat(1.0),
            radiating_ratio: Population::SPHERICAL,
        }
    }
}

/// Saturn: the one ring system that is a structure rather than a trace.
///
/// The B ring is optically thick enough that its depth is a lower bound — Cassini found parts
/// past 5 — and the Cassini Division is a gap only by comparison.
pub const SATURN: &[Band] = &[
    Band { name: "C", inner_m: 7.4658e7, outer_m: 9.2000e7, optical_depth: 0.10 },
    Band { name: "B", inner_m: 9.2000e7, outer_m: 1.17580e8, optical_depth: 1.50 },
    Band { name: "Cassini Division", inner_m: 1.17580e8, outer_m: 1.22170e8, optical_depth: 0.08 },
    Band { name: "A", inner_m: 1.22170e8, outer_m: 1.36775e8, optical_depth: 0.50 },
];

/// Jupiter: dust, at an optical depth of parts per million. Not visible, and correctly so.
pub const JUPITER: &[Band] = &[
    Band { name: "halo", inner_m: 9.2000e7, outer_m: 1.22500e8, optical_depth: 1.0e-6 },
    Band { name: "main", inner_m: 1.22500e8, outer_m: 1.29000e8, optical_depth: 3.0e-6 },
    Band { name: "Amalthea gossamer", inner_m: 1.29000e8, outer_m: 1.82000e8, optical_depth: 1.0e-7 },
    Band { name: "Thebe gossamer", inner_m: 1.82000e8, outer_m: 2.26000e8, optical_depth: 1.0e-8 },
];

/// Uranus: narrow, dark and nearly empty between. The area average is far below any one ring's
/// own depth, because most of the annulus is nothing.
pub const URANUS: &[Band] = &[
    Band { name: "inner", inner_m: 3.7850e7, outer_m: 5.0000e7, optical_depth: 4.0e-3 },
    Band { name: "epsilon", inner_m: 5.1100e7, outer_m: 5.1200e7, optical_depth: 1.0 },
];

/// Neptune: arcs rather than rings, which this does not model. The depths are the arcs' own,
/// spread over the whole circumference.
pub const NEPTUNE: &[Band] = &[
    Band { name: "Galle", inner_m: 4.1900e7, outer_m: 4.2900e7, optical_depth: 1.0e-4 },
    Band { name: "Le Verrier", inner_m: 5.3150e7, outer_m: 5.3250e7, optical_depth: 1.0e-2 },
    Band { name: "Lassell", inner_m: 5.3250e7, outer_m: 5.7200e7, optical_depth: 1.0e-4 },
    Band { name: "Adams", inner_m: 6.2900e7, outer_m: 6.2970e7, optical_depth: 3.0e-2 },
];

pub const ALL: [RingSystem; 4] = [
    RingSystem { body_id: "Saturn", bands: SATURN, albedo: 0.50 },
    RingSystem { body_id: "Jupiter", bands: JUPITER, albedo: 0.05 },
    RingSystem { body_id: "Uranus", bands: URANUS, albedo: 0.03 },
    RingSystem { body_id: "Neptune", bands: NEPTUNE, albedo: 0.03 },
];

pub fn for_body(id: &str) -> Option<&'static RingSystem> {
    ALL.iter().find(|r| r.body_id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const R_SATURN: f64 = 5.8232e7;

    #[test]
    fn saturns_rings_sit_where_they_are_measured_to() {
        let s = for_body("Saturn").unwrap();
        // 1.28 to 2.35 Saturn radii, which is where the C and A ring edges are.
        assert!((s.inner_m() / R_SATURN - 1.282).abs() < 0.01, "{}", s.inner_m() / R_SATURN);
        assert!((s.outer_m() / R_SATURN - 2.349).abs() < 0.01, "{}", s.outer_m() / R_SATURN);
        assert!(s.bands.windows(2).all(|w| w[0].outer_m <= w[1].inner_m), "bands must not overlap");
    }

    /// The Cassini Division is a gap only by comparison, and the model should say so rather
    /// than making it empty.
    #[test]
    fn the_cassini_division_is_thin_but_not_nothing() {
        let s = for_body("Saturn").unwrap();
        let at = |r: f64| s.depth_at(r * R_SATURN);
        assert!(at(1.8) > 1.0, "the B ring is optically thick: {}", at(1.8));
        assert!(at(2.06) < at(1.8) / 10.0, "the division is far thinner: {}", at(2.06));
        assert!(at(2.06) > 0.0, "but it is not empty");
        assert!(at(2.2) > at(2.06) * 4.0, "and the A ring is thicker again: {}", at(2.2));
        assert_eq!(at(3.0), 0.0, "outside the A ring there is nothing");
        assert_eq!(at(1.0), 0.0, "and nothing inside the C ring either");
    }

    /// The ordering the data exists for: one structure, two traces, one that is not there.
    #[test]
    fn the_four_systems_are_orders_of_magnitude_apart() {
        let covering = |id: &str| {
            let r = for_body(id).unwrap();
            r.cross_section_m2() / (std::f64::consts::PI * r.outer_m() * r.outer_m())
        };
        let (saturn, uranus, neptune, jupiter) =
            (covering("Saturn"), covering("Uranus"), covering("Neptune"), covering("Jupiter"));
        assert!(saturn > 0.3, "Saturn's rings fill most of their annulus: {saturn}");
        assert!(uranus < saturn / 50.0, "Uranus's are threads: {uranus}");
        assert!(neptune < uranus, "Neptune's are fainter still: {neptune}");
        assert!(jupiter < 1.0e-5, "Jupiter's are dust: {jupiter}");
    }

    /// Saturn's rings reflect about as much as the planet does, which is the missing
    /// three quarters of a magnitude in its brightness.
    #[test]
    fn saturns_rings_reflect_a_planets_worth_of_light() {
        let s = for_body("Saturn").unwrap();
        let disc = std::f64::consts::PI * R_SATURN * R_SATURN;
        let ratio = s.cross_section_m2() * s.albedo / (disc * 0.499);
        assert!(ratio > 1.0 && ratio < 8.0, "rings against planet: {ratio}");
    }

    #[test]
    fn a_ring_becomes_a_population_with_the_pole_it_was_given() {
        let s = for_body("Saturn").unwrap();
        let pole = DVec3::new(0.0, 0.6, 0.8).normalize();
        let p = s.population(pole);
        assert_eq!(p.pole, pole);
        assert!(p.count > 0.0 && p.covering_fraction() > 0.0);
        // Flat: a ring is metres thick across a hundred thousand kilometres.
        assert!(p.inclination.sky_density(0.0) > p.inclination.sky_density(0.01) * 100.0);
    }

    #[test]
    fn a_body_with_no_rings_has_none() {
        assert!(for_body("Earth").is_none());
        assert!(for_body("Luna").is_none());
    }
}
