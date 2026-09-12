pub mod spectral;
pub mod spectral_color;

use std::f64::consts::PI as PI_F64;
use bevy::prelude::*;
use csv::ReaderBuilder;
use em_foundations::reference_frame::equatorial;
use spectral::SpectralType;

use crate::presentation::render_space::ToRender;

/// GPU-ready star data: pre-computed direction vector + apparent magnitude.
/// 16 bytes total, maps directly to a WGSL `vec4<f32>`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct StarGpuData {
    /// Unit direction vector in Bevy Y-up render space, rotated out of the catalogue's
    /// equatorial frame into the ecliptic first, so it agrees with the simulation.
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

            // RA arrives in hours and dec in degrees; the catalogue frame is equatorial
            // J2000, so the direction has to be tilted into the ecliptic before it can
            // share a sky with the simulation. Then the usual sim -> render conversion.
            let ra_rad = (ra as f64) * PI_F64 / 12.0;
            let dec_rad = (dec as f64).to_radians();

            let ecliptic = equatorial::ecliptic_direction(ra_rad, dec_rad);
            let dir = ecliptic.to_render().to_array();

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presentation::render_space::ToSim;
    use bevy::math::DVec3;

    /// Ecliptic latitude of a loaded star, in degrees: undo the render swizzle, then read
    /// the angle out of the ecliptic plane.
    fn ecliptic_latitude(star: &DistantStar) -> f64 {
        let render = DVec3::new(star.gpu.dir[0] as f64, star.gpu.dir[1] as f64, star.gpu.dir[2] as f64);
        render.to_sim().normalize().z.asin().to_degrees()
    }

    /// The catalogue is equatorial and the simulation is ecliptic, so the import has to
    /// rotate by the obliquity. These are published ecliptic latitudes; without the
    /// rotation each star lands at its *declination* instead, which for Sirius is 23° out
    /// and for Polaris is 23° out in the other direction.
    #[test]
    fn stars_land_at_their_published_ecliptic_latitude() {
        let stars = load_csv("assets/catalogs/hygdata_v42_mag_sort.csv");
        let expected = [
            ("Sirius", -39.605),
            ("Vega", 61.733),
            ("Aldebaran", -5.467),
            ("Regulus", 0.465),
            ("Spica", -2.055),
            ("Polaris", 66.099),
        ];
        for (name, latitude) in expected {
            let star = stars
                .iter()
                .find(|s| s.proper == name)
                .unwrap_or_else(|| panic!("{name} is not in the brightest-stars catalogue"));
            let found = ecliptic_latitude(star);
            assert!(
                (found - latitude).abs() < 0.01,
                "{name}: ecliptic latitude {found:.3}°, expected {latitude:.3}° \
                 (declination is {:.3}° — if that is what came out, the obliquity \
                 rotation was skipped)",
                star.dec,
            );
        }
    }

    /// Every loaded direction must be exactly what the frame conversion and the render
    /// conversion produce together, and must stay a unit vector.
    #[test]
    fn the_import_agrees_with_the_render_conversion() {
        use crate::presentation::render_space::sim_to_render;
        let stars = load_csv("assets/catalogs/hygdata_v42_dist_sort.csv");
        for star in stars.iter().take(50) {
            let sim = equatorial::ecliptic_direction(
                (star.ra as f64) * PI_F64 / 12.0,
                (star.dec as f64).to_radians(),
            );
            let expected = sim_to_render(sim);
            let dir = DVec3::new(star.gpu.dir[0] as f64, star.gpu.dir[1] as f64, star.gpu.dir[2] as f64);
            assert!((dir - expected).length() < 1e-6, "{}: {dir:?} vs {expected:?}", star.proper);
            assert!((dir.length() - 1.0).abs() < 1e-6, "{} is not a unit vector", star.proper);
        }
    }
}
