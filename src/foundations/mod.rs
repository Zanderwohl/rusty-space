//! Core physics and math foundations.
//!
//! # Unit Conventions
//!
//! This crate uses **SI base units** internally, with angles in **degrees** (astronomical
//! catalog convention). Convert to radians only at math call-sites (e.g., before trig functions).
//!
//! | Quantity              | Internal Unit         | Notes                                      |
//! |-----------------------|-----------------------|--------------------------------------------|
//! | Distance              | meters (m)            |                                            |
//! | Mass                  | kilograms (kg)        |                                            |
//! | Time                  | seconds since J2000   | Julian days only at persistence boundary  |
//! | Velocity              | m/s                   |                                            |
//! | Acceleration          | m/s²                  |                                            |
//! | Gravitational param   | m³/s²                 | μ = G × mass                               |
//! | Angles                | degrees               | Convert to radians at math call-sites      |
//! | Angular velocity      | rad/s                 | Output of orbital mechanics                |
//!
//! ## Examples
//!
//! ```ignore
//! // Storing an angle (degrees internally)
//! let mean_anomaly_deg = 174.796;
//!
//! // At math boundary, convert to radians
//! let mean_anomaly_rad = mean_anomaly_deg.to_radians();
//! let result = mean_anomaly_rad.sin();
//!
//! // Time: use seconds internally, Julian days at file boundaries
//! let seconds_since_j2000 = jd_to_seconds(julian_day);
//! ```

pub mod reference_frame;
pub mod gravity;
pub mod kepler;
pub mod spin;
pub mod time;
