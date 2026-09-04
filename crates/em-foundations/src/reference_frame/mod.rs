pub mod conversions;
pub mod transformation;
pub mod observation;

use glam::{DMat4, DQuat, DVec3};
use transformation::Transformation;


/// A position and orientation in simulation space.
///
/// Simulation space is right-handed and **Z-up**: ecliptic of J2000, +X toward the
/// vernal equinox. The only frame `em-foundations` and `em-sim` use; Y-up renderers
/// convert at their own boundary (`presentation::render_space` in the app crate).
///
/// - +X forward, +Y right, +Z up.
/// - Yaw about Z (0 = +X, π/2 = +Y); pitch from the XY plane toward Z (±π/2 = down/up);
///   roll about X.
#[derive(Clone)]
pub struct ReferenceFrame {
    pub(in crate::reference_frame) mat: DMat4,
}

impl ReferenceFrame {
    #[inline]
    pub fn new(translation: DVec3, rotation: DQuat) -> Self {
        DMat4::from_rotation_translation(rotation, translation).into()
    }

    pub const IDENTITY: ReferenceFrame = ReferenceFrame {
        mat: DMat4::IDENTITY,
    };

    /// From position and yaw/pitch, radians, roll = 0.
    #[inline]
    pub fn from_position_yaw_pitch(position: DVec3, yaw: f64, pitch: f64) -> Self {
        let (forward, right, up) = Self::basis_from_yaw_pitch(yaw, pitch);
        DMat4::from_cols(
            forward.extend(0.0),
            right.extend(0.0),
            up.extend(0.0),
            position.extend(1.0),
        ).into()
    }

    /// Orthonormal (forward, right, up) from yaw and pitch, right-handed (det = +1).
    ///
    /// Cross-product order matters: `world_up x forward` gives +Y right; the reverse
    /// gives det = -1, a mirror that cannot become a quaternion.
    fn basis_from_yaw_pitch(yaw: f64, pitch: f64) -> (DVec3, DVec3, DVec3) {
        let forward = DVec3::new(
            yaw.cos() * pitch.cos(),
            yaw.sin() * pitch.cos(),
            pitch.sin(),
        ).normalize();

        let world_up = DVec3::Z;
        let right = Self::right_from_forward(forward, world_up, yaw);
        let up = forward.cross(right).normalize();

        (forward, right, up)
    }

    /// `right = world_up x forward`, falling back to a yaw-derived axis at
    /// pitch = ±π/2, where the cross product vanishes and `normalize()` gives NaN.
    fn right_from_forward(forward: DVec3, world_up: DVec3, yaw: f64) -> DVec3 {
        let right = world_up.cross(forward);
        if right.length_squared() < 1e-20 {
            // Gimbal singularity: use the right vector implied by yaw alone.
            DVec3::new(-yaw.sin(), yaw.cos(), 0.0)
        } else {
            right.normalize()
        }
    }

    /// Transformation taking points from self's local coordinates to `other`'s.
    /// With `other = IDENTITY`, local to world.
    #[inline]
    pub fn transform_to(&self, other: Self) -> Transformation {
        (other.mat.inverse() * self.mat).into()
    }

    #[inline]
    pub fn x_axis(&self) -> DVec3 {
        self.mat.x_axis.truncate()
    }

    #[inline]
    pub fn y_axis(&self) -> DVec3 {
        self.mat.y_axis.truncate()
    }

    #[inline]
    pub fn z_axis(&self) -> DVec3 {
        self.mat.z_axis.truncate()
    }

    #[inline]
    pub fn local_forward(&self) -> DVec3 {
        self.x_axis()
    }

    #[inline]
    pub fn local_right(&self) -> DVec3 {
        self.y_axis()
    }

    #[inline]
    pub fn local_up(&self) -> DVec3 {
        self.z_axis()
    }

    /// The origin, in universal coordinates.
    #[inline]
    pub fn universal_origin(&self) -> DVec3 {
        self.mat.w_axis.truncate()
    }

    #[inline]
    pub fn position(&self) -> DVec3 {
        self.universal_origin()
    }

    #[inline]
    pub fn set_position(&mut self, position: DVec3) {
        self.mat.w_axis = position.extend(1.0);
    }

    #[inline]
    pub fn with_position(mut self, position: DVec3) -> Self {
        self.set_position(position);
        self
    }

    #[inline]
    pub fn translate(&mut self, delta: DVec3) {
        let new_pos = self.position() + delta;
        self.set_position(new_pos);
    }

    #[inline]
    pub fn translated(mut self, delta: DVec3) -> Self {
        self.translate(delta);
        self
    }

    /// Translates by a local-coordinate vector: +X forward, +Y right, +Z up.
    #[inline]
    pub fn translate_local(&mut self, local_delta: DVec3) {
        let world_delta = self.local_forward() * local_delta.x
            + self.local_right() * local_delta.y
            + self.local_up() * local_delta.z;
        self.translate(world_delta);
    }

    #[inline]
    pub fn translated_local(mut self, local_delta: DVec3) -> Self {
        self.translate_local(local_delta);
        self
    }

    /// Yaw about Z, radians: 0 = +X, π/2 = +Y.
    #[inline]
    pub fn yaw(&self) -> f64 {
        let forward = self.local_forward();
        forward.y.atan2(forward.x)
    }

    /// Pitch from the XY plane toward Z, radians: -π/2 = down, +π/2 = up.
    #[inline]
    pub fn pitch(&self) -> f64 {
        let forward = self.local_forward();
        let horizontal_len = (forward.x * forward.x + forward.y * forward.y).sqrt();
        forward.z.atan2(horizontal_len)
    }

    /// Preserves position and pitch; sets roll to 0.
    #[inline]
    pub fn set_yaw(&mut self, yaw: f64) {
        let pitch = self.pitch();
        let position = self.position();
        *self = Self::from_position_yaw_pitch(position, yaw, pitch);
    }

    #[inline]
    pub fn with_yaw(mut self, yaw: f64) -> Self {
        self.set_yaw(yaw);
        self
    }

    /// Preserves position and yaw; sets roll to 0.
    #[inline]
    pub fn set_pitch(&mut self, pitch: f64) {
        let yaw = self.yaw();
        let position = self.position();
        *self = Self::from_position_yaw_pitch(position, yaw, pitch);
    }

    #[inline]
    pub fn with_pitch(mut self, pitch: f64) -> Self {
        self.set_pitch(pitch);
        self
    }

    /// Preserves position; sets roll to 0.
    #[inline]
    pub fn set_yaw_pitch(&mut self, yaw: f64, pitch: f64) {
        let position = self.position();
        *self = Self::from_position_yaw_pitch(position, yaw, pitch);
    }

    #[inline]
    pub fn with_yaw_pitch(mut self, yaw: f64, pitch: f64) -> Self {
        self.set_yaw_pitch(yaw, pitch);
        self
    }

    #[inline]
    pub fn rotate_yaw(&mut self, delta_yaw: f64) {
        self.set_yaw(self.yaw() + delta_yaw);
    }

    #[inline]
    pub fn rotate_pitch(&mut self, delta_pitch: f64) {
        self.set_pitch(self.pitch() + delta_pitch);
    }

    /// Universal-frame vector from self's origin to other's.
    #[inline]
    pub fn vector_to(&self, other: Self) -> DVec3 {
        other.universal_origin() - self.universal_origin()
    }

    #[inline]
    pub fn lerp_origin(&self, other: Self, amount: f64) -> DVec3 {
        self.universal_origin().lerp(other.universal_origin(), amount)
    }

    /// Look at a point given in self's local frame.
    #[inline]
    pub fn look_at(&self, target: DVec3) -> Self {
        let transformation = self.transform_to(DMat4::IDENTITY.into());
        let universal_target = transformation.point(target);
        self.look_at_universal(universal_target, self.z_axis()) // the up hint is orthogonalised to the new direction
    }

    /// Look at a point in the universal frame.
    pub fn look_at_universal(&self, universal_target: DVec3, up: DVec3) -> Self {
        let forward = (universal_target - self.universal_origin()).normalize(); // +X
        // Right-handed: +Y = up x forward, +Z = forward x right. `forward x up` gives a
        // mirrored (det = -1) basis, disagreeing with `look_at_universal_roll_rads`.
        let right = Self::right_from_forward(forward, up, self.yaw()); // +Y
        let actual_up = forward.cross(right).normalize(); // +Z

        DMat4::from_cols(
            forward.extend(0.0),
            right.extend(0.0),
            actual_up.extend(0.0),
            self.universal_origin().extend(1.0),
        ).into()
    }

    /// Look at a local-frame point, rolling to an angle.
    pub fn look_at_roll_rads(&self, target: DVec3, angle_rad: f64) -> Self {
        let transformation = self.transform_to(DMat4::IDENTITY.into());
        let universal_target = transformation.point(target);
        self.look_at_universal_roll_rads(universal_target, angle_rad)
    }

    /// Look at a universal-frame point, rolling to an angle.
    pub fn look_at_universal_roll_rads(&self, universal_target: DVec3, angle_rad: f64) -> Self {
        let to_target = (universal_target - self.universal_origin()).normalize();
        let base_rotation = DQuat::from_rotation_arc(DVec3::X, to_target);
        let roll_rotation = DQuat::from_axis_angle(to_target, angle_rad);
        let final_rotation = roll_rotation * base_rotation;
        DMat4::from_rotation_translation(final_rotation, self.universal_origin()).into()
    }
}
