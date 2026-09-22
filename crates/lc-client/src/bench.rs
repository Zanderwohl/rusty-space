//! `--bench <frames>`: time the real pipeline, print percentiles, and quit.
//!
//! Three numbers, because no one of them says where a frame went:
//!
//! - **frame**, wall time between frames. macOS paces a windowed surface to the display even
//!   without vsync, so this comes in whole refresh intervals and a small saving cannot move it.
//! - **main**, the main world's CPU time, `First` to `Last`. Rendering is pipelined, so this is
//!   the CPU side of the critical path and is not quantized.
//! - **render**, per pass, from Bevy's render diagnostics: CPU time on Metal, and GPU time too
//!   where the backend has timestamp queries.

use bevy::diagnostic::DiagnosticsStore;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;

use crate::dev::DevEntry;

pub struct BenchPlugin;

impl Plugin for BenchPlugin {
    fn build(&self, app: &mut App) {
        // The entry is inserted before the client plugin is, so this is known at build time and
        // a run that is not a bench pays nothing for the diagnostics.
        if app.world().get_resource::<DevEntry>().is_none_or(|dev| dev.bench.is_none()) {
            return;
        }
        app.add_plugins(RenderDiagnosticsPlugin)
            .init_resource::<Bench>()
            .add_systems(Startup, unlock_present)
            .add_systems(First, start)
            .add_systems(Last, record);
    }
}

#[derive(Resource, Default)]
struct Bench {
    frames: u32,
    started: Option<Instant>,
    frame_ms: Vec<f64>,
    main_ms: Vec<f64>,
}

/// Take the display's refresh out of the frame time, as far as the platform allows.
fn unlock_present(mut window: Single<&mut Window>) {
    window.present_mode = bevy::window::PresentMode::AutoNoVsync;
}

fn start(mut bench: ResMut<Bench>) {
    bench.started = Some(Instant::now());
}

/// Real time rather than virtual, which Bevy clamps.
fn record(
    dev: Res<DevEntry>,
    time: Res<Time<Real>>,
    window: Single<&Window>,
    diagnostics: Res<DiagnosticsStore>,
    mut bench: ResMut<Bench>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(wanted) = dev.bench else { return };
    bench.frames += 1;
    if bench.frames <= dev.after_frames {
        return;
    }
    let main = bench.started.map_or(0.0, |at| at.elapsed().as_secs_f64() * 1e3);
    bench.main_ms.push(main);
    bench.frame_ms.push(time.delta_secs_f64() * 1e3);
    if bench.frame_ms.len() < wanted as usize {
        return;
    }

    // The window lands on whichever display it lands on, and one at twice the scale has four
    // times the pixels: two runs are only comparable at the same size.
    println!(
        "bench: {}x{}  {} frames",
        window.physical_width(),
        window.physical_height(),
        bench.frame_ms.len(),
    );
    println!("  frame   {}", summary(&mut bench.frame_ms));
    println!("  main    {}", summary(&mut bench.main_ms));
    let mut passes: Vec<(String, f64)> = diagnostics
        .iter()
        .filter(|d| d.path().as_str().starts_with("render/"))
        .filter_map(|d| Some((d.path().as_str().to_owned(), d.average()?)))
        .filter(|(_, ms)| *ms >= 0.01)
        .collect();
    passes.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (path, ms) in passes.iter().take(24) {
        println!("  {ms:7.3} ms  {path}");
    }
    exit.write(AppExit::Success);
}

fn summary(samples: &mut [f64]) -> String {
    samples.sort_by(f64::total_cmp);
    let at = |q: f64| samples[((samples.len() - 1) as f64 * q).round() as usize];
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    format!(
        "mean {mean:6.2} ms  p50 {:6.2}  p95 {:6.2}  p99 {:6.2}  max {:6.2}",
        at(0.5),
        at(0.95),
        at(0.99),
        at(1.0),
    )
}
