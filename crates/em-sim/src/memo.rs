//! Where bodies are at a few recent instants, remembered, so a frame asking about every body at
//! one instant solves each primary once rather than once per descendant.
//!
//! [`propagate::state_at`](crate::propagate::state_at) walks a body's whole parent chain and
//! solves Kepler's equation at every link, so asking about two hundred bodies re-solved the Sun
//! for each of them and a planet for each of its moons. This is the arena's answer without
//! writing the arena: keyed by the system's [`generation`](crate::system::System::generation),
//! which every edit that can move a body advances, and by the instant itself.
//!
//! An instant is remembered only once it has been asked about twice. A search over future
//! times asks about each one once, and a table built for every one of them would cost it more
//! than the solve it saves.

use std::sync::Mutex;

use em_foundations::time::Instant;
use glam::DVec3;

use crate::id::BodyIndex;

/// Instants remembered at once: the clock's own, and room for a couple of retarded ones.
const SLOTS: usize = 4;

pub(crate) type State = Option<(DVec3, DVec3)>;

#[derive(Clone, Copy, PartialEq)]
struct Key {
    generation: u32,
    /// Bits rather than the float, so the key is exact and `NaN` is merely never equal.
    seconds: u64,
}

impl Key {
    fn new(generation: u32, time: Instant) -> Self {
        Self { generation, seconds: time.to_j2000_seconds().to_bits() }
    }
}

struct Slot {
    key: Key,
    /// Per body: not yet asked, or the answer.
    states: Vec<Option<State>>,
}

#[derive(Default)]
struct Inner {
    /// Most recently used first.
    slots: Vec<Slot>,
    /// Instants asked about once, most recent first. A second ask promotes one to a slot.
    seen: Vec<Key>,
}

#[derive(Default)]
pub(crate) struct StateMemo(Mutex<Inner>);

/// A copy starts empty. The answers would still be right — the generation travels with the
/// copy — but a copy exists to be advanced, and the first edit would discard them anyway.
impl Clone for StateMemo {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// What the memo knows about one query.
pub(crate) enum Lookup {
    /// Answered.
    Known(State),
    /// Not remembered, but the instant is: compute and [`StateMemo::store`] it.
    Remember,
    /// Not remembered, and not worth remembering yet.
    Skip,
}

impl StateMemo {
    /// What is known about body `i` at `time`, and whether to remember the answer.
    pub(crate) fn lookup(&self, generation: u32, time: Instant, i: BodyIndex, bodies: usize)
        -> Lookup {
        let key = Key::new(generation, time);
        let mut inner = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(at) = inner.slots.iter().position(|slot| slot.key == key) {
            // Most recently used first, so the clock's own instant is never the one evicted.
            inner.slots[..=at].rotate_right(1);
            return match inner.slots[0].states.get(i.get()).copied().flatten() {
                Some(state) => Lookup::Known(state),
                None => Lookup::Remember,
            };
        }
        match inner.seen.iter().position(|seen| *seen == key) {
            Some(at) => {
                inner.seen.remove(at);
                // The evicted slot's table is reused, so a promotion allocates only while
                // the slots are filling.
                let mut states = match inner.slots.len() >= SLOTS {
                    true => inner.slots.pop().map(|slot| slot.states).unwrap_or_default(),
                    false => Vec::new(),
                };
                states.clear();
                states.resize(bodies, None);
                inner.slots.insert(0, Slot { key, states });
                Lookup::Remember
            }
            None => {
                inner.seen.insert(0, key);
                inner.seen.truncate(SLOTS);
                Lookup::Skip
            }
        }
    }

    /// The answer for a cached ancestor, if there is one. Only asked while remembering.
    pub(crate) fn known(&self, generation: u32, time: Instant, i: BodyIndex) -> Option<State> {
        let key = Key::new(generation, time);
        let inner = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let slot = inner.slots.iter().find(|slot| slot.key == key)?;
        slot.states.get(i.get()).copied().flatten()
    }

    pub(crate) fn store(&self, generation: u32, time: Instant, answers: &[(BodyIndex, State)]) {
        let key = Key::new(generation, time);
        let mut inner = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(slot) = inner.slots.iter_mut().find(|slot| slot.key == key) else { return };
        for (i, state) in answers {
            if let Some(entry) = slot.states.get_mut(i.get()) {
                *entry = Some(*state);
            }
        }
    }
}
