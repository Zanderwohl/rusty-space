//! Worldlines, and the retarded-time solve that is the centre of the whole design.

use glam::DVec3;
use smallvec::SmallVec;

use crate::coord::Coord;

/// An object's position as a function of server-frame coordinate time.
///
/// Working units are **microseconds and light-microseconds**, so `c = 1` and a velocity is
/// dimensionless `beta`.
///
/// The signature is the hard constraint of the design, in two ways.
///
/// It must be evaluable at *arbitrary past* time, because solving for retarded time visits
/// times that are not the current tick and were not necessarily visited in order. That
/// forbids storing motion only as an integrator state; a worldline is a closed form or a
/// stored spline.
///
/// It is also total and single-valued in `t`, which is what makes closed causal loops
/// unrepresentable: nothing can move backward in coordinate time, so no effect can be placed
/// before its cause. That property is free and is worth not losing.
pub trait Worldline {
    /// Position in light-microseconds, in the frame the caller is solving in.
    fn position_at(&self, t: f64) -> DVec3;

    /// Velocity as `beta`, dimensionless. `|velocity_at| < 1` for every sub-luminal
    /// worldline.
    fn velocity_at(&self, t: f64) -> DVec3;

    /// The inclusive range of coordinate time over which this worldline is defined.
    fn defined_over(&self) -> (f64, f64);

    /// Every worldline in the current design returns `true`. The single-root invariant of
    /// [`retarded_times`] is asserted against this rather than assumed globally, so that the
    /// day it stops holding, the affected code is already identified.
    fn is_subluminal(&self) -> bool {
        true
    }
}

/// Solve `t_r + |x_o - w(t_r)| = t_o` for the emission times whose light reaches an observer
/// at `observer`.
///
/// Returns a collection rather than an `Option`. For a sub-luminal worldline it always holds
/// zero or one root and the caller pays nothing for the generality; the signature is chosen
/// now because changing it later would touch every call site, and superluminal motion makes
/// the count zero, one or more. See `lightcone/docs/10-superluminal.md`.
///
/// Empty means the light has not arrived yet, or has already passed, or the worldline is not
/// defined over the interval that would have emitted it.
pub fn retarded_times(observer: Coord, w: &dyn Worldline) -> SmallVec<[f64; 2]> {
    retarded_times_at(observer.time_f64(), observer.position(), w)
}

/// [`retarded_times`] in the continuous domain, for solving inside a system where local
/// precision beats the 300 m grid.
pub fn retarded_times_at(t_o: f64, x_o: DVec3, w: &dyn Worldline) -> SmallVec<[f64; 2]> {
    let mut out = SmallVec::new();

    if !w.is_subluminal() {
        debug_assert!(
            false,
            "superluminal worldlines fold f(t) and need root isolation; not supported yet"
        );
        return out;
    }

    let (t0, t1) = w.defined_over();
    let mut hi = t1.min(t_o);
    if hi < t0 || !hi.is_finite() {
        return out;
    }

    // f(t) = t + |x_o - w(t)| - t_o, strictly increasing for |v| < 1, so exactly one root.
    let f = |t: f64| t + (x_o - w.position_at(t)).length() - t_o;

    let f_hi = f(hi);
    if f_hi < 0.0 {
        return out; // all of this worldline's light has already gone past.
    }
    if f_hi == 0.0 {
        out.push(hi);
        return out;
    }

    // An unbounded worldline has no finite lower bracket to start from, so walk one back.
    // f falls without bound at rate (1 - beta), so doubling terminates in O(log) steps; the
    // first guess is the light travel time from the source's position at `hi`, which is the
    // answer for a source that is not moving.
    let mut lo = t0;
    if !lo.is_finite() {
        let mut step = (x_o - w.position_at(hi)).length().max(1.0);
        lo = hi - step;
        let mut guard = 0;
        while f(lo) > 0.0 {
            step *= 2.0;
            lo = hi - step;
            guard += 1;
            debug_assert!(guard <= 256, "bracket expansion failed; is the worldline sub-luminal?");
            if guard > 256 {
                return out;
            }
        }
    }

    let f_lo = f(lo);
    if f_lo > 0.0 {
        return out; // the earliest light has not arrived yet.
    }
    if f_lo == 0.0 {
        out.push(lo);
        return out;
    }

    // Safeguarded Newton: take the Newton step when it stays in the bracket, bisect when it
    // does not. Newton alone can leave the bracket near a shallow derivative; bisection alone
    // takes 50 iterations to reach f64 precision.
    let mut t = 0.5 * (lo + hi);
    for _ in 0..96 {
        let diff = x_o - w.position_at(t);
        let r = diff.length();
        let ft = t + r - t_o;

        if ft > 0.0 {
            hi = t;
        } else if ft < 0.0 {
            lo = t;
        } else {
            break;
        }

        // f'(t) = 1 - n . v, with n the unit vector from source to observer.
        let deriv = if r > 0.0 { 1.0 - (diff / r).dot(w.velocity_at(t)) } else { 1.0 };

        let mut next = if deriv.abs() > 1e-15 { t - ft / deriv } else { f64::NAN };
        if !(next > lo && next < hi) {
            next = 0.5 * (lo + hi);
        }

        // Converged once the bracket is at the resolution f64 can represent here.
        let tol = 4.0 * f64::EPSILON * t.abs().max(1.0);
        if (next - t).abs() <= tol || (hi - lo) <= tol {
            t = next;
            break;
        }
        t = next;
    }

    out.push(t);
    out
}

/// A worldline that does not move.
#[derive(Debug, Clone, Copy)]
pub struct Static {
    pub position: DVec3,
}

impl Static {
    pub fn new(position: DVec3) -> Self {
        Self { position }
    }
}

impl Worldline for Static {
    fn position_at(&self, _t: f64) -> DVec3 {
        self.position
    }
    fn velocity_at(&self, _t: f64) -> DVec3 {
        DVec3::ZERO
    }
    fn defined_over(&self) -> (f64, f64) {
        (f64::NEG_INFINITY, f64::INFINITY)
    }
}

/// Constant velocity. `velocity` is `beta`: light-microseconds per microsecond.
#[derive(Debug, Clone, Copy)]
pub struct Inertial {
    pub origin: DVec3,
    pub velocity: DVec3,
    /// Coordinate time at which the object is at `origin`.
    pub epoch: f64,
}

impl Inertial {
    pub fn new(origin: DVec3, velocity: DVec3, epoch: f64) -> Self {
        Self { origin, velocity, epoch }
    }
}

impl Worldline for Inertial {
    fn position_at(&self, t: f64) -> DVec3 {
        self.origin + self.velocity * (t - self.epoch)
    }
    fn velocity_at(&self, _t: f64) -> DVec3 {
        self.velocity
    }
    fn defined_over(&self) -> (f64, f64) {
        (f64::NEG_INFINITY, f64::INFINITY)
    }
    fn is_subluminal(&self) -> bool {
        self.velocity.length_squared() < 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Micros;

    fn observer(t: i64, x: i64, y: i64, z: i64) -> Coord {
        Coord::new(Micros::new(t), x, y, z).unwrap()
    }

    #[test]
    fn a_static_source_is_seen_at_exactly_its_light_travel_time_ago() {
        let w = Static::new(DVec3::new(1000.0, 0.0, 0.0));
        let roots = retarded_times(observer(5000, 0, 0, 0), &w);
        assert_eq!(roots.len(), 1);
        assert!((roots[0] - 4000.0).abs() < 1e-6, "got {}", roots[0]);
    }

    #[test]
    fn an_observer_on_top_of_a_static_source_sees_it_now() {
        let w = Static::new(DVec3::ZERO);
        let roots = retarded_times(observer(7, 0, 0, 0), &w);
        assert_eq!(roots.len(), 1);
        assert!((roots[0] - 7.0).abs() < 1e-9);
    }

    /// The closed form for a source crossing at constant beta with perpendicular distance b,
    /// closest approach at t = 0. Squaring gives
    ///   (beta^2 - 1) t^2 + 2 t_o t + (b^2 - t_o^2) = 0
    /// and for beta < 1 exactly one of the two roots satisfies the unsquared equation.
    fn closed_form_root(beta: f64, b: f64, t_o: f64) -> f64 {
        let a = beta * beta - 1.0;
        let disc = beta * beta * t_o * t_o - a * b * b;
        let r = disc.sqrt();
        let (r1, r2) = ((-t_o + r) / a, (-t_o - r) / a);
        let valid =
            |t: f64| t <= t_o && (t + ((beta * t).powi(2) + b * b).sqrt() - t_o).abs() < 1e-6;
        if valid(r1) { r1 } else { r2 }
    }

    #[test]
    fn a_moving_source_matches_the_closed_form() {
        let b = 100.0;
        for &beta in &[0.0, 0.1, 0.5, 0.9, 0.99] {
            for &t_o in &[0.0, 50.0, 150.0, 1_000.0, 1e6] {
                let w = Inertial::new(DVec3::new(0.0, b, 0.0), DVec3::new(beta, 0.0, 0.0), 0.0);
                let roots = retarded_times_at(t_o, DVec3::ZERO, &w);
                assert_eq!(roots.len(), 1, "beta={beta} t_o={t_o}");
                let expect = closed_form_root(beta, b, t_o);
                let tol = 1e-6 * expect.abs().max(1.0);
                assert!(
                    (roots[0] - expect).abs() < tol,
                    "beta={beta} t_o={t_o}: solver {} closed form {expect}",
                    roots[0]
                );
            }
        }
    }

    #[test]
    fn every_subluminal_worldline_has_exactly_one_root() {
        // A cheap deterministic sweep rather than a random one, so a failure is reproducible.
        let mut seed = 0x5eed_1234_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        for _ in 0..2000 {
            let beta_mag = next() * 0.95;
            let dir = DVec3::new(next() - 0.5, next() - 0.5, next() - 0.5).normalize();
            let origin = DVec3::new(next() - 0.5, next() - 0.5, next() - 0.5) * 1e5;
            let w = Inertial::new(origin, dir * beta_mag, 0.0);
            let t_o = (next() - 0.2) * 1e6;
            let x_o = DVec3::new(next() - 0.5, next() - 0.5, next() - 0.5) * 1e5;
            let roots = retarded_times_at(t_o, x_o, &w);
            assert_eq!(roots.len(), 1, "an infinite sub-luminal worldline always has one root");
            // And it satisfies the equation it was asked to solve.
            let t_r = roots[0];
            let residual = t_r + (x_o - w.position_at(t_r)).length() - t_o;
            let scale = t_o.abs().max(1e5);
            assert!(residual.abs() < 1e-6 * scale, "residual {residual} at scale {scale}");
            assert!(t_r <= t_o + 1e-9, "emission must not be after reception");
        }
    }

    #[test]
    fn a_bounded_worldline_reports_no_root_outside_its_span() {
        let w = BoundedStatic { position: DVec3::new(1000.0, 0.0, 0.0), span: (0.0, 10.0) };
        // Light emitted in [0, 10] arrives in [1000, 1010].
        assert!(retarded_times_at(500.0, DVec3::ZERO, &w).is_empty(), "not arrived yet");
        assert!(retarded_times_at(5000.0, DVec3::ZERO, &w).is_empty(), "already gone past");
        assert_eq!(retarded_times_at(1005.0, DVec3::ZERO, &w).len(), 1);
    }

    struct BoundedStatic {
        position: DVec3,
        span: (f64, f64),
    }
    impl Worldline for BoundedStatic {
        fn position_at(&self, _t: f64) -> DVec3 {
            self.position
        }
        fn velocity_at(&self, _t: f64) -> DVec3 {
            DVec3::ZERO
        }
        fn defined_over(&self) -> (f64, f64) {
            self.span
        }
    }
}
