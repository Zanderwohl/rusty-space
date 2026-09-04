//! Core physics and math foundations for orbital mechanics.
//!
//! Depends on `glam` and `serde` only — no engine, no ECS, no rendering.
//!
//! # Units
//!
//! SI throughout, and **angles are radians, without exception**.
//!
//! | Quantity            | Unit                   |
//! |---------------------|------------------------|
//! | Distance            | metres                 |
//! | Mass                | kilograms              |
//! | Time                | see [`time`]           |
//! | Velocity            | m/s                    |
//! | Acceleration        | m/s²                   |
//! | Gravitational param | m³/s² (μ = G × mass)   |
//! | Angles              | **radians**            |
//! | Angular velocity    | rad/s                  |
//!
//! Degrees are a storage and display convention and stop at this crate's boundary;
//! callers convert once, on the way in. Hence no `Radians`/`Degrees` newtypes.
//!
//! Time is typed instead, because seconds and Julian days share no origin and mixing
//! them is silent. See [`time`].
//!
#![forbid(unsafe_code)]

pub mod reference_frame;
pub mod gravity;
pub mod kepler;
pub mod spin;
pub mod time;

pub mod common;
pub mod mappings;
pub mod patched_conics;

#[cfg(test)]
mod standalone {
    //! Proves the crate is self-sufficient: pure `glam`, no engine, no ECS.
    use glam::DVec3;

    #[test]
    fn propagates_a_circular_orbit_with_no_engine() {
        const MU: f64 = 3.986004418e14; // Earth
        let a = 7.0e6;

        let p = crate::kepler::semi_parameter::definition(a, 0.0);
        assert!((p - a).abs() < 1e-6, "circular semi-parameter should equal a");

        let period = crate::kepler::period::third_law(a, MU);
        assert!((period - 5828.5).abs() < 1.0, "LEO period ~5829 s, got {period}");

        let acc = crate::gravity::one_body_acceleration(MU, DVec3::new(a, 0.0, 0.0));
        assert!((acc.length() - MU / (a * a)).abs() < 1e-9);

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
