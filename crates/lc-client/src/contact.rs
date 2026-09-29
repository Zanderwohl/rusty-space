//! Another craft as this ship sees it: one [`Presence`], reckoned forward between statements.

use glam::DVec3;
use lc_proto::{Presence, ShipId};
use lc_world::sighted::Reckoning;
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
    /// The half-angle they were sent in, radians.
    pub emit_spread_rad: f64,
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
    pub emit_spread_rad: f64,
}

impl DriveAt {
    pub fn of(at_s: f64, change: &lc_proto::DriveChange) -> Self {
        Self {
            at_s,
            drive_w: change.power_w,
            emit: lc_world::emit::Ends { fore_w: change.emit_fore_w, aft_w: change.emit_aft_w },
            emit_spread_rad: change.emit_spread_rad,
        }
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
        let stated = DriveAt { at_s: emitted_s, drive_w: presence.drive_w, emit, emit_spread_rad: presence.emit_spread_rad };
        let sighting = lc_world::pursuit::Sighting {
            target: lc_world::motion::ShipId(presence.ship_id.0),
            position_ly,
            beta,
            length_m: presence.length_m,
            emitted_s,
        };
        Self {
            ship_id: presence.ship_id,
            name: presence.name,
            length_m: presence.length_m,
            position_ly,
            beta,
            facing: DVec3::from_array(presence.facing).normalize_or_zero(),
            drive_w: presence.drive_w,
            emit,
            emit_spread_rad: presence.emit_spread_rad,
            emitted_s,
            form: presence.form,
            building: presence.building.map(|b| (emitted_s, b)),
            glow: presence.glow.unwrap_or_else(|| lc_world::glow::Glow::unfitted(&lc_world::fitting::Balance::DEFAULT).into()),
            glare: presence.glare,
            reckoning: Reckoning::new(system, sighting),
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
        self.emit_spread_rad = latest.emit_spread_rad;
    }
}

