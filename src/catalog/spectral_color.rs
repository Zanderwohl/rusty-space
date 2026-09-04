use bevy::color::Color;
use super::spectral::{LuminosityClass, SpectralClass, SpectralType};

impl SpectralType {
    /// Convert this spectral classification to a perceptually-accurate sRGB color.
    ///
    /// Uses effective temperature derived from spectral class, subtype, and luminosity,
    /// then maps through a B-V → sRGB lookup table computed from CIE 1931 2° color
    /// matching functions applied to real stellar spectra (Charity 2001, after Kurucz,
    /// Silva, and Pickles model atmospheres).
    pub fn to_color(&self) -> Color {
        let bv = self.b_minus_v();
        bv_to_color(bv)
    }

    /// Compute the B-V color index for this spectral type.
    ///
    /// Accounts for luminosity class: supergiants of the same spectral type are
    /// redder (higher B-V) than main-sequence stars at the same type because their
    /// lower surface gravity shifts the ionization balance.
    pub fn b_minus_v(&self) -> f32 {
        let subtype = self.subtype.unwrap_or(5.0).clamp(0.0, 9.9);

        match self.class {
            SpectralClass::O => lerp_subtype(subtype, &O_BV),
            SpectralClass::B => lerp_subtype(subtype, &B_BV),
            SpectralClass::A => lerp_subtype(subtype, &A_BV),
            SpectralClass::F => lerp_subtype(subtype, &F_BV),
            SpectralClass::G => lerp_subtype(subtype, &G_BV),
            SpectralClass::K => lerp_subtype(subtype, &K_BV),
            SpectralClass::M => lerp_subtype(subtype, &M_BV),
            SpectralClass::WN | SpectralClass::WC => {
                // Wolf-Rayet: extremely hot, bluer than O stars
                lerp(-0.35, -0.25, subtype / 9.0)
            }
            SpectralClass::C => {
                // Carbon stars: very red, B-V ~ 1.5–3.0
                lerp(1.5, 2.5, subtype / 9.0)
            }
            SpectralClass::S => {
                // S-type (ZrO): similar to late K / early M
                lerp(1.2, 1.7, subtype / 9.0)
            }
            SpectralClass::D => {
                // White dwarfs: subtype 0=hottest(~100kK), 9=coolest(~5.6kK)
                // Using the formula Teff = 50400/subtype and the BV table
                let sub = if subtype < 0.5 { 0.5 } else { subtype };
                let teff = 50400.0 / sub;
                teff_to_bv(teff)
            }
            SpectralClass::Unknown => 0.65, // solar-like default
        }
        .clamp(-0.40, 2.00)
            + self.luminosity_bv_offset()
    }

    /// Luminosity class shifts the B-V: giants and supergiants are redder
    /// at the same spectral type due to lower surface gravity.
    fn luminosity_bv_offset(&self) -> f32 {
        match self.luminosity.unwrap_or(LuminosityClass::V) {
            LuminosityClass::V | LuminosityClass::VI | LuminosityClass::VII => 0.0,
            LuminosityClass::IV => 0.02,
            LuminosityClass::III => 0.05,
            LuminosityClass::II => 0.08,
            LuminosityClass::Ib => 0.10,
            LuminosityClass::Iab => 0.12,
            LuminosityClass::Ia => 0.14,
            LuminosityClass::Ia0 => 0.16,
        }
    }
}

/// B-V color index values for subtypes 0–9 of each spectral class (main sequence).
/// Derived from Pecaut & Mamajek (2013) and Fitzgerald (1970) calibrations.
///
/// Each array has 10 entries for subtypes 0.0 through 9.0.
/// Values between subtypes are linearly interpolated.

// O0–O9: B-V ranges from about -0.33 to -0.30
const O_BV: [f32; 10] = [
    -0.33, // O0
    -0.33, // O1
    -0.32, // O2
    -0.32, // O3
    -0.32, // O4
    -0.32, // O5
    -0.31, // O6
    -0.31, // O7
    -0.31, // O8
    -0.30, // O9 (→ B0)
];

// B0–B9: B-V ranges from -0.30 to -0.02
const B_BV: [f32; 10] = [
    -0.30, // B0
    -0.26, // B1
    -0.24, // B2
    -0.20, // B3
    -0.18, // B4
    -0.16, // B5
    -0.14, // B6
    -0.12, // B7
    -0.09, // B8
    -0.04, // B9 (→ A0)
];

// A0–A9: B-V ranges from 0.00 to 0.25
const A_BV: [f32; 10] = [
    0.00, // A0
    0.02, // A1
    0.05, // A2
    0.08, // A3
    0.11, // A4
    0.14, // A5
    0.17, // A6
    0.20, // A7
    0.23, // A8
    0.25, // A9 (→ F0)
];

// F0–F9: B-V ranges from 0.30 to 0.53
const F_BV: [f32; 10] = [
    0.30, // F0
    0.33, // F1
    0.35, // F2
    0.38, // F3
    0.41, // F4
    0.44, // F5
    0.46, // F6
    0.48, // F7
    0.50, // F8
    0.53, // F9 (→ G0)
];

// G0–G9: B-V ranges from 0.58 to 0.76
const G_BV: [f32; 10] = [
    0.58, // G0
    0.60, // G1
    0.63, // G2 (Sun = 0.65)
    0.65, // G3
    0.67, // G4
    0.68, // G5
    0.70, // G6
    0.72, // G7
    0.74, // G8
    0.76, // G9 (→ K0)
];

// K0–K9: B-V ranges from 0.81 to 1.36
const K_BV: [f32; 10] = [
    0.81, // K0
    0.86, // K1
    0.92, // K2
    0.99, // K3
    1.05, // K4
    1.15, // K5
    1.22, // K6
    1.33, // K7
    1.35, // K8
    1.36, // K9 (→ M0)
];

// M0–M9: B-V ranges from 1.40 to 2.00
const M_BV: [f32; 10] = [
    1.40, // M0
    1.46, // M1
    1.49, // M2
    1.51, // M3
    1.54, // M4
    1.61, // M5
    1.72, // M6
    1.80, // M7
    1.90, // M8
    2.00, // M9
];

/// Linearly interpolate within a 10-element subtype table.
fn lerp_subtype(subtype: f32, table: &[f32; 10]) -> f32 {
    let idx = subtype.floor() as usize;
    if idx >= 9 {
        return table[9];
    }
    let frac = subtype - idx as f32;
    table[idx] * (1.0 - frac) + table[idx + 1] * frac
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Approximate B-V → effective temperature using Ballesteros (2012) formula.
/// Only used for white dwarfs where we go temperature-first.
fn teff_to_bv(teff: f32) -> f32 {
    // Ballesteros (2012): T = 4600 * (1/(0.92*BV + 1.7) + 1/(0.92*BV + 0.62))
    // Inverted numerically with a simple approximation:
    // BV ≈ -3.684e-1 + 1.236e4/T - 1.2e7/T² (Sekiguchi & Fukugita 2000 fit)
    let t = teff;
    -0.3684 + 1.236e4 / t - 1.2e7 / (t * t)
}

/// B-V color index → linear sRGB color.
///
/// Lookup table derived from CIE 1931 2° color matching functions applied to
/// Kurucz/Silva/Pickles model stellar spectra, D65 white point, sRGB primaries.
/// Data from Mitchell Charity (2001).
///
/// Table covers B-V from -0.40 to +2.00 in steps of 0.05.
fn bv_to_color(bv: f32) -> Color {
    // 49 entries: B-V = -0.40, -0.35, ..., +1.95, +2.00
    const TABLE: [(u8, u8, u8); 49] = [
        // B-V = -0.40 → deep blue (hottest O stars)
        (155, 178, 255), // -0.40
        (158, 181, 255), // -0.35
        (163, 185, 255), // -0.30
        (170, 191, 255), // -0.25
        (178, 197, 255), // -0.20
        (187, 204, 255), // -0.15
        (196, 210, 255), // -0.10
        (204, 216, 255), // -0.05
        (211, 221, 255), //  0.00
        (218, 226, 255), //  0.05
        (223, 229, 255), //  0.10
        (228, 233, 255), //  0.15
        (233, 236, 255), //  0.20
        (238, 239, 255), //  0.25
        (243, 242, 255), //  0.30
        (248, 246, 255), //  0.35
        (254, 249, 255), //  0.40
        (255, 249, 251), //  0.45
        (255, 247, 245), //  0.50
        (255, 245, 239), //  0.55
        (255, 243, 234), //  0.60
        (255, 241, 229), //  0.65
        (255, 239, 224), //  0.70
        (255, 237, 219), //  0.75
        (255, 235, 214), //  0.80
        (255, 233, 210), //  0.85
        (255, 229, 206), //  0.90
        (255, 230, 202), //  0.95
        (255, 229, 198), //  1.00
        (255, 227, 195), //  1.05
        (255, 226, 191), //  1.10
        (255, 224, 187), //  1.15
        (255, 223, 184), //  1.20
        (255, 221, 180), //  1.25
        (255, 218, 173), //  1.30
        (255, 218, 173), //  1.35
        (255, 216, 169), //  1.40
        (255, 214, 165), //  1.45
        (255, 213, 161), //  1.50
        (255, 210, 156), //  1.55
        (255, 208, 150), //  1.60
        (255, 204, 143), //  1.65
        (255, 200, 133), //  1.70
        (255, 193, 120), //  1.75
        (255, 183, 101), //  1.80
        (255, 169, 75),  //  1.85
        (255, 149, 35),  //  1.90
        (255, 123, 0),   //  1.95
        (255, 82, 0),    //  2.00 → deep red (coolest M stars)
    ];

    let bv_clamped = bv.clamp(-0.40, 2.00);
    let t = (bv_clamped + 0.40) / 0.05;
    let idx = (t.floor() as usize).min(TABLE.len() - 2);
    let frac = t - idx as f32;

    let (r0, g0, b0) = TABLE[idx];
    let (r1, g1, b1) = TABLE[idx + 1];

    let r = lerp(r0 as f32, r1 as f32, frac) / 255.0;
    let g = lerp(g0 as f32, g1 as f32, frac) / 255.0;
    let b = lerp(b0 as f32, b1 as f32, frac) / 255.0;

    Color::linear_rgba(srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), 1.0)
}

/// sRGB gamma decode: convert sRGB [0,1] → linear [0,1].
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
mod tests {
    use crate::catalog::spectral::SpectralType;

    #[test]
    fn sun_is_warm_white() {
        let sun = SpectralType::parse("G2V").unwrap();
        let color = sun.to_color();
        // Sun should be warm white / very slightly yellow-orange
        let rgba = color.to_linear();
        // Red > Green > Blue for a warm white
        assert!(rgba.red > rgba.blue);
        assert!(rgba.green > rgba.blue);
    }

    #[test]
    fn hot_star_is_blue() {
        let rigel = SpectralType::parse("B8Ia").unwrap();
        let color = rigel.to_color();
        let rgba = color.to_linear();
        // Blue should dominate for a B star
        assert!(rgba.blue > rgba.red);
    }

    #[test]
    fn cool_star_is_red() {
        let betelgeuse = SpectralType::parse("M2Ia").unwrap();
        let color = betelgeuse.to_color();
        let rgba = color.to_linear();
        // Red should strongly dominate for an M star
        assert!(rgba.red > rgba.green);
        assert!(rgba.red > rgba.blue);
    }

    #[test]
    fn color_temperature_monotonic() {
        // Going O→M the blue component should decrease
        let types = ["O5V", "B5V", "A0V", "F5V", "G2V", "K5V", "M5V"];
        let blues: Vec<f32> = types
            .iter()
            .map(|t| {
                let s = SpectralType::parse(t).unwrap();
                s.to_color().to_linear().blue
            })
            .collect();

        for i in 1..blues.len() {
            assert!(
                blues[i] <= blues[i - 1] + 0.01,
                "Blue should decrease from {} to {}: {} vs {}",
                types[i - 1],
                types[i],
                blues[i - 1],
                blues[i]
            );
        }
    }

    #[test]
    fn white_dwarf_hot() {
        let wd = SpectralType::parse("DA2").unwrap();
        let color = wd.to_color();
        let rgba = color.to_linear();
        // DA2 ≈ 25,200K — very hot, should be blue-white
        assert!(rgba.blue > rgba.red);
    }

    #[test]
    fn wolf_rayet_very_blue() {
        let wr = SpectralType::parse("WN5").unwrap();
        let color = wr.to_color();
        let rgba = color.to_linear();
        assert!(rgba.blue > rgba.red);
    }

    #[test]
    fn supergiant_redder_than_dwarf() {
        let dwarf = SpectralType::parse("G2V").unwrap();
        let supergiant = SpectralType::parse("G2Ia").unwrap();
        // Supergiant should have a higher B-V (redder)
        assert!(supergiant.b_minus_v() > dwarf.b_minus_v());
    }
}
