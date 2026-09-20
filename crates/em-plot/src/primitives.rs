//! What a chart emits: geometry and text placements, in pixels.
//!
//! Nothing here rasterises. Text is where a plotting library acquires a dependency tree —
//! font loading, shaping, atlasing — and delegating it costs one trait and removes all of
//! that. A backend turns these into meshes, painter calls, or an SVG string.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Straight-alpha RGBA, 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

impl Rgba {
    pub const BLACK: Self = Self(0.0, 0.0, 0.0, 1.0);
    pub const fn opaque(r: f32, g: f32, b: f32) -> Self {
        Self(r, g, b, 1.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pub points: Vec<Point>,
    pub color: Rgba,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub min: Point,
    pub max: Point,
    pub color: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub at: Point,
    pub text: String,
    pub size: f32,
    pub anchor: Anchor,
    pub color: Rgba,
}

/// Everything a chart produced, ready for a backend.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Primitives {
    pub polylines: Vec<Polyline>,
    pub quads: Vec<Quad>,
    pub labels: Vec<Label>,
}

impl Primitives {
    pub fn is_empty(&self) -> bool {
        self.polylines.is_empty() && self.quads.is_empty() && self.labels.is_empty()
    }
}

/// How wide a string will be. The only thing a chart needs to know about text in order to
/// place it, size margins, and avoid collisions.
pub trait TextMetrics {
    fn measure(&self, text: &str, size: f32) -> (f32, f32);
}

/// Every glyph the same box. Enough for layout, and it keeps the core testable with no font.
#[derive(Clone, Copy, Debug)]
pub struct Monospace {
    pub advance_ratio: f32,
}

impl Default for Monospace {
    fn default() -> Self {
        Self { advance_ratio: 0.6 }
    }
}

impl TextMetrics for Monospace {
    fn measure(&self, text: &str, size: f32) -> (f32, f32) {
        (text.chars().count() as f32 * size * self.advance_ratio, size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monospace_measures_proportionally() {
        let m = Monospace::default();
        let (w, h) = m.measure("hello", 10.0);
        assert!((w - 30.0).abs() < 1e-4, "{w}");
        assert_eq!(h, 10.0);
        assert_eq!(m.measure("", 10.0).0, 0.0);
        assert!(m.measure("hello", 20.0).0 > w);
    }

    #[test]
    fn an_empty_primitive_set_reports_itself_empty() {
        assert!(Primitives::default().is_empty());
        let mut p = Primitives::default();
        p.labels.push(Label {
            at: Point::new(0.0, 0.0),
            text: "x".into(),
            size: 1.0,
            anchor: Anchor::Start,
            color: Rgba::BLACK,
        });
        assert!(!p.is_empty());
    }
}
