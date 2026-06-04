pub mod motive;
pub mod universe;
pub mod appearance;

// Re-export simulation types that are commonly used with body types.
// The canonical definitions live in `crate::sim`.
pub use crate::sim::{SimulationObject, unload_simulation_objects};
