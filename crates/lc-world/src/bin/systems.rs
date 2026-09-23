//! Draw what the system generator makes, as the plots in `lightcone/docs/26-system-generation.md`.
//!
//! One sample of stars, seven pictures of it. Nothing here models anything: it runs
//! `sky::generate` under its default tuning and plots the answer, so a figure in the
//! documentation cannot drift from the code that made it.
//!
//! ```bash
//! cargo run -p lc-world --features plots --bin systems -- lightcone/docs/plots
//! ```

use std::process::ExitCode;

use em_plot::chart::{Axis, Chart, Marker, Rect, Style};
use em_plot::primitives::{Anchor, Label, Monospace, Point, Polyline, Primitives, Quad, Rgba};
use em_plot::scale::Scale;
use em_plot::raster;

use lc_world::sky::generate::architecture::Class;
use lc_world::sky::generate::{self, AU, GeneratedSystem, Tuning, disc, planet};
use lc_world::sky::{AuthoredStars, CatalogStar, StarId, StarProvider};
use lc_world::worlds::Atmosphere;

const WIDTH: u32 = 1200;
const HEIGHT: u32 = 730;
const STARS: u64 = 1200;

const INK: Rgba = Rgba(0.14, 0.15, 0.19, 1.0);
const PAPER: Rgba = Rgba(1.0, 1.0, 1.0, 1.0);
const FAINT: Rgba = Rgba(0.55, 0.57, 0.62, 1.0);

const ROCKY: Rgba = Rgba(0.76, 0.42, 0.24, 0.75);
const ICY: Rgba = Rgba(0.45, 0.66, 0.80, 0.75);
const ICE_GIANT: Rgba = Rgba(0.18, 0.55, 0.55, 0.75);
const GAS_GIANT: Rgba = Rgba(0.85, 0.62, 0.20, 0.75);
const MARK: Rgba = Rgba(0.10, 0.10, 0.12, 1.0);

fn color_of(class: Class) -> Rgba {
    match class {
        Class::Rocky => ROCKY,
        Class::Icy => ICY,
        Class::IceGiant => ICE_GIANT,
        Class::GasGiant => GAS_GIANT,
    }
}

fn main() -> ExitCode {
    let into = std::env::args().nth(1).unwrap_or_else(|| "lightcone/docs/plots".to_string());
    match run(&into) {
        Ok(n) => {
            println!("{n} plots written to {into}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("systems: {e}");
            ExitCode::FAILURE
        }
    }
}

/// A sun-like star with this key, and one scaled to a luminosity.
fn star(key: u64, luminosity: f64) -> CatalogStar {
    let teff = 5772.0 * luminosity.powf(0.13);
    let mut s = AuthoredStars::sample().stars()[1].clone();
    s.id = StarId::synthesize("plots", key);
    s.luminosity_solar = luminosity;
    s.star.teff_k = teff;
    s.star.radius_m =
        em_spectra::stellar::radius_from_luminosity(luminosity * em_spectra::stellar::SOLAR_LUMINOSITY, teff);
    s.mass_solar = em_spectra::stellar::main_sequence_mass_solar(luminosity);
    s.star.mu = em_spectra::stellar::mu_from_mass_solar(s.mass_solar);
    // Peculiar speeds as the local disc has them: a half-normal at forty kilometers a second,
    // which puts the mean near thirty and leaves a tail into the halo.
    let speed = lc_world::rng::gaussian(lc_world::rng::hash(&[key, 0xfe])).abs() * 42_000.0;
    s.metallicity = lc_world::sky::metallicity::from_speed(speed, key);
    s
}

/// The sample every plot is drawn from: sun-like stars, and a spread of luminosities.
fn sample() -> Vec<(CatalogStar, GeneratedSystem)> {
    (0..STARS)
        .map(|k| {
            // A luminosity function weighted to the dim end, as the galaxy is.
            let u = lc_world::rng::uniform(lc_world::rng::hash(&[k, 0x1_0f_0f])) ;
            let luminosity = 10f64.powf(-2.5 + 3.5 * u * u);
            let s = star(k, luminosity);
            let system = generate::system_for(&s);
            (s, system)
        })
        .collect()
}

fn run(into: &str) -> Result<usize, String> {
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let sample = sample();
    architecture(into, &sample)?;
    counts(into, &sample)?;
    mass_radius(into, &sample)?;
    retention(into, &sample)?;
    moons(into, &sample)?;
    habitable(into, &sample)?;
    metals(into, &sample)?;
    Ok(7)
}

/// Where the planets are and how heavy, in units of each star's own snow line.
///
/// Every star in the sample. Measuring the axis in snow lines puts a red dwarf's system and a
/// blue star's on the same picture.
fn architecture(into: &str, sample: &[(CatalogStar, GeneratedSystem)]) -> Result<(), String> {
    let mut by_class: [Vec<(f64, f64)>; 4] = Default::default();
    let (mut stalled, mut stirred) = (Vec::new(), Vec::new());
    let mut habitable = (f64::INFINITY, 0.0f64);
    let t = Tuning::default();
    for (star, system) in sample {
        let snow = disc::radius_at(&star.star, t.disc.snow_k);
        for p in &system.planets {
            by_class[p.class as usize].push((p.semi_major_m / snow, p.mass_earths()));
        }
        for b in system.architecture.belts() {
            let point = (b.semi_major_m / snow, b.debris_earths);
            if b.stirred { stirred.push(point) } else { stalled.push(point) }
        }
        habitable.0 = habitable.0.min(disc::radius_at(&star.star, t.disc.habitable_k.0) / snow);
        habitable.1 = habitable.1.max(disc::radius_at(&star.star, t.disc.habitable_k.1) / snow);
    }

    figure(
        into,
        "architecture",
        "Semi-major axis against mass, in snow lines",
        ("semi-major axis, in the star's own snow lines", Scale::Log10, (0.005, 40.0)),
        ("mass, Earths", Scale::Log10, (0.002, 6000.0)),
        |chart| {
            let mut layers = vec![band(chart, habitable, Rgba(0.30, 0.62, 0.36, 0.10)), rule(chart, 1.0, "snow line")];
            layers.push(chart.scatter(&stalled, Marker::new(2.5, Rgba(0.55, 0.56, 0.60, 0.45))));
            layers.push(chart.scatter(&stirred, Marker::new(4.0, Rgba(0.65, 0.20, 0.25, 0.85))));
            for (k, points) in by_class.iter().enumerate() {
                layers.push(chart.scatter(points, Marker::new(2.5, color_of(CLASSES[k]))));
            }
            layers.push(legend(chart, &[
                ("rocky", ROCKY), ("icy", ICY), ("ice giant", ICE_GIANT), ("gas giant", GAS_GIANT),
                ("never grew: too little mass", Rgba(0.55, 0.56, 0.60, 1.0)),
                ("never grew: stirred by a giant", Rgba(0.65, 0.20, 0.25, 1.0)),
                ("the habitable zone", Rgba(0.30, 0.62, 0.36, 1.0)),
            ]));
            layers
        },
    )
}

const CLASSES: [Class; 4] = [Class::Rocky, Class::Icy, Class::IceGiant, Class::GasGiant];

/// How many planets a star gets, and how many of them are giants.
fn counts(into: &str, sample: &[(CatalogStar, GeneratedSystem)]) -> Result<(), String> {
    let all: Vec<f64> = sample.iter().map(|(_, s)| s.planets.len() as f64).collect();
    let giants: Vec<f64> =
        sample.iter().map(|(_, s)| s.planets.iter().filter(|p| p.class.is_giant()).count() as f64).collect();
    let habitable: Vec<f64> = sample.iter().map(|(_, s)| s.habitable().count() as f64).collect();

    figure(
        into,
        "counts",
        "Planets, giants and habitable worlds per star",
        ("bodies", Scale::Linear, (-0.6, 17.6)),
        ("share of stars", Scale::Linear, (0.0, 0.62)),
        |chart| {
            vec![
                bars(chart, &all, 19, -0.30, 0.26, Rgba(0.35, 0.45, 0.70, 0.85)),
                bars(chart, &giants, 19, -0.02, 0.26, GAS_GIANT),
                bars(chart, &habitable, 19, 0.26, 0.26, Rgba(0.30, 0.62, 0.36, 0.85)),
                legend(chart, &[
                    ("planets", Rgba(0.35, 0.45, 0.70, 1.0)),
                    ("giants", GAS_GIANT),
                    ("habitable", Rgba(0.30, 0.62, 0.36, 1.0)),
                ]),
            ]
        },
    )
}

/// Mass against radius, with the three planets the relation is anchored on.
fn mass_radius(into: &str, sample: &[(CatalogStar, GeneratedSystem)]) -> Result<(), String> {
    let mut by_class: [Vec<(f64, f64)>; 4] = Default::default();
    for (_, system) in sample {
        for p in &system.planets {
            by_class[p.class as usize].push((p.mass_earths(), p.radius_earths()));
        }
    }
    let anchors = [(1.0, 1.0, "Earth"), (17.15, 3.86, "Neptune"), (317.8, 11.21, "Jupiter")];

    figure(
        into,
        "mass-radius",
        "Mass against radius",
        ("mass, Earths", Scale::Log10, (0.002, 6000.0)),
        ("radius, Earths", Scale::Log10, (0.1, 30.0)),
        |chart| {
            let mut layers: Vec<Primitives> = by_class
                .iter()
                .enumerate()
                .map(|(k, points)| chart.scatter(points, Marker::new(3.0, color_of(CLASSES[k]))))
                .collect();
            let mut marks = chart.scatter(
                &anchors.iter().map(|(m, r, _)| (*m, *r)).collect::<Vec<_>>(),
                Marker::new(9.0, MARK),
            );
            for (m, r, name) in anchors {
                marks.labels.push(Label {
                    at: chart_point(chart, m, r, 0.0, -14.0),
                    text: name.into(),
                    size: 14.0,
                    anchor: Anchor::Middle,
                    color: MARK,
                });
            }
            layers.push(marks);
            layers.push(legend(chart, &[
                ("rocky", ROCKY), ("icy", ICY), ("ice giant", ICE_GIANT), ("gas giant", GAS_GIANT),
            ]));
            layers
        },
    )
}

/// The chain that decides what a body has over it, with the solar system laid on top.
fn retention(into: &str, sample: &[(CatalogStar, GeneratedSystem)]) -> Result<(), String> {
    let mut by_air: [Vec<(f64, f64)>; 4] = Default::default();
    for (_, system) in sample {
        for p in system.planets.iter().filter(|p| !p.class.is_giant()) {
            let v = disc::escape_speed(p.mass_earths(), p.radius_earths()) / 1000.0;
            by_air[air_index(p.atmosphere)].push((p.equilibrium_k, v));
        }
    }
    // Mass and radius in Earth units, semi-major axis in astronomical units.
    let real: [(f64, f64, f64, &str); 8] = [
        (0.0553, 0.383, 0.387, "Mercury"), (0.815, 0.949, 0.723, "Venus"),
        (1.0, 1.0, 1.0, "Earth"), (0.0123, 0.2727, 1.0, "Luna"),
        (0.107, 0.532, 1.524, "Mars"), (0.015, 0.2859, 5.2, "Io"),
        (0.0225, 0.404, 9.58, "Titan"), (0.00359, 0.2124, 30.07, "Triton"),
    ];

    figure(
        into,
        "retention",
        "Equilibrium temperature against escape velocity",
        ("equilibrium temperature, K", Scale::Log10, (30.0, 2000.0)),
        ("escape velocity, km/s", Scale::Log10, (0.5, 60.0)),
        |chart| {
            let colors = [Rgba(0.75, 0.75, 0.78, 0.7), Rgba(0.45, 0.66, 0.80, 0.8), Rgba(0.30, 0.62, 0.36, 0.8), Rgba(0.85, 0.62, 0.20, 0.8)];
            let mut layers: Vec<Primitives> =
                by_air.iter().enumerate().map(|(k, p)| chart.scatter(p, Marker::new(3.0, colors[k]))).collect();
            layers.push(threshold(chart, 28.0));
            let mut marks = Primitives::default();
            for (mass, radius, au, name) in real {
                let t = planet::REFERENCE_K / au.sqrt();
                let v = disc::escape_speed(mass, radius) / 1000.0;
                marks.quads.push(dot(chart, t, v, 4.0, MARK));
                marks.labels.push(Label {
                    at: chart_point(chart, t, v, 0.0, -13.0),
                    text: name.into(),
                    size: 13.0,
                    anchor: Anchor::Middle,
                    color: MARK,
                });
            }
            layers.push(marks);
            layers.push(legend(chart, &[
                ("airless", Rgba(0.75, 0.75, 0.78, 1.0)),
                ("a trace", Rgba(0.45, 0.66, 0.80, 1.0)),
                ("opaque", Rgba(0.30, 0.62, 0.36, 1.0)),
                ("an envelope", GAS_GIANT),
                ("nitrogen held, as drawn", FAINT),
            ]));
            layers
        },
    )
}

fn air_index(a: Atmosphere) -> usize {
    match a {
        Atmosphere::None => 0,
        Atmosphere::Thin => 1,
        Atmosphere::Thick => 2,
        Atmosphere::Envelope => 3,
    }
}

/// Grown moons against caught ones, in the one plot that separates them.
fn moons(into: &str, sample: &[(CatalogStar, GeneratedSystem)]) -> Result<(), String> {
    let (mut grown, mut caught) = (Vec::new(), Vec::new());
    for (star, system) in sample {
        for p in &system.planets {
            let hill = generate::moon::hill_radius_m(p.semi_major_m, p.mass_kg, star.mass_solar);
            for m in &p.moons {
                let point = (m.semi_major_m / hill, m.inclination_rad.to_degrees());
                if m.regular { grown.push(point) } else { caught.push(point) }
            }
        }
    }
    figure(
        into,
        "moons",
        "Moon semi-major axis against inclination",
        ("semi-major axis, Hill radii", Scale::Log10, (0.0005, 1.0)),
        ("inclination from the planet's equator, degrees", Scale::Linear, (0.0, 180.0)),
        |chart| {
            vec![
                chart.scatter(&caught, Marker::new(2.0, Rgba(0.55, 0.40, 0.70, 0.35))),
                chart.scatter(&grown, Marker::new(3.5, Rgba(0.85, 0.50, 0.15, 0.85))),
                legend(chart, &[
                    ("grown in a disc", Rgba(0.85, 0.50, 0.15, 1.0)),
                    ("caught", Rgba(0.55, 0.40, 0.70, 1.0)),
                ]),
            ]
        },
    )
}

/// The habitable zone, and what lands in it, across three and a half decades of starlight.
fn habitable(into: &str, sample: &[(CatalogStar, GeneratedSystem)]) -> Result<(), String> {
    let (mut all, mut good, mut inner, mut outer) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (star, system) in sample {
        for p in &system.planets {
            let point = (p.semi_major_m / AU, star.luminosity_solar);
            if p.habitable { good.push(point) } else { all.push(point) }
        }
        let t = Tuning::default();
        inner.push((disc::radius_at(&star.star, t.disc.habitable_k.0) / AU, star.luminosity_solar));
        outer.push((disc::radius_at(&star.star, t.disc.habitable_k.1) / AU, star.luminosity_solar));
    }
    inner.sort_by(|a, b| a.1.total_cmp(&b.1));
    outer.sort_by(|a, b| a.1.total_cmp(&b.1));

    figure(
        into,
        "habitable",
        "Semi-major axis against stellar luminosity",
        ("semi-major axis, AU", Scale::Log10, (0.002, 200.0)),
        ("stellar luminosity, Suns", Scale::Log10, (0.003, 40.0)),
        |chart| {
            vec![
                chart.scatter(&all, Marker::new(2.0, Rgba(0.70, 0.71, 0.75, 0.30))),
                track(chart, &inner, Rgba(0.30, 0.62, 0.36, 0.9)),
                track(chart, &outer, Rgba(0.30, 0.62, 0.36, 0.9)),
                chart.scatter(&good, Marker::new(4.0, Rgba(0.16, 0.52, 0.24, 0.95))),
                legend(chart, &[
                    ("habitable", Rgba(0.16, 0.52, 0.24, 1.0)),
                    ("everything else", Rgba(0.70, 0.71, 0.75, 1.0)),
                    ("zone edges", Rgba(0.30, 0.62, 0.36, 1.0)),
                ]),
            ]
        },
    )
}

/// Metals are the rock: what a star's abundance buys it.
fn metals(into: &str, sample: &[(CatalogStar, GeneratedSystem)]) -> Result<(), String> {
    let mut points = Vec::new();
    for (star, system) in sample {
        let giants = system.planets.iter().filter(|p| p.class.is_giant()).count() as f64;
        points.push((star.metallicity, giants + lc_world::rng::uniform(lc_world::rng::hash(&[star.id.get()])) * 0.7 - 0.35));
    }
    let mut binned: Vec<(f64, f64)> = Vec::new();
    for b in 0..14 {
        let lo = -2.1 + b as f64 * 0.2;
        let inside: Vec<f64> = sample
            .iter()
            .filter(|(s, _)| (lo..lo + 0.2).contains(&s.metallicity))
            .map(|(_, sys)| sys.planets.iter().filter(|p| p.class.is_giant()).count() as f64)
            .collect();
        if inside.len() > 8 {
            binned.push((lo + 0.1, inside.iter().sum::<f64>() / inside.len() as f64));
        }
    }

    figure(
        into,
        "metallicity",
        "Giants per system against metallicity",
        ("[Fe/H]", Scale::Linear, (-2.2, 0.7)),
        ("giants in the system", Scale::Linear, (-0.6, 8.0)),
        |chart| {
            vec![
                chart.scatter(&points, Marker::new(2.5, Rgba(0.55, 0.57, 0.70, 0.35))),
                track(chart, &binned, Rgba(0.80, 0.35, 0.20, 1.0)),
                legend(chart, &[
                    ("one star, jittered", Rgba(0.55, 0.57, 0.70, 1.0)),
                    ("mean in a tenth of a dex", Rgba(0.80, 0.35, 0.20, 1.0)),
                ]),
            ]
        },
    )
}

// --- drawing helpers -------------------------------------------------------------------

const AREA: Rect = Rect { x: 104.0, y: 70.0, width: WIDTH as f32 - 148.0, height: HEIGHT as f32 - 132.0 };

fn figure(
    into: &str,
    name: &str,
    title: &str,
    x: (&str, Scale, (f64, f64)),
    y: (&str, Scale, (f64, f64)),
    draw: impl FnOnce(&Chart) -> Vec<Primitives>,
) -> Result<(), String> {
    let metrics = Monospace::default();
    let mut chart = Chart::new(
        AREA,
        Axis { scale: x.1, range: x.2, ticks: 8 },
        Axis { scale: y.1, range: y.2, ticks: 6 },
        &metrics,
    );
    chart.style = Style { axis: INK, text_size: 14.0, line_width: 1.0, ..Style::default() };

    let mut furniture = Primitives::default();
    furniture.labels.push(Label {
        at: Point::new(14.0, 26.0),
        text: title.into(),
        size: 20.0,
        anchor: Anchor::Start,
        color: INK,
    });
    furniture.labels.push(Label {
        at: Point::new(AREA.x + AREA.width / 2.0, AREA.y + AREA.height + 46.0),
        text: x.0.into(),
        size: 15.0,
        anchor: Anchor::Middle,
        color: INK,
    });
    furniture.labels.push(Label {
        at: Point::new(14.0, AREA.y - 14.0),
        text: y.0.into(),
        size: 15.0,
        anchor: Anchor::Start,
        color: INK,
    });

    let mut layers = vec![chart.frame()];
    layers.extend(draw(&chart));
    layers.push(furniture);

    let refs: Vec<&Primitives> = layers.iter().collect();
    let pixmap = raster::render(&refs, WIDTH, HEIGHT, PAPER).ok_or("could not allocate the image")?;
    pixmap.save_png(format!("{into}/{name}.png")).map_err(|e| e.to_string())
}

fn chart_point(chart: &Chart, vx: f64, vy: f64, dx: f32, dy: f32) -> Point {
    let t = (
        chart.x.scale.normalize(vx, chart.x.range) as f32,
        chart.y.scale.normalize(vy, chart.y.range) as f32,
    );
    Point::new(chart.area.x + t.0 * chart.area.width + dx, chart.area.y + (1.0 - t.1) * chart.area.height + dy)
}

fn dot(chart: &Chart, vx: f64, vy: f64, half: f32, color: Rgba) -> Quad {
    let p = chart_point(chart, vx, vy, 0.0, 0.0);
    Quad { min: Point::new(p.x - half, p.y - half), max: Point::new(p.x + half, p.y + half), color }
}

/// A shaded vertical band, for a range that means something.
fn band(chart: &Chart, span: (f64, f64), color: Rgba) -> Primitives {
    let lo = chart_point(chart, span.0, chart.y.range.1, 0.0, 0.0);
    let hi = chart_point(chart, span.1, chart.y.range.0, 0.0, 0.0);
    Primitives { quads: vec![Quad { min: lo, max: hi, color }], ..Primitives::default() }
}

/// A labeled vertical line, for a radius that means something.
fn rule(chart: &Chart, at: f64, text: &str) -> Primitives {
    let top = chart_point(chart, at, chart.y.range.1, 0.0, 0.0);
    let bottom = chart_point(chart, at, chart.y.range.0, 0.0, 0.0);
    Primitives {
        polylines: vec![Polyline { points: vec![top, bottom], color: FAINT, width: 1.0 }],
        labels: vec![Label { at: Point::new(top.x + 6.0, top.y + 14.0), text: text.into(), size: 13.0, anchor: Anchor::Start, color: FAINT }],
        ..Primitives::default()
    }
}

/// The escape velocity at which a molecule of `molar_g` is held, across the temperature axis.
fn threshold(chart: &Chart, molar_g: f64) -> Primitives {
    let tuning = Tuning::default();
    let points: Vec<Point> = (0..=160)
        .map(|k| {
            let t = chart.x.scale.denormalize(k as f64 / 160.0, chart.x.range);
            let exo = planet::exosphere_k(t, &tuning);
            let v = tuning.world.retention * disc::thermal_speed(molar_g, exo) / 1000.0;
            chart_point(chart, t, v, 0.0, 0.0)
        })
        .collect();
    Primitives { polylines: vec![Polyline { points, color: FAINT, width: 2.0 }], ..Primitives::default() }
}

fn track(chart: &Chart, points: &[(f64, f64)], color: Rgba) -> Primitives {
    let line: Vec<Point> = points.iter().map(|(x, y)| chart_point(chart, *x, *y, 0.0, 0.0)).collect();
    Primitives { polylines: vec![Polyline { points: line, color, width: 2.0 }], ..Primitives::default() }
}

/// A histogram of integer counts, as a share of the sample, offset so three can sit side by side.
fn bars(chart: &Chart, values: &[f64], bins: usize, offset: f64, width: f64, color: Rgba) -> Primitives {
    let mut counts = vec![0usize; bins];
    for v in values {
        let k = (*v as usize).min(bins - 1);
        counts[k] += 1;
    }
    let total = values.len().max(1) as f64;
    let quads = counts
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(k, n)| {
            let share = *n as f64 / total;
            let lo = chart_point(chart, k as f64 + offset, 0.0, 0.0, 0.0);
            let hi = chart_point(chart, k as f64 + offset + width, share, 0.0, 0.0);
            Quad { min: Point::new(lo.x, hi.y), max: Point::new(hi.x, lo.y), color }
        })
        .collect();
    Primitives { quads, ..Primitives::default() }
}

fn legend(chart: &Chart, entries: &[(&str, Rgba)]) -> Primitives {
    let mut out = Primitives::default();
    let right = chart.area.x + chart.area.width - 10.0;
    for (k, (text, color)) in entries.iter().enumerate() {
        let y = chart.area.y + 18.0 + k as f32 * 21.0;
        out.quads.push(Quad {
            min: Point::new(right - 11.0, y - 10.0),
            max: Point::new(right, y - 1.0),
            color: Rgba(color.0, color.1, color.2, 1.0),
        });
        out.labels.push(Label {
            at: Point::new(right - 18.0, y),
            text: (*text).into(),
            size: 14.0,
            anchor: Anchor::End,
            color: INK,
        });
    }
    out
}
