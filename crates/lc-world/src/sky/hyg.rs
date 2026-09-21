//! The HYG catalogue, behind the provider interface.
//!
//! Validation data, not the world. Its own numbering reaches [`Provenance`] and stops there.

use std::path::Path;

use em_foundations::reference_frame::equatorial;
use glam::DVec3;

use super::record::{StarRecord, assemble_all};
use super::{CatalogueStar, StarProvider};

const PARSEC_LY: f64 = 3.261_563_777;
/// One parsec per year, in m/s. HYG's velocity unit.
const PC_PER_YEAR_MS: f64 = 3.085_677_581e16 / 3.155_76e7;
/// HYG stores this distance when the parallax is useless.
const UNKNOWN_DISTANCE_PC: f64 = 100_000.0;

pub const SOURCE: &str = "hyg-v42";

#[derive(Debug)]
pub enum HygError {
    Io(std::io::Error),
    Csv(csv::Error),
    MissingColumn(&'static str),
}

impl std::fmt::Display for HygError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Csv(e) => write!(f, "{e}"),
            Self::MissingColumn(c) => write!(f, "HYG is missing the {c} column"),
        }
    }
}
impl std::error::Error for HygError {}

pub struct HygProvider {
    stars: Vec<CatalogueStar>,
    /// Rows rejected for unusable data, so the loss is visible rather than silent.
    pub skipped: usize,
}

impl HygProvider {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, HygError> {
        let records = Self::read_records(path)?;
        let (stars, skipped) = assemble_all(SOURCE, &records);
        Ok(Self { stars, skipped })
    }

    /// The catalogue as records, before anything is derived from them.
    ///
    /// This is what the packer writes into a sky chunk, so the chunk and this importer are
    /// the same data taking two routes to the same `assemble`.
    pub fn read_records(path: impl AsRef<Path>) -> Result<Vec<StarRecord>, HygError> {
        let mut reader = csv::Reader::from_path(path).map_err(HygError::Csv)?;
        let headers = reader.headers().map_err(HygError::Csv)?.clone();
        let col = |name: &'static str| {
            headers
                .iter()
                .position(|h| h == name)
                .ok_or(HygError::MissingColumn(name))
        };
        let (c_id, c_dist, c_ci, c_lum) = (col("id")?, col("dist")?, col("ci")?, col("lum")?);
        let (c_x, c_y, c_z) = (col("x")?, col("y")?, col("z")?);
        let (c_vx, c_vy, c_vz) = (col("vx")?, col("vy")?, col("vz")?);
        let (c_comp, c_primary) = (col("comp")?, col("comp_primary")?);
        let c_proper = col("proper")?;

        let mut records = Vec::with_capacity(120_000);
        for record in reader.records() {
            let r = record.map_err(HygError::Csv)?;
            let num = |i: usize| r.get(i).and_then(|s| s.trim().parse::<f64>().ok());

            let (Some(key), Some(dist), Some(ci), Some(lum)) =
                (num(c_id), num(c_dist), num(c_ci), num(c_lum))
            else {
                continue;
            };
            // Rows with no usable parallax carry a sentinel distance and a luminosity derived
            // from it, which is meaningless. Zero is legitimate: it is the Sun. This check is
            // HYG's own and stays here; everything that applies to any catalogue is in
            // `StarRecord::assemble`.
            if dist < 0.0 || dist >= UNKNOWN_DISTANCE_PC {
                continue;
            }

            let equatorial_pc = DVec3::new(
                num(c_x).unwrap_or(0.0),
                num(c_y).unwrap_or(0.0),
                num(c_z).unwrap_or(0.0),
            );
            let velocity_eq = DVec3::new(
                num(c_vx).unwrap_or(0.0),
                num(c_vy).unwrap_or(0.0),
                num(c_vz).unwrap_or(0.0),
            ) * PC_PER_YEAR_MS;

            records.push(StarRecord {
                key: key as u64,
                name: r.get(c_proper).filter(|s| !s.is_empty()).map(str::to_owned),
                position_ly: equatorial::to_ecliptic(equatorial_pc) * PARSEC_LY,
                velocity: equatorial::to_ecliptic(velocity_eq),
                color_index: ci,
                luminosity_solar: lum,
                component_index: num(c_comp).unwrap_or(1.0) as u8,
                group: num(c_primary).map(|p| p as u64),
            });
        }
        // Grouping is resolved in `assemble_all`, over the stars that survive derivation.
        Ok(records)
    }
}

impl StarProvider for HygProvider {
    fn name(&self) -> &str {
        SOURCE
    }
    fn stars(&self) -> &[CatalogueStar] {
        &self.stars
    }
}

#[cfg(test)]
mod tests {
    use em_spectra::stellar;

    use super::*;
    use crate::sky::{CatalogueStar, StarId};

    fn catalogue() -> Option<HygProvider> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/catalogs/hygdata_v42.csv"
        );
        HygProvider::load(path).ok()
    }

    #[test]
    fn the_whole_catalogue_loads_with_synthetic_ids() {
        let Some(p) = catalogue() else {
            eprintln!("HYG not present; skipping");
            return;
        };
        assert!(p.len() > 100_000, "only {} stars loaded", p.len());
        assert!(p.skipped < p.len() / 3, "{} rows rejected", p.skipped);

        // No catalogue number is an identity, and ids are unique.
        let mut ids: Vec<u64> = p.stars().iter().map(|s| s.id.get()).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before, "synthetic ids collided");
        for s in p.stars().iter().take(5000) {
            assert_ne!(
                s.id.get(),
                s.provenance.key,
                "an id must not be the HYG number"
            );
            assert_eq!(s.id, StarId::synthesise(SOURCE, s.provenance.key));
        }
    }

    #[test]
    fn the_sun_comes_out_solar() {
        let Some(p) = catalogue() else { return };
        let sol = p
            .stars()
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))
            .expect("Sol");
        assert!((sol.luminosity_solar - 1.0).abs() < 0.01);
        assert!(
            (sol.star.teff_k - 5772.0).abs() < 30.0,
            "Teff {}",
            sol.star.teff_k
        );
        assert!((sol.star.radius_m / stellar::SOLAR_RADIUS - 1.0).abs() < 0.05);
        assert!((sol.mass_solar - 1.0).abs() < 0.02);
    }

    #[test]
    fn positions_are_ecliptic_and_in_light_years() {
        let Some(p) = catalogue() else { return };
        // Sirius: 8.6 ly away, and well off the ecliptic plane.
        let sirius = p
            .stars()
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sirius"))
            .expect("Sirius");
        let d = sirius.position_ly.length();
        assert!((d - 8.6).abs() < 0.2, "Sirius is {d} ly away");
        // Rotating into the ecliptic must have moved it: its equatorial and ecliptic
        // latitudes differ by tens of degrees.
        assert!(
            sirius.position_ly.z.abs() / d > 0.4,
            "Sirius should sit well south of the ecliptic"
        );
    }

    #[test]
    fn multiples_are_grouped_and_singles_are_not() {
        let Some(p) = catalogue() else { return };
        let grouped: Vec<&CatalogueStar> = p
            .stars()
            .iter()
            .filter(|s| s.component.group.is_some())
            .collect();
        assert!(
            grouped.len() > 100,
            "only {} components are in a multiple",
            grouped.len()
        );
        assert!(
            grouped.len() < p.len() / 10,
            "multiples should be a minority"
        );

        // Every grouped star shares its group with at least one other.
        let mut counts: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();
        for s in &grouped {
            *counts.entry(s.component.group.unwrap()).or_default() += 1;
        }
        assert!(
            counts.values().all(|c| *c >= 2),
            "a group of one is not a multiple"
        );
        assert!(
            grouped.iter().any(|s| !s.component.is_primary()),
            "no secondaries found"
        );
    }

    #[test]
    fn metallicity_spans_a_plausible_range() {
        let Some(p) = catalogue() else { return };
        let feh: Vec<f64> = p.stars().iter().map(|s| s.metallicity).collect();
        let mean = feh.iter().sum::<f64>() / feh.len() as f64;
        assert!((-0.8..=0.1).contains(&mean), "mean [Fe/H] is {mean}");
        assert!(feh.iter().any(|f| *f < -1.0), "no metal-poor stars found");
        assert!(feh.iter().any(|f| *f > -0.2), "no metal-rich stars found");
    }
}
