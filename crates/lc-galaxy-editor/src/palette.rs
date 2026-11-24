//! Colors the panels and views share.

use bevy_egui::egui::Color32;

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

/// Distinct hues for populations, in timeline order.
pub const POPULATIONS: [Color32; 6] = [
    Color32::from_rgb(120, 180, 255),
    Color32::from_rgb(255, 170, 90),
    Color32::from_rgb(140, 220, 140),
    Color32::from_rgb(230, 120, 200),
    Color32::from_rgb(240, 220, 110),
    Color32::from_rgb(150, 140, 240),
];

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
