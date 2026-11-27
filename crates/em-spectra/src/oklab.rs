//! Björn Ottosson's OKLab (2020): a perceptual space where equal steps look equal, for mapping
//! light to display colors without the browns and grays sRGB gives dim colors.

/// Linear sRGB to OKLab `[L, a, b]`; `L` runs 0 to 1 from black to white.
pub fn from_linear_srgb([r, g, b]: [f64; 3]) -> [f64; 3] {
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

/// OKLab to linear sRGB. Components fall outside `[0, 1]` for colors outside the gamut.
pub fn to_linear_srgb([l, a, b]: [f64; 3]) -> [f64; 3] {
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);
    [
        4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_,
        -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701_0 * s_,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_is_lightness_one_with_no_chroma_and_colors_round_trip() {
        let [l, a, b] = from_linear_srgb([1.0, 1.0, 1.0]);
        assert!((l - 1.0).abs() < 1e-4 && a.abs() < 1e-4 && b.abs() < 1e-4, "{l} {a} {b}");
        for c in [[0.8, 0.2, 0.1], [0.05, 0.3, 0.9], [0.5, 0.5, 0.5]] {
            let back = to_linear_srgb(from_linear_srgb(c));
            assert!(c.iter().zip(back).all(|(x, y)| (x - y).abs() < 1e-6), "{c:?} came back {back:?}");
        }
    }
}
