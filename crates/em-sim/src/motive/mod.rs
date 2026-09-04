//! How a body moves: fixed, Keplerian, or Newtonian, and the timeline of transitions
//! between them.

pub mod compound;
pub mod kepler;

pub use compound::{Motive, MotiveSelection, TransitionEvent};
