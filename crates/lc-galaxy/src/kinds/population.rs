use serde::{Deserialize, Serialize};

use crate::compiled::{Age, Compiled, Dispersion, Feh, Imf, Population};
use crate::feature::{FeatureId, FeatureKind, Problem};

/// A named set of stellar statistics that shapes contribute density to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PopulationKind {
    pub frame: FeatureId,
    pub imf: Imf,
    pub age: Age,
    pub feh: Feh,
    pub dispersion: Dispersion,
}

/// The universe is about 13.8 Gyr old; no star is older.
pub const MAX_AGE_GYR: f64 = 13.8;

impl FeatureKind for PopulationKind {
    const KIND: &'static str = "population";
    const VERSION: u32 = 1;
    const FIELDS: &'static [&'static str] = &["frame", "imf", "age", "feh", "dispersion"];

    fn new() -> Self {
        Self {
            frame: FeatureId(0),
            imf: Imf::Kroupa,
            age: Age::Uniform { min_gyr: 0.0, max_gyr: 10.0 },
            feh: Feh { zero_age: 0.1, per_gyr: -0.05, per_kly: -0.018, scatter: 0.15 },
            dispersion: Dispersion { zero_age_km_s: 10.0, exponent: 0.5 },
        }
    }

    fn depends_on(&self) -> Vec<FeatureId> {
        vec![self.frame]
    }

    fn summary(&self) -> String {
        let age = match self.age {
            Age::Uniform { min_gyr, max_gyr } => format!("{min_gyr}–{max_gyr} Gyr"),
            Age::Burst { gyr, spread_gyr } => format!("{gyr} ± {spread_gyr} Gyr"),
        };
        format!("{:?}, {age}", self.imf)
    }

    fn evaluate(&self, id: FeatureId, out: &mut Compiled) -> Result<(), Problem> {
        super::frame(out, self.frame)?;
        match self.age {
            Age::Uniform { min_gyr, max_gyr } if !(0.0 <= min_gyr && min_gyr <= max_gyr && max_gyr <= MAX_AGE_GYR) => {
                return Err(Problem::Invalid(format!("ages must satisfy 0 ≤ min ≤ max ≤ {MAX_AGE_GYR} Gyr")));
            }
            Age::Burst { gyr, spread_gyr } if !(0.0..=MAX_AGE_GYR).contains(&gyr) || spread_gyr < 0.0 => {
                return Err(Problem::Invalid(format!("a burst must be within 0–{MAX_AGE_GYR} Gyr, with a spread ≥ 0")));
            }
            _ => {}
        }
        if self.feh.scatter < 0.0 || self.dispersion.zero_age_km_s < 0.0 {
            return Err(Problem::Invalid("scatter and dispersion cannot be negative".into()));
        }
        out.populations.insert(
            id,
            Population {
                frame: self.frame,
                imf: self.imf.clone(),
                age: self.age.clone(),
                feh: self.feh.clone(),
                dispersion: self.dispersion.clone(),
            },
        );
        Ok(())
    }
}
