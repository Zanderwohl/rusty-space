//! The standing parking orbit: flown to where the craft believes it belongs, and again whenever
//! that moves by more than [`parking::REPARK`]. Once held, the collectors' reading joins the
//! belief, which is how a misjudged star is found. See `lightcone/docs/20-solar-power.md` §Parking.

use lc_proto::{Order, Refusal};
use lc_world::craft::{Craft, CraftId};
use lc_world::knowledge::Knowledge;
use lc_world::motion::{Change, Event as Change_};
use lc_world::parking;
use lc_world::solar::SOLAR_STEP_S;

use crate::server::{BURN_POWER_W, KIND_BURN, Server, motion_id, refusal_for};
use crate::journal::Journal;
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

#[derive(Clone, Copy, Debug, Default)]
pub struct Park {
    pub checked_t: Option<i64>,
    /// Meters from the star. `None` until flown, as after a restart.
    pub radius_m: Option<f64>,
}

/// Meters from the star, and the course there. `parked`: whether it holds its parking orbit, the
/// only place its collectors' reading counts.
pub fn target(craft: &Craft, knowledge: &Knowledge, now_s: f64, parked: bool) -> Option<(f64, lc_world::navigation::Course)> {
    let system = craft.system.as_deref()?;
    let fitting = craft.fitting()?;
    let here = craft.position_at(now_s * 1.0e6) / lc_world::motion::LIGHT_US_PER_LY;
    let mut host = knowledge.host(system.star, here);
    if let Some(flux) = craft.flux_w_m2_at(now_s).filter(|_| parked && holding(craft)) {
        host = host.with_reading(flux);
    }
    let at = parking::for_host(fitting, now_s, &host)?;
    Some((at.distance_m, parking::course(system, at.distance_m)?))
}

fn holding(craft: &Craft) -> bool {
    matches!(craft.motion.motive, lc_world::motion::Motive::Holding(_))
}

pub fn wants_repark(held_m: Option<f64>, wanted_m: f64) -> bool {
    held_m.is_none_or(|held| (wanted_m / held - 1.0).abs() > parking::REPARK)
}

impl<J: Journal> Server<J> {
    /// [`Order::Park`], and each correction after it.
    pub(crate) fn fly_to_park(&mut self, id: CraftId, at_s: f64) -> Result<f64, Refusal> {
        let knowledge = self.instruments.aboard.get(&id).map(|a| &a.knowledge);
        let craft = self.fleet.get(id).ok_or(Refusal::NotYours)?;
        // Here rather than in `act`, so a correction meets them too.
        if craft.is_refitting(at_s) {
            return Err(Refusal::Refitting);
        }
        if self.emissions.is_emitting(id) {
            return Err(Refusal::UnderWay);
        }
        let parked = self.parks.contains_key(&id);
        let (radius_m, course) = knowledge
            .and_then(|k| target(craft, k, at_s, parked))
            .ok_or(Refusal::Uncharacterized)?;
        let drive = crate::fitting::within_budget(craft, at_s, craft.rated_drive(at_s), |drive| {
            Change::SetCourse { course: course.clone(), drive }
        })?;
        let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
        craft
            .apply(&Change_ { ship: motion_id(id), at_t: at_s, change: Change::SetCourse { course, drive } })
            .map_err(refusal_for)?;
        self.parks.insert(id, Park { checked_t: Some(self.now_t), radius_m: Some(radius_m) });
        Ok(drive.accel_g)
    }

    /// Once a solar segment. A correction that is refused waits for the next check; the order
    /// stands.
    pub(crate) fn steer_parks(&mut self, wire: &mut impl Transport, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) {
        let now = self.now_t;
        let now_s = now as f64 * 1.0e-6;
        let step_t = (SOLAR_STEP_S * 1.0e6) as i64;
        let due: Vec<CraftId> = self
            .parks
            .iter()
            .filter(|(_, park)| park.checked_t.is_none_or(|t| now - t >= step_t))
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            let held_m = self.parks.get(&id).and_then(|p| p.radius_m);
            let wanted = self.instruments.aboard.get(&id).zip(self.fleet.get(id)).and_then(|(a, craft)| target(craft, &a.knowledge, now_s, true));
            if let Some(park) = self.parks.get_mut(&id) {
                park.checked_t = Some(now);
            }
            let Some((wanted_m, _)) = wanted else { continue };
            if !wants_repark(held_m, wanted_m) || self.fly_to_park(id, now_s).is_err() {
                continue;
            }
            self.emit(id, KIND_BURN, BURN_POWER_W, "{}".into(), now, events, deliveries);
            self.tell_flying(wire, id);
        }
    }
}

pub fn ends_parking(order: &Order) -> bool {
    matches!(
        order,
        Order::Burn { .. } | Order::SetCourse { .. } | Order::Cross { .. } | Order::CutDrive | Order::Intercept { .. } | Order::BreakOff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parked_ship_moves_only_past_the_deadband() {
        assert!(wants_repark(None, 1.0), "never flown");
        assert!(!wants_repark(Some(1.0), 1.0 + 0.5 * parking::REPARK));
        assert!(wants_repark(Some(1.0), 1.0 + 2.0 * parking::REPARK));
        assert!(wants_repark(Some(1.0), 1.0 - 2.0 * parking::REPARK));
    }
}
