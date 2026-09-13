//! Charts, curves and heat maps for this repository's plots.
//!
//! The core emits geometry and text *placements* and rasterises nothing — it needs only a way
//! to measure a string in order to place labels. That is what keeps the dependency list at
//! zero; a backend turns the output into meshes, painter calls or SVG.
//!
//! The load-bearing part is [`decimate::min_max`]. A light curve has millions of samples
//! against a few hundred pixels, and subsampling aliases: a one-sample transit disappears at
//! some zoom levels and reappears at others.

#![forbid(unsafe_code)]

pub mod chart;
pub mod colormap;
pub mod decimate;
pub mod font;
pub mod primitives;
pub mod scale;
#[cfg(feature = "raster")]
pub mod raster;
pub mod svg;

pub use chart::{Axis, Chart, Rect};
pub use colormap::ColorMap;
pub use decimate::{Envelope, min_max};
pub use primitives::{Monospace, Primitives, TextMetrics};
pub use scale::Scale;
