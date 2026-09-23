//! The belief list every readout is drawn from, built at most once a frame.
//!
//! [`Knowledge::bodies_of`] walks every file a system holds and rebuilds each body's belief out
//! of its evidence, and [`Knowledge::system_plane`] folds the result again. The map, the system
//! panel and the telescope all want the same answer in the same frame, and each was paying for
//! it: eight rebuilds a frame for one list.
//!
//! Keyed by the star and the coordinate second. Both readers run after the clock has advanced,
//! so the second ask in a frame is a read, and a frame that asks for neither builds nothing.

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
    /// inventory is hashed forward once instead of being scanned per row. `None` for a body no
    /// generator made -- a transit's false positive -- which has nowhere to be flown to. Both
    /// halves of that go away in phase 7, when a course carries a subject.
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
    key: Option<(StarId, f64)>,
    held: Held,
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

    /// What this craft believes now, rebuilt only when the system or the second has changed.
    pub fn held(&mut self, session: &Session) -> &Held {
        let Some(system) = session.system.as_ref() else {
            self.key = None;
            self.held = Held::default();
            return &self.held;
        };
        let key = (system.star, session.coordinate_time_s());
        if self.key != Some(key) {
            self.key = Some(key);
            self.held = build(&session.knowledge, system, key.0, key.1);
        }
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
