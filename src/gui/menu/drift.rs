//! Slow camera drift while the app sits in the main menu, so the starfield
//! turns behind the menu.
//!
//! The starfield shader is translation-invariant and ignores the mesh's
//! transform - only camera orientation moves it - so the drift is applied to
//! the camera itself. The camera's resting rotation is put back when the menu
//! is left, so entering the planetarium starts from the usual view.

use std::time::{SystemTime, UNIX_EPOCH};

use bevy::prelude::*;

use crate::camera::PlanetariumCamera;
use crate::gui::app::AppState;

/// One radian a minute.
const DRIFT_RATE: f32 = 1.0 / 60.0;

#[derive(Resource)]
struct MenuDrift {
    axis: Dir3,
    resting: Quat,
    /// Total angle turned so far. The rotation is rebuilt from this each frame
    /// rather than composed onto itself, so f32 error cannot accumulate over
    /// the minutes a menu might be left open.
    angle: f32,
}

pub struct MenuDriftPlugin;

impl Plugin for MenuDriftPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::MainMenu), start_drift)
            .add_systems(OnExit(AppState::MainMenu), stop_drift)
            .add_systems(Update, drift_camera.run_if(in_state(AppState::MainMenu)));
    }
}

fn start_drift(mut commands: Commands, camera: Query<&Transform, With<PlanetariumCamera>>) {
    let Ok(transform) = camera.single() else {
        return;
    };

    commands.insert_resource(MenuDrift {
        axis: random_axis(),
        resting: transform.rotation,
        angle: 0.0,
    });
}

fn drift_camera(
    drift: Option<ResMut<MenuDrift>>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<PlanetariumCamera>>,
) {
    let Some(mut drift) = drift else {
        return;
    };
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };

    drift.angle += DRIFT_RATE * time.delta_secs();
    transform.rotation = Quat::from_axis_angle(drift.axis.into(), drift.angle) * drift.resting;
}

fn stop_drift(
    mut commands: Commands,
    drift: Option<Res<MenuDrift>>,
    mut camera: Query<&mut Transform, With<PlanetariumCamera>>,
) {
    if let Some(drift) = drift {
        if let Ok(mut transform) = camera.single_mut() {
            transform.rotation = drift.resting;
        }
    }
    commands.remove_resource::<MenuDrift>();
}

/// A uniformly distributed direction, seeded from the wall clock. The drift
/// only has to look arbitrary, so this avoids pulling in an rng dependency.
fn random_axis() -> Dir3 {
    let seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);

    let z = unit_float(splitmix64(seed)) * 2.0 - 1.0;
    let theta = unit_float(splitmix64(seed.wrapping_add(0x9e37_79b9_7f4a_7c15))) * std::f32::consts::TAU;
    let r = (1.0 - z * z).max(0.0).sqrt();

    Dir3::new(Vec3::new(r * theta.cos(), r * theta.sin(), z)).unwrap_or(Dir3::Y)
}

fn splitmix64(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The top 24 bits of `bits` as a float in `[0, 1)`.
fn unit_float(bits: u64) -> f32 {
    ((bits >> 40) as f32) / ((1u64 << 24) as f32)
}
