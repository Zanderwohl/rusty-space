//! The field's mode on a craft, and its field as observers are shown it. Each order settles first,
//! and does nothing to a craft with no fitting.

use super::Craft;
use crate::field::{Field, Mode};
use crate::fitting::{Setting, Switch, Switching};
use crate::glow::{Glow, Starlit};
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

    /// Its field as its light left it at `t`, coordinate seconds. A wreck answers from what it wore
    /// until it ended; `None` for a craft that never had a fitting.
    pub fn glow_at(&self, t: f64) -> Option<Glow> {
        let (heat_j, field, shade) = self.worn_at(t)?;
        Some(Glow { temperature_k: field.temperature_k(heat_j), shade, envelope_m2: field.area_m2 })
    }

    /// Its star as it lit it at `t`, seen from along `to_observer`. `None` between systems.
    pub fn starlit_at(&self, t: f64, to_observer: glam::DVec3) -> Option<Starlit> {
        let system = self.system.as_deref()?;
        let to_star = self.to_star(self.motion_at(t), t)?;
        let at = self.position_at(t * 1.0e6) / crate::motion::LIGHT_US_PER_LY;
        let distance_m = (system.star_position_at(t)? - at).length() * crate::system::M_PER_LY;
        Some(Starlit {
            teff_k: system.star_teff_k(),
            radius_m: system.star_radius_m(),
            distance_m,
            cos_phase: to_star.dot(to_observer.normalize_or_zero()),
        })
    }

    /// Its field's heat as its light left it at `t`, J.
    pub fn heat_seen_j_at(&self, t: f64) -> Option<f64> {
        self.worn_at(t).map(|(heat_j, _, _)| heat_j)
    }

    /// The live account from its settlement on; before it, or once the craft has ended, what the
    /// settlements left.
    fn worn_at(&self, t: f64) -> Option<(f64, Field, Mode)> {
        if let Some(fitting) = &self.fitting
            && t >= fitting.since_s()
        {
            return Some((fitting.heat_j_at(&self.motion, t), fitting.field(), fitting.shade_at(t)));
        }
        self.glows.at(t)
    }

    /// After anything that settles the account or changes the field.
    pub(crate) fn note_glow(&mut self) {
        let Some(fitting) = &self.fitting else { return };
        let posture = fitting.posture();
        self.glows.push(Glowed {
            at_s: fitting.since_s(),
            heat_j: fitting.heat_j_at(&self.motion, fitting.since_s()),
            field: fitting.field(),
            shade: posture.shade,
            switch: posture.switch,
        });
        self.note_lit();
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

    /// A wreck has no fitting left, and its light still in flight still shows the field it died in.
    #[test]
    fn a_wreck_is_seen_in_the_field_it_ended_in() {
        let mut craft = fitted();
        let hot_j = 0.9 * craft.fitting().unwrap().field().heat_max_j();
        craft.adjust(1000.0, |fitting| fitting.take_burst(Burst::Vent(hot_j)));
        let alive = craft.glow_at(1500.0).unwrap();
        craft.settle(2000.0);
        craft.end(2000.0);
        assert!(craft.fitting().is_none(), "premise: a wreck has no fitting");
        let seen = craft.glow_at(1500.0).expect("its light still in flight");
        assert!((seen.temperature_k / alive.temperature_k - 1.0).abs() < 1.0e-3, "{seen:?} against {alive:?}");
        assert_eq!(seen.shade, Mode::Black);
        assert!(seen.temperature_k > 3.0 * Balance::DEFAULT.field_idle_k);
    }

    /// A fit is the field for all time before it, and from it until the first settlement the history
    /// runs from the fit's own heat, not from whatever the settlement found.
    #[test]
    fn a_fit_is_its_own_first_sample() {
        let mut craft = fitted();
        let at_fit = craft.glow_at(0.0).unwrap();
        assert_eq!(craft.glow_at(-1.0e6), Some(at_fit), "before the fit, the fit's field");
        let hot_j = 0.5 * craft.fitting().unwrap().field().heat_max_j();
        craft.adjust(5000.0, |fitting| fitting.take_burst(Burst::Vent(hot_j)));
        craft.settle(9000.0);
        let before_vent = craft.glow_at(2500.0).unwrap();
        assert!((before_vent.temperature_k / at_fit.temperature_k - 1.0).abs() < 1.0e-3, "{before_vent:?} against {at_fit:?}");
    }

    #[test]
    fn no_fitting_no_field() {
        assert_eq!(Craft::at(CraftId(1), Kind::Ship, glam::DVec3::ZERO).glow_at(0.0), None);
    }
}
