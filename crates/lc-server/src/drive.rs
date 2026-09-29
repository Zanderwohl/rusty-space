//! Every craft's drive transitions, stated as events.
//!
//! Found after the fact, once a tick, over the tick just finished — see [`lc_world::ignition`]
//! for why that and not a schedule of the plan's future. Each is stamped at the instant it
//! happened, so it is delivered at light delay from there like anything else, and the client's
//! cursor still has it in range: a tick's deliveries are read from the previous tick onward.

use lc_proto::DriveChange;
use lc_world::craft::CraftId;

use crate::journal::Journal;
use crate::server::{KIND_DRIVE, Server};
use crate::world::{Event, Scheduled};

impl<J: Journal> Server<J> {
    /// State every transition in `(after_t, now]`. After every order and re-solve of the tick,
    /// so what is stated is what was flown.
    pub(crate) fn announce_drives(
        &mut self,
        after_t: i64,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) {
        let (after_s, now_s) = (after_t as f64 * 1.0e-6, self.now_t() as f64 * 1.0e-6);
        let balance = &self.balance;
        let found: Vec<(CraftId, lc_world::ignition::Transition)> = self
            .fleet
            .iter()
            .flat_map(|craft| {
                // A wreck's plan goes on past its end, and nothing of it happened.
                let until_s = craft.ended_s().map_or(now_s, |end_s| end_s.min(now_s));
                lc_world::ignition::transitions(craft, balance, after_s, until_s)
                    .into_iter()
                    // The main drive and the emits are stated; the thrusters are E4's instants to look at.
                    .filter(|transition| transition.drive_stepped() || transition.emit_stepped())
                    .map(move |transition| (craft.id, transition))
            })
            .collect();
        for (id, transition) in found {
            let change = DriveChange {
                power_w: transition.power_w,
                facing: transition.facing.to_array(),
                emit_fore_w: transition.emit.fore_w,
                emit_aft_w: transition.emit.aft_w,
                emit_spread_rad: transition.emit_spread_rad,
            };
            let payload = serde_json::to_string(&change).unwrap_or_else(|_| "{}".into());
            // A plume going out is as visible as the plume was, so a cut carries what it cut.
            let total = |drive_w: f64, emit: lc_world::emit::Ends| drive_w + emit.fore_w + emit.aft_w;
            let power_w = total(transition.power_w, transition.emit).max(total(transition.was_w, transition.was_emit));
            // Rounded up, never into the tick before: that is where the cursor already stands.
            let at_t = ((transition.at_s * 1.0e6).ceil() as i64).clamp(after_t + 1, self.now_t());
            self.emit(id, KIND_DRIVE, power_w, payload, at_t, events, deliveries);
        }
    }
}
