//! Plot a CSV, or a built-in demo series, to PNG.
//!
//! Exists so the plotting module can be tuned on its own, with something to look at.

use std::process::ExitCode;

use em_plot::chart::{Axis, Chart, Rect, Style};
use em_plot::primitives::{Anchor, Label, Monospace, Point, Primitives, Rgba};
use em_plot::scale::Scale;
use em_plot::{raster, svg};

const USAGE: &str = "\
em-plot - plot a CSV or a demo series

  em-plot <input.csv> <output.png> [options]
  em-plot --demo <name> <output.png> [options]

Options
  --x <col>        x column, by header name or index        (default 0)
  --y <col>        y column, repeatable                     (default 1)
  --width <px>                                              (default 1400)
  --height <px>                                             (default 760)
  --xlog           logarithmic x
  --ylog           logarithmic y
  --symlog <t>     symmetric-log y about a linear threshold
  --title <text>
  --xlabel <text>
  --ylabel <text>
  --dark           light on dark
  --svg            also write <output>.svg
  --demo <name>    transit | flicker | periodogram | list
";

struct Options {
    input: Option<String>,
    output: String,
    demo: Option<String>,
    x: String,
    ys: Vec<String>,
    width: u32,
    height: u32,
    x_scale: Scale,
    y_scale: Scale,
    title: Option<String>,
    xlabel: Option<String>,
    ylabel: Option<String>,
    dark: bool,
    svg: bool,
}

fn main() -> ExitCode {
    match run() {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("em-plot: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Result<Options, String> {
    let mut args = std::env::args().skip(1).peekable();
    let mut positional = Vec::new();
    let mut o = Options {
        input: None,
        output: String::new(),
        demo: None,
        x: "0".into(),
        ys: Vec::new(),
        width: 1400,
        height: 760,
        x_scale: Scale::Linear,
        y_scale: Scale::Linear,
        title: None,
        xlabel: None,
        ylabel: None,
        dark: false,
        svg: false,
    };
    while let Some(a) = args.next() {
        let mut want = |flag: &str| -> Result<String, String> {
            args.next().ok_or_else(|| format!("{flag} needs a value"))
        };
        match a.as_str() {
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--x" => o.x = want("--x")?,
            "--y" => o.ys.push(want("--y")?),
            "--width" => o.width = want("--width")?.parse().map_err(|_| "bad --width")?,
            "--height" => o.height = want("--height")?.parse().map_err(|_| "bad --height")?,
            "--xlog" => o.x_scale = Scale::Log10,
            "--ylog" => o.y_scale = Scale::Log10,
            "--symlog" => {
                let t = want("--symlog")?.parse().map_err(|_| "bad --symlog")?;
                o.y_scale = Scale::SymLog { linear_threshold: t };
            }
            "--title" => o.title = Some(want("--title")?),
            "--xlabel" => o.xlabel = Some(want("--xlabel")?),
            "--ylabel" => o.ylabel = Some(want("--ylabel")?),
            "--dark" => o.dark = true,
            "--svg" => o.svg = true,
            "--demo" => o.demo = Some(want("--demo")?),
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => positional.push(other.to_string()),
        }
    }

    if o.demo.is_some() {
        o.output = positional.first().cloned().ok_or("need an output path")?;
    } else {
        o.input = Some(positional.first().cloned().ok_or_else(|| USAGE.to_string())?);
        o.output = positional.get(1).cloned().ok_or("need an output path")?;
    }
    if o.ys.is_empty() {
        o.ys.push("1".into());
    }
    Ok(o)
}

fn run() -> Result<String, String> {
    let o = parse_args()?;
    let (series, names) = match &o.demo {
        Some(name) => demo(name)?,
        None => read_csv(o.input.as_ref().unwrap(), &o.x, &o.ys)?,
    };
    if series.iter().all(|s| s.is_empty()) {
        return Err("no finite points to plot".into());
    }
    draw(&o, &series, &names)
}

/// Column by header name, else by index.
fn column(headers: &csv::StringRecord, key: &str) -> Result<usize, String> {
    if let Some(i) = headers.iter().position(|h| h == key) {
        return Ok(i);
    }
    key.parse::<usize>().map_err(|_| format!("no column {key:?}"))
}

fn read_csv(path: &str, x: &str, ys: &[String]) -> Result<(Vec<Vec<(f64, f64)>>, Vec<String>), String> {
    let mut reader = csv::Reader::from_path(path).map_err(|e| format!("{path}: {e}"))?;
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let xi = column(&headers, x)?;
    let yis: Vec<usize> = ys.iter().map(|y| column(&headers, y)).collect::<Result<_, _>>()?;
    let names = yis
        .iter()
        .map(|i| headers.get(*i).map(str::to_owned).unwrap_or_else(|| format!("col {i}")))
        .collect();

    let mut series = vec![Vec::new(); yis.len()];
    for record in reader.records() {
        let r = record.map_err(|e| e.to_string())?;
        let Some(xv) = r.get(xi).and_then(|v| v.trim().parse::<f64>().ok()) else { continue };
        for (k, yi) in yis.iter().enumerate() {
            if let Some(yv) = r.get(*yi).and_then(|v| v.trim().parse::<f64>().ok()) {
                if xv.is_finite() && yv.is_finite() {
                    series[k].push((xv, yv));
                }
            }
        }
    }
    Ok((series, names))
}

/// Deterministic pseudo-noise, so a demo looks the same every run.
fn noise(i: u64) -> f64 {
    let mut x = i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    ((x >> 11) as f64 / (1u64 << 53) as f64) - 0.5
}

fn demo(name: &str) -> Result<(Vec<Vec<(f64, f64)>>, Vec<String>), String> {
    match name {
        // Two million samples with a transit a few hundred wide: the case decimation exists
        // for, and the reason a chart of this is not a polyline through the samples.
        "transit" => {
            let n = 2_000_000u64;
            let points = (0..n)
                .map(|i| {
                    let x = i as f64 / n as f64 * 400.0;
                    let phase = (x % 88.7) / 88.7;
                    let in_transit = (0.499..0.503).contains(&phase);
                    let y = 1.0 + noise(i) * 6e-5 - if in_transit { 1.02e-4 } else { 0.0 };
                    (x, y)
                })
                .collect();
            Ok((vec![points], vec!["relative flux".into()]))
        }
        // A swarm's flicker: stationary noise about a mean, with a knee.
        "flicker" => {
            let n = 400_000u64;
            let points = (0..n)
                .map(|i| {
                    let t = i as f64 / n as f64 * 240.0;
                    let mut v = 0.0;
                    for k in 1..14u64 {
                        let f = k as f64 * 0.37;
                        v += (t * f + noise(k) * 6.0).sin() / (1.0 + f * f * 0.7);
                    }
                    (t, 5.33e-6 * (1.0 + 0.35 * v * 1.6))
                })
                .collect();
            Ok((vec![points], vec!["deficit".into()]))
        }
        // Log-log, spanning decades.
        "periodogram" => {
            let n = 4_000u64;
            let points = (0..n)
                .map(|i| {
                    let f = 10f64.powf(-4.0 + 4.0 * i as f64 / n as f64);
                    let knee = 2.1e-2;
                    let p = 1.0 / (1.0 + (f / knee).powi(2)) * (1.0 + noise(i) * 0.8).abs().max(1e-3);
                    (f, p.max(1e-6))
                })
                .collect();
            Ok((vec![points], vec!["power".into()]))
        }
        "list" => Err("demos: transit, flicker, periodogram".into()),
        other => Err(format!("unknown demo {other:?}; try --demo list")),
    }
}

fn bounds(series: &[Vec<(f64, f64)>], x_scale: Scale, y_scale: Scale) -> ((f64, f64), (f64, f64)) {
    let mut x = (f64::INFINITY, f64::NEG_INFINITY);
    let mut y = (f64::INFINITY, f64::NEG_INFINITY);
    for s in series {
        for (a, b) in s {
            // A log axis cannot show a non-positive value, so it does not get a vote on the
            // range either.
            if matches!(x_scale, Scale::Log10) && *a <= 0.0 {
                continue;
            }
            if matches!(y_scale, Scale::Log10) && *b <= 0.0 {
                continue;
            }
            x = (x.0.min(*a), x.1.max(*a));
            y = (y.0.min(*b), y.1.max(*b));
        }
    }
    (x_scale.valid_range(x), y_scale.pad(y, 0.06))
}

const PALETTE: [Rgba; 4] = [
    Rgba::opaque(0.18, 0.55, 0.86),
    Rgba::opaque(0.92, 0.47, 0.20),
    Rgba::opaque(0.30, 0.70, 0.42),
    Rgba::opaque(0.78, 0.36, 0.68),
];

fn draw(o: &Options, series: &[Vec<(f64, f64)>], names: &[String]) -> Result<String, String> {
    let (xr, yr) = bounds(series, o.x_scale, o.y_scale);
    let metrics = Monospace::default();
    let (fg, bg) = if o.dark {
        (Rgba::opaque(0.86, 0.88, 0.92), Rgba::opaque(0.09, 0.10, 0.13))
    } else {
        (Rgba::opaque(0.15, 0.16, 0.20), Rgba::opaque(1.0, 1.0, 1.0))
    };

    let margin_left = 96.0;
    let margin_bottom = 62.0;
    let area = Rect {
        x: margin_left,
        y: 44.0,
        width: o.width as f32 - margin_left - 28.0,
        height: o.height as f32 - margin_bottom - 52.0,
    };

    let mut chart = Chart::new(
        area,
        Axis { scale: o.x_scale, range: xr, ticks: 8 },
        Axis { scale: o.y_scale, range: yr, ticks: 6 },
        &metrics,
    );
    chart.style = Style { axis: fg, text_size: 14.0, line_width: 1.0, ..Style::default() };

    let mut layers = vec![chart.frame()];
    let mut legend = Primitives::default();
    for (k, s) in series.iter().enumerate() {
        let colour = PALETTE[k % PALETTE.len()];
        // The envelope carries the extremes; at these densities it is a solid block, so it
        // goes in faint and the mean track goes over it.
        chart.style.series = Rgba(colour.0, colour.1, colour.2, 0.35);
        layers.push(chart.series(s));
        layers.push(chart.mean_track(s, colour));
        if series.len() > 1 {
            legend.labels.push(Label {
                at: Point::new(area.x + area.width - 8.0, area.y + 18.0 + k as f32 * 20.0),
                text: names.get(k).cloned().unwrap_or_default(),
                size: 14.0,
                anchor: Anchor::End,
                colour: PALETTE[k % PALETTE.len()],
            });
        }
    }

    let mut furniture = Primitives::default();
    if let Some(t) = &o.title {
        furniture.labels.push(Label {
            at: Point::new(o.width as f32 / 2.0, 30.0),
            text: t.clone(),
            size: 20.0,
            anchor: Anchor::Middle,
            colour: fg,
        });
    }
    if let Some(t) = &o.xlabel {
        furniture.labels.push(Label {
            at: Point::new(area.x + area.width / 2.0, o.height as f32 - 12.0),
            text: t.clone(),
            size: 15.0,
            anchor: Anchor::Middle,
            colour: fg,
        });
    }
    if let Some(t) = &o.ylabel {
        // No rotated text in the bitmap font, so the y label sits above the axis.
        furniture.labels.push(Label {
            at: Point::new(6.0, area.y - 14.0),
            text: t.clone(),
            size: 15.0,
            anchor: Anchor::Start,
            colour: fg,
        });
    }
    layers.push(legend);
    layers.push(furniture);

    let refs: Vec<&Primitives> = layers.iter().collect();
    let pixmap = raster::render(&refs, o.width, o.height, bg).ok_or("could not allocate the image")?;
    pixmap
        .save_png(&o.output)
        .map_err(|e| format!("{}: {e}", o.output))?;

    let total: usize = series.iter().map(Vec::len).sum();
    let mut message = format!("{} points -> {} ({}x{})", total, o.output, o.width, o.height);
    if o.svg {
        let path = format!("{}.svg", o.output.trim_end_matches(".png"));
        let doc = svg::render(&refs, o.width as f32, o.height as f32);
        std::fs::write(&path, doc).map_err(|e| format!("{path}: {e}"))?;
        message.push_str(&format!(" and {path}"));
    }
    Ok(message)
}
