//! Where the map is looked at from.

use glam::DVec3;

use crate::plane::Plane;
use crate::snapshot::{M_PER_AU, M_PER_LY};

/// Elevation stops just short of the pole, where the azimuth stops being defined.
///
/// The same limit and the same reason as `lc_client::ui::Look::PITCH_LIMIT`.
pub const ELEVATION_LIMIT: f64 = std::f64::consts::FRAC_PI_2 - 1.0e-3;

/// And the camera is never *in* the plane.
///
/// `AGENTS.md`: an infinitely thin sheet containing the camera swings wildly from frame to
/// frame. Nothing the map draws is a sheet — the rings are tubes with a real radius, which is
/// most of the answer — but a floor costs nothing and is what stops the next person's filled
/// disc from re-teaching it.
///
/// An angle rather than a distance, so the stand-off it buys scales by itself: `d · sin` of
/// this is a fixed fraction of the view at every scale.
///
/// **It has to exceed the angular half-width of a drawn line, and the first value did not.**
/// A line is a tube, and a tube a host draws at 1.6 pixels has an angular radius of about
/// three milliradians from the camera. At a floor of two, the camera cleared the plane by less
/// than that and sat *inside* the nearest ring — and the inside of a tube is a solid wall, so
/// an edge-on map came out as a rectangle of flat green with nothing in it.
///
/// Three degrees. Still edge-on to look at, and a whole order of magnitude clear of any tube
/// the host is likely to draw.
pub const ELEVATION_FLOOR: f64 = 5.0e-2;

/// Decades of stand-off per notch of wheel.
///
/// Zoom is additive in `log10` meters, which is what makes one notch mean the same *fraction*
/// at a hull as at a spiral arm. Seven notches is a decade, and the whole range is about 113 —
/// continuous, where a discrete tier would jump by five orders between two of them and be a
/// control nobody could aim.
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
    /// Light-years from the world origin, simulation axes. What the camera looks at.
    ///
    /// A position, not a thing. Saturn moves; the host holds the selection and writes this
    /// every frame from wherever the selection is now.
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

    /// Turn by a relative amount, radians, and hold both clamps.
    ///
    /// A turn that would carry the elevation through the plane stops at the floor **on the
    /// side it came from**. Letting it cross would teleport the view to the mirror image of
    /// itself in one frame, which reads as the camera jumping rather than as a limit.
    pub fn turn(&mut self, d_azimuth: f64, d_elevation: f64) {
        self.azimuth = (self.azimuth + d_azimuth).rem_euclid(std::f64::consts::TAU);
        let side = if self.elevation < 0.0 { -1.0 } else { 1.0 };
        let next = (self.elevation + d_elevation).clamp(-ELEVATION_LIMIT, ELEVATION_LIMIT);
        // One comparison for both limits: a `next` on the other side of the plane has a
        // negative product, and one too close to it has a small positive product.
        self.elevation = match next * side < ELEVATION_FLOOR {
            true => side * ELEVATION_FLOOR,
            false => next,
        };
    }

    /// Slide the focus across the plane, in fractions of the stand-off.
    ///
    /// Fractions rather than meters, because a drag of so many pixels has to move the view by
    /// the same part of itself at every zoom. In meters it is imperceptible at a light-year
    /// and throws the system off screen at a kilometer.
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
    /// Up is the **plane's** normal, not `+Z`. That is what makes the plane toggle tilt the
    /// whole view, which is the point of having a toggle. It is well conditioned because
    /// [`ELEVATION_LIMIT`] keeps the forward vector off the normal.
    pub fn orientation(&self, plane: Plane) -> (DVec3, DVec3) {
        (-self.offset_direction(plane), plane.normal())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One notch is the same fraction wherever it is spent.
    ///
    /// Break it by subtracting meters instead of decades and the ratio at a hull and the ratio
    /// at a spiral arm stop agreeing, which is the whole difference between a usable zoom and
    /// one that is dead at one end.
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
    /// Every third turn is aimed *straight at zero*, which is what a hand dragging the view
    /// flat does and what a drifting sweep never quite manages. Written the obvious way — a
    /// thousand sine-driven deltas — this passed with the floor deleted, because none of them
    /// happened to land on it. Break the floor now and it fails on the first aimed step.
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

    /// A turn that would cross the plane stops on the side it came from.
    #[test]
    fn a_turn_does_not_jump_the_plane() {
        let mut orbit = Orbit::framing(DVec3::ZERO, M_PER_AU);
        orbit.elevation = 0.5;
        orbit.turn(0.0, -10.0);
        assert!(orbit.elevation > 0.0, "crossed to {}", orbit.elevation);

        orbit.elevation = -0.5;
        orbit.turn(0.0, 10.0);
        assert!(orbit.elevation < 0.0, "crossed to {}", orbit.elevation);
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
