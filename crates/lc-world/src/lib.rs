//! Game world model: populations, stellar photometry and occultation.
//!
//! Engine-agnostic. The server runs this headless and the client runs the same code for
//! prediction, so anything stochastic here is addressed by a hash rather than generated in
//! sequence — see [`rng`].

#![forbid(unsafe_code)]

pub mod distribution;
pub mod emission;
pub mod flicker;
pub mod occluder;
pub mod population;
pub mod rng;
pub mod shell;
pub mod star;

pub use distribution::{Distribution, Inclination};
pub use emission::{Body, EmissionModel, invert_moments};
pub use flicker::{Flicker, Regime};
pub use occluder::{Occluder, transit_depth};
pub use population::Population;
pub use shell::{Shell, ShellSample};
pub use star::Star;
