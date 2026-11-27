//! Colors the panels and views share.

use bevy_egui::egui::Color32;
use em_spectra::{blackbody, cie, presets};

pub const SELECTED: Color32 = Color32::from_rgb(40, 62, 92);
pub const ROLLBACK: Color32 = Color32::from_rgb(150, 110, 40);
pub const ROLLBACK_HOT: Color32 = Color32::from_rgb(230, 170, 60);
/// While a row is dragged: the rows it reads, and the rows that read it.
pub const READS: Color32 = Color32::from_rgb(90, 170, 230);
pub const READ_BY: Color32 = Color32::from_rgb(230, 140, 80);
pub const KIND: Color32 = Color32::from_rgb(120, 160, 200);
pub const PROBLEM: Color32 = Color32::from_rgb(235, 110, 90);
pub const UNREAD: Color32 = Color32::from_rgb(220, 180, 80);
pub const VIEW_BACKGROUND: Color32 = Color32::from_rgb(6, 7, 12);
pub const GRID: Color32 = Color32::from_rgba_premultiplied(40, 46, 60, 255);
pub const CELL: Color32 = Color32::from_rgba_premultiplied(70, 60, 30, 255);
pub const SUPPORT: Color32 = Color32::from_rgba_premultiplied(60, 110, 120, 255);
pub const PICK: Color32 = Color32::from_rgb(255, 230, 120);
pub const AXIS_X: Color32 = Color32::from_rgb(190, 70, 70);
pub const AXIS_Y: Color32 = Color32::from_rgb(70, 170, 80);
pub const AXIS_Z: Color32 = Color32::from_rgb(80, 110, 220);
pub const HISTOGRAM: Color32 = Color32::from_rgb(110, 150, 200);
/// A star against the pinned bake.
pub const DIFF_NEW: Color32 = Color32::from_rgb(110, 210, 120);
pub const DIFF_CHANGED: Color32 = Color32::from_rgb(230, 180, 70);
pub const DIFF_SAME: Color32 = Color32::from_gray(85);
pub const DIFF_GONE: Color32 = Color32::from_rgb(235, 90, 80);

/// Distinct hues for populations, in timeline order.
pub const POPULATIONS: [Color32; 6] = [
    Color32::from_rgb(120, 180, 255),
    Color32::from_rgb(255, 170, 90),
    Color32::from_rgb(140, 220, 140),
    Color32::from_rgb(230, 120, 200),
    Color32::from_rgb(240, 220, 110),
    Color32::from_rgb(150, 140, 240),
];

/// Temperatures [`camera`] tabulates, as log2 kelvin; outside them it clamps.
const CAMERA_LOG_T: (f64, f64) = (9.0, 17.0);
const CAMERA_SAMPLES: usize = 512;

/// The hue the client's starfield gives a blackbody at `kelvin` through the default mapping:
/// band radiance through [`presets::natural`], divided by its brightest channel, as
/// `starfield.wgsl` does. At rest, so no Doppler shift, and without the warm re-emission of a
/// population or the display tone map.
pub fn camera(kelvin: f64) -> Color32 {
    // Integrating Planck's law per band is too slow to repeat for every star every frame.
    static TABLE: std::sync::OnceLock<Vec<Color32>> = std::sync::OnceLock::new();
    let (lo, hi) = CAMERA_LOG_T;
    let table = TABLE.get_or_init(|| {
        let mapping = presets::natural();
        (0..CAMERA_SAMPLES)
            .map(|i| {
                let t = (lo + (hi - lo) * i as f64 / (CAMERA_SAMPLES - 1) as f64).exp2();
                let linear = mapping.apply_f64(&blackbody::per_band(t));
                let peak = linear.iter().copied().fold(0.0, f64::max);
                let c = |v: f64| (cie::encode_srgb((v / peak).clamp(0.0, 1.0)) * 255.0).round() as u8;
                if peak > 0.0 { Color32::from_rgb(c(linear[0]), c(linear[1]), c(linear[2])) } else { Color32::BLACK }
            })
            .collect()
    });
    let at = ((kelvin.max(1.0).log2() - lo) / (hi - lo) * (CAMERA_SAMPLES - 1) as f64).round();
    table[at.clamp(0.0, (CAMERA_SAMPLES - 1) as f64) as usize]
}

/// Approximate color of a blackbody at `kelvin`, after Tanner Helland's fit.
pub fn blackbody(kelvin: f64) -> Color32 {
    let t = (kelvin / 100.0).clamp(10.0, 400.0);
    let r = if t <= 66.0 { 255.0 } else { 329.7 * (t - 60.0).powf(-0.1332) };
    let g = if t <= 66.0 { 99.47 * t.ln() - 161.12 } else { 288.12 * (t - 60.0).powf(-0.0755) };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.52 * (t - 10.0).ln() - 305.04
    };
    let c = |v: f64| v.clamp(0.0, 255.0) as u8;
    Color32::from_rgb(c(r), c(g), c(b))
}

/// Blue through white to red over `t ∈ [0, 1]`.
pub fn diverging(t: f64) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let (lo, mid, hi) = ([70.0, 120.0, 230.0], [235.0, 235.0, 235.0], [230.0, 80.0, 60.0]);
    let (a, b, u) = if t < 0.5 { (lo, mid, t * 2.0) } else { (mid, hi, t * 2.0 - 1.0) };
    let m = |i: usize| (a[i] + (b[i] - a[i]) * u) as u8;
    Color32::from_rgb(m(0), m(1), m(2))
}

/// Dark violet through yellow over `t ∈ [0, 1]`.
pub fn sequential(t: f64) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let (a, b) = ([60.0, 40.0, 120.0], [250.0, 230.0, 90.0]);
    let m = |i: usize| (a[i] + (b[i] - a[i]) * t) as u8;
    Color32::from_rgb(m(0), m(1), m(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_reddens_cool_stars_and_blues_hot_ones() {
        let [r, _, b, _] = camera(3000.0).to_array();
        assert!(r > b, "3000 K: {:?}", camera(3000.0));
        let [r, _, b, _] = camera(30000.0).to_array();
        assert!(b > r, "30000 K: {:?}", camera(30000.0));
        for k in [3000.0, 5772.0, 30000.0] {
            let [r, g, b, _] = camera(k).to_array();
            assert_eq!(r.max(g).max(b), 255, "{k} K is not at full value");
        }
    }
}
