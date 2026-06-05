pub mod spectral;

use std::f32::consts::PI;
use bevy::prelude::*;
use csv::ReaderBuilder;
use spectral::SpectralType;

/// GPU-ready star data: pre-computed direction vector + apparent magnitude.
/// 16 bytes total, maps directly to a WGSL `vec4<f32>`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StarGpuData {
    /// Unit direction vector in Bevy Y-up space (pre-computed from RA/dec)
    pub dir: [f32; 3],
    /// Apparent magnitude as seen from Earth
    pub mag: f32,
}

/// A single star entry from the HYG catalog.
#[derive(Clone, Debug, Default)]
pub struct DistantStar {
    pub id: u32,
    pub proper: String,
    pub spect: String,
    /// Parsed MK spectral classification
    pub spectral: Option<SpectralType>,
    /// Right ascension in hours (0-24)
    pub ra: f32,
    /// Declination in degrees (-90 to +90)
    pub dec: f32,
    /// Distance in parsecs
    pub dist: f32,
    /// Absolute magnitude
    pub absmag: f32,
    /// Pre-computed GPU data
    pub gpu: StarGpuData,
}

#[derive(Resource)]
pub struct Catalogs {
    pub hyg_closest: Vec<DistantStar>,
    pub hyg_brightest: Vec<DistantStar>,
}

pub fn load_catalogs(mut commands: Commands) {
    let closest = load_csv("assets/catalogs/hygdata_v42_dist_sort.csv");
    let brightest = load_csv("assets/catalogs/hygdata_v42_mag_sort.csv");

    info!("Loaded {} closest stars, {} brightest stars", closest.len(), brightest.len());

    commands.insert_resource(Catalogs {
        hyg_closest: closest,
        hyg_brightest: brightest,
    });
}

fn load_csv(path: &str) -> Vec<DistantStar> {
    let mut reader = ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .unwrap_or_else(|e| panic!("Failed to open {path}: {e}"));

    let headers = reader.headers().unwrap().clone();

    let col = |name: &str| -> usize {
        headers.iter().position(|h| h == name)
            .unwrap_or_else(|| panic!("Column \"{name}\" not found in {path}"))
    };

    let id_col = col("id");
    let proper_col = col("proper");
    let ra_col = col("ra");
    let dec_col = col("dec");
    let dist_col = col("dist");
    let mag_col = col("mag");
    let absmag_col = col("absmag");
    let spect_col = col("spect");

    reader
        .records()
        .filter_map(|r| r.ok())
        .filter(|r| r.get(id_col) != Some("0"))
        .map(|r| {
            let f = |col: usize| -> f32 {
                r.get(col).unwrap_or("0").parse().unwrap_or(0.0)
            };
            let id: u32 = r.get(id_col).unwrap_or("0").parse().unwrap_or(0);

            let ra = f(ra_col);
            let dec = f(dec_col);
            let mag = f(mag_col);

            // Convert RA (hours) and Dec (degrees) to radians
            let ra_rad = ra * PI / 12.0;
            let dec_rad = dec * PI / 180.0;

            // Compute direction vector in Z-up astronomical coordinates
            let x = dec_rad.cos() * ra_rad.cos();
            let y = dec_rad.cos() * ra_rad.sin();
            let z = dec_rad.sin();

            // Swizzle to Bevy Y-up: (x, z, -y)
            let dir = [x, z, -y];

            let spect_str = r.get(spect_col).unwrap_or("").to_string();
            let spectral = SpectralType::parse(&spect_str);

            DistantStar {
                id,
                proper: r.get(proper_col).unwrap_or("").to_string(),
                spect: spect_str,
                spectral,
                ra,
                dec,
                dist: f(dist_col),
                absmag: f(absmag_col),
                gpu: StarGpuData { dir, mag },
            }
        })
        .collect()
}
