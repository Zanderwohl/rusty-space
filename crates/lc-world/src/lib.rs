//! Game world model: the systems, what is in them, and how a ship moves through them.
//!
//! Engine-agnostic, and that is load-bearing rather than tidy. The server is authoritative over
//! ship motion and the client predicts locally by running the same code, so a brachistochrone,
//! a station and a ballistic arc have to mean the same thing on both sides — they are here
//! rather than in the client for exactly that reason. Anything stochastic is addressed by a
//! hash rather than generated in sequence, for the same reason; see [`rng`].

#![forbid(unsafe_code)]

pub mod attitude;
pub mod boost;
pub mod coast;
pub mod consort;
pub mod craft;
pub mod distribution;
pub mod emission;
pub mod escape;
pub mod escort;
pub mod flicker;
pub mod flight;
pub mod injection;
pub mod instrument;
pub mod motion;
pub mod libration;
pub mod navigation;
pub mod observation;
pub mod occluder;
pub mod population;
pub mod pursuit;
pub mod resume;
pub mod rings;
pub mod rng;
pub mod scenario;
pub mod shell;
pub mod sky;
pub mod star;
pub mod surface;
pub mod system;
pub mod transfer;
pub mod worldline;

pub use distribution::{Distribution, Inclination};
pub use emission::{Body, EmissionModel, invert_moments};
pub use flicker::{Flicker, Regime};
pub use instrument::{Instrument, SurveyRegime};
pub use observation::{Observation, Target, observe};
pub use occluder::{Occluder, transit_depth};
pub use population::Population;
pub use sky::{CatalogueStar, StarId, StarProvider};
pub use shell::{Shell, ShellSample};
pub use star::Star;
