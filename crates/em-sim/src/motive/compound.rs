use std::collections::HashMap;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use crate::motive::kepler::{KeplerMotive, KeplerRotation, KeplerShape, KeplerEpoch};
use em_foundations::time::Instant;
use crate::time_map::SortedTimes;

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

    /// Build a Keplerian motive from parts, defaulting the rest.
    ///
    /// For use only when you genuinely have parts. Given a whole [`KeplerMotive`], use
    /// [`Motive::from_keplerian`]: this constructor names four of its six fields, so
    /// passing one through here silently drops `anomalistic_period` and any explicit
    /// mu. The save path did exactly that and lost every fitted period on write.
    pub fn keplerian(primary_id: String, shape: KeplerShape, rotation: KeplerRotation, epoch: KeplerEpoch) -> Self {
        let mut new = Self::new();
        let zero = Instant::from_seconds_since_j2000(0.0);
        new.insert_event(zero, TransitionEvent::Epoch, MotiveSelection::Keplerian(KeplerMotive { primary_id, shape, rotation, epoch, anomalistic_period: None, gravitational_parameter: None }));
        new
    }

    /// A Keplerian motive with an explicit gravitational parameter, for an orbit whose
    /// effective mu is not `G(M_primary + m)` — a barycentric one, for instance.
    ///
    /// Carries the same hazard as [`Motive::keplerian`]: it still drops
    /// `anomalistic_period`. Prefer [`Motive::from_keplerian`] whenever you hold a
    /// whole [`KeplerMotive`].
    pub fn keplerian_with_gm(primary_id: String, shape: KeplerShape, rotation: KeplerRotation,
                             epoch: KeplerEpoch, gravitational_parameter: f64) -> Self {
        let mut new = Self::new();
        new.insert_event(Instant::from_seconds_since_j2000(0.0), TransitionEvent::Epoch,
            MotiveSelection::Keplerian(KeplerMotive {
                primary_id, shape, rotation, epoch,
                anomalistic_period: None,
                gravitational_parameter: Some(gravitational_parameter),
            }));
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

    /// The motive in force at `time`.
    ///
    /// Invariant: there must be at least one motive. A time before every event clamps to
    /// the earliest one rather than failing — scrubbing a timeline backwards past the
    /// first event is a thing an editor does, not an error.
    pub fn motive_at(&self, time: Instant) -> &(TransitionEvent, MotiveSelection) {
        let time = self.times.get_at_or_before(time)
            .or_else(|| self.times.get(0).copied())
            .expect("Invariant violated: CompoundMotive must have at least one motive.");
        self.motives.get(&time).unwrap_or_else(|| panic!(
            "Invariant violated: CompoundMotive.times holds {time} but CompoundMotive.motives has no such key."))
    }

    /// Mutable access to the motive in force at `time`.
    ///
    /// For an editor: changing these values changes how the body moves from its epoch,
    /// not just from now.
    pub fn motive_at_mut(&mut self, time: Instant) -> Option<&mut MotiveSelection> {
        let at = self.times.get_at_or_before(time)?;
        self.motives.get_mut(&at).map(|(_, selection)| selection)
    }

    /// Get the motive that was active just before the motive at the given time.
    /// Returns None if there is no previous motive (i.e., the motive at `time` is the first one).
    pub fn motive_before(&self, time: Instant) -> Option<&(TransitionEvent, MotiveSelection)> {
        // First find the current motive's time
        let current_time = self.times.get_at_or_before(time)
            .or_else(|| self.times.get(0).copied())?;
        // Then find the motive before that time
        let prev_time = self.times.get_before(current_time)?;
        self.motives.get(&prev_time)
    }

    /// The half-open window during which the motive active at `time` stays active.
    ///
    /// `None` for a bound means "forever in that direction": the earliest segment also
    /// covers every time before it, and the last runs to the end of time. A cache keyed on
    /// the active motive can hold until the clock leaves this window — the motive itself
    /// does not change when the clock crosses an event, so watching the data alone would
    /// miss the transition.
    pub fn active_segment_range(&self, time: Instant) -> (Option<Instant>, Option<Instant>) {
        let Some(start) = self.times.get_at_or_before(time).or_else(|| self.times.get(0).copied())
        else {
            return (None, None);
        };
        let end = self.times.get(self.times.get_index_after(start)).copied();
        let start = if self.times.get(0).copied() == Some(start) { None } else { Some(start) };
        (start, end)
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

