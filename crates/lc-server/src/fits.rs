//! Orbit fits, solved beside the tick rather than in it.
//!
//! One fit costs up to seconds, against a 50 ms tick. The tick takes the looks
//! ([`Knowledge::fit_job`](lc_world::knowledge::Knowledge::fit_job)), a thread solves them, and a
//! later tick files the answer. The job is stamped with the time its looks were taken, so what
//! is filed does not depend on which tick picks it up.

use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use lc_world::craft::CraftId;
use lc_world::knowledge::Subject;
use lc_world::knowledge::primary::{FitJob, Solved};

/// A fit that finished, with what it was of: a fit that found nothing has no [`Solved`] to say.
type Finished = (CraftId, Subject, Option<Solved>);

pub(crate) struct Fits {
    send: Sender<Finished>,
    /// Behind a `Mutex` only to be `Sync`: a receiver is `Send` and not `Sync`, and a `Server`
    /// that is not `Sync` cannot be handed to `tokio::spawn`, which is how a test runs one.
    /// Nothing contends for it -- it is drained once a tick, on the tick.
    ///
    /// A poisoned lock is taken anyway rather than refused. What it guards is a receiver, which
    /// a panic elsewhere cannot leave half-written, and a shard that stopped fitting orbits
    /// because an unrelated thread died would be the worse outcome.
    done: Mutex<Receiver<Finished>>,
    /// Fits in flight, by craft, and what of. Several to a craft is safe: `Knowledge::fit_job`
    /// records the attempt, so the next job is always a different body. One each left a craft in
    /// a system of two hundred bodies placing them one at a time.
    busy: HashMap<CraftId, Vec<Subject>>,
    /// Half the machine, so the tick and the network keep the rest.
    most: usize,
}

impl Default for Fits {
    fn default() -> Self {
        let (send, done) = channel();
        let cores = std::thread::available_parallelism().map_or(2, usize::from);
        Self { send, done: Mutex::new(done), busy: HashMap::new(), most: (cores / 2).clamp(1, 4) }
    }
}

impl Fits {
    pub fn full(&self) -> bool {
        self.busy.values().map(Vec::len).sum::<usize>() >= self.most
    }

    /// What a craft has fits running for, oldest first.
    pub fn fitting(&self, id: CraftId) -> &[Subject] {
        self.busy.get(&id).map_or(&[], Vec::as_slice)
    }

    fn done_with(&mut self, id: CraftId, subject: Subject) {
        if let Some(running) = self.busy.get_mut(&id) {
            if let Some(at) = running.iter().position(|s| *s == subject) {
                running.remove(at);
            }
            if running.is_empty() {
                self.busy.remove(&id);
            }
        }
    }

    pub fn start(&mut self, id: CraftId, job: FitJob) {
        let subject = job.subject;
        self.busy.entry(id).or_default().push(subject);
        let send = self.send.clone();
        std::thread::spawn(move || {
            // Always answered, or the craft is busy forever. A panic in the solve is a fit that
            // found nothing, not a lost thread.
            let solved = std::panic::catch_unwind(AssertUnwindSafe(|| job.solve())).ok().flatten();
            let _ = send.send((id, subject, solved));
        });
    }

    /// Every fit that has finished since the last call. Never blocks.
    pub fn finished(&mut self) -> Vec<(CraftId, Solved)> {
        let mut taken = Vec::new();
        {
            let done = self.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            loop {
                match done.try_recv() {
                    Ok(finished) => taken.push(finished),
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
                }
            }
        }
        let mut out = Vec::new();
        for (id, subject, solved) in taken {
            self.done_with(id, subject);
            out.extend(solved.map(|s| (id, s)));
        }
        out
    }

    /// Every fit in flight, waited for. For a test that needs a fit to have landed.
    #[cfg(test)]
    pub fn wait(&mut self) -> Vec<(CraftId, Solved)> {
        let mut out = Vec::new();
        while !self.busy.is_empty() {
            let received = self.done.lock().unwrap_or_else(std::sync::PoisonError::into_inner).recv();
            let Ok((id, subject, solved)) = received else { break };
            self.done_with(id, subject);
            out.extend(solved.map(|s| (id, s)));
        }
        out
    }
}
