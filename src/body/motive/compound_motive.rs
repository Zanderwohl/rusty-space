use std::collections::HashMap;
use bevy::math::DVec3;
use bevy::prelude::Component;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use crate::body::motive::kepler_motive::{EccentricitySMA, KeplerEpoch, KeplerEulerAngles, KeplerMotive, KeplerRotation, KeplerShape, MeanAnomalyAtJ2000};
use crate::util;
use crate::util::time_map::SortedTimes;

#[derive(Component, Serialize, Deserialize, Clone)]
pub struct Motive {
    times: SortedTimes,
    motives: HashMap<u64, (TransitionEvent, MotiveSelection)>
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TransitionEvent {
    Epoch,
    SOIChange,
    Impulse,
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
    pub fn has_event_in_range(&self, start: f64, end: f64) -> bool {
        if self.times.is_empty() {
            return false;
        }
        
        // Find the first event after start
        let index_after_start = self.times.get_index_after(start);
        
        // If there's an event at that index and it's <= end, we have a match
        if let Some(&event_time) = self.times.get(index_after_start) {
            event_time <= end
        } else {
            false
        }
    }

    /// Iterate over all events in time order
    pub fn iter_events(&self) -> impl Iterator<Item = (f64, &TransitionEvent, &MotiveSelection)> {
        self.times.iter().filter_map(|time| {
            let key = util::bitfutz::f64::to_u64(*time);
            self.motives.get(&key).map(|(event, selection)| (*time, event, selection))
        })
    }

    /// Create a fixed motive at the origin (no parent)
    pub fn fixed(position: DVec3) -> Self {
        Self::fixed_with_parent(None, position)
    }

    /// Create a fixed motive relative to a parent body
    pub fn fixed_with_parent(primary_id: Option<String>, position: DVec3) -> Self {
        let mut new = Self::new();
        new.insert_event(0.0, TransitionEvent::Epoch, MotiveSelection::Fixed { primary_id, position });
        new
    }

    pub fn newtonian(position: DVec3, velocity: DVec3) -> Self {
        let mut new = Self::new();
        new.insert_event(0.0, TransitionEvent::Epoch, MotiveSelection::Newtonian { position, velocity });
        new
    }

    pub fn keplerian(primary_id: String, shape: KeplerShape, rotation: KeplerRotation, epoch: KeplerEpoch) -> Self {
        let mut new = Self::new();
        new.insert_event(0.0, TransitionEvent::Epoch, MotiveSelection::Keplerian(KeplerMotive { primary_id, shape, rotation, epoch }));
        new
    }

    pub fn insert_event(&mut self, time: f64, event: TransitionEvent, motive_selection: MotiveSelection) {
        let key = util::bitfutz::f64::to_u64(time);
        self.times.insert(time);
        self.motives.insert(key, (event, motive_selection));
    }

    pub fn remove_event(&mut self, time: f64) -> bool {
        let key = util::bitfutz::f64::to_u64(time);
        if self.times.remove_time(time) {
            self.motives.remove(&key);
            true
        } else {
            false
        }
    }
    
    pub fn remove_all_events_after(&mut self, time: f64) {
        let index = self.times.get_index_after(time);
        // get rid of all events after the index
        let drained_times = self.times.remove_after(index);
        let keys = drained_times.iter().map(|time| util::bitfutz::f64::to_u64(*time)).collect::<Vec<u64>>();
        for key in keys {
            self.motives.remove(&key);
        }
    }

    /// Invariant: There must be at least one motive.
    pub fn motive_at(&self, time: f64) -> &(TransitionEvent, MotiveSelection) {
        let time = self.times.get_at_or_before(time).expect("Invariant violated: CompoundMotive must have at least one motive.");
        let key = util::bitfutz::f64::to_u64(time);
        self.motives.get(&key).expect(format!("Invariant violated: CompoundMotive.times gave the time {}, but CompoundMotive.time motives has no such key {}.", time, key).as_ref())
    }

    pub fn is_fixed(&self, time: f64) -> bool {
        let (_, motive) = self.motive_at(time);
        MotiveSelection::Fixed { primary_id: None, position: DVec3::ZERO }.same_kind(motive)
    }
    
    pub fn is_newtonian(&self, time: f64) -> bool {
        let (_, motive) = self.motive_at(time);
        MotiveSelection::Newtonian { position: DVec3::ZERO, velocity: DVec3::ZERO}.same_kind(motive)
    }
    
    pub fn is_keplerian(&self, time: f64) -> bool {
        let (_, motive) = self.motive_at(time);
        KEPLER_COMPARISON_EMPTY.same_kind(motive)
    }
}

lazy_static! {
    pub static ref KEPLER_COMPARISON_EMPTY: MotiveSelection = MotiveSelection::Keplerian(KeplerMotive {
        primary_id: String::from(""),
        shape: KeplerShape::EccentricitySMA(EccentricitySMA {
            eccentricity: 0.0,
            semi_major_axis: 0.0,
        }),
        rotation: KeplerRotation::EulerAngles(KeplerEulerAngles {
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            argument_of_periapsis: 0.0,
        }),
        epoch: KeplerEpoch::J2000(MeanAnomalyAtJ2000 {
            mean_anomaly: 0.0,
        }),
    });
}
