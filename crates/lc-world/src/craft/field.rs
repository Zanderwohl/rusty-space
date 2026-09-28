//! The field's mode on a craft, and its field as observers are shown it. Each order settles first,
//! and does nothing to a craft with no fitting.

use super::Craft;
use crate::field::{Field, Mode};
use crate::fitting::{Setting, Switch, Switching};
use crate::glow::Glow;
use crate::seen::Glowed;

impl Craft {
    /// The player's order. See [`Fitting::set_setting`](crate::fitting::Fitting::set_setting).
    pub fn set_field(&mut self, setting: Setting, now_s: f64) -> Result<(), Switching> {
        self.settle(now_s);
        let set = self.fitting.as_mut().map_or(Ok(()), |fitting| fitting.set_setting(setting));
        self.note_glow();
        set
    }

    pub fn begin_switch(&mut self, to: Mode, now_s: f64) -> Result<(), Switching> {
        self.settle(now_s);
        let begun = self.fitting.as_mut().map_or(Ok(()), |fitting| fitting.begin_switch(to));
        self.note_glow();
        begun
    }

    /// A switch done by `now_s`, taken out of the account. See
    /// [`Fitting::take_flip`](crate::fitting::Fitting::take_flip).
    pub fn take_flip(&mut self, now_s: f64) -> Option<Switch> {
        self.settle(now_s);
        let flip = self.fitting.as_mut()?.take_flip();
        self.note_glow();
        flip
    }

    /// Its field as its light left it at `t`, coordinate seconds. `None` with no fitting.
    pub fn glow_at(&self, t: f64) -> Option<Glow> {
        let fitting = self.fitting.as_ref()?;
        let (heat_j, envelope_m2, shade) = self.worn_at(t)?;
        let temperature_k = Field::of(envelope_m2, fitting.balance()).temperature_k(heat_j);
        Some(Glow { temperature_k, shade, envelope_m2 })
    }

    /// Its field's heat as its light left it at `t`, J. `None` with no fitting.
    pub fn heat_seen_j_at(&self, t: f64) -> Option<f64> {
        self.worn_at(t).map(|(heat_j, _, _)| heat_j)
    }

    /// The live account from its settlement on; before it, what the settlements left.
    fn worn_at(&self, t: f64) -> Option<(f64, f64, Mode)> {
        let fitting = self.fitting.as_ref()?;
        if t >= fitting.since_s() {
            return Some((fitting.heat_j_at(&self.motion, t), fitting.geometry().envelope_area_m2, fitting.shade_at(t)));
        }
        self.glows.at(t)
    }

    /// After anything that settles the account or changes the field.
    pub(super) fn note_glow(&mut self) {
        let Some(fitting) = &self.fitting else { return };
        let posture = fitting.posture();
        self.glows.push(Glowed {
            at_s: fitting.since_s(),
            heat_j: fitting.heat_j_at(&self.motion, fitting.since_s()),
            envelope_m2: fitting.geometry().envelope_area_m2,
            shade: posture.shade,
            switch: posture.switch,
        });
    }
}
