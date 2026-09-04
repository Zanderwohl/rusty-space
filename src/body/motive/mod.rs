//! Body motion. The types live in `em-sim`; nothing ECS-specific remains here.
//!
//! Propagation moved to `em_sim::propagate`, driven from `crate::sim::world`.

// Re-exported under their old paths so app call sites are untouched by the move.
pub use em_sim::body as info;
pub use em_sim::motive::kepler as kepler_motive;
pub use em_sim::motive::{Motive, MotiveSelection, TransitionEvent};
