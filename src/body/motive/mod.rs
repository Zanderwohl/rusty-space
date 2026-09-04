//! Body motion. The types live in `em-sim`; what remains here is legacy ECS-only
//! motive components and the Bevy systems that drive propagation.

pub mod fixed_motive;
pub mod newton_motive;
pub mod trajectory;
pub mod calculate_body_positions;

// Re-exported under their old paths so app call sites are untouched by the move.
pub use em_sim::body as info;
pub use em_sim::motive::kepler as kepler_motive;
pub use em_sim::motive::{Motive, MotiveSelection, TransitionEvent};
pub use calculate_body_positions::{calculate_body_positions, PhysicsGraph, PositionCache, SimulationPerformanceMetrics};
