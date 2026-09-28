//! The field's mode on a craft: each settles first, and does nothing to a craft with no fitting.

use super::Craft;
use crate::field::Mode;
use crate::fitting::{Setting, Switch, Switching};

impl Craft {
    /// The player's order. See [`Fitting::set_setting`](crate::fitting::Fitting::set_setting).
    pub fn set_field(&mut self, setting: Setting, now_s: f64) -> Result<(), Switching> {
        self.settle(now_s);
        self.fitting.as_mut().map_or(Ok(()), |fitting| fitting.set_setting(setting))
    }

    pub fn begin_switch(&mut self, to: Mode, now_s: f64) -> Result<(), Switching> {
        self.settle(now_s);
        self.fitting.as_mut().map_or(Ok(()), |fitting| fitting.begin_switch(to))
    }

    /// A switch done by `now_s`, taken out of the account. See
    /// [`Fitting::take_flip`](crate::fitting::Fitting::take_flip).
    pub fn take_flip(&mut self, now_s: f64) -> Option<Switch> {
        self.settle(now_s);
        self.fitting.as_mut()?.take_flip()
    }
}
