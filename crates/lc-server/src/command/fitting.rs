//! `energize` and `finish-refit`: a ship's energy and modules, by fiat.

use lc_world::craft::CraftId;

use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;

impl<J: Journal> Server<J> {
    /// Add `modules` ME to `id`'s storage, or fill it when `None`. Anything past capacity is
    /// dropped rather than refused.
    pub(super) fn energize(
        &mut self,
        id: CraftId,
        modules: Option<f64>,
        wire: &mut impl Transport,
    ) -> Result<String, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let craft = self.fleet.get_mut(id).ok_or("no such ship")?;
        // Settled first, so what it holds includes everything collected and spent up to now.
        craft.settle(now_s);
        let fitting = craft.fitting().ok_or_else(|| format!("{} has no storage", craft.designation()))?;
        let me_j = fitting.balance.module_energy_j();
        let capacity_j = fitting.capacity_j_at(now_s);
        let before_j = fitting.stored_j_at(&craft.motion, now_s);
        let room_j = (capacity_j - before_j).max(0.0);
        let added_j = modules.map_or(room_j, |me| (me * me_j).min(room_j));
        craft.grant(added_j, now_s);
        let name = craft.designation();
        self.tell_fitted(wire, id);
        Ok(format!(
            "{name}: +{:.3} ME, {:.3} / {:.3} ME",
            added_j / me_j,
            (before_j + added_j) / me_j,
            capacity_j / me_j,
        ))
    }

    pub(super) fn finish_refit(&mut self, id: CraftId, wire: &mut impl Transport) -> Result<String, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let craft = self.fleet.get_mut(id).ok_or("no such ship")?;
        let name = craft.designation();
        if !craft.finish_refit(now_s) {
            return Err(format!("{name} is not refitting"));
        }
        // Told here rather than by `keep_accounts`, which would find nothing left to finish.
        self.refitting.remove(&id);
        self.tell_fitted(wire, id);
        Ok(format!("{name}: refit finished"))
    }
}
