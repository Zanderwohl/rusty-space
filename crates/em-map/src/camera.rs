//! Where the map is looked at from.

use glam::DVec3;

use crate::plane::Plane;
use crate::snapshot::{M_PER_AU, M_PER_LY};

/// Elevation stops just short of the pole, where the azimuth stops being defined.
///
/// The same limit and the same reason as `lc_client::ui::Look::PITCH_LIMIT`.
pub const ELEVATION_LIMIT: f64 = std::f64::consts::FRAC_PI_2 - 1.0e-3;

/// And the camera is never in the plane. An angle, so the stand-off scales by itself.
///
/// It must exceed a drawn line's angular half-width, near three milliradians for a tube drawn
/// at 1.6 pixels; a camera closer than that is inside the nearest ring, whose inside is
/// opaque. Three degrees is still edge-on and an order of magnitude clear.
pub const ELEVATION_FLOOR: f64 = 5.0e-2;

/// Decades of stand-off per notch of wheel.
///
/// Zoom is additive in `log10` meters, so one notch means the same fraction at a hull as at a
/// spiral arm. Seven notches to a decade, and about 113 over the whole range.
pub const ZOOM_DECADES_PER_NOTCH: f64 = 0.15;

/// `log10` of the closest the camera stands off, meters. A kilometer: a hull.
pub const LOG_MIN_M: f64 = 3.0;

/// And the furthest. 1e20 m is about ten thousand light-years.
pub const LOG_MAX_M: f64 = 20.0;

/// High enough to read as a plane seen from above, low enough that height off it still shows.
const DEFAULT_ELEVATION: f64 = 25.0 * std::f64::consts::PI / 180.0;

/// An orbit camera about a point in the reference plane.
///
/// Angles are measured in the plane's own basis, so toggling the plane tilts the whole view
/// rather than leaving the rings lying at an angle under an unchanged camera.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    /// Light-years from the world origin, simulation axes. A position, not a thing: the host
    /// holds the selection and writes this every frame from wherever it is now.
    pub focus_ly: DVec3,
    /// Bearing within the plane, radians, from the plane basis's first axis.
    pub azimuth: f64,
    /// Above the plane, radians. Never zero and never at a pole — see the two clamps.
    pub elevation: f64,
    /// `log10` of the stand-off in meters. Zoom is addition here.
    pub log_distance_m: f64,
}

impl Default for Orbit {
    fn default() -> Self {
        Self::framing(DVec3::ZERO, 40.0 * M_PER_AU)
    }
}

impl Orbit {
    /// Looking at `focus_ly` from `distance_m` away, at the default bearing.
    pub fn framing(focus_ly: DVec3, distance_m: f64) -> Self {
        let mut orbit = Self {
            focus_ly,
            azimuth: 0.0,
            elevation: DEFAULT_ELEVATION,
            log_distance_m: 0.0,
        };
        orbit.set_distance_m(distance_m);
        orbit
    }

    pub fn distance_m(&self) -> f64 {
        10f64.powf(self.log_distance_m)
    }

    pub fn set_distance_m(&mut self, meters: f64) {
        let log = match meters > 0.0 && meters.is_finite() {
            true => meters.log10(),
            false => LOG_MIN_M,
        };
        self.log_distance_m = log.clamp(LOG_MIN_M, LOG_MAX_M);
    }

    /// Positive notches move closer, matching the wheel everywhere else in the interface.
    pub fn zoom(&mut self, notches: f64) {
        self.log_distance_m = (self.log_distance_m - notches * ZOOM_DECADES_PER_NOTCH)
            .clamp(LOG_MIN_M, LOG_MAX_M);
    }

    /// Turn by a relative amount, radians, holding both clamps.
    ///
    /// The plane can be crossed but not landed on: a turn ending inside the floor continues to
    /// the far side, so elevation skips a band of about six degrees. Stopping at the floor
    /// instead confines the camera to one hemisphere.
    pub fn turn(&mut self, d_azimuth: f64, d_elevation: f64) {
        self.azimuth = (self.azimuth + d_azimuth).rem_euclid(std::f64::consts::TAU);
        let next = (self.elevation + d_elevation).clamp(-ELEVATION_LIMIT, ELEVATION_LIMIT);
        if next.abs() >= ELEVATION_FLOOR {
            self.elevation = next;
            return;
        }
        // Inside the band: leave it on the side the turn was heading for, or the side it
        // started from when the turn was not about elevation.
        let heading = match d_elevation.partial_cmp(&0.0) {
            Some(std::cmp::Ordering::Less) => -1.0,
            Some(std::cmp::Ordering::Greater) => 1.0,
            _ if self.elevation < 0.0 => -1.0,
            _ => 1.0,
        };
        self.elevation = heading * ELEVATION_FLOOR;
    }

    /// Zoom while holding `anchor_ly` still on screen: with the distance multiplied by `k`,
    /// a focus at `anchor + (focus - anchor)*k` scales `eye - anchor` by the same `k`.
    ///
    /// Returns whether the focus moved, which it does not when the anchor is already the
    /// focus — so a locked camera stays locked while the wheel turns over it.
    pub fn zoom_about(&mut self, anchor_ly: DVec3, notches: f64) -> bool {
        let before = self.distance_m();
        self.zoom(notches);
        let k = self.distance_m() / before;
        if !k.is_finite() || k <= 0.0 {
            return false;
        }
        let moved = anchor_ly + (self.focus_ly - anchor_ly) * k;
        let changed = moved != self.focus_ly;
        self.focus_ly = moved;
        changed
    }

    /// A ray from the eye through a point on the viewport, in simulation axes.
    ///
    /// `ndc` is `[-1, 1]` across the viewport with `+y` up, which is the frame a renderer hands
    /// out; `aspect` is width over height.
    pub fn ray(&self, plane: Plane, ndc: glam::DVec2, fov_y: f64, aspect: f64) -> DVec3 {
        let (forward, right, up) = self.view_basis(plane);
        let tan_half = (fov_y * 0.5).tan();
        (forward + right * (ndc.x * tan_half * aspect) + up * (ndc.y * tan_half))
            .normalize_or(forward)
    }

    /// Where a camera-relative offset lands on the viewport, in normalized device
    /// coordinates. The exact inverse of [`Orbit::ray`].
    ///
    /// `offset` is in simulation axes from the eye. `None` for anything at or behind the plane
    /// of the eye; [`Orbit::clip`] is the form that can still answer for those.
    pub fn project(&self, plane: Plane, offset: DVec3, fov_y: f64, aspect: f64)
        -> Option<glam::DVec2> {
        let clip = self.clip(plane, offset, fov_y, aspect);
        if !(clip.w > 0.0) || !clip.w.is_finite() {
            return None;
        }
        Some(glam::DVec2::new(clip.x / clip.w, clip.y / clip.w))
    }

    /// The same projection without the perspective divide.
    ///
    /// `w` is the depth along the view direction, so behind the eye it is negative while `x`
    /// and `y` keep their signs. Dividing anyway mirrors the point through the middle of the
    /// view, and an edge marker built from that points away from what it marks. `z` is unused.
    pub fn clip(&self, plane: Plane, offset: DVec3, fov_y: f64, aspect: f64) -> glam::DVec4 {
        let (forward, right, up) = self.view_basis(plane);
        let tan_half = (fov_y * 0.5).tan();
        glam::DVec4::new(
            offset.dot(right) / (tan_half * aspect),
            offset.dot(up) / tan_half,
            0.0,
            offset.dot(forward),
        )
    }

    /// The camera's own orthonormal frame: forward, screen right, screen up.
    ///
    /// Not the plane's normal for up. [`Orbit::orientation`] hands out the normal because
    /// `look_to` wants it and orthonormalizes it; at any elevation but zero the normal is not
    /// perpendicular to forward, so anything casting rays has to orthonormalize too.
    pub fn view_basis(&self, plane: Plane) -> (DVec3, DVec3, DVec3) {
        let (forward, normal) = self.orientation(plane);
        let right = forward.cross(normal).normalize_or(DVec3::X);
        (forward, right, right.cross(forward).normalize_or(normal))
    }

    /// Slide the focus across the plane, in fractions of the stand-off. Fractions rather than
    /// meters, so a drag moves the view by the same part of itself at every zoom.
    pub fn pan(&mut self, plane: Plane, right: f64, ahead: f64) {
        let normal = plane.normal();
        let screen_right = (-self.offset_direction(plane)).cross(normal).normalize_or_zero();
        if screen_right == DVec3::ZERO {
            return;
        }
        let screen_ahead = normal.cross(screen_right);
        let step_ly = self.distance_m() / M_PER_LY;
        self.focus_ly += (screen_right * right + screen_ahead * ahead) * step_ly;
    }

    /// Unit vector from the focus toward the eye, simulation axes.
    pub fn offset_direction(&self, plane: Plane) -> DVec3 {
        let (u, v, n) = plane.basis();
        let (sin_az, cos_az) = self.azimuth.sin_cos();
        let (sin_el, cos_el) = self.elevation.sin_cos();
        (u * cos_az + v * sin_az) * cos_el + n * sin_el
    }

    /// Light-years from the world origin, simulation axes.
    pub fn eye_ly(&self, plane: Plane) -> DVec3 {
        self.focus_ly + self.offset_direction(plane) * (self.distance_m() / M_PER_LY)
    }

    /// Which way the camera faces and which way is up, simulation axes.
    ///
    /// Up is the plane's normal, not `+Z`, which is what makes the plane toggle tilt the whole
    /// view. Well conditioned because [`ELEVATION_LIMIT`] keeps forward off the normal.
    pub fn orientation(&self, plane: Plane) -> (DVec3, DVec3) {
        (-self.offset_direction(plane), plane.normal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Projecting must be the exact inverse of casting, or a label lands away from the body
    /// the cursor picks.
    #[test]
    fn a_projected_ray_lands_where_it_was_cast_from() {
        let fov = std::f64::consts::FRAC_PI_4;
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            for aspect in [0.5, 1.0, 1.77] {
                let orbit = Orbit {
                    focus_ly: DVec3::new(0.3, -0.2, 0.05),
                    azimuth: 0.9,
                    elevation: 0.4,
                    log_distance_m: 12.0,
                };
                for ndc in [
                    glam::DVec2::ZERO,
                    glam::DVec2::new(0.9, 0.9),
                    glam::DVec2::new(-0.7, 0.3),
                    glam::DVec2::new(0.2, -0.95),
                ] {
                    for range in [1.0e-3f64, 1.0, 1.0e6] {
                        let offset = orbit.ray(plane, ndc, fov, aspect) * range;
                        let back = orbit.project(plane, offset, fov, aspect).expect("in front");
                        assert!(
                            (back - ndc).length() < 1.0e-9,
                            "{plane:?} aspect {aspect}: {ndc:?} came back {back:?}",
                        );
                    }
                }
                // Nothing behind the eye projects.
                let (forward, ..) = orbit.view_basis(plane);
                assert!(orbit.project(plane, -forward, fov, aspect).is_none());
                assert!(orbit.project(plane, DVec3::ZERO, fov, aspect).is_none());
            }
        }
    }

    /// An edge marker is built from the undivided form. Dividing through a negative `w` puts
    /// the arrow on the opposite edge, pointing away from what it marks.
    #[test]
    fn the_undivided_form_still_says_which_way_a_thing_lies() {
        let fov = std::f64::consts::FRAC_PI_4;
        let orbit = Orbit::framing(DVec3::ZERO, M_PER_AU);
        let (forward, right, up) = orbit.view_basis(Plane::Ecliptic);

        // Behind and to the right: `w` negative, `x` positive, which is the pair a caller
        // reads.
        let behind = -forward * 2.0 + right * 0.5 + up * 0.25;
        let clip = orbit.clip(Plane::Ecliptic, behind, fov, 1.6);
        assert!(clip.w < 0.0, "{clip:?}");
        assert!(clip.x > 0.0 && clip.y > 0.0, "{clip:?}");
        assert!(orbit.project(Plane::Ecliptic, behind, fov, 1.6).is_none());
    }

    /// In front it is the projection exactly. The labels project and the marks clip over one
    /// picture, so the two must not be able to drift apart.
    #[test]
    fn dividing_the_clip_is_the_projection() {
        let fov = std::f64::consts::FRAC_PI_4;
        let orbit = Orbit { focus_ly: DVec3::new(0.3, -0.2, 0.05), azimuth: 0.9,
            elevation: 0.4, log_distance_m: 12.0 };
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            for ndc in [glam::DVec2::ZERO, glam::DVec2::new(0.9, -0.4)] {
                let offset = orbit.ray(plane, ndc, fov, 1.6) * 3.0;
                let clip = orbit.clip(plane, offset, fov, 1.6);
                let divided = glam::DVec2::new(clip.x / clip.w, clip.y / clip.w);
                let projected = orbit.project(plane, offset, fov, 1.6).expect("in front");
                assert!((divided - projected).length() < 1.0e-12, "{plane:?} {ndc:?}");
            }
        }
    }

    /// One notch is the same fraction wherever it is spent. Subtracting meters instead of
    /// decades leaves the zoom dead at one end of the range.
    #[test]
    fn a_notch_is_the_same_fraction_at_every_scale() {
        let ratio_at = |log| {
            let mut orbit = Orbit::default();
            orbit.log_distance_m = log;
            let before = orbit.distance_m();
            orbit.zoom(1.0);
            orbit.distance_m() / before
        };
        let close = ratio_at(4.0);
        let far = ratio_at(18.0);
        assert!((close - far).abs() < 1e-9, "{close} near, {far} far");
        assert!(close < 1.0, "a positive notch should move closer, got {close}");
    }

    /// And the whole range is reachable in a number of notches a hand can produce.
    #[test]
    fn the_whole_range_is_a_hundred_notches_or_so() {
        let mut orbit = Orbit::default();
        orbit.log_distance_m = LOG_MAX_M;
        let mut notches = 0;
        while orbit.log_distance_m > LOG_MIN_M && notches < 1000 {
            orbit.zoom(1.0);
            notches += 1;
        }
        assert!((100..=130).contains(&notches), "{notches} notches end to end");
        assert_eq!(orbit.log_distance_m, LOG_MIN_M, "the near stop is a clamp");
    }

    /// The camera never stands in the plane it is drawing.
    ///
    /// Every third turn is aimed straight at zero, which is what a hand dragging the view flat
    /// does. A sweep of arbitrary deltas never lands on the floor and so tests nothing.
    #[test]
    fn the_camera_is_never_in_the_plane() {
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            for start in [0.4, -0.4] {
                let mut orbit = Orbit::framing(DVec3::ZERO, M_PER_AU);
                orbit.elevation = start;
                for i in 0..1000 {
                    let d_elevation = match i % 3 {
                        0 => -orbit.elevation,
                        1 => -orbit.elevation * 2.0,
                        _ => (i as f64 * 0.017).sin() * 0.9,
                    };
                    orbit.turn(0.11, d_elevation);
                    let height = plane.height_m(orbit.eye_ly(plane), orbit.focus_ly).abs();
                    let floor = orbit.distance_m() * ELEVATION_FLOOR * 0.99;
                    assert!(height >= floor, "{plane:?} step {i}: {height} m, want {floor}");
                }
            }
        }
    }

    /// Both hemispheres are reachable. Stopping a turn at the floor on the side it came from
    /// confines the camera above the plane.
    #[test]
    fn the_camera_can_get_under_the_plane() {
        let mut orbit = Orbit::framing(DVec3::ZERO, M_PER_AU);
        let mut lowest: f64 = orbit.elevation;
        for _ in 0..500 {
            orbit.turn(0.0, -0.006);
            lowest = lowest.min(orbit.elevation);
        }
        assert!(lowest < -0.5, "never got below the plane; lowest was {lowest}");
        assert!(orbit.elevation >= -ELEVATION_LIMIT);
    }

    /// And it passes through the band rather than landing in it.
    #[test]
    fn a_turn_steps_over_the_plane_rather_than_onto_it() {
        for (from, delta, want) in [(0.5, -0.5, -1.0), (-0.5, 0.5, 1.0)] {
            let mut orbit = Orbit::framing(DVec3::ZERO, M_PER_AU);
            orbit.elevation = from;
            orbit.turn(0.0, delta);
            assert!(orbit.elevation.abs() >= ELEVATION_FLOOR, "landed in the plane");
            assert_eq!(orbit.elevation.signum(), want, "stepped the wrong way from {from}");
        }
    }

    /// And it never reaches a pole, where the azimuth would stop meaning anything.
    #[test]
    fn a_turn_does_not_reach_a_pole() {
        let mut orbit = Orbit::default();
        for _ in 0..100 {
            orbit.turn(0.0, 1.0);
        }
        assert!(orbit.elevation <= ELEVATION_LIMIT);
        let (forward, up) = orbit.orientation(Plane::Ecliptic);
        assert!(forward.cross(up).length() > 1e-4, "forward and up have become parallel");
    }

    /// Azimuth is measured in the plane's basis, so the toggle tilts the view rather than
    /// leaving the rings at an angle under an unchanged camera.
    #[test]
    fn the_plane_decides_where_the_camera_stands() {
        let orbit = Orbit::framing(DVec3::ZERO, M_PER_AU);
        let ecliptic = orbit.eye_ly(Plane::Ecliptic);
        let galactic = orbit.eye_ly(Plane::Galactic);
        assert!(ecliptic.distance(galactic) > 1e-9, "the two planes put the eye in one place");
        // And the stand-off is the plane's business only in direction, never in distance.
        assert!((ecliptic.length() - galactic.length()).abs() < 1e-12);
    }

    /// A pan of the same fraction covers the same part of the view at every zoom.
    #[test]
    fn a_pan_is_a_fraction_of_the_view() {
        let moved_at = |meters| {
            let mut orbit = Orbit::framing(DVec3::ZERO, meters);
            orbit.pan(Plane::Ecliptic, 0.25, 0.0);
            orbit.focus_ly.length() * M_PER_LY / meters
        };
        assert!((moved_at(M_PER_AU) - moved_at(1.0e4 * M_PER_AU)).abs() < 1e-9);
        assert!(moved_at(M_PER_AU) > 0.0, "a pan moved nothing");
    }

    /// The anchor holds its place on screen: that is the whole of what zoom-to-cursor means.
    ///
    /// Checked by projecting it, rather than by trusting the algebra — the anchor has to land
    /// on the same pixel before and after, and a camera that merely ended up nearer would pass
    /// a test written about distances.
    #[test]
    fn a_zoom_holds_its_anchor_on_screen() {
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            let mut orbit = Orbit::framing(DVec3::ZERO, 40.0 * M_PER_AU);
            orbit.turn(0.7, -0.2);
            // Somewhere off-center, in the plane, a quarter of the view away.
            let (u, v, _) = plane.basis();
            let anchor = (u * 0.3 + v * 0.2) * 10.0 * M_PER_AU / M_PER_LY;

            let before = screen_of(&orbit, plane, anchor);
            let closer = orbit.distance_m();
            assert!(orbit.zoom_about(anchor, 12.0), "an off-center anchor should move the focus");
            assert!(orbit.distance_m() < closer, "it should have come closer");
            let after = screen_of(&orbit, plane, anchor);

            assert!(
                (before - after).length() < 1.0e-6,
                "{plane:?}: the anchor moved from {before:?} to {after:?}",
            );
        }
    }

    /// A ray cast through a point on the viewport comes back to that same point.
    ///
    /// The round trip is the assertion worth making: a flipped sign, a dropped aspect or a
    /// half-angle used where a full one belongs all survive "it hit something", and all of
    /// them put the anchor somewhere the pointer is not.
    #[test]
    fn a_cursor_ray_lands_where_the_cursor_is() {
        let fov_y = std::f32::consts::FRAC_PI_4 as f64;
        let tan_half = (fov_y * 0.5).tan();
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            for aspect in [1.0, 16.0 / 9.0, 0.6] {
                let mut orbit = Orbit::framing(DVec3::ZERO, 40.0 * M_PER_AU);
                // Steeply enough that every sampled ray still meets the plane: from low down,
                // one through the top of the viewport points above the horizon and correctly
                // meets nothing, which `a_ray_that_meets_nothing_says_so` covers instead.
                orbit.turn(0.4, 0.5);
                for ndc in [
                    glam::DVec2::ZERO,
                    glam::DVec2::new(0.5, 0.0),
                    glam::DVec2::new(-0.3, 0.4),
                    glam::DVec2::new(0.2, -0.6),
                ] {
                    let direction = orbit.ray(plane, ndc, fov_y, aspect);
                    let hit = plane
                        .intersect(orbit.eye_ly(plane), direction, orbit.focus_ly)
                        .expect("a ray through the viewport should meet the plane");
                    let back = screen_of(&orbit, plane, hit) / tan_half;
                    assert!(
                        (back.x - ndc.x * aspect).abs() < 1e-9 && (back.y - ndc.y).abs() < 1e-9,
                        "{plane:?} at {aspect}: {ndc:?} came back as {back:?}",
                    );
                }
            }
        }
    }

    /// The middle of the viewport is the focus, which is the one point that can be checked
    /// without trusting any of the projection at all.
    #[test]
    fn the_middle_of_the_view_is_the_focus() {
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            let mut orbit = Orbit::framing(DVec3::new(0.5, -0.25, 0.0), 12.0 * M_PER_AU);
            orbit.turn(1.1, 0.15);
            let direction = orbit.ray(plane, glam::DVec2::ZERO, 0.8, 1.5);
            let hit = plane.intersect(orbit.eye_ly(plane), direction, orbit.focus_ly).unwrap();
            let off_m = hit.distance(orbit.focus_ly) * M_PER_LY;
            assert!(off_m < 1.0e3, "{plane:?}: the middle missed the focus by {off_m} m");
        }
    }

    /// A ray pointing away from the plane, or along it, names nothing.
    #[test]
    fn a_ray_that_meets_nothing_says_so() {
        let orbit = Orbit::framing(DVec3::ZERO, M_PER_AU);
        let plane = Plane::Ecliptic;
        let eye = orbit.eye_ly(plane);
        assert!(plane.intersect(eye, DVec3::Z, orbit.focus_ly).is_none(), "away from the plane");
        assert!(plane.intersect(eye, DVec3::X, orbit.focus_ly).is_none(), "along the plane");
    }

    /// Zooming about the focus is an ordinary zoom and leaves the focus alone, which is what
    /// keeps a camera locked on the ship locked while the wheel turns over it.
    #[test]
    fn zooming_about_the_focus_does_not_move_it() {
        let mut orbit = Orbit::framing(DVec3::new(1.0, 2.0, 3.0), 40.0 * M_PER_AU);
        let focus = orbit.focus_ly;
        assert!(!orbit.zoom_about(focus, 5.0), "it reported a move it did not make");
        assert_eq!(orbit.focus_ly, focus);
        assert!(orbit.distance_m() < 40.0 * M_PER_AU, "but it still zoomed");
    }

    /// Where a point lands on screen, as a fraction of the viewport. Perspective divide and no
    /// more, which is all the assertion above needs.
    fn screen_of(orbit: &Orbit, plane: Plane, at_ly: DVec3) -> glam::DVec2 {
        let (forward, right, up) = orbit.view_basis(plane);
        let offset = at_ly - orbit.eye_ly(plane);
        let depth = offset.dot(forward);
        glam::DVec2::new(offset.dot(right) / depth, offset.dot(up) / depth)
    }

    /// A pan stays in the plane. Otherwise sliding across a map would walk it off the surface
    /// its own rings are drawn on.
    #[test]
    fn a_pan_stays_in_the_plane() {
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            let mut orbit = Orbit::framing(DVec3::ZERO, M_PER_AU);
            orbit.turn(1.1, 0.2);
            let before = orbit.focus_ly;
            orbit.pan(plane, 0.3, -0.7);
            assert!(plane.height_m(orbit.focus_ly, before).abs() < 1.0, "{plane:?} left the plane");
            assert!(orbit.focus_ly != before);
        }
    }

    /// The eye is where the distance says it is, and the camera looks back at the focus.
    #[test]
    fn the_eye_stands_off_by_the_distance() {
        let orbit = Orbit::framing(DVec3::new(2.0, -1.0, 0.5), 3.0 * M_PER_AU);
        for plane in [Plane::Ecliptic, Plane::Galactic] {
            let eye = orbit.eye_ly(plane);
            let span = eye.distance(orbit.focus_ly) * M_PER_LY;
            assert!((span / (3.0 * M_PER_AU) - 1.0).abs() < 1e-9, "{span} m");
            // Loosely, and the reason is worth knowing: `orientation` is built from the two
            // angles and never forms `focus - eye`, so it is the *accurate* one. This
            // reference is the lossy one — subtracting two light-year positions three
            // astronomical units apart cancels five digits — and holding it to `1e-12` would
            // be asserting that the worse answer is exact.
            let (forward, _) = orbit.orientation(plane);
            let toward = (orbit.focus_ly - eye).normalize();
            assert!((forward - toward).length() < 1e-9, "the camera is not looking at its focus");
        }
    }

    #[test]
    fn the_stand_off_is_clamped_at_both_ends() {
        let mut orbit = Orbit::default();
        orbit.zoom(1.0e6);
        assert_eq!(orbit.log_distance_m, LOG_MIN_M);
        orbit.zoom(-1.0e6);
        assert_eq!(orbit.log_distance_m, LOG_MAX_M);

        orbit.set_distance_m(0.0);
        assert_eq!(orbit.log_distance_m, LOG_MIN_M, "a nonsense distance is not a NaN");
        orbit.set_distance_m(f64::NAN);
        assert!(orbit.distance_m().is_finite());
    }
}
