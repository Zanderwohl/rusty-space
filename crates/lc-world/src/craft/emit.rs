//! A craft pushed by what it emits. See `lightcone/docs/31-directed-energy.md` §Emitting on purpose.

use super::Craft;
use crate::emit::Boost;
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
}
