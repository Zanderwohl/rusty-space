//! Putting a signal out, and where it goes.
//!
//! Two questions, and the second one is the interesting half. *How far* a transmission carries
//! is the inverse square everything else already uses. *Who is inside it* is geometry, and the
//! geometry is what makes a tight beam a game mechanic rather than a flag: a beam is a cone of
//! diffraction-limited width, so at interstellar range a radio "tight beam" still floods the
//! whole target system and everyone loitering in it. Covertness is bought with wavelength and
//! aperture, and the numbers say how much. See `lightcone/docs/05-observation.md`.
//!
//! Engine-free and stateless, like the rest of this crate: the server schedules deliveries with
//! it and the client explains an aim with it, and they must agree.

use glam::DVec3;

/// What a craft transmits with: a wavelength and a dish to launch it from.
///
/// Both are needed and neither is a tier. Beamwidth is `lambda / D`, so a shorter wavelength
/// and a wider dish buy the same thing — a tighter beam — and a design that offered "narrow"
/// and "wide" would be hiding the trade rather than posing it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transmitter {
    /// Meters. 0.03 m is 10 GHz, inside the free-space microwave window.
    pub wavelength_m: f64,
    /// The launching aperture's diameter, meters.
    pub aperture_m: f64,
}

impl Transmitter {
    /// What a crewed ship carries: a thirty-meter dish at 10 GHz, which is a milliradian.
    ///
    /// The numbers are the second row of the table in `lightcone/docs/05-observation.md`, and
    /// they are the ones that make the point: a milliradian across four light-years is a spot
    /// 250 AU wide. Aiming at somebody in another system illuminates their whole system.
    pub const SHIP: Self = Self {
        wavelength_m: 0.03,
        aperture_m: 30.0,
    };

    /// Half the diffraction-limited beamwidth, radians.
    ///
    /// Halved because [`Beam`] is a cone measured from its axis and `lambda / D` is the full
    /// width. Clamped to a hemisphere: a dish smaller than its own wavelength does not radiate
    /// backwards however the arithmetic reads.
    pub fn half_angle_rad(&self) -> f64 {
        let full = self.wavelength_m / self.aperture_m.max(f64::MIN_POSITIVE);
        (full * 0.5).clamp(0.0, std::f64::consts::FRAC_PI_2)
    }

    /// The spot the beam makes at `distance`, across, in whatever unit `distance` is in.
    pub fn spot_at(&self, distance: f64) -> f64 {
        2.0 * distance * self.half_angle_rad().tan()
    }
}

/// A transmission's shape: which way it was pointed, and how wide the cone is.
///
/// Isotropic is [`Beam::OMNI`] rather than a variant beside this one, because a sphere is a
/// cone of half-angle `pi` and every rule below then has one form. A special case for "in every
/// direction" is a second code path to get the gain wrong in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Beam {
    /// Unit vector the beam was pointed along, at the moment it left.
    pub axis: DVec3,
    /// Half-angle of the cone, radians.
    pub half_angle_rad: f64,
}

impl Beam {
    /// In every direction. Cheap to send, heard by everyone in range, and it announces where
    /// you are to every one of them.
    pub const OMNI: Self = Self {
        axis: DVec3::X,
        half_angle_rad: std::f64::consts::PI,
    };

    pub fn along(axis: DVec3, half_angle_rad: f64) -> Self {
        Self {
            axis: axis.normalize_or(DVec3::X),
            half_angle_rad: half_angle_rad.clamp(0.0, std::f64::consts::PI),
        }
    }

    pub fn is_omni(&self) -> bool {
        self.half_angle_rad >= std::f64::consts::PI
    }

    /// Whether a receiver `offset` from where the beam left is inside the cone.
    ///
    /// A receiver at the transmitter itself is inside anything: the offset has no direction to
    /// compare, and a signal you are standing in is one you hear.
    pub fn covers(&self, offset: DVec3) -> bool {
        let Some(toward) = offset.try_normalize() else {
            return true;
        };
        toward.dot(self.axis) >= self.half_angle_rad.cos()
    }

    /// How much louder the cone is than the same power spread over a sphere.
    ///
    /// `4 pi` over the cone's solid angle `2 pi (1 - cos theta)`, written as
    /// `1 / sin^2(theta/2)` — the same number, and the form that survives a narrow beam.
    /// `1 - cos theta` cancels: at a milliradian it keeps ten digits, and at the microradian
    /// an optical link makes it keeps four.
    pub fn gain(&self) -> f64 {
        let s = (self.half_angle_rad * 0.5).sin();
        if s <= 0.0 { f64::INFINITY } else { s.powi(-2) }
    }
}

/// Where to point, to hit a craft that is where it was last seen to be and coasting.
///
/// The **advanced**-time solve, the mirror of the retarded one the rest of the game runs on:
/// find `t_a` with `t_a = t_send + |x_target(t_a) - x_send|`, where `x_target` is the sender's
/// *prediction*. It is built from a sighting, which is already old, and extrapolated forward
/// by the flight time, which is longer still — so a beam aimed across four light-years rests on
/// eight years of guesswork and a quarry that maneuveres in the meantime is missed. That is the
/// mechanic. Stations are easy to hit; ships under thrust are not.
///
/// Every position is light-microseconds and every time is microseconds, so `c = 1` and `beta`
/// is light-microseconds per microsecond. `None` when the prediction cannot be closed on — a
/// target the sender believes is receding at or past `c`, which nothing sub-luminal is.
pub fn aim_at(
    from: DVec3,
    t_send_us: f64,
    seen_at: DVec3,
    beta: DVec3,
    seen_t_us: f64,
) -> Option<DVec3> {
    // Where the prediction puts the target at the instant of transmission, relative to here.
    let d0 = seen_at + beta * (t_send_us - seen_t_us) - from;
    let closing = d0.dot(beta);
    let spread = 1.0 - beta.length_squared();
    if spread <= 0.0 {
        return None;
    }
    // The non-negative root of `tau^2 spread - 2 tau closing - |d0|^2 = 0`. The discriminant
    // cannot be negative: `spread` and `|d0|^2` are both non-negative.
    let tau = (closing + (closing * closing + spread * d0.length_squared()).sqrt()) / spread;
    (d0 + beta * tau).try_normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: f64 = 1_000.0;

    #[test]
    fn a_ship_dish_is_a_milliradian_and_floods_a_system_at_four_light_years() {
        let full = Transmitter::SHIP.half_angle_rad() * 2.0;
        assert!((full - 1.0e-3).abs() < 1.0e-9, "{full}");
        // Light-years across, at four of them. The doc says 250 AU; an AU is 1/63241 of a
        // light-year, so a quarter of a thousandth of one is about 250 AU.
        let spot_ly = Transmitter::SHIP.spot_at(4.0);
        let spot_au = spot_ly * 63_241.0;
        assert!((spot_au - 253.0).abs() < 5.0, "{spot_au} AU");
    }

    /// The gain is what the cone buys, and the exact identity is worth pinning because the
    /// obvious spelling of it loses ten digits at the width a real dish makes.
    #[test]
    fn gain_is_four_over_theta_squared_for_a_narrow_beam_and_one_for_a_sphere() {
        assert_eq!(Beam::OMNI.gain(), 1.0);
        let narrow = Beam::along(DVec3::X, 1.0e-3);
        assert!(
            (narrow.gain() - 4.0e6).abs() / 4.0e6 < 1.0e-6,
            "{}",
            narrow.gain()
        );
        // An optical link is a microradian, and there the canceling form has lost four
        // decimal places. This is why the identity is spelled the way it is.
        let optical = Beam::along(DVec3::X, 1.0e-6);
        assert!(
            (optical.gain() - 4.0e12).abs() / 4.0e12 < 1.0e-9,
            "{}",
            optical.gain()
        );
        let naive = 2.0 / (1.0 - 1.0e-6_f64.cos());
        assert!(
            (naive - optical.gain()).abs() / optical.gain() > 1.0e-5,
            "the trap is real"
        );
    }

    #[test]
    fn a_sphere_covers_every_direction_and_a_cone_does_not() {
        for dir in [DVec3::X, -DVec3::X, DVec3::Y, DVec3::new(1.0, -2.0, 3.0)] {
            assert!(Beam::OMNI.covers(dir), "{dir}");
        }
        let beam = Beam::along(DVec3::X, 0.1);
        assert!(beam.covers(DVec3::X * 5.0));
        assert!(beam.covers(DVec3::new(1.0, 0.05, 0.0)));
        assert!(!beam.covers(DVec3::new(1.0, 0.2, 0.0)));
        assert!(!beam.covers(-DVec3::X));
    }

    /// A still target is aimed at directly: the advanced solve has nothing to lead.
    #[test]
    fn a_still_target_is_aimed_straight_at() {
        let at = DVec3::new(1_000.0 * MS, 0.0, 0.0);
        let axis = aim_at(DVec3::ZERO, 0.0, at, DVec3::ZERO, 0.0).unwrap();
        assert!((axis - DVec3::X).length() < 1.0e-12, "{axis}");
    }

    /// The whole point: the beam leads a crossing target, and lands where it will be.
    #[test]
    fn a_crossing_target_is_led_and_the_light_meets_it() {
        let from = DVec3::ZERO;
        // A light-second away, crossing at a tenth of `c`.
        let seen_at = DVec3::new(1_000_000.0, 0.0, 0.0);
        let beta = DVec3::new(0.0, 0.1, 0.0);
        let axis = aim_at(from, 0.0, seen_at, beta, 0.0).unwrap();
        assert!(axis.y > 0.0, "the beam did not lead: {axis}");

        // March the light out along the axis and the target along its line; they meet.
        let tau = {
            let d0 = seen_at;
            let spread = 1.0 - beta.length_squared();
            (d0.dot(beta) + (d0.dot(beta).powi(2) + spread * d0.length_squared()).sqrt()) / spread
        };
        let light = from + axis * tau;
        let target = seen_at + beta * tau;
        assert!(
            (light - target).length() < 1.0e-6,
            "{light} against {target}"
        );
    }

    /// Aiming at a target running away faster than light is not a solve with a bad answer; it
    /// is a question with none. Nothing sub-luminal reaches it, so it is refused rather than
    /// answered with the direction it happens to be in.
    #[test]
    fn a_target_the_sender_believes_is_superluminal_cannot_be_aimed_at() {
        let axis = aim_at(DVec3::ZERO, 0.0, DVec3::X * MS, DVec3::X * 1.5, 0.0);
        assert_eq!(axis, None);
    }
}
