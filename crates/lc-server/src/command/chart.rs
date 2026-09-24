//! `chart`: a whole system's orbits, handed to a craft as the charting office's claims.

use lc_world::craft::CraftId;
use lc_world::knowledge::observatory::chart_system;
use lc_world::sky::StarId;

use crate::journal::Journal;
use crate::server::Server;

impl<J: Journal> Server<J> {
    /// Chart `star`'s system for `id`, or the system it is in when `None`. What it learns reaches
    /// its client with the next page of knowledge, like anything else it learns.
    pub(super) fn chart(&mut self, id: CraftId, star: Option<u64>) -> Result<String, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let star = match star {
            Some(raw) => StarId::from_raw(raw),
            None => {
                let craft = self.fleet.get(id).ok_or("no such ship")?;
                craft.system.as_ref().ok_or("between the stars there is no system to chart; add star:<id>")?.star
            }
        };
        let system = self.world.system_for(star, now_s).ok_or_else(|| format!("no star {:#x} here", star.get()))?;
        let charted = chart_system(&mut self.aboard(id).knowledge, &system, now_s);
        Ok(format!("charted {charted} bodies of star {:#x}", star.get()))
    }
}
