//! An SVG backend, so the core is testable without a window.

use std::fmt::Write;

use crate::primitives::{Anchor, Primitives, Rgba};

fn color(c: Rgba) -> String {
    format!(
        "rgb({},{},{})",
        (c.0.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.1.clamp(0.0, 1.0) * 255.0).round() as u8,
        (c.2.clamp(0.0, 1.0) * 255.0).round() as u8
    )
}

/// Render primitives to an SVG document.
pub fn render(primitives: &[&Primitives], width: f32, height: f32) -> String {
    let mut s = String::with_capacity(4096);
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\">"
    );
    for p in primitives {
        for q in &p.quads {
            let _ = write!(
                s,
                "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{}\" \
                 fill-opacity=\"{:.3}\"/>",
                q.min.x,
                q.min.y,
                q.max.x - q.min.x,
                q.max.y - q.min.y,
                color(q.color),
                q.color.3
            );
        }
        for line in &p.polylines {
            let pts: Vec<String> =
                line.points.iter().map(|pt| format!("{:.2},{:.2}", pt.x, pt.y)).collect();
            let _ = write!(
                s,
                "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{:.2}\" \
                 stroke-opacity=\"{:.3}\"/>",
                pts.join(" "),
                color(line.color),
                line.width,
                line.color.3
            );
        }
        for l in &p.labels {
            let anchor = match l.anchor {
                Anchor::Start => "start",
                Anchor::Middle => "middle",
                Anchor::End => "end",
            };
            let _ = write!(
                s,
                "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"{:.2}\" text-anchor=\"{anchor}\" \
                 fill=\"{}\">{}</text>",
                l.at.x,
                l.at.y,
                l.size,
                color(l.color),
                escape(&l.text)
            );
        }
    }
    s.push_str("</svg>");
    s
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::{Axis, Chart, Rect};
    use crate::primitives::Monospace;

    /// The phase's exit criterion, end to end: two million samples into eight hundred pixels,
    /// with the narrow feature still in the output.
    #[test]
    fn two_million_samples_render_with_the_transit_intact() {
        let n = 2_000_000usize;
        let dip = 1_234_567usize;
        let points: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let x = i as f64 / n as f64;
                let y = if i == dip { 0.9990 } else { 1.0 + ((i * 2654435761) % 97) as f64 * 1e-6 };
                (x, y)
            })
            .collect();

        let m = Monospace::default();
        let area = Rect { x: 50.0, y: 10.0, width: 800.0, height: 220.0 };
        let chart = Chart::new(area, Axis::linear((0.0, 1.0)), Axis::linear((0.9985, 1.0002)), &m);

        let frame = chart.frame();
        let series = chart.series(&points);
        let svg = render(&[&frame, &series], 900.0, 280.0);

        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert_eq!(series.polylines.len(), 800, "one column per pixel");

        // The deepest sample must reach its own pixel row, not be averaged away.
        let dip_column = (dip as f64 / n as f64 * 800.0) as u32;
        let column = series
            .polylines
            .get(dip_column as usize)
            .expect("the transit's column must be drawn");
        let lowest = column.points.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let others: f32 = series
            .polylines
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != dip_column as usize)
            .map(|(_, l)| l.points.iter().map(|p| p.y).fold(f32::MIN, f32::max))
            .fold(f32::MIN, f32::max);
        assert!(lowest > others + 50.0, "the transit should stand out: {lowest} vs {others}");
    }

    #[test]
    fn the_document_is_well_formed_and_escapes_text() {
        use crate::primitives::{Label, Point, Primitives};
        let mut p = Primitives::default();
        p.labels.push(Label {
            at: Point::new(1.0, 2.0),
            text: "a < b & c".into(),
            size: 10.0,
            anchor: Anchor::Start,
            color: Rgba::BLACK,
        });
        let svg = render(&[&p], 10.0, 10.0);
        assert!(svg.contains("a &lt; b &amp; c"));
        assert_eq!(svg.matches("<text").count(), 1);
        assert!(!svg.contains("NaN"));
    }

    #[test]
    fn an_empty_chart_still_produces_a_valid_document() {
        let svg = render(&[&Primitives::default()], 100.0, 50.0);
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
    }
}
