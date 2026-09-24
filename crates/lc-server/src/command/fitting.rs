//! `energize`, `drain`, `refit-finish` and `refit-magic`: a ship's energy and modules, by fiat.

use lc_world::craft::CraftId;
use lc_world::fitting::Loadout;

use super::Bound;

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

    /// Take `modules` ME out of `id`'s storage, or empty it when `None`. Asking for more than it
    /// holds empties it rather than being refused.
    pub(super) fn drain(&mut self, id: CraftId, modules: Option<f64>, wire: &mut impl Transport) -> Result<String, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let craft = self.fleet.get_mut(id).ok_or("no such ship")?;
        craft.settle(now_s);
        let fitting = craft.fitting().ok_or_else(|| format!("{} has no storage", craft.designation()))?;
        let me_j = fitting.balance.module_energy_j();
        let capacity_j = fitting.capacity_j_at(now_s);
        let before_j = fitting.stored_j_at(&craft.motion, now_s);
        let taken_j = modules.map_or(before_j, |me| (me * me_j).min(before_j));
        craft.drain(taken_j, now_s);
        let name = craft.designation();
        self.tell_fitted(wire, id);
        Ok(format!(
            "{name}: -{:.3} ME, {:.3} / {:.3} ME",
            taken_j / me_j,
            (before_j - taken_j) / me_j,
            capacity_j / me_j,
        ))
    }

    /// Rebuild `id` to the loadout `args` name, each count defaulting to what it has now. Energy
    /// is neither asked for nor spent; only the modules have to fit the hull.
    pub(super) fn refit_magic(&mut self, id: CraftId, args: &Bound, wire: &mut impl Transport) -> Result<String, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let craft = self.fleet.get_mut(id).ok_or("no such ship")?;
        let name = craft.designation();
        let now = craft.fitting().ok_or_else(|| format!("{name} has no modules"))?.loadout_at(now_s);
        let count = |arg: &str, was: u32| args.count(arg).unwrap_or(was);
        let target = Loadout {
            storage: count("storage", now.storage),
            drones: count("drones", now.drones),
            living: count("living", now.living),
            engines: count("engines", now.engines),
            data: count("data", now.data),
            slots: count("slots", now.slots),
        };
        if target.slots == 0 || target.modules() > target.slots {
            return Err(format!("{} modules do not fit in {} slots", target.modules(), target.slots));
        }
        craft.refit_at_once(target, now_s);
        self.refitting.remove(&id);
        self.tell_fitted(wire, id);
        let Loadout { storage, drones, living, engines, data, slots } = target;
        Ok(format!(
            "{name}: storage {storage}, drones {drones}, living {living}, engines {engines}, data {data}, {slots} slots"
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
