//! The bake's distributions as charts, each against the curve the field promises.
//!
//! em-plot draws them, as it draws the client's light curve; this file turns its raster into an
//! egui texture and redraws only when the bake or the size changes.

use bevy_egui::egui;
use em_plot::chart::{Axis, Bin, Chart, Rect, Style};
use em_plot::primitives::{Anchor, Label, Point, Polyline, Primitives, Rgba};
use em_plot::{raster, scale::Scale};
use lc_galaxy::attributes::{MAX_MASS, MIN_MASS, age_fraction, imf_fraction, tracks};
use lc_galaxy::kinds::MAX_AGE_GYR;
use lc_galaxy::{BakedStar, Compiled, FeatureId};

use crate::doc::Doc;
use crate::palette;

const BG: Rgba = Rgba::opaque(0.024, 0.027, 0.047);
const FG: Rgba = Rgba::opaque(0.62, 0.66, 0.72);
const BARS: Rgba = Rgba(0.43, 0.59, 0.78, 0.85);
const EXPECTED: Rgba = Rgba::opaque(0.95, 0.72, 0.35);
const ZAMS: Rgba = Rgba(0.75, 0.75, 0.80, 0.6);

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Plot {
    #[default]
    Hr,
    Mass,
    Age,
    Feh,
}

impl Plot {
    const ALL: [Plot; 4] = [Plot::Hr, Plot::Mass, Plot::Age, Plot::Feh];

    pub fn named(name: &str) -> Option<Plot> {
        Some(match name {
            "hr" => Plot::Hr,
            "mass" => Plot::Mass,
            "age" => Plot::Age,
            "feh" => Plot::Feh,
            _ => return None,
        })
    }

    fn label(self) -> &'static str {
        match self {
            Plot::Hr => "HR diagram",
            Plot::Mass => "initial mass",
            Plot::Age => "age",
            Plot::Feh => "[Fe/H]",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Key {
    plot: Plot,
    generation: u64,
    size: (u32, u32),
}

#[derive(Default)]
pub struct Charts {
    pub plot: Plot,
    drawn: Option<(Key, egui::TextureHandle)>,
}

impl Charts {
    pub fn showing(plot: Plot) -> Self {
        Self { plot, drawn: None }
    }
}

pub fn show(ui: &mut egui::Ui, charts: &mut Charts, doc: &Doc) {
    let Some(r) = &doc.result else { return };
    let Ok(bake) = &r.bake else { return };
    ui.horizontal(|ui| {
        for p in Plot::ALL {
            ui.selectable_value(&mut charts.plot, p, p.label());
        }
    });
    let width = ui.available_width().max(160.0);
    let size = (width as u32, (width * 0.62) as u32);
    let key = Key { plot: charts.plot, generation: doc.generation, size };
    if charts.drawn.as_ref().map(|(k, _)| *k) != Some(key) {
        let image = render(charts.plot, &bake.stars, &r.compiled, size);
        let texture = ui.ctx().load_texture("galaxy chart", image, egui::TextureOptions::LINEAR);
        charts.drawn = Some((key, texture));
    }
    if let Some((_, texture)) = &charts.drawn {
        ui.image((texture.id(), egui::vec2(size.0 as f32, size.1 as f32)));
    }
}

fn rgba(c: egui::Color32) -> Rgba {
    Rgba::opaque(c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0)
}

/// A chart with room for its tick labels and two axis titles.
fn chart<'a>(x: Axis, y: Axis, size: (u32, u32), metrics: &'a raster::BitmapMetrics) -> Chart<'a> {
    let (w, h) = (size.0 as f32, size.1 as f32);
    let area = Rect { x: 8.0, y: 10.0, width: w - 16.0, height: h - 50.0 };
    let mut c = Chart::new(area, x, y, metrics);
    c.style = Style { axis: FG, text_size: 10.0, ..Style::default() };
    let margin = c.y_label_width() + 22.0;
    c.area = Rect { x: margin, width: w - margin - 10.0, ..area };
    c
}

fn at(c: &Chart, x: f64, y: f64) -> Point {
    Point::new(
        c.area.x + c.x.scale.normalize(x, c.x.range) as f32 * c.area.width,
        c.area.y + c.area.height - c.y.scale.normalize(y, c.y.range) as f32 * c.area.height,
    )
}

fn titles(c: &Chart, x: &str, y: &str) -> Primitives {
    let label = |at: Point, text: &str, anchor| Label { at, text: text.into(), size: 10.0, anchor, color: FG };
    Primitives {
        labels: vec![
            label(Point::new(c.area.x + c.area.width / 2.0, c.area.y + c.area.height + 30.0), x, Anchor::Middle),
            label(Point::new(c.area.x + 4.0, c.area.y + 10.0), y, Anchor::Start),
        ],
        ..Primitives::default()
    }
}

fn edges(lo: f64, hi: f64, n: usize, log: bool) -> Vec<(f64, f64)> {
    let f = |i: usize| {
        let t = i as f64 / n as f64;
        if log { lo * (hi / lo).powf(t) } else { lo + (hi - lo) * t }
    };
    (0..n).map(|i| (f(i), f(i + 1))).collect()
}

fn count(edges: &[(f64, f64)], values: impl Iterator<Item = f64>) -> Vec<Bin> {
    let mut bins: Vec<Bin> = edges.iter().map(|&(a, b)| (a, b, 0.0)).collect();
    for v in values {
        if let Some(bin) = bins.iter_mut().find(|(a, b, _)| v >= *a && v < *b) {
            bin.2 += 1.0;
        }
    }
    bins
}

/// Baked stars per population, which scales each population's expected curve.
fn per_population(stars: &[BakedStar], compiled: &Compiled) -> Vec<(FeatureId, f64)> {
    compiled
        .populations
        .keys()
        .map(|id| (*id, stars.iter().filter(|s| s.attributes.population == Some(*id)).count() as f64))
        .collect()
}

fn y_axis(bins: &[Bin], expected: &[Bin]) -> Axis {
    let peak = bins.iter().chain(expected).map(|b| b.2).fold(1.0, f64::max);
    Axis { scale: Scale::Linear, range: (0.0, peak * 1.15), ticks: 4 }
}

fn render(plot: Plot, stars: &[BakedStar], compiled: &Compiled, size: (u32, u32)) -> egui::ColorImage {
    let metrics = raster::BitmapMetrics;
    let counts = per_population(stars, compiled);
    let layers: Vec<Primitives> = match plot {
        Plot::Hr => {
            let x = Axis { scale: Scale::Log10, range: (40_000.0, 2_000.0), ticks: 4 };
            let y = Axis { scale: Scale::Log10, range: (1e-4, 1e6), ticks: 5 };
            let c = chart(x, y, size, &metrics);
            let zams: Vec<Point> = edges(MIN_MASS, MAX_MASS, 120, true)
                .iter()
                .map(|(m, _)| tracks::evolve(*m, 0.0, 0.0))
                .map(|e| at(&c, e.teff_k, e.luminosity_solar))
                .collect();
            let lit: Vec<&BakedStar> = stars.iter().filter(|s| s.attributes.luminosity_solar > 0.0).collect();
            let points: Vec<(f64, f64)> = lit.iter().map(|s| (s.attributes.teff_k, s.attributes.luminosity_solar)).collect();
            let scatter = c.scatter_with(&points, 4.0, |i, _| {
                let slot = lit[i].attributes.population.and_then(|p| compiled.population_index(p)).unwrap_or(0);
                rgba(palette::POPULATIONS[slot % palette::POPULATIONS.len()])
            });
            vec![
                c.frame(),
                Primitives { polylines: vec![Polyline { points: zams, color: ZAMS, width: 1.0 }], ..Primitives::default() },
                scatter,
                titles(&c, "Teff (K)", "L / Lsun"),
            ]
        }
        Plot::Mass => {
            let e = edges(MIN_MASS, MAX_MASS, 24, true);
            let bins = count(&e, stars.iter().map(|s| s.attributes.initial_mass_solar));
            let expected: Vec<Bin> = e
                .iter()
                .map(|&(lo, hi)| {
                    let n = counts.iter().map(|(id, n)| n * imf_fraction(&compiled.populations[id].imf, lo, hi)).sum();
                    (lo, hi, n)
                })
                .collect();
            let x = Axis { scale: Scale::Log10, range: (MIN_MASS, MAX_MASS), ticks: 4 };
            let c = chart(x, y_axis(&bins, &expected), size, &metrics);
            vec![c.frame(), c.histogram(&bins, BARS), c.steps(&expected, EXPECTED), titles(&c, "initial mass (Msun)", "stars")]
        }
        Plot::Age => {
            let e = edges(0.0, MAX_AGE_GYR, 28, false);
            let bins = count(&e, stars.iter().map(|s| s.attributes.age_gyr));
            let expected: Vec<Bin> = e
                .iter()
                .map(|&(lo, hi)| {
                    let n = counts.iter().map(|(id, n)| n * age_fraction(&compiled.populations[id].age, lo, hi)).sum();
                    (lo, hi, n)
                })
                .collect();
            let x = Axis::linear((0.0, MAX_AGE_GYR));
            let c = chart(x, y_axis(&bins, &expected), size, &metrics);
            vec![c.frame(), c.histogram(&bins, BARS), c.steps(&expected, EXPECTED), titles(&c, "age (Gyr)", "stars")]
        }
        Plot::Feh => {
            let e = edges(-2.5, 1.0, 35, false);
            let bins = count(&e, stars.iter().map(|s| s.attributes.feh));
            let c = chart(Axis::linear((-2.5, 1.0)), y_axis(&bins, &[]), size, &metrics);
            vec![c.frame(), c.histogram(&bins, BARS), titles(&c, "[Fe/H] (dex)", "stars")]
        }
    };
    let refs: Vec<&Primitives> = layers.iter().collect();
    match raster::render(&refs, size.0, size.1, BG) {
        // tiny_skia's RGBA is premultiplied, which is what egui takes.
        Some(pixmap) => egui::ColorImage::from_rgba_premultiplied([size.0 as usize, size.1 as usize], pixmap.data()),
        None => egui::ColorImage::new([1, 1], vec![egui::Color32::BLACK]),
    }
}
