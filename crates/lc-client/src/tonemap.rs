//! Compressing sixty stops of physical flux into a display.

use em_spectra::{BandMapping, PerBand};
use glam::Vec3;

/// Rec. 709 luminance weights.
const LUMA: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

/// What a pixel gets, what the glow pass gets, and how far the source really was from the
/// window.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Shaded {
    /// Hue and saturation, largest channel 1. Carried separately so brightness can be
    /// applied differently to a surface and to a point.
    pub chroma: Vec3,
    /// Position within the displayed window, `[0, 1]`.
    pub value: f32,
    /// Stops above the top of the window. Zero when the source fits.
    pub glow: f32,
    /// Signed stops relative to the reference; negative below it, and unclamped.
    ///
    /// The window is two or three stops wide, so a source ten stops down has a `value` of
    /// zero and would be invisible. A point source is not: it is *small*. Keeping the true
    /// offset lets a renderer map it to size over a much wider range than the colour spans.
    pub stops: f32,
}

impl Shaded {
    /// Colour of an extended surface: chroma at the windowed brightness.
    pub fn colour(&self) -> Vec3 {
        self.chroma * self.value
    }

    /// Colour of a point source, spread over `visible_stops` rather than the window's width.
    pub fn point_colour(&self, visible_stops: f32) -> Vec3 {
        self.chroma * self.point_brightness(visible_stops)
    }

    /// How bright a point source reads, `[0, 1]`, across a range wider than the window.
    pub fn point_brightness(&self, visible_stops: f32) -> f32 {
        if visible_stops <= 0.0 {
            return self.value;
        }
        (1.0 + self.stops / visible_stops).clamp(0.0, 1.0)
    }

    pub fn is_dark(&self) -> bool {
        self.chroma == Vec3::ZERO
    }
}

/// Where the displayed range sits, and how wide it is.
///
/// Physical flux spans something like sixty stops and no tone curve maps that to a screen.
/// The mapping is therefore logarithmic — which is what magnitudes already are — compressed
/// into a two or three stop window, with everything above it routed into glow rather than
/// into pixel value. A very bright source is not a brighter white; it is a wider halo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneMap {
    /// Luminance that maps to the top of the window.
    pub reference: f32,
    /// Width of the window, in stops.
    pub stops: f32,
}

impl Default for ToneMap {
    fn default() -> Self {
        Self { reference: 1.0, stops: 2.5 }
    }
}

impl ToneMap {
    /// Shift the window by `stops`, as an exposure control would.
    #[must_use]
    pub fn exposed(mut self, stops: f32) -> Self {
        self.reference *= 2f32.powf(-stops);
        self
    }

    pub fn shade(&self, radiance: &PerBand<f32>, mapping: &BandMapping) -> Shaded {
        let linear = Vec3::from_array(mapping.apply(radiance));
        let luminance = linear.dot(LUMA);
        if !(luminance > 0.0) || self.reference <= 0.0 || self.stops <= 0.0 {
            return Shaded::default();
        }
        // Stops relative to the top of the window: 0 at the reference, negative below.
        let above = (luminance / self.reference).log2();
        let peak = linear.max_element();
        Shaded {
            chroma: if peak > 0.0 { linear / peak } else { Vec3::ONE },
            value: (above / self.stops + 1.0).clamp(0.0, 1.0),
            glow: above.max(0.0),
            stops: above,
        }
    }
}

#[cfg(test)]
mod tests {
    use em_spectra::{Band, presets};

    use super::*;

    fn flat(v: f32) -> PerBand<f32> {
        PerBand::splat(v)
    }

    #[test]
    fn a_source_at_the_reference_fills_the_window_without_glowing() {
        let m = presets::natural();
        let tone = ToneMap { reference: 1.0, stops: 2.5 };
        // The radiance whose luminance is exactly the reference.
        let scale = 1.0 / Vec3::from_array(m.apply(&flat(1.0))).dot(LUMA);
        let at_reference = tone.shade(&flat(scale), &m);
        assert!(at_reference.glow.abs() < 1e-5, "glow {}", at_reference.glow);
        assert!(at_reference.colour().max_element() > 0.99);
    }

    #[test]
    fn brightness_beyond_the_window_becomes_glow_not_a_whiter_white() {
        let m = presets::natural();
        let tone = ToneMap::default();
        let bright = tone.shade(&flat(1e3), &m);
        let brighter = tone.shade(&flat(1e6), &m);
        assert_eq!(bright.colour(), brighter.colour(), "the pixel has nowhere left to go");
        assert!(brighter.glow > bright.glow + 9.0, "a thousandfold is ten stops of glow");
    }

    #[test]
    fn the_window_is_logarithmic_and_only_as_wide_as_it_says() {
        let m = presets::natural();
        let tone = ToneMap { reference: 1.0, stops: 2.0 };
        let l = |v: f32| tone.shade(&flat(v), &m).colour().max_element();
        let top = 1.0 / Vec3::from_array(m.apply(&flat(1.0))).dot(LUMA);
        // Two stops down is the bottom of the window; anything below is black.
        assert!(l(top) > 0.99);
        assert!((l(top / 2.0) - 0.5).abs() < 0.01, "one stop down should be half way");
        assert_eq!(l(top / 8.0), 0.0, "three stops down is outside a two-stop window");
    }

    #[test]
    fn colour_survives_the_compression() {
        let m = presets::natural();
        let mut red = PerBand::splat(0.0f32);
        red[Band::R] = 5.0;
        let shaded = ToneMap::default().shade(&red, &m);
        assert!(shaded.chroma.x > shaded.chroma.y && shaded.chroma.x > shaded.chroma.z);
    }

    #[test]
    fn darkness_and_degenerate_settings_give_black_rather_than_nan() {
        let m = presets::natural();
        for tone in [ToneMap::default(), ToneMap { reference: 0.0, stops: 2.0 }, ToneMap { reference: 1.0, stops: 0.0 }] {
            let s = tone.shade(&flat(0.0), &m);
            assert!(s.colour().is_finite() && s.glow.is_finite() && s.stops.is_finite());
        }
        assert!(ToneMap::default().shade(&flat(0.0), &m).is_dark());
    }

    /// A source far below the window is invisible as a surface and still drawable as a
    /// point, which is what a star field needs.
    #[test]
    fn a_point_source_stays_visible_below_the_window() {
        let m = presets::natural();
        let tone = ToneMap { reference: 1.0, stops: 2.5 };
        let faint = tone.shade(&flat(1e-2), &m);
        assert_eq!(faint.value, 0.0, "six stops down is outside a 2.5 stop window");
        assert!(faint.colour() == Vec3::ZERO);
        assert!(faint.point_brightness(12.0) > 0.3, "but a point is merely dim");
        assert!(faint.point_colour(12.0).length() > 0.0);
        // And further down is dimmer still, rather than equally black.
        let fainter = tone.shade(&flat(1e-4), &m);
        assert!(fainter.point_brightness(12.0) < faint.point_brightness(12.0));
        // Past the visible range it does go out.
        assert_eq!(tone.shade(&flat(1e-9), &m).point_brightness(12.0), 0.0);
    }

    #[test]
    fn exposure_slides_the_window() {
        let m = presets::natural();
        let base = ToneMap::default();
        let brighter = base.exposed(2.0);
        // A source already at the top of the window has nowhere to go, so test a dim one.
        let v = |t: ToneMap| t.shade(&flat(0.1), &m).colour().max_element();
        assert_eq!(v(base), 0.0, "0.1 is more than 2.5 stops below the reference");
        assert!(v(brighter) > 0.4, "opening up two stops must bring it into the window");
    }
}
