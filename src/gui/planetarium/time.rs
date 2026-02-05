use std::time::Instant as StdInstant;
use bevy::prelude::*;
use crate::foundations::time::Instant;

/// Represents a queue of simulation times to be processed.
/// Instead of storing each time value, we store the start time and count,
/// computing values on-the-fly to avoid unbounded memory usage.
#[derive(Clone, Debug, Default)]
pub struct PreviousTimes {
    /// The first time in the queue
    start_time: f64,
    /// Number of steps remaining to process
    count: usize,
    /// Time step between each value
    step: f64,
}

impl PreviousTimes {
    /// Create an empty queue
    pub fn new() -> Self {
        Self {
            start_time: 0.0,
            count: 0,
            step: 1.0,
        }
    }
    
    /// Create a queue with the given parameters
    pub fn with_values(start_time: f64, count: usize, step: f64) -> Self {
        Self { start_time, count, step }
    }
    
    /// Returns true if there are no times to process
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    
    /// Returns the number of times remaining
    pub fn len(&self) -> usize {
        self.count
    }
    
    /// Returns the last time in the queue (or None if empty)
    pub fn last(&self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.start_time + self.step * (self.count - 1) as f64)
        }
    }
    
    /// Returns the first time in the queue (or None if empty)
    pub fn first(&self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.start_time)
        }
    }
    
    /// Get the time at index i (0-based)
    pub fn get(&self, i: usize) -> Option<f64> {
        if i < self.count {
            Some(self.start_time + self.step * i as f64)
        } else {
            None
        }
    }
    
    /// Drain n items from the front, advancing start_time
    pub fn drain_front(&mut self, n: usize) {
        let to_drain = n.min(self.count);
        self.start_time += self.step * to_drain as f64;
        self.count -= to_drain;
    }
    
    /// Clear all times
    pub fn clear(&mut self) {
        self.count = 0;
    }
    
    /// Expand the queue by adding more steps at the end.
    /// If the queue is empty, sets the start_time.
    pub fn expand(&mut self, new_start: f64, additional_count: usize, step: f64) {
        if self.count == 0 {
            // Queue is empty - set fresh values
            self.start_time = new_start;
            self.count = additional_count;
            self.step = step;
        } else {
            // Queue has items - just add to count
            // (assumes step hasn't changed, which it shouldn't mid-simulation)
            self.step = step;
            self.count += additional_count;
        }
    }
    
    /// Set the queue to have exactly this many steps starting from start_time
    pub fn set(&mut self, start_time: f64, count: usize, step: f64) {
        self.start_time = start_time;
        self.count = count;
        self.step = step;
    }
    
    /// Create an iterator that yields times without modifying the queue
    pub fn iter(&self) -> PreviousTimesIter {
        PreviousTimesIter {
            current: self.start_time,
            remaining: self.count,
            step: self.step,
        }
    }
}

/// Iterator over PreviousTimes that yields each time value
pub struct PreviousTimesIter {
    current: f64,
    remaining: usize,
    step: f64,
}

impl Iterator for PreviousTimesIter {
    type Item = f64;
    
    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            None
        } else {
            let value = self.current;
            self.current += self.step;
            self.remaining -= 1;
            Some(value)
        }
    }
    
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for PreviousTimesIter {}

#[derive(Resource)]
pub struct SimTime {
    /// Current simulation time
    pub time: Instant,
    /// Queue of simulation times that need to be stepped through
    pub previous_times: PreviousTimes,
    /// Physics time step in simulation seconds
    pub step: f64,
    /// GUI speed multiplier (sim seconds per real second)
    pub gui_speed: f64,
    /// Whether the simulation is currently playing
    pub playing: bool,
    /// Display mode - seconds only vs formatted time
    pub seconds_only: bool,
    
    // === Performance settings ===
    
    /// Maximum real-world time (in seconds) to spend on physics per frame.
    /// If exceeded, remaining steps are deferred to next frame.
    /// The simulation will naturally slow down if it can't keep up with gui_speed.
    pub max_frame_time: f64,
    
    // === Time accumulation ===
    
    /// Accumulated simulation time that hasn't been queued yet.
    /// Used when gui_speed * delta_time is less than one step - we accumulate
    /// partial time until we have enough for a full step, preventing overshoot.
    pub accumulated_time: f64,
    
    // === Performance tracking ===
    
    /// Fraction of requested sim time that was actually simulated (1.0 = keeping up, <1.0 = falling behind)
    pub sim_time_fraction: f64,
    /// When the current frame's physics calculations started
    pub frame_start: Option<StdInstant>,
    /// Number of physics steps completed this frame
    pub steps_completed: usize,
    /// Number of physics steps requested this frame
    pub steps_requested: usize,
}

impl Default for SimTime {
    fn default() -> Self {
        Self {
            time: Instant::from_seconds_since_j2000(0.0),
            previous_times: PreviousTimes::new(),
            step: 0.1,
            gui_speed: 1.0,
            playing: false,
            seconds_only: false,
            // Performance defaults
            max_frame_time: 1.0 / 10.0,
            accumulated_time: 0.0,
            sim_time_fraction: 1.0,
            frame_start: None,
            steps_completed: 0,
            steps_requested: 0,
        }
    }
}

impl SimTime {
    /// Start timing a new frame of physics calculations
    pub fn begin_frame(&mut self) {
        self.frame_start = Some(StdInstant::now());
        self.steps_completed = 0;
        self.steps_requested = self.previous_times.len().max(1);
    }
    
    /// Check if we've exceeded the frame time budget
    pub fn frame_time_exceeded(&self) -> bool {
        if let Some(start) = self.frame_start {
            start.elapsed().as_secs_f64() >= self.max_frame_time
        } else {
            false
        }
    }
    
    /// End the frame and calculate sim_time_fraction
    pub fn end_frame(&mut self) {
        if self.steps_requested > 0 {
            self.sim_time_fraction = self.steps_completed as f64 / self.steps_requested as f64;
        } else {
            self.sim_time_fraction = 1.0;
        }
        self.frame_start = None;
    }
    
    /// Record that a physics step was completed
    pub fn step_completed(&mut self) {
        self.steps_completed += 1;
    }
}
