//! The light curve as a picture: em-plot draws it, egui shows it.
//!
//! Two things meet here that otherwise have no reason to know about each other. em-plot
//! rasterises to a `tiny_skia` pixmap with no engine and no UI toolkit in it, which is what
//! made the headless snapshot possible; egui wants a texture. The bridge is a copy and a
//! fingerprint, and keeping it in one file is what stops the plotting code learning about egui.

use bevy_egui::egui;
use em_plot::chart::{Axis, Chart, Rect, Style};
use em_plot::primitives::{Primitives, Rgba};
use em_plot::{raster, scale::Scale};
use em_spectra::Band;

const BG: Rgba = Rgba::opaque(0.07, 0.08, 0.11);
const FG: Rgba = Rgba::opaque(0.72, 0.76, 0.82);
const SERIES: Rgba = Rgba::opaque(0.35, 0.75, 1.0);
const MEAN: Rgba = Rgba::opaque(0.95, 0.72, 0.35);
/// The unobscured star. Everything in the plot is read against this line.
const BASELINE: Rgba = Rgba(0.55, 0.60, 0.70, 0.55);

const YEAR_S: f64 = 31_557_600.0;

/// What a drawing depends on. Anything else changing does not need a redraw.
#[derive(Clone, Copy, PartialEq)]
struct Drawn {
    samples: usize,
    span: (f64, f64),
    size: (u32, u32),
}

/// A cached rendering of the curve.
#[derive(Default)]
pub struct CurvePlot {
    texture: Option<egui::TextureHandle>,
    drawn: Option<Drawn>,
}

impl CurvePlot {
    /// Draw the curve into `ui`, rasterising only when what it shows has changed.
    pub fn show(&mut self, ui: &mut egui::Ui, samples: &[(f64, f64)], size: egui::Vec2) {
        let (w, h) = (size.x.max(64.0) as u32, size.y.max(48.0) as u32);
        if samples.len() < 2 {
            ui.weak("No measurements yet. Point the telescope and wait for the light.");
            return;
        }
        let now = Drawn {
            samples: samples.len(),
            span: (samples[0].0, samples[samples.len() - 1].0),
            size: (w, h),
        };
        if self.drawn != Some(now) || self.texture.is_none() {
            if let Some(image) = render(samples, w, h) {
                match &mut self.texture {
                    Some(handle) => handle.set(image, egui::TextureOptions::LINEAR),
                    None => {
                        self.texture =
                            Some(ui.ctx().load_texture("light-curve", image, egui::TextureOptions::LINEAR))
                    }
                }
                self.drawn = Some(now);
            }
        }
        if let Some(texture) = &self.texture {
            ui.add(egui::Image::new(texture).fit_to_exact_size(size));
        }
    }
}

/// Rasterise the curve. Separate from the widget so it can be checked without a context.
/// The caption is egui's job, not the plot's: baked into the bitmap it collided with the axis
/// labels, and egui draws text better than a 12-pixel bitmap font does.
pub fn caption(band: Band) -> String {
    format!("{band:?} — flux against the bare star, by emission year")
}

pub fn render(samples: &[(f64, f64)], width: u32, height: u32) -> Option<egui::ColorImage> {
    if samples.len() < 2 || width == 0 || height == 0 {
        return None;
    }
    // Relative flux, not the deficit: with re-emission a measurement can land above the
    // unobscured star, and a plot of "how much is missing" cannot show that at all.
    let flux: Vec<(f64, f64)> =
        samples.iter().map(|(t, d)| (t / YEAR_S, 1.0 - d)).collect();

    let span = (flux[0].0, flux[flux.len() - 1].0);
    let span = if span.1 > span.0 { span } else { (span.0, span.0 + 1e-9) };
    // The baseline is always in frame. A curve auto-scaled to its own noise looks like a
    // detection even when the star is doing nothing.
    let extent = flux.iter().fold((1.0f64, 1.0f64), |r, (_, f)| (r.0.min(*f), r.1.max(*f)));

    let metrics = raster::BitmapMetrics;
    let style = Style { axis: FG, text_size: 11.0, series: SERIES, ..Style::default() };
    let area = Rect { x: 8.0, y: 8.0, width: width as f32 - 16.0, height: height as f32 - 26.0 };

    let x = Axis { scale: Scale::Linear, range: span, ticks: 5 };
    let y = Axis { scale: Scale::Linear, range: Scale::Linear.pad(extent, 0.2), ticks: 4 };
    let mut chart = Chart::new(area, x, y, &metrics);
    chart.style = style;
    // Measured with the style it will be drawn with: a flux axis wants several decimals, and a
    // fixed margin clips the leading digits off the left edge.
    let margin = chart.y_label_width() + 12.0;
    chart.area = Rect { x: margin, width: width as f32 - margin - 12.0, ..area };

    let mut layers = vec![chart.frame()];
    let mut baseline = chart.series(&[(span.0, 1.0), (span.1, 1.0)]);
    for line in &mut baseline.polylines {
        line.color = BASELINE;
    }
    layers.push(baseline);
    layers.push(chart.series(&flux));
    layers.push(chart.mean_track(&flux, MEAN));

    let refs: Vec<&Primitives> = layers.iter().collect();
    let pixmap = raster::render(&refs, width, height, BG)?;
    // tiny_skia hands back premultiplied RGBA, which is what egui wants; saying so is cheaper
    // than dividing it out and multiplying it back.
    Some(egui::ColorImage::from_rgba_premultiplied(
        [width as usize, height as usize],
        pixmap.data(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(n: usize, f: impl Fn(usize) -> f64) -> Vec<(f64, f64)> {
        (0..n).map(|k| (k as f64 * YEAR_S * 0.01, f(k))).collect()
    }

    #[test]
    fn a_curve_rasterises_to_the_size_it_was_asked_for() {
        let image = render(&curve(400, |k| (k % 40) as f64 * 1e-4), 520, 190)
            .expect("an image");
        assert_eq!(image.size, [520, 190]);
        assert_eq!(image.pixels.len(), 520 * 190);
    }

    #[test]
    fn too_few_samples_draw_nothing_rather_than_a_degenerate_axis() {
        assert!(render(&[], 400, 150).is_none());
        assert!(render(&curve(1, |_| 0.0), 400, 150).is_none());
        assert!(render(&curve(2, |_| 0.0), 0, 150).is_none());
    }

    /// A flat curve still has to produce a picture. The span and the range are both degenerate
    /// and an unguarded axis turns that into a division by zero or an empty frame.
    #[test]
    fn a_curve_that_never_moves_still_draws() {
        let flat = vec![(0.0, 0.0); 64];
        let image = render(&flat, 400, 150).expect("a flat curve is still a curve");
        assert!(image.pixels.iter().any(|p| *p != image.pixels[0]), "the frame should be drawn");
    }

    /// The bug this guards: a curve scaled to its own noise looks like a detection. Whatever
    /// else is in frame, the unobscured star has to be.
    #[test]
    fn the_baseline_is_always_in_frame() {
        for offset in [-0.4, -1e-6, 0.0, 1e-6, 0.4] {
            let samples = curve(64, |_| offset);
            let image = render(&samples, 400, 150).expect("an image");
            let lit = image.pixels.iter().filter(|p| p.r() > 90 || p.g() > 90).count();
            assert!(lit > 100, "offset {offset} drew almost nothing: {lit} lit pixels");
        }
    }

    /// An excess is not a small deficit. Re-emission puts measurements above the bare star, and
    /// the two must not render identically.
    #[test]
    fn an_excess_and_a_deficit_do_not_look_the_same() {
        let deficit = render(&curve(200, |_| 0.3), 400, 150).unwrap();
        let excess = render(&curve(200, |_| -0.3), 400, 150).unwrap();
        assert_ne!(deficit.pixels, excess.pixels);
    }

    /// The caption is text, not pixels. Baked into the bitmap it collided with the axis
    /// labels, and the picture does not otherwise depend on which band it is of.
    #[test]
    fn the_band_is_named_in_text_beside_the_picture() {
        assert!(caption(Band::ThermalIr).contains("ThermalIr"));
        assert!(caption(Band::V).contains("emission year"), "say what the axis is");
    }

    /// A million samples is a real case: the curve holds thousands and the plot is hundreds of
    /// pixels wide, so decimation is doing the work and the cost must not track the sample count.
    #[test]
    fn a_long_curve_costs_about_what_a_short_one_does() {
        let short = std::time::Instant::now();
        render(&curve(200, |k| (k % 17) as f64 * 1e-3), 400, 150).unwrap();
        let short = short.elapsed();
        let long = std::time::Instant::now();
        render(&curve(200_000, |k| (k % 17) as f64 * 1e-3), 400, 150).unwrap();
        let long = long.elapsed();
        assert!(
            long < short * 200 + std::time::Duration::from_millis(50),
            "a thousandfold more samples took {long:?} against {short:?}"
        );
    }
}
