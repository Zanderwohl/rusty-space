//! CIE 1931 color matching, and conversion to sRGB.

use crate::blackbody::spectral_radiance;

/// Integration limits for the visible range, nanometers.
pub const VISIBLE_NM: (f64, f64) = (380.0, 780.0);

/// Piecewise Gaussian: one sigma below the mean, another above.
#[inline]
fn lobe(x: f64, mu: f64, sigma_lo: f64, sigma_hi: f64) -> f64 {
    let t = (x - mu) / if x < mu { sigma_lo } else { sigma_hi };
    (-0.5 * t * t).exp()
}

// Wyman, Sloan & Shirley (JCGT 2013) multi-lobe fits to the CIE 1931 2-degree observer.
// Accurate to well under a percent; the y-bar integral comes out at 106.92 against the
// tabulated 106.857. Chosen over a 400-entry table because it is exact enough for color and
// costs nothing to carry.

pub fn x_bar(nm: f64) -> f64 {
    1.056 * lobe(nm, 599.8, 37.9, 31.0) + 0.362 * lobe(nm, 442.0, 16.0, 26.7)
        - 0.065 * lobe(nm, 501.1, 20.4, 26.2)
}

pub fn y_bar(nm: f64) -> f64 {
    0.821 * lobe(nm, 568.8, 46.9, 40.5) + 0.286 * lobe(nm, 530.9, 16.3, 31.1)
}

pub fn z_bar(nm: f64) -> f64 {
    1.217 * lobe(nm, 437.0, 11.8, 36.0) + 0.681 * lobe(nm, 459.0, 26.0, 13.8)
}

/// Integrate a spectral radiance function, taking wavelength in **meters**, against the
/// matching functions. Unnormalized: only ratios and chromaticity are meaningful.
pub fn xyz_from_spectral(f: impl Fn(f64) -> f64) -> [f64; 3] {
    let (lo, hi) = VISIBLE_NM;
    let mut xyz = [0.0; 3];
    let mut nm = lo;
    while nm <= hi {
        let v = f(nm * 1e-9);
        xyz[0] += v * x_bar(nm);
        xyz[1] += v * y_bar(nm);
        xyz[2] += v * z_bar(nm);
        nm += 1.0;
    }
    xyz
}

pub fn xyz_from_blackbody(temperature_k: f64) -> [f64; 3] {
    xyz_from_spectral(|l| spectral_radiance(l, temperature_k))
}

/// CIE `(x, y)` chromaticity.
pub fn chromaticity(xyz: [f64; 3]) -> (f64, f64) {
    let s = xyz[0] + xyz[1] + xyz[2];
    if s <= 0.0 { (0.0, 0.0) } else { (xyz[0] / s, xyz[1] / s) }
}

const XYZ_TO_RGB: [[f64; 3]; 3] = [
    [3.2406, -1.5372, -0.4986],
    [-0.9689, 1.8758, 0.0415],
    [0.0557, -0.2040, 1.0570],
];

const RGB_TO_XYZ: [[f64; 3]; 3] = [
    [0.4124, 0.3576, 0.1805],
    [0.2126, 0.7152, 0.0722],
    [0.0193, 0.1192, 0.9505],
];

fn apply(m: &[[f64; 3]; 3], v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2])
}

/// XYZ to linear sRGB, D65. Components may be negative for colors outside the gamut.
pub fn linear_srgb_from_xyz(xyz: [f64; 3]) -> [f64; 3] {
    apply(&XYZ_TO_RGB, xyz)
}

pub fn xyz_from_linear_srgb(rgb: [f64; 3]) -> [f64; 3] {
    apply(&RGB_TO_XYZ, rgb)
}

/// Linear to sRGB-encoded.
pub fn encode_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

pub fn decode_srgb(c: f64) -> f64 {
    if c <= 0.040_45 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Clip negatives and scale so the largest component is 1. For displaying a color whose
/// absolute brightness is carried elsewhere — see the tone mapping in `07-rendering.md`.
pub fn normalize_to_max(rgb: [f64; 3]) -> [f64; 3] {
    let clipped: [f64; 3] = std::array::from_fn(|i| rgb[i].max(0.0));
    let m = clipped.iter().cloned().fold(0.0, f64::max);
    if m <= 0.0 { [0.0; 3] } else { std::array::from_fn(|i| clipped[i] / m) }
}

/// Saturation as `(max - min) / max`. Zero is neutral.
pub fn chroma(rgb: [f64; 3]) -> f64 {
    let max = rgb.iter().cloned().fold(f64::MIN, f64::max);
    let min = rgb.iter().cloned().fold(f64::MAX, f64::min);
    if max <= 0.0 { 0.0 } else { (max - min) / max }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_matching_functions_integrate_to_the_tabulated_value() {
        let mut sum = 0.0;
        let mut nm = 380.0;
        while nm <= 780.0 {
            sum += y_bar(nm);
            nm += 1.0;
        }
        assert!((sum - 106.857).abs() < 0.2, "y-bar integral is {sum}, tabulated 106.857");
    }

    #[test]
    fn blackbody_chromaticity_tracks_the_planckian_locus() {
        for (t, x, y) in [(5772.0, 0.3250, 0.3345), (6504.0, 0.3127, 0.3290), (3000.0, 0.4369, 0.4041)] {
            let (gx, gy) = chromaticity(xyz_from_blackbody(t));
            assert!((gx - x).abs() < 0.01 && (gy - y).abs() < 0.01, "T={t}: ({gx}, {gy})");
        }
    }

    #[test]
    fn hotter_is_bluer_along_the_locus() {
        let mut prev = f64::INFINITY;
        for t in [2000.0, 3000.0, 5000.0, 8000.0, 15000.0, 30000.0] {
            let (x, _) = chromaticity(xyz_from_blackbody(t));
            assert!(x < prev, "chromaticity x must fall with temperature");
            prev = x;
        }
    }

    #[test]
    fn the_srgb_matrices_are_inverses() {
        for v in [[0.3, 0.4, 0.5], [1.0, 0.0, 0.0], [0.2, 0.9, 0.1]] {
            let back = xyz_from_linear_srgb(linear_srgb_from_xyz(v));
            for i in 0..3 {
                assert!((back[i] - v[i]).abs() < 1e-3, "{v:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn gamma_encoding_round_trips() {
        for c in [0.0, 0.001, 0.0031308, 0.05, 0.5, 1.0] {
            assert!((decode_srgb(encode_srgb(c)) - c).abs() < 1e-12, "{c}");
        }
    }

    #[test]
    fn normalizing_clips_out_of_gamut_and_peaks_at_one() {
        let n = normalize_to_max([2.0, -0.5, 1.0]);
        assert_eq!(n, [1.0, 0.0, 0.5]);
        assert_eq!(normalize_to_max([0.0, 0.0, 0.0]), [0.0; 3]);
    }
}
