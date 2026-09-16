//! The last burn of a crossing, when there is a station to join rather than a stop to make.
//!
//! Split from [`crate::flight`] for room, and it earns the separation: everything here is about
//! one burn held at one angle, and nothing in it knows what a brachistochrone is.

use glam::DVec3;

/// The fastest a crossing may be going when its last burn begins, for that burn to be an
/// [`Injection`].
///
/// The linear ramp below is what constant proper acceleration does while `gamma` is near one:
/// the coordinate rate is `alpha / gamma^3`, so at this speed the burn runs about four parts in
/// a thousand slow and everything downstream of it by the same. Above it the crossing brakes to
/// rest the exact way instead, and picks the station's velocity up on arrival as it always did.
///
/// A transfer about one primary cannot reach this. Falling the length of Jupiter's Hill sphere
/// at five gravities peaks at half a per cent of `c`, and crossing thirty astronomical units
/// peaks at five — which is the whole solar system, and the edge of what this is offered for.
pub const INJECTION_MAX_BETA: f64 = 0.05;

/// The last burn of a crossing: one burn, held at one angle, that kills the speed the ship came
/// in with and gives it the speed it is joining.
///
/// A station is an orbit and an orbit moves, so arriving at one is not arriving at rest. The
/// crossing used to stop dead at the injection point and pick the orbit's velocity up for
/// nothing — kilometres a second, appearing between two samples. This is that velocity being
/// paid for, and paid for in *one* burn aimed at the difference of the two rather than in a
/// brake followed by a second burn across it. The ship turns once, to the angle that does both
/// jobs at once.
///
/// **Newtonian, and only offered where that is true.** The velocity is taken to ramp linearly
/// from one end to the other, which is what a constant proper acceleration does only near
/// `gamma = 1`. See [`INJECTION_MAX_BETA`], and [`Cruise::plan_onto`], which refuses the form
/// above it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Injection {
    from_beta: DVec3,
    to_beta: DVec3,
    aim: DVec3,
    duration_s: f64,
}

impl Injection {
    /// The burn that takes a ship from one velocity to another at `alpha`.
    pub fn new(alpha: f64, from_beta: DVec3, to_beta: DVec3) -> Self {
        let change = to_beta - from_beta;
        Self {
            from_beta,
            to_beta,
            aim: change.normalize_or_zero(),
            duration_s: change.length() / alpha,
        }
    }

    pub fn duration_s(&self) -> f64 {
        self.duration_s
    }

    /// The one angle the whole burn is held at: what kills the incoming velocity and imparts
    /// the one being joined, added together.
    pub fn aim(&self) -> DVec3 {
        self.aim
    }

    pub fn arrive_beta(&self) -> DVec3 {
        self.to_beta
    }

    /// Ground the whole burn covers, light-seconds. The mean of the two velocities times the
    /// time it takes, which is exact for a velocity that ramps linearly and is why the line
    /// above has to be aimed short of the target by this much.
    pub fn displacement_ls(&self) -> DVec3 {
        (self.from_beta + self.to_beta) * 0.5 * self.duration_s
    }

    /// How far into the burn it has got at `t`: ground covered since it began, and how fast.
    pub fn at(&self, t: f64) -> (DVec3, DVec3) {
        let t = t.clamp(0.0, self.duration_s);
        let beta = self.beta_at(t);
        ((self.from_beta + beta) * 0.5 * t, beta)
    }

    fn beta_at(&self, t: f64) -> DVec3 {
        if self.duration_s <= 0.0 {
            return self.to_beta;
        }
        self.from_beta.lerp(self.to_beta, t / self.duration_s)
    }

    /// Ship seconds over the first `t` of the burn.
    ///
    /// The midpoint rule rather than the integral. `sqrt(1 - beta^2)` over a linear ramp does
    /// have a closed form and it is not worth writing: at the speeds this form is allowed at
    /// the whole dilation is parts in a thousand, and the midpoint's error is parts in a
    /// million of that.
    pub fn proper_s(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, self.duration_s);
        let middle = self.beta_at(t * 0.5).length_squared().min(1.0);
        t * (1.0 - middle).sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::Drive;

    /// The burn's own closed form: a velocity that ramps linearly covers the mean of its two
    /// ends times the time it takes, and arrives on the second of them.
    #[test]
    fn an_injection_covers_the_mean_of_its_two_velocities() {
        let alpha = Drive::DEFAULT.alpha();
        let (from, to) = (DVec3::X * 1.0e-4, DVec3::Y * 1.0e-5);
        let burn = Injection::new(alpha, from, to);
        assert!((burn.duration_s() - (to - from).length() / alpha).abs() < 1.0e-9);
        let (ran, beta) = burn.at(burn.duration_s());
        assert!((beta - to).length() < 1.0e-18, "{beta:?}");
        assert!((ran - burn.displacement_ls()).length() < 1.0e-18);
        assert!(
            (burn.displacement_ls() - (from + to) * 0.5 * burn.duration_s()).length() < 1.0e-18
        );
        // Halfway through is halfway between, and half the ground is not covered by then —
        // the ship is slowing, so the first half of the burn covers more than the second.
        let (half_ran, half_beta) = burn.at(burn.duration_s() * 0.5);
        assert!((half_beta - (from + to) * 0.5).length() < 1.0e-18);
        assert!(half_ran.length() > burn.displacement_ls().length() * 0.5);
        // And the crew ages a shade less than the clock, never more.
        let aboard = burn.proper_s(burn.duration_s());
        assert!(aboard < burn.duration_s() && aboard > burn.duration_s() * 0.999);
    }
}
