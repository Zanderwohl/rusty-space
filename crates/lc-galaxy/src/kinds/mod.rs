//! The feature kinds this build knows.

use crate::compiled::{Compiled, Frame, Population, Target};
use crate::feature::{FeatureId, Params, Problem, Registry};

mod frame;
mod plummer;
mod population;
mod profiles;
mod regions;
mod shape;
mod spiral;
mod variable;

pub use frame::FrameKind;
pub use plummer::{Contribution, Plummer, PlummerShape};
pub use population::{MAX_AGE_GYR, MAX_POPULATIONS, PopulationKind};
pub use profiles::{ExponentialDisc, PowerHalo, Sersic};
pub use regions::{Bar, Ellipsoid, King, Ring};
pub use shape::{Field, ShapeKind, Unit};
pub use spiral::{LogSpiral, MAX_ARMS};
pub use variable::Variable;

pub fn register(r: &mut Registry) {
    r.register::<Variable>();
    r.register::<FrameKind>();
    r.register::<PopulationKind>();
    r.register::<Plummer>();
    r.register::<King>();
    r.register::<ExponentialDisc>();
    r.register::<Sersic>();
    r.register::<PowerHalo>();
    r.register::<Ellipsoid>();
    r.register::<Ring>();
    r.register::<Bar>();
    r.register::<LogSpiral>();
}

/// The shape-kind face of `params`, if its kind places a shape.
pub fn as_shape(params: &mut dyn Params) -> Option<&mut dyn ShapeKind> {
    macro_rules! first_of {
        ($any:ident: $($t:ty),*) => {
            $(if $any.is::<$t>() {
                return $any.downcast_mut::<$t>().map(|t| t as &mut dyn ShapeKind);
            })*
        };
    }
    let any = params.as_any_mut();
    first_of!(any: Plummer, King, ExponentialDisc, Sersic, PowerHalo, Ellipsoid, Ring, Bar, LogSpiral);
    None
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
