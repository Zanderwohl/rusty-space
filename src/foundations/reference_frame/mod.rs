pub mod conversions;
pub mod transformation;
pub mod observation;

use bevy::math::{DMat4, DQuat, DVec3};
use transformation::Transformation;


/// Right-handed, Z-up coordinate system.
///
/// Axis conventions:
/// - +X is forward
/// - +Y is right
/// - +Z is up
///
/// Angle conventions (Euler angles):
/// - Yaw: rotation around Z axis (0 = facing +X, π/2 = facing +Y)
/// - Pitch: angle from XY plane toward Z (-π/2 = down, 0 = horizontal, +π/2 = up)
/// - Roll: rotation around the forward (X) axis
#[derive(Clone)]
pub struct ReferenceFrame {
    pub(in crate::foundations::reference_frame) mat: DMat4,
}

impl ReferenceFrame {
    #[inline]
    pub fn new(translation: DVec3, rotation: DQuat) -> Self {
        DMat4::from_rotation_translation(rotation, translation).into()
    }

    pub const IDENTITY: ReferenceFrame = ReferenceFrame {
        mat: DMat4::IDENTITY,
    };

    /// Creates a reference frame from position and yaw/pitch angles (roll = 0).
    ///
    /// - Yaw: rotation around Z axis (0 = facing +X, π/2 = facing +Y)
    /// - Pitch: angle from XY plane toward Z (-π/2 = down, 0 = horizontal, +π/2 = up)
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

    /// Computes the orthonormal basis vectors from yaw and pitch (roll = 0).
    /// Returns (forward, right, up) vectors.
    fn basis_from_yaw_pitch(yaw: f64, pitch: f64) -> (DVec3, DVec3, DVec3) {
        // Forward direction from yaw and pitch
        let forward = DVec3::new(
            yaw.cos() * pitch.cos(),
            yaw.sin() * pitch.cos(),
            pitch.sin(),
        ).normalize();

        // Right is perpendicular to forward and world up
        let world_up = DVec3::Z;
        let right = forward.cross(world_up).normalize();

        // Local up is perpendicular to both forward and right
        let up = right.cross(forward).normalize();

        (forward, right, up)
    }

    /// Produces a transformation from Reference Frame self to Reference frame other.
    ///
    /// Points in self's local coordinates are transformed to other's local coordinates.
    /// When `other` is `IDENTITY`, this transforms local points to world coordinates.
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

    /// Local forward direction (+X axis in local coordinates).
    #[inline]
    pub fn local_forward(&self) -> DVec3 {
        self.x_axis()
    }

    /// Local right direction (+Y axis in local coordinates).
    #[inline]
    pub fn local_right(&self) -> DVec3 {
        self.y_axis()
    }

    /// Local up direction (+Z axis in local coordinates).
    #[inline]
    pub fn local_up(&self) -> DVec3 {
        self.z_axis()
    }

    /// The coordinates of the origin, in the universal reference frame.
    #[inline]
    pub fn universal_origin(&self) -> DVec3 {
        self.mat.w_axis.truncate()
    }

    /// Alias for `universal_origin()`.
    #[inline]
    pub fn position(&self) -> DVec3 {
        self.universal_origin()
    }

    /// Sets the position, preserving orientation.
    #[inline]
    pub fn set_position(&mut self, position: DVec3) {
        self.mat.w_axis = position.extend(1.0);
    }

    /// Returns a new frame with the given position, preserving orientation.
    #[inline]
    pub fn with_position(mut self, position: DVec3) -> Self {
        self.set_position(position);
        self
    }

    /// Translates the frame by a vector in universal (world) coordinates.
    #[inline]
    pub fn translate(&mut self, delta: DVec3) {
        let new_pos = self.position() + delta;
        self.set_position(new_pos);
    }

    /// Returns a new frame translated by a vector in universal coordinates.
    #[inline]
    pub fn translated(mut self, delta: DVec3) -> Self {
        self.translate(delta);
        self
    }

    /// Translates the frame by a vector in local coordinates.
    ///
    /// For example, `translate_local(DVec3::new(1.0, 0.0, 0.0))` moves forward,
    /// and `translate_local(DVec3::new(0.0, 1.0, 0.0))` moves right.
    #[inline]
    pub fn translate_local(&mut self, local_delta: DVec3) {
        let world_delta = self.local_forward() * local_delta.x
            + self.local_right() * local_delta.y
            + self.local_up() * local_delta.z;
        self.translate(world_delta);
    }

    /// Returns a new frame translated by a vector in local coordinates.
    #[inline]
    pub fn translated_local(mut self, local_delta: DVec3) -> Self {
        self.translate_local(local_delta);
        self
    }

    /// Extracts the yaw angle (rotation around Z axis).
    ///
    /// Returns the angle in radians where 0 = facing +X, π/2 = facing +Y.
    #[inline]
    pub fn yaw(&self) -> f64 {
        let forward = self.local_forward();
        forward.y.atan2(forward.x)
    }

    /// Extracts the pitch angle (angle from XY plane toward Z).
    ///
    /// Returns the angle in radians where -π/2 = down, 0 = horizontal, +π/2 = up.
    #[inline]
    pub fn pitch(&self) -> f64 {
        let forward = self.local_forward();
        let horizontal_len = (forward.x * forward.x + forward.y * forward.y).sqrt();
        forward.z.atan2(horizontal_len)
    }

    /// Sets the yaw angle, preserving position and pitch (sets roll to 0).
    #[inline]
    pub fn set_yaw(&mut self, yaw: f64) {
        let pitch = self.pitch();
        let position = self.position();
        *self = Self::from_position_yaw_pitch(position, yaw, pitch);
    }

    /// Returns a new frame with the given yaw, preserving position and pitch.
    #[inline]
    pub fn with_yaw(mut self, yaw: f64) -> Self {
        self.set_yaw(yaw);
        self
    }

    /// Sets the pitch angle, preserving position and yaw (sets roll to 0).
    #[inline]
    pub fn set_pitch(&mut self, pitch: f64) {
        let yaw = self.yaw();
        let position = self.position();
        *self = Self::from_position_yaw_pitch(position, yaw, pitch);
    }

    /// Returns a new frame with the given pitch, preserving position and yaw.
    #[inline]
    pub fn with_pitch(mut self, pitch: f64) -> Self {
        self.set_pitch(pitch);
        self
    }

    /// Sets both yaw and pitch angles, preserving position (sets roll to 0).
    #[inline]
    pub fn set_yaw_pitch(&mut self, yaw: f64, pitch: f64) {
        let position = self.position();
        *self = Self::from_position_yaw_pitch(position, yaw, pitch);
    }

    /// Returns a new frame with the given yaw and pitch, preserving position.
    #[inline]
    pub fn with_yaw_pitch(mut self, yaw: f64, pitch: f64) -> Self {
        self.set_yaw_pitch(yaw, pitch);
        self
    }

    /// Adjusts yaw by the given delta (additive).
    #[inline]
    pub fn rotate_yaw(&mut self, delta_yaw: f64) {
        self.set_yaw(self.yaw() + delta_yaw);
    }

    /// Adjusts pitch by the given delta (additive).
    #[inline]
    pub fn rotate_pitch(&mut self, delta_pitch: f64) {
        self.set_pitch(self.pitch() + delta_pitch);
    }

    /// A vector in the universal frame pointing from self's origin to other's origin.
    #[inline]
    pub fn vector_to(&self, other: Self) -> DVec3 {
        other.universal_origin() - self.universal_origin()
    }

    /// A lerped point between self's origin and other's origin.
    #[inline]
    pub fn lerp_origin(&self, other: Self, amount: f64) -> DVec3 {
        self.universal_origin().lerp(other.universal_origin(), amount)
    }

    /// Look at a point in self's reference frame.
    #[inline]
    pub fn look_at(&self, target: DVec3) -> Self {
        let transformation = self.transform_to(DMat4::IDENTITY.into());
        let universal_target = transformation.point(target);
        self.look_at_universal(universal_target, self.z_axis()) // z up hint will be orthagonalized to new direction, making it slightly inconsistent
    }

    /// Look at a point in the global reference frame.
    /// universal +z is used as the up hint.
    pub fn look_at_universal(&self, universal_target: DVec3, up: DVec3) -> Self {
        let forward = (universal_target - self.universal_origin()).normalize(); // +X
        let right = forward.cross(up).normalize(); // +Y
        let actual_up = right.cross(forward).normalize(); // +Z

        DMat4::from_cols(
            forward.extend(0.0),
            right.extend(0.0),
            actual_up.extend(0.0),
            self.universal_origin().extend(1.0),
        ).into()
    }

    /// Look at a point in self's reference frame, rolling to an angle.
    pub fn look_at_roll_rads(&self, target: DVec3, angle_rad: f64) -> Self {
        let transformation = self.transform_to(DMat4::IDENTITY.into());
        let universal_target = transformation.point(target);
        self.look_at_universal_roll_rads(universal_target, angle_rad)
    }

    /// Look at a point in the universal reference frame, rolling to an angle.
    pub fn look_at_universal_roll_rads(&self, universal_target: DVec3, angle_rad: f64) -> Self {
        let to_target = (universal_target - self.universal_origin()).normalize();
        let base_rotation = DQuat::from_rotation_arc(DVec3::X, to_target);
        let roll_rotation = DQuat::from_axis_angle(to_target, angle_rad);
        let final_rotation = roll_rotation * base_rotation;
        DMat4::from_rotation_translation(final_rotation, self.universal_origin()).into()
    }
}
