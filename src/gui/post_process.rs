//! Demonstrates Bevy's built-in postprocessing features.
//!
//! Currently, this simply consists of chromatic aberration.

use bevy::{
    core_pipeline::post_process::ChromaticAberration, prelude::*,
};

/// The number of units per frame to add to or subtract from intensity when the
/// arrow keys are held.
const CHROMATIC_ABERRATION_INTENSITY_ADJUSTMENT_SPEED: f32 = 0.002;

/// The maximum supported chromatic aberration intensity level.
const MAX_CHROMATIC_ABERRATION_INTENSITY: f32 = 0.4;

/// The settings that the user can control.
#[derive(Resource)]
pub struct PostProcessSettings {
    /// The intensity of the chromatic aberration effect.
    chromatic_aberration_intensity: f32,
}

impl Default for PostProcessSettings {
    fn default() -> Self {
        Self {
            chromatic_aberration_intensity: 0.4,
        }
    }
}

/// Updates the [`ChromaticAberration`] settings per the [`PostProcessSettings`].
pub fn update_post_process_settings(
    mut chromatic_aberration: Query<&mut ChromaticAberration>,
    app_settings: Res<PostProcessSettings>,
) {
    let intensity = app_settings.chromatic_aberration_intensity;

    // Pick a reasonable maximum sample size for the intensity to avoid an
    // artifact whereby the individual samples appear instead of producing
    // smooth streaks of color.
    //
    // Don't take this formula too seriously; it hasn't been heavily tuned.
    let max_samples = ((intensity - 0.02) / (0.20 - 0.02) * 56.0 + 8.0)
        .clamp(8.0, 64.0)
        .round() as u32;

    for mut chromatic_aberration in &mut chromatic_aberration {
        chromatic_aberration.intensity = intensity;
        chromatic_aberration.max_samples = max_samples;
    }
}
