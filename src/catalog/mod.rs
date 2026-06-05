use bevy::prelude::*;
use csv::ReaderBuilder;

/// Numerical star data suitable for copying to a GPU buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StarGpuData {
    pub ra: f32,
    pub dec: f32,
    pub dist: f32,
    pub mag: f32,
    pub absmag: f32,
    pub _padding: [f32; 3],
}

/// A single star entry from the HYG catalog.
#[derive(Clone, Debug, Default)]
pub struct DistantStar {
    pub proper: String,
    pub spect: String,
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

            DistantStar {
                proper: r.get(proper_col).unwrap_or("").to_string(),
                spect: r.get(spect_col).unwrap_or("").to_string(),
                gpu: StarGpuData {
                    ra: f(ra_col),
                    dec: f(dec_col),
                    dist: f(dist_col),
                    mag: f(mag_col),
                    absmag: f(absmag_col),
                    _padding: [0.0; 3],
                },
            }
        })
        .collect()
}
