//! Authoring the Lightcone galaxy: a timeline of features that defines density and attribute
//! fields over space, and the bake that turns those fields into stars.
//!
//! Engine-free, and linked by nothing that ships. The design is
//! `lightcone/docs/34-galaxy-authoring.md`.
//!
//! Lengths are light-years and angles are stored in degrees, with the unit in the field name;
//! evaluation converts once. Positions are in the galaxy frame of `34-galaxy-authoring.md`:
//! origin at the main galaxy's center, +Z toward galactic north.

pub mod attributes;
pub mod codec;
pub mod compiled;
pub mod feature;
pub mod kinds;
pub mod num;
pub mod sample;
pub mod stats;
pub mod timeline;

pub use compiled::{Aabb, Compiled, Op, Shape, Target, Term};
pub use feature::{Feature, FeatureId, FeatureKind, Output, Params, Problem, Registry};
pub use num::Num;
pub use sample::{Bake, BakeError, BakedStar, Cell, StarKey, bake};
pub use timeline::{Row, Timeline, Unread};
