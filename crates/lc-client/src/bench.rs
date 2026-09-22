//! `--bench <frames>`: time the real pipeline, print percentiles, and quit.
//!
//! - **frame**: wall time between frames. macOS paces a window to the display even without
//!   vsync, so this comes in whole refresh intervals.
//! - **main**: the main world's CPU time, `First` to `Last`. Not quantized.
//! - **render**: per pass, from Bevy's render diagnostics; CPU only on Metal. For GPU time use
//!   `tools/gpu_passes.py`.

use bevy::diagnostic::DiagnosticsStore;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::diagnostic::RenderDiagnosticsPlugin;

use crate::dev::DevEntry;

pub struct BenchPlugin;

impl Plugin for BenchPlugin {
    fn build(&self, app: &mut App) {
        // The entry is inserted before this plugin, so a run that is not a bench adds nothing.
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

    // A window on a Retina display has four times the pixels; compare runs at the same size.
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
