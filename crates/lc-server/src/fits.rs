//! Orbit fits, solved beside the tick rather than in it.
//!
//! One fit costs up to seconds, against a 50 ms tick. The tick takes the looks
//! ([`Knowledge::fit_job`](lc_world::knowledge::Knowledge::fit_job)), a thread solves them, and a
//! later tick files the answer. The job is stamped with the time its looks were taken, so what
//! is filed does not depend on which tick picks it up.

use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use lc_world::craft::CraftId;
use lc_world::knowledge::primary::{FitJob, Solved};

pub(crate) struct Fits {
    send: Sender<(CraftId, Option<Solved>)>,
    done: Receiver<(CraftId, Option<Solved>)>,
    /// Craft with a fit in flight. One each: a second would be fitting the same arc.
    busy: HashSet<CraftId>,
    /// Half the machine, so the tick and the network keep the rest.
    most: usize,
}

impl Default for Fits {
    fn default() -> Self {
        let (send, done) = channel();
        let cores = std::thread::available_parallelism().map_or(2, usize::from);
        Self { send, done, busy: HashSet::new(), most: (cores / 2).clamp(1, 4) }
    }
}

impl Fits {
    pub fn busy(&self, id: CraftId) -> bool {
        self.busy.contains(&id)
    }

    pub fn full(&self) -> bool {
        self.busy.len() >= self.most
    }

    pub fn start(&mut self, id: CraftId, job: FitJob) {
        self.busy.insert(id);
        let send = self.send.clone();
        std::thread::spawn(move || {
            // Always answered, or the craft is busy forever. A panic in the solve is a fit that
            // found nothing, not a lost thread.
            let solved = std::panic::catch_unwind(AssertUnwindSafe(|| job.solve())).ok().flatten();
            let _ = send.send((id, solved));
        });
    }

    /// Every fit that has finished since the last call. Never blocks.
    pub fn finished(&mut self) -> Vec<(CraftId, Solved)> {
        let mut out = Vec::new();
        loop {
            match self.done.try_recv() {
                Ok((id, solved)) => {
                    self.busy.remove(&id);
                    out.extend(solved.map(|s| (id, s)));
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Every fit in flight, waited for. For a test that needs a fit to have landed.
    #[cfg(test)]
    pub fn wait(&mut self) -> Vec<(CraftId, Solved)> {
        let mut out = Vec::new();
        while !self.busy.is_empty() {
            let Ok((id, solved)) = self.done.recv() else { break };
            self.busy.remove(&id);
            out.extend(solved.map(|s| (id, s)));
        }
        out
    }
}
