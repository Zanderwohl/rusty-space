//! What a record is about.
//!
//! Every body and population belongs to a star, and a report about a system is one entry
//! however many of them it carries. See `lightcone/docs/23-factions.md`.

use serde::{Deserialize, Serialize};

use super::record::Witness;
use crate::rng;
use crate::sky::StarId;

/// A body inside a star's system: a planet, a moon, a comet.
///
/// Hashed from the star and the generator's key, so it is stable across processes and never
/// carries the generator's catalog-derived name, which must not reach a player.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BodyId(u64);

impl BodyId {
    pub fn of(star: StarId, key: &str) -> Self {
        let tag = key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
        });
        Self(rng::hash(&[star.get(), tag]))
    }

    /// A body only one craft believes in: a transit that matched no real planet.
    ///
    /// Keyed by the witness as well as the star, so two craft with the same false positive get
    /// different ids and never merge. That is the honest outcome — they have no shared object to
    /// agree about — and it is why this cannot be [`BodyId::of`] with a made-up key. `bucket`
    /// groups near-equal periods, so one craft's repeated transits of its own phantom land on
    /// one body. See `lightcone/docs/25-system-knowledge.md#which-body-a-transit-is`.
    pub fn phantom(star: StarId, witness: Witness, bucket: i64) -> Self {
        Self(rng::hash(&[star.get(), witness.0, bucket as u64, 0xfa_1_5e]))
    }

    #[inline]
    pub fn get(self) -> u64 {
        self.0
    }

    /// The id a wire message carries; see [`StarId::from_raw`].
    #[inline]
    pub fn from_raw(raw: u64) -> Self {
        Self(raw)
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

    /// What a report groups this under.
    pub fn system(self) -> Subject {
        match self.star() {
            Some(star) => Self::Star(star),
            None => self,
        }
    }

    /// A stable number for seeding a measurement's noise, distinct across the variants.
    ///
    /// A star's is its own id unchanged, so what a telescope saw before this existed is what it
    /// sees now.
    pub fn key(self) -> u64 {
        match self {
            Self::Star(star) => star.get(),
            Self::Body { star, body } => rng::hash(&[star.get(), body.get()]),
            Self::Population { star, index } => rng::hash(&[star.get(), index as u64, 0x_b_e_1_7]),
            Self::Craft(id) => rng::hash(&[id as u64, 0x_c_2_a_f_7]),
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

impl From<Subject> for lc_proto::Subject {
    fn from(subject: Subject) -> Self {
        match subject {
            Subject::Star(star) => Self::Star(star.get()),
            Subject::Body { star, body } => Self::Body { star: star.get(), body: body.get() },
            Subject::Population { star, index } => Self::Population { star: star.get(), index },
            Subject::Craft(id) => Self::Craft(id),
        }
    }
}

impl From<lc_proto::Subject> for Subject {
    fn from(subject: lc_proto::Subject) -> Self {
        match subject {
            lc_proto::Subject::Star(star) => Self::Star(StarId::from_raw(star)),
            lc_proto::Subject::Body { star, body } => {
                Self::Body { star: StarId::from_raw(star), body: BodyId::from_raw(body) }
            }
            lc_proto::Subject::Population { star, index } => {
                Self::Population { star: StarId::from_raw(star), index }
            }
            lc_proto::Subject::Craft(id) => Self::Craft(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_is_its_star_and_its_key() {
        let (a, b) = (StarId::synthesize("t", 1), StarId::synthesize("t", 2));
        assert_eq!(BodyId::of(a, "b"), BodyId::of(a, "b"));
        assert_ne!(BodyId::of(a, "b"), BodyId::of(a, "c"));
        assert_ne!(BodyId::of(a, "b"), BodyId::of(b, "b"), "the same key around another star");
    }

    #[test]
    fn everything_in_a_system_is_grouped_under_its_star() {
        let star = StarId::synthesize("t", 1);
        let planet = Subject::Body { star, body: BodyId::of(star, "b") };
        let belt = Subject::Population { star, index: 0 };
        assert_eq!(planet.system(), Subject::Star(star));
        assert_eq!(belt.system(), Subject::Star(star));
        assert_eq!(Subject::Craft(7).system(), Subject::Craft(7));
        assert_eq!(Subject::from(star).as_star(), Some(star));
        assert_eq!(planet.as_star(), None);
    }
}
