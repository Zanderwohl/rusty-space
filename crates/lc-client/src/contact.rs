//! Another craft as this ship sees it: one [`Presence`], reckoned forward between statements.

use glam::DVec3;
use lc_proto::{Presence, ShipId};
use lc_world::sighted::{Receiver, Reckoning};
use lc_world::system::LocalSystem;

/// Another craft, as this ship currently sees it.
///
/// Everything here is **retarded**. The position is where the light arriving now left from, so
/// a contact under way is drawn behind where it actually is, and the faster it is going the
/// further behind. That is the game rather than a lag.
///
/// Reckoned forward between statements rather than held still — see [`lc_world::sighted`] for
/// why, and for what that gets wrong. [`Contact::reckon`] runs once a frame, after the clock.
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub ship_id: ShipId,
    pub name: String,
    pub length_m: f64,
    /// Light-years from the world origin, where the light left.
    pub position_ly: DVec3,
    pub beta: DVec3,
    /// Unit vector the nose pointed along, as last stated.
    pub facing: DVec3,
    /// What its main drive was sending aft, `F c`, watts, as last stated. Zero when coasting.
    pub drive_w: f64,
    /// What its emits were sending out of each end, watts, as last stated.
    pub emit: lc_world::emit::Ends,
    /// The cone they fill out of the fore end, the aft end's being the other way, as last stated:
    /// only to an observer it glances, and `None` to everyone else.
    pub emit_cone: Option<lc_world::signal::Beam>,
    /// Coordinate seconds the light left.
    pub emitted_s: f64,
    /// Its form and the refit step it had under way, as the statement's light left it, with when
    /// that was, coordinate seconds.
    pub form: lc_proto::Form,
    pub building: Option<(f64, lc_proto::Building)>,
    /// Its field, as the statement's light left it.
    pub glow: lc_proto::Glow,
    /// Its light as this ship sees it from inside one of its beams, as last stated.
    pub glare: Option<lc_proto::Glare>,
    reckoning: Reckoning,
    /// What the statement said the drive and emits were doing, at the statement's own instant.
    stated: DriveAt,
}

/// A drive event, as a contact's plume reads it: what its drive and emits send from `at_s`,
/// coordinate seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DriveAt {
    pub at_s: f64,
    pub drive_w: f64,
    pub emit: lc_world::emit::Ends,
}

impl DriveAt {
    pub fn of(at_s: f64, change: &lc_proto::DriveChange) -> Self {
        Self {
            at_s,
            drive_w: change.power_w,
            emit: lc_world::emit::Ends { fore_w: change.emit_fore_w, aft_w: change.emit_aft_w },
        }
    }

    /// A balanced emit sends equal power out of both ends and pushes nothing.
    fn is_thrusting(&self) -> bool {
        self.drive_w > 0.0 || self.emit.fore_w != self.emit.aft_w
    }
}

impl Contact {
    /// A contact from a statement. `system` is the one this ship is in, which a contact inside
    /// it is reckoned along a conic about.
    pub fn seen(presence: Presence, system: Option<&LocalSystem>) -> Self {
        let position_ly = DVec3::from_array(presence.at_ly);
        let beta = DVec3::from_array(presence.beta);
        let emitted_s = presence.emitted_t as f64 * 1.0e-6;
        let emit = lc_world::emit::Ends { fore_w: presence.emit_fore_w, aft_w: presence.emit_aft_w };
        let stated = DriveAt { at_s: emitted_s, drive_w: presence.drive_w, emit };
        let receiver = Receiver {
            position_ly: DVec3::from_array(presence.observer_ly),
            beta: DVec3::from_array(presence.observer_beta),
            at_s: presence.arrive_t as f64 * 1.0e-6,
        };
        let sighting = lc_world::pursuit::Sighting {
            target: lc_world::motion::ShipId(presence.ship_id.0),
            position_ly,
            beta,
            length_m: presence.length_m,
            emitted_s,
        };
        Self {
            ship_id: presence.ship_id,
            // Named by `Uplink::relabel`, from what this ship knows; never from the wire.
            name: crate::craft_names::unnamed(presence.ship_id),
            length_m: presence.length_m,
            position_ly,
            beta,
            facing: DVec3::from_array(presence.facing).normalize_or_zero(),
            drive_w: presence.drive_w,
            emit,
            emit_cone: (presence.emit_spread_rad > 0.0)
                .then(|| lc_world::signal::Beam::along(DVec3::from_array(presence.emit_axis), presence.emit_spread_rad)),
            emitted_s,
            form: presence.form,
            building: presence.building.map(|b| (emitted_s, b)),
            glow: presence.glow.unwrap_or_else(|| lc_world::glow::Glow::unfitted(&lc_world::fitting::Balance::DEFAULT).into()),
            glare: presence.glare,
            reckoning: Reckoning::new(system, sighting, stated.is_thrusting(), receiver),
            stated,
        }
    }

    /// Bring the contact up to what `observer_ly` sees at `now_s`. `drives` is this craft's
    /// drive events, oldest first.
    pub fn reckon(
        &mut self,
        system: Option<&LocalSystem>,
        observer_ly: DVec3,
        now_s: f64,
        drives: &[DriveAt],
    ) {
        let seen = self.reckoning.appearance_at(system, observer_ly, now_s);
        self.position_ly = seen.position_ly;
        self.beta = seen.beta;
        self.emitted_s = seen.emitted_s;
        // The latest word on the drive and emits at the instant drawn, statement or event. The
        // statement stands when nothing has been said since, including when this clock is behind it.
        let stated_s = self.reckoning.seen.emitted_s;
        let latest = drives
            .iter()
            .rev()
            .find(|d| d.at_s <= seen.emitted_s)
            .filter(|d| d.at_s > stated_s || seen.emitted_s < stated_s)
            .copied()
            .unwrap_or(self.stated);
        self.drive_w = latest.drive_w;
        self.emit = latest.emit;
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use lc_world::flight::JULIAN_YEAR_S;
    use lc_world::system::M_PER_LY;

    const ARRIVE_T: i64 = 1_000_000_000;
    const APART_M: f64 = 1.0e9;

    /// A contact a million kilometers off an observer doing half of `c`, and drifting from it.
    fn presence(drive_w: f64, emit_w: (f64, f64)) -> Presence {
        let observer_ly = DVec3::X * 1.0e-9;
        let observer_beta = DVec3::X * 0.5;
        Presence {
            ship_id: ShipId(2),
            length_m: 500.0,
            at_ly: (observer_ly + DVec3::Y * APART_M / M_PER_LY).to_array(),
            beta: (observer_beta + DVec3::Y * 1.0e-4).to_array(),
            facing: [1.0, 0.0, 0.0],
            drive_w,
            emit_fore_w: emit_w.0,
            emit_aft_w: emit_w.1,
            emit_axis: [0.0; 3],
            emit_spread_rad: 0.0,
            emitted_t: ARRIVE_T - 3_335_641,
            arrive_t: ARRIVE_T,
            observer_ly: observer_ly.to_array(),
            observer_beta: observer_beta.to_array(),
            form: lc_proto::Form::default(),
            building: None,
            glow: None,
            glare: None,
        }
    }

    fn drawn_from(own_ly: DVec3, presence: Presence, since_s: f64) -> DVec3 {
        let mut contact = Contact::seen(presence, None);
        contact.reckon(None, own_ly, ARRIVE_T as f64 * 1.0e-6 + since_s, &[]);
        (contact.position_ly - own_ly) * M_PER_LY
    }

    /// Drawn at the stated offset from wherever this ship has itself at, drifting from the arrival.
    #[test]
    fn a_contact_under_thrust_is_drawn_from_this_ship() {
        let own_ly = DVec3::new(3.0e-9, 2.0e-9, 0.0);
        let since_s = 100.0;
        let drift_m = 1.0e-4 * since_s / JULIAN_YEAR_S * M_PER_LY;
        let expected = DVec3::new(0.0, APART_M + drift_m, 0.0);
        for (drive_w, emit_w) in [(1.0e18, (0.0, 0.0)), (0.0, (0.0, 1.0e18))] {
            let off = drawn_from(own_ly, presence(drive_w, emit_w), since_s);
            assert!((off - expected).length() < 1.0, "{off:?}");
        }
    }

    /// A balanced emit pushes nothing, so the contact keeps its own line.
    #[test]
    fn a_balanced_emit_is_coasting() {
        let own_ly = DVec3::new(3.0e-9, 2.0e-9, 0.0);
        let off = drawn_from(own_ly, presence(0.0, (1.0e18, 1.0e18)), 100.0);
        assert!((off.y - APART_M).abs() > 1.0e3, "{off:?}");
    }
}
