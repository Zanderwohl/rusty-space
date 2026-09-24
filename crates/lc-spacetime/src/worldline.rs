//! Worldlines, and the retarded-time solve that is the center of the whole design.

use glam::DVec3;
use smallvec::SmallVec;

use crate::coord::Coord;

/// An object's position as a function of server-frame coordinate time, in microseconds and
/// light-microseconds.
///
/// Two constraints ride on this signature. It must be evaluable at *arbitrary past* time,
/// because retarded-time solving visits times out of order — so motion may not be stored as
/// an integrator state, only as a closed form or a spline. And being total and single-valued
/// in `t` is what makes causal loops unrepresentable rather than merely checked for.
pub trait Worldline {
    /// Position in light-microseconds, in the frame the caller is solving in.
    fn position_at(&self, t: f64) -> DVec3;

    /// Velocity as `beta`, dimensionless.
    fn velocity_at(&self, t: f64) -> DVec3;

    /// The inclusive range of coordinate time over which this worldline is defined.
    fn defined_over(&self) -> (f64, f64);

    /// Everything returns `true` today. [`retarded_times`] asserts its single-root invariant
    /// against this rather than assuming it, so the affected code is already identified.
    fn is_subluminal(&self) -> bool {
        true
    }

    /// A ball containing this worldline over `[t0, t1]`: a center and a radius, both in
    /// light-microseconds.
    ///
    /// Used to bound arrival times without solving for them, so it must *contain* the motion —
    /// a ball that is too small silently drops what it excludes. The default is the only bound
    /// that needs nothing but `c`: over a span the worldline cannot leave a ball of that span's
    /// own radius. An implementation that knows its own shape should give a tighter one, and a
    /// loose bound costs only traversal that turns out to be unnecessary.
    ///
    /// A jump is not bounded by `c`, so across a break the default is one such ball per piece,
    /// merged.
    fn bounding_ball(&self, t0: f64, t1: f64) -> (DVec3, f64) {
        let mut ball: Option<(DVec3, f64)> = None;
        for (a, b) in pieces(self, t0, t1) {
            let piece = (self.position_at(a), (b - a).max(0.0));
            ball = Some(match ball {
                None => piece,
                Some((center, radius)) => {
                    (center, radius.max(center.distance(piece.0) + piece.1))
                }
            });
        }
        ball.unwrap_or((self.position_at(t0), (t1 - t0).max(0.0)))
    }

    /// Coordinate times at which this worldline jumps, ascending. At a break it already holds
    /// its new position.
    ///
    /// Across a jump the retarded equation steps, and a solve over it converges onto the step:
    /// an image no light left from. The solvers split here, so each piece has at most one root.
    fn breaks(&self) -> SmallVec<[f64; 2]> {
        SmallVec::new()
    }
}

/// `[t0, t1]` split at `w`'s breaks. Each piece but the last ends just short of the break that
/// closes it, so it reads the position the worldline jumped *from*.
fn pieces<W: Worldline + ?Sized>(w: &W, t0: f64, t1: f64) -> SmallVec<[(f64, f64); 3]> {
    let mut out = SmallVec::new();
    let mut lo = t0;
    for at in w.breaks() {
        if at > t1 {
            break;
        }
        if at > lo {
            out.push((lo, just_before(at)));
            lo = at;
        }
    }
    out.push((lo, t1));
    out
}

/// Far enough below `t` that a caller converting microseconds to seconds still lands on the
/// earlier side of it, and a nanosecond at the least: no position moves measurably in that.
fn just_before(t: f64) -> f64 {
    t - (t.abs() * 8.0 * f64::EPSILON).max(1.0e-3)
}

/// Solve `t_a - |w(t_a) - x_e| = t_e` for when an event's light reaches a worldline.
///
/// The mirror of [`retarded_times`]: there the observer is fixed and the source moves, here the
/// event is fixed and the observer moves. `g(t) = t - t_e - |w(t) - x_e|` rises at
/// `1 - n . v >= 1 - |v| > 0` for anything sub-luminal, so there is at most one root, and
/// `None` means the light never reaches this worldline while it is defined.
///
/// Across a break the earliest piece the light lands in wins. An observer that jumps ahead of
/// a wavefront meets it a second time, and that second arrival is not another delivery.
pub fn arrival_time_at(t_e: f64, x_e: DVec3, w: &dyn Worldline) -> Option<f64> {
    if !w.is_subluminal() {
        debug_assert!(false, "superluminal worldlines fold g(t); not supported yet");
        return None;
    }
    let (t0, t1) = w.defined_over();
    // Nothing before the event can receive it, so the search starts at the event's own time.
    let mut lo = t0.max(t_e);
    if lo > t1 || !lo.is_finite() && t0.is_finite() {
        return None;
    }
    if !lo.is_finite() {
        lo = t_e;
    }
    pieces(w, lo, t1).into_iter().find_map(|(lo, hi)| arrival_within(t_e, x_e, w, lo, hi))
}

/// [`arrival_time_at`] over one continuous piece, `[lo, hi]`.
fn arrival_within(t_e: f64, x_e: DVec3, w: &dyn Worldline, mut lo: f64, hi: f64) -> Option<f64> {
    let g = |t: f64| t - t_e - (w.position_at(t) - x_e).length();

    let g_lo = g(lo);
    if g_lo > 0.0 {
        // The worldline starts already inside the event's future cone: the light passed before
        // this observer existed.
        return None;
    }
    if g_lo == 0.0 {
        return Some(lo);
    }

    // Walk out a bracket. `g` rises at least at rate `1 - |v|`, so doubling terminates; an
    // unbounded worldline needs it because there is no finite upper limit to start from.
    let mut hi = hi;
    if !hi.is_finite() {
        let mut step = (w.position_at(lo) - x_e).length().max(1.0);
        hi = lo + step;
        let mut guard = 0;
        while g(hi) < 0.0 {
            step *= 2.0;
            hi = lo + step;
            guard += 1;
            debug_assert!(guard <= 256, "bracket expansion failed; is the worldline sub-luminal?");
            if guard > 256 {
                return None;
            }
        }
    }
    let g_hi = g(hi);
    if g_hi < 0.0 {
        return None; // still on its way when the worldline ends.
    }
    if g_hi == 0.0 {
        return Some(hi);
    }

    // Safeguarded Newton, for the same reason `retarded_times_at` uses one: Newton alone can
    // leave the bracket where the derivative is shallow, and bisection alone is fifty
    // iterations to f64 precision.
    let mut t = 0.5 * (lo + hi);
    for _ in 0..96 {
        let diff = w.position_at(t) - x_e;
        let r = diff.length();
        let gt = t - t_e - r;
        if gt > 0.0 {
            hi = t;
        } else if gt < 0.0 {
            lo = t;
        } else {
            break;
        }
        let deriv = if r > 0.0 { 1.0 - (diff / r).dot(w.velocity_at(t)) } else { 1.0 };
        let mut next = if deriv.abs() > 1e-15 { t - gt / deriv } else { f64::NAN };
        if !(next > lo && next < hi) {
            next = 0.5 * (lo + hi);
        }
        let tol = 4.0 * f64::EPSILON * t.abs().max(1.0);
        if (next - t).abs() <= tol || (hi - lo) <= tol {
            t = next;
            break;
        }
        t = next;
    }
    Some(t)
}

/// [`arrival_time_at`] for an event given as a grid coordinate.
pub fn arrival_time(event: Coord, w: &dyn Worldline) -> Option<f64> {
    arrival_time_at(event.time_f64(), event.position(), w)
}

/// Solve `t_r + |x_o - w(t_r)| = t_o` for the emission times whose light reaches `observer`.
///
/// A collection rather than an `Option`: a sub-luminal worldline gives zero or one root per
/// continuous piece, but superluminal motion folds `f` and gives zero, one or more, and
/// changing the signature later would touch every call site. See
/// `lightcone/docs/10-superluminal.md`.
///
/// Empty means the light has not arrived, has already passed, or was never emitted.
///
/// Ascending, so the last is the newest light. A worldline with breaks gives up to one root
/// per piece.
pub fn retarded_times(observer: Coord, w: &dyn Worldline) -> SmallVec<[f64; 2]> {
    retarded_times_at(observer.time_f64(), observer.position(), w)
}

/// [`retarded_times`] in the continuous domain, where local precision beats the 300 m grid.
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
    let hi = t1.min(t_o);
    if hi < t0 || !hi.is_finite() {
        return out;
    }
    out.extend(pieces(w, t0, hi).into_iter().filter_map(|(lo, hi)| retarded_within(t_o, x_o, w, lo, hi)));
    out
}

/// [`retarded_times_at`] over one continuous piece, `[t0, hi]`.
fn retarded_within(t_o: f64, x_o: DVec3, w: &dyn Worldline, t0: f64, mut hi: f64) -> Option<f64> {
    // f(t) = t + |x_o - w(t)| - t_o, strictly increasing for |v| < 1, so exactly one root.
    let f = |t: f64| t + (x_o - w.position_at(t)).length() - t_o;

    let f_hi = f(hi);
    if f_hi < 0.0 {
        return None; // all of this piece's light has already gone past.
    }
    if f_hi == 0.0 {
        return Some(hi);
    }

    // An unbounded worldline gives no finite lower bracket, so walk one back. f falls at
    // rate (1 - beta), so doubling terminates in O(log) steps. The first guess is the static
    // answer: the light travel time from where the source is at `hi`.
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
                return None;
            }
        }
    }

    let f_lo = f(lo);
    if f_lo > 0.0 {
        return None; // the earliest light has not arrived yet.
    }
    if f_lo == 0.0 {
        return Some(lo);
    }

    // Safeguarded Newton. Newton alone can leave the bracket where the derivative is
    // shallow; bisection alone needs 50 iterations to reach f64 precision.
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
    Some(t)
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
    /// Exact: it is a point.
    fn bounding_ball(&self, _t0: f64, _t1: f64) -> (DVec3, f64) {
        (self.position, 0.0)
    }
}

/// Constant velocity, `beta`.
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
    /// Exact: a straight segment's smallest containing ball is the one on its own ends.
    fn bounding_ball(&self, t0: f64, t1: f64) -> (DVec3, f64) {
        let (a, b) = (self.position_at(t0), self.position_at(t1));
        ((a + b) * 0.5, (b - a).length() * 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::Micros;

    /// At `from` until `at`, and at `to` from then on.
    struct Jump {
        from: DVec3,
        to: DVec3,
        at: f64,
    }

    impl Worldline for Jump {
        fn position_at(&self, t: f64) -> DVec3 {
            if t < self.at { self.from } else { self.to }
        }
        fn velocity_at(&self, _t: f64) -> DVec3 {
            DVec3::ZERO
        }
        fn defined_over(&self) -> (f64, f64) {
            (f64::NEG_INFINITY, f64::INFINITY)
        }
        fn breaks(&self) -> SmallVec<[f64; 2]> {
            smallvec::smallvec![self.at]
        }
    }

    fn jump() -> Jump {
        Jump { from: DVec3::ZERO, to: DVec3::new(1000.0, 0.0, 0.0), at: 0.0 }
    }

    /// Near where it left, the craft is seen there until that light passes, then not at all.
    #[test]
    fn a_jump_is_seen_to_vanish_at_light_delay_and_appear_later_still() {
        let w = jump();
        let x_o = DVec3::new(-100.0, 0.0, 0.0);
        let before = retarded_times_at(50.0, x_o, &w);
        assert_eq!(before.as_slice(), &[-50.0], "still seen where it was");
        assert!(retarded_times_at(150.0, x_o, &w).is_empty(), "seen somewhere in the gap");
        let after = retarded_times_at(1200.0, x_o, &w);
        assert_eq!(after.len(), 1);
        assert!((after[0] - 100.0).abs() < 1e-6, "got {after:?}");
    }

    /// Near where it landed, it is seen in both places for a while.
    #[test]
    fn near_the_landing_a_jump_is_seen_in_both_places() {
        let w = jump();
        let roots = retarded_times_at(150.0, DVec3::new(1100.0, 0.0, 0.0), &w);
        assert_eq!(roots.len(), 2, "{roots:?}");
        assert!((roots[0] + 950.0).abs() < 1e-6, "{roots:?}");
        assert!((roots[1] - 50.0).abs() < 1e-6, "{roots:?}");
    }

    /// Without its break the solve reports an image at the step, which the tests above rely on
    /// not happening.
    #[test]
    fn without_its_break_a_jump_is_solved_onto_the_step() {
        struct Undeclared(Jump);
        impl Worldline for Undeclared {
            fn position_at(&self, t: f64) -> DVec3 {
                self.0.position_at(t)
            }
            fn velocity_at(&self, t: f64) -> DVec3 {
                self.0.velocity_at(t)
            }
            fn defined_over(&self) -> (f64, f64) {
                self.0.defined_over()
            }
        }
        let roots = retarded_times_at(150.0, DVec3::new(-100.0, 0.0, 0.0), &Undeclared(jump()));
        assert!(roots.first().is_some_and(|t| t.abs() < 1.0), "{roots:?}");
    }

    /// An observer that jumps receives in whichever piece the light reaches it first.
    #[test]
    fn a_jumping_observer_receives_where_it_is_when_the_light_arrives() {
        let w = jump();
        let arrived = arrival_time_at(-10.0, DVec3::new(500.0, 0.0, 0.0), &w).expect("it arrives");
        assert!((arrived - 490.0).abs() < 1e-6, "got {arrived}");
        // Light that would have reached it where it was reaches it there.
        let early = arrival_time_at(-100.0, DVec3::new(-20.0, 0.0, 0.0), &w).expect("it arrives");
        assert!((early + 80.0).abs() < 1e-6, "got {early}");
    }

    #[test]
    fn a_ball_across_a_jump_holds_both_ends() {
        let w = jump();
        let (center, radius) = w.bounding_ball(-10.0, 10.0);
        assert!(center.distance(w.from) <= radius && center.distance(w.to) <= radius);
    }

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

    /// The two solves are the same statement read from opposite ends: if an event's light
    /// reaches a moving observer at `t_a`, then that observer at `t_a` sees the event emitted
    /// at exactly the event's own time.
    #[test]
    fn arrival_and_retarded_time_are_inverses() {
        let observer = Inertial::new(DVec3::new(1000.0, 0.0, 0.0), DVec3::new(0.3, -0.2, 0.1), 0.0);
        for (t_e, x_e) in [
            (0.0, DVec3::ZERO),
            (-500.0, DVec3::new(0.0, 4000.0, 0.0)),
            (250.0, DVec3::new(-3000.0, 1000.0, 2000.0)),
        ] {
            let t_a = arrival_time_at(t_e, x_e, &observer).expect("the light arrives");
            assert!(t_a > t_e, "light arrived before it left");
            // The defining equation, to f64.
            let gap = (observer.position_at(t_a) - x_e).length();
            assert!((t_a - t_e - gap).abs() < 1e-6 * t_a.abs().max(1.0), "{t_a} {gap}");

            // And the inverse: solving backwards from the arrival recovers the emission.
            let source = Static::new(x_e);
            let back = retarded_times_at(t_a, observer.position_at(t_a), &source);
            assert_eq!(back.len(), 1);
            assert!((back[0] - t_e).abs() < 1e-6 * t_e.abs().max(1.0), "{} vs {t_e}", back[0]);
        }
    }

    /// A static observer's arrival is the light travel time and nothing else, which is the
    /// case every other one has to reduce to.
    #[test]
    fn a_static_observer_receives_at_the_light_travel_time() {
        let observer = Static::new(DVec3::new(0.0, 0.0, 300.0));
        let t_a = arrival_time_at(10.0, DVec3::ZERO, &observer).unwrap();
        assert!((t_a - 310.0).abs() < 1e-9, "{t_a}");
    }

    /// Light that passed before the observer existed, and light still traveling when it
    /// stopped, both come back as nothing rather than as a time outside the worldline.
    #[test]
    fn light_outside_a_worldline_s_life_never_arrives() {
        struct Window(f64, f64);
        impl Worldline for Window {
            fn position_at(&self, _t: f64) -> DVec3 {
                DVec3::new(1000.0, 0.0, 0.0)
            }
            fn velocity_at(&self, _t: f64) -> DVec3 {
                DVec3::ZERO
            }
            fn defined_over(&self) -> (f64, f64) {
                (self.0, self.1)
            }
        }
        // Emitted at the origin at t = 0; it passes x = 1000 at t = 1000.
        assert!(arrival_time_at(0.0, DVec3::ZERO, &Window(2000.0, 3000.0)).is_none(), "passed");
        assert!(arrival_time_at(0.0, DVec3::ZERO, &Window(-100.0, 500.0)).is_none(), "en route");
        assert!(arrival_time_at(0.0, DVec3::ZERO, &Window(-100.0, 5000.0)).is_some());
    }

    /// A bounding ball has to *contain* the motion: one that is too small silently drops what
    /// it excludes, and the traversal that uses it would lose receptions rather than slow down.
    #[test]
    fn a_bounding_ball_contains_the_worldline_it_bounds() {
        let inertial = Inertial::new(DVec3::new(5.0, -2.0, 1.0), DVec3::new(0.6, 0.1, -0.3), 100.0);
        let statics = Static::new(DVec3::new(7.0, 7.0, 7.0));
        for (t0, t1) in [(0.0, 1000.0), (-500.0, -100.0), (42.0, 42.0)] {
            for w in [&inertial as &dyn Worldline, &statics as &dyn Worldline] {
                let (center, radius) = w.bounding_ball(t0, t1);
                for step in 0..=16 {
                    let t = t0 + (t1 - t0) * step as f64 / 16.0;
                    let out = (w.position_at(t) - center).length();
                    assert!(out <= radius + 1e-9, "escaped its ball by {}", out - radius);
                }
            }
        }
        // The default, for a worldline that says nothing about its own shape, is the one `c`
        // alone gives: a span cannot be left faster than light.
        struct Wanderer;
        impl Worldline for Wanderer {
            fn position_at(&self, t: f64) -> DVec3 {
                DVec3::new((t * 0.01).sin() * 50.0, (t * 0.013).cos() * 50.0, 0.0)
            }
            fn velocity_at(&self, t: f64) -> DVec3 {
                DVec3::new((t * 0.01).cos() * 0.5, -(t * 0.013).sin() * 0.65, 0.0)
            }
            fn defined_over(&self) -> (f64, f64) {
                (f64::NEG_INFINITY, f64::INFINITY)
            }
        }
        let (center, radius) = Wanderer.bounding_ball(0.0, 400.0);
        for step in 0..=64 {
            let t = 400.0 * step as f64 / 64.0;
            assert!((Wanderer.position_at(t) - center).length() <= radius + 1e-9);
        }
    }
}
