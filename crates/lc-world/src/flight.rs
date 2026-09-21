//! Interstellar crossings under constant proper acceleration.
//!
//! Units are `c = 1`: lengths in light-seconds, times in seconds, so a speed is a bare
//! fraction and `alpha` is an inverse time. The ship boosts at a fixed proper acceleration,
//! flips at the midpoint and brakes symmetrically; if it would pass the drive's speed cap on
//! the way it levels off and coasts instead.
//!
//! **Every crossing coasts.** The flip is a turn and a turn takes time — see [`crate::attitude`]
//! — so the plan holds the drive off for at least [`Drive::flip_s`] between the boost and the
//! brake, and the ship covers that ground at its peak speed. For a five-hundred-meter hull it is
//! a minute in the middle of a journey of years; for a fifty-kilometer one it is nearly two
//! hours, and for a short hop it is most of the trip.

use glam::DVec3;

pub use crate::injection::{INJECTION_MAX_BETA, Injection};

// What a crossing has cost so far. A child module so it can read the plan's own phases.
mod lit;

/// Standard gravity, m/s^2.
pub const G0: f64 = 9.80665;

/// Meters per second.
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
    /// How fast it throws its reaction mass, meters a second.
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
    /// Coming about to face the first burn, before anything is lit. See [`Cruise::turn_s`].
    Turn,
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

/// How many times the line is re-aimed to allow for where the last burn carries the ship.
///
/// An injection does not run along the crossing's line — it is turning the ship's velocity as
/// well as killing it — so the line has to end where the injection *starts*, and be tilted
/// against the ground the injection makes across it. Both depend on how fast the ship will be
/// going, which depends on the line.
///
/// The solve inside a round gets the part along the line exactly right, so what iterates is only
/// the tilt, and it squares each round: over a thirty-five-thousand-kilometer transfer the miss
/// goes two kilometers, two meters, two millimeters, two microns. Four rounds holds every case
/// this is offered for to a few meters, which is what [`crate::navigation::ARRIVAL_ROUNDS`]
/// holds a planet to.
const AIM_ROUNDS: usize = 4;

/// How finely the turn-in is timed against the line it is turning onto.
///
/// A ship drifts while it comes about, so where the crossing starts depends on how long the turn
/// takes — and which way it has to turn depends on where the crossing starts. Iterating that
/// converges only while the drift is small next to the distance: a fifty-kilometer hull flipping
/// for two hours over a three-light-second hop moves a fifth of the way there while it turns,
/// and the iteration walks away instead of settling.
///
/// So it is bisected instead. The turn cannot last longer than a flip and cannot take less than
/// no time, and the answer is a turn time somewhere between — so the bracket is
/// `[0, flip]` whatever the geometry, the sign change is guaranteed, and no case diverges.
/// Cheap: deciding which way the first burn points needs the line and the ship's velocity, not
/// the profile, so all of this runs before the expensive part runs once.
const TURN_STEPS: usize = 48;

/// How many times the timed turn is re-checked against the heading the crossing is actually
/// flown along. See [`Cruise::solve`].
const TURN_CORRECTIONS: usize = 3;

/// How far a correction may move the turn, as a fraction of a flip.
///
/// The correction exists for the *tilt*: a crossing that ends on a station flies a line aimed off
/// the target, by a couple of degrees at the most, and the bisection above times the turn against
/// the straight one. A couple of degrees is a few per cent of a flip, so a correction larger than
/// this is not the tilt — it is the drift, in the geometry where iterating the drift walks away
/// instead of settling, and there the bisected answer is the one to trust.
const TURN_CORRECTION_LIMIT: f64 = 0.1;

/// What one aiming round works out about a line. See [`Cruise::along`].
#[derive(Clone, Copy, Debug, Default)]
struct Solved {
    /// Along the line, as far as the last burn's beginning — not as far as the target.
    distance_ls: f64,
    t0_s: f64,
    peak_s: f64,
    boost_s: f64,
    boost_ls: f64,
    coast_s: f64,
    coast_beta: f64,
    inject: Option<Injection>,
    /// The ship was already going too fast to stop where it was asked to, and this is the
    /// shorter flight that says where it does stop. See [`Cruise::along`].
    clamped: bool,
}

/// One planned crossing: boost, optional coast, brake.
///
/// The plan is computed once and then only sampled, so the trajectory does not drift with
/// the frame rate and a paused or fast-forwarded clock lands in the same place.
#[derive(Clone, Debug, PartialEq)]
pub struct Cruise {
    pub from_ly: DVec3,
    pub to_ly: DVec3,
    /// The velocity the crossing ends on: the station's, where there is one to join, and zero
    /// where the plan would not carry one.
    ///
    /// A parameter like [`Cruise::initial_beta`] and on the wire for the same reason — the plan
    /// is re-made from it at the far end, never shipped solved. Which is why it records what the
    /// crossing settled on rather than what it was handed: re-planning from an ask that was
    /// refused would make a different crossing at the far end.
    arrive_beta: DVec3,
    /// The velocity the ship had when this was planned.
    ///
    /// Kept because it is a *parameter* of the plan and everything below is derived from it:
    /// a crossing is put back on the wire as the five values [`Cruise::plan_from`] takes, and
    /// re-planned at the far end rather than shipped as a solved trajectory. See
    /// [`crate::resume`].
    beta0: DVec3,
    /// Which way the nose was pointing when this was ordered.
    ///
    /// A plan parameter, because the crossing has to leave time for the ship to come about
    /// before it lights anything — see [`Cruise::turn_s`]. Zero means *unknown*, and buys no
    /// turn at all: a caller that cannot say where the nose was does not get to have the answer
    /// invented for it.
    attitude0: DVec3,
    /// Coordinate seconds at which the order was given. The first burn lights `turn_s` later.
    pub start_s: f64,
    pub drive: Drive,
    direction: DVec3,
    distance_ls: f64,
    alpha: f64,
    /// Coming about to face the first burn. Nothing is lit and the ship drifts at `beta0`.
    turn_s: f64,
    /// Where the turn leaves the ship, which is where the crossing proper begins.
    turned_ly: DVec3,
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
    /// Where on the rest profile the drive goes out, and the speed there.
    peak_s: f64,
    coast_beta: f64,
    /// The last burn, when it is one burn onto a moving station rather than a brake to rest.
    inject: Option<Injection>,
    /// Ship seconds at the end of boost, across the coast, and for the whole crossing.
    boost_proper_s: f64,
    coast_proper_s: f64,
    proper_s: f64,
}

impl Cruise {
    /// Plan a crossing. A zero-length one is already arrived.
    pub fn plan(from_ly: DVec3, to_ly: DVec3, start_s: f64, drive: Drive) -> Self {
        Self::plan_from(from_ly, DVec3::ZERO, to_ly, DVec3::ZERO, start_s, drive)
    }

    /// Plan a crossing that ends **on a station's velocity** rather than at rest.
    ///
    /// A ship arriving at an orbit is joining something that is already moving, and the last
    /// burn is aimed at the difference: what kills the speed it came in with and what gives it
    /// the speed it is joining, added together, held at one angle for one burn. It arrives
    /// alongside its station and moving with it, which is what [`Injection`] costs and what
    /// arriving at rest and picking the velocity up between two samples did not.
    ///
    /// `arrive_beta` of zero is [`Cruise::plan_from`] exactly. So is anything the form cannot
    /// honestly carry — see [`INJECTION_MAX_BETA`] — because a crossing fast enough to need the
    /// hyperbolic brake is one this approximation would lie about.
    pub fn plan_onto(
        from_ly: DVec3,
        beta0: DVec3,
        to_ly: DVec3,
        arrive_beta: DVec3,
        attitude0: DVec3,
        start_s: f64,
        drive: Drive,
    ) -> Self {
        Self::solve(from_ly, beta0, to_ly, arrive_beta, attitude0, start_s, drive)
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
        attitude0: DVec3,
        start_s: f64,
        drive: Drive,
    ) -> Self {
        Self::solve(from_ly, beta0, to_ly, DVec3::ZERO, attitude0, start_s, drive)
    }

    /// Time the turn-in, then fly the crossing that follows it.
    ///
    /// **A ship cannot thrust in a direction it is not facing**, so before any of what
    /// [`Cruise::with_turn`] does, it swings to face its first burn — drifting, with nothing lit.
    /// That moves where the crossing starts, which moves the line, which moves the heading the
    /// nose has to reach. Two passes, and they are not the same kind of pass:
    ///
    /// The first bisects. Iterating the drift converges only while the drift is small next to
    /// the distance, and a fifty-kilometer hull flipping for two hours over a three-light-second
    /// hop covers a fifth of the way there while it turns — the iteration walks away instead of
    /// settling. Bisection cannot: the turn lasts somewhere between no time and a flip, so
    /// `[0, flip]` brackets it whatever the geometry.
    ///
    /// The rest correct, because the line the crossing is actually flown along is not the
    /// straight one to the target — an injection tilts it, by a couple of degrees at the most.
    /// Settling on the heading really flown is what makes "nothing is lit until the nose is
    /// round" true rather than nearly true, and it is also what lets a nose already pointed the
    /// right way cost nothing at all. [`TURN_CORRECTION_LIMIT`] is what keeps it from wandering
    /// off into the geometry the bisection was for.
    fn solve(
        from_ly: DVec3,
        beta0: DVec3,
        to_ly: DVec3,
        arrive_beta: DVec3,
        attitude0: DVec3,
        start_s: f64,
        drive: Drive,
    ) -> Self {
        // Where the nose has to end up if the ship drifts for `t` first, and how long that swing
        // takes. Only the line and `beta0` decide which way the first burn points, so this is
        // arithmetic rather than a profile solve — which is what makes bisecting it cheap.
        let turn_for = |t: f64| {
            let drifted = from_ly + beta0 * (t / JULIAN_YEAR_S);
            let line = (to_ly - drifted).normalize_or_zero();
            let across = beta0 - line * beta0.dot(line);
            // The match is the first thing lit when there is drift to shed, and it points against
            // that drift; otherwise the boost is, and it points down the line.
            let first = if across.length() > 1.0e-12 { -across.normalize_or_zero() } else { line };
            crate::attitude::turn_time_s(attitude0, first, drive.slew_rate_rad_s)
        };
        // No turn to make — the nose is already round, or the caller did not say where it was —
        // is answered exactly rather than bisected to a picosecond of one. It is the common case,
        // and a crossing that turns for no time should be the crossing it was before this existed.
        if turn_for(0.0) <= 0.0 {
            return Self::with_turn(
                from_ly, beta0, to_ly, arrive_beta, attitude0, 0.0, start_s, drive,
            );
        }
        let (mut lo, mut hi) = (0.0, drive.flip_s());
        for _ in 0..TURN_STEPS {
            let mid = 0.5 * (lo + hi);
            if turn_for(mid) > mid {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let mut turn_s = 0.5 * (lo + hi);
        let mut out =
            Self::with_turn(from_ly, beta0, to_ly, arrive_beta, attitude0, turn_s, start_s, drive);
        for _ in 0..TURN_CORRECTIONS {
            let heading = out.aim_at(start_s).to;
            let needed = crate::attitude::turn_time_s(attitude0, heading, drive.slew_rate_rad_s);
            if needed == turn_s
                || (needed - turn_s).abs() > TURN_CORRECTION_LIMIT * drive.flip_s()
            {
                break;
            }
            turn_s = needed;
            out = Self::with_turn(
                from_ly, beta0, to_ly, arrive_beta, attitude0, turn_s, start_s, drive,
            );
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn with_turn(
        from_ly: DVec3,
        beta0: DVec3,
        to_ly: DVec3,
        arrive_beta: DVec3,
        attitude0: DVec3,
        turn_s: f64,
        start_s: f64,
        drive: Drive,
    ) -> Self {
        let alpha = drive.alpha();
        let cap = drive.cap();
        let ordered_from = from_ly;
        let ordered_to = to_ly;

        let from_ly = ordered_from + beta0 * (turn_s / JULIAN_YEAR_S);
        let turned_ly = from_ly;

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

        // The whole reach: from where the match leaves the ship to where the crossing has to end
        // up, light-seconds. The *line* is shorter than this whenever there is an injection,
        // because the last burn does not travel along it — see [`AIM_ROUNDS`].
        let reach_ls = (to_ly - from_ly) * JULIAN_YEAR_S;

        // Speed and distance at which the boost would reach the cap.
        let gamma_cap = (1.0 - cap * cap).sqrt().recip();
        let cap_ls = (gamma_cap - 1.0) / alpha;
        let t_cap = gamma_cap * cap / alpha;

        // How long the ship spends pointing neither way. The brake cannot light until the flip
        // is over, so this is a floor on the coast and a term in the distance the crossing
        // covers — not an adjustment made afterwards.
        let flip_s = drive.flip_s();

        let mut direction = reach_ls.normalize_or_zero();
        let aim = |direction| {
            Self::along(direction, reach_ls, beta0, arrive_beta, alpha, cap, cap_ls, t_cap, flip_s)
        };
        let mut solved = aim(direction);
        for _ in 1..AIM_ROUNDS {
            // Re-aim, by the part of the injection that runs **across** the line and only that
            // part. What runs along it is already in the solve above, and taking the whole
            // displacement off shortens the line as well as tilting it — which over-rotates by
            // about a factor of two and leaves the iteration oscillating instead of converging.
            //
            // The line and the profile are re-solved together. A direction one round ahead of
            // the profile it was paired with misses by tens of kilometers on a transfer of tens
            // of thousands.
            let Some(inject) = solved.inject else { break };
            let ran = inject.displacement_ls();
            let across = ran - direction * ran.dot(direction);
            let next = (reach_ls - across).normalize_or_zero();
            if next == direction || next == DVec3::ZERO {
                break;
            }
            direction = next;
            solved = aim(direction);
        }
        let Solved { distance_ls, t0_s, peak_s, boost_s, boost_ls, coast_s, coast_beta, inject, .. } =
            solved;

        // **The point asked for, not the point reached.** They differ, because the aiming above
        // stops after [`AIM_ROUNDS`] and leaves a couple of meters on a transfer of tens of
        // thousands of kilometers. Keeping the *order* is what lets a crossing be re-planned
        // from its recipe and come out the same crossing rather than one aimed two meters
        // further on each time — see [`crate::resume`].
        //
        // A clamped crossing is the exception and has to report where it really ends, because
        // there the whole answer is that the ship could not stop where it was told to.
        let to_ly = if solved.clamped {
            let ends_ls = direction * distance_ls
                + match &inject {
                    Some(inject) => inject.displacement_ls(),
                    None => direction * distance_of(alpha, peak_s),
                };
            from_ly + ends_ls / JULIAN_YEAR_S
        } else {
            ordered_to
        };

        let boost_proper_s = proper_of(alpha, t0_s + boost_s) - proper_of(alpha, t0_s);
        let coast_proper_s = coast_s * (1.0 - coast_beta * coast_beta).sqrt();
        // The last burn. Braking to rest starts at the peak and ends there, so it ages the crew
        // by the whole of the rest profile up to the peak — which is *not* the boost's share
        // back again unless the ship began at rest.
        let (brake_s_len, brake_proper_s) = match &inject {
            Some(inject) => (inject.duration_s(), inject.proper_s(inject.duration_s())),
            None => (peak_s, proper_of(alpha, peak_s)),
        };

        Self {
            from_ly: ordered_from,
            to_ly,
            beta0,
            // What the crossing *does*, not what was asked of it. A plan that fell back to the
            // exact brake ends at rest, and a recipe saying otherwise would re-plan into a
            // crossing its own sampler disagreed with — see `Cruise::along`.
            arrive_beta: inject.map(|inject| inject.arrive_beta()).unwrap_or(DVec3::ZERO),
            attitude0,
            start_s,
            drive,
            direction,
            distance_ls,
            alpha,
            turn_s,
            turned_ly,
            match_s,
            match_dir,
            match_ls,
            match_along: along_ordered,
            matched_ly: from_ly,
            t0_s,
            boost_s,
            brake_s: boost_s + coast_s,
            arrive_s: boost_s + coast_s + brake_s_len,
            boost_ls,
            peak_s,
            coast_beta,
            inject,
            boost_proper_s,
            coast_proper_s,
            proper_s: boost_proper_s + coast_proper_s + brake_proper_s,
        }
    }

    /// Solve the profile for a line already chosen: how long to boost, how long to coast, and
    /// what the last burn is.
    ///
    /// Split out because the injection makes the line's *direction* part of the answer rather
    /// than an input — the last burn runs off the line — so this is what an aiming round calls.
    #[allow(clippy::too_many_arguments)]
    fn along(
        direction: DVec3,
        reach_ls: DVec3,
        beta0: DVec3,
        arrive_beta: DVec3,
        alpha: f64,
        cap: f64,
        cap_ls: f64,
        t_cap: f64,
        flip_s: f64,
    ) -> Solved {
        // Signed speed along the line, once the ship is on it.
        let along = beta0.dot(direction).clamp(-MAX_BETA, MAX_BETA);
        // `sinh(phi) = gamma * beta`, so this is where the rest profile is already at `along`.
        let t0_s = along / (1.0 - along * along).sqrt() / alpha;
        // Even in `t0`: a profile run backwards covers the same ground.
        let x0 = distance_of(alpha, t0_s);

        // **The last burn, as a function of where the drive goes out.** Braking to rest is the
        // rest profile run backwards and covers `x(peak)` along the line. An injection is not
        // along the line at all, so what the line has to leave room for is its shadow on it.
        let joining = arrive_beta.length();
        let injecting = joining > 0.0 && joining < INJECTION_MAX_BETA;
        let injection =
            |peak_s: f64| Injection::new(alpha, direction * beta_of(alpha, peak_s), arrive_beta);
        let terminal = |peak_s: f64| -> f64 {
            if injecting {
                injection(peak_s).displacement_ls().dot(direction)
            } else {
                distance_of(alpha, peak_s)
            }
        };
        let give_up = || {
            Self::along(direction, reach_ls, beta0, DVec3::ZERO, alpha, cap, cap_ls, t_cap, flip_s)
        };

        // Too fast to stop in what is left: the shortest flight from here is to turn round and
        // brake the whole way, and it ends past the target. Saying so is better than pretending
        // a drive can do what it cannot — the ship stops where it actually stops. The flip is
        // part of "what it cannot": a ship still coming about is a ship still closing.
        let least_ls = along.max(0.0) * flip_s + terminal(t0_s);
        let asked_ls = reach_ls.dot(direction);
        let clamped = t0_s > 0.0 && asked_ls < least_ls;
        let want_ls = if clamped { least_ls } else { asked_ls };
        if want_ls <= 0.0 {
            return Solved { t0_s, ..Solved::default() };
        }

        // The ship has to outrun what it is joining before any of this holds: below that speed
        // the last burn is an acceleration rather than a brake, and the equation stops rising
        // with the boost. Bisecting from there keeps it on the branch that is monotone.
        let floor_s =
            if injecting { joining / (1.0 - joining * joining).sqrt() / alpha } else { 0.0 };
        let reach = |peak_s: f64| {
            distance_of(alpha, peak_s) + beta_of(alpha, peak_s) * flip_s + terminal(peak_s)
        };
        if injecting && reach(floor_s) > want_ls + x0 {
            // A hop shorter than the orbit it ends on. The ship would never get ahead of the
            // station, so there is no one angle that does both jobs.
            return give_up();
        }

        // Boost, coast and the last burn together cover the line:
        //
        //     x(peak) - x(t0)  +  beta(peak) * coast  +  terminal(peak)  =  D
        //
        // `peak` being where the ship is on the rest profile when the drive goes out. Rising in
        // `peak` on every term above the floor, which is what lets [`peak_time`] bisect it.
        let at_cap_ls = want_ls + x0 - (cap_ls + terminal(t_cap));
        let (peak_s, coast_s, coast_beta) = if !injecting && at_cap_ls >= cap * flip_s {
            (t_cap, at_cap_ls / cap, cap)
        } else {
            let peak_s = peak_time(want_ls + x0, floor_s, t_cap, &reach);
            (peak_s, flip_s, beta_of(alpha, peak_s))
        };
        // Too fast for a linear ramp to be the truth. Brake to rest the exact way instead and
        // let the arrival pick the station's velocity up, which is what every crossing did
        // before this form existed.
        if injecting && coast_beta > INJECTION_MAX_BETA {
            return give_up();
        }

        Solved {
            distance_ls: want_ls - terminal(peak_s),
            t0_s,
            peak_s,
            boost_s: (peak_s - t0_s).max(0.0),
            boost_ls: distance_of(alpha, peak_s) - x0,
            coast_s,
            coast_beta,
            inject: injecting.then(|| injection(peak_s)),
            clamped,
        }
    }

    /// The velocity this was planned from. With the other four public fields, the whole recipe.
    pub fn initial_beta(&self) -> DVec3 {
        self.beta0
    }

    /// The instants its phases change, on its own clock: the burn lights, the match gives way
    /// to the boost, the drive goes out for the flip, lights for the brake, and goes out on
    /// arrival. Coincident where a phase has no length.
    pub fn phase_changes_s(&self) -> [f64; 5] {
        let lit = self.start_s + self.turn_s;
        let line = lit + self.match_s;
        [lit, line, line + self.boost_s, line + self.brake_s, line + self.arrive_s]
    }

    /// Coordinate seconds the whole crossing takes, the turn-in and the match included.
    pub fn duration_s(&self) -> f64 {
        self.turn_s + self.match_s + self.arrive_s
    }

    /// Ship seconds the whole crossing takes. Never more than [`Cruise::duration_s`].
    ///
    /// Read off the end of the trajectory rather than summed from the parts, so it cannot
    /// disagree with what [`Cruise::at`] says the crew's clock reads on arrival. Summing them was
    /// how the match's share went missing.
    pub fn proper_duration_s(&self) -> f64 {
        self.at(self.start_s + self.duration_s()).proper_s
    }

    /// How long the ship spends coming about before its first burn, seconds.
    ///
    /// Nothing is lit and the ship drifts at the velocity it began with. Zero when the nose was
    /// already where the burn needs it, and zero when the planner was not told where the nose
    /// was — see the `attitude0` this was planned from.
    pub fn turn_s(&self) -> f64 {
        self.turn_s
    }

    /// Which way the nose was pointing when this was ordered. With the others, the whole recipe.
    pub fn initial_attitude(&self) -> DVec3 {
        self.attitude0
    }

    /// Which way the last burn points.
    ///
    /// Straight back down the line for a crossing that ends at rest. For one that ends on a
    /// station it is [`Injection::aim`] — the one angle that kills the speed the ship came in
    /// with and imparts the one it is joining.
    pub fn last_aim(&self) -> DVec3 {
        match &self.inject {
            Some(inject) => inject.aim(),
            None => -self.direction,
        }
    }

    /// The velocity the crossing ends on. Zero unless it is joining a station.
    pub fn arrive_beta(&self) -> DVec3 {
        self.arrive_beta
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
        // **The turn-in and the burn it is for are one order.** The ship was told where to point
        // the moment the crossing began, and the burn lights when it gets there — so the aim is
        // the same through both, and `from: None` hands the turn its start: whatever the ship was
        // already doing, which only the ship knows.
        if self.match_s > 0.0 {
            // Shedding the velocity across the line: the drive points against it, and this is
            // the first thing the crossing asks for.
            if since < self.turn_s + self.match_s {
                return Aim { to: -self.match_dir, from: None, since_s: self.start_s };
            }
        }
        let after_match = self.start_s + self.turn_s + self.match_s;
        // A ship that had no match to fly was never told anything before the boost — so the
        // boost *is* its first order, given when the crossing began rather than when the drive
        // lit, and the turn-in is the time it took to obey it.
        let before_boost = (self.match_s > 0.0).then_some(-self.match_dir);
        let t = since - self.turn_s - self.match_s;
        if t < self.boost_s {
            let since_s = if self.match_s > 0.0 { after_match } else { self.start_s };
            return Aim { to: self.direction, from: before_boost, since_s };
        }
        // Everything from the end of the boost onward is one order — turn around and brake —
        // so the turn is not restarted at the moment the drive relights.
        Aim {
            to: self.last_aim(),
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
            Phase::Brake => self.last_aim(),
            Phase::Turn | Phase::Coast | Phase::Arrived => DVec3::ZERO,
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
        let ordered = now_s - self.start_s;
        // **Coming about**, with nothing lit: a straight drift at whatever the ship had.
        let inv_gamma0 = (1.0 - self.beta0.length_squared()).max(0.0).sqrt();
        if ordered < self.turn_s {
            let t = ordered.max(0.0);
            return FlightState {
                position_ly: self.from_ly + self.beta0 * (t / JULIAN_YEAR_S),
                beta: self.beta0,
                proper_s: t * inv_gamma0,
                phase: Phase::Turn,
            };
        }
        // The crew aged through the turn, and everything below is measured from its end.
        let turned_proper = self.turn_s * inv_gamma0;
        let since = ordered - self.turn_s;
        // The match comes next, in its own direction. See `plan_from`.
        if since < self.match_s {
            let left = self.match_s - since.max(0.0);
            let across_ls = self.match_ls - distance_of(self.alpha, left);
            let along_ls = self.match_along * since.max(0.0);
            return FlightState {
                position_ly: self.turned_ly
                    + (self.match_dir * across_ls + self.direction * along_ls) / JULIAN_YEAR_S,
                beta: self.match_dir * beta_of(self.alpha, left) + self.direction * self.match_along,
                proper_s: turned_proper + proper_of(self.alpha, self.match_s)
                    - proper_of(self.alpha, left),
                phase: Phase::Match,
            };
        }
        let matched_proper = turned_proper + proper_of(self.alpha, self.match_s);
        let t = (since - self.match_s).clamp(0.0, self.arrive_s);
        if self.arrive_s <= 0.0 {
            return FlightState {
                position_ly: self.to_ly,
                beta: DVec3::ZERO,
                proper_s: matched_proper,
                phase: Phase::Arrived,
            };
        }

        // Vectors rather than a distance along the line, because the last burn need not run
        // along it: an injection is turning the ship's velocity as well as killing it.
        let (traveled_ls, beta, proper_s, phase) = if t < self.boost_s {
            // Offset onto the rest profile: this ship entered that trajectory at `t0`. See
            // `plan_from`.
            let on = self.t0_s + t;
            (
                self.direction * (distance_of(self.alpha, on) - distance_of(self.alpha, self.t0_s)),
                self.direction * beta_of(self.alpha, on),
                proper_of(self.alpha, on) - proper_of(self.alpha, self.t0_s),
                Phase::Boost,
            )
        } else if t < self.brake_s {
            let c = t - self.boost_s;
            let inv_gamma = (1.0 - self.coast_beta * self.coast_beta).sqrt();
            (
                self.direction * (self.boost_ls + self.coast_beta * c),
                self.direction * self.coast_beta,
                self.boost_proper_s + c * inv_gamma,
                Phase::Coast,
            )
        } else {
            let phase = if t >= self.arrive_s { Phase::Arrived } else { Phase::Brake };
            match &self.inject {
                // One burn at one angle, off the end of the line. See [`Injection`].
                Some(inject) => {
                    let (offset_ls, beta) = inject.at(t - self.brake_s);
                    (
                        self.direction * self.distance_ls + offset_ls,
                        beta,
                        self.boost_proper_s + self.coast_proper_s + inject.proper_s(t - self.brake_s),
                        phase,
                    )
                }
                // The rest profile run backwards from the peak. Symmetric with the boost only
                // for a ship that started at rest; for any other it is the longer half.
                None => {
                    let s = self.arrive_s - t;
                    (
                        self.direction
                            * (self.distance_ls + distance_of(self.alpha, self.peak_s)
                                - distance_of(self.alpha, s)),
                        self.direction * beta_of(self.alpha, s),
                        self.proper_s - proper_of(self.alpha, s),
                        phase,
                    )
                }
            }
        };

        FlightState {
            // From the matched point, which is where the line begins. For a ship that started
            // on the line already, that is where it started.
            position_ly: self.matched_ly + traveled_ls / JULIAN_YEAR_S,
            beta,
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

/// Where on the rest profile the drive goes out, given everything the crossing has to fit in.
///
/// Solves `reach(t) = k`, where `reach` is the ground the whole crossing covers with the drive
/// going out at `t` — boost, the coast the flip needs, and the last burn — and `k` is the
/// crossing's distance plus the ground the ship had already made on that profile. Every term
/// rises with `t` on the branch this is bracketed over, so the left side is strictly increasing:
/// there is exactly one root, bisection cannot land on the wrong one, and no starting guess can
/// send it somewhere else. Newton would converge faster and would also have to be nursed through
/// the cap, where `beta` flattens out.
///
/// With neither a coast nor an injection this inverts in closed form — `(at)^2 = k^2 + 2k` for
/// `k = a x` — which is what the solver did before turning cost anything. With them the equation
/// is a quartic in `at` at best, and a quartic's radicals are a worse numerical object than
/// fifty bisections.
///
/// A fixed iteration count, not a tolerance. Both ends of the wire plan the same crossing from
/// the same recipe, and a loop that stops when it is close enough is a loop that can stop one
/// step later somewhere else.
fn peak_time(k: f64, lower_s: f64, upper_s: f64, reach: &dyn Fn(f64) -> f64) -> f64 {
    let (mut lo, mut hi) = (lower_s.max(0.0), upper_s.max(lower_s).max(1.0));
    // The caller's bound is the time at the speed cap, which brackets the root whenever this is
    // reached at all. Widening is for the callers that have not checked.
    for _ in 0..64 {
        if reach(hi) >= k {
            break;
        }
        hi *= 2.0;
    }
    // Enough halvings to exhaust an f64 over any bracket this can be handed; the last few are
    // no-ops once `lo` and `hi` are neighbors.
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
            "a fifty-kilometer hull's hop is more than half flip: {} s of {} s",
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
            let c = Cruise::plan_from(DVec3::ZERO, DVec3::X * along, DVec3::X * 4.0, DVec3::ZERO, 0.0, Drive::DEFAULT);
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
        let c = Cruise::plan_from(DVec3::ZERO, beta0, near, DVec3::ZERO, 0.0, Drive::DEFAULT);
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

    const M_PER_LY: f64 = 9.4607304725808e15;

    const RATE: f64 = crate::attitude::RATE_RAD_S;

    /// Nothing is lit until the nose is round. A crossing that has to turn end for end spends a
    /// minute doing it, drifting, with the drive off.
    #[test]
    fn a_crossing_comes_about_before_it_lights_anything() {
        let c = Cruise::plan_from(
            DVec3::ZERO,
            DVec3::ZERO,
            -DVec3::X * 4.0,
            DVec3::X,
            0.0,
            Drive::DEFAULT,
        );
        assert!((c.turn_s() - 60.0).abs() < 1.0e-9, "a flip in {} s", c.turn_s());
        for t in [0.0, 15.0, 30.0, 59.9] {
            assert_eq!(c.at(t).phase, Phase::Turn, "at {t} s");
            assert_eq!(c.thrust_at(t), DVec3::ZERO, "the drive was lit at {t} s");
        }
        assert_eq!(c.at(60.1).phase, Phase::Boost);
        assert_ne!(c.thrust_at(60.1), DVec3::ZERO);
    }

    /// And the turn is over exactly when the burn starts — the whole point of paying for it.
    /// Whatever the hull, whatever it was pointing at, and whether the first burn is the match
    /// or the boost.
    #[test]
    fn the_nose_has_arrived_when_the_first_burn_lights() {
        let cases = [
            ("flip, at rest", DVec3::ZERO, DVec3::X, -DVec3::X * 4.0),
            ("square on, at rest", DVec3::ZERO, DVec3::Y, DVec3::X * 4.0),
            ("skew, at rest", DVec3::ZERO, DVec3::new(1.0, 1.0, 1.0).normalize(), DVec3::X * 4.0),
            ("into a match", DVec3::Y * 1.0e-4, -DVec3::X, DVec3::X * 1.0e-7),
        ];
        for length_m in [500.0, 5_000.0, 50_000.0] {
            let drive = Drive {
                slew_rate_rad_s: crate::attitude::rate_rad_s(length_m),
                ..Drive::DEFAULT
            };
            for (what, beta0, attitude0, to) in cases {
                let c = Cruise::plan_from(DVec3::ZERO, beta0, to, attitude0, 0.0, drive);
                let lit = c.turn_s();
                let aim = c.aim_at(lit);
                let nose = crate::attitude::turned(
                    aim.from.unwrap_or(attitude0),
                    aim.to,
                    drive.slew_rate_rad_s,
                    lit - aim.since_s,
                );
                let thrust = c.thrust_at(lit + 1.0e-6);
                assert!(
                    (nose - thrust).length() < 1.0e-9,
                    "{what} at {length_m} m: thrusting {thrust} with the nose at {nose}",
                );
            }
        }
    }

    /// Coming about is not stopping. The ship keeps what it had and covers ground doing it,
    /// which is why the turn has to be inside the plan and not bolted on before it.
    #[test]
    fn a_ship_drifts_while_it_comes_about() {
        let beta0 = DVec3::Y * 1.0e-4;
        let c = Cruise::plan_from(
            DVec3::ZERO,
            beta0,
            DVec3::X * 1.0e-7,
            -DVec3::X,
            0.0,
            Drive::DEFAULT,
        );
        assert!(c.turn_s() > 1.0, "premise: there is a turn to make");
        // Up to but not including the end of it, which is already the first burn.
        for k in 0..10 {
            let t = c.turn_s() * k as f64 / 10.0;
            let s = c.at(t);
            assert_eq!(s.beta, beta0, "it changed speed while coasting round");
            let want = beta0 * (t / JULIAN_YEAR_S);
            assert!((s.position_ly - want).length() < 1.0e-18, "at {t} s it was at {:?}", s.position_ly);
        }
        // And the crew aged through it, a shade slower than the clock.
        let aboard = c.at(c.turn_s()).proper_s;
        assert!(aboard < c.turn_s() && aboard > c.turn_s() * 0.999);
    }

    /// The drift is paid for: the crossing still ends where it was asked to, having started from
    /// somewhere else than where the order was given.
    #[test]
    fn the_crossing_still_lands_though_it_drifted_while_turning() {
        let to = DVec3::X * 1.0e-7;
        let beta0 = DVec3::Y * 1.0e-4;
        let c = Cruise::plan_from(DVec3::ZERO, beta0, to, -DVec3::X, 0.0, Drive::DEFAULT);
        let moved = c.at(c.turn_s()).position_ly.length() * 9.4607304725808e15;
        assert!(moved > 1.0e5, "premise: it drifted {moved:e} m while turning");
        let miss = (c.at(c.duration_s()).position_ly - to).length() * 9.4607304725808e15;
        assert!(miss < 1.0, "landed {miss:e} m out");
    }

    /// A nose already where the burn needs it turns for no time at all — and neither does one
    /// the planner was never told about, because an invented answer is worse than none.
    #[test]
    fn a_nose_already_round_costs_nothing_and_an_unknown_one_buys_nothing() {
        let to = DVec3::X * 4.0;
        let ready = Cruise::plan_from(DVec3::ZERO, DVec3::ZERO, to, DVec3::X, 0.0, Drive::DEFAULT);
        assert_eq!(ready.turn_s(), 0.0);
        let unknown =
            Cruise::plan_from(DVec3::ZERO, DVec3::ZERO, to, DVec3::ZERO, 0.0, Drive::DEFAULT);
        assert_eq!(unknown.turn_s(), 0.0);
        // Neither turned, so they fly the same trajectory — they differ only in remembering
        // which way the nose was, which is a parameter rather than a part of the path.
        assert_eq!(ready.duration_s(), unknown.duration_s());
        assert_eq!(ready.at(ready.duration_s()).position_ly, unknown.at(unknown.duration_s()).position_ly);
        assert_eq!(unknown, Cruise::plan(DVec3::ZERO, to, 0.0, Drive::DEFAULT));
    }

    /// When there is a match to fly, the turn is onto *that* — the first thing lit, not the
    /// boost that follows it.
    #[test]
    fn the_turn_in_is_onto_the_match_when_there_is_one() {
        let beta0 = DVec3::Y * 1.0e-4;
        let c = Cruise::plan_from(
            DVec3::ZERO,
            beta0,
            DVec3::X * 1.0e-7,
            DVec3::X,
            0.0,
            Drive::DEFAULT,
        );
        assert_eq!(c.at(c.turn_s() + 1.0e-6).phase, Phase::Match);
        let onto = c.aim_at(0.0).to;
        assert!(onto.dot(beta0) < 0.0, "the match burns against the drift, not {onto}");
        // A right angle from the nose, so half a flip.
        // The turn it charged for is the turn onto the heading it actually flies, not onto an
        // earlier guess at it. See `TURN_ROUNDS`.
        let want = crate::attitude::turn_time_s(DVec3::X, onto, RATE);
        assert!((c.turn_s() - want).abs() < 1.0e-12, "{} s against {want} s", c.turn_s());
    }

    /// The crew's clock covers the whole thing, turn and match included. Summing the parts is
    /// how the match's share went missing from this before.
    #[test]
    fn the_ship_clock_covers_the_turn_and_the_match_too() {
        let c = Cruise::plan_from(
            DVec3::ZERO,
            DVec3::Y * 1.0e-4,
            DVec3::X * 1.0e-7,
            -DVec3::X,
            0.0,
            Drive::DEFAULT,
        );
        assert!(c.turn_s() > 1.0 && c.at(c.turn_s() + 1.0e-6).phase == Phase::Match);
        let end = c.at(c.duration_s());
        assert!(
            (c.proper_duration_s() - end.proper_s).abs() < 1.0e-9,
            "{} s against the {} s the crew actually read",
            c.proper_duration_s(),
            end.proper_s,
        );
        assert!(c.proper_duration_s() > c.turn_s(), "the turn alone is not the whole crossing");
        assert!(c.proper_duration_s() < c.duration_s());
    }

    /// A transfer between two orbits of one body, as the numbers actually are: thirty-five
    /// thousand kilometers, and a station going a few kilometers a second across the line.
    fn transfer(span_m: f64, station_m_s: DVec3) -> (Cruise, DVec3, DVec3) {
        let to = DVec3::X * (span_m / M_PER_LY);
        let onto = station_m_s / C_M_S;
        (Cruise::plan_onto(DVec3::ZERO, DVec3::ZERO, to, onto, DVec3::ZERO, 0.0, Drive::DEFAULT), to, onto)
    }

    /// **The thing this is for.** A crossing planned onto a station ends alongside it *and
    /// moving with it*, having burned for the velocity rather than been given it.
    #[test]
    fn a_crossing_onto_a_station_arrives_moving_with_it() {
        let (c, to, onto) = transfer(35.0e6, DVec3::Y * 3000.0);
        let end = c.at(c.duration_s());
        assert_eq!(end.phase, Phase::Arrived);
        assert_eq!(end.beta, onto, "it must end on the station's velocity exactly");
        let miss_m = (end.position_ly - to).length() * M_PER_LY;
        assert!(miss_m < 10.0, "{miss_m} m off a thirty-five-thousand-kilometer transfer");
    }

    /// One burn, at one angle, doing both jobs: pointing back down the track to kill what the
    /// ship came in with, and across it to impart what it is joining.
    #[test]
    fn the_last_burn_is_one_angle_that_does_both_jobs() {
        let (c, _, onto) = transfer(35.0e6, DVec3::Y * 3000.0);
        let aim = c.last_aim();
        assert!(aim.dot(c.direction) < 0.0, "it has to be braking: {aim} against {}", c.direction);
        assert!(aim.dot(onto) > 0.0, "and imparting: {aim} against {onto}");
        // Which is the vector difference and nothing more: kill the one, add the other.
        let want = (onto - c.direction * c.peak_beta()).normalize();
        assert!((aim - want).length() < 1.0e-12, "{aim} is not {want}");
        // Held, not swept. One angle for the whole burn.
        let lit = c.start_s + c.brake_s;
        for k in 0..=10 {
            let t = lit + (c.duration_s() - c.brake_s) * k as f64 / 10.0;
            assert_eq!(c.thrust_at(t.min(c.start_s + c.duration_s() - 1.0)), aim);
        }
    }

    /// The last burn does not run along the crossing's line, so the line is aimed off the
    /// target to allow for it — tilted against the ground the injection makes across it.
    #[test]
    fn the_line_is_aimed_off_the_target_to_allow_for_the_injection() {
        let (c, to, _) = transfer(35.0e6, DVec3::Y * 3000.0);
        let straight = to.normalize();
        let tilt = c.direction.dot(straight).clamp(-1.0, 1.0).acos().to_degrees();
        assert!(tilt > 1.0 && tilt < 10.0, "the line was tilted {tilt} degrees");
        // And the tilt is *against* the station's drift, not with it.
        assert!(c.direction.y < 0.0, "aimed the wrong side: {}", c.direction);
    }

    /// Prograde, retrograde, across and skew all land, which is what says the aiming converges
    /// rather than happening to work in the one case it was written against.
    #[test]
    fn the_aiming_lands_whichever_way_the_station_is_going() {
        let cases = [
            ("prograde", DVec3::X * 3000.0),
            ("retrograde", DVec3::X * -3000.0),
            ("across", DVec3::Y * 3000.0),
            ("skew", DVec3::new(1000.0, 2500.0, -800.0)),
        ];
        for (what, station) in cases {
            let (c, to, onto) = transfer(35.0e6, station);
            assert!(c.inject.is_some(), "{what} did not get an injection");
            let end = c.at(c.duration_s());
            let miss_m = (end.position_ly - to).length() * M_PER_LY;
            assert!(miss_m < 10.0, "{what} missed by {miss_m} m");
            assert!((end.beta - onto).length() < 1.0e-15, "{what} ended at {:?}", end.beta);
        }
    }

    /// Where the linear ramp would be a lie, the crossing brakes to rest the exact way instead.
    /// An interstellar crossing is the case: it spends its second half above nine tenths of `c`.
    #[test]
    fn a_relativistic_crossing_will_not_take_the_newtonian_injection() {
        let (c, _, _) = transfer(4.0 * M_PER_LY, DVec3::Y * 30_000.0);
        assert!(c.peak_beta() > INJECTION_MAX_BETA, "premise: {}", c.peak_beta());
        assert!(c.inject.is_none(), "it must fall back to the exact brake");
        assert_eq!(c.arrive_beta(), DVec3::ZERO, "and say so, rather than claim an arrival");
        assert!(c.at(c.duration_s()).beta.length() < 1.0e-9, "which means it ends at rest");
    }

    /// And where the ship would never get ahead of what it is joining, there is no one angle
    /// that does both jobs — so there is no injection either.
    #[test]
    fn a_hop_shorter_than_the_orbit_it_joins_will_not_take_one_either() {
        let (c, _, _) = transfer(1000.0, DVec3::Y * 3000.0);
        assert!(c.peak_beta() < 1.0e-5, "premise: it never outruns the station");
        assert!(c.inject.is_none());
    }

    /// Asking for nothing to arrive on is the crossing that was there before, to the bit.
    #[test]
    fn arriving_on_nothing_is_the_crossing_it_always_was() {
        let to = DVec3::X * 4.0;
        let onto = Cruise::plan_onto(DVec3::ZERO, DVec3::ZERO, to, DVec3::ZERO, DVec3::ZERO, 0.0, Drive::DEFAULT);
        assert_eq!(
            onto,
            Cruise::plan_from(DVec3::ZERO, DVec3::ZERO, to, DVec3::ZERO, 0.0, Drive::DEFAULT)
        );
    }

    /// The path through the injection is a path: no jump where it lights, and the crew's clock
    /// keeps running forward and behind the world's.
    #[test]
    fn the_injection_joins_on_and_the_clock_survives_it() {
        let (c, _, _) = transfer(35.0e6, DVec3::Y * 3000.0);
        let eps = 1.0e-3;
        let (before, after) = (c.at(c.brake_s - eps), c.at(c.brake_s + eps));
        let jump_m = (after.position_ly - before.position_ly).length() * M_PER_LY;
        assert!(jump_m < 100.0, "the injection lit with a {jump_m} m jump");
        assert!((after.beta - before.beta).length() < 1.0e-9, "and a velocity step");

        let mut last = -1.0;
        for k in 0..=200 {
            let t = c.duration_s() * k as f64 / 200.0;
            let s = c.at(t);
            assert!(s.proper_s >= last - 1.0e-9, "{last} -> {}", s.proper_s);
            assert!(s.proper_s <= t + 1.0e-9, "aboard {} past coordinate {t}", s.proper_s);
            last = s.proper_s;
        }
    }

    /// The flip still covers the turn into it. Aiming at the difference is a *shorter* swing
    /// than turning end for end, so the coast the crossing already holds open is enough.
    #[test]
    fn the_nose_is_round_before_the_injection_lights() {
        let (c, _, _) = transfer(35.0e6, DVec3::Y * 3000.0);
        let swing = crate::attitude::angle_between(c.direction, c.last_aim());
        assert!(swing <= std::f64::consts::PI, "{swing} rad is more than a flip");
        assert!(
            crate::attitude::turn_time_s(c.direction, c.last_aim(), Drive::DEFAULT.slew_rate_rad_s)
                <= c.coast_s() + 1.0e-9,
            "the turn does not fit in the {} s coast",
            c.coast_s(),
        );
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



