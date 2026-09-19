//! The F3 overlay.
//!
//! This replaces `iyes_perf_ui`, which never moved past Bevy 0.17. The rows are the ones
//! that crate's `PerfUiFramerateEntries`, `PerfUiWindowEntries` and `PerfUiFixedTimeEntries`
//! bundles drew, so the overlay reads the same as it always did.

use bevy::diagnostic::{DiagnosticPath, DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::input::ButtonInput;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::sim::despawn_recursive_entities_with;

/// Fraction of the frame history the "low" row averages over — the usual "10% low".
const LOW_FRACTION: f64 = 0.1;

/// Frames of history to keep. Every averaged row is only as good as this window.
const HISTORY: usize = 100;

pub struct DebugPlugin;

#[derive(Component)]
struct DebugUI;

/// A value cell, tagged with what it displays. One system fills all of them.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Metric {
    Fps,
    FpsAvg,
    FpsLow,
    FpsWorst,
    FrameTime,
    FrameTimeWorst,
    CursorPosition,
    Resolution,
    ScaleFactor,
    WindowMode,
    PresentMode,
    FixedTimeStep,
    FixedOverstep,
}

impl Metric {
    fn label(self) -> &'static str {
        match self {
            Metric::Fps => "FPS",
            Metric::FpsAvg => "FPS (avg)",
            Metric::FpsLow => "FPS (low)",
            Metric::FpsWorst => "FPS (worst)",
            Metric::FrameTime => "Frame Time",
            Metric::FrameTimeWorst => "Frame Time (max)",
            Metric::CursorPosition => "Cursor Position",
            Metric::Resolution => "Resolution",
            Metric::ScaleFactor => "Scale Factor",
            Metric::WindowMode => "Window Mode",
            Metric::PresentMode => "Present Mode",
            Metric::FixedTimeStep => "Fixed Time Step",
            Metric::FixedOverstep => "Fixed Overstep",
        }
    }
}

const ROWS: [Metric; 13] = [
    Metric::Fps,
    Metric::FpsAvg,
    Metric::FpsLow,
    Metric::FpsWorst,
    Metric::FrameTime,
    Metric::FrameTimeWorst,
    Metric::CursorPosition,
    Metric::Resolution,
    Metric::ScaleFactor,
    Metric::WindowMode,
    Metric::PresentMode,
    Metric::FixedTimeStep,
    Metric::FixedOverstep,
];

const FONT_SIZE: f32 = 12.0;
const VALUE_COLUMN: f32 = 128.0;
const LABEL_COLOR: Color = Color::WHITE;
const VALUE_COLOR: Color = Color::srgb(0.65, 0.65, 0.65);

impl Plugin for DebugPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin {
            max_history_length: HISTORY,
            smoothing_factor: 2.0 / (HISTORY as f64 + 1.0),
        })
        .init_state::<DebugState>()
        .add_systems(OnEnter(DebugState::Off), despawn_recursive_entities_with::<DebugUI>)
        .add_systems(OnEnter(DebugState::AllPerf), add_all_perf)
        .add_systems(Update, toggle_perf)
        .add_systems(Update, update_perf.run_if(in_state(DebugState::AllPerf)));
    }
}

fn add_all_perf(mut commands: Commands) {
    let root = commands
        .spawn((
            DebugUI,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(16.0),
                right: Val::Px(16.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                row_gap: Val::Px(2.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
            // Above everything, including the menu overlays at `em_ui::widgets::OVERLAY_Z`.
            GlobalZIndex(i32::MAX),
        ))
        .id();

    for metric in ROWS {
        let row = commands
            .spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .id();

        let label = commands
            .spawn((
                Text::new(metric.label()),
                TextFont { font_size: FontSize::Px(FONT_SIZE), ..default() },
                TextColor(LABEL_COLOR),
            ))
            .id();

        let value = commands
            .spawn((
                Node {
                    width: Val::Px(VALUE_COLUMN),
                    justify_content: JustifyContent::FlexEnd,
                    ..default()
                },
                children![(
                    Text::new("—"),
                    TextFont { font_size: FontSize::Px(FONT_SIZE), ..default() },
                    TextColor(VALUE_COLOR),
                    metric,
                )],
            ))
            .id();

        commands.entity(row).add_children(&[label, value]);
        commands.entity(root).add_child(row);
    }
}

fn update_perf(
    diagnostics: Res<DiagnosticsStore>,
    window: Query<&Window, With<PrimaryWindow>>,
    fixed: Res<Time<Fixed>>,
    mut cells: Query<(&Metric, &mut Text, &mut TextColor)>,
) {
    let window = window.single().ok();

    for (metric, mut text, mut color) in &mut cells {
        let (value, tint) = match metric {
            Metric::Fps => {
                let fps = diagnostics
                    .get(&FrameTimeDiagnosticsPlugin::FPS)
                    .and_then(|d| d.smoothed());
                (
                    fps.map(|v| format!("{v:.2}")),
                    fps.map(|v| fps_color(v as f32)),
                )
            }
            Metric::FpsAvg => (
                diagnostics
                    .get(&FrameTimeDiagnosticsPlugin::FPS)
                    .and_then(|d| d.average())
                    .map(|v| format!("{v:.2}")),
                None,
            ),
            Metric::FpsLow => (
                low_percentile(&diagnostics, &FrameTimeDiagnosticsPlugin::FPS)
                    .map(|v| format!("{v:.2}")),
                None,
            ),
            Metric::FpsWorst => (
                extremum(&diagnostics, &FrameTimeDiagnosticsPlugin::FPS, f64::min)
                    .map(|v| format!("{v:.2}")),
                None,
            ),
            Metric::FrameTime => (
                diagnostics
                    .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
                    .and_then(|d| d.smoothed())
                    .map(|v| format!("{v:.3} ms")),
                None,
            ),
            Metric::FrameTimeWorst => (
                extremum(&diagnostics, &FrameTimeDiagnosticsPlugin::FRAME_TIME, f64::max)
                    .map(|v| format!("{v:.3} ms")),
                None,
            ),
            Metric::CursorPosition => (
                window
                    .and_then(|w| w.cursor_position())
                    .map(|p| format!("{:.0}, {:.0}", p.x, p.y)),
                None,
            ),
            Metric::Resolution => (
                window.map(|w| {
                    format!(
                        "{}x{}",
                        w.resolution.physical_width(),
                        w.resolution.physical_height()
                    )
                }),
                None,
            ),
            Metric::ScaleFactor => (
                window.map(|w| format!("{:.2}", w.resolution.scale_factor())),
                None,
            ),
            Metric::WindowMode => (window.map(|w| format!("{:?}", w.mode)), None),
            Metric::PresentMode => (window.map(|w| format!("{:?}", w.present_mode)), None),
            Metric::FixedTimeStep => {
                let hz = 1.0 / fixed.timestep().as_secs_f64();
                (Some(format!("{hz:.2} Hz")), None)
            }
            Metric::FixedOverstep => (
                Some(format!("{:.3} ms", fixed.overstep().as_secs_f64() * 1000.0)),
                None,
            ),
        };

        let next = value.unwrap_or_else(|| "—".to_string());
        if text.as_str() != next {
            text.0 = next;
        }
        let next_color = tint.unwrap_or(VALUE_COLOR);
        if color.0 != next_color {
            color.0 = next_color;
        }
    }
}

/// Mean of the worst [`LOW_FRACTION`] of the history — the "10% low" a frame-pacing graph
/// shows, which catches stutter that the plain average hides.
fn low_percentile(diagnostics: &DiagnosticsStore, path: &DiagnosticPath) -> Option<f64> {
    let diagnostic = diagnostics.get(path)?;
    let mut values: Vec<f64> = diagnostic.values().copied().collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let take = ((values.len() as f64 * LOW_FRACTION).round() as usize).max(1);
    Some(values[..take].iter().sum::<f64>() / take as f64)
}

fn extremum(
    diagnostics: &DiagnosticsStore,
    path: &DiagnosticPath,
    pick: fn(f64, f64) -> f64,
) -> Option<f64> {
    let diagnostic = diagnostics.get(path)?;
    diagnostic.values().copied().reduce(pick)
}

/// Red below 30, yellow at 60, green from 120 up — the thresholds the old overlay used.
fn fps_color(fps: f32) -> Color {
    const BAD: Color = Color::srgb(0.9, 0.25, 0.25);
    const OK: Color = Color::srgb(0.9, 0.9, 0.3);
    const GOOD: Color = Color::srgb(0.35, 0.9, 0.4);

    if fps < 30.0 {
        BAD
    } else if fps < 60.0 {
        mix(BAD, OK, (fps - 30.0) / 30.0)
    } else if fps < 120.0 {
        mix(OK, GOOD, (fps - 60.0) / 60.0)
    } else {
        GOOD
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let a = a.to_srgba();
    let b = b.to_srgba();
    Color::srgb(
        a.red + (b.red - a.red) * t,
        a.green + (b.green - a.green) * t,
        a.blue + (b.blue - a.blue) * t,
    )
}

fn toggle_perf(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<State<DebugState>>,
    mut next_state: ResMut<NextState<DebugState>>,
) {
    if keyboard.just_pressed(KeyCode::F3) {
        match state.get() {
            DebugState::Off => {
                next_state.set(DebugState::AllPerf);
            },
            DebugState::AllPerf => {
                next_state.set(DebugState::Off);
            },
        }
    }
}

#[derive(States, Debug, Clone, PartialEq, Eq, Hash, Default)]
enum DebugState {
    #[default]
    Off,
    AllPerf,
}
