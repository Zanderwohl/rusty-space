//! Body appearance — the serializable data half. Meshes and materials are the app's job.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone)]
pub enum Appearance {
    #[default]
    Empty,
    DebugBall(DebugBall),
    Star(StarBall),
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct AppearanceColor {
    pub r: u16,
    pub g: u16,
    pub b: u16,
}

impl Appearance {
    pub fn radius(&self) -> f64 {
        match self {
            Appearance::Empty => 1.0,
            Appearance::DebugBall(DebugBall { radius, .. }) => *radius,
            Appearance::Star(StarBall { radius, ..}) => *radius,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DebugBall {
    pub radius: f64,
    pub color: AppearanceColor,
    /// Latitudes to pick out on the wireframe, in degrees. Empty for none.
    #[serde(default)]
    pub highlight_latitudes: Vec<f64>,
}

impl DebugBall {
    pub fn highlight_latitudes(&self) -> Vec<f64> {
        self.highlight_latitudes.clone()
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct StarBall {
    pub radius: f64,
    pub color: AppearanceColor,
    pub light: AppearanceColor,
    pub absolute_magnitude: f32,
}

impl StarBall {
    pub fn intensity(&self) -> f32 {
        // Absolute magnitude -> luminous flux (lumens), scaled off the Sun.
        const SUN_ABSOLUTE_MAGNITUDE: f64 = 4.83;
        const SUN_LUMINOUS_FLUX_LM: f64 = 3.5e28;
        let m = self.absolute_magnitude as f64;
        let luminosity_ratio = 10f64.powf(0.4 * (SUN_ABSOLUTE_MAGNITUDE - m));
        (SUN_LUMINOUS_FLUX_LM * luminosity_ratio) as f32
    }

    pub fn emissive_luminance(&self) -> f32 {
        // Nits (cd/m^2); solar photosphere ≈ 1.8e9 nits.
        const SUN_ABSOLUTE_MAGNITUDE: f64 = 4.83;
        const SUN_SURFACE_LUMINANCE_NITS: f64 = 1.83e9;
        let m = self.absolute_magnitude as f64;
        let luminosity_ratio = 10f64.powf(0.4 * (SUN_ABSOLUTE_MAGNITUDE - m));
        (SUN_SURFACE_LUMINANCE_NITS * luminosity_ratio) as f32
    }
    
}
