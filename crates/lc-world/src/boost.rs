//! Changing inertial frames, in the units the rest of the world uses.
//!
//! Seconds and **light-seconds**, so `c` is one and every formula below is the textbook one
//! with no constants in it. Positions elsewhere are light-years; a light-year is
//! [`JULIAN_YEAR_S`] light-seconds, and converting at the boundary is cheaper than carrying a
//! factor through the algebra.
//!
//! A boost is written here as "into the frame of something moving at `beta`", and the inverse
//! is the same call with `-beta`. That is not a convenience: it is the identity that makes the
//! round trip exact, and it is what the tests check.

use glam::DVec3;

use crate::flight::MAX_BETA;

/// The Lorentz factor of a velocity, clamped where `gamma` would stop being a number.
pub fn gamma_of(beta: DVec3) -> f64 {
    let b2 = beta.length_squared().min(MAX_BETA * MAX_BETA);
    (1.0 - b2).sqrt().recip()
}

/// An event, as a displacement from whatever event a frame is pinned to.
///
/// Displacements rather than coordinates, because the origin is always a specific event — the
/// sighting a pursuit is anchored at — and carrying absolute positions through a boost would
/// mean boosting numbers of order a light-year to answer a question about kilometers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    /// Seconds.
    pub t: f64,
    /// Light-seconds.
    pub x: DVec3,
}

/// An event as seen from a frame moving at `beta`.
///
/// `t' = γ(t − β·x)`, `x' = x + ((γ−1)/β² (β·x) − γt) β`.
pub fn to_frame(event: Event, beta: DVec3) -> Event {
    let b2 = beta.length_squared();
    if b2 <= 0.0 {
        return event;
    }
    let gamma = gamma_of(beta);
    let along = beta.dot(event.x);
    Event {
        t: gamma * (event.t - along),
        x: event.x + beta * ((gamma - 1.0) * along / b2 - gamma * event.t),
    }
}

/// The inverse of [`to_frame`], which is the same boost the other way.
pub fn from_frame(event: Event, beta: DVec3) -> Event {
    to_frame(event, -beta)
}

/// A velocity as seen from a frame moving at `beta`.
///
/// `u'_∥ = (u_∥ − β)/(1 − β·u)` and `u'_⊥ = u_⊥/(γ(1 − β·u))`, which is the addition law and
/// not a subtraction: nothing here can be carried past `c` by composing two velocities under
/// it, and a unit vector comes out a unit vector, which is aberration.
pub fn velocity_to_frame(u: DVec3, beta: DVec3) -> DVec3 {
    let b2 = beta.length_squared();
    if b2 <= 0.0 {
        return u;
    }
    let gamma = gamma_of(beta);
    let denominator = 1.0 - beta.dot(u);
    // Head-on at `c` is the one case this vanishes, and the answer there is `c` the other way.
    if denominator.abs() < f64::MIN_POSITIVE {
        return -u;
    }
    let along = beta * (beta.dot(u) / b2);
    let across = u - along;
    ((along - beta) + across / gamma) / denominator
}

/// The inverse of [`velocity_to_frame`].
pub fn velocity_from_frame(u: DVec3, beta: DVec3) -> DVec3 {
    velocity_to_frame(u, -beta)
}

/// How far apart two points are, as the frame moving at `beta` measures it.
///
/// `separation` is the gap at one instant of the *world's* time, so this is the proper
/// distance between two things moving with that frame: stretched by `gamma` along the boost,
/// unchanged across it. The inverse of the contraction, because the world is the frame doing
/// the contracting.
pub fn separation_in_frame(separation: DVec3, beta: DVec3) -> f64 {
    to_frame(Event { t: 0.0, x: separation }, beta).x.length()
}

/// A displacement measured in the moving frame, as the world measures it at one world instant.
///
/// The exact inverse of [`separation_in_frame`]: contracted by `gamma` along the boost and
/// unchanged across it.
///
/// **Not** the spatial part of [`from_frame`] at `t' = 0`, and the difference is the whole
/// reason this is written out. Simultaneous in the frame is not simultaneous in the world, so
/// inverting a separation means transforming the two *worldlines* and differencing them at one
/// world instant — and when you do that, the `γt'` term appears on both sides and cancels
/// symbolically. Canceling it in the algebra rather than in `f64` is what keeps this usable:
/// a chase at `0.99c` puts the frame's anchor event tens of light-years behind, and subtracting
/// two positions of that size to recover a five-kilometer standoff leaves about a hundred bits
/// of nothing.
pub fn separation_in_world(separation: DVec3, beta: DVec3) -> DVec3 {
    let b2 = beta.length_squared();
    if b2 <= 0.0 {
        return separation;
    }
    let gamma = gamma_of(beta);
    separation + beta * (beta.dot(separation) * ((gamma - 1.0) / b2 - gamma))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: DVec3, b: DVec3, tolerance: f64) -> bool {
        (a - b).length() <= tolerance * b.length().max(1.0)
    }

    const FAST: DVec3 = DVec3::new(0.6, 0.0, 0.0);

    /// The round trip, which is the identity everything else here leans on.
    #[test]
    fn a_boost_and_its_inverse_are_the_identity() {
        for beta in [DVec3::ZERO, FAST, DVec3::new(-0.4, 0.5, 0.3), DVec3::new(0.0, 0.0, 0.99)] {
            for event in [
                Event { t: 0.0, x: DVec3::ZERO },
                Event { t: 1_000.0, x: DVec3::new(3.0, -4.0, 5.0) },
                Event { t: -7.5e6, x: DVec3::new(1.0e7, 2.0e6, -3.0e6) },
            ] {
                let there_and_back = from_frame(to_frame(event, beta), beta);
                assert!(
                    (there_and_back.t - event.t).abs() <= 1.0e-9 * event.t.abs().max(1.0),
                    "{beta} took {event:?} to {there_and_back:?}",
                );
                assert!(close(there_and_back.x, event.x, 1.0e-9), "{beta}: {there_and_back:?}");
            }
        }
    }

    /// The interval is what a boost preserves, and checking it is checking the boost is a
    /// boost rather than some other linear map that happens to invert.
    #[test]
    fn the_interval_survives() {
        let event = Event { t: 500.0, x: DVec3::new(120.0, -80.0, 30.0) };
        let interval = |e: Event| e.t * e.t - e.x.length_squared();
        for beta in [FAST, DVec3::new(0.1, -0.7, 0.2), DVec3::new(0.0, 0.95, 0.0)] {
            let moved = to_frame(event, beta);
            let (before, after) = (interval(event), interval(moved));
            assert!((after - before).abs() < 1.0e-6 * before.abs(), "{beta}: {before} to {after}");
        }
    }

    /// Something at rest in the frame is at rest, and something at rest in the world is going
    /// backwards at exactly the frame's own speed.
    #[test]
    fn the_frames_own_velocity_is_what_it_removes() {
        assert!(close(velocity_to_frame(FAST, FAST), DVec3::ZERO, 1.0e-12));
        assert!(close(velocity_to_frame(DVec3::ZERO, FAST), -FAST, 1.0e-12));
        assert!(close(velocity_from_frame(DVec3::ZERO, FAST), FAST, 1.0e-12));
    }

    /// **Nothing composes past `c`.** The whole reason this exists rather than a subtraction:
    /// two velocities a hair under `c` in opposite directions come out a hair under `c`, where
    /// subtracting would have given nearly twice it.
    #[test]
    fn velocities_compose_rather_than_add() {
        let fast = DVec3::X * 0.9;
        let other = -DVec3::X * 0.9;
        let relative = velocity_to_frame(other, fast);
        assert!(relative.length() < 1.0, "composed to {}", relative.length());
        // The textbook answer for this pair.
        assert!((relative.length() - (1.8 / 1.81)).abs() < 1.0e-12, "{}", relative.length());

        for beta in [0.5, 0.9, 0.99, 0.999] {
            for u in [0.5, 0.9, 0.99, 0.999] {
                let composed = velocity_from_frame(DVec3::X * u, DVec3::X * beta);
                assert!(composed.length() < 1.0, "{beta} and {u} gave {composed}");
            }
        }
    }

    /// A unit vector stays one, which is aberration: a direction swings forward rather than
    /// changing length.
    #[test]
    fn a_direction_is_aberrated_and_not_stretched() {
        // A direction already along the boost has nowhere to swing, so it is checked for
        // length alone; the others must actually move.
        for direction in [DVec3::Y, DVec3::new(0.6, 0.8, 0.0), -DVec3::X] {
            let seen = velocity_to_frame(direction, FAST);
            assert!((seen.length() - 1.0).abs() < 1.0e-12, "{direction} became {seen}");
            if direction.cross(FAST).length() > 0.0 {
                assert!(seen.angle_between(direction) > 0.1, "{direction} barely swung");
            }
        }
    }

    /// Velocity round-trips too, and at speeds where a Galilean version is nowhere near.
    #[test]
    fn a_velocity_and_its_inverse_are_the_identity() {
        for beta in [FAST, DVec3::new(-0.2, 0.9, 0.1)] {
            for u in [DVec3::ZERO, DVec3::new(0.3, -0.4, 0.1), DVec3::new(0.0, 0.0, -0.8)] {
                assert!(close(velocity_from_frame(velocity_to_frame(u, beta), beta), u, 1.0e-9));
            }
        }
    }

    /// Two things moving together are further apart in their own frame than the world says.
    #[test]
    fn a_separation_is_longer_in_the_frame_that_is_moving() {
        let gamma = gamma_of(FAST);
        // Along the boost: stretched by gamma, which is the inverse of the contraction the
        // world sees.
        let along = separation_in_frame(DVec3::X * 100.0, FAST);
        assert!((along - 100.0 * gamma).abs() < 1.0e-9, "{along}");
        // Across it: untouched.
        let across = separation_in_frame(DVec3::Y * 100.0, FAST);
        assert!((across - 100.0).abs() < 1.0e-9, "{across}");
    }

    /// The two separations are inverses, which is what says the algebraic cancellation is the
    /// same map as the honest round trip and not merely close to it.
    #[test]
    fn a_separation_goes_into_a_frame_and_back_out() {
        for beta in [DVec3::ZERO, FAST, DVec3::new(-0.3, 0.6, 0.5), DVec3::new(0.0, 0.0, 0.999)] {
            for gap in [DVec3::X * 100.0, DVec3::new(3.0, -4.0, 12.0), DVec3::Y * 1.0e-5] {
                let out = separation_in_world(
                    to_frame(Event { t: 0.0, x: gap }, beta).x,
                    beta,
                );
                assert!(close(out, gap, 1.0e-12), "{beta} took {gap} to {out}");
            }
        }
    }

    /// Along the boost it is the contraction, and across it there is none.
    #[test]
    fn a_separation_in_the_world_is_the_contracted_one() {
        let gamma = gamma_of(FAST);
        let along = separation_in_world(DVec3::X * 100.0, FAST);
        assert!((along.x - 100.0 / gamma).abs() < 1.0e-9, "{along}");
        let across = separation_in_world(DVec3::Y * 100.0, FAST);
        assert!((across.y - 100.0).abs() < 1.0e-9, "{across}");
    }

    /// At everyday speeds it has to agree with the arithmetic it replaces, or every number
    /// tuned against the old one is now wrong.
    #[test]
    fn it_is_the_galilean_answer_when_nothing_is_fast() {
        let slow = DVec3::new(1.0e-5, -2.0e-6, 0.0);
        let u = DVec3::new(3.0e-6, 1.0e-6, -4.0e-7);
        let relative = velocity_to_frame(u, slow);
        assert!(close(relative, u - slow, 1.0e-8), "{relative} against {}", u - slow);
        let gap = separation_in_frame(DVec3::X * 1.0e4, slow);
        assert!((gap - 1.0e4).abs() < 1.0e-4, "{gap}");
    }
}
