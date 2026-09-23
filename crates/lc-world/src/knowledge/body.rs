//! What a craft believes about the bodies of a system, and about the plane they share.
//!
//! A fold over records, like [`super::Belief`] is for a star, and held nowhere: a plane drawn
//! from orbits has to move when an orbit does, and a stored copy is a second thing to keep in
//! step. See `lightcone/docs/25-system-knowledge.md`.

use glam::DVec3;

use super::conclusion::Hypothesis;
use super::record::{Method, Orbit, Orientation};
use crate::sky::StarId;
use super::subject::BodyId;
use super::{Knowledge, Subject, Witness};

/// Where a body is believed to be now, as an offset from its own star.
///
/// From the star rather than from the world origin: what an orbit says is where a body sits
/// about its primary, and the star's own position is a separate belief with its own error. A
/// reader that wants an absolute position adds the two and carries both errors.
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
    /// Whose word the orbit is on, and how many hands it passed through.
    pub stated_by: Option<Witness>,
    pub hops: usize,
}

impl Knowledge {
    /// What is believed about one body, or `None` when nothing is held about it.
    pub fn body_belief(&self, star: StarId, body: BodyId, now_s: f64) -> Option<BodyBelief> {
        let subject = Subject::Body { star, body };
        let file = self.file(subject)?;
        let orbit = file.orbits().iter().max_by(|a, b| {
            (a.witness == self.owner)
                .cmp(&(b.witness == self.owner))
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
            position_now: orbit.map_or(Placed::Unknown, |o| placed_at(o, now_s)),
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
    /// The mean of the orbit poles, weighted by `1 / sigma^2` and by class so a giant counts for
    /// more, which approximates the invariable plane without knowing a single mass. Recomputed
    /// on every call and stored nowhere: it moves whenever an orbit does.
    pub fn system_plane(&self, star: StarId) -> SystemPlane {
        let mut sum = DVec3::ZERO;
        let mut weight = 0.0;
        let mut circle: Option<DVec3> = None;
        for belief in self.bodies_of(star, 0.0) {
            match belief.orientation {
                Orientation::Known { pole, sigma_rad, .. } => {
                    // A giant's pole is the invariable plane's; a rock's is near it. Squared
                    // sigma because that is how independent errors combine.
                    let w = class_weight(&belief) / sigma_rad.max(1.0e-6).powi(2);
                    // The sign of a pole is the direction of travel, which an edge-on
                    // measurement does not settle. Fold onto whichever half the first one
                    // picked, or two counter-orbiting readings of one plane would cancel.
                    let signed = if sum.dot(pole) < 0.0 { -pole } else { pole };
                    sum += signed * w;
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
        let sigma_rad = plane_scatter(self, star, pole, weight);
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
fn plane_scatter(knowledge: &Knowledge, star: StarId, pole: DVec3, weight: f64) -> f64 {
    let mut spread = 0.0;
    let mut count = 0usize;
    for belief in knowledge.bodies_of(star, 0.0) {
        if let Orientation::Known { pole: p, .. } = belief.orientation {
            let signed = if pole.dot(p) < 0.0 { -p } else { p };
            let off = pole.dot(signed).clamp(-1.0, 1.0).acos();
            spread += off * off;
            count += 1;
        }
    }
    let scatter = if count > 1 { (spread / count as f64).sqrt() } else { 0.0 };
    scatter.max((1.0 / weight).sqrt())
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
    // How far along the ring the pole's own error carries the body, added to the size error.
    let along_au = offset_au.length() * sigma_rad;
    Placed::Known { offset_au, sigma_au: (sigma_au * sigma_au + along_au * along_au).sqrt() }
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
}
