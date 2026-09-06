//! How a body moves: fixed, Keplerian, or Newtonian, plus the timeline of transitions.

pub mod compound;
pub mod kepler;

pub use compound::{Frontier, Motive, MotiveSelection, TransitionEvent};
