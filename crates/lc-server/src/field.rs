//! Collapse: a field that reaches `Q_max` destroys its ship. See `lightcone/docs/30-the-field.md`
//! §Collapse.
//!
//! Nothing is scheduled ahead or kept for it. The instant is a closed form of the account, which
//! every change of input settles first and a checkpoint keeps, so solving it again each tick is the
//! schedule, and a restart restores it with the account. What goes out is an ordinary event, and
//! observers learn of it at their own light delay; the wreck stays in the fleet with its worldline
//! ended, so its light already in flight goes on arriving until the last of it has passed.

use lc_proto::{Outbound, ShipId};
use lc_world::craft::{Craft, CraftId};

use crate::journal::Journal;
use crate::server::{KIND_COLLAPSE, Server};
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// 30 takes the spike as a second long.
const SPIKE_S: f64 = 1.0;

/// When `craft`'s field reaches `Q_max` by `until_s`, if it does, walking the account through the
/// day-long starlight segments it will be settled at.
pub fn collapse_by(craft: &Craft, until_s: f64) -> Option<f64> {
    let mut ahead: Option<Craft> = None;
    loop {
        let fitting = ahead.as_ref().unwrap_or(craft).fitting()?;
        let since_s = fitting.since_s();
        let segment_end_s = lc_world::solar::segment_end(since_s);
        if let Some(at_s) = fitting.collapse_s().filter(|&t| t <= segment_end_s.min(until_s)) {
            return Some(at_s);
        }
        if segment_end_s >= until_s {
            return None;
        }
        let next = ahead.get_or_insert_with(|| craft.clone());
        next.settle(segment_end_s);
        if next.fitting().is_none_or(|f| f.since_s() <= since_s) {
            return None;
        }
    }
}

impl<J: Journal> Server<J> {
    /// Destroy every craft whose field has reached `Q_max` by now, at the instant it did.
    pub(crate) fn collapse_fields(
        &mut self,
        after_t: i64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let now_s = self.now_t as f64 * 1.0e-6;
        let due: Vec<(CraftId, f64)> =
            self.fleet.iter().filter_map(|craft| Some((craft.id, collapse_by(craft, now_s)?))).collect();
        for (id, at_s) in due {
            // Up, so the field is at its limit by the stamp, and never into the tick before, where
            // cursors already stand: only an input changed at a past instant asks for that.
            let at_t = ((at_s * 1.0e6).ceil() as i64).clamp(after_t + 1, self.now_t);
            self.collapse(id, at_t, wire, events, deliveries);
        }
    }

    fn collapse(
        &mut self,
        id: CraftId,
        at_t: i64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let at_s = at_t as f64 * 1.0e-6;
        let Some(craft) = self.fleet.get_mut(id) else { return };
        craft.settle(at_s);
        let Some(fitting) = craft.fitting() else { return };
        let released_j = fitting.field().released_j(fitting.stored_j_at(&craft.motion, at_s));
        let spike_w = fitting.balance().collapse_spike_fraction * released_j / SPIKE_S;
        let name = craft.name.clone();
        craft.end(at_s);
        let payload = serde_json::to_string(&lc_proto::Released { released_j }).unwrap_or_else(|_| "{}".into());
        self.emit(id, KIND_COLLAPSE, spike_w, payload, at_t, events, deliveries);

        self.pursuits.remove(&id);
        self.refitting.remove(&id);
        self.reserved.remove(&id);
        self.instruments.aboard.remove(&id);
        self.auto_ack.remove(&id);
        self.owed.remove(&id);
        self.answered.remove(&id);
        self.destroyed.push(id.0);

        let account = self.by_account.iter().find(|(_, ship)| ship.0 == id.0).map(|(account, _)| account.clone());
        let owner = self.owners.remove(&id);
        if account.is_none() && owner.is_none() {
            return;
        }
        let successor = self.spawn(name.clone());
        if let Some(account) = account {
            self.by_account.insert(account, successor);
        }
        let Some(from) = owner else { return };
        let Some(state) = self.clients.get_mut(&from) else { return };
        let account = state.account.clone();
        *state = crate::server::Connected::new(successor, account.clone(), state.permission);
        self.owners.insert(CraftId(successor.0), from);
        wire.send(from, Outbound::Collapsed { ship_id: ShipId(id.0), at_t, released_j, successor });
        let name = name.unwrap_or_else(|| self.ship(successor).map(Craft::designation).unwrap_or_default());
        self.welcome(from, successor, name, &account, wire);
    }

    /// Drop each wreck once the light of its end has passed every craft there is.
    pub(crate) fn sweep_wrecks(&mut self) {
        let now_t = self.now_t as f64;
        let ended: Vec<(CraftId, f64)> =
            self.fleet.iter().filter_map(|craft| Some((craft.id, craft.ended_s()? * 1.0e6))).collect();
        for (id, end_t) in ended {
            let Some(at) = self.fleet.get(id).map(|wreck| wreck.position_at(end_t)) else { continue };
            let passed = self.fleet.iter().filter(|craft| craft.ended_s().is_none()).all(|craft| {
                lc_spacetime::arrival_time_at(end_t, at, &craft.worldline()).is_none_or(|t| t <= now_t)
            });
            if passed {
                self.fleet.remove(id);
            }
        }
    }

    /// Craft destroyed since this was last called, whose saved rows are to be deleted.
    pub fn take_destroyed(&mut self) -> Vec<i64> {
        std::mem::take(&mut self.destroyed)
    }

    /// Hand back what [`Server::take_destroyed`] took, because deleting them failed.
    pub fn untake_destroyed(&mut self, ids: Vec<i64>) {
        self.destroyed.extend(ids);
    }
}
