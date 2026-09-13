//! Render what the client knows, without opening a window.
//!
//! A window is the obvious way to check a renderer and it is not the only one. This draws
//! the same session state — the sky as the observer sees it, and the light curve the
//! telescope has accumulated — straight to a PNG, so the data path can be checked on its own.

use std::process::ExitCode;

use em_plot::chart::{Axis, Chart, Marker, Rect, Style};
use em_plot::colormap::ColorMap;
use em_plot::primitives::{Anchor, Label, Point, Primitives, Rgba};
use em_plot::{raster, scale::Scale};
use em_spectra::{Band, BandMask, presets};
use lc_client::session::{Session, TIME_RATE};
use lc_world::instrument::Instrument;
use lc_world::sky::{AuthoredStars, StarProvider};

const WIDTH: u32 = 1400;
const HEIGHT: u32 = 1000;
const BG: Rgba = Rgba::opaque(0.07, 0.08, 0.11);
const FG: Rgba = Rgba::opaque(0.82, 0.85, 0.90);

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let output = args.next().unwrap_or_else(|| "snapshot.png".into());
    let catalogue = args.next();

    match run(&output, catalogue.as_deref()) {
        Ok(msg) => {
            println!("{msg}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("snapshot: {e}");
            ExitCode::FAILURE
        }
    }
}

fn load(path: Option<&str>) -> Result<Box<dyn StarProvider>, String> {
    match path {
        Some(p) => {
            let hyg = lc_world::sky::hyg::HygProvider::load(p).map_err(|e| e.to_string())?;
            Ok(Box::new(hyg))
        }
        None => Ok(Box::new(AuthoredStars::sample())),
    }
}

fn run(output: &str, catalogue: Option<&str>) -> Result<String, String> {
    let provider = load(catalogue)?;
    let mut session = Session::new(provider.as_ref(), 4000);
    session.telescope = Instrument::BASELINE
        .with_aperture(4.0)
        .with_bands(BandMask::ALL)
        .cooled_to(45.0);

    // Point at the nearest modelled system we are not effectively inside. The catalogue
    // puts the Sun about an astronomical unit away, which is not an interstellar target.
    let target = session
        .stars
        .iter()
        .take(lc_client::session::MODELLED_STARS)
        .find(|s| s.position_ly.length() > 1.0)
        .map(|s| s.id)
        .unwrap_or(session.stars[0].id);
    session.point_at(Some(target));
    let name = session
        .star(target)
        .and_then(|s| s.name.clone())
        .unwrap_or_else(|| "nearest star".into());

    // Two in-game years, sampled finely enough to resolve a transit.
    let real_seconds = 2.0 * 31_557_600.0 / TIME_RATE;
    let steps = 3000;
    for _ in 0..steps {
        session.advance(real_seconds / steps as f64);
        session.observe(2.0e4);
    }

    // The metrics must be the ones the raster backend draws with, or labels overflow.
    let metrics = raster::BitmapMetrics;
    let mut layers: Vec<Primitives> = Vec::new();

    // --- all-sky map -----------------------------------------------------------------
    let sky_area = Rect { x: 90.0, y: 60.0, width: WIDTH as f32 - 130.0, height: 440.0 };
    let sky_chart = {
        let mut c = Chart::new(
            sky_area,
            Axis { scale: Scale::Linear, range: (-180.0, 180.0), ticks: 7 },
            Axis { scale: Scale::Linear, range: (-90.0, 90.0), ticks: 5 },
            &metrics,
        );
        c.style = Style { axis: FG, text_size: 13.0, ..Style::default() };
        c
    };
    layers.push(sky_chart.frame());

    let sky = session.sky();
    // Brightness reads as size across fourteen stops, not as value across the tone map's
    // two and a half: a faint star is small, not black.
    let brightness = |s: &lc_client::session::SkyStar| {
        s.shaded.point_brightness(lc_client::session::POINT_STOPS)
    };
    for bucket in 0..4 {
        let (lo, hi) = (bucket as f32 / 4.0, (bucket + 1) as f32 / 4.0);
        let members: Vec<&lc_client::session::SkyStar> = sky
            .iter()
            .filter(|s| {
                let v = brightness(s);
                v > lo && (v <= hi || bucket == 3)
            })
            .collect();
        if members.is_empty() {
            continue;
        }
        let points: Vec<(f64, f64)> = members
            .iter()
            .map(|s| {
                let p = s.position_ly.normalize_or_zero();
                (p.y.atan2(p.x).to_degrees(), p.z.clamp(-1.0, 1.0).asin().to_degrees())
            })
            .collect();
        let colours: Vec<Rgba> = members
            .iter()
            .map(|s| {
                let c = lc_client::session::point_colour(&s.shaded);
                Rgba(c.x, c.y, c.z, 0.9)
            })
            .collect();
        let size = 1.0 + bucket as f32 * 1.4;
        layers.push(sky_chart.scatter_with(&points, size, |i, _| colours[i]));
    }

    // --- light curve -----------------------------------------------------------------
    let curve_area = Rect { x: 90.0, y: 610.0, width: WIDTH as f32 - 130.0, height: 320.0 };
    let samples = session.curve.samples().to_vec();
    if samples.is_empty() {
        return Err("the telescope recorded nothing".into());
    }
    let years: Vec<(f64, f64)> =
        samples.iter().map(|(t, d)| (t / 31_557_600.0, 1.0 - d)).collect();
    let span = (
        years.first().unwrap().0,
        years.last().unwrap().0,
    );
    let flux_range = years.iter().fold((f64::MAX, f64::MIN), |r, (_, f)| (r.0.min(*f), r.1.max(*f)));

    // Measure the y labels before placing the axis: a flux axis needs seven decimals, and a
    // fixed margin clips the leading digits off the left edge. The measurement has to use the
    // style the chart will *draw* with — measuring at the default size and drawing at 13 was
    // the bug this replaces.
    let style = Style {
        axis: FG,
        text_size: 13.0,
        series: Rgba::opaque(0.35, 0.75, 1.0),
        ..Style::default()
    };
    let x_axis = Axis { scale: Scale::Linear, range: span, ticks: 8 };
    let y_axis =
        Axis { scale: Scale::Linear, range: Scale::Linear.pad(flux_range, 0.15), ticks: 5 };

    let mut curve_chart = Chart::new(curve_area, x_axis, y_axis, &metrics);
    curve_chart.style = style;
    let margin = curve_chart.y_label_width() + 14.0;
    let curve_area = Rect { x: margin, width: WIDTH as f32 - margin - 40.0, ..curve_area };
    curve_chart.area = curve_area;
    layers.push(curve_chart.frame());
    layers.push(curve_chart.series(&years));
    layers.push(curve_chart.mean_track(&years, Rgba::opaque(0.45, 0.85, 1.0)));

    // --- furniture -------------------------------------------------------------------
    let mut text = Primitives::default();
    let mut say = |x: f32, y: f32, s: String, size: f32, anchor, colour| {
        text.labels.push(Label { at: Point::new(x, y), text: s, size, anchor, colour });
    };
    say(WIDTH as f32 / 2.0, 34.0, "LIGHTCONE - OBSERVER SNAPSHOT".into(), 20.0, Anchor::Middle, FG);
    say(90.0, 54.0, format!("SKY - {} STARS", sky.len()), 13.0, Anchor::Start, FG);
    say(
        WIDTH as f32 - 40.0,
        54.0,
        format!("T + {:.2} YEARS", session.coordinate_time_s() / 31_557_600.0),
        13.0,
        Anchor::End,
        FG,
    );
    say(curve_area.x, 592.0, format!("LIGHT CURVE - {}", name.to_uppercase()), 13.0, Anchor::Start, FG);
    let age_years = sky
        .iter()
        .find(|s| s.id == target)
        .map(|s| s.light_age_s / 31_557_600.0)
        .unwrap_or(0.0);
    say(
        WIDTH as f32 - 40.0,
        592.0,
        format!("LIGHT IS {age_years:.1} YEARS OLD"),
        13.0,
        Anchor::End,
        Rgba::opaque(0.95, 0.70, 0.35),
    );
    say(
        WIDTH as f32 / 2.0,
        HEIGHT as f32 - 22.0,
        "EMISSION TIME (YEARS) - RELATIVE FLUX".into(),
        13.0,
        Anchor::Middle,
        FG,
    );
    let _ = ColorMap::Viridis;
    layers.push(text);

    let refs: Vec<&Primitives> = layers.iter().collect();
    let pixmap = raster::render(&refs, WIDTH, HEIGHT, BG).ok_or("could not allocate the image")?;
    pixmap.save_png(output).map_err(|e| format!("{output}: {e}"))?;

    let deepest = session.curve.deepest();
    Ok(format!(
        "{} stars, {} samples over {:.2} years, deepest dip {:.3e} -> {output}",
        sky.len(),
        session.curve.len(),
        span.1 - span.0,
        deepest
    ))
}
