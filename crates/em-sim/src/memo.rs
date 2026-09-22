//! `propagate::state_at`'s answers at a few recent instants, so a frame asking about every body
//! solves each primary once rather than once per descendant.
//!
//! Keyed by the system's [`generation`](crate::system::System::generation), which every edit
//! that can move a body advances, and by the instant. An instant gets a table only on its
//! second query: a search over future times asks about each once, and building a table each
//! time would cost more than the solve it saves.

use std::sync::Mutex;

use em_foundations::time::Instant;
use glam::DVec3;

use crate::id::BodyIndex;

/// The clock's own instant, and room for a couple of retarded ones.
const SLOTS: usize = 4;

pub(crate) type State = Option<(DVec3, DVec3)>;

#[derive(Clone, Copy, PartialEq)]
struct Key {
    generation: u32,
    /// Bits, so the key is exact.
    seconds: u64,
}

impl Key {
    fn new(generation: u32, time: Instant) -> Self {
        Self { generation, seconds: time.to_j2000_seconds().to_bits() }
    }
}

struct Slot {
    key: Key,
    /// Per body: `None` until asked.
    states: Vec<Option<State>>,
}

#[derive(Default)]
struct Inner {
    slots: Vec<Slot>,
    /// Instants asked about once. A second ask gives one a slot.
    seen: Vec<Key>,
}

#[derive(Default)]
pub(crate) struct StateMemo(Mutex<Inner>);

/// A copy starts empty: it exists to be advanced, and the first edit would discard them.
impl Clone for StateMemo {
    fn clone(&self) -> Self {
        Self::default()
    }
}

pub(crate) enum Lookup {
    Known(State),
    /// Compute it and [`StateMemo::store`] it.
    Remember,
    /// Compute it; not worth storing yet.
    Skip,
}

impl StateMemo {
    pub(crate) fn lookup(&self, generation: u32, time: Instant, i: BodyIndex, bodies: usize)
        -> Lookup {
        let key = Key::new(generation, time);
        let mut inner = self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(at) = inner.slots.iter().position(|slot| slot.key == key) {
            // Most recent first, so the clock's own instant is not the one evicted.
            inner.slots[..=at].rotate_right(1);
            return match inner.slots[0].states.get(i.get()).copied().flatten() {
                Some(state) => Lookup::Known(state),
                None => Lookup::Remember,
            };
        }
        match inner.seen.iter().position(|seen| *seen == key) {
            Some(at) => {
                inner.seen.remove(at);
                // Reuse the evicted table rather than allocate.
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
