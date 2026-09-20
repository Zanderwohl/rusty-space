//! One frame's worth of decisions: what is where, at what size, with what under it.

use glam::{DVec3, Vec3};

use crate::camera::Orbit;
use crate::plane::Plane;
use crate::rings::{self, MAX_RINGS, Ring};
use crate::snapshot::{ItemKey, ItemKind, M_PER_LY, MapSnapshot};

/// How far out anything may be placed, in render units.
///
/// `f32` has a 24-bit mantissa, so a coordinate this size still resolves a hundredth of a unit.
/// Past it a placement is both invisible and a source of `inf`, so it is dropped — which is
/// also the bound `lc_client::view`'s own tier test asserts against.
pub const MAX_RENDER_UNITS: f32 = 1.0e5;

/// Where one item goes.
///
/// **Simulation axes, Z-up**, camera-relative and already divided by `meters_per_unit`. The
/// rotation into a renderer's axes is the renderer's: `em_render::render_space` is the one
/// place that does it, and a second converter here would be a second chance to get the
/// handedness wrong and no way to notice.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub key: ItemKey,
    pub kind: ItemKind,
    pub label: String,
    pub at: Vec3,
    /// Where a drop-line from [`Placement::at`] meets the plane. Equal to `at` for something
    /// already in it, which is the host's signal to draw no line at all.
    pub foot: Vec3,
    /// Render units. The true size, unfloored: something too small to be a sphere is drawn as
    /// a point instead, and the host decides which from [`Placement::angular_radius`].
    pub radius: f32,
    /// How big it looks from the eye, radians. Compared against the viewport's radians per
    /// pixel, this is what decides a sphere from a point.
    pub angular_radius: f32,
    /// How far a ring or a shell reaches, in render units, with the half-angle that says
    /// which of the two it is.
    pub annulus: Option<Annulus>,
    /// Spin axis or ring normal, simulation axes, unit length.
    pub pole: Vec3,
}

impl Placement {
    /// Whether there is a drop-line to draw. Something in the plane has nowhere to fall, and a
    /// zero-length tube is a degenerate mesh rather than an invisible one.
    pub fn has_drop_line(&self) -> bool {
        (self.at - self.foot).length_squared() > f32::EPSILON
    }
}

/// A population's reach, in render units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Annulus {
    pub inner: f32,
    pub outer: f32,
    pub half_angle_rad: f32,
}

impl Annulus {
    /// The shape normalized to an outer radius of one, which is what a mesh is built from: the
    /// host scales it by [`Annulus::outer`] rather than rebuilding it per zoom.
    pub fn unit(&self) -> crate::outline::Extent {
        let outer = self.outer.max(f32::MIN_POSITIVE) as f64;
        crate::outline::Extent {
            inner: (self.inner as f64 / outer).clamp(0.0, 1.0),
            outer: 1.0,
            half_angle_rad: self.half_angle_rad as f64,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RingPlacement {
    /// Render units.
    pub radius: f32,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapFrame {
    /// Where the camera is, light-years from the world origin.
    pub eye_ly: DVec3,
    /// Meters in one render unit.
    pub meters_per_unit: f64,
    pub plane: Plane,
    /// The plane's normal, simulation axes. What the rings lie perpendicular to.
    pub plane_normal: Vec3,
    /// The focus, camera-relative: what the camera is looking at.
    pub focus: Vec3,
    /// The observer, camera-relative — **where the rings and the spokes are centered**.
    ///
    /// Not the focus. A decade ring answers "how far is that from *me*", and the whole game is
    /// the ship's perspective, so the scale stays on the ship even while the camera is looking
    /// at a star. Falls back to the focus when a snapshot has no observer in it, which is a
    /// snapshot with nothing to be a perspective of.
    pub rings_at: Vec3,
    pub placements: Vec<Placement>,
    pub rings: Vec<RingPlacement>,
}

/// Reduce a snapshot and a camera to a frame.
///
/// The one pure function everything else is tested through. `meters_per_unit` is the host's,
/// because which scale tier is in force is the host's business — see `lc_client::view`.
pub fn compose(snapshot: &MapSnapshot, orbit: &Orbit, plane: Plane, meters_per_unit: f64)
    -> MapFrame {
    let eye_ly = orbit.eye_ly(plane);
    let ly_per_unit = meters_per_unit / M_PER_LY;
    let relative = |at_ly: DVec3| ((at_ly - eye_ly) / ly_per_unit).as_vec3();

    // Rings are measured from the observer, not from whatever the camera happens to be on.
    let rings_ly = snapshot.observer().map_or(orbit.focus_ly, |o| o.position_ly);
    let (near_m, far_m) = snapshot.extent_m(rings_ly);
    // Out to whichever is further, the furthest thing or the edge of the view. A map zoomed
    // out past everything in it would otherwise lose its scale exactly when it needs one.
    let outer_m = far_m.max(orbit.distance_m());
    let rings = rings::decades(near_m, outer_m, MAX_RINGS)
        .into_iter()
        .map(|Ring { radius_m, label }| RingPlacement {
            radius: (radius_m / meters_per_unit) as f32,
            label,
        })
        .filter(|ring| ring.radius.is_finite() && ring.radius <= MAX_RENDER_UNITS)
        .collect();

    let mut placements = Vec::with_capacity(snapshot.items.len());
    for item in &snapshot.items {
        let at = relative(item.position_ly);
        if !at.is_finite() || at.length() > MAX_RENDER_UNITS {
            continue;
        }
        // The same plane the rings are drawn on, anchored at the observer. Measuring height
        // above a plane through the camera's focus while the rings sat on the ship was two
        // planes answering one question.
        let foot = relative(plane.foot_ly(item.position_ly, rings_ly));
        if !foot.is_finite() {
            continue;
        }
        let range_m = (item.position_ly.distance(eye_ly) * M_PER_LY).max(f64::MIN_POSITIVE);
        placements.push(Placement {
            key: item.key,
            kind: item.kind,
            label: item.label.clone(),
            at,
            foot,
            radius: (item.radius_m / meters_per_unit) as f32,
            angular_radius: (item.radius_m / range_m) as f32,
            annulus: item.annulus_m.map(|a| Annulus {
                inner: (a.inner / meters_per_unit) as f32,
                outer: (a.outer / meters_per_unit) as f32,
                half_angle_rad: a.half_angle_rad as f32,
            }),
            pole: item.pole.normalize_or(DVec3::Z).as_vec3(),
        });
    }

    MapFrame {
        eye_ly,
        meters_per_unit,
        plane,
        plane_normal: plane.normal().as_vec3(),
        focus: relative(orbit.focus_ly),
        rings_at: relative(rings_ly),
        placements,
        rings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{M_PER_AU, MapItem};

    fn at(key: u64, position_ly: DVec3, radius_m: f64) -> MapItem {
        MapItem::body(ItemKey(key), format!("{key}"), ItemKind::Planet, position_ly, radius_m,
            DVec3::Z)
    }

    fn au(x: f64, y: f64, z: f64) -> DVec3 {
        DVec3::new(x, y, z) * M_PER_AU / M_PER_LY
    }

    /// Nothing ever reaches the renderer as a number `f32` cannot hold.
    ///
    /// Break it by dropping the cull and a catalogue star at a thousand light-years, composed
    /// at the system tier, arrives as `inf` — which draws as nothing and takes the whole
    /// instance batch with it.
    #[test]
    fn every_placement_stays_inside_f32() {
        let far = DVec3::new(1000.0, 0.0, 0.0);
        let snapshot = MapSnapshot::observed(0.0, vec![at(1, au(1.0, 0.0, 0.1), 6.4e6),
            at(2, far, 7.0e8)]);
        let orbit = Orbit::framing(DVec3::ZERO, 10.0 * M_PER_AU);
        let frame = compose(&snapshot, &orbit, Plane::Ecliptic, M_PER_AU);

        assert_eq!(frame.placements.len(), 1, "the thousand-light-year star should be culled");
        for p in &frame.placements {
            assert!(p.at.is_finite() && p.at.length() <= MAX_RENDER_UNITS, "{:?}", p.at);
            assert!(p.foot.is_finite() && p.radius.is_finite());
        }
    }

    /// A drop-line ends in the plane, for both planes and both sides of it.
    ///
    /// Break it by using `+Z` for the galactic plane and only the galactic half fails, which
    /// is why both are here.
    #[test]
    fn a_drop_line_ends_in_the_plane() {
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            for height in [0.3, -0.3] {
                let there = au(2.0, 1.0, 0.0) + plane.normal() * height * M_PER_AU / M_PER_LY;
                let snapshot = MapSnapshot::observed(0.0, vec![at(1, there, 6.4e6)]);
                let orbit = Orbit::framing(DVec3::ZERO, 10.0 * M_PER_AU);
                let frame = compose(&snapshot, &orbit, plane, M_PER_AU);

                let p = &frame.placements[0];
                assert!(p.has_drop_line(), "{plane:?} at height {height} drew no line");
                // The drop is along the normal, and it ends level with the focus.
                let drop = p.at - p.foot;
                assert!(drop.cross(frame.plane_normal).length() < 1e-3, "{plane:?}: not vertical");
                let above = (p.foot - frame.focus).dot(frame.plane_normal);
                assert!(above.abs() < 1e-3, "{plane:?}: foot {above} units off the plane");
            }
        }
    }

    /// Something in the plane has nowhere to fall, and the host must draw no tube: a
    /// zero-length one is degenerate geometry, not an invisible one.
    #[test]
    fn something_in_the_plane_has_no_drop_line() {
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            let (u, _, _) = plane.basis();
            let snapshot = MapSnapshot::observed(0.0, vec![at(1, u * 3.0e-5, 6.4e6)]);
            let orbit = Orbit::framing(DVec3::ZERO, 10.0 * M_PER_AU);
            let frame = compose(&snapshot, &orbit, plane, M_PER_AU);
            assert!(!frame.placements[0].has_drop_line(), "{plane:?}");
        }
    }

    /// Angular size is what decides a sphere from a point, so it has to be a real angle: a
    /// body twice as far away looks half as big.
    #[test]
    fn angular_size_falls_off_with_range() {
        let near = MapSnapshot::observed(0.0, vec![at(1, au(1.0, 0.0, 0.0), 7.0e7)]);
        let far = MapSnapshot::observed(0.0, vec![at(1, au(2.0, 0.0, 0.0), 7.0e7)]);
        let orbit = Orbit { focus_ly: DVec3::ZERO, azimuth: 0.0, elevation: 0.2,
            log_distance_m: (1.0e-6 * M_PER_AU).log10() };

        let a = compose(&near, &orbit, Plane::Ecliptic, M_PER_AU).placements[0].angular_radius;
        let b = compose(&far, &orbit, Plane::Ecliptic, M_PER_AU).placements[0].angular_radius;
        assert!((a / b - 2.0).abs() < 0.01, "{a} against {b}");
    }

    /// A map zoomed out past everything in it still has a scale on it.
    #[test]
    fn the_rings_reach_the_edge_of_the_view() {
        let snapshot = MapSnapshot::observed(0.0, vec![at(1, au(1.0, 0.0, 0.0), 6.4e6)]);
        let orbit = Orbit::framing(DVec3::ZERO, 1.0e4 * M_PER_AU);
        let frame = compose(&snapshot, &orbit, Plane::Ecliptic, M_PER_AU);

        assert!(!frame.rings.is_empty());
        let outermost = frame.rings.iter().map(|r| r.radius).fold(0.0f32, f32::max);
        assert!(outermost >= 1.0e4, "outermost ring at {outermost} units, view is 1e4");
    }

    /// The rings measure distance from the **ship**, not from whatever the camera is on.
    ///
    /// The scale belongs to the observer: "how far is that from me" is the question a decade
    /// ring answers, and centering it on a star the camera happened to be looking at answers a
    /// different one.
    #[test]
    fn the_rings_are_centered_on_the_observer() {
        let ship = au(30.0, 0.0, 0.0);
        let star = DVec3::ZERO;
        let snapshot = MapSnapshot::observed(0.0, vec![
            MapItem::body(ItemKey::from_name("observer"), "ship", ItemKind::Observer, ship,
                100.0, DVec3::Z),
            at(2, star, 7.0e8),
        ]);
        // The camera is looking at the star, which is where the rings must *not* be.
        let orbit = Orbit::framing(star, 10.0 * M_PER_AU);
        let frame = compose(&snapshot, &orbit, Plane::Ecliptic, M_PER_AU);

        let ship_at = frame.placements.iter().find(|p| p.kind == ItemKind::Observer).unwrap().at;
        assert!(
            (frame.rings_at - ship_at).length() < 1e-3,
            "rings at {:?}, ship at {ship_at:?}",
            frame.rings_at,
        );
        assert!(
            (frame.rings_at - frame.focus).length() > 1.0,
            "the test is not distinguishing the two",
        );
    }

    /// With nobody to be a perspective of, the rings fall back to the focus rather than to the
    /// world origin.
    #[test]
    fn rings_without_an_observer_fall_back_to_the_focus() {
        let snapshot = MapSnapshot::observed(0.0, vec![at(1, au(1.0, 0.0, 0.0), 6.4e6)]);
        let orbit = Orbit::framing(au(1.0, 0.0, 0.0), 10.0 * M_PER_AU);
        let frame = compose(&snapshot, &orbit, Plane::Ecliptic, M_PER_AU);
        assert_eq!(frame.rings_at, frame.focus);
    }

    /// An empty snapshot is a frame too — a ship between systems has one — and it must not be
    /// a pile of NaNs.
    #[test]
    fn nothing_to_draw_is_still_a_frame() {
        let frame = compose(&MapSnapshot::observed(0.0, Vec::new()), &Orbit::default(),
            Plane::Ecliptic, M_PER_AU);
        assert!(frame.placements.is_empty());
        assert!(frame.focus.is_finite());
        for ring in &frame.rings {
            assert!(ring.radius.is_finite() && ring.radius > 0.0);
        }
    }
}
