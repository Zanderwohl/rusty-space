//! When a trajectory meets a body.
//!
//! The same shape of question as [`crossing`](crate::crossing) — a root of a distance in `t` —
//! and a different answer, because a body is five orders of magnitude smaller than its sphere
//! of influence. A sampler dense enough to bracket an Earth-sized target over a heliocentric
//! year needs some ten million samples, and one that is not dense enough does not report a
//! miss: it reports nothing, and the craft flies through the planet.
//!
//! So this marches rather than samples. From any instant it advances by the time the traveller
//! would need to close the gap at its greatest possible speed, which cannot skip over contact.
//! For a two-body arc about the body in question that speed bound is exact — the vis-viva
//! speed at the surface — and elsewhere it is padded. See [`STEP_SAFETY`].
//!
//! # Spheres
//!
//! A body is its [`radius`](crate::system::System::radius) and nothing else. There is no
//! terrain, no oblateness and no atmosphere here; a grazing pass over a mountain is a miss.

use em_foundations::time::{Instant, TimeDelta};
use glam::DVec3;

use crate::crossing::Traveller;
use crate::id::BodyIndex;
use crate::propagate;
use crate::system::System;

/// A trajectory meeting a body's surface.
#[derive(Debug, Clone, Copy)]
pub struct Impact {
    pub body: BodyIndex,
    pub time: Instant,
    /// Simulation-space position at contact, metres.
    pub position: DVec3,
    /// Measured from the body's centre: the point on the surface that was struck. Its
    /// direction is the site, its length the radius.
    pub local_position: DVec3,
    /// Velocity relative to the body, metres a second. What an impact is judged by — a
    /// touchdown and a lithobraking differ only in this.
    pub relative_velocity: DVec3,
}

impl Impact {
    /// Closing speed at contact, metres a second.
    pub fn speed_m_s(&self) -> f64 {
        self.relative_velocity.length()
    }

    /// The component straight into the ground. A grazing pass that clips the surface has a
    /// large [`speed_m_s`](Self::speed_m_s) and a small one of these.
    pub fn vertical_speed_m_s(&self) -> f64 {
        let up = self.local_position.normalize_or_zero();
        -self.relative_velocity.dot(up)
    }
}

/// How much of the safe step to actually take.
///
/// The speed bound is exact for a two-body arc about the body being tested, and that is the
/// case that matters — a craft hits what it is orbiting. Off that case the body is itself
/// accelerating under something else, and the bound is short by that. Half the step costs
/// twice the iterations and buys the margin.
pub const STEP_SAFETY: f64 = 0.5;

/// Floor on a march step, seconds.
///
/// Without one, a traveller asymptotically approaching a surface it never reaches takes ever
/// smaller steps and the march does not terminate. A millisecond is far below any contact
/// anyone cares to time.
const MIN_STEP_SECONDS: f64 = 1.0e-3;

/// Ceiling on the march, so a long window cannot become an unbounded loop.
const MAX_STEPS: usize = 1_000_000;

/// Contact is refined to here. A millimetre a second of closing speed times this is nothing.
const CONTACT_TOLERANCE_SECONDS: f64 = 1.0e-3;

/// Height above `body`'s surface, metres: negative below it.
///
/// `None` when either the traveller or the body is not analytic at `time`.
pub fn surface_distance(
    system: &System,
    traveller: &dyn Traveller,
    body: BodyIndex,
    time: Instant,
) -> Option<f64> {
    let (at, _) = traveller.state_at(time)?;
    let (centre, _) = propagate::state_at(system, body, time)?;
    Some((at - centre).length() - system.radius(body))
}

/// The first contact with `body` inside `window`, if there is one.
///
/// `window` is `(start, end)` with `start < end`. A traveller that begins below the surface
/// is reported as a contact at `start`: it is already there, and saying "no impact" because
/// the boundary was never crossed inside the window would be worse than useless.
pub fn impact_with(
    system: &System,
    traveller: &dyn Traveller,
    body: BodyIndex,
    window: (Instant, Instant),
) -> Option<Impact> {
    let (start, end) = window;
    if !(end - start).is_finite() || (end - start).to_seconds() <= 0.0 {
        return None;
    }
    let radius = system.radius(body);
    // `is_nan` spelled out: a NaN compares false against everything, so the bound alone would
    // let one through into the march.
    if radius.is_nan() || radius <= 0.0 {
        return None;
    }
    let mu = system.gravitational_constant() * system.mass(body);

    let mut cursor = start;
    let mut above = surface_distance(system, traveller, body, cursor)?;
    if above <= 0.0 {
        return contact(system, traveller, body, cursor);
    }

    for _ in 0..MAX_STEPS {
        let step = safe_step(system, traveller, body, cursor, above, radius, mu)?;
        let next = (cursor + step).min(end);
        let Some(distance) = surface_distance(system, traveller, body, next) else {
            // A gap in what can be evaluated is not a contact, and it is not something to
            // march across either.
            return None;
        };
        if distance <= 0.0 {
            let time = refine(system, traveller, body, (cursor, above), (next, distance))?;
            return contact(system, traveller, body, time);
        }
        if next >= end {
            return None;
        }
        cursor = next;
        above = distance;
    }
    None
}

/// The soonest contact with any of `bodies`.
///
/// Every candidate is marched over the whole window and the earliest taken, so the answer does
/// not depend on the order they came in.
pub fn first_impact(
    system: &System,
    traveller: &dyn Traveller,
    bodies: &[BodyIndex],
    window: (Instant, Instant),
) -> Option<Impact> {
    bodies
        .iter()
        .filter_map(|&body| impact_with(system, traveller, body, window))
        .min_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal))
}

/// Bodies worth testing a traveller against: those whose spheres of influence it could be in.
///
/// A body whose sphere does not reach the traveller cannot be hit before the traveller first
/// enters that sphere, and that is a crossing, not a collision. Pairing this with
/// [`crossing::first_crossing_of`](crate::crossing::first_crossing_of) covers the rest.
pub fn candidates_at(system: &System, traveller: &dyn Traveller, time: Instant) -> Vec<BodyIndex> {
    let Some((at, _)) = traveller.state_at(time) else { return Vec::new() };
    crate::influence::containment_chain(system, at, time)
}

/// How far the march may advance without risking stepping over contact.
///
/// The traveller cannot reach the surface before it has covered `above` metres, and it cannot
/// cover them faster than its greatest possible speed. That speed is bounded by energy: a
/// two-body arc about this body trades height for speed at `v^2 = v0^2 + 2 mu (1/r - 1/r0)`,
/// which is largest at the surface.
fn safe_step(
    system: &System,
    traveller: &dyn Traveller,
    body: BodyIndex,
    time: Instant,
    above: f64,
    radius: f64,
    mu: f64,
) -> Option<TimeDelta> {
    let (at, velocity) = traveller.state_at(time)?;
    let (centre, carried) = propagate::state_at(system, body, time)?;
    let separation = (at - centre).length().max(radius);
    let closing = (velocity - carried).length();
    // The `max(0.0)` is not redundant: `separation` is floored at the radius, so at the
    // surface the term is exactly zero and rounding can take it just below.
    let fastest =
        (closing * closing + 2.0 * mu * (1.0 / radius - 1.0 / separation).max(0.0)).sqrt();
    let seconds = if fastest > 0.0 { above / fastest } else { f64::INFINITY };
    Some(TimeDelta::from_seconds((seconds * STEP_SAFETY).max(MIN_STEP_SECONDS)))
}

/// Bisect a bracketed contact down to [`CONTACT_TOLERANCE_SECONDS`].
fn refine(
    system: &System,
    traveller: &dyn Traveller,
    body: BodyIndex,
    mut outside: (Instant, f64),
    mut inside: (Instant, f64),
) -> Option<Instant> {
    while (inside.0 - outside.0).to_seconds().abs() > CONTACT_TOLERANCE_SECONDS {
        let middle = outside.0 + (inside.0 - outside.0) / 2.0;
        let distance = surface_distance(system, traveller, body, middle)?;
        if distance > 0.0 {
            outside = (middle, distance);
        } else {
            inside = (middle, distance);
        }
    }
    // The outside end, so a reported contact is never one the traveller has already passed
    // through. The same choice `crossing` makes about rounding an arrival upward.
    Some(outside.0)
}

fn contact(
    system: &System,
    traveller: &dyn Traveller,
    body: BodyIndex,
    time: Instant,
) -> Option<Impact> {
    let (position, velocity) = traveller.state_at(time)?;
    let (centre, carried) = propagate::state_at(system, body, time)?;
    Some(Impact {
        body,
        time,
        position,
        local_position: position - centre,
        relative_velocity: velocity - carried,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::solar_system;

    const EARTH_RADIUS_M: f64 = 6.371e6;
    /// Fast, and started close, so the flyby is over before Earth's frame turns appreciably.
    /// See the flyby note in [`crate::crossing`]'s tests: Earth's frame is not inertial, and a
    /// pass that takes hours has a geometry set by that rather than by where it was aimed. At
    /// these numbers the approach takes five hundred seconds and Earth's own path bends by
    /// seven hundred metres, against a radius of six thousand kilometres.
    const SPEED: f64 = 200_000.0;
    const STANDOFF_M: f64 = 1.0e8;

    fn built() -> System {
        let mut system = System::from_contents(&solar_system()).expect("the bundled system builds");
        system.rebuild_derived(Instant::J2000);
        propagate::evaluate_at(&mut system, Instant::J2000);
        system
    }

    fn named(system: &System, name: &str) -> BodyIndex {
        system.indices().find(|&i| system.name(i) == name).expect("a body by that name")
    }

    struct Line {
        from: DVec3,
        velocity: DVec3,
    }

    impl Traveller for Line {
        fn state_at(&self, time: Instant) -> Option<(DVec3, DVec3)> {
            let dt = (time - Instant::J2000).to_seconds();
            Some((self.from + self.velocity * dt, self.velocity))
        }
    }

    /// A line aimed at a body, posed in that body's frame. `offset` is the miss distance
    /// across the approach, in body radii.
    fn aimed(system: &System, body: BodyIndex, from_m: f64, offset_radii: f64) -> Line {
        let (at, carried) = propagate::state_at(system, body, Instant::J2000).expect("a state");
        let radius = system.radius(body);
        Line {
            from: at + DVec3::new(-from_m, offset_radii * radius, 0.0),
            velocity: carried + DVec3::new(SPEED, 0.0, 0.0),
        }
    }

    /// The contact is on the surface, not near it.
    #[test]
    fn a_line_aimed_at_a_body_hits_it_on_the_surface() {
        let system = built();
        let earth = named(&system, "Earth");
        let line = aimed(&system, earth, STANDOFF_M, 0.0);
        let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(1.0));

        let hit = impact_with(&system, &line, earth, window).expect("it hits");
        assert_eq!(hit.body, earth);
        let height = hit.local_position.length() - EARTH_RADIUS_M;
        // A millisecond of bisection tolerance is two hundred metres at this speed.
        assert!(height.abs() < 300.0, "{height} m off the surface");
        assert!(height >= 0.0, "a contact is reported before the surface, never through it");

        // Head-on, so nearly all the closing speed is straight down.
        // Not exact: Earth's velocity at contact is not its velocity at the epoch the line
        // was built against, and the difference is the frame's own turn over the approach.
        assert!((hit.speed_m_s() - SPEED).abs() < 10.0, "{} m/s", hit.speed_m_s());
        assert!(
            hit.vertical_speed_m_s() > SPEED * 0.99,
            "{} m/s of {SPEED}",
            hit.vertical_speed_m_s(),
        );
    }

    /// A pass that clips the limb has the same speed and almost none of it downward.
    #[test]
    fn a_graze_is_mostly_sideways() {
        let system = built();
        let earth = named(&system, "Earth");
        let line = aimed(&system, earth, STANDOFF_M, 0.98);
        let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(1.0));

        let hit = impact_with(&system, &line, earth, window).expect("it clips the limb");
        assert!((hit.speed_m_s() - SPEED).abs() < 10.0);
        // At a miss distance of `b` radii the descent is `sqrt(1 - b^2)` of the speed: 0.20
        // here, against 1.00 for the head-on pass above.
        assert!(
            hit.vertical_speed_m_s() < SPEED * 0.3,
            "a graze came in at {} m/s downward",
            hit.vertical_speed_m_s(),
        );
    }

    /// The reason this marches instead of sampling: over a long window a uniform sampler
    /// steps further than the traverse of a planet takes, so it walks straight over it and
    /// reports nothing. The march cannot, because it never advances further than the gap.
    #[test]
    fn a_long_window_does_not_let_the_traveller_through_the_planet() {
        let system = built();
        let earth = named(&system, "Earth");
        let line = aimed(&system, earth, STANDOFF_M, 0.0);
        let year = TimeDelta::from_days(365.25);
        let window = (Instant::J2000, Instant::J2000 + year);

        let hit = impact_with(&system, &line, earth, window).expect("it still hits");
        assert!((hit.local_position.length() - EARTH_RADIUS_M).abs() < 300.0);

        // The hazard, as numbers. A sampler spending its whole budget on this window steps
        // further than the planet is wide in time.
        let uniform_step = year.to_seconds() / crate::crossing::SAMPLES_PER_REVOLUTION / 512.0;
        let traverse = 2.0 * EARTH_RADIUS_M / SPEED;
        assert!(
            uniform_step > traverse,
            "{uniform_step} s a sample against a {traverse} s traverse",
        );
    }

    /// A miss is a miss, and marching to the end of the window says so.
    #[test]
    fn a_line_that_passes_wide_reports_nothing() {
        let system = built();
        let earth = named(&system, "Earth");
        let line = aimed(&system, earth, STANDOFF_M, 4.0);
        let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(1.0));
        assert!(impact_with(&system, &line, earth, window).is_none());
    }

    /// A real orbit, run for a year. The Moon does not hit the Earth.
    #[test]
    fn a_body_on_its_own_orbit_does_not_hit_what_it_orbits() {
        let system = built();
        let earth = named(&system, "Earth");
        let luna = named(&system, "Luna");
        let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(365.0));
        let path = crate::crossing::BodyPath::new(&system, luna);
        assert!(impact_with(&system, &path, earth, window).is_none());
    }

    /// Starting below the surface is a contact at the start, not a boundary that was never
    /// crossed. Saying "no impact" for something already inside the planet is worse than
    /// useless.
    #[test]
    fn something_already_inside_is_already_in_contact() {
        let system = built();
        let earth = named(&system, "Earth");
        let line = aimed(&system, earth, 0.0, 0.0);
        let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(1.0));
        let hit = impact_with(&system, &line, earth, window).expect("it is already there");
        assert_eq!(hit.time, Instant::J2000);
    }

    /// Of several bodies, the soonest is reported, whichever order they are given in.
    #[test]
    fn the_first_impact_is_the_soonest_one() {
        let system = built();
        let earth = named(&system, "Earth");
        let luna = named(&system, "Luna");
        let line = aimed(&system, earth, STANDOFF_M, 0.0);
        let window = (Instant::J2000, Instant::J2000 + TimeDelta::from_days(1.0));

        let hit = first_impact(&system, &line, &[luna, earth], window).expect("it hits something");
        assert_eq!(hit.body, earth, "the line was aimed at Earth");
        let same = first_impact(&system, &line, &[earth, luna], window).expect("still hits");
        assert_eq!(hit.time, same.time, "the order of the candidates changed the answer");
    }

    /// The candidate set is what holds the traveller, so a craft in low orbit is tested
    /// against the thing it is about to hit.
    #[test]
    fn the_candidates_are_whatever_holds_the_traveller() {
        let system = built();
        let earth = named(&system, "Earth");
        let line = aimed(&system, earth, EARTH_RADIUS_M * 2.0, 0.0);
        let candidates = candidates_at(&system, &line, Instant::J2000);
        assert!(candidates.contains(&earth), "two radii up is inside Earth's influence");
    }
}
