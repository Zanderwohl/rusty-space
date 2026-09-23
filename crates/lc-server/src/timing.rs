//! Where a tick's wall-clock time went, stage by stage.
//!
//! Real time, not coordinate time: this is about whether the shard keeps its 50 ms, which the
//! world's clock cannot see. See `lightcone/docs/plans/server-tick-lag.md`.

use std::fmt;
use std::time::{Duration, Instant};

/// One tick's stages, in the order they ran.
#[derive(Clone, Debug)]
pub struct Stages {
    started: Instant,
    last: Instant,
    spent: Vec<(&'static str, Duration)>,
}

impl Default for Stages {
    fn default() -> Self {
        let now = Instant::now();
        Self { started: now, last: now, spent: Vec::new() }
    }
}

impl Stages {
    pub fn restart(&mut self) {
        let now = Instant::now();
        self.started = now;
        self.last = now;
        self.spent.clear();
    }

    /// Charge everything since the previous mark to `stage`.
    pub fn mark(&mut self, stage: &'static str) {
        let now = Instant::now();
        self.spent.push((stage, now - self.last));
        self.last = now;
    }

    pub fn total(&self) -> Duration {
        self.last - self.started
    }

    pub fn spent(&self) -> &[(&'static str, Duration)] {
        &self.spent
    }
}

impl fmt::Display for Stages {
    /// Stages under a tenth of a millisecond are left out, so the line names what mattered.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1} ms", self.total().as_secs_f64() * 1.0e3)?;
        let mut first = true;
        for (stage, spent) in &self.spent {
            let ms = spent.as_secs_f64() * 1.0e3;
            if ms < 0.1 {
                continue;
            }
            f.write_str(if first { ": " } else { ", " })?;
            first = false;
            write!(f, "{stage} {ms:.1}")?;
        }
        Ok(())
    }
}

/// Over-budget ticks, reported at most once per `every` ticks: the worst of them in full and how
/// many there were, so a sustained overrun is one line a second rather than twenty.
#[derive(Debug)]
pub struct Overruns {
    budget: Duration,
    every: u32,
    seen: u32,
    slow: u32,
    worst: Option<Stages>,
}

impl Overruns {
    pub fn new(budget: Duration, every: u32) -> Self {
        Self { budget, every: every.max(1), seen: 0, slow: 0, worst: None }
    }

    /// Note one tick; the report line when one is due.
    pub fn note(&mut self, tick: &Stages) -> Option<String> {
        self.seen += 1;
        if tick.total() > self.budget {
            self.slow += 1;
            if self.worst.as_ref().is_none_or(|w| tick.total() > w.total()) {
                self.worst = Some(tick.clone());
            }
        }
        if self.seen < self.every {
            return None;
        }
        self.seen = 0;
        let slow = std::mem::take(&mut self.slow);
        let worst = self.worst.take()?;
        Some(format!("{slow} of the last {} ticks over budget; worst {worst}", self.every))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stages(ms: &[(&'static str, u64)]) -> Stages {
        let started = Instant::now();
        let mut last = started;
        let spent = ms
            .iter()
            .map(|(stage, ms)| {
                let d = Duration::from_millis(*ms);
                last += d;
                (*stage, d)
            })
            .collect();
        Stages { started, last, spent }
    }

    #[test]
    fn a_line_names_the_stages_that_took_time() {
        let line = stages(&[("resync", 0), ("fit", 40), ("tell", 2)]).to_string();
        assert_eq!(line, "42.0 ms: fit 40.0, tell 2.0");
    }

    #[test]
    fn overruns_report_the_worst_once_per_window() {
        let mut overruns = Overruns::new(Duration::from_millis(50), 3);
        assert!(overruns.note(&stages(&[("a", 60)])).is_none());
        assert!(overruns.note(&stages(&[("a", 90)])).is_none());
        let line = overruns.note(&stages(&[("a", 10)])).expect("a window with overruns reports");
        assert!(line.starts_with("2 of the last 3"), "{line}");
        assert!(line.contains("90.0 ms"), "{line}");
        // A quiet window says nothing.
        for _ in 0..3 {
            assert!(overruns.note(&stages(&[("a", 10)])).is_none());
        }
    }
}
