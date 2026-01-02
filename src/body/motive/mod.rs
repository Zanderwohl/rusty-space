pub mod info;
pub mod fixed_motive;
pub mod newton_motive;
pub mod mass;
pub mod kepler_motive;
pub mod compound_motive;

pub use compound_motive::{Motive, MotiveSelection, TransitionEvent};
