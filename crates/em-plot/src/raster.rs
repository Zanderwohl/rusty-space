//! A PNG backend, for looking at a chart.

use tiny_skia::{Color, LineCap, Paint, PathBuilder, Pixmap, PremultipliedColorU8, Stroke, Transform};

use crate::font::{GLYPH_H, GLYPH_W, glyph};
use crate::primitives::{Anchor, Primitives, Rgba, TextMetrics};

/// Text measurement that matches what [`render`] actually draws.
///
/// The generic `Monospace` estimate does not: this font steps by whole pixels, so at size 13
/// it advances 12 px per character where a 0.6 ratio predicts 7.8. Laying out against the
/// estimate and drawing with the font puts the leading digits of an axis label off the edge
/// of the image. A backend that draws text should supply the metrics for it.
#[derive(Clone, Copy, Debug, Default)]
pub struct BitmapMetrics;

/// Integer scale factor this font uses at a requested size.
pub fn glyph_scale(size: f32) -> i32 {
    ((size / GLYPH_H as f32).round() as i32).max(1)
}

impl TextMetrics for BitmapMetrics {
    fn measure(&self, text: &str, size: f32) -> (f32, f32) {
        let scale = glyph_scale(size) as f32;
        (text.chars().count() as f32 * (GLYPH_W as f32 + 1.0) * scale, GLYPH_H as f32 * scale)
    }
}

/// Blend an axis-aligned rectangle straight into the pixels, with edge coverage.
///
/// Not `fill_rect`. Sending small rectangles through the path rasteriser is slower — a
/// scatter is a hundred thousand of them — and tiny-skia's anti-aliased hairline scan
/// converter asserts on sub-two-pixel geometry, which a 1.6 px marker is.
fn fill_quad(pm: &mut Pixmap, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let (lo_x, hi_x) = (x0.min(x1), x0.max(x1));
    let (lo_y, hi_y) = (y0.min(y1), y0.max(y1));
    if c.3 <= 0.0 || hi_x <= 0.0 || hi_y <= 0.0 || lo_x >= w as f32 || lo_y >= h as f32 {
        return;
    }
    let (px0, py0) = (lo_x.floor().max(0.0) as i32, lo_y.floor().max(0.0) as i32);
    let (px1, py1) = (hi_x.ceil().min(w as f32) as i32, hi_y.ceil().min(h as f32) as i32);
    let data = pm.pixels_mut();
    for py in py0..py1 {
        let cover_y = (hi_y.min(py as f32 + 1.0) - lo_y.max(py as f32)).clamp(0.0, 1.0);
        if cover_y <= 0.0 {
            continue;
        }
        for px in px0..px1 {
            let cover_x = (hi_x.min(px as f32 + 1.0) - lo_x.max(px as f32)).clamp(0.0, 1.0);
            let a = c.3 * cover_x * cover_y;
            if a <= 0.0 {
                continue;
            }
            let i = (py * w + px) as usize;
            let dst = data[i];
            let inv = 255.0 * (1.0 - a);
            let blend = |src: f32, d: u8| (src * a * 255.0 + d as f32 / 255.0 * inv).round().clamp(0.0, 255.0) as u8;
            let (r, g, b) = (blend(c.0, dst.red()), blend(c.1, dst.green()), blend(c.2, dst.blue()));
            let alpha = (dst.alpha() as f32 + a * (255.0 - dst.alpha() as f32)).round() as u8;
            if let Some(p) = PremultipliedColorU8::from_rgba(r.min(alpha), g.min(alpha), b.min(alpha), alpha) {
                data[i] = p;
            }
        }
    }
}

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba(c.0.clamp(0.0, 1.0), c.1.clamp(0.0, 1.0), c.2.clamp(0.0, 1.0), c.3.clamp(0.0, 1.0)).unwrap_or(Color::BLACK));
    p.anti_alias = true;
    p
}

/// Draw a string with the built-in bitmap font. Returns the width used.
fn text(pixmap: &mut Pixmap, at: (f32, f32), s: &str, size: f32, anchor: Anchor, color: Rgba) {
    let scale = ((size / GLYPH_H as f32).round() as i32).max(1);
    let advance = (GLYPH_W as i32 + 1) * scale;
    let width = s.chars().count() as i32 * advance;
    let start_x = match anchor {
        Anchor::Start => at.0 as i32,
        Anchor::Middle => at.0 as i32 - width / 2,
        Anchor::End => at.0 as i32 - width,
    };
    // `at.1` is the text baseline, as SVG has it, so lift the cell above it.
    let top = at.1 as i32 - GLYPH_H as i32 * scale;

    for (i, c) in s.chars().enumerate() {
        let Some(rows) = glyph(c) else { continue };
        let gx = start_x + i as i32 * advance;
        for (r, bits) in rows.iter().enumerate() {
            for col in 0..GLYPH_W {
                if bits & (1 << (GLYPH_W - 1 - col)) == 0 {
                    continue;
                }
                let x = (gx + col as i32 * scale) as f32;
                let y = (top + r as i32 * scale) as f32;
                fill_quad(pixmap, x, y, x + scale as f32, y + scale as f32, color);
            }
        }
    }
}

/// Rasterise primitives onto a new pixmap.
pub fn render(
    layers: &[&Primitives],
    width: u32,
    height: u32,
    background: Rgba,
) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width.max(1), height.max(1))?;
    pixmap.fill(Color::from_rgba(background.0, background.1, background.2, background.3)?);

    for layer in layers {
        for q in &layer.quads {
            fill_quad(&mut pixmap, q.min.x, q.min.y, q.max.x, q.max.y, q.color);
        }
        for line in &layer.polylines {
            let mut pb = PathBuilder::new();
            let mut points = line.points.iter();
            let Some(first) = points.next() else { continue };
            pb.move_to(first.x, first.y);
            let mut any = false;
            for p in points {
                pb.line_to(p.x, p.y);
                any = true;
            }
            // A zero-length polyline has no stroke; nudge it so single columns still show.
            if !any {
                pb.line_to(first.x, first.y + 0.01);
            }
            if let Some(path) = pb.finish() {
                let stroke = Stroke { width: line.width.max(0.5), line_cap: LineCap::Butt, ..Stroke::default() };
                pixmap.stroke_path(&path, &paint(line.color), &stroke, Transform::identity(), None);
            }
        }
        for l in &layer.labels {
            text(&mut pixmap, (l.at.x, l.at.y), &l.text, l.size, l.anchor, l.color);
        }
    }
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Axis, Chart, Rect};
    use crate::primitives::Monospace;

    #[test]
    fn a_chart_rasterises_with_ink_on_it() {
        let m = Monospace::default();
        let c = Chart::new(
            Rect { x: 60.0, y: 20.0, width: 400.0, height: 200.0 },
            Axis::linear((0.0, 1.0)),
            Axis::linear((0.0, 1.0)),
            &m,
        );
        let points: Vec<(f64, f64)> =
            (0..10_000).map(|i| (i as f64 / 1e4, (i as f64 / 300.0).sin() * 0.5 + 0.5)).collect();
        let pm = render(&[&c.frame(), &c.series(&points)], 520, 280, Rgba::opaque(1.0, 1.0, 1.0))
            .expect("pixmap");
        let ink = pm.pixels().iter().filter(|p| p.red() < 250 || p.blue() < 250).count();
        assert!(ink > 2000, "only {ink} pixels were drawn");
        assert!(!pm.encode_png().unwrap().is_empty());
    }

    #[test]
    fn sub_pixel_markers_render_without_panicking() {
        // A 1.6 px marker used to crash tiny-skia's hairline scan converter.
        let mut pm = Pixmap::new(40, 40).unwrap();
        pm.fill(Color::WHITE);
        for size in [0.4f32, 1.0, 1.6, 2.0, 7.5] {
            let h = size / 2.0;
            fill_quad(&mut pm, 20.0 - h, 20.0 - h, 20.0 + h, 20.0 + h, Rgba::opaque(0.0, 0.0, 0.0));
        }
        assert!(pm.pixels().iter().any(|p| p.red() < 200));
    }

    #[test]
    fn a_quad_outside_the_pixmap_is_dropped_not_wrapped() {
        let mut pm = Pixmap::new(20, 20).unwrap();
        pm.fill(Color::WHITE);
        for (x, y) in [(-90.0f32, 5.0f32), (500.0, 5.0), (5.0, -90.0), (5.0, 500.0)] {
            fill_quad(&mut pm, x, y, x + 4.0, y + 4.0, Rgba::opaque(0.0, 0.0, 0.0));
        }
        assert!(pm.pixels().iter().all(|p| p.red() > 250), "nothing should have been drawn");
    }

    #[test]
    fn alpha_accumulates_toward_the_color() {
        let mut pm = Pixmap::new(8, 8).unwrap();
        pm.fill(Color::WHITE);
        let faint = Rgba(0.0, 0.0, 0.0, 0.25);
        let before = pm.pixels()[3 * 8 + 3].red();
        fill_quad(&mut pm, 2.0, 2.0, 6.0, 6.0, faint);
        let once = pm.pixels()[3 * 8 + 3].red();
        fill_quad(&mut pm, 2.0, 2.0, 6.0, 6.0, faint);
        let twice = pm.pixels()[3 * 8 + 3].red();
        assert!(once < before && twice < once, "{before} -> {once} -> {twice}");
    }

    #[test]
    fn the_metrics_match_what_the_font_draws() {
        let m = BitmapMetrics;
        for size in [8.0f32, 13.0, 20.0, 32.0] {
            let scale = glyph_scale(size);
            let (w, h) = m.measure("12345", size);
            assert_eq!(w, 5.0 * (GLYPH_W as f32 + 1.0) * scale as f32);
            assert_eq!(h, GLYPH_H as f32 * scale as f32);
        }
        // The generic estimate is the one that was wrong.
        let generic = crate::primitives::Monospace::default().measure("0.9999982", 13.0).0;
        let actual = m.measure("0.9999982", 13.0).0;
        assert!(actual > generic, "{actual} vs the estimate {generic}");
    }

    #[test]
    fn a_measured_label_fits_the_space_it_asked_for() {
        let m = BitmapMetrics;
        let text = "0.9999982";
        let (w, _) = m.measure(text, 13.0);
        let mut pm = Pixmap::new((w.ceil() as u32) + 2, 24).unwrap();
        pm.fill(Color::WHITE);
        // Anchored at the right edge of exactly the measured width: nothing should be cut.
        super::text(&mut pm, (w + 1.0, 20.0), text, 13.0, Anchor::End, Rgba::BLACK);
        let leftmost = pm
            .pixels()
            .iter()
            .enumerate()
            .filter(|(_, p)| p.red() < 128)
            .map(|(i, _)| i as u32 % pm.width())
            .min();
        assert!(leftmost.is_some_and(|x| x >= 1), "the label overflowed its measured width");
    }

    #[test]
    fn text_lands_inside_the_pixmap() {
        let mut pm = Pixmap::new(200, 60).unwrap();
        pm.fill(Color::WHITE);
        text(&mut pm, (100.0, 40.0), "1.25e-6", 14.0, Anchor::Middle, Rgba::BLACK);
        let ink = pm.pixels().iter().filter(|p| p.red() < 128).count();
        assert!(ink > 40, "text drew only {ink} pixels");
    }
}
