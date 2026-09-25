//! The belief list every readout is drawn from, built at most once a frame.
//!
//! [`Knowledge::bodies_of`] walks every file a system holds and rebuilds each body's belief out
//! of its evidence, and [`Knowledge::system_plane`] folds the result again. The map, the system
//! panel and the telescope all want the same answer in the same frame.
//!
//! Built again only when the star or what the craft knows has changed ([`Knowledge::revision`]);
//! between those, a new time only moves each body along its orbit. A frame that asks for neither
//! builds nothing.

use bevy::prelude::Resource;
use lc_world::knowledge::{BodyBelief, BodyId, Knowledge, SystemPlane};
use lc_world::navigation::Target;
use lc_world::sky::StarId;
use lc_world::system::LocalSystem;
use std::collections::HashMap;

use crate::session::Session;

/// What this craft believes is in the system it is in.
#[derive(Default)]
pub struct Held {
    /// Outward by believed distance, as [`Knowledge::bodies_of`] orders them.
    pub bodies: Vec<BodyBelief>,
    pub plane: SystemPlane,
    targets: HashMap<BodyId, Target>,
}

impl Held {
    /// Which truth target a believed body is, where it is one the generator made.
    ///
    /// A [`BodyId`] is a hash and cannot be turned back into the generator's key, so the
    /// inventory is hashed forward once rather than scanned per row. `None` for a body no
    /// generator made, which has nowhere to be flown to. Goes away in phase 7, when a course
    /// carries a subject.
    pub fn target(&self, body: BodyId) -> Option<&Target> {
        self.targets.get(&body)
    }

    /// A list made by hand, for a test that needs one without a session behind it.
    #[cfg(test)]
    pub(crate) fn from_parts(bodies: Vec<BodyBelief>, targets: HashMap<BodyId, Target>) -> Self {
        Self { bodies, plane: SystemPlane::Unknown, targets }
    }

    /// The belief held about whatever is at a target, if anything is.
    pub fn at(&self, target: &Target) -> Option<&BodyBelief> {
        self.bodies.iter().find(|b| self.target(b.body) == Some(target))
    }
}

/// The cache. Ask through [`Beliefs::held`]; nothing else reaches the list.
#[derive(Resource, Default)]
pub struct Beliefs {
    /// The star and the knowledge revision it was built from.
    key: Option<(StarId, u64)>,
    /// Coordinate seconds the bodies are placed at.
    at_s: f64,
    held: Held,
    #[cfg(test)]
    builds: usize,
    /// Always empty. Handed back when nothing is going to read the list, so a frame with the
    /// map hidden and no panel open pays nothing at all.
    blank: Held,
}

impl Beliefs {
    /// The list, built only if `wanted`. A caller that may not read it says so here rather
    /// than holding an `Option` it has to unwrap at every use.
    pub fn held_if(&mut self, wanted: bool, session: &Session) -> &Held {
        if wanted { self.held(session) } else { &self.blank }
    }

    /// What this craft believes now.
    pub fn held(&mut self, session: &Session) -> &Held {
        let Some(system) = session.system.as_ref() else {
            self.key = None;
            self.held = Held::default();
            return &self.held;
        };
        let key = (system.star, session.knowledge.revision());
        let now_s = session.coordinate_time_s();
        if self.key != Some(key) {
            self.key = Some(key);
            self.held = build(&session.knowledge, system, key.0, now_s);
            #[cfg(test)]
            {
                self.builds += 1;
            }
        } else if self.at_s != now_s {
            session.knowledge.move_to(&mut self.held.bodies, now_s);
        }
        self.at_s = now_s;
        &self.held
    }
}

/// What a session believes, built outright. The cache is the way in; this is for a caller that
/// has no world to hold one in.
pub fn of(session: &Session) -> Held {
    let Some(system) = session.system.as_ref() else { return Held::default() };
    build(&session.knowledge, system, system.star, session.coordinate_time_s())
}

fn build(knowledge: &Knowledge, system: &LocalSystem, star: StarId, now_s: f64) -> Held {
    let bodies = knowledge.bodies_of(star, now_s);
    let plane = Knowledge::plane_of(&bodies);
    let targets = system
        .inventory()
        .iter()
        .filter_map(|entry| match &entry.target {
            Target::Body(name) => Some((BodyId::of(star, name), entry.target.clone())),
            _ => None,
        })
        .collect();
    Held { bodies, plane, targets }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use lc_world::knowledge::{Method, Orbit, Orientation, Witness};

    /// At a star, with one planet found: an orbit of known size and known orientation, which is
    /// the only kind that moves with the clock.
    fn at_a_star() -> Session {
        let stars = lc_world::sky::AuthoredStars::sample();
        let star = lc_world::sky::StarProvider::stars(&stars)[2].clone();
        let mut session = Session::new(&stars, 3);
        session.issue_charts(30.0);
        session.ship.motion.position_ly = star.position_ly;
        session.sync_system();
        let system = session.system.clone().expect("the ship is at a star");
        let key = system
            .inventory()
            .iter()
            .find_map(|e| match &e.target {
                Target::Body(key) => Some(key.clone()),
                _ => None,
            })
            .expect("a body");
        let period = 3.0e7;
        session.knowledge.found_planet(
            star.id,
            BodyId::of(star.id, &key),
            Orbit {
                about: None,
                witness: session.knowledge.owner,
                period_s: (period, period * 1.0e-3),
                semi_major_au: (1.5, 0.15),
                eccentricity: None,
                orientation: Orientation::Known {
                    pole: DVec3::Z,
                    sigma_rad: 0.01,
                    node: 0.0,
                    periapsis: 0.0,
                },
                epoch_s: Some(0.0),
                method: Method::Transit,
                stated_s: 0.0,
                lineage: Vec::new(),
            },
            1.0,
            0.0,
        );
        session
    }

    /// A held plane is the one `Knowledge::system_plane` gives, because that is what the map's
    /// angles were measured against before the list was shared.
    #[test]
    fn a_held_plane_is_the_one_the_knowledge_reports() {
        let session = at_a_star();
        let star = session.system.as_ref().expect("a local system").star;
        assert_eq!(of(&session).plane, session.knowledge.system_plane(star));
    }

    /// Whatever the cache hands back is what a fresh build would say. A list held past the
    /// second it was built for draws every body at an angle it has already left.
    #[test]
    fn the_cache_never_answers_with_a_stale_list() {
        let mut session = at_a_star();
        let mut beliefs = Beliefs::default();
        let placed = |held: &Held| held.bodies.iter().map(|b| b.position_now).collect::<Vec<_>>();

        let first = placed(beliefs.held(&session));
        assert_eq!(first, placed(&of(&session)));
        assert_eq!(placed(beliefs.held(&session)), first, "the same second answers the same");

        session.advance(3600.0);
        let later = placed(beliefs.held(&session));
        assert_eq!(later, placed(&of(&session)), "a held list outlived its second");
        assert_ne!(later, first, "the body did not move, so nothing was proved");
        assert!(!beliefs.held(&session).targets.is_empty(), "a system has bodies to go to");
    }

    /// Only learning something builds the list again; time alone moves what is held.
    #[test]
    fn the_list_is_built_again_only_when_something_is_learned() {
        let mut session = at_a_star();
        let mut beliefs = Beliefs::default();
        beliefs.held(&session);
        session.advance(3600.0);
        beliefs.held(&session);
        assert_eq!(beliefs.builds, 1, "time alone built it again");

        let first = beliefs.held(&session).bodies[0].subject;
        session.knowledge.name_it(first, "Newfound", 1.0);
        assert_eq!(beliefs.held(&session).bodies[0].given.as_deref(), Some("Newfound"));
        assert_eq!(beliefs.builds, 2);
    }

    /// Labels are built once per thing learned, and name what was learned.
    #[test]
    fn labels_are_built_again_only_when_something_is_learned() {
        let mut session = at_a_star();
        let first = session.home_labels();
        session.advance(3600.0);
        assert!(std::sync::Arc::ptr_eq(&first, &session.home_labels()), "time alone built them");

        let system = session.system.clone().unwrap();
        let key = system
            .inventory()
            .iter()
            .find_map(|e| match &e.target {
                Target::Body(key) if e.depth > 0 => Some(key.clone()),
                _ => None,
            })
            .expect("a body under the star");
        assert_eq!(session.home_labels().of(&key), "unidentified body");
        let subject = lc_world::knowledge::Subject::Body { star: system.star, body: BodyId::of(system.star, &key) };
        session.knowledge.name_it(subject, "Newfound", 1.0);
        assert_eq!(session.home_labels().of(&key), "Newfound");
    }

    /// A body no generator made has nowhere to be flown to, which is what a false positive
    /// should mean rather than a crash or a course into empty space.
    #[test]
    fn a_phantom_body_joins_to_no_target() {
        let stars = lc_world::sky::AuthoredStars::sample();
        let star = lc_world::sky::StarProvider::stars(&stars)[2].clone();
        let system = LocalSystem::for_star(&star).expect("a generated system");
        let held = build(&Knowledge::new(Witness(1)), &system, star.id, 0.0);

        let real = system
            .inventory()
            .iter()
            .find_map(|e| match &e.target {
                Target::Body(key) => Some(key.clone()),
                _ => None,
            })
            .expect("a body");
        assert!(held.target(BodyId::of(star.id, &real)).is_some());
        assert_eq!(held.target(BodyId::from_raw(7)), None, "a phantom");
    }
}
