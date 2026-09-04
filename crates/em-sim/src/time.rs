//! Simulation clock and the queue of pending physics steps.

use std::time::Instant as StdInstant;
use em_foundations::time::Instant;

/// A queue of simulation times, stored as start/count/step rather than as values, so a
/// large backlog costs no memory.
#[derive(Clone, Debug, Default)]
pub struct PreviousTimes {
    start_time: f64,
    count: usize,
    step: f64,
}

impl PreviousTimes {
    pub fn new() -> Self {
        Self {
            start_time: 0.0,
            count: 0,
            step: 1.0,
        }
    }
    
    pub fn with_values(start_time: f64, count: usize, step: f64) -> Self {
        Self { start_time, count, step }
    }
    
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
    
    pub fn len(&self) -> usize {
        self.count
    }
    
    pub fn last(&self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.start_time + self.step * (self.count - 1) as f64)
        }
    }
    
    pub fn first(&self) -> Option<f64> {
        if self.count == 0 {
            None
        } else {
            Some(self.start_time)
        }
    }
    
    pub fn get(&self, i: usize) -> Option<f64> {
        if i < self.count {
            Some(self.start_time + self.step * i as f64)
        } else {
            None
        }
    }
    
    pub fn drain_front(&mut self, n: usize) {
        let to_drain = n.min(self.count);
        self.start_time += self.step * to_drain as f64;
        self.count -= to_drain;
    }
    
    pub fn clear(&mut self) {
        self.count = 0;
    }
    
    pub fn truncate(&mut self, max: usize) {
        if self.count > max {
            self.count = max;
        }
    }
    
    /// Append `additional_count` steps. Sets `start_time` if the queue was empty.
    pub fn expand(&mut self, new_start: f64, additional_count: usize, step: f64) {
        if self.count == 0 {
            self.start_time = new_start;
            self.count = additional_count;
            self.step = step;
        } else {
            self.step = step;
            self.count += additional_count;
        }
    }
    
    pub fn set(&mut self, start_time: f64, count: usize, step: f64) {
        self.start_time = start_time;
        self.count = count;
        self.step = step;
    }
    
    pub fn iter(&self) -> PreviousTimesIter {
        PreviousTimesIter {
            current: self.start_time,
            remaining: self.count,
            step: self.step,
        }
    }
}

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

/// The simulation clock.
#[cfg_attr(feature = "bevy", derive(bevy_ecs::prelude::Resource))]
pub struct SimTime {
    pub time: Instant,
    pub previous_times: PreviousTimes,
    /// Physics step, in simulation seconds.
    pub step: f64,
    /// Sim seconds per real second.
    pub gui_speed: f64,
    pub playing: bool,
    pub seconds_only: bool,

    /// Real seconds of physics per frame. Steps past the budget defer to the next frame,
    /// so the simulation falls behind `gui_speed` rather than dropping the frame.
    pub max_frame_time: f64,

    /// Sim time not yet worth a whole step. Accumulated instead of queued, to avoid
    /// overshoot when `gui_speed * delta` is below one step.
    pub accumulated_time: f64,

    /// Simulated over requested; below 1.0 means falling behind.
    pub sim_time_fraction: f64,
    pub frame_start: Option<StdInstant>,
    pub steps_completed: usize,
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
            max_frame_time: 1.0 / 50.0,
            accumulated_time: 0.0,
            sim_time_fraction: 1.0,
            frame_start: None,
            steps_completed: 0,
            steps_requested: 0,
        }
    }
}

impl SimTime {
    pub fn begin_frame(&mut self) {
        self.frame_start = Some(StdInstant::now());
        self.steps_completed = 0;
        self.steps_requested = self.previous_times.len().max(1);
    }
    
    pub fn frame_time_exceeded(&self) -> bool {
        if let Some(start) = self.frame_start {
            start.elapsed().as_secs_f64() >= self.max_frame_time
        } else {
            false
        }
    }
    
    pub fn end_frame(&mut self) {
        if self.steps_requested > 0 {
            self.sim_time_fraction = self.steps_completed as f64 / self.steps_requested as f64;
        } else {
            self.sim_time_fraction = 1.0;
        }
        self.frame_start = None;
    }
    
    pub fn step_completed(&mut self) {
        self.steps_completed += 1;
    }
}
