//! The feature kinds this build knows.

use crate::compiled::{Compiled, Frame, Population, Target};
use crate::feature::{FeatureId, Problem, Registry};

mod frame;
mod plummer;
mod population;
mod variable;

pub use frame::FrameKind;
pub use plummer::{Contribution, Plummer, PlummerShape};
pub use population::{MAX_AGE_GYR, PopulationKind};
pub use variable::Variable;

pub fn register(r: &mut Registry) {
    r.register::<Variable>();
    r.register::<FrameKind>();
    r.register::<PopulationKind>();
    r.register::<Plummer>();
}

pub(crate) fn frame(out: &Compiled, id: FeatureId) -> Result<&Frame, Problem> {
    out.frames.get(&id).ok_or(Problem::Missing(id))
}

pub(crate) fn population(out: &Compiled, id: FeatureId) -> Result<&Population, Problem> {
    out.populations.get(&id).ok_or(Problem::Missing(id))
}

pub(crate) fn check_target(out: &Compiled, target: Target) -> Result<(), Problem> {
    match target.population() {
        Some(p) => population(out, p).map(|_| ()),
        None => Ok(()),
    }
}

pub(crate) fn fmt_v3((x, y, z): (f64, f64, f64)) -> String {
    format!("({x}, {y}, {z})")
}
