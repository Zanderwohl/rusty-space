use std::collections::HashMap;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use crate::motive::kepler::{KeplerMotive, KeplerRotation, KeplerShape, KeplerEpoch};
use em_foundations::time::Instant;
use crate::time_map::SortedTimes;

#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Component))]
#[derive(Serialize, Deserialize, Clone)]
pub struct Motive {
    times: SortedTimes,
    motives: HashMap<Instant, (TransitionEvent, MotiveSelection)>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TransitionEvent {
    Epoch,
    SOIChange,
    Impulse,
    /// Release a Fixed motive to Newtonian physics.
    /// The Newtonian motive's velocity is interpreted as LOCAL velocity (relative to the parent's frame).
    /// Position is computed from the previous Fixed motive's resolved position at transition time.
    Release,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum MotiveSelection {
    /// Fixed position relative to a parent body (or origin if primary_id is None)
    Fixed { 
        primary_id: Option<String>,
        position: DVec3,
    },
    /// Newtonian physics - affected by gravity from Major bodies
    Newtonian { 
        position: DVec3, 
        velocity: DVec3,
    },
    /// Keplerian orbit around a primary body
    Keplerian(KeplerMotive),
}

impl MotiveSelection {
    pub fn same_kind(&self, other: &MotiveSelection) -> bool {
        match (self, other) {
            (MotiveSelection::Fixed { .. }, MotiveSelection::Fixed { .. }) => true,
            (MotiveSelection::Newtonian { .. }, MotiveSelection::Newtonian { .. }) => true,
            (MotiveSelection::Keplerian(_), MotiveSelection::Keplerian(_)) => true,
            _ => false
        }
    }

    /// Get the primary_id if this motive has one (Fixed with Some or Keplerian)
    pub fn primary_id(&self) -> Option<&str> {
        match self {
            MotiveSelection::Fixed { primary_id, .. } => primary_id.as_deref(),
            MotiveSelection::Keplerian(k) => Some(&k.primary_id),
            MotiveSelection::Newtonian { .. } => None,
        }
    }
}

impl Motive {
    fn new() -> Self {
        Self {
            times: SortedTimes::new(),
            motives: HashMap::new()
        }
    }

    /// Create an empty motive with no events (must add events before use)
    pub fn empty() -> Self {
        Self::new()
    }

    /// Check if this motive has no events
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// Check if any event occurred in the time range (start, end] using binary search.
    /// Returns true if there's at least one event with time > start AND time <= end.
    /// This is O(log n) instead of O(n).
    pub fn has_event_in_range(&self, start: Instant, end: Instant) -> bool {
        if self.times.is_empty() {
            return false;
        }
        
        // Find the first event after start
        let index_after_start = self.times.get_index_after(start);

        // If there's an event at that index and it's <= end, we have a match
        match self.times.get(index_after_start) {
            Some(&event_time) => event_time <= end,
            None => false,
        }
    }

    /// Iterate over all events in time order
    pub fn iter_events(&self) -> impl Iterator<Item = (Instant, &TransitionEvent, &MotiveSelection)> {
        self.times.iter().filter_map(|time| {
            self.motives.get(time).map(|(event, selection)| (*time, event, selection))
        })
    }

    /// Create a fixed motive at the origin (no parent)
    pub fn fixed(position: DVec3) -> Self {
        Self::fixed_with_parent(None, position)
    }

    /// Create a fixed motive relative to a parent body
    pub fn fixed_with_parent(primary_id: Option<String>, position: DVec3) -> Self {
        let mut new = Self::new();
        let zero = Instant::from_seconds_since_j2000(0.0);
        new.insert_event(zero, TransitionEvent::Epoch, MotiveSelection::Fixed { primary_id, position });
        new
    }

    pub fn newtonian(position: DVec3, velocity: DVec3) -> Self {
        let mut new = Self::new();
        let zero = Instant::from_seconds_since_j2000(0.0);
        new.insert_event(zero, TransitionEvent::Epoch, MotiveSelection::Newtonian { position, velocity });
        new
    }

    /// A motive that is this orbit for all time.
    pub fn from_keplerian(kepler: KeplerMotive) -> Self {
        let mut new = Self::new();
        new.insert_event(Instant::from_seconds_since_j2000(0.0), TransitionEvent::Epoch,
                         MotiveSelection::Keplerian(kepler));
        new
    }

    pub fn keplerian(primary_id: String, shape: KeplerShape, rotation: KeplerRotation, epoch: KeplerEpoch) -> Self {
        let mut new = Self::new();
        let zero = Instant::from_seconds_since_j2000(0.0);
        new.insert_event(zero, TransitionEvent::Epoch, MotiveSelection::Keplerian(KeplerMotive { primary_id, shape, rotation, epoch, anomalistic_period: None }));
        new
    }

    pub fn insert_event(&mut self, time: Instant, event: TransitionEvent, motive_selection: MotiveSelection) {
        self.times.insert(time);
        self.motives.insert(time, (event, motive_selection));
    }

    pub fn remove_event(&mut self, time: Instant) -> bool {
        if self.times.remove_time(time) {
            self.motives.remove(&time);
            true
        } else {
            false
        }
    }
    
    pub fn remove_all_events_after(&mut self, time: Instant) {
        let index = self.times.get_index_after(time);
        // get rid of all events after the index
        for time in self.times.remove_after(index) {
            self.motives.remove(&time);
        }
    }

    /// Invariant: There must be at least one motive.
    pub fn motive_at(&self, time: Instant) -> &(TransitionEvent, MotiveSelection) {
        let time = self.times.get_at_or_before(time)
            .expect("Invariant violated: CompoundMotive must have at least one motive.");
        self.motives.get(&time).unwrap_or_else(|| panic!(
            "Invariant violated: CompoundMotive.times holds {time} but CompoundMotive.motives has no such key."))
    }

    /// Get the motive that was active just before the motive at the given time.
    /// Returns None if there is no previous motive (i.e., the motive at `time` is the first one).
    pub fn motive_before(&self, time: Instant) -> Option<&(TransitionEvent, MotiveSelection)> {
        // First find the current motive's time
        let current_time = self.times.get_at_or_before(time)?;
        // Then find the motive before that time
        let prev_time = self.times.get_before(current_time)?;
        self.motives.get(&prev_time)
    }

    pub fn is_fixed(&self, time: Instant) -> bool {
        matches!(self.motive_at(time).1, MotiveSelection::Fixed { .. })
    }

    pub fn is_newtonian(&self, time: Instant) -> bool {
        matches!(self.motive_at(time).1, MotiveSelection::Newtonian { .. })
    }

    pub fn is_keplerian(&self, time: Instant) -> bool {
        matches!(self.motive_at(time).1, MotiveSelection::Keplerian(_))
    }
}

