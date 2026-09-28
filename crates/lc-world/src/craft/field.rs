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

#[cfg(test)]
mod tests {
    use crate::craft::{Craft, CraftId, Kind};
    use crate::field::{Burst, Mode};
    use crate::fitting::{Balance, Fitting};

    fn fitted() -> Craft {
        let mut craft = Craft::at(CraftId(1), Kind::Ship, glam::DVec3::ZERO);
        craft.fit(Some(Fitting::full(crate::form::Form::starting(), Balance::DEFAULT, 0.0)));
        craft
    }

    /// Asked about a moment already settled past, a craft answers with the field it wore then: a
    /// vent since, and a switch since, are not yet in the light that left before them.
    #[test]
    fn the_field_is_read_as_it_was_when_the_light_left() {
        let mut craft = fitted();
        let before = craft.glow_at(100.0).unwrap();
        let vent_j = 0.2 * craft.fitting().unwrap().field().heat_max_j();
        craft.adjust(1000.0, |fitting| fitting.take_burst(Burst::Vent(vent_j)));
        assert_eq!(before.shade, Mode::Black, "premise: a full fitting starts Black");
        craft.begin_switch(Mode::Clear, 2000.0).unwrap();
        let done_s = 2000.0 + Balance::DEFAULT.field_switch_s;
        craft.settle(done_s + 5000.0);

        let then = craft.glow_at(100.0).unwrap();
        assert!((then.temperature_k / before.temperature_k - 1.0).abs() < 1.0e-6, "{then:?} against {before:?}");
        let vented = craft.glow_at(1000.0).unwrap();
        assert!(vented.temperature_k > 1.2 * then.temperature_k, "{vented:?}");
        assert!(craft.glow_at(999.0).unwrap().temperature_k < 1.0001 * then.temperature_k);
        assert_eq!(craft.glow_at(done_s - 1.0).unwrap().shade, Mode::Black);
        assert_eq!(craft.glow_at(done_s).unwrap().shade, Mode::Clear);
        assert!(craft.heat_seen_j_at(999.0).unwrap() < craft.heat_seen_j_at(1000.0).unwrap());
    }

    #[test]
    fn no_fitting_no_field() {
        assert_eq!(Craft::at(CraftId(1), Kind::Ship, glam::DVec3::ZERO).glow_at(0.0), None);
    }
}
