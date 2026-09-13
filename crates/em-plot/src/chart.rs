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

        let xt = self.x.scale.ticks(self.x.range, self.x.ticks);
        let xf = TickFormat::for_axis(self.x.scale, &xt);
        for v in xt {
            let at = self.px(v, self.y.range.0);
            out.polylines.push(Polyline {
                points: vec![Point::new(at.x, bottom), Point::new(at.x, bottom + 4.0)],
                colour: self.style.axis,
                width: self.style.line_width,
            });
            out.labels.push(Label {
                at: Point::new(at.x, bottom + 6.0 + self.style.text_size),
                text: xf.apply(v),
                size: self.style.text_size,
                anchor: Anchor::Middle,
                colour: self.style.axis,
            });
        }
        let yt = self.y.scale.ticks(self.y.range, self.y.ticks);
        let yf = TickFormat::for_axis(self.y.scale, &yt);
        for v in yt {
            let at = self.px(self.x.range.0, v);
            out.polylines.push(Polyline {
                points: vec![Point::new(left - 4.0, at.y), Point::new(left, at.y)],
                colour: self.style.axis,
                width: self.style.line_width,
            });
            out.labels.push(Label {
                at: Point::new(left - 6.0, at.y + self.style.text_size * 0.35),
                text: yf.apply(v),
                size: self.style.text_size,
                anchor: Anchor::End,
                colour: self.style.axis,
            });
        }
        out
    }

    /// Width the y-axis labels need, so a caller can size the margin before drawing.
    pub fn y_label_width(&self) -> f32 {
        let ticks = self.y.scale.ticks(self.y.range, self.y.ticks);
        let f = TickFormat::for_axis(self.y.scale, &ticks);
        ticks
            .iter()
            .map(|v| self.metrics.measure(&f.apply(*v), self.style.text_size).0)
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

    /// A line through the per-column mean.
    ///
    /// A dense envelope is a solid block, which is honest about the extremes and silent about
    /// everything between them. The mean track is what puts the shape back.
    pub fn mean_track(&self, points: &[(f64, f64)], colour: Rgba) -> Primitives {
        let means = decimate::means(points, self.x.range, self.area.width.max(1.0) as u32);
        if means.len() < 2 {
            return Primitives::default();
        }
        let pts = means
            .iter()
            .map(|(i, v)| {
                Point::new(self.area.x + *i as f32 + 0.5, self.px(self.x.range.0, *v).y)
            })
            .collect();
        Primitives {
            polylines: vec![Polyline { points: pts, colour, width: self.style.line_width }],
            ..Primitives::default()
        }
    }
}

/// How a whole axis's ticks are written.
///
/// Precision comes from the tick *spacing*, not from each value's own magnitude. Deciding
/// per value gives an axis reading "1.00, 1.0000, 0.9999", where the same quantity is
/// written three ways and the reader cannot tell whether the first two differ.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TickFormat {
    decimals: usize,
    scientific: bool,
}

impl TickFormat {
    /// Log ticks are powers of ten and read as such; `0.0001, 0.0010, 0.0100` is the same
    /// information spelled at four times the width.
    pub fn for_axis(scale: Scale, ticks: &[f64]) -> Self {
        match scale {
            Scale::Log10 => Self { decimals: 0, scientific: true },
            _ => Self::for_ticks(ticks),
        }
    }

    pub fn for_ticks(ticks: &[f64]) -> Self {
        let step = ticks
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .filter(|d| *d > 0.0)
            .fold(f64::INFINITY, f64::min);
        let largest = ticks.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        if !step.is_finite() || step <= 0.0 {
            return Self { decimals: 2, scientific: false };
        }
        if largest >= 1e5 || (largest > 0.0 && largest < 1e-3) {
            return Self { decimals: 2, scientific: true };
        }
        Self { decimals: (-step.log10()).ceil().clamp(0.0, 9.0) as usize, scientific: false }
    }

    pub fn apply(&self, v: f64) -> String {
        // Rounding a tick at the edge of a padded range can land on -0.0, which prints as
        // "-0.00e0". Adding zero normalises it and is the identity for everything else.
        let v = v + 0.0;
        if self.scientific {
            format!("{:.*e}", self.decimals, v)
        } else {
            format!("{:.*}", self.decimals, v)
        }
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
    fn an_axis_writes_all_its_ticks_the_same_way() {
        // The failure this replaces: an axis reading "1.00, 1.0000, 0.9999".
        let near_one = [0.99990, 0.99995, 1.00000, 1.00005];
        let f = TickFormat::for_ticks(&near_one);
        let rendered: Vec<String> = near_one.iter().map(|v| f.apply(*v)).collect();
        assert!(
            rendered.iter().all(|s| s.len() == rendered[0].len()),
            "inconsistent widths: {rendered:?}"
        );
        assert!(rendered.iter().collect::<std::collections::HashSet<_>>().len() == 4,
            "ticks must stay distinguishable: {rendered:?}");

        assert_eq!(TickFormat::for_ticks(&[0.0, 50.0, 100.0]).apply(50.0), "50");
        // Negative zero is still zero.
        assert_eq!(TickFormat::for_ticks(&[-1.0, 0.0, 1.0]).apply(-0.0), "0");
        assert!(!TickFormat::for_axis(Scale::Log10, &[1.0]).apply(-0.0).starts_with('-'));
        assert!(TickFormat::for_ticks(&[1e-9, 2e-9]).apply(1e-9).contains('e'));
        // A log axis writes powers of ten, whatever their magnitude.
        let log = TickFormat::for_axis(Scale::Log10, &[1e-4, 1e-3, 1e-2]);
        assert_eq!(log.apply(1e-4), "1e-4");
        assert_eq!(log.apply(100.0), "1e2");
        assert!(TickFormat::for_ticks(&[1e7, 2e7]).apply(1e7).contains('e'));
    }

    #[test]
    fn a_mean_track_follows_the_middle_of_the_envelope() {
        let m = Monospace::default();
        let c = chart(&m);
        let points: Vec<(f64, f64)> = (0..100_000)
            .map(|i| (i as f64 / 1e5, if i % 2 == 0 { 0.0 } else { 1.0 }))
            .collect();
        let track = c.mean_track(&points, Rgba::BLACK);
        assert_eq!(track.polylines.len(), 1);
        let line = &track.polylines[0];
        assert_eq!(line.points.len(), 800);
        let want = c.px(0.0, 0.5).y;
        assert!(line.points.iter().all(|p| (p.y - want).abs() < 0.5), "the track should sit at 0.5");
    }
}
