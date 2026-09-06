use std::collections::HashMap;
use glam::DVec3;
use serde::{Deserialize, Serialize};
use crate::motive::kepler::{KeplerMotive, KeplerRotation, KeplerShape, KeplerEpoch};
use em_foundations::time::Instant;
use crate::time_map::SortedTimes;

#[derive(Serialize, Deserialize, Clone)]
pub struct Motive {
    times: SortedTimes,
    motives: HashMap<Instant, (TransitionEvent, MotiveSelection)>,
    /// How far the sphere-of-influence chain has been worked out. Solver progress, not
    /// authored data, so it is rebuilt on load rather than stored.
    #[serde(skip)]
    frontier: Frontier,
}

/// How far into the future a timeline has been solved for sphere-of-influence changes.
///
/// The arcs are evaluable at any instant whatever this says — a body always has a position.
/// What this records is how far the search for the *next* join has got, so the work can be
/// handed out a slice at a time and picked up again next frame instead of stopping the
/// world.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Default)]
pub enum Frontier {
    /// Never looked at.
    #[default]
    Unsolved,
    /// Searched up to here. Any further join lies beyond it.
    Reached(Instant),
    /// Searched as far as the solver intends to look. Nothing more will be found.
    Complete,
}

impl Frontier {
    /// Where a search should pick up, if it is not finished.
    pub fn resume_from(self) -> Option<Instant> {
        match self {
            Frontier::Unsolved => None,
            Frontier::Reached(time) => Some(time),
            Frontier::Complete => None,
        }
    }

    pub fn is_complete(self) -> bool {
        matches!(self, Frontier::Complete)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum TransitionEvent {
    Epoch,
    SOIChange,
    Impulse,
    /// Release a Fixed motive to Newtonian physics. The Newtonian velocity is local, in
    /// the parent's frame; position comes from the previous Fixed motive.
    Release,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum MotiveSelection {
    /// Fixed position relative to a parent, or to the origin when `primary_id` is `None`.
    Fixed { 
        primary_id: Option<String>,
        position: DVec3,
    },
    /// Integrated under gravity from bodies flagged major.
    Newtonian { 
        position: DVec3, 
        velocity: DVec3,
    },
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

    /// The primary, if this motive has one.
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
            motives: HashMap::new(),
            frontier: Frontier::Unsolved,
        }
    }

    /// No events. Add one before use: [`Motive::motive_at`] panics on an empty motive.
    pub fn empty() -> Self {
        Self::new()
    }

    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// Whether any event falls in `(start, end]`. O(log n).
    pub fn has_event_in_range(&self, start: Instant, end: Instant) -> bool {
        if self.times.is_empty() {
            return false;
        }
        
        let index_after_start = self.times.get_index_after(start);

        match self.times.get(index_after_start) {
            Some(&event_time) => event_time <= end,
            None => false,
        }
    }

    /// Events in time order.
    pub fn iter_events(&self) -> impl Iterator<Item = (Instant, &TransitionEvent, &MotiveSelection)> {
        self.times.iter().filter_map(|time| {
            self.motives.get(time).map(|(event, selection)| (*time, event, selection))
        })
    }

    pub fn fixed(position: DVec3) -> Self {
        Self::fixed_with_parent(None, position)
    }

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
    /// Drops `anomalistic_period` and any explicit mu, silently changing the orbit. Given a
    /// whole [`KeplerMotive`], use [`Motive::from_keplerian`].
    pub fn keplerian(primary_id: String, shape: KeplerShape, rotation: KeplerRotation, epoch: KeplerEpoch) -> Self {
        let mut new = Self::new();
        let zero = Instant::from_seconds_since_j2000(0.0);
        new.insert_event(zero, TransitionEvent::Epoch, MotiveSelection::Keplerian(KeplerMotive { primary_id, shape, rotation, epoch, anomalistic_period: None, gravitational_parameter: None }));
        new
    }

    /// Keplerian with an explicit mu, for an orbit whose effective mu is not
    /// `G(M_primary + m)` — barycentric, say. Still drops `anomalistic_period`; prefer
    /// [`Motive::from_keplerian`].
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
    
    /// Remove every solver-derived [`TransitionEvent::SOIChange`] at or after `time`,
    /// leaving authored events — the epoch, impulses, releases — alone.
    ///
    /// This is what makes re-solving cheap: an edit rewrites the tail of the timeline, and
    /// the player's own decisions in that tail survive it.
    /// How far this timeline has been solved.
    pub fn frontier(&self) -> Frontier {
        self.frontier
    }

    /// Record solver progress. Only [`crate::patch`] should be moving this.
    pub fn set_frontier(&mut self, frontier: Frontier) {
        self.frontier = frontier;
    }

    pub fn remove_derived_events_after(&mut self, time: Instant) -> usize {
        let doomed: Vec<Instant> = self
            .times
            .iter()
            .filter(|t| **t >= time)
            .filter(|t| matches!(self.motives.get(t), Some((TransitionEvent::SOIChange, _))))
            .copied()
            .collect();
        for time in &doomed {
            self.times.remove_time(*time);
            self.motives.remove(time);
        }

        // Rewriting the tail un-solves it. Anything already worked out before `time` still
        // stands, so the search resumes from there rather than from the beginning.
        self.frontier = match self.times.get(0).copied() {
            Some(first) if time > first => Frontier::Reached(time),
            _ => Frontier::Unsolved,
        };
        doomed.len()
    }

    /// Every sphere-of-influence change on the timeline, in time order.
    pub fn soi_changes(&self) -> impl Iterator<Item = (Instant, &MotiveSelection)> {
        self.iter_events().filter_map(|(time, event, selection)| {
            matches!(event, TransitionEvent::SOIChange).then_some((time, selection))
        })
    }

    pub fn remove_all_events_after(&mut self, time: Instant) {
        let index = self.times.get_index_after(time);
        for time in self.times.remove_after(index) {
            self.motives.remove(&time);
        }
    }

    /// The motive in force at `time`. Panics on an empty motive; a time before every event
    /// clamps to the earliest.
    pub fn motive_at(&self, time: Instant) -> &(TransitionEvent, MotiveSelection) {
        let time = self.times.get_at_or_before(time)
            .or_else(|| self.times.get(0).copied())
            .expect("Invariant violated: CompoundMotive must have at least one motive.");
        self.motives.get(&time).unwrap_or_else(|| panic!(
            "Invariant violated: CompoundMotive.times holds {time} but CompoundMotive.motives has no such key."))
    }

    /// Mutable access to the motive in force at `time`. Edits apply from its epoch, not
    /// from `time`.
    pub fn motive_at_mut(&mut self, time: Instant) -> Option<&mut MotiveSelection> {
        let at = self.times.get_at_or_before(time)?;
        self.motives.get_mut(&at).map(|(_, selection)| selection)
    }

    /// The motive preceding the one active at `time`. `None` if that is the first.
    pub fn motive_before(&self, time: Instant) -> Option<&(TransitionEvent, MotiveSelection)> {
        let current_time = self.times.get_at_or_before(time)
            .or_else(|| self.times.get(0).copied())?;
        let prev_time = self.times.get_before(current_time)?;
        self.motives.get(&prev_time)
    }

    /// The half-open window over which the motive active at `time` stays active. A `None`
    /// bound is unbounded in that direction. Cache invalidation must key on this window:
    /// the motive data does not change when the clock crosses an event.
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

