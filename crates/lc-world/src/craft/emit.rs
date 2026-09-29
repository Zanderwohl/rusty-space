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

    /// The half-angle its balanced emit was sent in at `t`, as its light left it. `None` while none was lit.
    pub fn balanced_spread_at(&self, t: f64) -> Option<f64> {
        self.lits.iter().find(|lit| lit.from_s <= t && t < lit.until_s).map(|lit| lit.half_angle_rad)
    }

    /// Every instant a balanced emit it remembers lit or went out, coordinate seconds.
    pub fn balanced_changes_s(&self) -> impl Iterator<Item = f64> + '_ {
        self.lits.iter().flat_map(|lit| [lit.from_s, lit.until_s])
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

#[cfg(test)]
mod tests {
    use glam::DVec3;

    use crate::craft::{Craft, CraftId, Kind};
    use crate::emit::{Ends, emit_w};
    use crate::fitting::{Balance, Fitting, Lit};
    use crate::form::presets::{Builtin, turned_fore};

    /// Put out early, a balanced emit is still lit for light that left before, and is nothing
    /// after; nothing lit is nothing.
    #[test]
    fn a_balanced_emit_is_remembered_once_it_is_out() {
        let mut craft = Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO);
        craft.fit(Some(Fitting::full(turned_fore(Builtin::Plate.form(), 1), Balance::DEFAULT, 0.0)));
        assert_eq!(emit_w(&craft, 10.0), Ends::default());
        craft.adjust(100.0, |f| f.light(Lit { from_s: 100.0, until_s: 1000.0, power_w: 2.0e18, half_angle_rad: 0.01 }));
        let both = Ends { fore_w: 1.0e18, aft_w: 1.0e18 };
        assert_eq!(emit_w(&craft, 500.0), both);
        craft.adjust(400.0, |f| f.darken());
        assert!(craft.fitting().unwrap().lit().is_empty(), "premise: the fitting forgot it");
        assert_eq!(emit_w(&craft, 300.0), both, "as its light left");
        assert_eq!(emit_w(&craft, 500.0), Ends::default(), "put out at 400");
        assert_eq!(emit_w(&craft, 50.0), Ends::default(), "before it lit");
    }
}
