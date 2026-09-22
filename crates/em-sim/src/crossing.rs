//! When a path meets the edge of a sphere of influence.
//!
//! [`influence`](crate::influence) says which attractor owns a point. This says *when* a
//! traveler stops being owned by one and starts being owned by another — the join a patched
//! conic chain is made of.
//!
//! The traveler is a [`Traveler`], not a body index. A body in the system is one
//! ([`BodyPath`]), but so is a spacecraft the system has never heard of, whose arc is held
//! somewhere else entirely. The search needs exactly two things from it: where it is at an
//! arbitrary instant, and how fast it turns. Requiring a `BodyIndex` for those meant a craft
//! could only be given a predicted crossing by first being inserted into the arena.
//!
//! Everything is in meters, in simulation space (right-handed, Z-up, ecliptic of J2000).

use em_foundations::time::{Instant, TimeDelta};
use glam::DVec3;

use crate::id::BodyIndex;
use crate::influence::soi_at;
use crate::motive::MotiveSelection;
use crate::propagate;
use crate::system::System;

/// A path through the system, evaluable at an arbitrary instant.
///
/// Arbitrary is the whole requirement: the search visits times out of order and bisects
/// between them, so a state that is stepped rather than solved cannot be a traveler. That is
/// the same constraint [`propagate::position_at`] states by returning `None` for an integrated
/// body, and the same one `lc_spacetime::Worldline` states for the light-delay solve.
pub trait Traveler {
    /// Position and velocity in simulation space, meters and meters a second.
    fn state_at(&self, time: Instant) -> Option<(DVec3, DVec3)>;

    /// The same, measured from whatever the path is anchored on.
    ///
    /// Only the marker fields of [`Crossing`] use it, and they are zero without it.
    fn local_state_at(&self, _time: Instant) -> Option<(DVec3, DVec3)> {
        None
    }

    /// One revolution, for a closed path. `None` for an open one, or one with no scale.
    fn period_at(&self, _time: Instant) -> Option<TimeDelta> {
        None
    }

    /// How long the path takes to turn appreciably: a revolution, or for an open arc the
    /// `2 pi sqrt(|a|^3 / mu)` that is the same time constant without being a period.
    ///
    /// Sets the sampling density, so an answer that is too large under-samples silently and
    /// one that is too small is merely slow. `None` falls back to a fixed budget.
    fn timescale_at(&self, time: Instant) -> Option<TimeDelta> {
        self.period_at(time)
    }
}

/// A body of the system, as a traveler.
#[derive(Clone, Copy)]
pub struct BodyPath<'a> {
    pub system: &'a System,
    pub body: BodyIndex,
}

impl<'a> BodyPath<'a> {
    pub fn new(system: &'a System, body: BodyIndex) -> Self {
        Self { system, body }
    }

    /// The Keplerian arc in force at `time`, and the `mu` it is about.
    ///
    /// Keyed on `time` rather than on the arena's clock: those differ the moment a body has a
    /// patched chain, and getting it wrong is expensive both ways. A six-year heliocentric
    /// window measured against a ten-day parking orbit asks for a hundred thousand samples,
    /// and the reverse silently under-samples.
    fn arc_at(&self, time: Instant) -> Option<(&'a crate::motive::kepler::KeplerMotive, f64)> {
        let (_, selection) = self.system.motive(self.body).motive_at(time);
        match selection {
            MotiveSelection::Keplerian(kepler) => Some((
                kepler,
                propagate::gravitational_parameter_at(self.system, self.body, time),
            )),
            _ => None,
        }
    }
}

impl Traveler for BodyPath<'_> {
    fn state_at(&self, time: Instant) -> Option<(DVec3, DVec3)> {
        propagate::state_at(self.system, self.body, time)
    }

    fn local_state_at(&self, time: Instant) -> Option<(DVec3, DVec3)> {
        propagate::local_state_at(self.system, self.body, time)
    }

    fn period_at(&self, time: Instant) -> Option<TimeDelta> {
        let (kepler, mu) = self.arc_at(time)?;
        let period = kepler.period(mu);
        (period.to_seconds() > 0.0 && period.is_finite()).then_some(period)
    }

    fn timescale_at(&self, time: Instant) -> Option<TimeDelta> {
        if let Some(period) = self.period_at(time) {
            return Some(period);
        }
        // A hyperbolic arc has no period, but it has the same time constant, and the traverse
        // of a sphere is a fraction of it.
        let (kepler, mu) = self.arc_at(time)?;
        let a = kepler.semi_major_axis().abs();
        let scale = std::f64::consts::TAU * (a * a * a / mu).sqrt();
        scale.is_finite().then(|| TimeDelta::from_seconds(scale))
    }
}

/// A moment at which a traveler's path meets the edge of a sphere of influence.
#[derive(Debug, Clone, Copy)]
pub struct Crossing {
    /// Whose sphere was crossed.
    pub body: BodyIndex,
    pub time: Instant,
    /// The traveler's position in simulation space, on the boundary.
    pub position: DVec3,
    /// The same point measured from whatever the path is anchored on.
    ///
    /// This, not [`position`](Self::position), is what a marker is drawn at: a trajectory
    /// is drawn as an osculating ellipse anchored at the primary's *current* place, so a
    /// crossing a fortnight out would otherwise be placed where the primary will be by
    /// then — for a craft at Earth, some 3.6e10 m off the drawn path.
    pub local_position: DVec3,
    /// Its velocity there. A marker drawn at the crossing faces along this, so the craft
    /// meets it square on.
    pub velocity: DVec3,
    /// Normal of the traveler's orbital plane at the crossing, from `r x v` about its own
    /// anchor. Together with [`velocity`](Self::velocity) this orients a marker.
    /// [`DVec3::ZERO`] for a degenerate orbit.
    pub plane_normal: DVec3,
    /// `true` when the path passes from outside the sphere to inside.
    pub entering: bool,
}

/// Samples per revolution used to bracket a root.
///
/// A crossing is a transversal root of a smooth function, so the only way to miss a pair
/// is for an entry and its exit to fall inside a single step. At this density that means
/// passing through a sphere in under a seven-hundredth of an orbit.
pub const SAMPLES_PER_REVOLUTION: f64 = 512.0;

/// Ceiling on the bracketing pass, so a long window cannot become an unbounded loop.
const MAX_SAMPLES: usize = 200_000;

/// Bisection stops here. Well below a simulation step, and far below the accuracy of the
/// elements themselves.
const CROSSING_TOLERANCE_SECONDS: f64 = 1.0e-3;

/// How much of an orbit to search at a time when stepping outward.
///
/// [`crossings_of`] samples at a fixed density per revolution, so splitting the horizon into
/// chunks does not change what is found — it only stops early. The nearest crossing is
/// usually a fraction of an orbit away, and paying for the whole horizon every time made
/// this the most expensive thing in the frame.
const SEARCH_CHUNK_REVOLUTIONS: f64 = 0.25;

/// Signed distance from the boundary of `target`'s sphere, in meters: negative inside.
///
/// `None` when the traveler or the target is not analytic at `time`, or `target` has no
/// sphere. This is the scalar every search below is a root of.
pub fn boundary_distance_of(
    system: &System,
    traveler: &dyn Traveler,
    target: BodyIndex,
    time: Instant,
) -> Option<f64> {
    let soi = soi_at(system, target, time)?;
    let (position, _) = traveler.state_at(time)?;
    let offset = position - soi.center;
    Some(offset.length() - soi.radius_toward(offset))
}

/// Every crossing of `target`'s sphere within `window`, in time order.
///
/// `window` is `(start, end)` with `start < end`. The search is analytic in `t`, so a
/// window in the past costs exactly what one in the future costs.
///
/// Coarse-samples to bracket sign changes, then bisects each bracket. Only **transversal**
/// crossings are found: a path that grazes the boundary and turns back without passing
/// through leaves no sign change and is not reported. That is the honest limit of a
/// sign-change search, and it is the case where "did it enter?" has no useful answer
/// anyway.
pub fn crossings_of(
    system: &System,
    traveler: &dyn Traveler,
    target: BodyIndex,
    window: (Instant, Instant),
) -> Vec<Crossing> {
    let (start, end) = window;
    let span = end - start;
    if !span.is_finite() || span.to_seconds() <= 0.0 {
        return Vec::new();
    }

    let steps = bracketing_steps(traveler, start, span);
    let step = span / steps as f64;

    let mut found = Vec::new();
    let mut previous: Option<(Instant, f64)> = None;

    for i in 0..=steps {
        let time = start + step * i as f64;
        let Some(distance) = boundary_distance_of(system, traveler, target, time) else {
            // A gap in what can be evaluated is not a crossing; do not bracket across it.
            previous = None;
            continue;
        };

        if let Some((previous_time, previous_distance)) = previous {
            // Strictly opposite signs. A sample sitting exactly on the boundary is picked
            // up by the neighboring interval instead of counting twice.
            if (previous_distance < 0.0) != (distance < 0.0) {
                if let Some(crossing) = refine(
                    system, traveler, target,
                    (previous_time, previous_distance), (time, distance),
                ) {
                    found.push(crossing);
                }
            }
        }
        previous = Some((time, distance));
    }

    found
}

/// The first crossing strictly after `from`, within `horizon`.
pub fn next_crossing_of(
    system: &System,
    traveler: &dyn Traveler,
    target: BodyIndex,
    from: Instant,
    horizon: TimeDelta,
) -> Option<Crossing> {
    let chunk = search_chunk(traveler, from, horizon);
    let end = from + horizon;
    let mut cursor = from;
    while cursor < end {
        let stop = (cursor + chunk).min(end);
        if let Some(found) = crossings_of(system, traveler, target, (cursor, stop)).into_iter().next()
        {
            return Some(found);
        }
        cursor = stop;
    }
    None
}

/// The first crossing of `target`'s sphere within `window`, for a traveler whose speed
/// relative to the sphere's center never exceeds `speed_bound`.
///
/// Conservative advancement rather than a grid keyed on the traveler's time constant, which
/// for a near-straight hyperbola at a fraction of `c` is milliseconds. Outside the bounding
/// radius, or inside the smallest, the boundary cannot be reached sooner than the gap over
/// `speed_bound`, so that is the step; in the band between, a step moves the traveler a
/// quarter of the smallest radius. So the cost follows how close the path comes to the sphere
/// rather than how long the window is.
///
/// A crossing shorter than that quarter-radius step is missed, as a grazing one is by
/// [`crossings_of`]. A `speed_bound` that is not a bound can step over a crossing. The walk
/// stops after `max_samples`, and one that runs out reports nothing.
pub fn next_crossing_bounded(
    system: &System,
    traveler: &dyn Traveler,
    target: BodyIndex,
    window: (Instant, Instant),
    speed_bound: f64,
    max_samples: usize,
) -> Option<Crossing> {
    const BAND_STEP_OF_RADIUS: f64 = 0.25;
    let (start, end) = window;
    if !(speed_bound > 0.0 && speed_bound.is_finite()) || !(start < end) {
        return None;
    }
    let gap_step_s = (end - start).to_seconds() / max_samples.max(1) as f64;
    let mut time = start;
    let mut previous: Option<(Instant, f64)> = None;
    for _ in 0..max_samples {
        let sample = soi_at(system, target, time).zip(traveler.state_at(time));
        let step_s = match sample {
            None => {
                // A gap in what can be evaluated is not a crossing; do not bracket across it.
                previous = None;
                gap_step_s
            }
            Some((soi, (position, _))) => {
                let offset = position - soi.center;
                let reach = offset.length();
                let distance = reach - soi.radius_toward(offset);
                if let Some(before) = previous
                    && (before.1 < 0.0) != (distance < 0.0)
                {
                    return refine(system, traveler, target, before, (time, distance));
                }
                previous = Some((time, distance));
                let (inner, outer) = (soi.min_radius(), soi.bounding_radius());
                let clear = (reach - outer).max(inner - reach).max(inner * BAND_STEP_OF_RADIUS);
                clear / speed_bound
            }
        };
        if time >= end {
            break;
        }
        time = (time + TimeDelta::from_seconds(step_s)).min(end);
    }
    None
}

/// The last crossing strictly before `from`, within `horizon`.
pub fn previous_crossing_of(
    system: &System,
    traveler: &dyn Traveler,
    target: BodyIndex,
    from: Instant,
    horizon: TimeDelta,
) -> Option<Crossing> {
    let chunk = search_chunk(traveler, from, horizon);
    let start = from - horizon;
    let mut cursor = from;
    while cursor > start {
        let stop = (cursor - chunk).max(start);
        if let Some(found) =
            crossings_of(system, traveler, target, (stop, cursor)).into_iter().next_back()
        {
            return Some(found);
        }
        cursor = stop;
    }
    None
}

/// The soonest crossing of any of `targets`, and which one it was.
///
/// What a patched-conic walk actually asks: not "when does it leave *that* sphere" but "what
/// does it meet first". Searching each candidate to the full horizon and taking the earliest
/// is the only answer that does not depend on the order the candidates came in.
pub fn first_crossing_of(
    system: &System,
    traveler: &dyn Traveler,
    targets: &[BodyIndex],
    from: Instant,
    horizon: TimeDelta,
) -> Option<Crossing> {
    targets
        .iter()
        .filter_map(|&target| next_crossing_of(system, traveler, target, from, horizon))
        .min_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal))
}

/// A reasonable window for "the next crossing": three of the traveler's revolutions.
///
/// Three rather than one because an orbit can sit wholly inside a sphere for a revolution
/// and still meet a moon on the next.
pub fn default_horizon_of(traveler: &dyn Traveler, from: Instant) -> TimeDelta {
    traveler.period_at(from).map(|p| p * 3.0).unwrap_or(TimeDelta::from_days(365.0))
}

/// A quarter of the traveler's revolution, or the whole horizon if it has no period.
fn search_chunk(traveler: &dyn Traveler, from: Instant, horizon: TimeDelta) -> TimeDelta {
    traveler.period_at(from).map(|p| p * SEARCH_CHUNK_REVOLUTIONS).unwrap_or(horizon)
}

/// How many samples to bracket `span` with, from the traveler's own time constant.
fn bracketing_steps(traveler: &dyn Traveler, start: Instant, span: TimeDelta) -> usize {
    let revolutions = match traveler.timescale_at(start) {
        Some(scale) if scale.to_seconds() > 0.0 => span.to_seconds().abs() / scale.to_seconds(),
        // Nothing periodic to key on; a fixed budget still brackets a smooth function.
        _ => 1.0,
    };
    ((revolutions * SAMPLES_PER_REVOLUTION).ceil() as usize).clamp(64, MAX_SAMPLES)
}

/// Bisect a bracketed sign change down to [`CROSSING_TOLERANCE_SECONDS`].
fn refine(
    system: &System,
    traveler: &dyn Traveler,
    target: BodyIndex,
    mut low: (Instant, f64),
    mut high: (Instant, f64),
) -> Option<Crossing> {
    // `entering` is fixed by the bracket, not by the refined endpoint: the sign either
    // side of the root is what says which way the boundary was crossed.
    let entering = low.1 > 0.0;

    while (high.0 - low.0).to_seconds().abs() > CROSSING_TOLERANCE_SECONDS {
        let middle = low.0 + (high.0 - low.0) / 2.0;
        let Some(distance) = boundary_distance_of(system, traveler, target, middle) else {
            return None;
        };
        if (distance < 0.0) == (low.1 < 0.0) {
            low = (middle, distance);
        } else {
            high = (middle, distance);
        }
    }

    let time = low.0 + (high.0 - low.0) / 2.0;
    let (position, velocity) = traveler.state_at(time)?;
    let (local_position, local_velocity) =
        traveler.local_state_at(time).unwrap_or((DVec3::ZERO, DVec3::ZERO));

    // The orbital plane is taken about the traveler's own anchor, not about whichever
    // sphere is being crossed, so a marker lies flat in the drawn trajectory whether the
    // boundary belongs to that anchor or to a sibling moon.
    let normal = local_position.cross(local_velocity);
    let plane_normal = if normal.length_squared() > 0.0 { normal.normalize() } else { DVec3::ZERO };

    Some(Crossing { body: target, time, position, local_position, velocity, plane_normal, entering })
}

// === The body-indexed forms ===
//
// What every caller used before the traveler was a trait. Kept because a body of the system
// is by far the commonest traveler, and writing out the adaptor at each call site would say
// nothing.

pub fn boundary_distance(
    system: &System,
    traveler: BodyIndex,
    target: BodyIndex,
    time: Instant,
) -> Option<f64> {
    boundary_distance_of(system, &BodyPath::new(system, traveler), target, time)
}

pub fn crossings(
    system: &System,
    traveler: BodyIndex,
    target: BodyIndex,
    window: (Instant, Instant),
) -> Vec<Crossing> {
    crossings_of(system, &BodyPath::new(system, traveler), target, window)
}

pub fn next_crossing(
    system: &System,
    traveler: BodyIndex,
    target: BodyIndex,
    from: Instant,
    horizon: TimeDelta,
) -> Option<Crossing> {
    next_crossing_of(system, &BodyPath::new(system, traveler), target, from, horizon)
}

pub fn previous_crossing(
    system: &System,
    traveler: BodyIndex,
    target: BodyIndex,
    from: Instant,
    horizon: TimeDelta,
) -> Option<Crossing> {
    previous_crossing_of(system, &BodyPath::new(system, traveler), target, from, horizon)
}

pub fn default_horizon(system: &System, traveler: BodyIndex) -> TimeDelta {
    let now = system.time();
    default_horizon_of(&BodyPath::new(system, traveler), now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::solar_system;

    fn built() -> System {
        let mut system = System::from_contents(&solar_system()).expect("the bundled system builds");
        system.rebuild_derived(Instant::J2000);
        propagate::evaluate_at(&mut system, Instant::J2000);
        system
    }

    fn named(system: &System, name: &str) -> BodyIndex {
        system.indices().find(|&i| system.name(i) == name).expect("a body by that name")
    }

    /// A traveler the system has never heard of: a straight line, held nowhere but here.
    ///
    /// This is the whole point of the trait. Before it, giving a spacecraft a predicted
    /// crossing meant inserting it into the arena as a body first.
    struct Line {
        from: DVec3,
        velocity: DVec3,
        epoch: Instant,
    }

    /// A line flying past a body at `relative`, posed in that body's frame.
    ///
    /// Not in the arena's: Earth crosses its own sphere of influence every nine hours of
    /// heliocentric motion, so a line that is slow in the solar frame is not flying past
    /// Earth at all — Earth is running into it. The first version of these tests got that
    /// wrong and measured a chord a sixtieth of the one it asked for.
    /// Fast, and deliberately so. Earth's frame is not inertial — over a traverse it turns
    /// with the orbit — so only a flyby quick enough that the turn is negligible has a chord
    /// that can be predicted from the sphere's radius. At ten kilometers a second the traverse
    /// takes nine days, Earth's velocity swings eight degrees, and the "diameter" this test
    /// asks for is out by a sixth.
    const FLYBY_SPEED: f64 = 100_000.0;

    fn flyby(system: &System, body: BodyIndex, from: DVec3, relative: DVec3) -> Line {
        let (at, velocity) = propagate::state_at(system, body, Instant::J2000).expect("a state");
        Line { from: at + from, velocity: velocity + relative, epoch: Instant::J2000 }
    }

    impl Traveler for Line {
        fn state_at(&self, time: Instant) -> Option<(DVec3, DVec3)> {
            let dt = (time - self.epoch).to_seconds();
            Some((self.from + self.velocity * dt, self.velocity))
        }
    }

    /// Flown straight at Earth from outside its sphere, a line enters it and leaves again,
    /// and both roots land on the boundary.
    #[test]
    fn a_path_that_is_not_a_body_still_gets_its_crossings() {
        let system = built();
        let earth = named(&system, "Earth");
        let soi = soi_at(&system, earth, Instant::J2000).expect("Earth has a sphere");

        // Four sphere-radii out, aimed across the middle. See [`FLYBY_SPEED`].
        let reach = soi.bounding_radius();
        let speed = FLYBY_SPEED;
        let line = flyby(
            &system,
            earth,
            DVec3::new(-4.0 * reach, 0.0, 0.0),
            DVec3::new(speed, 0.0, 0.0),
        );

        let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_seconds(8.0 * reach / speed));
        let found = crossings_of(&system, &line, earth, window);
        assert_eq!(found.len(), 2, "in and out, not {}", found.len());
        assert!(found[0].entering, "the first crossing is an entry");
        assert!(!found[1].entering, "the second is an exit");
        assert!(found[0].time < found[1].time, "they came out of order");

        // Each root is on the boundary, to the tolerance the bisection promises.
        for crossing in &found {
            let distance =
                boundary_distance_of(&system, &line, earth, crossing.time).expect("evaluable");
            // A millisecond of bisection tolerance is a hundred meters at this speed.
            assert!(distance.abs() < 1_000.0, "{distance} m off the boundary");
        }

        // And the traverse is the chord it should be. Not the bounding diameter: Earth's
        // sphere is a surface of revolution about the Earth-Sun axis and is some 13% narrower
        // along it, which is very nearly the direction this line crosses. The chord is the two
        // radii in the directions actually crossed, and asking for `bounding_radius` instead
        // was out by exactly that flattening.
        let crossed = (found[1].time - found[0].time).to_seconds() * speed;
        let chord = soi.radius_toward(-DVec3::X) + soi.radius_toward(DVec3::X);
        assert!(
            (crossed / chord - 1.0).abs() < 0.02,
            "crossed {crossed:e} m of a {chord:e} m chord",
        );
    }

    /// And the soonest of several spheres is the one reported, whatever order they are given.
    #[test]
    fn the_first_crossing_is_the_soonest_one() {
        let system = built();
        let earth = named(&system, "Earth");
        let luna = named(&system, "Luna");
        let soi = soi_at(&system, earth, Instant::J2000).expect("Earth has a sphere");

        let reach = soi.bounding_radius();
        let speed = FLYBY_SPEED;
        let line = flyby(
            &system,
            earth,
            DVec3::new(-4.0 * reach, 0.0, 0.0),
            DVec3::new(speed, 0.0, 0.0),
        );
        let horizon = TimeDelta::from_seconds(8.0 * reach / speed);

        let earth_first = next_crossing_of(&system, &line, earth, Instant::J2000, horizon);
        let soonest = first_crossing_of(&system, &line, &[luna, earth], Instant::J2000, horizon)
            .expect("it meets something");
        assert_eq!(soonest.body, earth, "the line enters Earth's sphere before any moon's");
        assert_eq!(soonest.time, earth_first.expect("Earth is met").time);
    }

    /// A line going the other way meets nothing, and says so rather than guessing.
    #[test]
    fn a_path_that_never_arrives_reports_nothing() {
        let system = built();
        let earth = named(&system, "Earth");
        let soi = soi_at(&system, earth, Instant::J2000).expect("Earth has a sphere");
        let reach = soi.bounding_radius();
        let line = flyby(
            &system,
            earth,
            DVec3::new(-4.0 * reach, 0.0, 0.0),
            DVec3::new(-FLYBY_SPEED, 0.0, 0.0),
        );
        let horizon = TimeDelta::from_seconds(8.0 * reach / FLYBY_SPEED);
        assert!(next_crossing_of(&system, &line, earth, Instant::J2000, horizon).is_none());
    }
}
