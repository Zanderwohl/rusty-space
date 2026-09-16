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

/// How many times the line is re-aimed to allow for where the last burn carries the ship.
///
/// An injection does not run along the crossing's line — it is turning the ship's velocity as
/// well as killing it — so the line has to end where the injection *starts*, and be tilted
/// against the ground the injection makes across it. Both depend on how fast the ship will be
/// going, which depends on the line.
///
/// The solve inside a round gets the part along the line exactly right, so what iterates is only
/// the tilt, and it squares each round: over a thirty-five-thousand-kilometre transfer the miss
/// goes two kilometres, two metres, two millimetres, two microns. Four rounds holds every case
/// this is offered for to a few metres, which is what [`crate::navigation::ARRIVAL_ROUNDS`]
/// holds a planet to.
const AIM_ROUNDS: usize = 4;

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
        Self::plan_from(from_ly, DVec3::ZERO, to_ly, start_s, drive)
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
        start_s: f64,
        drive: Drive,
    ) -> Self {
        Self::solve(from_ly, beta0, to_ly, arrive_beta, start_s, drive)
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
        Self::solve(from_ly, beta0, to_ly, DVec3::ZERO, start_s, drive)
    }

    fn solve(
        from_ly: DVec3,
        beta0: DVec3,
        to_ly: DVec3,
        arrive_beta: DVec3,
        start_s: f64,
        drive: Drive,
    ) -> Self {
        let alpha = drive.alpha();
        let cap = drive.cap();
        let ordered_from = from_ly;
        let ordered_to = to_ly;

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
            // the profile it was paired with misses by tens of kilometres on a transfer of tens
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
        // stops after [`AIM_ROUNDS`] and leaves a couple of metres on a transfer of tens of
        // thousands of kilometres. Keeping the *order* is what lets a crossing be re-planned
        // from its recipe and come out the same crossing rather than one aimed two metres
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

    /// Coordinate seconds the whole crossing takes, the match included.
    pub fn duration_s(&self) -> f64 {
        self.match_s + self.arrive_s
    }

    /// Ship seconds the whole crossing takes. Never more than [`Cruise::duration_s`].
    pub fn proper_duration_s(&self) -> f64 {
        self.proper_s
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

        // Vectors rather than a distance along the line, because the last burn need not run
        // along it: an injection is turning the ship's velocity as well as killing it.
        let (travelled_ls, beta, proper_s, phase) = if t < self.boost_s {
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
            position_ly: self.matched_ly + travelled_ls / JULIAN_YEAR_S,
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

    const M_PER_LY: f64 = 9.4607304725808e15;

    /// A transfer between two orbits of one body, as the numbers actually are: thirty-five
    /// thousand kilometres, and a station going a few kilometres a second across the line.
    fn transfer(span_m: f64, station_m_s: DVec3) -> (Cruise, DVec3, DVec3) {
        let to = DVec3::X * (span_m / M_PER_LY);
        let onto = station_m_s / C_M_S;
        (Cruise::plan_onto(DVec3::ZERO, DVec3::ZERO, to, onto, 0.0, Drive::DEFAULT), to, onto)
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
        assert!(miss_m < 10.0, "{miss_m} m off a thirty-five-thousand-kilometre transfer");
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
        let onto = Cruise::plan_onto(DVec3::ZERO, DVec3::ZERO, to, DVec3::ZERO, 0.0, Drive::DEFAULT);
        assert_eq!(onto, Cruise::plan_from(DVec3::ZERO, DVec3::ZERO, to, 0.0, Drive::DEFAULT));
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


