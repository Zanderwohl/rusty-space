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

    fn arriving(&self, here: DVec3, toward: DVec3, field_rad: &PerBand<f64>, from_s: f64, to_s: f64) -> PerBand<f64> {
        let apart_rad = |at: DVec3| {
            let to = (at - here).normalize_or_zero();
            if to == DVec3::ZERO { f64::INFINITY } else { to.angle_between(toward) }
        };
        let mut flux = PerBand::splat(0.0);
        let mut add = |apart_rad: f64, each: PerBand<f64>| {
            Band::ALL.into_iter().filter(|band| apart_rad <= field_rad[*band]).for_each(|band| flux[band] += each[band]);
        };
        let widest_rad = Band::ALL.into_iter().map(|band| field_rad[band]).fold(0.0, f64::max);
        for afterglow in self.afterglows.values() {
            let apart = apart_rad(afterglow.position());
            if apart > widest_rad {
                continue;
            }
            let distance_us = afterglow.position().distance(here);
            let delay_s = distance_us * 1.0e-6;
            add(apart, afterglow.mean_flux(from_s - delay_s, to_s - delay_s, distance_us * LIGHT_MICROSECOND_M));
        }
        for craft in self.fleet.iter().filter(|craft| craft.id != self.observer) {
            let Some(&left_t) = retarded_times_at(to_s * 1.0e6, here, &craft.worldline()).last() else { continue };
            let at = craft.position_at(left_t);
            let apart = apart_rad(at);
            if apart > widest_rad {
                continue;
            }
            let left_s = left_t * 1.0e-6;
            let glow = craft.glow_at(left_s).unwrap_or_else(|| Glow::unfitted(self.balance));
            let to_observer = here - at;
            let shadow_m2 = shadow_toward_m2(craft, to_observer, left_s);
            let starlit = craft.starlit_at(left_s, to_observer);
            add(apart, glow::light(&glow, self.balance, shadow_m2, starlit, to_observer.length() * LIGHT_MICROSECOND_M).total());
        }
        flux
    }
}

#[cfg(test)]
mod tests {
    use lc_proto::{Order, Refusal};
    use lc_world::knowledge::{Sample, Subject};

    use super::*;
    use crate::field::tests::{APART_US, DYING, WATCHING, scene, until_collapse};
    use crate::journal::Memory;
    use crate::server::Server;
    use crate::transport::Loopback;
    use crate::world::{coasting, still};

    const ASIDE: ShipId = ShipId(3);

    fn stare(at: lc_proto::Gaze, integration_s: f64) -> Order {
        Order::SetDuty { duty: lc_proto::Duty::Stare { at }, integration_s }
    }

    fn place_of(at: DVec3) -> [i64; 3] {
        [at.x.round() as i64, at.y.round() as i64, at.z.round() as i64]
    }

    fn curve(server: &Server<Memory>, ship: ShipId, subject: Subject, band: Band) -> Vec<Sample> {
        let knowledge = &server.instruments.aboard[&CraftId(ship.0)].knowledge;
        knowledge.own_series(subject, band).map(|s| s.samples().to_vec()).unwrap_or_default()
    }

    /// Staring from three light-days at where a ship collapses, eight resolution elements off its
    /// star, a craft records nothing of it until the light arrives and then the afterglow, sample by
    /// sample, for thirty days, then nothing. A craft beside it staring a tenth of a radian off
    /// records none of it.
    #[tokio::test]
    async fn a_stare_at_a_collapse_records_its_afterglow_from_when_its_light_arrives() {
        let Some((mut server, mut wire)) = scene() else { return };
        let dying_at = server.ship(DYING).unwrap().position_at(0.0);
        let watcher_at = server.ship(WATCHING).unwrap().position_at(0.0);
        server.fleet.insert(still(ASIDE, watcher_at));
        server.next_ship = 4;
        let there = place_of(dying_at);
        let aside = place_of(dying_at + DVec3::X * 0.1 * APART_US);
        for (ship, at) in [(WATCHING, there), (ASIDE, aside)] {
            server.act_on_knowledge(CraftId(ship.0), &stare(lc_proto::Gaze::Place(at), 1.0e4), 0.0).unwrap();
        }
        let star_at = crate::server::course_tests::a_star().unwrap().position_ly * LIGHT_US_PER_LY;
        let apart_rad = (star_at - watcher_at).angle_between(dying_at - watcher_at);
        let optics = lc_world::knowledge::survey::Optics::of(server.ship(WATCHING).unwrap().sensor);
        let resolution_rad = optics.resolution_rad(Band::V);
        assert!(apart_rad > 3.0 * resolution_rad && apart_rad < lc_world::knowledge::survey::FIELD_RAD, "premise: its star glares on it");
        let at_t = until_collapse(&mut server, &mut wire).await.at_t;
        let afterglow = server.afterglows[&CraftId(DYING.0)];
        assert_eq!(afterglow.at_s, at_t as f64 * 1.0e-6);
        let distance_us = afterglow.position().distance(watcher_at);
        let arrives_s = afterglow.at_s + distance_us * 1.0e-6;
        let over_s = arrives_s + afterglow.duration_s;
        while (server.now_t() as f64) * 1.0e-6 < over_s + 3.0 * crate::field::tests::DAY_S {
            server.tick(&mut wire).await.unwrap();
        }

        let v = curve(&server, WATCHING, Subject::Place(there), Band::V);
        let first_light = afterglow.mean_flux(afterglow.at_s + 1.0, afterglow.at_s + 1.0e4, distance_us * LIGHT_MICROSECOND_M)[Band::V];
        let (mut before, mut during, mut after) = (0, 0, 0);
        for pair in v.windows(2) {
            let [prev, s] = pair else { unreachable!() };
            let delay_s = distance_us * 1.0e-6;
            let want = afterglow.mean_flux(prev.observed_s - delay_s, s.observed_s - delay_s, distance_us * LIGHT_MICROSECOND_M)[Band::V];
            if s.observed_s <= arrives_s {
                before += 1;
                assert!(s.deficit < 1.0e-2 * first_light, "{s:?} before the light arrives");
            } else if prev.observed_s >= arrives_s && s.observed_s <= over_s {
                during += 1;
                assert!((s.deficit - want).abs() < 5.0 * s.sigma + 1.0e-3 * want, "{s:?} against {want:e}");
                assert!(s.deficit > 5.0 * s.sigma, "{s:?}");
            } else if prev.observed_s >= over_s {
                after += 1;
                assert!(s.deficit.abs() < 5.0 * s.sigma, "{s:?} after it is over");
            }
        }
        let tick_s = v[1].observed_s - v[0].observed_s;
        assert!(before > 5 && after > 5, "{before} {after}");
        assert!((during as f64 - afterglow.duration_s / tick_s).abs() < 2.0, "{during} samples over thirty days");

        let aside = curve(&server, ASIDE, Subject::Place(aside), Band::V);
        assert!(aside.len() > during, "premise: it stared throughout");
        assert!(aside.iter().all(|s| s.deficit.abs() < 5.0 * s.sigma), "a stare a tenth of a radian off saw it");
    }

    /// The afterglow comes back with its wreck, and the wreck is kept until the afterglow's light,
    /// not its end's, has passed.
    #[cfg(feature = "storage")]
    #[tokio::test]
    async fn an_afterglow_outlives_a_restart() {
        let Some((mut server, mut wire)) = scene() else { return };
        until_collapse(&mut server, &mut wire).await;
        let afterglow = server.afterglows[&CraftId(DYING.0)];
        let restarted = crate::field::tests::restart(&server);
        assert_eq!(restarted.afterglows.get(&CraftId(DYING.0)), Some(&afterglow));
    }

    /// A craft moving across the line of sight at a tenth of light is a tenth of a radian from where
    /// its light shows it: a stare that followed where it is would see nothing.
    #[tokio::test]
    async fn a_stare_at_a_craft_follows_where_its_light_left() {
        // Ten light-seconds, and ticks of a few seconds, so an idle field is bright and barely moves.
        const APART_US: f64 = 10.0e6;
        let start_t = 100 * APART_US as i64;
        let mut server = Server::new(Memory::default(), start_t, 1);
        server.set_rate(0.01);
        server.fleet.insert(still(ShipId(1), DVec3::ZERO));
        server.fleet.insert(coasting(ShipId(2), DVec3::Y * APART_US, DVec3::X * 0.1, start_t));
        server.next_ship = 3;
        let now_s = start_t as f64 * 1.0e-6;
        let refused = server.act_on_knowledge(CraftId(1), &stare(lc_proto::Gaze::Craft(9), 1.0), now_s);
        assert_eq!(refused, Err(Refusal::NotInSight));
        server.act_on_knowledge(CraftId(1), &stare(lc_proto::Gaze::Craft(2), 1.0), now_s).unwrap();

        let seen = Seen { fleet: &server.fleet, afterglows: &server.afterglows, balance: &server.balance, observer: CraftId(1) };
        let sighted = seen.sighted(2, DVec3::ZERO, now_s).unwrap();
        let target = server.fleet.get(CraftId(2)).unwrap();
        let left_t = start_t as f64 - sighted.length();
        assert!(sighted.distance(target.position_at(left_t)) < 1.0, "{sighted}");
        assert!(sighted.angle_between(target.position_at(start_t as f64)) > 0.09);

        let mut wire = Loopback::new();
        for _ in 0..5 {
            server.tick(&mut wire).await.unwrap();
        }
        let heat = curve(&server, ShipId(1), Subject::Craft(2), Band::ThermalIr);
        assert!(heat.len() >= 4, "{heat:?}");
        assert!(heat.iter().all(|s| s.deficit > 5.0 * s.sigma), "{heat:?}");
    }
}
