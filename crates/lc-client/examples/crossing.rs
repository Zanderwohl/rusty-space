//! Render the sky at intervals through a crossing, without opening a window.
//!
//! What it is for: aberration and Doppler beaming are changes to *where* and *how bright*
//! every star is, and neither is visible in a single frame. Six frames of one crossing put
//! the two effects side by side.
//!
//! ```text
//! cargo run -p lc-client --example crossing -- crossing.png assets/catalogs/hygdata_v42.csv
//! ```

use em_plot::chart::{Axis, Chart, Rect, Style};
use em_plot::primitives::{Anchor, Label, Point, Primitives, Rgba};
use em_plot::{raster, scale::Scale};
use lc_client::action::refresh_exposure;
use lc_client::session::{POINT_STOPS, Session, SkyStar, TIME_RATE, point_color};
use lc_client::ui::UiState;
use lc_world::sky::{AuthoredStars, StarProvider};

const WIDTH: u32 = 1500;
const HEIGHT: u32 = 1000;
const BG: Rgba = Rgba::opaque(0.07, 0.08, 0.11);
const FG: Rgba = Rgba::opaque(0.82, 0.85, 0.90);
const FRAMES: usize = 6;

fn main() {
    let mut args = std::env::args().skip(1);
    let output = args.next().unwrap_or_else(|| "crossing.png".into());
    let catalogue = args.next();

    let provider: Box<dyn StarProvider> = match &catalogue {
        Some(p) => match lc_world::sky::hyg::HygProvider::load(p) {
            Ok(h) => Box::new(h),
            Err(e) => {
                eprintln!("crossing: {e}");
                std::process::exit(1);
            }
        },
        None => Box::new(AuthoredStars::sample()),
    };

    let ui = UiState::default();
    let mut session = Session::new(provider.as_ref(), 3000);

    // The first star that is actually interstellar: the catalogue puts the Sun about an
    // astronomical unit out and flying to it is not a crossing.
    let Some(target) = session
        .stars
        .iter()
        .find(|s| s.position_ly.length() > 1.0)
        .map(|s| s.id)
    else {
        eprintln!("crossing: nothing to fly to");
        std::process::exit(1);
    };
    // Charted, because an example that flies somewhere needs something to call it.
    session.issue_charts(lc_client::session::CHARTED_LY);
    let name = session.name_of(target);
    let distance = session.distance_to(session.star(target).unwrap());

    session.fly_to(target);
    let cruise = session.cruise().clone().expect("a crossing");
    println!(
        "{name}: {distance:.2} ly, {:.2} coordinate years, {:.2} aboard, peak {:.4}c",
        cruise.duration_s() / 31_557_600.0,
        cruise.proper_duration_s() / 31_557_600.0,
        cruise.peak_beta()
    );

    let step_real_s = cruise.duration_s() / TIME_RATE / (FRAMES - 1) as f64;
    let mut layers: Vec<Primitives> = Vec::new();
    let metrics = raster::BitmapMetrics;

    for frame in 0..FRAMES {
        if frame > 0 {
            session.advance(step_real_s);
        }
        refresh_exposure(&ui, &mut session);
        let sky = session.sky();
        layers.extend(panel(frame, &session, &sky, target, &metrics));
    }

    let mut title = Primitives::default();
    title.labels.push(Label {
        at: Point::new(24.0, 24.0),
        text: format!(
            "crossing to {name} — {distance:.2} ly at {:.0} g",
            session.ship.motion.drive.accel_g
        ),
        size: 17.0,
        anchor: Anchor::Start,
        color: FG,
    });
    layers.push(title);

    let refs: Vec<&Primitives> = layers.iter().collect();
    let Some(pixmap) = raster::render(&refs, WIDTH, HEIGHT, BG) else {
        eprintln!("crossing: could not allocate the image");
        std::process::exit(1);
    };
    if let Err(e) = pixmap.save_png(&output) {
        eprintln!("crossing: {output}: {e}");
        std::process::exit(1);
    }
    println!("wrote {output}");
}

/// One all-sky panel: longitude and latitude of every star, as the ship sees it.
fn panel(
    frame: usize,
    session: &Session,
    sky: &[SkyStar],
    target: lc_world::sky::StarId,
    metrics: &raster::BitmapMetrics,
) -> Vec<Primitives> {
    let (col, row) = (frame % 3, frame / 3);
    let area = Rect {
        x: 70.0 + col as f32 * 480.0,
        y: 110.0 + row as f32 * 450.0,
        width: 400.0,
        height: 290.0,
    };
    let mut chart = Chart::new(
        area,
        Axis {
            scale: Scale::Linear,
            range: (-180.0, 180.0),
            ticks: 5,
        },
        Axis {
            scale: Scale::Linear,
            range: (-90.0, 90.0),
            ticks: 3,
        },
        metrics,
    );
    chart.style = Style {
        axis: FG,
        text_size: 11.0,
        ..Style::default()
    };

    let mut out = vec![chart.frame()];
    // Bucketed by brightness so the bright ones are drawn larger, the way a star chart does.
    for bucket in 0..4 {
        let (lo, hi) = (bucket as f32 / 4.0, (bucket + 1) as f32 / 4.0);
        let members: Vec<&SkyStar> = sky
            .iter()
            .filter(|s| {
                let v = s.shaded.point_brightness(POINT_STOPS);
                v > lo && (v <= hi || bucket == 3)
            })
            .collect();
        if members.is_empty() {
            continue;
        }
        let points: Vec<(f64, f64)> = members
            .iter()
            .map(|s| {
                let d = s.apparent_dir;
                (
                    d.y.atan2(d.x).to_degrees(),
                    d.z.clamp(-1.0, 1.0).asin().to_degrees(),
                )
            })
            .collect();
        let colors: Vec<Rgba> = members
            .iter()
            .map(|s| {
                let c = point_color(&s.shaded);
                Rgba(c.x, c.y, c.z, 0.9)
            })
            .collect();
        out.push(chart.scatter_with(&points, 0.8 + bucket as f32 * 1.2, |i, _| colors[i]));
    }

    let beta = session.ship.motion.beta.length();
    let years = session.coordinate_time_s() / 31_557_600.0;
    let aboard = session.ship.motion.clock_s / 31_557_600.0;
    let left = session
        .star(target)
        .map(|s| session.distance_to(s))
        .unwrap_or(0.0);

    // Two lines, not one: at this panel width the single-line form runs into the next panel.
    let mut text = Primitives::default();
    for (k, line) in [
        format!("T+{years:.2} y   ship {aboard:.2} y"),
        format!("{beta:.4}c   {left:.2} ly to go"),
    ]
    .into_iter()
    .enumerate()
    {
        text.labels.push(Label {
            at: Point::new(area.x, area.y - 44.0 + k as f32 * 17.0),
            text: line,
            size: 12.0,
            anchor: Anchor::Start,
            color: FG,
        });
    }
    out.push(text);
    out
}
