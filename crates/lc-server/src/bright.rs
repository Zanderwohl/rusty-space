//! Light that lasts and carries far: an event whose glow goes on for weeks and is seen from
//! light-years off, kept apart from the craft so the per-tick loops over them never touch it, and
//! asked of only by what searches a wide area for long-lived light. A wreck today; a supernova is
//! the same shape. See `lightcone/docs/30-the-field.md` §Collapse.
//!
//! Each is kept until its light has faded below [`crate::world::FAINTEST`] everywhere it could
//! land, the horizon a front is kept to, and not by which craft happen to exist: a craft made later
//! still finds it in its sky.

use std::any::Any;

use em_spectra::PerBand;
use glam::DVec3;
use lc_world::afterglow::Afterglow;
use lc_world::craft::{Craft, CraftId, Fleet};
use lc_world::signal::Beam;

use crate::server::KIND_COLLAPSE;

pub trait BrightEvent: Send + Sync + std::fmt::Debug {
    /// The event it began with, by which a sighting of it is known; `None` for one restored from a
    /// checkpoint that did not record it.
    fn event_id(&self) -> Option<i64>;
    fn source_id(&self) -> i64;
    fn kind(&self) -> i16;
    /// Where its light leaves from, light-microseconds.
    fn position(&self) -> DVec3;
    /// Coordinate seconds its light starts and stops leaving.
    fn begins_s(&self) -> f64;
    fn ends_s(&self) -> f64;
    /// W, as its light left at `t_s`.
    fn power_w(&self, t_s: f64) -> f64;
    /// The most it ever puts out, which sets how far it is seen.
    fn peak_w(&self) -> f64;
    /// Mean flux in each band, W/m², `distance_m` off, over light that left from `from_s` to `to_s`.
    fn mean_flux(&self, from_s: f64, to_s: f64, distance_m: f64) -> PerBand<f64>;
    /// What a sighting of it carries, as its event did.
    fn payload(&self) -> String;
    /// What is still seen as a craft until the light of its end arrives.
    fn hull(&self) -> Option<&Craft> {
        None
    }
    fn as_any(&self) -> &dyn Any;

    /// Coordinate microseconds after which its light is too faint for anything, anywhere. In
    /// integers: out at the edge of the world a double steps in hundreds of microseconds.
    fn faded_t(&self) -> i64 {
        let ends_t = (self.ends_s() * 1.0e6).ceil() as i64;
        ends_t.saturating_add(crate::world::reach_us(self.peak_w(), &Beam::OMNI).ceil() as i64)
    }
}

/// A collapse: the craft as it ended, seen as a hull until that light passes, and its afterglow.
#[derive(Debug)]
pub struct Wreck {
    pub craft: Craft,
    pub afterglow: Afterglow,
    pub event_id: Option<i64>,
}

impl BrightEvent for Wreck {
    fn event_id(&self) -> Option<i64> {
        self.event_id
    }

    fn source_id(&self) -> i64 {
        self.craft.id.0
    }

    fn kind(&self) -> i16 {
        KIND_COLLAPSE
    }

    fn position(&self) -> DVec3 {
        self.afterglow.position()
    }

    fn begins_s(&self) -> f64 {
        self.afterglow.at_s
    }

    fn ends_s(&self) -> f64 {
        self.afterglow.ends_s()
    }

    fn power_w(&self, t_s: f64) -> f64 {
        self.afterglow.power_w(t_s)
    }

    /// The afterglow's first light. The spike is brighter, but it is a front of its own and is
    /// followed as one.
    fn peak_w(&self) -> f64 {
        self.afterglow.power_w(self.afterglow.at_s)
    }

    fn mean_flux(&self, from_s: f64, to_s: f64, distance_m: f64) -> PerBand<f64> {
        self.afterglow.mean_flux(from_s, to_s, distance_m)
    }

    fn payload(&self) -> String {
        let released_j = self.afterglow.spike_j + self.afterglow.energy_j;
        serde_json::to_string(&lc_proto::Released { released_j }).unwrap_or_default()
    }

    fn hull(&self) -> Option<&Craft> {
        Some(&self.craft)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// One bright event's light as it reaches a place: when that light left, and from how far.
pub struct Arriving<'a> {
    pub event: &'a dyn BrightEvent,
    pub left_s: f64,
    pub distance_us: f64,
}

#[derive(Debug, Default)]
pub struct Brights {
    events: Vec<Box<dyn BrightEvent>>,
}

impl Brights {
    pub fn push(&mut self, event: Box<dyn BrightEvent>) {
        self.events.retain(|e| e.source_id() != event.source_id());
        self.events.push(event);
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn BrightEvent> {
        self.events.iter().map(|e| e.as_ref())
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn wrecks(&self) -> impl Iterator<Item = &Wreck> {
        self.events.iter().filter_map(|e| e.as_any().downcast_ref::<Wreck>())
    }

    pub fn wreck(&self, id: CraftId) -> Option<&Wreck> {
        self.wrecks().find(|w| w.craft.id == id)
    }

    pub fn hulls(&self) -> impl Iterator<Item = &Craft> {
        self.events.iter().filter_map(|e| e.hull())
    }

    pub fn hull(&self, id: CraftId) -> Option<&Craft> {
        self.hulls().find(|c| c.id == id)
    }

    /// Every one whose light is reaching `here` at coordinate microseconds `t_us`, light having
    /// left it during its life.
    pub fn arriving(&self, here: DVec3, t_us: f64) -> impl Iterator<Item = Arriving<'_>> {
        self.iter().filter_map(move |event| {
            let distance_us = event.position().distance(here);
            let left_s = (t_us - distance_us) * 1.0e-6;
            (left_s >= event.begins_s() && left_s < event.ends_s()).then_some(Arriving { event, left_s, distance_us })
        })
    }

    /// Drop each whose light has faded everywhere by `now_t`, and say whose they were.
    pub fn sweep(&mut self, now_t: i64) -> Vec<i64> {
        let mut gone = Vec::new();
        self.events.retain(|e| {
            let kept = e.faded_t() > now_t;
            if !kept {
                gone.push(e.source_id());
            }
            kept
        });
        gone
    }
}

/// What can be seen as a craft: every one there is, and every wreck's hull until the light of its
/// end has gone by. Only what asks about craft *seen* takes this; what acts on craft takes the fleet.
#[derive(Clone, Copy)]
pub struct Seeable<'a> {
    pub fleet: &'a Fleet,
    pub brights: &'a Brights,
}

impl<'a> Seeable<'a> {
    pub fn get(&self, id: CraftId) -> Option<&'a Craft> {
        self.fleet.get(id).or_else(|| self.brights.hull(id))
    }

    pub fn iter(&self) -> impl Iterator<Item = &'a Craft> + 'a {
        self.fleet.iter().chain(self.brights.hulls())
    }

    /// What could be seen as a craft from `here` at coordinate microseconds `t_us`: every craft,
    /// and each hull whose end's light has yet to go by there. The rest are years of wrecks that
    /// would each cost a retarded solve to find nothing.
    pub fn from(&self, here: DVec3, t_us: f64) -> impl Iterator<Item = &'a Craft> + 'a {
        let hulls = self.brights.hulls().filter(move |hull| {
            let Some(end_s) = hull.ended_s() else { return true };
            let end_t = end_s * 1.0e6;
            end_t + hull.position_at(end_t).distance(here) > t_us
        });
        self.fleet.iter().chain(hulls)
    }
}

#[cfg(test)]
mod tests {
    use lc_proto::ShipId;
    use lc_world::fitting::Balance;

    use super::*;
    use crate::world::still;

    fn wreck(id: i64, at: DVec3, at_s: f64, released_j: f64) -> Box<Wreck> {
        let mut craft = still(ShipId(id), at);
        craft.end(at_s);
        Box::new(Wreck { craft, afterglow: Afterglow::of(at, at_s, released_j, &Balance::DEFAULT), event_id: Some(id * 10) })
    }

    /// Kept to where nothing could see its afterglow any more, which for a starting ship's collapse
    /// is the edge of the world, and not to whoever there is.
    #[test]
    fn a_wreck_is_kept_until_its_light_has_faded_everywhere() {
        let mut brights = Brights::default();
        brights.push(wreck(1, DVec3::ZERO, 0.0, 1.4e26));
        let faded_t = brights.iter().next().unwrap().faded_t();
        let ends_t = (Balance::DEFAULT.collapse_afterglow_s * 1.0e6) as i64;
        assert!(faded_t > ends_t + (1.0e6 * lc_world::flight::JULIAN_YEAR_S * 1.0e3) as i64, "seen a thousand light-years off");
        assert!(brights.sweep(ends_t + 1).is_empty(), "its last light has only just left");
        assert!(brights.sweep(faded_t - 1).is_empty(), "not a microsecond early");
        assert_eq!(brights.sweep(faded_t), vec![1]);
        assert!(brights.is_empty());
    }

    /// Its light reaches a place a second off a second after it starts, and stops when the
    /// afterglow's last has gone by.
    #[test]
    fn its_light_arrives_and_goes_by() {
        let mut brights = Brights::default();
        brights.push(wreck(1, DVec3::ZERO, 0.0, 1.0e20));
        let here = DVec3::X * 1.0e6;
        let ends_s = Balance::DEFAULT.collapse_afterglow_s;
        assert_eq!(brights.arriving(here, 0.5e6).count(), 0);
        let [lit] = &brights.arriving(here, 2.0e6).collect::<Vec<_>>()[..] else { panic!("lit") };
        assert!((lit.left_s - 1.0).abs() < 1e-9 && (lit.distance_us - 1.0e6).abs() < 1e-6);
        assert_eq!(brights.arriving(here, (ends_s + 2.0) * 1.0e6).count(), 0);
    }

    /// A wreck's hull is seen as a craft, beside the fleet's.
    #[test]
    fn a_hull_is_seeable_beside_the_fleet() {
        let mut fleet = Fleet::new();
        fleet.insert(still(ShipId(2), DVec3::ZERO));
        let mut brights = Brights::default();
        brights.push(wreck(1, DVec3::ZERO, 0.0, 1.0e20));
        let seeable = Seeable { fleet: &fleet, brights: &brights };
        assert!(seeable.get(CraftId(1)).is_some() && seeable.get(CraftId(2)).is_some());
        assert_eq!(seeable.iter().count(), 2);
        assert!(fleet.get(CraftId(1)).is_none(), "not a craft any more");
        let here = DVec3::X * 1.0e6;
        assert_eq!(seeable.from(here, 0.5e6).count(), 2, "its end's light is still on the way");
        assert_eq!(seeable.from(here, 1.5e6).count(), 1, "and has gone by");
    }
}
