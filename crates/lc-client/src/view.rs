//! Turning world coordinates into something a GPU can hold.
//!
//! No Bevy here. What to draw is a decision; drawing it is a separate job, and phase 5 found
//! that mixing the two is what makes a renderer unextractable.

use glam::{DVec3, Vec3};

/// Which of the three scale tiers a distance belongs to.
///
/// `f32` has a 24-bit mantissa, so at 1e11 m its spacing is 8 km and absolute world
/// coordinates are unusable. Everything is camera-relative; the tier decides what one render
/// unit means so the numbers that reach the GPU stay small.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleTier {
    /// A ship or a structure.
    Surface,
    /// Planets, orbits, in-system traffic.
    System,
    /// Stars as points.
    Interstellar,
}

impl ScaleTier {
    pub const SURFACE_LIMIT_M: f64 = 1e4;
    pub const SYSTEM_LIMIT_M: f64 = 1e14;

    pub fn for_distance(metres: f64) -> Self {
        let d = metres.abs();
        if d < Self::SURFACE_LIMIT_M {
            Self::Surface
        } else if d < Self::SYSTEM_LIMIT_M {
            Self::System
        } else {
            Self::Interstellar
        }
    }

    /// Metres per render unit.
    pub fn metres_per_unit(&self) -> f64 {
        match self {
            Self::Surface => 1.0,
            Self::System => 1.496e11,          // one AU
            Self::Interstellar => 9.460_730e15, // one light-year
        }
    }
}

/// Reduce a world position to render coordinates relative to the camera.
///
/// Subtracting in `f64` before narrowing is the whole point: doing it the other way loses the
/// difference entirely at system scale, where the two positions agree to more digits than
/// `f32` carries.
pub fn camera_relative(world_m: DVec3, camera_m: DVec3, tier: ScaleTier) -> Vec3 {
    ((world_m - camera_m) / tier.metres_per_unit()).as_vec3()
}

/// How finely retarded time is sampled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sampling {
    /// One solve per drawable.
    PerObject,
    /// One solve for a whole system, shared by everything in it.
    PerSystem,
}

/// Per object where the error would be visible, per system where it would not.
///
/// The observer's own system is where real-time play happens and where light delay is
/// seconds to hours, so it is exact — and cheap, because there are few objects. A distant
/// system is a few pixels across and its internal light-crossing time bounds the error.
pub fn sampling_for(distance_m: f64, own_system: bool) -> Sampling {
    if own_system || distance_m < ScaleTier::SYSTEM_LIMIT_M {
        Sampling::PerObject
    } else {
        Sampling::PerSystem
    }
}

/// Angular error from sharing one retarded time across a system, radians.
///
/// A body moving at `speed` is misplaced by `speed * dt`, and `dt` is bounded by the
/// system's own light-crossing time.
pub fn shared_sampling_error_rad(
    system_radius_m: f64,
    speed_m_s: f64,
    viewing_distance_m: f64,
) -> f64 {
    if viewing_distance_m <= 0.0 {
        return 0.0;
    }
    let crossing_time_s = system_radius_m / 299_792_458.0;
    speed_m_s * crossing_time_s / viewing_distance_m
}

#[cfg(test)]
mod tests {
    use super::*;

    const AU: f64 = 1.496e11;
    const LY: f64 = 9.460_730e15;

    #[test]
    fn tiers_split_where_the_design_says() {
        assert_eq!(ScaleTier::for_distance(500.0), ScaleTier::Surface);
        assert_eq!(ScaleTier::for_distance(AU), ScaleTier::System);
        assert_eq!(ScaleTier::for_distance(30.0 * LY), ScaleTier::Interstellar);
        // And the boundaries themselves.
        assert_eq!(ScaleTier::for_distance(1e4 - 1.0), ScaleTier::Surface);
        assert_eq!(ScaleTier::for_distance(1e4), ScaleTier::System);
        assert_eq!(ScaleTier::for_distance(1e14), ScaleTier::Interstellar);
    }

    /// The reduction exists because narrowing first destroys the difference.
    ///
    /// `f32` spacing at 1 AU is 2^14 metres, about 16 km, so an offset below 8 km vanishes
    /// entirely and a larger one survives only in 16 km steps.
    #[test]
    fn subtracting_before_narrowing_keeps_a_difference_f32_would_lose() {
        let camera = DVec3::new(AU, 0.0, 0.0);
        let narrow_first = |offset: f64| {
            ((camera + DVec3::new(offset, 0.0, 0.0)).as_vec3() - camera.as_vec3()).x as f64
        };

        // A structure a kilometre away is simply gone.
        assert_eq!(narrow_first(1.0e3), 0.0, "this is the failure the reduction avoids");
        // A thousand kilometres survives, quantised to the 16 km grid.
        let coarse = narrow_first(1.0e6);
        assert!(coarse > 0.0 && (coarse - 1.0e6).abs() > 500.0, "quantised to {coarse}");

        // Subtracting in f64 first keeps both exactly.
        for offset in [1.0e3, 1.0e6] {
            let body = camera + DVec3::new(offset, 0.0, 0.0);
            let good = camera_relative(body, camera, ScaleTier::System).x as f64;
            // f32 carries about seven digits, so the tolerance is relative to it.
            assert!((good - offset / AU).abs() < 1e-6 * (offset / AU), "offset {offset}");
        }
    }

    #[test]
    fn render_coordinates_stay_small_in_every_tier() {
        for (tier, distance) in [
            (ScaleTier::Surface, 5e3),
            (ScaleTier::System, 40.0 * AU),
            (ScaleTier::Interstellar, 4000.0 * LY),
        ] {
            let v = camera_relative(DVec3::new(distance, 0.0, 0.0), DVec3::ZERO, tier);
            assert!(v.x.abs() < 1e5, "{tier:?} produced {}", v.x);
            // And it is still precise: f32 holds six digits at this magnitude.
            assert!(v.x.abs() > 1e-3);
        }
    }

    #[test]
    fn sampling_is_per_object_nearby_and_per_system_at_range() {
        assert_eq!(sampling_for(AU, false), Sampling::PerObject);
        assert_eq!(sampling_for(30.0 * LY, true), Sampling::PerObject, "own system is exact");
        assert_eq!(sampling_for(30.0 * LY, false), Sampling::PerSystem);
    }

    /// The bound that makes sharing a retarded time safe.
    #[test]
    fn a_shared_retarded_time_is_sub_pixel_at_range() {
        // A planetary system 100 AU across, a body at 30 km/s, seen from ten light-years.
        let err = shared_sampling_error_rad(100.0 * AU, 30_000.0, 10.0 * LY);
        assert!(err < 1e-6, "angular error {err} rad");
        // Four orders of magnitude under one pixel of a 60-degree field at 1080 lines.
        let pixel = (60.0f64).to_radians() / 1080.0;
        assert!(err * 1e4 < pixel, "error {err} against a pixel of {pixel}");
    }

    #[test]
    fn the_same_error_would_be_visible_inside_the_system() {
        // Which is why the observer's own system is sampled per object instead.
        let err = shared_sampling_error_rad(100.0 * AU, 30_000.0, 2.0 * AU);
        let pixel = (60.0f64).to_radians() / 1080.0;
        assert!(err > pixel, "error {err} would be visible against a pixel of {pixel}");
    }
}
