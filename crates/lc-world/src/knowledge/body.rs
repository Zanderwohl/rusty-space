//! What a craft believes about the bodies of a system, and about the plane they share.
//!
//! A fold over records, like [`super::Belief`] is for a star, and held nowhere: a plane drawn
//! from orbits has to move when an orbit does, and a stored copy is a second thing to keep in
//! step. See `lightcone/docs/25-system-knowledge.md`.

use glam::DVec3;

use super::conclusion::Hypothesis;
use super::record::{Colors, Method, Orbit, Orientation};
use crate::sky::StarId;
use super::subject::BodyId;
use super::{Knowledge, Subject, Witness};

/// Where a body is believed to be now, as an offset from its own **star**.
///
/// From the star rather than from the world origin: what an orbit says is where a body sits
/// about its primary, and the star's own position is a separate belief with its own error. A
/// reader that wants an absolute position adds the two and carries both errors.
///
/// From the star even for a moon, whose orbit is about its planet: `body_belief` walks the
/// chain and adds each step, so one reader does not have to know how deep a body sits. The
/// errors add in quadrature along the way, which is why a moon is always placed worse than its
/// planet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placed {
    /// A full orientation and an epoch: enough to say where on the orbit it is.
    Known { offset_au: DVec3, sigma_au: f64 },
    /// An orbit of known size and nothing else. A sphere of that radius, not a ring in a guessed
    /// plane — rule 4 of doc 25.
    Shell { radius_au: f64, sigma_au: f64 },
    Unknown,
}

/// What the system's plane is known to be, folded from the orbits held about its bodies.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SystemPlane {
    /// Nothing constrains it.
    #[default]
    Unknown,
    /// Only edge-on constraints: the pole lies on this great circle, whose own normal is the
    /// line of sight the transits were seen along. One craft watching from one place can get no
    /// further than this.
    Circle(DVec3),
    Known {
        pole: DVec3,
        sigma_rad: f64,
        /// Zero longitude: the ascending node on the galactic plane. Derived here rather than
        /// stored on any orbit, so refining the plane moves every longitude together.
        zero: DVec3,
    },
}

/// How much a giant counts for over a rocky planet when the poles are averaged.
///
/// The invariable plane is the angular momentum of the system, which the giants carry almost
/// all of: Jupiter alone holds about three fifths of the Sun's system. Weighting by class is
/// the part of that a craft can do without knowing any masses.
const GIANT_WEIGHT: f64 = 4.0;

/// Below this the weighted poles are too scattered to call a plane, radians.
///
/// Generated inclinations are gaussian with a two-degree sigma, so a system solved from good
/// orbits lands well inside it, and one solved from disagreeing orbits should say it cannot
/// tell rather than average them into a plane nothing lies in.
pub const PLANE_SCATTER_LIMIT_RAD: f64 = 20.0 * std::f64::consts::PI / 180.0;

/// Everything believed about one body.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyBelief {
    pub subject: Subject,
    pub body: BodyId,
    /// What this craft calls it: a letter until somebody names it, and `None` when nothing
    /// has assigned even that. See [`Knowledge::name_of`].
    pub name: Option<String>,
    /// What kind of thing it is, most probable first. Empty when nothing has concluded.
    pub kind: Vec<Hypothesis>,
    pub period_s: Option<(f64, f64)>,
    pub semi_major_au: Option<(f64, f64)>,
    pub orientation: Orientation,
    pub method: Option<Method>,
    pub position_now: Placed,
    /// Meters and one sigma. An angular diameter times a range, so it needs a look that had
    /// both: close enough to range the body, which from across a system nothing is.
    pub radius_m: Option<(f64, f64)>,
    /// Seconds for one turn, and one sigma, from the closest look that timed it.
    pub spin_s: Option<(f64, f64)>,
    /// Meters a second in the star's frame, and one sigma, from the orbit rather than from any
    /// measurement: elements and a time are a velocity, and nothing else here is.
    pub velocity_m_s: Option<(DVec3, f64)>,
    /// What it is believed to go round. `None` is the system's star.
    pub about: Option<BodyId>,
    /// Mean flux per band and one sigma on each, from the digest every visit folds into.
    /// `None` in a band nothing was measured in, which is not a zero.
    pub colors: Option<Colors>,
    /// Kilograms and one sigma, from whatever goes round *this* body: a satellite's orbit is
    /// its primary's mass by Kepler's third law and there is no other way to weigh one.
    pub mass_kg: Option<(f64, f64)>,
    /// Whose word the orbit is on, and how many hands it passed through.
    pub stated_by: Option<Witness>,
    pub hops: usize,
}

impl Knowledge {
    /// How far a chain of primaries is walked before it is called a cycle.
    ///
    /// A moon's moon is the deepest thing a system holds, so three steps is already generous.
    /// The guard is not about depth but about a *loop*: two bodies each fitted as the other's
    /// primary would otherwise recur until the stack ran out.
    const CHAIN: usize = 4;

    /// What is believed about one body, or `None` when nothing is held about it.
    pub fn body_belief(&self, star: StarId, body: BodyId, now_s: f64) -> Option<BodyBelief> {
        self.body_belief_within(star, body, now_s, Self::CHAIN)
    }

    fn body_belief_within(
        &self,
        star: StarId,
        body: BodyId,
        now_s: f64,
        depth: usize,
    ) -> Option<BodyBelief> {
        let subject = Subject::Body { star, body };
        let file = self.file(subject)?;
        // This craft's own before anybody else's, then the method that stands highest, then
        // the later statement. A craft holds one orbit per witness per method now, so without
        // the middle term a transit's shell would shadow a fit made before it.
        let orbit = file.orbits().iter().max_by(|a, b| {
            (a.witness == self.owner)
                .cmp(&(b.witness == self.owner))
                .then(a.method.standing().cmp(&b.method.standing()))
                .then(a.stated_s.total_cmp(&b.stated_s))
        });
        let kind = file
            .conclusions()
            .iter()
            .max_by(|a, b| a.stated_s.total_cmp(&b.stated_s))
            .map(|c| c.transits.clone())
            .unwrap_or_default();
        Some(BodyBelief {
            subject,
            body,
            name: self.name_of(subject),
            kind,
            period_s: orbit.map(|o| o.period_s),
            semi_major_au: orbit.map(|o| o.semi_major_au),
            orientation: orbit.map_or(Orientation::Unknown, |o| o.orientation),
            method: orbit.map(|o| o.method),
            position_now: orbit.map_or(Placed::Unknown, |o| {
                let here = placed_at(o, now_s);
                match o.about {
                    // Its own offset is from its primary, so the primary's own place is added.
                    // A step with nowhere to stand on is a body whose place is not known.
                    Some(primary) if depth > 0 => self
                        .body_belief_within(star, primary, now_s, depth - 1)
                        .map_or(Placed::Unknown, |up| added(up.position_now, here)),
                    Some(_) => Placed::Unknown,
                    None => here,
                }
            }),
            radius_m: measured_radius(file),
            spin_s: file
                .sightings()
                .iter()
                .filter_map(|seen| seen.spin_s.map(|spin| (seen.observed_s, spin)))
                .min_by(|a, b| a.1 .1.total_cmp(&b.1 .1))
                .map(|(_, spin)| spin),
            velocity_m_s: orbit.and_then(|o| moving_at(o, now_s)),
            about: orbit.and_then(|o| o.about),
            // The one with the most visits behind it. Two craft's digests are not folded
            // together: Welford's needs the rows, and a report carries the digest, not them.
            colors: file
                .colors()
                .iter()
                .max_by_key(|c| c.visits.iter().map(|(_, n)| u64::from(*n)).sum::<u64>())
                .cloned(),
            mass_kg: self.weighed(star, body),
            stated_by: orbit.map(|o| o.witness),
            hops: orbit.map_or(0, |o| o.lineage.len()),
        })
    }

    /// Every body believed to be in one system, outward by believed distance.
    ///
    /// Ordered by what is *believed*, so a body whose distance is refined moves in the list.
    /// Anything with no distance at all sorts last, since there is nowhere to put it.
    pub fn bodies_of(&self, star: StarId, now_s: f64) -> Vec<BodyBelief> {
        let mut found: Vec<BodyBelief> = self
            .members(star)
            .filter_map(|(subject, _)| match subject {
                Subject::Body { body, .. } => self.body_belief(star, body, now_s),
                _ => None,
            })
            .collect();
        found.sort_by(|a, b| {
            let key = |belief: &BodyBelief| belief.semi_major_au.map_or(f64::INFINITY, |(au, _)| au);
            key(a).total_cmp(&key(b)).then_with(|| a.name.cmp(&b.name)).then(a.body.cmp(&b.body))
        });
        found
    }

    /// The plane a system's bodies are believed to share.
    ///
    /// Recomputed on every call and stored nowhere: it moves whenever an orbit does. A caller
    /// that already holds the list should call [`plane_of`] instead of paying for a second one.
    ///
    /// [`plane_of`]: Self::plane_of
    pub fn system_plane(&self, star: StarId) -> SystemPlane {
        Self::plane_of(&self.bodies_of(star, 0.0))
    }

    /// The plane a list of beliefs shares.
    ///
    /// The mean of the orbit poles, weighted by `1 / sigma^2` and by class so a giant counts for
    /// more, which approximates the invariable plane without knowing a single mass.
    ///
    /// Any epoch's list answers the same, because only an orbit's orientation is read and that
    /// does not move with time.
    pub fn plane_of(beliefs: &[BodyBelief]) -> SystemPlane {
        let mut sum = DVec3::ZERO;
        let mut weight = 0.0;
        let mut circle: Option<DVec3> = None;
        for belief in beliefs.iter().filter(|b| in_the_system_plane(b)) {
            match belief.orientation {
                Orientation::Known { pole, sigma_rad, .. } => {
                    // A giant's pole is the invariable plane's; a rock's is near it. Squared
                    // sigma because that is how independent errors combine.
                    let w = class_weight(belief) / sigma_rad.max(1.0e-6).powi(2);
                    sum += folded(pole) * w;
                    weight += w;
                }
                Orientation::EdgeOnTo { toward } => circle = circle.or(Some(toward)),
                Orientation::Unknown => {}
            }
        }
        if weight <= 0.0 {
            return circle.map_or(SystemPlane::Unknown, SystemPlane::Circle);
        }
        let pole = sum.normalize_or_zero();
        if pole == DVec3::ZERO {
            return circle.map_or(SystemPlane::Unknown, SystemPlane::Circle);
        }
        let sigma_rad = plane_scatter(beliefs, pole, weight);
        if sigma_rad > PLANE_SCATTER_LIMIT_RAD {
            return circle.map_or(SystemPlane::Unknown, SystemPlane::Circle);
        }
        SystemPlane::Known { pole, sigma_rad, zero: zero_longitude(pole) }
    }
}

/// A giant weighs more because it carries most of the system's angular momentum.
fn class_weight(belief: &BodyBelief) -> f64 {
    use super::conclusion::{Class, Kind};
    let giant = belief
        .kind
        .first()
        .is_some_and(|h| matches!(h.kind, Kind::Planet { class: Class::Giant, .. }));
    if giant { GIANT_WEIGHT } else { 1.0 }
}

/// How far the orbit poles actually scatter about the mean, radians.
///
/// The larger of the weighted scatter and what the individual sigmas allow: a single orbit has
/// no scatter to measure and must still report its own error, and two orbits that disagree must
/// not report the confidence their sigmas claim.
fn plane_scatter(beliefs: &[BodyBelief], pole: DVec3, weight: f64) -> f64 {
    let mut spread = 0.0;
    let mut count = 0usize;
    for belief in beliefs.iter().filter(|b| in_the_system_plane(b)) {
        if let Orientation::Known { pole: p, .. } = belief.orientation {
            let off = pole.dot(folded(p)).clamp(-1.0, 1.0).acos();
            spread += off * off;
            count += 1;
        }
    }
    let scatter = if count > 1 { (spread / count as f64).sqrt() } else { 0.0 };
    scatter.max((1.0 / weight).sqrt())
}

/// Whether a body's orbit says anything about the plane the *system* lies in.
///
/// Only one that goes round the star. A moon's orbit is its planet's equator, which is tilted
/// by that planet's obliquity and says nothing about the system: Uranus's retinue is 98 degrees
/// off, Triton goes backwards, and a moon fitted on a close pass has so small a sigma that its
/// weight swamps every planet. Averaging them in pushed Sol's own scatter past the limit and
/// left the system reading as having no solved plane at all.
fn in_the_system_plane(belief: &BodyBelief) -> bool {
    belief.about.is_none()
}

/// A pole folded onto one hemisphere, so an average is not cancelled by direction of travel.
///
/// **Onto a fixed one.** The sign of a pole is which way the body goes round, and an edge-on
/// measurement does not settle it, so the two readings of one plane have to be brought
/// together. Folding onto "whichever half the first one picked" made that depend on the order
/// bodies were found in -- and `bodies_of` runs inward-out, so a newly found inner body with
/// the opposite sign flipped the whole basis and moved zero longitude to the other node, which
/// `25-system-knowledge.md` forbids.
fn folded(pole: DVec3) -> DVec3 {
    let north = em_foundations::reference_frame::galactic::north_pole();
    let along = pole.dot(north);
    // Perpendicular to galactic north within a milliradian: fall back to a second fixed axis
    // rather than letting the sign turn on the last digit.
    if along.abs() > 1.0e-3 {
        if along < 0.0 { -pole } else { pole }
    } else if pole.dot(DVec3::X) < 0.0 {
        -pole
    } else {
        pole
    }
}

/// Zero longitude for a plane: the ascending node on the galactic plane.
///
/// The same rule `em_map::Plane::about` draws with, and deliberately not a call into it — the
/// map is a reader of this crate and not the other way about.
fn zero_longitude(pole: DVec3) -> DVec3 {
    let north = em_foundations::reference_frame::galactic::north_pole();
    let node = north.cross(pole);
    if node.length() > 1.745e-2 {
        return node.normalize();
    }
    let center = em_foundations::reference_frame::galactic::center();
    (center - pole * center.dot(pole)).normalize_or(pole.any_orthonormal_vector())
}

/// What Kepler's equation is solved to, radians, and how many steps it may take.
///
/// Far tighter than a drawn position needs; Newton on an ellipse converges in a handful of
/// steps, so the cost of asking for more is nothing and the cap is only a guard.
const KEPLER_TOLERANCE: f64 = 1.0e-12;
const KEPLER_STEPS: u32 = 32;

/// Where an orbit puts its body at `now_s`.
///
/// Only a shell until the orientation is full and an epoch says where on the ring the body was:
/// an orbit of known size and unknown orientation is a sphere of that radius, and drawing it as
/// a ring in a guessed plane would be a claim nobody measured.
///
/// `epoch_s` is the periapsis passage. For a circle that is any point, which is why an
/// eccentricity of `None` is read as zero here rather than as an obstacle: a circular orbit has
/// no periapsis to be wrong about.
fn placed_at(orbit: &Orbit, now_s: f64) -> Placed {
    let (au, sigma_au) = orbit.semi_major_au;
    let (Orientation::Known { pole, sigma_rad, node, periapsis }, Some(epoch_s)) =
        (orbit.orientation, orbit.epoch_s)
    else {
        return Placed::Shell { radius_au: au, sigma_au };
    };
    if !(orbit.period_s.0 > 0.0) || !au.is_finite() {
        return Placed::Shell { radius_au: au, sigma_au };
    }
    let e = orbit.eccentricity.map_or(0.0, |(e, _)| e).clamp(0.0, 0.999);
    let mean = std::f64::consts::TAU * (now_s - epoch_s) / orbit.period_s.0;
    let eccentric =
        em_foundations::kepler::anomaly::eccentric_from_mean_newton(mean, e, KEPLER_TOLERANCE, KEPLER_STEPS);
    let true_anomaly = em_foundations::kepler::anomaly::true_from_eccentric(eccentric, e);
    let elements = em_foundations::kepler::state::Elements {
        semi_major_axis: au,
        eccentricity: e,
        // The pole as an inclination and a node, the same way `sky::generate` writes one.
        inclination: pole.z.clamp(-1.0, 1.0).acos(),
        longitude_of_ascending_node: node,
        argument_of_periapsis: periapsis,
        true_anomaly,
    };
    // Only the geometry is wanted, so the gravitational parameter can be anything positive:
    // `to_state`'s velocity half is discarded and its position half does not use mu.
    let Some((offset_au, _)) = em_foundations::kepler::state::to_state(1.0, &elements) else {
        return Placed::Shell { radius_au: au, sigma_au };
    };
    // **Where a body has got to is a phase, and a phase drifts.** A period known to a part in
    // a hundred is a body a quarter of the way round its orbit after twenty-five turns, and a
    // belief that reported only the size and the plane said a course could be flown against it.
    // `M = tau (t - epoch) / P`, so the period's error carries `tau |t - epoch| sigma_P / P^2`
    // of anomaly with it. Doc 25: the sigma is grown by how long since it was last seen.
    let (period_s, period_sigma) = orbit.period_s;
    let drift = std::f64::consts::TAU * (now_s - epoch_s).abs() * period_sigma / (period_s * period_s);
    // Capped at half a turn, past which the body is simply somewhere on its ring and an error
    // bar longer than the ring says nothing more than that.
    let along = (sigma_rad * sigma_rad + drift * drift).sqrt().min(std::f64::consts::PI);
    let along_au = offset_au.length() * along;
    Placed::Known { offset_au, sigma_au: (sigma_au * sigma_au + along_au * along_au).sqrt() }
}

/// One place on top of another, carrying both errors.
fn added(primary: Placed, own: Placed) -> Placed {
    match (primary, own) {
        (Placed::Known { offset_au: up, sigma_au: a }, Placed::Known { offset_au: here, sigma_au: b }) => {
            Placed::Known { offset_au: up + here, sigma_au: (a * a + b * b).sqrt() }
        }
        // A shell about a primary whose own place is known is still a shell, just a wider one:
        // the body is somewhere on a sphere about a point that is itself uncertain.
        (Placed::Known { sigma_au: a, .. }, Placed::Shell { radius_au, sigma_au: b }) => {
            Placed::Shell { radius_au, sigma_au: (a * a + b * b).sqrt() }
        }
        _ => Placed::Unknown,
    }
}

impl Knowledge {
    /// What a body weighs, from whatever is believed to go round it.
    ///
    /// **The only way to weigh anything.** A satellite's period and orbit radius are its
    /// primary's mass by Kepler's third law, `mu = 4 pi^2 a^3 / P^2`, and nothing a telescope
    /// does measures a mass directly. So a planet with no moon has no mass, which is the honest
    /// answer and the one doc 25 states for Venus.
    ///
    /// The tightest satellite's answer rather than an average: the errors are dominated by how
    /// well that one orbit is known, and a badly-known moon would only widen a well-known one.
    fn weighed(&self, star: StarId, body: BodyId) -> Option<(f64, f64)> {
        self.members(star)
            .filter_map(|(subject, file)| {
                let Subject::Body { body: satellite, .. } = subject else { return None };
                if satellite == body {
                    return None;
                }
                let orbit = file
                    .orbits()
                    .iter()
                    .filter(|o| o.about == Some(body))
                    .max_by(|a, b| a.stated_s.total_cmp(&b.stated_s))?;
                let (au, sigma_au) = orbit.semi_major_au;
                let (period_s, sigma_s) = orbit.period_s;
                if !(au > 0.0 && period_s > 0.0) {
                    return None;
                }
                let a_m = au * crate::navigation::AU;
                let n = std::f64::consts::TAU / period_s;
                let mu = n * n * a_m * a_m * a_m;
                // Cubed in the axis and squared in the period, so the errors come in at those
                // weights: a percent on the axis is three on the mass.
                let fraction =
                    3.0 * (sigma_au / au).abs() + 2.0 * (sigma_s / period_s).abs();
                let kg = mu / GRAVITY;
                Some((kg, kg * fraction))
            })
            .min_by(|a, b| (a.1 / a.0).total_cmp(&(b.1 / b.0)))
    }
}

/// Newton's constant, for turning a measured `mu` into a mass anyone can read.
const GRAVITY: f64 = 6.674_30e-11;

/// A radius, from the sighting with the best-known one: an angular diameter and a range from
/// the same look, so neither has to be carried across time while the body moves.
///
/// The tightest rather than the newest. A close pass measures a radius once and to a part in
/// thousands; later looks from across the system measure it far worse or not at all, and taking
/// the newest would throw the good one away.
fn measured_radius(file: &crate::knowledge::File) -> Option<(f64, f64)> {
    file.sightings()
        .iter()
        .filter_map(|seen| {
            let ((size, size_sigma), (range, range_sigma)) = (seen.size?, seen.range_m?);
            (size.is_finite() && size > 0.0 && range > 0.0).then(|| {
                let radius = size * range / 2.0;
                // Two independent fractional errors on a product.
                let fraction = ((size_sigma / size).powi(2) + (range_sigma / range).powi(2)).sqrt();
                (radius, radius * fraction)
            })
        })
        .min_by(|a, b| (a.1 / a.0).total_cmp(&(b.1 / b.0)))
}

/// The velocity an orbit implies, meters a second in the star's frame.
///
/// The one element `placed_at` throws away. It passes `mu = 1.0` because only the geometry is
/// wanted there; a velocity needs the real one, which the orbit itself states -- `n^2 a^3` is
/// Kepler's third law read backwards, exactly as `knowledge::arc` measures it.
fn moving_at(orbit: &Orbit, now_s: f64) -> Option<(DVec3, f64)> {
    let (Orientation::Known { pole, sigma_rad, node, periapsis }, Some(epoch_s)) =
        (orbit.orientation, orbit.epoch_s)
    else {
        return None;
    };
    let (au, sigma_au) = orbit.semi_major_au;
    if !(orbit.period_s.0 > 0.0 && au.is_finite() && au > 0.0) {
        return None;
    }
    let a_m = au * crate::navigation::AU;
    let n = std::f64::consts::TAU / orbit.period_s.0;
    let mu = n * n * a_m * a_m * a_m;
    let e = orbit.eccentricity.map_or(0.0, |(e, _)| e).clamp(0.0, 0.999);
    let mean = std::f64::consts::TAU * (now_s - epoch_s) / orbit.period_s.0;
    let eccentric = em_foundations::kepler::anomaly::eccentric_from_mean_newton(
        mean, e, KEPLER_TOLERANCE, KEPLER_STEPS,
    );
    let elements = em_foundations::kepler::state::Elements {
        semi_major_axis: a_m,
        eccentricity: e,
        inclination: pole.z.clamp(-1.0, 1.0).acos(),
        longitude_of_ascending_node: node,
        argument_of_periapsis: periapsis,
        true_anomaly: em_foundations::kepler::anomaly::true_from_eccentric(eccentric, e),
    };
    let (_, velocity) = em_foundations::kepler::state::to_state(mu, &elements)?;
    // `mu` is not independent here -- it came from this same orbit's own period and axis -- so
    // the speed is `n a`, which is linear in the axis rather than going as its square root. The
    // axis's whole fractional error enters, not half of it. The direction is only as good as
    // the plane. Summed rather than in quadrature because a single fit's elements are
    // correlated and this is the conservative reading of that.
    let fraction = (sigma_au / au).abs()
        + (orbit.period_s.1 / orbit.period_s.0).abs()
        + sigma_rad.abs();
    Some((velocity, velocity.length() * fraction))
}

/// `placed_at`'s geometry, for the orbit fit's round-trip test in `knowledge::arc`. AU.
#[cfg(test)]
pub(crate) fn placed_for_test(orbit: &Orbit, now_s: f64) -> glam::DVec3 {
    match placed_at(orbit, now_s) {
        Placed::Known { offset_au, .. } => offset_au,
        other => panic!("a full orientation should place it, got {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{Lineage, NameKind, Naming};
    use crate::sky::StarId;

    const YEAR_S: f64 = crate::flight::JULIAN_YEAR_S;

    fn star() -> StarId {
        StarId::synthesise("test", 1)
    }

    fn body(key: &str) -> BodyId {
        BodyId::of(star(), key)
    }

    fn orbit(au: f64, orientation: Orientation, epoch_s: Option<f64>) -> Orbit {
        Orbit {
            about: None,
            witness: Witness(1),
            period_s: (YEAR_S, YEAR_S * 1.0e-3),
            semi_major_au: (au, au * 0.01),
            eccentricity: None,
            orientation,
            epoch_s,
            method: Method::Transit,
            stated_s: 0.0,
            lineage: Lineage::new(),
        }
    }

    fn known(pole: DVec3, sigma_rad: f64) -> Orientation {
        Orientation::Known { pole: pole.normalize(), sigma_rad, node: 0.0, periapsis: 0.0 }
    }

    fn knowledge_with(orbits: &[(&str, Orbit)]) -> Knowledge {
        let mut k = Knowledge::new(Witness(1));
        for (key, o) in orbits {
            let subject = Subject::Body { star: star(), body: body(key) };
            k.orbits(subject, o.clone());
            k.named(
                subject,
                Naming {
                    witness: Witness(1),
                    name: (*key).to_string(),
                    kind: NameKind::Relative,
                    stated_s: 0.0,
                    lineage: Lineage::new(),
                },
            );
        }
        k
    }

    /// Bodies come back outward by what is *believed*, not by when they were found.
    #[test]
    fn bodies_are_ordered_by_believed_distance() {
        let k = knowledge_with(&[
            ("c", orbit(4.0, Orientation::Unknown, None)),
            ("b", orbit(0.5, Orientation::Unknown, None)),
        ]);
        let names: Vec<_> = k.bodies_of(star(), 0.0).into_iter().filter_map(|b| b.name).collect();
        // A relative name reads after whatever this craft calls the star, which is nothing yet.
        assert_eq!(names, ["? b", "? c"]);
    }

    /// An orbit of known size and unknown orientation is a sphere of that radius, never a ring
    /// in a guessed plane. Rule 4 of doc 25.
    #[test]
    fn an_orbit_without_an_orientation_is_only_a_shell() {
        let k = knowledge_with(&[("b", orbit(2.0, Orientation::Unknown, Some(0.0)))]);
        let belief = k.body_belief(star(), body("b"), 0.0).expect("a body is held");
        assert_eq!(belief.position_now, Placed::Shell { radius_au: 2.0, sigma_au: 0.02 });

        // An orientation with no epoch is no better: it says which plane, not where on the ring.
        let k = knowledge_with(&[("b", orbit(2.0, known(DVec3::Z, 0.01), None))]);
        let belief = k.body_belief(star(), body("b"), 0.0).unwrap();
        assert!(matches!(belief.position_now, Placed::Shell { .. }), "{:?}", belief.position_now);
    }

    /// With a plane and an epoch the body is somewhere in particular, and it goes round once a
    /// period.
    #[test]
    fn a_full_orientation_places_a_body_on_its_ring() {
        let k = knowledge_with(&[("b", orbit(1.0, known(DVec3::Z, 0.0), Some(0.0)))]);
        let at = |t: f64| match k.body_belief(star(), body("b"), t).unwrap().position_now {
            Placed::Known { offset_au, .. } => offset_au,
            other => panic!("not placed: {other:?}"),
        };
        let start = at(0.0);
        assert!((start.length() - 1.0).abs() < 1.0e-9, "a circle of one AU");
        assert!(start.z.abs() < 1.0e-9, "a +Z pole puts the ring in the XY plane");

        let quarter = at(YEAR_S * 0.25);
        assert!(start.dot(quarter).abs() < 1.0e-6, "a quarter period is a quarter turn");
        let half = at(YEAR_S * 0.5);
        assert!((half + start).length() < 1.0e-6, "half a period is the far side");
        assert!((at(YEAR_S) - start).length() < 1.0e-6, "a whole period comes back");
    }

    /// The pole's own error carries the body along its ring, and that is part of how well its
    /// position is known.
    #[test]
    fn an_uncertain_pole_widens_the_position() {
        let tight = knowledge_with(&[("b", orbit(1.0, known(DVec3::Z, 0.0), Some(0.0)))]);
        let loose = knowledge_with(&[("b", orbit(1.0, known(DVec3::Z, 0.1), Some(0.0)))]);
        let sigma = |k: &Knowledge| match k.body_belief(star(), body("b"), 0.0).unwrap().position_now
        {
            Placed::Known { sigma_au, .. } => sigma_au,
            other => panic!("not placed: {other:?}"),
        };
        assert!(sigma(&loose) > sigma(&tight) * 5.0, "{} vs {}", sigma(&loose), sigma(&tight));
    }

    /// Nothing held means no plane, and an edge-on transit gets no further than the circle its
    /// pole lies on. One craft watching from one place cannot do better.
    #[test]
    fn a_plane_needs_orbits_and_edge_on_only_gives_a_circle() {
        assert_eq!(Knowledge::new(Witness(1)).system_plane(star()), SystemPlane::Unknown);

        let toward = DVec3::X;
        let k = knowledge_with(&[("b", orbit(1.0, Orientation::EdgeOnTo { toward }, Some(0.0)))]);
        assert_eq!(k.system_plane(star()), SystemPlane::Circle(toward));
    }

    /// Two orbits about one pole solve the plane, and the giant pulls the mean toward itself
    /// because it carries the angular momentum.
    #[test]
    fn a_plane_is_the_weighted_mean_of_the_poles_it_holds() {
        let pole = DVec3::new(0.2, -0.3, 0.93).normalize();
        let k = knowledge_with(&[
            ("b", orbit(1.0, known(pole, 0.01), Some(0.0))),
            ("c", orbit(5.0, known(pole, 0.01), Some(0.0))),
        ]);
        match k.system_plane(star()) {
            SystemPlane::Known { pole: solved, sigma_rad, zero } => {
                assert!((solved - pole).length() < 1.0e-9, "{solved} is not {pole}");
                assert!(sigma_rad > 0.0 && sigma_rad < 0.02, "{sigma_rad}");
                // Zero longitude lies in the plane and on the galactic plane, by construction.
                assert!(zero.dot(solved).abs() < 1.0e-9, "the zero is out of its own plane");
                let north = em_foundations::reference_frame::galactic::north_pole();
                assert!(zero.dot(north).abs() < 1.0e-9, "the zero is off the galactic plane");
            }
            other => panic!("no plane: {other:?}"),
        }
    }

    /// **A pole's sign is the direction of travel, which an edge-on reading does not settle.**
    /// Folding the two halves together is what stops one retrograde orbit cancelling a
    /// prograde one into no plane at all.
    #[test]
    fn a_retrograde_orbit_does_not_cancel_the_plane() {
        let pole = DVec3::Z;
        let k = knowledge_with(&[
            ("b", orbit(1.0, known(pole, 0.01), Some(0.0))),
            ("c", orbit(5.0, known(-pole, 0.01), Some(0.0))),
        ]);
        match k.system_plane(star()) {
            SystemPlane::Known { pole: solved, .. } => {
                assert!(solved.dot(pole).abs() > 0.999, "{solved} is not the shared plane");
            }
            other => panic!("no plane: {other:?}"),
        }
    }

    /// Orbits that disagree about the plane say so, rather than averaging into one nothing
    /// lies in.
    #[test]
    fn poles_too_scattered_to_agree_are_not_a_plane() {
        let k = knowledge_with(&[
            ("b", orbit(1.0, known(DVec3::Z, 0.01), Some(0.0))),
            ("c", orbit(5.0, known(DVec3::X, 0.01), Some(0.0))),
        ]);
        assert_eq!(k.system_plane(star()), SystemPlane::Unknown, "sixty degrees apart is a plane");
    }

    /// **A belief goes stale, and it has to say so.** A period known to a part in a hundred
    /// puts the body a quarter of the way round its orbit after twenty-five turns, and the
    /// error bar said nothing about it: only the size and the plane went in, so a belief
    /// measured once was as good a thing to fly a course against a century later.
    #[test]
    fn a_position_grows_less_certain_the_longer_since_it_was_seen() {
        let period = 3.156e7;
        let orbit = Orbit {
            witness: Witness(1),
            about: None,
            // A part in a hundred on the period, and a part in a thousand on the size.
            period_s: (period, period * 0.01),
            semi_major_au: (1.0, 0.001),
            eccentricity: Some((0.0, 0.01)),
            orientation: Orientation::Known {
                pole: DVec3::Z,
                sigma_rad: 1.0e-4,
                node: 0.0,
                periapsis: 0.0,
            },
            epoch_s: Some(0.0),
            method: crate::knowledge::Method::Astrometric,
            stated_s: 0.0,
            lineage: Vec::new(),
        };
        let sigma_at = |t: f64| match placed_at(&orbit, t) {
            Placed::Known { sigma_au, .. } => sigma_au,
            other => panic!("{other:?}"),
        };

        // At the epoch there is no drift, so it is the size and the plane and nothing else.
        let fresh = sigma_at(0.0);
        assert!(fresh < 0.002, "at the epoch it is {fresh} AU");

        // Ten orbits on, a hundredth of a period is a tenth of a turn: most of an AU.
        let later = sigma_at(10.0 * period);
        assert!(later > 0.5, "ten orbits on it is only {later} AU");
        assert!(later > 100.0 * fresh, "{later} against {fresh}");

        // It grows the same either side of the epoch: a belief is no better backwards.
        assert!((sigma_at(-10.0 * period) - later).abs() < 1.0e-9);

        // And it stops growing once the body is simply somewhere on its ring.
        let ancient = sigma_at(10_000.0 * period);
        assert!(ancient < 4.0, "an error bar longer than the ring says nothing more: {ancient}");
        assert!(ancient >= later);
    }

    /// **A moon's plane is its planet's equator, not the system's.** Uranus's retinue is 98
    /// degrees off the ecliptic and Triton goes backwards; averaging them into the system's
    /// plane pushed the scatter past the limit and left Sol reading as unsolved.
    #[test]
    fn a_moons_orbit_says_nothing_about_the_system_plane() {
        let star = StarId::synthesise("plane", 61);
        let mut k = Knowledge::new(Witness(1));
        let pole = DVec3::new(0.1, 0.2, 0.97).normalize();
        let known = |p: DVec3| Orientation::Known { pole: p, sigma_rad: 0.01, node: 0.0, periapsis: 0.0 };

        let planet = crate::knowledge::BodyId::of(star, "planet");
        for (k_i, tilt) in [0.0f64, 0.01, -0.012].iter().enumerate() {
            let body = crate::knowledge::BodyId::of(star, &format!("p{k_i}"));
            let leaned = (pole + DVec3::X * *tilt).normalize();
            k.orbits(
                Subject::Body { star, body },
                Orbit { orientation: known(leaned), about: None, ..an_orbit() },
            );
        }
        let clean = k.system_plane(star);
        let SystemPlane::Known { pole: solved, sigma_rad, .. } = clean else {
            panic!("three planets is a plane: {clean:?}")
        };
        assert!(solved.dot(pole).abs() > 0.999, "{solved} against {pole}");

        // A moon of one of them, on its side and fitted from a close pass, must not move it.
        let sideways = pole.any_orthonormal_vector();
        k.orbits(
            Subject::Body { star, body: crate::knowledge::BodyId::of(star, "moon") },
            Orbit {
                orientation: Orientation::Known { pole: sideways, sigma_rad: 1.0e-5, node: 0.0, periapsis: 0.0 },
                about: Some(planet),
                ..an_orbit()
            },
        );
        let after = k.system_plane(star);
        let SystemPlane::Known { pole: still, sigma_rad: after_sigma, .. } = after else {
            panic!("a moon must not unsolve a system: {after:?}")
        };
        assert!(still.dot(solved).abs() > 0.9999, "a moon moved the system plane");
        assert!((after_sigma - sigma_rad).abs() < 1.0e-9, "and it must not move the scatter");
    }

    /// The basis may not turn on which body was found first. Folding onto "whichever half the
    /// first one picked" flipped it whenever a newly found inner body ran the other way, and
    /// zero longitude moved to the other node with it.
    #[test]
    fn the_plane_does_not_flip_when_a_body_is_found_inside_another() {
        let star = StarId::synthesise("plane", 62);
        let pole = DVec3::new(0.0, 0.3, 0.954).normalize();
        let known = |p: DVec3| Orientation::Known { pole: p, sigma_rad: 0.01, node: 0.0, periapsis: 0.0 };
        let solve = |orders: &[(&str, f64, DVec3)]| {
            let mut k = Knowledge::new(Witness(1));
            for (name, au, p) in orders {
                let body = crate::knowledge::BodyId::of(star, name);
                k.orbits(
                    Subject::Body { star, body },
                    Orbit {
                        orientation: known(*p),
                        semi_major_au: (*au, 0.01),
                        about: None,
                        ..an_orbit()
                    },
                );
            }
            match k.system_plane(star) {
                SystemPlane::Known { pole, zero, .. } => (pole, zero),
                other => panic!("{other:?}"),
            }
        };
        // The same three bodies, one of them counter-orbiting, found outermost first and then
        // innermost first.
        let outward = solve(&[("a", 1.0, pole), ("b", 2.0, -pole), ("c", 5.0, pole)]);
        let inward = solve(&[("c", 5.0, pole), ("b", 2.0, -pole), ("a", 1.0, pole)]);
        assert_eq!(outward, inward, "the basis turned on discovery order");
    }

    fn an_orbit() -> Orbit {
        Orbit {
            witness: Witness(1),
            about: None,
            period_s: (3.0e7, 1.0e5),
            semi_major_au: (1.0, 0.01),
            eccentricity: None,
            orientation: Orientation::Unknown,
            epoch_s: Some(0.0),
            method: crate::knowledge::Method::Astrometric,
            stated_s: 1.0,
            lineage: Vec::new(),
        }
    }
}
