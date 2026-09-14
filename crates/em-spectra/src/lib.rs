//! Bands, blackbody radiation, extinction, colour, and the band-to-display mapping.
//!
//! Physics rather than game rule, so both products share it. Depends on `serde` and nothing
//! else: no engine, no ECS, no rendering.
//!
//! # Bands
//!
//! Seven, defined in [`bands`]. B, V, R and I are the Johnson-Cousins optical run and share
//! one silicon detector; K, thermal infrared and 21 cm each need different hardware, which is
//! where the instrument tiers come from.
//!
//! [`BANDS`] is a compile-time constant and is defined here only. Changing it is one edit and
//! a bump to any serialised format that stores per-band data.
//!
//! # Units
//!
//! SI. Wavelengths in metres, temperatures in kelvin, radiance in W m^-3 sr^-1 per unit
//! wavelength. Extinction is in magnitudes, as astronomy has it.

#![forbid(unsafe_code)]

pub mod bands;
pub mod blackbody;
pub mod cie;
pub mod colour_index;
pub mod extinction;
pub mod mapping;
pub mod stellar;

pub use bands::{BANDS, Band, BandMask, PerBand};
pub use colour_index::teff_from_bv;
pub use mapping::{BandMapping, presets};
