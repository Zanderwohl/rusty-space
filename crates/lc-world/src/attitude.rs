//! Which way a craft points, and how long it takes to point somewhere else.
//!
//! A ship cannot thrust in a direction it is not facing, so every burn is really a turn and
//! then a burn. At the rates this game runs at the turn is usually over before anyone could
//! see it — a five-hundred-meter hull flips in a minute of coordinate time, which is
//! milliseconds of real time at the design clock — and it is here anyway, because "the drive
//! points wherever the trajectory needs it to, instantly" is the kind of small lie that other
//! things end up built on.
//!
//! **Closed form, like everything else that moves.** A turn is not integrated: it is where a
//! rotation has got to by a given time, from a known start, at a known rate. Two clients
//! stepping at different frame rates therefore agree, which stepping an angle each frame would
//! not.

use glam::{DQuat, DVec3};

/// How fast a hull of [`REFERENCE_LENGTH_M`] turns, radians a second.
///
/// A flip in a minute, which is brisk for something the size of a large building and is meant
/// to be: these are torch ships, and a drive that can pull five gravities is attached to
/// attitude control that can keep up with it.
pub const RATE_RAD_S: f64 = std::f64::consts::PI / 60.0;

/// The hull [`RATE_RAD_S`] is quoted for, meters.
pub const REFERENCE_LENGTH_M: f64 = 500.0;

/// How fast a hull of `length_m` can turn, radians a second.
///
/// Inversely with length, which is not a taste. Attitude thrusters scale with the area they
/// are mounted on and their moment arm with the hull, so torque goes as the cube; the moment of
/// inertia is mass times a length squared, and mass is already cubic, so it goes as the fifth
/// power. Angular acceleration is the ratio, `1/L²`, and the *time* to swing through a fixed
/// angle at that acceleration goes as `L` — so the rate goes as `1/L`.
///
/// A fifty-kilometer ship therefore turns a hundred times slower than a five-hundred-meter one:
/// a flip takes it the better part of two hours.
pub fn rate_rad_s(length_m: f64) -> f64 {
    if length_m <= 0.0 {
        return RATE_RAD_S;
    }
    RATE_RAD_S * REFERENCE_LENGTH_M / length_m
}

/// How long a hull turning at `rate_rad_s` takes to swing end for end, seconds.
///
/// The half-turn is the expensive one and the one a crossing has to make room for, so it has a
/// name of its own: [`crate::flight::Cruise`] coasts for at least this long between the boost
/// and the brake, because a ship that has not finished turning cannot brake.
pub fn flip_time_s(rate_rad_s: f64) -> f64 {
    if rate_rad_s <= 0.0 || !rate_rad_s.is_finite() {
        return 0.0;
    }
    std::f64::consts::PI / rate_rad_s
}

/// The angle between two directions, radians. Zero if either is nothing.
pub fn angle_between(from: DVec3, to: DVec3) -> f64 {
    let (a, b) = (from.normalize_or_zero(), to.normalize_or_zero());
    if a == DVec3::ZERO || b == DVec3::ZERO {
        return 0.0;
    }
    a.dot(b).clamp(-1.0, 1.0).acos()
}

/// How long it takes to swing from one direction to another, seconds.
pub fn turn_time_s(from: DVec3, to: DVec3, rate_rad_s: f64) -> f64 {
    if rate_rad_s <= 0.0 {
        return 0.0;
    }
    angle_between(from, to) / rate_rad_s
}

/// Where the nose has got to, `elapsed_s` into a turn from `from` toward `to`.
///
/// Arrives and stays: past the time the turn takes, this is `to` exactly rather than something
/// that overshot and came back.
///
/// The half-turn is the case worth knowing about. A flip is exactly antipodal, which leaves the
/// axis undetermined — there is no "shortest way round" when every way round is the same length
/// — so the rotation picks one and is consistent about it. Which way a ship rolls through its
/// flip is arbitrary; that it takes the same time whichever way is not.
pub fn turned(from: DVec3, to: DVec3, rate_rad_s: f64, elapsed_s: f64) -> DVec3 {
    let (a, b) = (from.normalize_or_zero(), to.normalize_or_zero());
    if a == DVec3::ZERO {
        return b;
    }
    if b == DVec3::ZERO {
        return a;
    }
    let whole = turn_time_s(a, b, rate_rad_s);
    if whole <= 0.0 || elapsed_s >= whole {
        return b;
    }
    if elapsed_s <= 0.0 {
        return a;
    }
    let arc = DQuat::from_rotation_arc(a, b);
    (DQuat::IDENTITY.slerp(arc, elapsed_s / whole) * a).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f64 = RATE_RAD_S;

    /// A flip is half a turn, and at the rated speed it takes the minute it is quoted at.
    #[test]
    fn a_flip_takes_the_time_it_says() {
        let seconds = turn_time_s(DVec3::X, -DVec3::X, RATE);
        assert!((seconds - 60.0).abs() < 1.0e-9, "{seconds} s");
        // And a right angle is half of that.
        assert!((turn_time_s(DVec3::X, DVec3::Y, RATE) - 30.0).abs() < 1.0e-9);
        // Going nowhere takes no time at all.
        assert_eq!(turn_time_s(DVec3::X, DVec3::X, RATE), 0.0);
    }

    /// A turn arrives and stays there. Asked about a time past the end it must not be
    /// somewhere it swung through on the way.
    #[test]
    fn a_turn_arrives_and_stops() {
        let (from, to) = (DVec3::X, DVec3::Y);
        let whole = turn_time_s(from, to, RATE);
        assert!((turned(from, to, RATE, whole) - to).length() < 1.0e-12);
        assert!((turned(from, to, RATE, whole * 10.0) - to).length() < 1.0e-12);
        assert!((turned(from, to, RATE, 0.0) - from).length() < 1.0e-12);
        assert!((turned(from, to, RATE, -5.0) - from).length() < 1.0e-12);
    }

    /// Halfway through is halfway round, and it stays a direction the whole way.
    #[test]
    fn a_turn_sweeps_at_a_steady_rate() {
        let (from, to) = (DVec3::X, DVec3::Y);
        let whole = turn_time_s(from, to, RATE);
        for k in 0..=10 {
            let part = k as f64 / 10.0;
            let nose = turned(from, to, RATE, whole * part);
            assert!((nose.length() - 1.0).abs() < 1.0e-9, "{nose} is not a direction");
            let swept = angle_between(from, nose);
            let want = angle_between(from, to) * part;
            assert!((swept - want).abs() < 1.0e-9, "at {part} it had swept {swept}, wanted {want}");
        }
    }

    /// The antipodal case, which has no shortest way round. Any axis will do; taking the whole
    /// time to get there will not be negotiated.
    #[test]
    fn a_half_turn_still_goes_all_the_way_round() {
        let (from, to) = (DVec3::X, -DVec3::X);
        let whole = turn_time_s(from, to, RATE);
        let middle = turned(from, to, RATE, whole * 0.5);
        assert!((middle.length() - 1.0).abs() < 1.0e-9, "{middle}");
        // Square on to both ends, which is what halfway through a half-turn means.
        assert!(middle.dot(from).abs() < 1.0e-9, "{middle} is not square to {from}");
        assert!((turned(from, to, RATE, whole) - to).length() < 1.0e-9);
    }

    /// A bigger ship is a slower ship, and by the length rather than by the mass — which is
    /// cubic, and would make a fifty-kilometer hull a million times more ponderous instead of a
    /// hundred.
    #[test]
    fn a_longer_hull_turns_more_slowly() {
        let small = rate_rad_s(500.0);
        let large = rate_rad_s(50_000.0);
        assert!((small / large - 100.0).abs() < 1.0e-9, "{small} against {large}");
        assert!((small - RATE_RAD_S).abs() < 1.0e-12, "the reference hull turns at the rate");
        // And the big one's flip is the better part of two hours.
        let flip = turn_time_s(DVec3::X, -DVec3::X, large);
        assert!(flip > 5_000.0 && flip < 7_200.0, "{flip} s");
    }

    /// The named half-turn and the general one are the same turn.
    #[test]
    fn a_flip_is_a_half_turn_by_another_name() {
        for length_m in [500.0, 5_000.0, 50_000.0] {
            let rate = rate_rad_s(length_m);
            let named = flip_time_s(rate);
            assert!((named - turn_time_s(DVec3::X, -DVec3::X, rate)).abs() < 1.0e-9, "{named} s");
        }
        // A hull that cannot turn is not one that turns instantly, but nothing may divide by it.
        assert_eq!(flip_time_s(0.0), 0.0);
        assert_eq!(flip_time_s(f64::INFINITY), 0.0);
    }

    /// Nothing is not a direction, and asking about one must not produce a broken vector.
    #[test]
    fn a_turn_from_or_to_nothing_is_not_a_turn() {
        assert_eq!(turned(DVec3::ZERO, DVec3::Y, RATE, 1.0), DVec3::Y);
        assert_eq!(turned(DVec3::X, DVec3::ZERO, RATE, 1.0), DVec3::X);
        assert_eq!(angle_between(DVec3::ZERO, DVec3::Y), 0.0);
    }
}
