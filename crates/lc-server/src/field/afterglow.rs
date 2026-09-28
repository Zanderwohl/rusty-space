//! What a stare at a place or a craft sees: every craft's field by its light, and every wreck's
//! spike and afterglow. See `lightcone/docs/30-the-field.md` §Collapse and §What an observer sees.

use std::collections::HashMap;

use em_spectra::{Band, PerBand};
use glam::DVec3;
use lc_proto::ShipId;
use lc_spacetime::{LIGHT_MICROSECOND_M, retarded_times_at};
use lc_world::afterglow::Afterglow;
use lc_world::craft::{CraftId, Fleet};
use lc_world::fitting::Balance;
use lc_world::glow::{self, Glow};
use lc_world::knowledge::observatory::Lights;
use lc_world::motion::LIGHT_US_PER_LY;

use super::shadow_toward_m2;

/// The shard's light, as `observer` receives it.
pub(crate) struct Seen<'a> {
    pub(crate) fleet: &'a Fleet,
    pub(crate) afterglows: &'a HashMap<CraftId, Afterglow>,
    pub(crate) balance: &'a Balance,
    pub(crate) observer: CraftId,
}

impl Lights for Seen<'_> {
    /// Once its light has stopped, where it ended: where its afterglow is.
    fn sighted(&self, id: i64, here: DVec3, at_s: f64) -> Option<DVec3> {
        let at_t = at_s * 1.0e6;
        let now_t = at_t.round() as i64;
        if let Some(seen) = crate::chase::sighting(self.fleet, self.observer, ShipId(id), now_t) {
            return Some(seen.position_ly * LIGHT_US_PER_LY);
        }
        let wreck = self.fleet.get(CraftId(id))?;
        let end_t = wreck.ended_s()? * 1.0e6;
        let at = wreck.position_at(end_t);
        (end_t + at.distance(here) <= at_t).then_some(at)
    }

    fn arriving(&self, here: DVec3, toward: DVec3, field_rad: f64, from_s: f64, to_s: f64) -> PerBand<f64> {
        let inside = |at: DVec3| {
            let to = (at - here).normalize_or_zero();
            to != DVec3::ZERO && to.angle_between(toward) <= field_rad
        };
        let mut flux = PerBand::splat(0.0);
        let mut add = |each: PerBand<f64>| Band::ALL.into_iter().for_each(|band| flux[band] += each[band]);
        for afterglow in self.afterglows.values().filter(|a| inside(a.position())) {
            let distance_us = afterglow.position().distance(here);
            let delay_s = distance_us * 1.0e-6;
            add(afterglow.mean_flux(from_s - delay_s, to_s - delay_s, distance_us * LIGHT_MICROSECOND_M));
        }
        for craft in self.fleet.iter().filter(|craft| craft.id != self.observer) {
            let Some(&left_t) = retarded_times_at(to_s * 1.0e6, here, &craft.worldline()).last() else { continue };
            let at = craft.position_at(left_t);
            if !inside(at) {
                continue;
            }
            let left_s = left_t * 1.0e-6;
            let glow = craft.glow_at(left_s).unwrap_or_else(|| Glow::unfitted(self.balance));
            let to_observer = here - at;
            let shadow_m2 = shadow_toward_m2(craft, to_observer, left_s);
            let starlit = craft.starlit_at(left_s, to_observer);
            add(glow::light(&glow, self.balance, shadow_m2, starlit, to_observer.length() * LIGHT_MICROSECOND_M).total());
        }
        flux
    }
}
