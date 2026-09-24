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
use lc_spacetime::Worldline;
use lc_world::motion::{LIGHT_US_PER_LY, Motive};
use lc_world::navigation::{Course, Plane, Target, Waypoint};
use lc_world::sky::StarId;
use lc_world::sky::generate::AU;
use lc_world::system::{LocalSystem, M_PER_LY};

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

/// Where a jump puts a craft.
enum Landing {
    Holding { system: Arc<LocalSystem>, waypoint: Waypoint },
    Drifting { system: Option<Arc<LocalSystem>>, at_ly: DVec3, beta: DVec3 },
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
        let ship = self.ship_named(from, args)?;
        let (landing, said) = match (args.id("target"), args.id("beside")) {
            (Some(target), None) => {
                let place = self.locate(target, args.id("star"))?;
                self.onto(ship, place, args.number("altitude").unwrap_or(2.0))?
            }
            (None, Some(other)) if args.id("star").is_none() => self.beside(ship, other)?,
            (None, Some(_)) => return Err("star: goes with a target, not with beside:".into()),
            (Some(_), Some(_)) => return Err("a target or beside:, not both".into()),
            (None, None) => return Err("a target or beside: is required".into()),
        };
        let name = self.jump(ship, landing, wire, events, deliveries)?;
        Ok(format!("{name} is {said}"))
    }

    /// A star by its catalog id, or a body by its id: in `star`'s system when that is given,
    /// and otherwise in any system already loaded. Loading every star's system to look would be
    /// the whole catalog generated in one tick.
    fn locate(&mut self, id: u64, star: Option<u64>) -> Result<Place, String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        if star.is_none()
            && let Some(system) = self.world.system_for(StarId::from_raw(id), now_s)
        {
            let key = system.sim().name(system.primary()).to_string();
            return Ok(Place { system, key, id, what: "star" });
        }
        let systems: Vec<Arc<LocalSystem>> = match star {
            Some(star) => vec![
                self.world.system_for(StarId::from_raw(star), now_s).ok_or_else(|| format!("no star {star:#x} here"))?,
            ],
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

    /// An equatorial orbit of `place`, `altitude_radii` above it.
    fn onto(&self, ship: CraftId, place: Place, altitude_radii: f64) -> Result<(Landing, String), String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let from = self.fleet.get(ship).ok_or("no such ship")?.motion.position_ly;
        let course = Course::Orbit { body: place.key, altitude_radii, plane: Plane::Equatorial };
        let waypoint = course
            .resolve(&place.system, from, now_s)
            .ok_or_else(|| format!("{} {:#x} cannot be orbited", place.what, place.id))?;
        let said = format!("holding {altitude_radii} radii above {} {:#x}", place.what, place.id);
        Ok((Landing::Holding { system: place.system, waypoint }, said))
    }

    /// Where an intercept at company would leave `ship` beside `other`, as it is now.
    ///
    /// On the same orbit a standoff ahead when `other` holds one, so the two stay together.
    /// Otherwise the same velocity a standoff to the side, which holds only as long as `other`
    /// does nothing.
    fn beside(&self, ship: CraftId, other: u64) -> Result<(Landing, String), String> {
        let now_s = self.now_t() as f64 * 1.0e-6;
        let quarry = i64::try_from(other)
            .ok()
            .and_then(|id| self.fleet.get(CraftId(id)))
            .ok_or_else(|| format!("no ship {other}"))?;
        if quarry.id == ship {
            return Err("a ship cannot be put beside itself".into());
        }
        let length_m = self.fleet.get(ship).ok_or("no such ship")?.length_m;
        let standoff_m = lc_world::pursuit::standoff_m(length_m, quarry.length_m);
        let said = format!("beside {}", quarry.designation());
        if let (Motive::Holding(Waypoint::Orbit(orbit)), Some(system)) = (&quarry.motion.motive, &quarry.system)
            && orbit.radius_m > 0.0
        {
            let mut ahead = orbit.clone();
            // Arc length into angle.
            ahead.phase_rad += standoff_m / orbit.radius_m;
            return Ok((Landing::Holding { system: system.clone(), waypoint: Waypoint::Orbit(ahead) }, said));
        }
        let line = quarry.worldline();
        let t_us = now_s * 1.0e6;
        let (at_ly, beta) = (line.position_at(t_us) / LIGHT_US_PER_LY, line.velocity_at(t_us));
        // Across the direction of travel, so neither runs into the other.
        let aside = beta.cross(DVec3::Z).try_normalize().or_else(|| beta.cross(DVec3::X).try_normalize()).unwrap_or(DVec3::X);
        let at_ly = at_ly + aside * standoff_m / M_PER_LY;
        Ok((Landing::Drifting { system: quarry.system.clone(), at_ly, beta }, said))
    }

    /// Put `id` down at `landing`, and put both ends of it on the air. Returns its name.
    fn jump(
        &mut self,
        id: CraftId,
        landing: Landing,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Result<String, String> {
        let at = self.now_t();
        let now_s = at as f64 * 1.0e-6;
        let craft = self.fleet.get_mut(id).ok_or("no such ship")?;
        let left = craft.position_at(at as f64);
        match landing {
            Landing::Holding { system, waypoint } => {
                craft.teleport(system, waypoint, now_s).ok_or("there is nowhere there to hold")?;
            }
            Landing::Drifting { system, at_ly, beta } => craft.teleport_drifting(system, at_ly, beta, now_s),
        }
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
        Ok(name)
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
