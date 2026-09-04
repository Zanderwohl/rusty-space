//! Core physics and math foundations for orbital mechanics.
//!
//! Depends on `glam` and `serde` only — no engine, no ECS, no rendering.
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

#![forbid(unsafe_code)]

pub mod reference_frame;
pub mod gravity;
pub mod kepler;
pub mod spin;
pub mod time;

// Shared numeric helpers these modules are built on.
pub mod common;
pub mod mappings;
pub mod patched_conics;

#[cfg(test)]
mod standalone {
    //! Proves the crate is self-sufficient: pure `glam`, no engine, no ECS.
    //! If this ever needs a Bevy import, the extraction has regressed.
    use glam::DVec3;

    #[test]
    fn propagates_a_circular_orbit_with_no_engine() {
        const MU: f64 = 3.986004418e14; // Earth
        let a = 7.0e6;

        // Geometry
        let p = crate::kepler::semi_parameter::definition(a, 0.0);
        assert!((p - a).abs() < 1e-6, "circular semi-parameter should equal a");

        // Timing
        let period = crate::kepler::period::third_law(a, MU);
        assert!((period - 5828.5).abs() < 1.0, "LEO period ~5829 s, got {period}");

        // Dynamics
        let acc = crate::gravity::one_body_acceleration(MU, DVec3::new(a, 0.0, 0.0));
        assert!((acc.length() - MU / (a * a)).abs() < 1e-9);

        // Energy
        let v = (MU / a).sqrt();
        let eps = crate::kepler::energy::mechanical::specific(v, MU, a);
        assert!((eps + MU / (2.0 * a)).abs() < 1e-6, "specific energy should be -mu/2a");
    }

    #[test]
    fn epoch_conversion_is_self_consistent() {
        use crate::time::Instant;
        assert_eq!(Instant::J2000.to_j2000_seconds(), 0.0);
        assert!((Instant::J2000.to_julian_day() - 2451545.0).abs() < 1e-9);
    }
}
