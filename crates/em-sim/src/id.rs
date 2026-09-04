//! Body identity: a stable hash for persistence, a dense index for the inner loop.
//!
//! [`BodyId`] is derived from a body's name and survives saving, loading and reordering.
//! [`BodyIndex`] is a slot in a [`System`](crate::system::System)'s arrays, is only valid
//! for the arena that produced it, and is invalidated whenever that arena's
//! [`generation`](crate::system::System::generation) changes.
//!
//! Two levels rather than one because they want opposite things. Identity has to be
//! stable across runs, which means hashing a name. The propagation loop wants an array
//! offset, and must not pay for a hash lookup per body per step.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A stable identifier for a body, derived from its name.
///
/// Stable across processes, platforms and Rust versions, because the hash is written out
/// here rather than borrowed from `DefaultHasher` — whose output is explicitly not
/// guaranteed between releases.
///
/// Case-sensitive: `"Jupiter"` and `"jupiter"` are different bodies. That is a deliberate
/// consequence of hashing the name verbatim, and the reason a system's names should be
/// normalised at the point they are authored rather than at the point they are hashed.
#[repr(transparent)]
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct BodyId(u64);

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

impl BodyId {
    /// Hash a name into an id. FNV-1a, 64-bit.
    ///
    /// `const`, so a preset can write `const SOL: BodyId = BodyId::from_name("Sol");`.
    pub const fn from_name(name: &str) -> Self {
        let bytes = name.as_bytes();
        let mut hash = FNV_OFFSET_BASIS;
        let mut i = 0;
        while i < bytes.len() {
            hash ^= bytes[i] as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
            i += 1;
        }
        Self(hash)
    }

    /// The raw hash. For persistence and debugging; not meaningful to compare across
    /// different naming schemes.
    #[inline(always)]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Rebuild an id from a previously stored [`raw`](Self::raw) value.
    #[inline(always)]
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

impl From<&str> for BodyId {
    fn from(name: &str) -> Self {
        Self::from_name(name)
    }
}

impl fmt::Debug for BodyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BodyId({:#018x})", self.0)
    }
}
impl fmt::Display for BodyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#018x}", self.0)
    }
}

/// A slot in a [`System`](crate::system::System)'s arrays.
///
/// Runtime-only and arena-specific. Inserting or removing a body can move existing bodies,
/// so an index is only valid while the system's generation is unchanged — hold a
/// [`BodyId`] across such a change and resolve it again.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BodyIndex(u32);

impl BodyIndex {
    #[inline(always)]
    pub const fn new(slot: u32) -> Self {
        Self(slot)
    }
    #[inline(always)]
    pub const fn get(self) -> usize {
        self.0 as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Published FNV-1a 64 vectors. If these change, every saved id changes with them.
    #[test]
    fn matches_the_reference_fnv1a_vectors() {
        assert_eq!(BodyId::from_name("").raw(), 0xcbf2_9ce4_8422_2325);
        assert_eq!(BodyId::from_name("a").raw(), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(BodyId::from_name("foobar").raw(), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn is_usable_in_const_context() {
        const SOL: BodyId = BodyId::from_name("Sol");
        assert_eq!(SOL, BodyId::from_name("Sol"));
    }

    #[test]
    fn distinct_names_give_distinct_ids() {
        let a = BodyId::from_name("Earth");
        let b = BodyId::from_name("Mars");
        assert_ne!(a, b);
        // Case matters, deliberately.
        assert_ne!(BodyId::from_name("Jupiter"), BodyId::from_name("jupiter"));
    }

    #[test]
    fn round_trips_through_raw() {
        let id = BodyId::from_name("Pluto Barycenter");
        assert_eq!(BodyId::from_raw(id.raw()), id);
    }

    /// The bundled system must not contain a hash collision. 64-bit FNV over a few
    /// hundred short names makes one vanishingly unlikely, but "unlikely" is not a thing
    /// to leave unchecked when the consequence is two bodies silently becoming one.
    #[test]
    fn the_bundled_system_has_no_collisions() {
        use std::collections::HashMap;
        let contents = crate::presets::solar_system();
        let mut seen: HashMap<BodyId, String> = HashMap::new();
        for b in &contents.bodies {
            let name = match b {
                crate::universe::SomeBody::KeplerEntry(k) => k.info.id.clone(),
                crate::universe::SomeBody::FixedEntry(f) => f.info.id.clone(),
                crate::universe::SomeBody::NewtonEntry(n) => n.info.id.clone(),
                crate::universe::SomeBody::CompoundEntry(c) => c.info.id.clone(),
                crate::universe::SomeBody::CompoundMotiveEntry(c) => c.info.id.clone(),
            };
            let id = BodyId::from_name(&name);
            if let Some(other) = seen.insert(id, name.clone()) {
                assert_eq!(other, name, "hash collision between {other:?} and {name:?}");
            }
        }
        assert!(seen.len() > 200, "expected the full system, hashed {}", seen.len());
    }

    #[test]
    fn body_index_is_a_slot() {
        assert_eq!(BodyIndex::new(7).get(), 7);
    }
}
