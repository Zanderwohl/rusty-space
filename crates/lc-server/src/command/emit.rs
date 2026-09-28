//! `emit`: light a ship's engines through the order's own checks, so what the console lights the
//! wire could have. 31 §Emitting on purpose.

use lc_proto::{Aim, Apertures, Lead, Order};
use lc_world::craft::CraftId;
use lc_world::form::capacity::ends;

use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;

const WAVELENGTH_M: f64 = 1.0e-6;
const SPREAD_RAD: f64 = 0.01;

impl<J: Journal> Server<J> {
    /// Aft sends the beam out of the stern and burns along the nose, fore out of the bow and
    /// against it, and both along the nose from the bow and back from the stern.
    pub(super) fn emit_command(
        &mut self,
        id: CraftId,
        which: &str,
        rating: f64,
        minutes: f64,
        wire: &mut impl Transport,
    ) -> Result<String, String> {
        let now_t = self.now_t();
        let craft = self.fleet.get(id).ok_or("no such ship")?;
        let name = craft.designation();
        let fitting = craft.fitting().ok_or_else(|| format!("{name} has no engines"))?;
        let (fore, aft) = ends(fitting.form(), fitting.balance()).ok_or_else(|| format!("{name}: the form does not place"))?;
        let nose = craft.facing_at(now_t as f64 * 1.0e-6).unwrap_or(glam::DVec3::X);
        let (apertures, rated_w, axis) = match which {
            "fore" => (Apertures::Fore, fore.rating_w, nose),
            "aft" => (Apertures::Aft, aft.rating_w, -nose),
            _ => (Apertures::Both, fore.rating_w.min(aft.rating_w), nose),
        };
        let order = Order::Emit {
            aim: Aim::Bearing(axis.to_array()),
            apertures,
            power_w: rating * rated_w,
            wavelength_m: WAVELENGTH_M,
            spread_rad: SPREAD_RAD,
            duration_s: minutes * 60.0,
            lead: Lead::Coasting,
        };
        self.order_emit(id, &order, now_t).map_err(|why| format!("refused: {why:?}"))?;
        self.tell_flying(wire, id);
        self.tell_fitted(wire, id);
        Ok(format!("{name}: {which} at {:.2e} W for {minutes} min", rating * rated_w))
    }
}
