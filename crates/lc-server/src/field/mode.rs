//! Clear, Black and Auto on the authority. See `lightcone/docs/30-the-field.md` §Clear and Black.
//!
//! Scheduled as a collapse is: nothing is kept but the account, whose next Auto switch is a closed
//! form re-solved each tick, so it runs with no client connected and a restart restores it. A
//! completed switch is taken out of the account here and only here, which is what makes each flip
//! exactly one event, stamped when and where it completed and seen by everyone at light delay.

use lc_proto::{FieldMode, Refusal, ShadeChange};
use lc_world::craft::{Craft, CraftId};
use lc_world::field::Mode;
use lc_world::fitting::Setting;

use super::collapse_by;
use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// Set Black and kept there, as tests of anything but the modes want.
#[cfg(test)]
pub(crate) fn hold_black(craft: &mut Craft) {
    let Some(mut fitting) = craft.fitting().cloned() else { return };
    fitting.set_posture(lc_world::fitting::Posture::BLACK);
    craft.fit(Some(fitting));
}

/// When `craft`'s field in Auto next begins a switch by `until_s`, and toward which shade, walking
/// the day-long starlight segments as [`collapse_by`] does.
pub fn auto_by(craft: &Craft, until_s: f64) -> Option<(f64, Mode)> {
    let mut ahead: Option<Craft> = None;
    loop {
        let fitting = ahead.as_ref().unwrap_or(craft).fitting()?;
        let since_s = fitting.since_s();
        let segment_end_s = lc_world::solar::segment_end(since_s);
        if let Some(due) = fitting.auto_s().filter(|&(t, _)| t <= segment_end_s.min(until_s)) {
            return Some(due);
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
    /// Take every switch done by now and begin every Auto switch due by now, in order, stopping at
    /// a collapse that comes first.
    pub(crate) fn keep_field_modes(
        &mut self,
        after_t: i64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let ids: Vec<CraftId> =
            self.fleet.iter().filter(|c| c.ended_s().is_none() && c.fitting().is_some()).map(|c| c.id).collect();
        for id in ids {
            while self.keep_field_mode(id, now_s, after_t, events, deliveries) {
                self.tell_fitted(wire, id);
            }
        }
    }

    /// One step of [`Server::keep_field_modes`]: whether it changed anything.
    fn keep_field_mode(
        &mut self,
        id: CraftId,
        now_s: f64,
        after_t: i64,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> bool {
        let Some(craft) = self.fleet.get(id) else { return false };
        let Some(fitting) = craft.fitting() else { return false };
        if let Some(switch) = fitting.posture().switch {
            if switch.done_s > now_s || collapse_by(craft, switch.done_s).is_some() {
                return false;
            }
            return self.flip(id, now_s, after_t, events, deliveries);
        }
        let Some((at_s, to)) = auto_by(craft, now_s) else { return false };
        if collapse_by(craft, at_s).is_some() {
            return false;
        }
        self.fleet.get_mut(id).is_some_and(|craft| craft.begin_switch(to, at_s).is_ok())
    }

    /// Take a switch done by `by_s` out of `id`'s account and state it. Stamped up, and never into
    /// the tick before, where cursors already stand.
    fn flip(&mut self, id: CraftId, by_s: f64, after_t: i64, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) -> bool {
        let Some(craft) = self.fleet.get_mut(id) else { return false };
        let Some(done_s) = craft.fitting().and_then(|f| f.posture().switch).map(|s| s.done_s).filter(|&t| t <= by_s) else {
            return false;
        };
        let Some(switch) = craft.take_flip(done_s) else { return false };
        let clear_absorptivity = craft.fitting().map_or(0.0, |f| f.balance().clear_absorptivity);
        // What changes is the starlight it reflects.
        let power_w = craft.starlight_w_at(done_s) * (1.0 - clear_absorptivity);
        let payload = serde_json::to_string(&ShadeChange { shade: switch.to.into() }).unwrap_or_else(|_| "{}".into());
        let at_t = ((done_s * 1.0e6).ceil() as i64).clamp(after_t + 1, self.now_t());
        self.emit(id, lc_proto::kind::SHADE, power_w, payload, at_t, events, deliveries);
        true
    }

    /// `Order::FieldMode` at `at`. Refused while a switch runs, and for Auto thresholds without
    /// both gaps open.
    pub(crate) fn order_field_mode(
        &mut self,
        id: CraftId,
        mode: FieldMode,
        at: i64,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<(), Refusal> {
        let setting = Setting::from(mode);
        if let Setting::Auto(thresholds) = setting
            && !thresholds.is_valid()
        {
            return Err(Refusal::Impossible);
        }
        self.fleet.get(id).ok_or(Refusal::NotYours)?.fitting().ok_or(Refusal::Impossible)?;
        let at_s = at as f64 * 1.0e-6;
        // Only when a switch is shorter than a tick: every other is taken before orders are read.
        self.flip(id, at_s, at - 1, events, deliveries);
        let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
        craft.set_field(setting, at_s).map_err(|_| Refusal::Switching)
    }
}
