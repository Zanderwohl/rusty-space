//! Turning a series and two axes into primitives.

use crate::decimate::{self, Envelope};
use crate::primitives::{Anchor, Label, Point, Polyline, Primitives, Rgba, TextMetrics};
use crate::scale::Scale;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    pub scale: Scale,
    pub range: (f64, f64),
    pub ticks: usize,
}

impl Axis {
    pub fn linear(range: (f64, f64)) -> Self {
        Self { scale: Scale::Linear, range, ticks: 5 }
    }
}

/// Pixel rectangle a chart draws into. `y` grows downward, as screens do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Style {
    pub series: Rgba,
    pub axis: Rgba,
    pub text_size: f32,
    pub line_width: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            series: Rgba::opaque(0.20, 0.55, 0.85),
            axis: Rgba::opaque(0.45, 0.45, 0.50),
            text_size: 10.0,
            line_width: 1.0,
        }
    }
}

pub struct Chart<'a> {
    pub area: Rect,
    pub x: Axis,
    pub y: Axis,
    pub style: Style,
    pub metrics: &'a dyn TextMetrics,
}

impl<'a> Chart<'a> {
    pub fn new(area: Rect, x: Axis, y: Axis, metrics: &'a dyn TextMetrics) -> Self {
        Self { area, x, y, style: Style::default(), metrics }
    }

    fn px(&self, vx: f64, vy: f64) -> Point {
        Point::new(
            self.area.x + (self.x.scale.normalise(vx, self.x.range) as f32) * self.area.width,
            self.area.y + self.area.height
                - (self.y.scale.normalise(vy, self.y.range) as f32) * self.area.height,
        )
    }

    /// Data coordinates of a pixel, for reading a value off the chart.
    pub fn value_at(&self, p: Point) -> (f64, f64) {
        let tx = ((p.x - self.area.x) / self.area.width) as f64;
        let ty = ((self.area.y + self.area.height - p.y) / self.area.height) as f64;
        (self.x.scale.denormalise(tx, self.x.range), self.y.scale.denormalise(ty, self.y.range))
    }

    /// Decimate a series to this chart's pixel width.
    pub fn envelope(&self, points: &[(f64, f64)]) -> Envelope {
        decimate::min_max(points, self.x.range, self.area.width.max(1.0) as u32)
    }

    /// Axes, ticks and labels.
    pub fn frame(&self) -> Primitives {
        let mut out = Primitives::default();
        let (left, bottom) = (self.area.x, self.area.y + self.area.height);
        let right = self.area.x + self.area.width;

        out.polylines.push(Polyline {
            points: vec![Point::new(left, self.area.y), Point::new(left, bottom), Point::new(right, bottom)],
            colour: self.style.axis,
            width: self.style.line_width,
        });

        for v in self.x.scale.ticks(self.x.range, self.x.ticks) {
            let at = self.px(v, self.y.range.0);
            out.polylines.push(Polyline {
                points: vec![Point::new(at.x, bottom), Point::new(at.x, bottom + 4.0)],
                colour: self.style.axis,
                width: self.style.line_width,
            });
            out.labels.push(Label {
                at: Point::new(at.x, bottom + 6.0 + self.style.text_size),
                text: format_tick(v),
                size: self.style.text_size,
                anchor: Anchor::Middle,
                colour: self.style.axis,
            });
        }
        for v in self.y.scale.ticks(self.y.range, self.y.ticks) {
            let at = self.px(self.x.range.0, v);
            out.polylines.push(Polyline {
                points: vec![Point::new(left - 4.0, at.y), Point::new(left, at.y)],
                colour: self.style.axis,
                width: self.style.line_width,
            });
            out.labels.push(Label {
                at: Point::new(left - 6.0, at.y + self.style.text_size * 0.35),
                text: format_tick(v),
                size: self.style.text_size,
                anchor: Anchor::End,
                colour: self.style.axis,
            });
        }
        out
    }

    /// Width the y-axis labels need, so a caller can size the margin before drawing.
    pub fn y_label_width(&self) -> f32 {
        self.y
            .scale
            .ticks(self.y.range, self.y.ticks)
            .iter()
            .map(|v| self.metrics.measure(&format_tick(*v), self.style.text_size).0)
            .fold(0.0, f32::max)
    }

    /// A series as its envelope: one vertical segment per pixel column.
    ///
    /// Not a polyline through the samples. At these densities the line would be a solid block
    /// that hides its own structure, and drawing every sample would be millions of vertices
    /// for a few hundred pixels of result.
    pub fn series(&self, points: &[(f64, f64)]) -> Primitives {
        let mut out = Primitives::default();
        let env = self.envelope(points);
        let step = self.area.width / self.area.width.max(1.0);
        for c in &env.columns {
            let x = self.area.x + c.index as f32 * step + 0.5;
            let (lo, hi) = (self.px(self.x.range.0, c.min), self.px(self.x.range.0, c.max));
            out.polylines.push(Polyline {
                points: vec![Point::new(x, lo.y), Point::new(x, hi.y)],
                colour: self.style.series,
                width: self.style.line_width,
            });
        }
        out
    }
}

fn format_tick(v: f64) -> String {
    let a = v.abs();
    if v == 0.0 {
        "0".to_string()
    } else if a >= 1e5 || a < 1e-3 {
        format!("{v:.1e}")
    } else if a >= 100.0 {
        format!("{v:.0}")
    } else if a >= 1.0 {
        format!("{v:.2}")
    } else {
        format!("{v:.4}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::Monospace;

    fn chart(m: &Monospace) -> Chart<'_> {
        Chart::new(
            Rect { x: 40.0, y: 10.0, width: 800.0, height: 200.0 },
            Axis::linear((0.0, 1.0)),
            Axis::linear((-1.0, 2.0)),
            m,
        )
    }

    #[test]
    fn pixels_and_values_round_trip() {
        let m = Monospace::default();
        let c = chart(&m);
        for (vx, vy) in [(0.0, -1.0), (0.5, 0.5), (1.0, 2.0), (0.25, 1.7)] {
            let (bx, by) = c.value_at(c.px(vx, vy));
            // Pixels are f32, so a round trip through them keeps about seven digits.
            let tol = |v: f64| v.abs().max(1.0) * 1e-6;
            assert!((bx - vx).abs() < tol(vx) && (by - vy).abs() < tol(vy), "({vx},{vy}) -> ({bx},{by})");
        }
    }

    #[test]
    fn the_y_axis_points_up_on_a_screen_that_points_down() {
        let m = Monospace::default();
        let c = chart(&m);
        assert!(c.px(0.0, 2.0).y < c.px(0.0, -1.0).y, "larger values must sit higher");
        assert!(c.px(1.0, 0.0).x > c.px(0.0, 0.0).x);
    }

    #[test]
    fn the_frame_carries_a_label_for_every_tick() {
        let m = Monospace::default();
        let c = chart(&m);
        let f = c.frame();
        let ticks = c.x.scale.ticks(c.x.range, c.x.ticks).len() + c.y.scale.ticks(c.y.range, c.y.ticks).len();
        assert_eq!(f.labels.len(), ticks);
        assert_eq!(f.polylines.len(), ticks + 1, "one polyline per tick, plus the axes");
        assert!(c.y_label_width() > 0.0);
    }

    #[test]
    fn a_series_draws_one_column_per_populated_pixel() {
        let m = Monospace::default();
        let c = chart(&m);
        let points: Vec<(f64, f64)> =
            (0..100_000).map(|i| (i as f64 / 100_000.0, (i as f64 / 500.0).sin())).collect();
        let s = c.series(&points);
        assert_eq!(s.polylines.len(), 800);
        for line in &s.polylines {
            assert_eq!(line.points.len(), 2, "a column is a vertical segment");
            assert!(line.points[0].x >= c.area.x && line.points[0].x <= c.area.x + c.area.width);
        }
    }

    #[test]
    fn ticks_format_across_the_ranges_a_light_curve_spans() {
        assert_eq!(format_tick(0.0), "0");
        assert_eq!(format_tick(1.0), "1.00");
        assert_eq!(format_tick(1234.0), "1234");
        assert!(format_tick(5.3e-6).contains('e'));
        assert!(format_tick(1.2e8).contains('e'));
    }
}
