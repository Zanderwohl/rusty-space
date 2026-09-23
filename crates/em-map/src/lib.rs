//! What a spatial map draws, decided without an engine.
//!
//! A [`MapSnapshot`] is a flat list of things and where they are. Nothing here knows how they
//! were found out: instruments, several sources folded together, and a coordinate-time reading
//! with no light delay are three providers and one type.
//!
//! # Frame and units
//!
//! | Quantity | Unit |
//! |---|---|
//! | Position | light-years from the world origin, `f64` |
//! | Radius, distance, height | meters, `f64` |
//! | Angle | radians |
//! | Time | coordinate seconds since J2000 |
//!
//! Axes are **simulation space**: right-handed, Z-up, ecliptic of J2000, +X toward the vernal
//! equinox. The same frame and units `lc_world`'s `Drawable::position_ly` and `lc_proto`'s
//! `Presence::at_ly` already use, so a provider converts nothing.
//!
//! [`Placement`] is camera-relative and scaled, and still Z-up. The rotation into a
//! renderer's axes belongs to the renderer: `em_render::render_space` is the one place for it.

#![forbid(unsafe_code)]

pub mod camera;
pub mod frame;
pub mod label;
pub mod outline;
pub mod plane;
pub mod rings;
pub mod rule;
pub mod snapshot;
pub mod weight;

pub use camera::Orbit;
pub use frame::{Annulus, MapFrame, Placement, RingPlacement, compose};
pub use plane::{Datum, Plane};
pub use rings::Ring;
pub use snapshot::{ItemKey, ItemKind, MapItem, MapSnapshot, Provenance};

#[cfg(test)]
mod standalone {
    //! Proves the crate is self-sufficient: no engine, no ECS, no renderer.
    use glam::DVec3;

    use crate::{ItemKey, ItemKind, MapItem, MapSnapshot, Orbit, Plane, compose};

    #[test]
    fn composes_a_frame_with_no_engine() {
        let earth = DVec3::new(1.0e-5, 0.0, 2.0e-7);
        let snapshot = MapSnapshot::observed(0.0, vec![MapItem::body(
            ItemKey::from_name("Earth"), "Earth", ItemKind::Planet, earth, 6.371e6, DVec3::Z)]);
        let orbit = Orbit::framing(DVec3::ZERO, 3.0e11);
        let frame = compose(&snapshot, &orbit, Plane::System.about(DVec3::Z), 1.495_978_707e11);

        assert_eq!(frame.placements.len(), 1);
        assert!(frame.placements[0].at.is_finite());
        assert!(frame.placements[0].has_drop_line(), "Earth is not in the ecliptic to the meter");
        assert!(!frame.rings.is_empty(), "a decade ring should have been chosen");
    }
}
