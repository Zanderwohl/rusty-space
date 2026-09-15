//! Interstellar crossings under constant proper acceleration.
//!
//! Units are `c = 1`: lengths in light-seconds, times in seconds, so a speed is a bare
//! fraction and `alpha` is an inverse time. The ship boosts at a fixed proper acceleration,
//! flips at the midpoint and brakes symmetrically; if it would pass the drive's speed cap on
//! the way it levels off and coasts instead.

use glam::DVec3;

/// Standard gravity, m/s^2.
pub const G0: f64 = 9.80665;

/// Metres per second.
pub const C_M_S: f64 = 299_792_458.0;

/// Seconds in a Julian year. A light-year is this many light-seconds, exactly.
pub const JULIAN_YEAR_S: f64 = 31_557_600.0;

/// How close to a star a crossing stops: about 63 astronomical units.
///
/// Outside the planets, far inside the Oort shell. Arriving at the catalogue position would
/// put the ship inside the star.
pub const STANDOFF_LY: f64 = 1.0e-3;

/// A ship's engine, as the two numbers a crossing needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    /// Proper acceleration, in g. What the crew feels; constant for the whole burn.
    pub accel_g: f64,
    /// Speed cap as a fraction of `c`, strictly below 1.
    pub max_beta: f64,
}

impl Drive {
    pub const DEFAULT: Self = Self { accel_g: 5.0, max_beta: 0.999 };

    /// Proper acceleration as an inverse time, which is what it is when `c = 1`.
    pub fn alpha(&self) -> f64 {
        (self.accel_g * G0 / C_M_S).max(f64::MIN_POSITIVE)
    }

    pub fn cap(&self) -> f64 {
        // A zero cap would make the coast branch divide by it.
        self.max_beta.clamp(1e-6, MAX_BETA)
    }
}

impl Default for Drive {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The fastest representable speed. Past this `gamma` is infinite and the shift factors stop
/// being numbers.
pub const MAX_BETA: f64 = 1.0 - 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Shedding the velocity that is across the line, before the crossing proper.
    Match,
    Boost,
    Coast,
    Brake,
    Arrived,
}

/// Where the ship is and how fast, at one coordinate time.
#[derive(Clone, Copy, Debug)]
pub struct FlightState {
    pub position_ly: DVec3,
    /// Velocity as a fraction of `c`.
    pub beta: DVec3,
    /// Ship-clock seconds since the burn began. Always less than the coordinate elapsed.
    pub proper_s: f64,
    pub phase: Phase,
}

/// One planned crossing: boost, optional coast, brake.
///
/// The plan is computed once and then only sampled, so the trajectory does not drift with
/// the frame rate and a paused or fast-forwarded clock lands in the same place.
#[derive(Clone, Debug, PartialEq)]
pub struct Cruise {
    pub from_ly: DVec3,
    pub to_ly: DVec3,
    /// Coordinate seconds at which the burn began.
    pub start_s: f64,
    pub drive: Drive,
    direction: DVec3,
    distance_ls: f64,
    alpha: f64,
    /// Shedding whatever velocity is across the line, before the crossing proper. Zero for a
    /// ship already on the line — which includes every ship starting from rest.
    match_s: f64,
    /// The direction the ship is drifting across the line, and the ground it covers doing so.
    match_dir: DVec3,
    match_ls: f64,
    /// The along-line speed, carried through the match.
    match_along: f64,
    /// Where the ship is once the match is done, which is where the line begins.
    matched_ly: DVec3,
    /// Where on the rest profile the boost begins, in seconds.
    ///
    /// Zero for a ship starting from rest. Signed: negative means the ship is moving *away*
    /// from the target and the burn first brings it back through zero. See [`Cruise::plan_from`].
    t0_s: f64,
    /// Seconds from start: end of boost, start of brake, arrival.
    boost_s: f64,
    brake_s: f64,
    arrive_s: f64,
    /// Light-seconds covered by the boost phase alone.
    boost_ls: f64,
    coast_beta: f64,
    /// Ship seconds at the end of boost, and for the whole crossing.
    boost_proper_s: f64,
    proper_s: f64,
}

impl Cruise {
    /// Plan a crossing. A zero-length one is already arrived.
    pub fn plan(from_ly: DVec3, to_ly: DVec3, start_s: f64, drive: Drive) -> Self {
        Self::plan_from(from_ly, DVec3::ZERO, to_ly, start_s, drive)
    }

    /// Plan a crossing **from whatever velocity the ship already has**.
    ///
    /// A burn at constant proper acceleration starting at speed `b0` is the same burn started
    /// from rest, entered part-way through: if a ship boosting from rest reaches `b0` at time
    /// `t0`, then this ship's trajectory is that one's from `t0` onward. So the whole profile
    /// generalises by an offset and the closed forms below are unchanged — including the brake,
    /// which still ends at rest and so is not touched at all.
    ///
    /// `t0` is **signed**. Negative means the ship is moving away from the target, and the burn
    /// first brings it back through zero — which is the same trajectory, entered before the
    /// point where it turns around.
    ///
    /// **Only the component along the line is carried.** A straight-line plan cannot express
    /// shedding a sideways velocity, because shedding it curves the path. For a ship leaving an
    /// orbit that component is thousandths of a percent of `c` and the error is nothing; for a
    /// ship re-aiming hard sideways at relativistic speed it is not, and that case wants a
    /// trajectory model that bends.
    pub fn plan_from(
        from_ly: DVec3,
        beta0: DVec3,
        to_ly: DVec3,
        start_s: f64,
        drive: Drive,
    ) -> Self {
        let alpha = drive.alpha();
        let cap = drive.cap();
        let ordered_from = from_ly;

        // **The match.** A crossing is a straight line, and a ship cannot fly a line it is
        // moving across — so before the crossing proper it sheds whatever velocity is not along
        // it. That is a burn of its own, in its own direction, and it takes real time and covers
        // real ground; the line is drawn from where the ship ends up, not from where it was.
        let aim = (to_ly - from_ly).normalize_or_zero();
        let along_ordered = beta0.dot(aim);
        let across_v = beta0 - aim * along_ordered;
        let across = across_v.length().min(MAX_BETA);
        let (match_s, match_dir, match_ls, from_ly) = if across > 1.0e-12 {
            let match_dir = across_v / across_v.length();
            // Where the rest profile is already at `across`; braking from there reaches zero.
            let m_t0 = across / (1.0 - across * across).sqrt() / alpha;
            let across_ls = distance_of(alpha, m_t0);
            // The along-line component carries on through the match. Held rather than
            // integrated: a rest-frame boost perpendicular to the velocity leaves the parallel
            // component exactly unchanged, and this thrust is perpendicular to the *line*
            // rather than to the velocity — so it is exact when the ship is moving purely
            // across the line, which is the case the match exists for, and the error grows with
            // the along-line speed while the match shortens with it.
            let carried = ordered_from
                + (match_dir * across_ls + aim * (along_ordered * m_t0)) / JULIAN_YEAR_S;
            (m_t0, match_dir, across_ls, carried)
        } else {
            (0.0, DVec3::ZERO, 0.0, from_ly)
        };

        let delta = to_ly - from_ly;
        let direction = delta.normalize_or_zero();
        let mut distance_ls = delta.length() * JULIAN_YEAR_S;

        // Signed speed along the line, once the ship is on it.
        let along = beta0.dot(direction).clamp(-MAX_BETA, MAX_BETA);
        // `sinh(phi) = gamma * beta`, so this is where the rest profile is already at `along`.
        let t0_s = along / (1.0 - along * along).sqrt() / alpha;
        // Even in `t0`: a profile run backwards covers the same ground.
        let x0 = distance_of(alpha, t0_s);

        // Speed and distance at which the boost would reach the cap.
        let gamma_cap = (1.0 - cap * cap).sqrt().recip();
        let cap_ls = (gamma_cap - 1.0) / alpha;
        let t_cap = gamma_cap * cap / alpha;

        // Too fast to stop in what is left: the shortest flight from here is to brake the whole
        // way, and it ends past the target. Saying so is better than pretending a drive can do
        // what it cannot — the ship stops where it actually stops.
        let mut to_ly = to_ly;
        if t0_s > 0.0 && distance_ls < x0 {
            distance_ls = x0;
            to_ly = from_ly + direction * (x0 / JULIAN_YEAR_S);
        }

        // Boost and brake together cover the distance: `2 x(peak) - x(t0) = D`.
        let x_peak = (distance_ls + x0) / 2.0;
        let (boost_s, boost_ls, coast_s, coast_beta) = if distance_ls <= 0.0 {
            (0.0, 0.0, 0.0, 0.0)
        } else if x_peak <= cap_ls {
            // Flip and burn: the cap is never reached.
            let k = alpha * x_peak;
            // From x = (sqrt(1 + (at)^2) - 1)/a, so (at)^2 = k^2 + 2k.
            let t_peak = (k * k + 2.0 * k).sqrt() / alpha;
            ((t_peak - t0_s).max(0.0), x_peak - x0, 0.0, 0.0)
        } else {
            let coast_ls = distance_ls - (cap_ls - x0) - cap_ls;
            ((t_cap - t0_s).max(0.0), cap_ls - x0, (coast_ls / cap).max(0.0), cap)
        };

        let boost_proper_s = proper_of(alpha, t0_s + boost_s) - proper_of(alpha, t0_s);
        let coast_proper_s = if coast_beta > 0.0 {
            coast_s * (1.0 - coast_beta * coast_beta).sqrt()
        } else {
            0.0
        };

        Self {
            from_ly: ordered_from,
            to_ly,
            start_s,
            drive,
            direction,
            distance_ls,
            alpha,
            match_s,
            match_dir,
            match_ls,
            match_along: along_ordered,
            matched_ly: from_ly,
            t0_s,
            boost_s,
            brake_s: boost_s + coast_s,
            arrive_s: 2.0 * boost_s + coast_s,
            boost_ls,
            coast_beta,
            boost_proper_s,
            proper_s: 2.0 * boost_proper_s + coast_proper_s,
        }
    }

    /// Coordinate seconds the whole crossing takes, the match included.
    pub fn duration_s(&self) -> f64 {
        self.match_s + self.arrive_s
    }

    /// Ship seconds the whole crossing takes. Never more than [`Cruise::duration_s`].
    pub fn proper_duration_s(&self) -> f64 {
        self.proper_s
    }

    /// The fastest the ship goes, as a fraction of `c`.
    pub fn peak_beta(&self) -> f64 {
        if self.coast_beta > 0.0 {
            self.coast_beta
        } else {
            beta_of(self.alpha, self.t0_s + self.boost_s)
        }
    }

    pub fn has_arrived(&self, now_s: f64) -> bool {
        now_s - self.start_s >= self.duration_s()
    }

    /// Fraction of the crossing completed, `[0, 1]`.
    pub fn progress(&self, now_s: f64) -> f64 {
        let whole = self.duration_s();
        if whole <= 0.0 {
            return 1.0;
        }
        ((now_s - self.start_s) / whole).clamp(0.0, 1.0)
    }

    /// Sample the trajectory. Outside the burn it holds the endpoints, at rest.
    pub fn at(&self, now_s: f64) -> FlightState {
        let since = now_s - self.start_s;
        // The match comes first, in its own direction. See `plan_from`.
        if since < self.match_s {
            let left = self.match_s - since.max(0.0);
            let across_ls = self.match_ls - distance_of(self.alpha, left);
            let along_ls = self.match_along * since.max(0.0);
            return FlightState {
                position_ly: self.from_ly
                    + (self.match_dir * across_ls + self.direction * along_ls) / JULIAN_YEAR_S,
                beta: self.match_dir * beta_of(self.alpha, left) + self.direction * self.match_along,
                proper_s: proper_of(self.alpha, self.match_s) - proper_of(self.alpha, left),
                phase: Phase::Match,
            };
        }
        let matched_proper = proper_of(self.alpha, self.match_s);
        let t = (since - self.match_s).clamp(0.0, self.arrive_s);
        if self.arrive_s <= 0.0 {
            return FlightState {
                position_ly: self.to_ly,
                beta: DVec3::ZERO,
                proper_s: matched_proper,
                phase: Phase::Arrived,
            };
        }

        let (travelled_ls, speed, proper_s, phase) = if t < self.boost_s {
            // Offset onto the rest profile: this ship entered that trajectory at `t0`. See
            // `plan_from`.
            let on = self.t0_s + t;
            (
                distance_of(self.alpha, on) - distance_of(self.alpha, self.t0_s),
                beta_of(self.alpha, on),
                proper_of(self.alpha, on) - proper_of(self.alpha, self.t0_s),
                Phase::Boost,
            )
        } else if t < self.brake_s {
            let c = t - self.boost_s;
            let inv_gamma = (1.0 - self.coast_beta * self.coast_beta).sqrt();
            (
                self.boost_ls + self.coast_beta * c,
                self.coast_beta,
                self.boost_proper_s + c * inv_gamma,
                Phase::Coast,
            )
        } else {
            // The brake is the boost run backwards, which keeps the two exactly symmetric.
            let s = self.arrive_s - t;
            let phase = if t >= self.arrive_s { Phase::Arrived } else { Phase::Brake };
            (
                self.distance_ls - distance_of(self.alpha, s),
                beta_of(self.alpha, s),
                self.proper_s - proper_of(self.alpha, s),
                phase,
            )
        };

        FlightState {
            // From the matched point, which is where the line begins. For a ship that started
            // on the line already, that is where it started.
            position_ly: self.matched_ly + self.direction * (travelled_ls / JULIAN_YEAR_S),
            beta: self.direction * speed,
            // The crew aged through the match too.
            proper_s: matched_proper + proper_s,
            phase,
        }
    }
}

/// Distance covered from rest after `t`, light-seconds.
///
/// The textbook `(sqrt(1 + (at)^2) - 1)/a` subtracts two nearly equal numbers early in the
/// burn and loses most of its digits there. This form is the same function without the
/// cancellation.
fn distance_of(alpha: f64, t: f64) -> f64 {
    let at = alpha * t;
    alpha * t * t / ((1.0 + at * at).sqrt() + 1.0)
}

/// Speed after `t` of boost from rest, as a fraction of `c`. Approaches 1 and never reaches it.
fn beta_of(alpha: f64, t: f64) -> f64 {
    let at = alpha * t;
    (at / (1.0 + at * at).sqrt()).min(MAX_BETA)
}

/// Ship seconds elapsed over `t` coordinate seconds of boost from rest.
fn proper_of(alpha: f64, t: f64) -> f64 {
    (alpha * t).asinh() / alpha
}

#[cfg(test)]
mod tests {
    use super::*;

    const LY: f64 = JULIAN_YEAR_S;

    fn to(ly: f64) -> Cruise {
        Cruise::plan(DVec3::ZERO, DVec3::X * ly, 0.0, Drive::DEFAULT)
    }

    #[test]
    fn a_crossing_starts_and_ends_at_rest_in_the_right_places() {
        let c = to(4.0);
        let start = c.at(0.0);
        let end = c.at(c.duration_s());
        assert!(start.beta.length() < 1e-12, "{:?}", start.beta);
        assert!(end.beta.length() < 1e-9, "the brake must finish, got {:?}", end.beta);
        assert!(start.position_ly.length() < 1e-9);
        assert!((end.position_ly - DVec3::X * 4.0).length() < 1e-6, "{:?}", end.position_ly);
        assert_eq!(end.phase, Phase::Arrived);
    }

    #[test]
    fn position_is_monotone_and_the_flip_is_at_the_midpoint() {
        let c = to(4.0);
        let mut last = -1.0;
        for k in 0..=200 {
            let s = c.at(c.duration_s() * k as f64 / 200.0);
            let x = s.position_ly.x;
            assert!(x >= last - 1e-9, "went backwards at {k}: {last} -> {x}");
            last = x;
        }
        let middle = c.at(c.duration_s() / 2.0);
        assert!((middle.position_ly.x - 2.0).abs() < 1e-6, "{:?}", middle.position_ly);
    }

    /// Five g never reaches the cap inside four light-years, so there is no coast.
    #[test]
    fn a_short_crossing_is_pure_flip_and_burn() {
        let c = to(4.0);
        let phases: Vec<Phase> = (0..50).map(|k| c.at(c.duration_s() * k as f64 / 50.0).phase).collect();
        assert!(!phases.contains(&Phase::Coast), "{phases:?}");
        assert!(phases.contains(&Phase::Boost) && phases.contains(&Phase::Brake));
        assert!(c.peak_beta() > 0.99 && c.peak_beta() < Drive::DEFAULT.max_beta, "{}", c.peak_beta());
    }

    #[test]
    fn a_long_crossing_levels_off_at_the_cap_and_coasts() {
        let c = to(100.0);
        let phases: Vec<Phase> = (0..50).map(|k| c.at(c.duration_s() * k as f64 / 50.0).phase).collect();
        assert!(phases.contains(&Phase::Coast), "{phases:?}");
        let fastest = (0..500)
            .map(|k| c.at(c.duration_s() * k as f64 / 500.0).beta.length())
            .fold(0.0f64, f64::max);
        assert!((fastest - Drive::DEFAULT.max_beta).abs() < 1e-9, "capped at {fastest}");
    }

    /// A crossing takes slightly longer than light plus the cost of turning round.
    #[test]
    fn nothing_outruns_light() {
        for ly in [0.1, 1.0, 4.0, 100.0, 1000.0] {
            let c = to(ly);
            assert!(c.duration_s() > ly * LY, "{ly} ly took {} s", c.duration_s());
            assert!(c.at(c.duration_s() / 2.0).beta.length() < 1.0);
        }
    }

    #[test]
    fn the_ship_clock_runs_slow_and_the_gap_widens_with_distance() {
        let short = to(1.0);
        let long = to(100.0);
        for c in [&short, &long] {
            assert!(c.proper_duration_s() < c.duration_s(), "{c:?}");
        }
        let short_ratio = short.proper_duration_s() / short.duration_s();
        let long_ratio = long.proper_duration_s() / long.duration_s();
        assert!(long_ratio < short_ratio, "{long_ratio} should be under {short_ratio}");
    }

    /// Four light-years at five g: about 4.4 years coordinate, about 1.2 aboard.
    ///
    /// Pins the numbers, not just the inequalities. Both follow from `x = (cosh(a tau) - 1)/a`
    /// with `a = 5 g / c`.
    #[test]
    fn four_light_years_at_five_g_takes_the_years_the_closed_form_says() {
        let c = to(4.0);
        let years = c.duration_s() / JULIAN_YEAR_S;
        let aboard = c.proper_duration_s() / JULIAN_YEAR_S;
        assert!((years - 4.37).abs() < 0.02, "{years} coordinate years");
        assert!((aboard - 1.21).abs() < 0.02, "{aboard} ship years");
    }

    #[test]
    fn proper_time_advances_monotonically_and_never_passes_coordinate_time() {
        let c = to(100.0);
        let mut last = -1.0;
        for k in 0..=300 {
            let t = c.duration_s() * k as f64 / 300.0;
            let s = c.at(t);
            assert!(s.proper_s >= last - 1e-6, "{last} -> {}", s.proper_s);
            assert!(s.proper_s <= t + 1e-6, "aboard {} past coordinate {t}", s.proper_s);
            last = s.proper_s;
        }
    }

    /// The stable form is not a stylistic preference: early in the burn the textbook one has
    /// lost most of its significant digits.
    #[test]
    fn the_distance_form_survives_the_start_of_the_burn() {
        let alpha = Drive::DEFAULT.alpha();
        let t = 1.0;
        let naive = ((1.0 + (alpha * t).powi(2)).sqrt() - 1.0) / alpha;
        let exact = 0.5 * alpha * t * t; // the a t^2 / 2 limit, to well past f64 precision here
        let ours = distance_of(alpha, t);
        assert!((ours - exact).abs() / exact < 1e-12, "{ours} vs {exact}");
        assert!(
            (naive - exact).abs() > (ours - exact).abs(),
            "the naive form should be measurably worse: {naive} vs {ours}"
        );
    }

    #[test]
    fn a_crossing_of_no_distance_is_already_over() {
        let c = Cruise::plan(DVec3::X, DVec3::X, 10.0, Drive::DEFAULT);
        assert_eq!(c.duration_s(), 0.0);
        assert!(c.has_arrived(10.0));
        assert_eq!(c.progress(10.0), 1.0);
        assert_eq!(c.at(10.0).phase, Phase::Arrived);
    }

    #[test]
    fn sampling_outside_the_burn_holds_the_endpoints() {
        let c = to(4.0);
        let before = c.at(-1e9);
        let after = c.at(c.duration_s() * 3.0);
        assert!(before.position_ly.length() < 1e-9 && before.beta.length() < 1e-12);
        assert!((after.position_ly - DVec3::X * 4.0).length() < 1e-6);
        assert!(after.beta.length() < 1e-9);
    }

    #[test]
    fn a_harder_burn_arrives_sooner() {
        let mild = Cruise::plan(DVec3::ZERO, DVec3::X * 10.0, 0.0, Drive { accel_g: 1.0, ..Drive::DEFAULT });
        let hard = Cruise::plan(DVec3::ZERO, DVec3::X * 10.0, 0.0, Drive { accel_g: 20.0, ..Drive::DEFAULT });
        assert!(hard.duration_s() < mild.duration_s());
        assert!(hard.proper_duration_s() < mild.proper_duration_s());
    }

    #[test]
    fn velocity_points_at_the_destination_throughout() {
        let target = DVec3::new(1.0, -2.0, 0.5);
        let c = Cruise::plan(DVec3::ZERO, target, 0.0, Drive::DEFAULT);
        let want = target.normalize();
        for k in 1..40 {
            let s = c.at(c.duration_s() * k as f64 / 40.0);
            assert!((s.beta.normalize() - want).length() < 1e-12, "{:?}", s.beta);
        }
    }
}
