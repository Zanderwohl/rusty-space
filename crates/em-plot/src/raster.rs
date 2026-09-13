//! A PNG backend, for looking at a chart.

use tiny_skia::{
    Color, FillRule, LineCap, Paint, PathBuilder, Pixmap, Rect as SkRect, Stroke, Transform,
};

use crate::font::{GLYPH_H, GLYPH_W, glyph};
use crate::primitives::{Anchor, Primitives, Rgba};

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba(c.0.clamp(0.0, 1.0), c.1.clamp(0.0, 1.0), c.2.clamp(0.0, 1.0), c.3.clamp(0.0, 1.0)).unwrap_or(Color::BLACK));
    p.anti_alias = true;
    p
}

/// Draw a string with the built-in bitmap font. Returns the width used.
fn text(pixmap: &mut Pixmap, at: (f32, f32), s: &str, size: f32, anchor: Anchor, colour: Rgba) {
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

    let p = paint(colour);
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
                if let Some(rect) = SkRect::from_xywh(x, y, scale as f32, scale as f32) {
                    pixmap.fill_rect(rect, &p, Transform::identity(), None);
                }
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
            if let Some(rect) =
                SkRect::from_ltrb(q.min.x, q.min.y, q.max.x.max(q.min.x + 0.01), q.max.y.max(q.min.y + 0.01))
            {
                pixmap.fill_rect(rect, &paint(q.colour), Transform::identity(), None);
            }
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
                pixmap.stroke_path(&path, &paint(line.colour), &stroke, Transform::identity(), None);
            }
        }
        for l in &layer.labels {
            text(&mut pixmap, (l.at.x, l.at.y), &l.text, l.size, l.anchor, l.colour);
        }
    }
    let _ = FillRule::Winding;
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
    fn text_lands_inside_the_pixmap() {
        let mut pm = Pixmap::new(200, 60).unwrap();
        pm.fill(Color::WHITE);
        text(&mut pm, (100.0, 40.0), "1.25e-6", 14.0, Anchor::Middle, Rgba::BLACK);
        let ink = pm.pixels().iter().filter(|p| p.red() < 128).count();
        assert!(ink > 40, "text drew only {ink} pixels");
    }
}
