//! What a record is about.
//!
//! Knowledge was keyed by star. A craft also learns about planets, belts and other craft, and
//! all of them are named, noted and reported the same way, so the key is a [`Subject`]. Every
//! body and population belongs to a star, and a report about a system is one entry however many
//! of them it carries — see `lightcone/docs/23-factions.md`.

use serde::{Deserialize, Serialize};

use crate::rng;
use crate::sky::StarId;

/// A body inside a star's system: a planet, a moon, a comet.
///
/// Hashed from the star and the key the system's generator knows it by, so it is stable across
/// processes and never the generator's own name — which is catalogue-derived and must not reach
/// a player any more than a star's catalogue name may.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyId(u64);

impl BodyId {
    pub fn of(star: StarId, key: &str) -> Self {
        let tag = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
        });
        Self(rng::hash(&[star.get(), tag]))
    }

    #[inline]
    pub fn get(self) -> u64 {
        self.0
    }
}

/// Anything a craft can know about.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Subject {
    Star(StarId),
    Body { star: StarId, body: BodyId },
    /// A belt, cloud or swarm, by its index in the star's list.
    Population { star: StarId, index: u32 },
    /// Another craft, by the id its shard gave it.
    Craft(i64),
}

impl Subject {
    /// The star whose system this belongs to. `None` for a craft, which belongs to nothing.
    pub fn star(self) -> Option<StarId> {
        match self {
            Self::Star(star) | Self::Body { star, .. } | Self::Population { star, .. } => {
                Some(star)
            }
            Self::Craft(_) => None,
        }
    }

    /// What a report groups this under: the star for anything in a system, the craft itself
    /// otherwise.
    pub fn system(self) -> Subject {
        match self.star() {
            Some(star) => Self::Star(star),
            None => self,
        }
    }

    pub fn as_star(self) -> Option<StarId> {
        match self {
            Self::Star(star) => Some(star),
            _ => None,
        }
    }
}

impl From<StarId> for Subject {
    fn from(star: StarId) -> Self {
        Self::Star(star)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_is_its_star_and_its_key() {
        let (a, b) = (StarId::synthesise("t", 1), StarId::synthesise("t", 2));
        assert_eq!(BodyId::of(a, "b"), BodyId::of(a, "b"));
        assert_ne!(BodyId::of(a, "b"), BodyId::of(a, "c"));
        assert_ne!(BodyId::of(a, "b"), BodyId::of(b, "b"), "the same key around another star");
    }

    #[test]
    fn everything_in_a_system_is_grouped_under_its_star() {
        let star = StarId::synthesise("t", 1);
        let planet = Subject::Body { star, body: BodyId::of(star, "b") };
        let belt = Subject::Population { star, index: 0 };
        assert_eq!(planet.system(), Subject::Star(star));
        assert_eq!(belt.system(), Subject::Star(star));
        assert_eq!(Subject::Craft(7).system(), Subject::Craft(7));
        assert_eq!(Subject::from(star).as_star(), Some(star));
        assert_eq!(planet.as_star(), None);
    }
}
