//! `energize`, `drain`, `refit`, `refit-finish` and `refit-magic`: a ship's energy and form, by fiat.

use lc_world::craft::CraftId;
use lc_world::form::{Form, rules};

use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;

impl<J: Journal> Server<J> {
    /// `None` fills it. Past capacity is dropped, not refused.
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
        let me_j = fitting.balance().module_energy_j();
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

    /// `None`, or more than it holds, empties it.
    pub(super) fn drain(&mut self, id: CraftId, modules: Option<f64>, wire: &mut impl Transport) -> Result<String, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let craft = self.fleet.get_mut(id).ok_or("no such ship")?;
        craft.settle(now_s);
        let fitting = craft.fitting().ok_or_else(|| format!("{} has no storage", craft.designation()))?;
        let me_j = fitting.balance().module_energy_j();
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

    /// Through the order's own checks, so what the console begins the wire could have.
    pub(super) fn refit_command(&mut self, id: CraftId, form: &Form, wire: &mut impl Transport) -> Result<String, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        self.refit(id, &form.into(), now_s).map_err(|why| format!("refused: {why:?}"))?;
        self.tell_fitted(wire, id);
        let craft = self.fleet.get(id).ok_or("no such ship")?;
        let plan = craft.fitting().and_then(|f| f.refit()).ok_or("the round did not begin")?;
        Ok(format!("{}: {} steps, {:.1} days", craft.designation(), plan.steps().len(), plan.duration_s() / 86_400.0))
    }

    /// Placed by the rules, since nothing else would ever check it.
    pub(super) fn refit_magic(&mut self, id: CraftId, form: Form, wire: &mut impl Transport) -> Result<String, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let craft = self.fleet.get_mut(id).ok_or("no such ship")?;
        let name = craft.designation();
        let balance = *craft.fitting().ok_or_else(|| format!("{name} has no form"))?.balance();
        if let Err(faults) = rules::check(&form, &balance) {
            let named: Vec<String> = faults.iter().map(ToString::to_string).collect();
            return Err(format!("{name}: {}", named.join("; ")));
        }
        if !craft.refit_at_once(form, now_s) {
            return Err(format!("{name}: the form does not measure"));
        }
        self.refitting.remove(&id);
        self.tell_fitted(wire, id);
        Ok(format!("{name}: rebuilt, {:.0} m", self.fleet.get(id).map_or(0.0, |c| c.length_m)))
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
