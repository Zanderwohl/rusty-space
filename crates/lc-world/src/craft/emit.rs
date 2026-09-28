//! A craft pushed by what it emits, and what it has lit as observers are shown it. See
//! `lightcone/docs/31-directed-energy.md` §Emitting on purpose.

use super::{Craft, HISTORY_S};
use crate::emit::Boost;
use crate::fitting::Lit;
use crate::motion;

impl Craft {
    /// Pushed by an emission from `now_s`, committing what it will spend.
    pub fn boost(&mut self, boost: Boost, now_s: f64) {
        self.remembering(now_s, |craft| {
            let (at, beta) = motion::state_at(&craft.motion, craft.system.as_deref(), now_s)
                .unwrap_or((craft.motion.position_ly, craft.motion.beta));
            craft.motion.position_ly = at;
            craft.motion.beta = beta;
            craft.motion.begin_boosting(boost);
        });
    }

    /// What its balanced emits sent from both ends together at `t`, W, as its light left it. The
    /// fitting forgets an emit once it is out, and an observer far off has not yet seen it lit.
    pub fn balanced_w_at(&self, t: f64) -> f64 {
        self.lits.iter().filter(|lit| lit.from_s <= t && t < lit.until_s).map(|lit| lit.power_w).sum()
    }

    /// After anything that settles the account: an emit the fitting no longer holds went out at
    /// the settlement, unless it had already ended.
    pub(crate) fn note_lit(&mut self) {
        let Some(fitting) = &self.fitting else { return };
        let since = fitting.since_s();
        let held = fitting.lit();
        for lit in &mut self.lits {
            if !held.iter().any(|h| h.from_s == lit.from_s && h.power_w == lit.power_w) {
                lit.until_s = lit.until_s.min(since);
            }
        }
        for lit in held {
            match self.lits.iter_mut().find(|l| l.from_s == lit.from_s && l.power_w == lit.power_w) {
                Some(kept) => kept.until_s = lit.until_s,
                None => self.lits.push(*lit),
            }
        }
        self.lits.retain(|lit: &Lit| lit.until_s > lit.from_s && lit.until_s >= since - HISTORY_S);
    }
}
