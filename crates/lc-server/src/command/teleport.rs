//! `teleport` and `where`: moving a ship by fiat, and finding the ids to move it to.
//!
//! A teleport is the one change to a worldline that is not motion. The craft keeps the stretch
//! it left, in the system it flew it in, marked as ending in a jump — see
//! `lc_world::craft::Craft::teleport` and `lc_spacetime::Worldline::breaks` — so an observer
//! who has not yet received the light of it goes on seeing the craft where it was. What anyone
//! learns *of* the jump arrives as two events, one at each end, each at its own light delay.

use std::fmt::Write as _;
use std::sync::Arc;

use glam::DVec3;
use lc_proto::{ClientId, kind};
use lc_world::craft::CraftId;
use lc_world::knowledge::BodyId;
use lc_world::navigation::{Course, Plane, Target};
use lc_world::sky::generate::AU;
use lc_world::system::LocalSystem;

use super::Bound;
use crate::journal::Journal;
use crate::server::{BURN_POWER_W, Server};
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// Where a teleport goes: a body of a system, by the key the system targets it with.
struct Place {
    system: Arc<LocalSystem>,
    key: String,
    /// What the asker called it, for the answer.
    id: u64,
    what: &'static str,
}

impl<J: Journal> Server<J> {
    pub(super) fn teleport_command(
        &mut self,
        from: ClientId,
        args: &Bound,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<String, String> {
        let ship = match args.id("ship") {
            Some(raw) => i64::try_from(raw)
                .ok()
                .map(CraftId)
                .filter(|id| self.fleet.get(*id).is_some())
                .ok_or_else(|| format!("no ship {raw}"))?,
            None => self.owned_by(from).ok_or("you have no ship to move")?,
        };
        let target = args.id("target").ok_or("a target is required")?;
        let place = self.locate(target, args.id("star"))?;
        self.teleport(ship, place, args.number("altitude").unwrap_or(2.0), wire, events, deliveries)
    }

    /// A star by its catalog id, or a body by its id: in `star`'s system when that is given,
    /// and otherwise in any system already loaded. Loading every star's system to look would be
    /// the whole catalog generated in one tick.
    fn locate(&mut self, id: u64, star: Option<u64>) -> Result<Place, String> {
        if star.is_none()
            && let Some(system) = self.world.system_of(id)
        {
            let key = system.sim().name(system.primary()).to_string();
            return Ok(Place { system, key, id, what: "star" });
        }
        let systems: Vec<Arc<LocalSystem>> = match star {
            Some(star) => vec![self.world.system_of(star).ok_or_else(|| format!("no star {star:#x} here"))?],
            None => self.world.loaded().cloned().collect(),
        };
        systems
            .into_iter()
            .find_map(|system| {
                let key = body_key(&system, id)?;
                Some(Place { system, key, id, what: "body" })
            })
            .ok_or_else(|| match star {
                Some(star) => format!("star {star:#x} has no body {id:#x}"),
                None => format!("nothing here is {id:#x}; for a body nobody has visited, add star:<id>"),
            })
    }

    fn teleport(
        &mut self,
        id: CraftId,
        place: Place,
        altitude_radii: f64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<String, String> {
        let at = self.now_t();
        let now_s = at as f64 * 1.0e-6;
        let craft = self.fleet.get(id).ok_or("no such ship")?;
        let left = craft.position_at(at as f64);
        let course = Course::Orbit { body: place.key, altitude_radii, plane: Plane::Equatorial };
        let waypoint = course
            .resolve(&place.system, craft.motion.position_ly, now_s)
            .ok_or_else(|| format!("{} {:#x} cannot be orbited", place.what, place.id))?;
        let craft = self.fleet.get_mut(id).ok_or("no such ship")?;
        craft.teleport(place.system, waypoint, now_s).ok_or("there is nowhere there to hold")?;
        let landed = craft.position_at(at as f64);
        let name = craft.designation();
        // A standing intercept is a plan to close from where it was.
        self.pursuits.remove(&id);
        // As loud as a burn, so that at a distance neither end is told apart from one by its
        // brightness alone.
        self.emit_from(id, left, kind::VANISH, BURN_POWER_W, "{}".into(), at, events, deliveries);
        self.emit_from(id, landed, kind::APPEAR, BURN_POWER_W, "{}".into(), at, events, deliveries);
        // Somewhere it did not fly to, which no client can reach by folding anything.
        self.tell_flying(wire, id);
        Ok(format!("{name} is holding {altitude_radii} radii above {} {:#x}", place.what, place.id))
    }

    pub(super) fn where_command(&self, from: ClientId, every: bool) -> Result<String, String> {
        let craft = self.owned_by(from).and_then(|id| self.fleet.get(id)).ok_or("you have no ship")?;
        let Some(system) = craft.system.as_ref() else {
            let at: DVec3 = craft.motion.position_ly;
            return Ok(format!("between the stars, at ({:.3}, {:.3}, {:.3}) ly", at.x, at.y, at.z));
        };
        let mut out = format!("star {:#x}", system.star.get());
        let mut skipped = 0;
        for entry in system.inventory() {
            let Target::Body(key) = &entry.target else { continue };
            if entry.depth == 0 {
                continue;
            }
            if !every && !entry.major {
                skipped += 1;
                continue;
            }
            let indent = "  ".repeat(entry.depth);
            let id = BodyId::of(system.star, key).get();
            let _ = write!(out, "\n{indent}{:?} {id:#x}, {:.4} AU", entry.kind, entry.orbit_radius_m / AU);
        }
        if skipped > 0 {
            let _ = write!(out, "\n{skipped} more with show:all");
        }
        Ok(out)
    }
}

/// The key a body is targeted by, from its id. Never shown: a key is the generator's name.
fn body_key(system: &LocalSystem, id: u64) -> Option<String> {
    system.inventory().iter().find_map(|entry| match &entry.target {
        Target::Body(key) if BodyId::of(system.star, key).get() == id => Some(key.clone()),
        _ => None,
    })
}
