//! The concentric circles that give the plane a scale.

use em_plot::scale::Scale;

use crate::snapshot::{M_PER_AU, M_PER_LY};

/// At most this many rings. More than a handful stops being a scale and starts being a texture.
pub const MAX_RINGS: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct Ring {
    pub radius_m: f64,
    pub label: String,
}

/// Decade rings spanning `[inner_m, outer_m]`, at most `max` of them.
///
/// `em_plot::Scale::Log10` already chooses powers of ten and already thins by a whole-decade
/// step when the range is wider than the count allows, which is exactly what fifteen orders of
/// magnitude needs; it also already survives a lower bound of zero, which is the case a map
/// centred on one of its own items hits every time. None of that is worth writing twice.
///
/// What this adds is the widening. `ticks` keeps only what falls strictly inside the range, so
/// asking it for `[1.2e11, 4.5e11]` returns nothing at all — both decades that frame the view
/// are outside it. Rounding out to whole decades first is what makes the outermost ring, the
/// one that frames the picture, the one that is actually drawn.
pub fn decades(inner_m: f64, outer_m: f64, max: usize) -> Vec<Ring> {
    let outer = outer_m.max(inner_m);
    if !outer.is_finite() || outer <= 0.0 || max == 0 {
        return Vec::new();
    }
    // A zero or negative inner bound is ordinary: the focus is usually on something. Start a
    // few decades in from the outer edge rather than at the first positive float.
    let lo = match inner_m.is_finite() && inner_m > 0.0 {
        true => 10f64.powf(inner_m.log10().floor()),
        false => 10f64.powf(outer.log10().floor() - max as f64 + 1.0),
    };
    let hi = 10f64.powf(outer.log10().ceil());
    Scale::Log10
        .ticks((lo, hi), max)
        .into_iter()
        .map(|radius_m| Ring { radius_m, label: label_m(radius_m) })
        .collect()
}

/// A distance in whatever unit a reader can hold it in.
///
/// The same ladder `lc_client::hud::span` uses, one step longer: a map goes further out than a
/// flight readout ever does.
pub fn label_m(metres: f64) -> String {
    match metres {
        m if m < 1.0e3 => format!("{m:.0} m"),
        m if m < 1.0e9 => format!("{:.0} thousand km", m / 1.0e6),
        m if m < 0.1 * M_PER_AU => format!("{:.0} million km", m / 1.0e9),
        m if m < 0.2 * M_PER_LY => format!("{:.0} AU", m / M_PER_AU),
        m => format!("{:.0} ly", m / M_PER_LY),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radii(rings: &[Ring]) -> Vec<f64> {
        rings.iter().map(|r| r.radius_m).collect()
    }

    /// Every ring is a power of ten. `nice_ticks`'s 1-2-5 spacings are right for a chart axis
    /// and wrong here: an order of magnitude is what the rings are for.
    #[test]
    fn every_ring_is_a_decade() {
        for radius in radii(&decades(1.0e6, 1.0e16, MAX_RINGS)) {
            let exponent = radius.log10();
            assert!((exponent - exponent.round()).abs() < 1e-9, "{radius:e} is not a decade");
        }
    }

    /// Fifteen orders of magnitude thin out rather than returning fifteen rings.
    #[test]
    fn a_huge_range_thins_instead_of_flooding() {
        let rings = decades(1.0, 1.0e18, MAX_RINGS);
        assert!(rings.len() <= MAX_RINGS, "{} rings", rings.len());
        assert!(rings.len() >= 3, "thinned away to {}", rings.len());
    }

    /// The outer ring frames the picture, so it has to survive. This is what the widening is
    /// for: `Scale::ticks` on the raw extent drops both decades that bracket the view, and a
    /// map framed between 1.2e11 and 4.5e11 came back with no rings at all.
    #[test]
    fn a_range_inside_one_decade_still_gets_its_frame() {
        let rings = decades(1.2e11, 4.5e11, MAX_RINGS);
        assert!(!rings.is_empty(), "a view between two decades drew no scale");
        let outermost = radii(&rings).into_iter().fold(0.0, f64::max);
        assert!(outermost >= 4.5e11, "outermost ring {outermost:e} is inside the view");
    }

    /// The focus is usually *on* something, so the nearest item is zero away. A log of zero is
    /// negative infinity and the ring list came back empty.
    #[test]
    fn a_focus_sitting_on_something_still_gets_rings() {
        let rings = decades(0.0, 1.0e13, MAX_RINGS);
        assert!(!rings.is_empty());
        for radius in radii(&rings) {
            assert!(radius.is_finite() && radius > 0.0, "{radius:e}");
        }
    }

    #[test]
    fn nothing_to_draw_draws_nothing() {
        assert!(decades(0.0, 0.0, MAX_RINGS).is_empty());
        assert!(decades(1.0, f64::INFINITY, MAX_RINGS).is_empty());
        assert!(decades(1.0, 1.0e12, 0).is_empty());
    }

    #[test]
    fn a_label_is_in_the_unit_that_reads() {
        assert_eq!(label_m(9.0e8), "900 thousand km");
        assert_eq!(label_m(1.0e9), "1 million km");
        assert_eq!(label_m(M_PER_AU), "1 AU");
        assert_eq!(label_m(10.0 * M_PER_AU), "10 AU");
        assert_eq!(label_m(M_PER_LY), "1 ly");
        assert_eq!(label_m(100.0 * M_PER_LY), "100 ly");
    }
}
