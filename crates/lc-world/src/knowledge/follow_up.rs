//! A run of looks at a satellite, each gap twice the last, for an orbit whose period is unknown.
//!
//! Bearings alone fix an orbit from about a third to three quarters of it: less does not bend
//! enough, more is too many turns to count. Doubling gaps put some prefix of the run in that
//! window whatever the period, and uneven spacing avoids aliasing it. Measured from 5 AU, nine
//! looks over 128 hours fit Jupiter's moons from Metis (7 h) to Callisto (17 d).
//!
//! Not saved: after a restart a body still without an orbit starts again.

use std::collections::BTreeMap;

use super::Subject;

/// How near a brighter body a body must be seen to be followed up, radians.
///
/// Only satellites need a run; running anything else re-arms its fit at every gap and starves
/// the fit queue. Takes in Callisto from anywhere more than 0.36 AU from Jupiter; ten degrees
/// took in Mars beside Earth from 5 AU.
pub const BESIDE_RAD: f64 = 2.0 * std::f64::consts::PI / 180.0;

/// Looks in one run, counting the one that started it.
pub const LOOKS: usize = 9;

/// The first gap, seconds. Nine looks then span 128 hours.
pub const FIRST_GAP_S: f64 = 3600.0;

/// Runs one body may have. A second has gaps this many times longer, for a longer period.
pub const ROUNDS: u32 = 2;
const STRETCH: f64 = 4.0;

/// Runs at once whose next gap is under this many first gaps. Half a tick's turns is about 29
/// looks an hour and a run's early looks are three in four hours, so this keeps runs inside
/// their share and starts all of Sol's satellites within a day.
pub const DENSE: usize = 32;
const DENSE_GAPS: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    last_s: f64,
    first_gap_s: f64,
    /// Looks taken, the first included.
    taken: usize,
}

impl Run {
    /// Gaps of 1, 1, 2, 4 ... from the last look taken, so a late look stretches the run.
    fn gap_s(&self) -> f64 {
        self.first_gap_s * 2f64.powi(self.taken.saturating_sub(2) as i32)
    }

    fn due_s(&self) -> f64 {
        self.last_s + self.gap_s()
    }

    fn dense(&self) -> bool {
        self.gap_s() < self.first_gap_s * DENSE_GAPS
    }
}

/// Every body a survey is following up, and how many runs each has had.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Following {
    runs: BTreeMap<Subject, Run>,
    rounds: BTreeMap<Subject, u32>,
}

impl Following {
    /// `false` when it has had [`ROUNDS`] runs or the telescope is full; the survey offers it
    /// again on its next pass.
    pub fn begin(&mut self, subject: Subject, at_s: f64) -> bool {
        let rounds = self.rounds.get(&subject).copied().unwrap_or(0);
        if self.runs.contains_key(&subject) || rounds >= ROUNDS {
            return false;
        }
        if self.runs.values().filter(|run| run.dense()).count() >= DENSE {
            return false;
        }
        let first_gap_s = FIRST_GAP_S * STRETCH.powi(rounds as i32);
        self.runs.insert(subject, Run { last_s: at_s, first_gap_s, taken: 1 });
        self.rounds.insert(subject, rounds + 1);
        true
    }

    pub fn following(&self, subject: Subject) -> bool {
        self.runs.contains_key(&subject)
    }

    /// Bodies with a look due by `at_s`, longest overdue first.
    pub fn due(&self, at_s: f64) -> Vec<Subject> {
        let mut due: Vec<(f64, Subject)> =
            self.runs.iter().filter(|(_, run)| run.due_s() <= at_s).map(|(s, run)| (run.due_s(), *s)).collect();
        due.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        due.into_iter().map(|(_, subject)| subject).collect()
    }

    /// A look that found nothing still counts, or a body hidden behind its planet would stay
    /// due for good.
    pub fn took(&mut self, subject: Subject, at_s: f64) {
        let Some(run) = self.runs.get_mut(&subject) else { return };
        run.taken += 1;
        run.last_s = at_s;
        if run.taken >= LOOKS {
            self.runs.remove(&subject);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::BodyId;
    use crate::sky::StarId;

    fn body(n: u64) -> Subject {
        let star = StarId::synthesize("follow_up", 1);
        Subject::Body { star, body: BodyId::of(star, &format!("b{n}")) }
    }

    /// Taken on time, a run is looks at 0, 1, 2, 4 ... 128 hours, and then it is over.
    #[test]
    fn a_run_doubles_its_gaps() {
        let mut f = Following::default();
        assert!(f.begin(body(1), 0.0));
        let mut at = Vec::new();
        let mut t = 0.0;
        while f.following(body(1)) {
            t += 60.0;
            if f.due(t).contains(&body(1)) {
                at.push(t);
                f.took(body(1), t);
            }
        }
        let hours: Vec<f64> = at.iter().map(|t| (t / 3600.0).round()).collect();
        assert_eq!(hours, vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0]);
    }

    /// A late look moves the rest of the run, so no gap is shorter than planned.
    #[test]
    fn a_late_look_stretches_the_run() {
        let mut f = Following::default();
        f.begin(body(1), 0.0);
        f.took(body(1), 3600.0);
        f.took(body(1), 5.0 * 3600.0);
        assert!(f.due(6.0 * 3600.0 - 1.0).is_empty(), "the gap after a late look is squeezed");
        assert_eq!(f.due(7.0 * 3600.0), vec![body(1)]);
    }

    #[test]
    fn a_body_gets_two_runs_the_second_slower() {
        let mut f = Following::default();
        for round in 0..ROUNDS {
            assert!(f.begin(body(1), 0.0), "round {round}");
            assert!(!f.begin(body(1), 0.0), "two runs at once");
            for _ in 1..LOOKS {
                f.took(body(1), 0.0);
            }
        }
        assert!(!f.begin(body(1), 0.0), "a third run");
    }

    /// Only so many runs in their dense early looks at once; one past them makes room.
    #[test]
    fn runs_wait_for_room() {
        let mut f = Following::default();
        for n in 0..DENSE as u64 {
            assert!(f.begin(body(n), 0.0));
        }
        assert!(!f.begin(body(99), 0.0), "the telescope is full");
        for _ in 0..3 {
            f.took(body(0), 0.0);
        }
        assert!(f.begin(body(99), 0.0), "a run in its sparse looks leaves room");
    }
}
