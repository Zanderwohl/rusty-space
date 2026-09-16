//! Interstellar crossings under constant proper acceleration.
//!
//! Units are `c = 1`: lengths in light-seconds, times in seconds, so a speed is a bare
//! fraction and `alpha` is an inverse time. The ship boosts at a fixed proper acceleration,
//! flips at the midpoint and brakes symmetrically; if it would pass the drive's speed cap on
//! the way it levels off and coasts instead.
//!
//! **Every crossing coasts.** The flip is a turn and a turn takes time — see [`crate::attitude`]
//! — so the plan holds the drive off for at least [`Drive::flip_s`] between the boost and the
//! brake, and the ship covers that ground at its peak speed. For a five-hundred-metre hull it is
//! a minute in the middle of a journey of years; for a fifty-kilometre one it is nearly two
//! hours, and for a short hop it is most of the trip.

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

/// What a craft can do under power: how hard it pushes, how fast it will go, how fast it turns,
/// and what any of that costs in light.
///
/// More than an engine, and deliberately: this is the bundle a crossing is planned from, so
/// anything the plan depends on belongs in it. The slew rate is here for that reason rather
/// than because attitude control is part of the drive — a plan that turns round has to make
/// room for the turn, and a plan put back together at the far end has to make room for the same
/// one. See [`crate::resume`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    /// Proper acceleration, in g. What the crew feels; constant for the whole burn.
    pub accel_g: f64,
    /// Speed cap as a fraction of `c`, strictly below 1.
    pub max_beta: f64,
    /// How fast it throws its reaction mass, metres a second.
    ///
    /// Nothing about a *trajectory* depends on this — a crossing is planned from the
    /// acceleration alone and would be the same at any exhaust speed. What it decides is the
    /// price: the jet power of a given thrust is `½ F v`, so a drive that throws mass slowly
    /// must throw a great deal of it, and one that throws it fast lights up the sky. See
    /// [`Drive::jet_power_w`].
    ///
    /// Five per cent of `c` for the default, which is a torch rather than anything anyone has
    /// built.
    pub exhaust_v_m_s: f64,
    /// How fast the hull can swing its nose, radians a second. See [`crate::attitude`].
    ///
    /// A property of the ship rather than of this struct's namesake, and a *plan* parameter:
    /// [`Cruise::plan_from`] holds the coast open for [`Drive::flip_s`] so the ship has finished
    /// turning before the brake lights. Stamp it on from the hull that is flying — see
    /// [`crate::craft::Craft::turning`] — rather than trusting a copy the hull may have
    /// outgrown since.
    pub slew_rate_rad_s: f64,
}

impl Drive {
    pub const DEFAULT: Self = Self {
        accel_g: 5.0,
        max_beta: 0.999,
        exhaust_v_m_s: 0.05 * C_M_S,
        slew_rate_rad_s: crate::attitude::RATE_RAD_S,
    };

    /// How long this craft takes to turn end for end, seconds.
    pub fn flip_s(&self) -> f64 {
        crate::attitude::flip_time_s(self.slew_rate_rad_s)
    }

    /// What the drive puts into its exhaust to push `mass_kg` at `accel_g`, watts.
    ///
    /// `½ F v` with `F = m a`, which is exact for a rocket: the thrust is the momentum carried
    /// off per second and the power is the kinetic energy in it. Everything visible about a
    /// burn comes from this one number — how long the plume is, how hot, and how far away
    /// somebody can see it happen.
    ///
    /// It is a large number. Two million tonnes at five gravities with a torch for an engine is
    /// a few times ten to the seventeenth watts, which is a fair fraction of what a small star
    /// puts out, and that is the honest answer for a ship that crosses between them.
    pub fn jet_power_w(&self, mass_kg: f64, accel_g: f64) -> f64 {
        0.5 * mass_kg * accel_g * G0 * self.exhaust_v_m_s
    }

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

/// An order to point somewhere: where from, where to, and when it was given.
///
/// `from` is `None` when nothing has been ordered before — the turn then starts from whatever
/// the ship was already pointing at, which only it knows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    pub to: DVec3,
    pub from: Option<DVec3>,
    pub since_s: f64,
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
    /// The velocity the ship had when this was planned.
    ///
    /// Kept because it is a *parameter* of the plan and everything below is derived from it:
    /// a crossing is put back on the wire as the five values [`Cruise::plan_from`] takes, and
    /// re-planned at the far end rather than shipped as a solved trajectory. See
    /// [`crate::resume`].
    beta0: DVec3,
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
    ///
    /// The gap between the first two is the coast, and it is never nothing — the flip lives in
    /// it. The brake is not the boost's length back again: it runs the rest profile down from
    /// the peak to zero, which takes as long as that profile took to reach the peak in the first
    /// place, and only a ship that set out from rest boosted for exactly that long.
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
    /// generalises by an offset and the closed forms below are unchanged.
    ///
    /// The offset is on the **boost only**. The brake ends at rest, so it is the rest profile run
    /// backwards from the peak whatever the ship was doing when the crossing began — which makes
    /// it the longer of the two halves for a ship that set out already moving. Giving it the
    /// boost's length instead used to teleport such a ship forward at the flip, by a tenth of a
    /// light-year for a crossing entered at half `c`.
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

        // How long the ship spends pointing neither way. The brake cannot light until the flip
        // is over, so this is a floor on the coast and a term in the distance the crossing
        // covers — not an adjustment made afterwards.
        let flip_s = drive.flip_s();

        // Too fast to stop in what is left: the shortest flight from here is to turn round and
        // brake the whole way, and it ends past the target. Saying so is better than pretending
        // a drive can do what it cannot — the ship stops where it actually stops. The flip is
        // part of "what it cannot": a ship still coming about is a ship still closing.
        let least_ls = x0 + along.max(0.0) * flip_s;
        let mut to_ly = to_ly;
        if t0_s > 0.0 && distance_ls < least_ls {
            distance_ls = least_ls;
            to_ly = from_ly + direction * (least_ls / JULIAN_YEAR_S);
        }

        // Boost, coast and brake together cover the distance:
        //
        //     x(peak) - x(t0)  +  beta(peak) * coast  +  x(peak)  =  D
        //
        // `peak` being where the ship is on the rest profile when the drive goes out. The brake
        // is that profile run backwards to rest, so it takes `peak` seconds and covers `x(peak)`
        // however fast the ship was going when the crossing began.
        let (peak_s, boost_s, boost_ls, coast_s, coast_beta) = if distance_ls <= 0.0 {
            (0.0, 0.0, 0.0, 0.0, 0.0)
        } else {
            // What would be left to cover at the cap, once boost and brake have taken their
            // share. More than the flip can use means the cap really is the binding constraint.
            let at_cap_ls = distance_ls - (2.0 * cap_ls - x0);
            let (peak_s, coast_s, coast_beta) = if at_cap_ls >= cap * flip_s {
                (t_cap, at_cap_ls / cap, cap)
            } else {
                let peak_s = peak_time(alpha, distance_ls + x0, flip_s, t_cap);
                (peak_s, flip_s, beta_of(alpha, peak_s))
            };
            (
                peak_s,
                (peak_s - t0_s).max(0.0),
                distance_of(alpha, peak_s) - x0,
                coast_s,
                coast_beta,
            )
        };

        let boost_proper_s = proper_of(alpha, t0_s + boost_s) - proper_of(alpha, t0_s);
        let coast_proper_s = coast_s * (1.0 - coast_beta * coast_beta).sqrt();
        // The brake starts at the peak and ends at rest, so it ages the crew by the whole of the
        // rest profile up to the peak — which is *not* the boost's share back again unless the
        // ship began at rest.
        let brake_proper_s = proper_of(alpha, peak_s);

        Self {
            from_ly: ordered_from,
            to_ly,
            beta0,
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
            arrive_s: boost_s + coast_s + peak_s,
            boost_ls,
            coast_beta,
            boost_proper_s,
            proper_s: boost_proper_s + coast_proper_s + brake_proper_s,
        }
    }

    /// The velocity this was planned from. With the other four public fields, the whole recipe.
    pub fn initial_beta(&self) -> DVec3 {
        self.beta0
    }

    /// Coordinate seconds the whole crossing takes, the match included.
    pub fn duration_s(&self) -> f64 {
        self.match_s + self.arrive_s
    }

    /// Ship seconds the whole crossing takes. Never more than [`Cruise::duration_s`].
    pub fn proper_duration_s(&self) -> f64 {
        self.proper_s
    }

    /// How long the drive is out between the boost and the brake, seconds.
    ///
    /// Never less than [`Drive::flip_s`], and equal to it whenever the crossing is short enough
    /// that the speed cap never comes into it. See [`Cruise::plan_from`].
    pub fn coast_s(&self) -> f64 {
        self.brake_s - self.boost_s
    }

    /// The fastest the ship goes, as a fraction of `c`. Reached at the flip and held through it.
    pub fn peak_beta(&self) -> f64 {
        self.coast_beta
    }

    /// Which way the ship is being told to point, and when it was told.
    ///
    /// Not the same question as [`Cruise::thrust_at`], and the difference is the flip. A ship
    /// coasting between the boost and the brake has nothing lit, so its *thrust* is zero — but
    /// it is not idling, it is turning around, and it was told to the moment the boost ended.
    /// Putting the flip in the coast is what makes it free of *thrust* — the drive is off
    /// anyway — and [`Cruise::plan_from`] holds the coast open long enough for the turn to
    /// finish, so the brake never lights on a nose that is still coming about.
    ///
    /// `from` is where the previous order left the nose, or `None` when this is the first order
    /// of the crossing and the answer is whatever the ship was already doing.
    pub fn aim_at(&self, now_s: f64) -> Aim {
        let since = now_s - self.start_s;
        // Shedding the velocity across the line: the drive points against it, and this is the
        // first thing the crossing asks for.
        if since < self.match_s {
            return Aim { to: -self.match_dir, from: None, since_s: self.start_s };
        }
        let after_match = self.start_s + self.match_s;
        // A ship that had no match to fly was never told anything before the boost.
        let before_boost = (self.match_s > 0.0).then_some(-self.match_dir);
        let t = since - self.match_s;
        if t < self.boost_s {
            return Aim { to: self.direction, from: before_boost, since_s: after_match };
        }
        // Everything from the end of the boost onward is one order — turn around and brake —
        // so the turn is not restarted at the moment the drive relights.
        Aim {
            to: -self.direction,
            from: Some(self.direction),
            since_s: after_match + self.boost_s,
        }
    }

    /// Which way the drive is pointing, or zero where it is not lit.
    ///
    /// Not the velocity: a crossing is burn, flip and burn, so the ship spends the whole second
    /// half of it pointing back the way it came while still moving forward.
    pub fn thrust_at(&self, now_s: f64) -> DVec3 {
        match self.at(now_s).phase {
            // Shedding the velocity across the line, so the drive points against it.
            Phase::Match => -self.match_dir,
            Phase::Boost => self.direction,
            Phase::Brake => -self.direction,
            Phase::Coast | Phase::Arrived => DVec3::ZERO,
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
            // The rest profile run backwards from the peak. Symmetric with the boost only for
            // a ship that started at rest; for any other it is the longer half.
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

/// Where on the rest profile the drive goes out, given that the coast must last `flip_s`.
///
/// Solves `2 x(t) + beta(t) * flip_s = k`, where `k` is the crossing's distance plus the ground
/// the ship had already made on that profile. Both terms rise with `t` and neither ever falls,
/// so the left side is strictly increasing: there is exactly one root, bisection cannot land on
/// the wrong one, and no starting guess can send it somewhere else. Newton would converge faster
/// and would also have to be nursed through the cap, where `beta` flattens out.
///
/// Without the coast term this inverts in closed form — `(at)^2 = k^2 + 2k` for `k = a x` —
/// which is what the solver did before turning cost anything. With it the equation is a quartic
/// in `at`, and a quartic's radicals are a worse numerical object than fifty bisections.
///
/// A fixed iteration count, not a tolerance. Both ends of the wire plan the same crossing from
/// the same recipe, and a loop that stops when it is close enough is a loop that can stop one
/// step later somewhere else.
fn peak_time(alpha: f64, k: f64, flip_s: f64, upper_s: f64) -> f64 {
    let reach = |t: f64| 2.0 * distance_of(alpha, t) + beta_of(alpha, t) * flip_s;
    let (mut lo, mut hi) = (0.0, upper_s.max(1.0));
    // The caller's bound is the time at the speed cap, which brackets the root whenever this is
    // reached at all. Widening is for the callers that have not checked.
    for _ in 0..64 {
        if reach(hi) >= k {
            break;
        }
        hi *= 2.0;
    }
    // Enough halvings to exhaust an f64 over any bracket this can be handed; the last few are
    // no-ops once `lo` and `hi` are neighbours.
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if reach(mid) < k {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
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
        // Boost and brake are the same length from rest and the flip sits between them, so the
        // halfway moment is somewhere in the turn.
        let middle = c.at(c.duration_s() / 2.0);
        assert_eq!(middle.phase, Phase::Coast);
        assert!((middle.position_ly.x - 2.0).abs() < 1e-5, "{:?}", middle.position_ly);
    }

    /// Five g never reaches the cap inside four light-years, so the only coast is the flip.
    #[test]
    fn a_short_crossing_is_flip_and_burn_and_the_flip_is_all_the_coast_there_is() {
        let c = to(4.0);
        assert_eq!(c.coast_s(), Drive::DEFAULT.flip_s(), "the coast is the flip and nothing more");
        assert!(c.peak_beta() > 0.99 && c.peak_beta() < Drive::DEFAULT.max_beta, "{}", c.peak_beta());
        let phases: Vec<Phase> = (0..50).map(|k| c.at(c.duration_s() * k as f64 / 50.0).phase).collect();
        assert!(phases.contains(&Phase::Boost) && phases.contains(&Phase::Brake));
    }

    /// The flip is a turn, and a turn takes time — so the drive goes out for at least as long as
    /// the turn takes, whatever else the crossing is doing.
    #[test]
    fn every_crossing_holds_the_coast_open_for_the_flip() {
        for ly in [1.0e-6, 1.0e-3, 0.1, 1.0, 4.0, 100.0, 1000.0] {
            let c = to(ly);
            assert!(
                c.coast_s() >= Drive::DEFAULT.flip_s() - 1.0e-6,
                "{ly} ly coasted {} s, short of the {} s flip",
                c.coast_s(),
                Drive::DEFAULT.flip_s(),
            );
            assert_eq!(c.at(c.start_s + c.match_s + c.boost_s + c.coast_s() * 0.5).phase, Phase::Coast);
        }
    }

    /// And the turn has actually finished by the time the brake lights. This is the whole point:
    /// the order goes out when the boost ends, and nothing thrusts on a nose still coming about.
    #[test]
    fn the_nose_has_come_round_before_the_brake_lights() {
        for length_m in [500.0, 5_000.0, 50_000.0] {
            let drive = Drive {
                slew_rate_rad_s: crate::attitude::rate_rad_s(length_m),
                ..Drive::DEFAULT
            };
            let c = Cruise::plan(DVec3::ZERO, DVec3::X * 0.02, 0.0, drive);
            let lit = c.start_s + c.brake_s;
            let aim = c.aim_at(lit);
            assert_eq!(aim.to, -c.direction, "the brake is a turn round, not a nudge");
            let nose = crate::attitude::turned(
                aim.from.expect("the boost pointed it somewhere"),
                aim.to,
                drive.slew_rate_rad_s,
                lit - aim.since_s,
            );
            assert!(
                (nose - c.thrust_at(lit + 1.0)).length() < 1.0e-9,
                "a {length_m} m hull was still at {nose} when the brake lit",
            );
        }
    }

    /// A hull a hundred times longer turns a hundred times more slowly, and over a hop of a few
    /// light-seconds that turn is most of the journey — so the ponderous one takes far longer
    /// over the same trip, and is still coming about while the nimble one has arrived.
    #[test]
    fn a_ponderous_hull_takes_longer_over_the_same_hop() {
        // Three light-seconds: an in-system errand, and short enough that the flip dominates.
        let hop = DVec3::X * 1.0e-7;
        let plan = |length_m: f64| {
            Cruise::plan(DVec3::ZERO, hop, 0.0, Drive {
                slew_rate_rad_s: crate::attitude::rate_rad_s(length_m),
                ..Drive::DEFAULT
            })
        };
        let (nimble, ponderous) = (plan(500.0), plan(50_000.0));
        assert!(ponderous.duration_s() > nimble.duration_s());
        // The big hull spends most of the trip turning rather than burning, so it has to be
        // going slower at the flip — it covers the distance during the turn instead.
        assert!(ponderous.peak_beta() < nimble.peak_beta(), "{ponderous:?}");
        assert!(
            ponderous.coast_s() > 0.5 * ponderous.duration_s(),
            "a fifty-kilometre hull's hop is more than half flip: {} s of {} s",
            ponderous.coast_s(),
            ponderous.duration_s(),
        );
    }

    /// Past the cap the coast is longer than the flip, and then the cap is what sets it.
    #[test]
    fn a_long_crossing_coasts_for_the_distance_rather_than_for_the_turn() {
        let c = to(100.0);
        assert!(c.coast_s() > Drive::DEFAULT.flip_s() * 1000.0, "{} s", c.coast_s());
        assert_eq!(c.peak_beta(), Drive::DEFAULT.max_beta);
    }

    /// The trajectory must not jump where the phases meet. It used to: the brake was given the
    /// boost's length, which is only right for a ship that set out from rest — one already
    /// moving teleported forward at the flip by as much as a tenth of a light-year.
    #[test]
    fn the_phases_join_up_even_from_a_moving_start() {
        for along in [-0.5, -0.01, 0.0, 0.3, 0.9] {
            let c = Cruise::plan_from(DVec3::ZERO, DVec3::X * along, DVec3::X * 4.0, 0.0, Drive::DEFAULT);
            let eps = 1.0e-3;
            for (name, t) in [("flip", c.boost_s), ("brake", c.brake_s)] {
                let (before, after) = (c.at(t - eps), c.at(t + eps));
                let jump = (after.position_ly - before.position_ly).length();
                // Two milliseconds of coasting, generously: a light-second is 3e-8 ly.
                assert!(jump < 1.0e-9, "at beta {along} the {name} jumped {jump} ly");
                let shear = (after.beta - before.beta).length();
                assert!(shear < 1.0e-6, "at beta {along} the {name} sheared {shear}");
            }
            let end = c.at(c.duration_s());
            assert!((end.position_ly - DVec3::X * 4.0).length() < 1e-6, "{:?}", end.position_ly);
            assert!(end.beta.length() < 1e-9, "{:?}", end.beta);
        }
    }

    /// A ship already going too fast to stop overruns, and the flip is part of why: it closes
    /// the whole time it is coming about, before the brake can do anything at all.
    #[test]
    fn a_ship_too_fast_to_stop_overruns_by_the_flip_as_well() {
        let beta0 = DVec3::X * 0.9;
        let near = DVec3::X * 1.0e-6;
        let c = Cruise::plan_from(DVec3::ZERO, beta0, near, 0.0, Drive::DEFAULT);
        let end = c.at(c.duration_s());
        assert!(end.position_ly.x > near.x, "it must overshoot, not stop short");
        assert!(end.beta.length() < 1e-9, "but it does stop: {:?}", end.beta);
        // It cannot brake until it has turned, and it is still doing 0.9 c while it turns.
        let drifted = 0.9 * Drive::DEFAULT.flip_s() / JULIAN_YEAR_S;
        assert!(end.position_ly.x > drifted, "{} ly is less than the {drifted} ly of the flip", end.position_ly.x);
        assert_eq!(c.boost_s, 0.0, "there is nothing to boost: it is already past the cap it needs");
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

