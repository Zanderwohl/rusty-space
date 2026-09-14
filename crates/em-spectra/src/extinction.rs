//! Interstellar extinction, and the reddening-versus-temperature degeneracy.

use crate::bands::{Band, PerBand};

/// `A_lambda / A_V` for a standard `R_V = 3.1` diffuse ISM sightline.
///
/// The radio entry is not exactly zero, but it is ten orders of magnitude below V, which is
/// what makes 21 cm see through clouds that are opaque in the optical.
pub const RATIO: PerBand<f64> = PerBand::new([1.32, 1.00, 0.75, 0.48, 0.11, 0.06, 1e-10]);

/// Mean visual extinction per kiloparsec in the galactic plane, magnitudes.
///
/// Reference only. Sightline extinction is not modelled: occlusion currently lives on a
/// source's own emission shell, and extended interstellar dust is deferred with the special
/// zones it belongs to.
pub const A_V_PER_KPC: f64 = 1.8;

/// Extinction in a band, magnitudes, for a sightline of `a_v` visual magnitudes.
#[inline]
pub fn extinction_mag(band: Band, a_v: f64) -> f64 {
    a_v * RATIO[band]
}

/// Surviving fraction of flux through `a_v` magnitudes of visual extinction.
#[inline]
pub fn transmission(band: Band, a_v: f64) -> f64 {
    10f64.powf(-0.4 * extinction_mag(band, a_v))
}

/// Colour excess `E(b1 - b2)` produced by `a_v` magnitudes of extinction.
#[inline]
pub fn colour_excess(b1: Band, b2: Band, a_v: f64) -> f64 {
    a_v * (RATIO[b1] - RATIO[b2])
}

/// A colour axis of a colour-colour diagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour(pub Band, pub Band);

impl Colour {
    pub const BV: Self = Self(Band::B, Band::V);
    pub const VI: Self = Self(Band::V, Band::I);
    pub const VK: Self = Self(Band::V, Band::K);

    /// Reddening per unit `A_V` along this axis.
    #[inline]
    pub fn reddening(self) -> f64 {
        RATIO[self.0] - RATIO[self.1]
    }
}

/// A main-sequence reference point. Colours are approximate, adequate for locus geometry.
#[derive(Clone, Copy, Debug)]
pub struct MsPoint {
    pub class: &'static str,
    pub bv: f64,
    pub vi: f64,
    pub vk: f64,
}

impl MsPoint {
    pub fn colour(&self, c: Colour) -> f64 {
        match c {
            Colour::BV => self.bv,
            Colour::VI => self.vi,
            Colour::VK => self.vk,
            _ => panic!("no tabulated colour for {c:?}"),
        }
    }
}

pub const MAIN_SEQUENCE: [MsPoint; 11] = [
    MsPoint { class: "A0V", bv: 0.00, vi: 0.00, vk: 0.00 },
    MsPoint { class: "A5V", bv: 0.15, vi: 0.17, vk: 0.38 },
    MsPoint { class: "F0V", bv: 0.29, vi: 0.33, vk: 0.70 },
    MsPoint { class: "F5V", bv: 0.44, vi: 0.48, vk: 1.03 },
    MsPoint { class: "G0V", bv: 0.59, vi: 0.64, vk: 1.41 },
    MsPoint { class: "G5V", bv: 0.68, vi: 0.72, vk: 1.60 },
    MsPoint { class: "K0V", bv: 0.82, vi: 0.87, vk: 1.96 },
    MsPoint { class: "K5V", bv: 1.15, vi: 1.33, vk: 2.85 },
    MsPoint { class: "M0V", bv: 1.41, vi: 1.84, vk: 3.65 },
    MsPoint { class: "M2V", bv: 1.50, vi: 2.15, vk: 4.11 },
    MsPoint { class: "M5V", bv: 1.61, vi: 2.89, vk: 5.96 },
];

/// Direction of the reddening vector in an `(x, y)` colour-colour diagram, degrees.
pub fn reddening_angle_deg(x: Colour, y: Colour) -> f64 {
    y.reddening().atan2(x.reddening()).to_degrees()
}

/// Direction of the stellar locus between two reference points, degrees.
pub fn locus_angle_deg(x: Colour, y: Colour, a: &MsPoint, b: &MsPoint) -> f64 {
    (b.colour(y) - a.colour(y)).atan2(b.colour(x) - a.colour(x)).to_degrees()
}

/// Angle between the reddening vector and the locus, degrees.
///
/// This is what decides whether photometry can tell a reddened star from a cool one. Near
/// zero the two are degenerate and no integration time helps, because the information is not
/// in the measurement.
pub fn degeneracy_separation_deg(x: Colour, y: Colour, a: &MsPoint, b: &MsPoint) -> f64 {
    (locus_angle_deg(x, y, a, b) - reddening_angle_deg(x, y)).abs()
}

/// Length of the reddening vector per unit `A_V` in a diagram — how far one magnitude of
/// extinction moves a star, and therefore the signal available against measurement noise.
pub fn reddening_displacement(x: Colour, y: Colour) -> f64 {
    x.reddening().hypot(y.reddening())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(class: &str) -> &'static MsPoint {
        MAIN_SEQUENCE.iter().find(|p| p.class == class).unwrap()
    }

    #[test]
    fn the_transmission_table() {
        for (band, a_v, want) in [
            (Band::B, 1.0, 0.2965),
            (Band::V, 1.0, 0.3981),
            (Band::R, 1.0, 0.5012),
            (Band::I, 1.0, 0.6427),
            (Band::K, 1.0, 0.9036),
            (Band::B, 5.0, 0.0023),
            (Band::V, 5.0, 0.0100),
            (Band::I, 5.0, 0.1096),
            (Band::K, 5.0, 0.6026),
            (Band::ThermalIr, 5.0, 0.7586),
        ] {
            let got = transmission(band, a_v);
            assert!((got - want).abs() < 5e-4, "{} at A_V={a_v}: {got} vs {want}", band.name());
        }
    }

    #[test]
    fn radio_passes_through_a_cloud_that_extinguishes_everything_else() {
        assert!(transmission(Band::V, 5.0) < 0.02);
        assert!(transmission(Band::Radio, 5.0) > 0.999_999);
        assert!(transmission(Band::Radio, 1000.0) > 0.999_999);
    }

    #[test]
    fn extinction_is_monotonic_with_wavelength() {
        for pair in Band::ALL.windows(2) {
            assert!(RATIO[pair[0]] > RATIO[pair[1]], "{:?} must redden", pair);
        }
    }

    #[test]
    fn the_reddening_vector_matches_the_extinction_law() {
        assert!((Colour::BV.reddening() - 0.32).abs() < 1e-12);
        assert!((Colour::VI.reddening() - 0.52).abs() < 1e-12);
        assert!((Colour::VK.reddening() - 0.89).abs() < 1e-12);
        let slope = Colour::BV.reddening() / Colour::VI.reddening();
        assert!((slope - 0.6154).abs() < 1e-3, "{slope}");
    }

    /// Corrects the design note, which claimed K settles the degeneracy decisively. It does
    /// not: a longer baseline buys displacement against noise, not angle, and over the blind
    /// zone the locus runs along the reddening vector in both diagrams.
    #[test]
    fn adding_k_does_not_rescue_the_blind_zone() {
        let vi = reddening_displacement(Colour::VI, Colour::BV);
        let vk = reddening_displacement(Colour::VK, Colour::BV);
        assert!((vk / vi - 1.55).abs() < 0.02, "V-K reach is {}x, not 1.55", vk / vi);

        let (k0, m0) = (ms("K0V"), ms("M0V"));
        for (label, x) in [("V-I", Colour::VI), ("V-K", Colour::VK)] {
            let sep = degeneracy_separation_deg(x, Colour::BV, k0, m0);
            assert!(sep < 1.0, "{label} over K0-M0 is {sep} deg, and should be degenerate");
        }
    }

    #[test]
    fn the_blind_zone_is_late_k_to_early_m() {
        // Hot and solar-type stars separate; K0 through M0 does not.
        let sep = |a, b| degeneracy_separation_deg(Colour::VI, Colour::BV, ms(a), ms(b));
        for (a, b) in [("A0V", "A5V"), ("F0V", "F5V"), ("G5V", "K0V")] {
            assert!(sep(a, b) > 9.0, "{a}->{b} should separate: {}", sep(a, b));
        }
        for (a, b) in [("K0V", "K5V"), ("K5V", "M0V")] {
            assert!(sep(a, b) < 5.0, "{a}->{b} is the blind zone: {}", sep(a, b));
        }
        // Late M turns away from the reddening direction and separates again.
        assert!(sep("M2V", "M5V") > 20.0);
    }

    #[test]
    fn colour_excess_reddens_a_star_toward_a_cooler_reading() {
        use crate::colour_index::teff_from_bv;
        let intrinsic = 0.0; // A0
        let reddened = intrinsic + colour_excess(Band::B, Band::V, 0.9375);
        assert!((reddened - 0.30).abs() < 1e-3, "{reddened}");
        assert!((teff_from_bv(reddened) - 7462.0).abs() < 5.0, "an A0 must read as an F star");
    }
}
