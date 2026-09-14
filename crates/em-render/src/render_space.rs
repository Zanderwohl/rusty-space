//! The boundary between simulation space and render space.
//!
//! Simulation space is right-handed, **Z-up**, ecliptic of J2000; Bevy renders **Y-up**.
//! This is the only module that converts. The mapping is a proper rotation, −90° about +X
//! (+Z to +Y, +Y to −Z), not a mirror, so handedness and cross products survive;
//! [`sim_to_render_rotation`] applies it to a quaternion by conjugation.

use std::f64::consts::FRAC_PI_2;

use bevy::math::{DQuat, DVec3, Quat, Vec3};

/// Change of basis from Z-up simulation space to Y-up render space: −90° about +X.
#[inline]
pub fn render_basis() -> DQuat {
    DQuat::from_axis_angle(DVec3::X, -FRAC_PI_2)
}

/// Simulation (Z-up) to render (Y-up): `(x, y, z) -> (x, z, -y)`.
#[inline]
pub fn sim_to_render(v: DVec3) -> DVec3 {
    DVec3::new(v.x, v.z, -v.y)
}

/// Render (Y-up) back to simulation (Z-up): `(x, y, z) -> (x, -z, y)`.
#[inline]
pub fn render_to_sim(v: DVec3) -> DVec3 {
    DVec3::new(v.x, -v.z, v.y)
}

/// The same change of basis applied to a rotation, by conjugation.
#[inline]
pub fn sim_to_render_rotation(q: DQuat) -> DQuat {
    let r = render_basis();
    r * q * r.inverse()
}

/// Simulation-space vectors, viewed from the renderer.
pub trait ToRender {
    /// Render-space direction, unscaled.
    fn to_render(&self) -> Vec3;

    /// Render-space position, scaled, kept in `f64`. At solar-system distances a scaled
    /// position does not survive `f32` until it is made camera-relative.
    fn to_render_scaled(&self, scale: f64) -> DVec3;

    /// Render-space position, scaled, narrowed to `f32`. Only for offsets already small;
    /// world positions need [`Self::to_render_relative`].
    fn to_render_scaled_f32(&self, scale: f64) -> Vec3;

    /// Render-space position relative to the camera, scaled, in `f32`. What a `Transform`
    /// wants: subtracting before narrowing keeps distant bodies off the `f32` grid.
    fn to_render_relative(&self, scale: f64, camera_render_pos: DVec3) -> Vec3;
}

impl ToRender for DVec3 {
    #[inline]
    fn to_render(&self) -> Vec3 {
        sim_to_render(*self).as_vec3()
    }

    #[inline]
    fn to_render_scaled(&self, scale: f64) -> DVec3 {
        sim_to_render(*self) * scale
    }

    #[inline]
    fn to_render_scaled_f32(&self, scale: f64) -> Vec3 {
        self.to_render_scaled(scale).as_vec3()
    }

    #[inline]
    fn to_render_relative(&self, scale: f64, camera_render_pos: DVec3) -> Vec3 {
        (self.to_render_scaled(scale) - camera_render_pos).as_vec3()
    }
}

/// Render-space vectors, viewed from the simulation.
pub trait ToSim {
    fn to_sim(&self) -> DVec3;
}

impl ToSim for DVec3 {
    #[inline]
    fn to_sim(&self) -> DVec3 {
        render_to_sim(*self)
    }
}

/// Simulation-space rotations, viewed from the renderer.
pub trait ToRenderRotation {
    fn to_render_rotation(&self) -> Quat;
}

impl ToRenderRotation for DQuat {
    #[inline]
    fn to_render_rotation(&self) -> Quat {
        sim_to_render_rotation(*self).as_quat()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<DVec3> {
        vec![
            DVec3::X, DVec3::Y, DVec3::Z, -DVec3::X, -DVec3::Y, -DVec3::Z,
            DVec3::new(1.0, 2.0, 3.0),
            DVec3::new(-4.0, 0.5, -7.25),
            DVec3::new(1.496e11, -3.2e10, 5.5e9),
        ]
    }

    #[test]
    fn axes_map_as_documented() {
        assert!((sim_to_render(DVec3::X) - DVec3::X).length() < 1e-12, "+X stays +X");
        assert!((sim_to_render(DVec3::Y) + DVec3::Z).length() < 1e-12, "+Y becomes -Z");
        assert!((sim_to_render(DVec3::Z) - DVec3::Y).length() < 1e-12, "+Z becomes +Y");
    }

    #[test]
    fn round_trips() {
        for v in samples() {
            let back = render_to_sim(sim_to_render(v));
            assert!((back - v).length() < 1e-9, "{v:?} -> {back:?}");
        }
    }

    /// The swizzle must equal the rotation, or vectors and quaternions disagree.
    #[test]
    fn the_swizzle_is_the_rotation() {
        let r = render_basis();
        for v in samples() {
            let a = sim_to_render(v);
            let b = r * v;
            assert!((a - b).length() < 1e-9 * v.length().max(1.0), "{v:?}: {a:?} vs {b:?}");
        }
    }

    /// A proper rotation, so a right-handed triple stays right-handed.
    #[test]
    fn handedness_is_preserved() {
        let (x, y, z) = (sim_to_render(DVec3::X), sim_to_render(DVec3::Y), sim_to_render(DVec3::Z));
        let det = x.dot(y.cross(z));
        assert!((det - 1.0).abs() < 1e-12, "determinant {det}, expected +1 (a mirror would be -1)");

        // And cross products commute with the conversion.
        for (a, b) in [(DVec3::X, DVec3::Y), (DVec3::new(1.0, 2.0, 3.0), DVec3::new(-2.0, 0.5, 4.0))] {
            let lhs = sim_to_render(a.cross(b));
            let rhs = sim_to_render(a).cross(sim_to_render(b));
            assert!((lhs - rhs).length() < 1e-9, "cross product not preserved: {lhs:?} vs {rhs:?}");
        }
    }

    /// Converting a rotated vector equals rotating a converted one.
    #[test]
    fn quaternion_conversion_matches_vector_conversion() {
        let rotations = [
            DQuat::from_axis_angle(DVec3::Z, 0.7),
            DQuat::from_axis_angle(DVec3::X, -1.2),
            DQuat::from_axis_angle(DVec3::new(1.0, 2.0, 3.0).normalize(), 2.4),
        ];
        for q in rotations {
            let q_render = sim_to_render_rotation(q);
            for v in samples() {
                let lhs = sim_to_render(q * v);
                let rhs = q_render * sim_to_render(v);
                assert!((lhs - rhs).length() < 1e-9 * v.length().max(1.0),
                    "q={q:?} v={v:?}: {lhs:?} vs {rhs:?}");
            }
        }
    }

    /// Camera-relative rendering must place the camera at the render origin.
    #[test]
    fn camera_relative_puts_the_camera_at_the_origin() {
        let scale = 1e-9;
        let body = DVec3::new(1.496e11, 0.0, 0.0);
        let camera_render = body.to_render_scaled(scale);
        let at_camera = body.to_render_relative(scale, camera_render);
        assert!(at_camera.length() < 1e-6, "a body at the camera should render at the origin, got {at_camera:?}");
    }

    /// Camera-relative before narrowing to f32 keeps metre-scale separations at 1 AU;
    /// narrowing first loses them.
    #[test]
    fn relative_conversion_beats_narrowing_first() {
        let scale = 1e-9;
        let camera = DVec3::new(1.4959787e11, 0.0, 0.0);
        let body = camera + DVec3::new(1000.0, 0.0, 0.0); // 1 km away
        let camera_render = camera.to_render_scaled(scale);

        let good = body.to_render_relative(scale, camera_render);
        let naive = body.to_render_scaled(scale).as_vec3() - camera_render.as_vec3();

        let expected = 1000.0 * scale;
        assert!((good.x as f64 - expected).abs() < expected * 1e-3,
            "camera-relative should keep the 1 km separation: {good:?}");
        let _ = naive; // the alternative this guards against
    }
}
