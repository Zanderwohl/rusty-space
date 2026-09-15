//! A physical craft: something with a worldline, an instrument, and a name.
//!
//! [`motion`](crate::motion) says how a thing moves. This says what a thing *is*, so that the
//! same physics runs for a probe dropped into a ring, a relay parked at L2 and the ship the
//! player is sitting in. None of it was ever specific to the player; it was only ever reached
//! through them.
//!
//! [`Fleet`] is the single source of truth for craft state, in the same sense
//! `em_sim::System` is for body state: anything else that wants to draw or index a craft holds
//! its [`CraftId`] and looks it up. Nothing per-craft lives in two places.
//!
//! A craft owns the answer to "when does my arc leave this sphere", because solving it costs a
//! few hundred boundary evaluations and both the client and the server need exactly one copy.

use std::sync::Arc;

use glam::DVec3;

use crate::instrument::Instrument;
use crate::motion::{self, Event, Flight, Motive, Rejected, ShipState};
use crate::system::LocalSystem;

/// A craft, by the identifier whoever owns it uses. Opaque here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CraftId(pub i64);

/// A hull's beam and height, as fractions of its length.
///
/// Every craft is the same ovoid at a different size: five long by three across by one deep.
/// Shape is not yet a thing a craft can differ in, so it is a constant rather than a field —
/// one that anything drawing or picking a hull reads, so the silhouette, the bounding size and
/// the zoom limits cannot drift apart.
pub const BEAM_PER_LENGTH: f64 = 3.0 / 5.0;
pub const HEIGHT_PER_LENGTH: f64 = 1.0 / 5.0;

/// The span of hull lengths the game is designed around, metres. Nothing enforces it; it is
/// what the camera, the reticle and the point-source crossover are expected to cope with.
pub const LENGTH_RANGE_M: (f64, f64) = (500.0, 50_000.0);

/// What a craft is for.
///
/// It decides the defaults and what the interface offers, never the physics: a probe on a
/// hyperbola and a ship on the same hyperbola are the same arc.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    /// Crewed. The only kind with an acceleration limit set by what a body can stand.
    #[default]
    Ship,
    /// Sent somewhere to look at it and say what it saw.
    Probe,
    /// Parked to pass messages on. Its whole purpose is to be somewhere with a line of sight
    /// to two places that have none to each other.
    Relay,
    /// Put somewhere to be seen. Transmits, never listens.
    Beacon,
}

impl Kind {
    /// What it can pull, and how fast it will go.
    ///
    /// A crew caps acceleration and nothing else does; an uncrewed craft is limited by its
    /// structure, which is a long way above anything a person survives.
    pub fn drive(self) -> crate::flight::Drive {
        match self {
            Kind::Ship => crate::flight::Drive::DEFAULT,
            Kind::Probe => crate::flight::Drive { accel_g: 30.0, max_beta: 0.999 },
            // Neither of these is going anywhere in a hurry once it is placed.
            Kind::Relay | Kind::Beacon => crate::flight::Drive { accel_g: 1.0, max_beta: 0.9 },
        }
    }

    /// How long a hull of this kind is by default, metres. A craft may be given another.
    pub fn length_m(self) -> f64 {
        match self {
            Kind::Ship => 500.0,
            Kind::Probe => 40.0,
            Kind::Relay => 120.0,
            Kind::Beacon => 20.0,
        }
    }

    /// What a craft of this kind carries to look with. A beacon carries nothing.
    pub fn sensor(self) -> Instrument {
        match self {
            Kind::Ship => Instrument::SHIP,
            Kind::Probe => Instrument::PROBE,
            Kind::Relay | Kind::Beacon => Instrument::PROBE,
        }
    }
}

/// One craft: a worldline, an instrument, and who it is.
#[derive(Clone)]
pub struct Craft {
    pub id: CraftId,
    pub kind: Kind,
    /// What a player calls it. `None` until someone does.
    pub name: Option<String>,
    pub motion: ShipState,
    /// How long the hull is, metres. Defaults to the kind's, and is a field rather than a
    /// lookup because two ships of one kind are allowed to be different sizes — see
    /// [`LENGTH_RANGE_M`], which is the span the camera and the reticle are built for.
    pub length_m: f64,
    /// The system its motive is defined against, if it is in one.
    ///
    /// Shared and never mutated: every motive is evaluated at the time asked for rather than
    /// read out of a propagated arena, so nothing has to advance a system and every craft in
    /// one can point at the same copy.
    pub system: Option<Arc<LocalSystem>>,
    pub sensor: Instrument,
    /// Below this, an arrival is not a detection.
    pub noise_floor: f32,
    /// The patch the current arc is heading for, solved once when the arc began.
    ///
    /// Private because it is derived: the arc is the fact and this is an answer about it, and
    /// an answer that can be set from outside is an answer that can be about a different arc.
    patch: Option<Event>,
}

impl Craft {
    /// A craft of `kind`, at rest at a point, light-years from the world origin.
    pub fn at(id: CraftId, kind: Kind, position_ly: DVec3) -> Self {
        let mut motion = ShipState::at(position_ly);
        motion.drive = kind.drive();
        Self {
            id,
            kind,
            name: None,
            motion,
            length_m: kind.length_m(),
            system: None,
            sensor: kind.sensor(),
            noise_floor: 0.0,
            patch: None,
        }
    }

    /// What to call it on screen: its name, or its kind and number.
    pub fn designation(&self) -> String {
        match &self.name {
            Some(name) => name.clone(),
            None => format!("{:?} {}", self.kind, self.id.0),
        }
    }

    /// The craft as something a light-delay solve can evaluate.
    pub fn worldline(&self) -> Flight<'_> {
        Flight::new(&self.motion, self.system.as_deref())
    }

    /// Which way the nose points at a coordinate second, or `None` when nothing decides it.
    pub fn facing_at(&self, now_s: f64) -> Option<DVec3> {
        motion::facing(&self.motion, self.system.as_deref(), now_s)
    }

    /// Where it is at a coordinate microsecond, light-microseconds from the world origin.
    pub fn position_at(&self, t_us: f64) -> DVec3 {
        use lc_spacetime::Worldline;
        self.worldline().position_at(t_us)
    }

    /// Put it on an approach, and drop whatever the old motive had predicted.
    ///
    /// Not an [`Event`], because an approach is not an order a client sends: it is what the
    /// authority works out *from* a standing order, once per re-solve, against a sighting only
    /// it can vouch for. The client receives the answer as a motive and folds it.
    pub fn begin_rendezvous(&mut self, plan: crate::pursuit::Rendezvous, now_s: f64) {
        self.motion.begin_rendezvous(plan);
        self.solve_patch(now_s);
    }

    /// Fold an event, and re-solve the patch if the arc changed.
    pub fn apply(&mut self, event: &Event) -> Result<(), Rejected> {
        motion::apply(&mut self.motion, self.system.as_deref(), event)?;
        self.solve_patch(event.at_t);
        Ok(())
    }

    /// Put it in a system, or take it out of one. Anything defined against the old system's
    /// bodies goes; a crossing does not, because leaving is what one is for.
    pub fn enter(&mut self, system: Option<Arc<LocalSystem>>, now_s: f64) {
        // Only on the way *out*. A craft that had no system has nothing defined against one,
        // so entering must leave its motive alone -- dropping it there cancelled a station the
        // caller had just set for the system it was being put into.
        let left = match (&self.system, &system) {
            (None, _) => false,
            (Some(old), Some(new)) => !Arc::ptr_eq(old, new),
            (Some(_), None) => true,
        };
        self.system = system;
        if left {
            self.motion.leave_system(now_s);
        }
        self.solve_patch(now_s);
    }

    /// Move to a coordinate time, folding the patch its arc was solved for if that time has
    /// come.
    ///
    /// The patch is folded *before* the step rather than after, so it is always stamped with
    /// its own solved coordinate whatever step happened to run past it. That is what lets a
    /// server at 438 seconds and a client at 61 reach the same arc.
    pub fn advance(&mut self, now_s: f64, elapsed_s: f64) {
        self.patch_if_due(now_s);
        let was = matches!(self.motion.motive, Motive::Falling(_));
        motion::advance(&mut self.motion, self.system.as_deref(), now_s, elapsed_s);
        // A crossing that arrived, or an arc that lost its system: either way the answer the
        // patch held is about a motive the craft is no longer on.
        if was != matches!(self.motion.motive, Motive::Falling(_)) {
            self.solve_patch(now_s);
        }
    }

    /// When the current arc leaves the sphere it was solved in, if it does.
    pub fn patch_due_at(&self) -> Option<f64> {
        self.patch.as_ref().map(|event| event.at_t)
    }

    /// Solve for the next patch, for whoever is authoritative over this craft.
    fn solve_patch(&mut self, from_s: f64) {
        self.patch = match (&self.motion.motive, self.system.as_deref()) {
            (Motive::Falling(_), Some(system)) => {
                motion::repatch_at(&self.motion, system, from_s)
            }
            // A prediction is about one conic. Anything else -- a course set, a station held,
            // a shell crossed -- makes it an answer to a question nobody is asking, and
            // folding it would cut the drive on a flight that is under way.
            _ => None,
        };
    }

    fn patch_if_due(&mut self, now_s: f64) {
        let Some(system) = self.system.clone() else {
            self.patch = None;
            return;
        };
        if !matches!(self.motion.motive, Motive::Falling(_)) {
            self.patch = None;
            return;
        }
        let event = match self.patch.take_if(|e| e.at_t <= now_s) {
            Some(solved) => Some(solved),
            // The backstop: a craft that ended up in a sphere the prediction did not cover,
            // which a course change or a system reload can do.
            None => motion::repatch_due(&self.motion, &system, now_s),
        };
        if let Some(event) = event {
            let _ = motion::apply(&mut self.motion, Some(&system), &event);
            self.solve_patch(event.at_t);
        }
    }
}

impl std::fmt::Debug for Craft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Craft")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("motion", &self.motion)
            .field("in_a_system", &self.system.is_some())
            .finish()
    }
}

/// Every craft there is. The single source of truth for craft state.
#[derive(Clone, Debug, Default)]
pub struct Fleet {
    craft: Vec<Craft>,
}

impl Fleet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a craft, replacing any with the same identifier.
    pub fn insert(&mut self, craft: Craft) {
        match self.craft.iter_mut().find(|c| c.id == craft.id) {
            Some(existing) => *existing = craft,
            None => self.craft.push(craft),
        }
    }

    pub fn remove(&mut self, id: CraftId) -> Option<Craft> {
        let at = self.craft.iter().position(|c| c.id == id)?;
        Some(self.craft.remove(at))
    }

    pub fn get(&self, id: CraftId) -> Option<&Craft> {
        self.craft.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: CraftId) -> Option<&mut Craft> {
        self.craft.iter_mut().find(|c| c.id == id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Craft> {
        self.craft.iter()
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Craft> {
        self.craft.iter_mut()
    }

    pub fn len(&self) -> usize {
        self.craft.len()
    }

    pub fn is_empty(&self) -> bool {
        self.craft.is_empty()
    }

    /// Route an event to the craft it happened to.
    ///
    /// `None` when there is no such craft, which is not an error: a client can be told about
    /// something that happened to a craft it has never heard of.
    pub fn apply(&mut self, id: CraftId, event: &Event) -> Option<Result<(), Rejected>> {
        Some(self.get_mut(id)?.apply(event))
    }

    /// Move every craft to a coordinate time.
    pub fn advance(&mut self, now_s: f64, elapsed_s: f64) {
        for craft in &mut self.craft {
            craft.advance(now_s, elapsed_s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{Change, ShipId};
    use crate::navigation::{Course, Plane};
    use crate::sky::{CatalogueStar, StarProvider};

    fn sol() -> Option<Arc<LocalSystem>> {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .ok()?;
        let sun: CatalogueStar =
            provider.stars().iter().find(|s| s.name.as_deref() == Some("Sol"))?.clone();
        let mut system = LocalSystem::for_star(&sun)?;
        system.advance_to(0.0);
        Some(Arc::new(system))
    }

    fn escaping(system: &LocalSystem) -> (DVec3, DVec3) {
        let earth = system.body_named("Earth").expect("Earth");
        let (at_m, carried) = system.body_state_at(earth, 0.0).expect("a state");
        let radius = system.sim().radius(earth) * 3.0;
        let mu = system.sim().gravitational_constant() * system.sim().mass(earth);
        (
            system.origin_ly + (at_m + DVec3::X * radius) / crate::system::M_PER_LY,
            carried + DVec3::Y * (mu / radius).sqrt() * 1.6,
        )
    }

    /// The whole point: the physics does not know or care which craft it is running for.
    /// A probe and a ship given the same course fly the same arc.
    #[test]
    fn a_probe_and_a_ship_fly_the_same_arc() {
        let Some(system) = sol() else { return };
        let course = Change::SetCourse {
            course: Course::Orbit {
                body: "Earth".into(),
                altitude_radii: 2.0,
                plane: Plane::Equatorial,
            },
            // The same drive, because the *kind* changes what a craft can pull and the
            // physics changes nothing.
            drive: Kind::Ship.drive(),
        };

        let mut fleet = Fleet::new();
        for (id, kind) in [(1, Kind::Ship), (2, Kind::Probe)] {
            let mut craft = Craft::at(CraftId(id), kind, DVec3::ZERO);
            craft.enter(Some(system.clone()), 0.0);
            fleet.insert(craft);
        }
        for id in [1, 2] {
            let event = Event { ship: ShipId(id), at_t: 0.0, change: course.clone() };
            fleet.apply(CraftId(id), &event).expect("a craft").expect("a course");
        }
        fleet.advance(30_000.0, 30_000.0);

        let ship = fleet.get(CraftId(1)).expect("the ship");
        let probe = fleet.get(CraftId(2)).expect("the probe");
        assert_eq!(ship.motion.position_ly, probe.motion.position_ly);
        assert_eq!(ship.motion.motive, probe.motion.motive);
        // And they are still different things.
        assert_ne!(ship.kind, probe.kind);
        assert!(probe.sensor.aperture_m2 < ship.sensor.aperture_m2);
        assert!(probe.kind.drive().accel_g > ship.kind.drive().accel_g);
    }

    /// A craft owns its own patch, so a fleet of them all get one without anyone outside
    /// keeping a table of arcs.
    #[test]
    fn every_craft_solves_its_own_patch() {
        let Some(system) = sol() else { return };
        let (at, velocity) = escaping(&system);

        let mut fleet = Fleet::new();
        for id in 1..=3 {
            let mut craft = Craft::at(CraftId(id), Kind::Probe, at);
            craft.enter(Some(system.clone()), 0.0);
            // Before the cut: a craft at rest is falling straight down the line to its
            // primary, which has no elements, and cutting there leaves it drifting.
            craft.motion.beta = crate::coast::beta_of(velocity);
            craft
                .apply(&Event { ship: ShipId(id), at_t: 0.0, change: Change::CutDrive })
                .expect("the engine cuts");
            fleet.insert(craft);
        }

        let due: Vec<Option<f64>> = fleet.iter().map(|c| c.patch_due_at()).collect();
        assert!(due.iter().all(|d| d.is_some()), "{due:?}");
        assert!(due.windows(2).all(|w| w[0] == w[1]), "the same arc gave different answers");

        // Run past it: every one of them changes primary, and none needed telling.
        let patch = due[0].expect("a patch");
        fleet.advance(patch + 3_600.0, patch + 3_600.0);
        for craft in fleet.iter() {
            let Motive::Falling(arc) = &craft.motion.motive else {
                panic!("{:?}", craft.motion.motive)
            };
            assert_ne!(arc.primary, "Earth", "craft {:?} never left", craft.id);
        }
    }

    /// Leaving a system drops what was defined against it, and the patch with it.
    #[test]
    fn leaving_a_system_drops_the_arc_and_its_patch() {
        let Some(system) = sol() else { return };
        let (at, velocity) = escaping(&system);
        let mut craft = Craft::at(CraftId(1), Kind::Relay, at);
        craft.enter(Some(system.clone()), 0.0);
        craft.motion.beta = crate::coast::beta_of(velocity);
        craft.apply(&Event { ship: ShipId(1), at_t: 0.0, change: Change::CutDrive }).unwrap();
        assert!(craft.patch_due_at().is_some());

        craft.enter(None, 0.0);
        assert!(craft.patch_due_at().is_none(), "a patch about a system that is not there");
        assert!(matches!(craft.motion.motive, Motive::Drifting { .. }), "{:?}", craft.motion.motive);
    }

    /// A fleet is a set: one entry per identifier, and inserting the same one twice replaces
    /// rather than duplicates.
    #[test]
    fn a_fleet_holds_one_of_each() {
        let mut fleet = Fleet::new();
        fleet.insert(Craft::at(CraftId(1), Kind::Ship, DVec3::ZERO));
        fleet.insert(Craft::at(CraftId(1), Kind::Beacon, DVec3::X));
        assert_eq!(fleet.len(), 1);
        assert_eq!(fleet.get(CraftId(1)).unwrap().kind, Kind::Beacon);
        assert!(fleet.get(CraftId(2)).is_none());
        assert!(fleet.apply(CraftId(2), &Event {
            ship: ShipId(2),
            at_t: 0.0,
            change: Change::CutDrive,
        })
        .is_none(), "an event for a craft nobody has heard of is not an error");
        assert_eq!(fleet.remove(CraftId(1)).map(|c| c.kind), Some(Kind::Beacon));
        assert!(fleet.is_empty());
    }

    /// A craft says what it is called, whether or not anyone has named it.
    #[test]
    fn an_unnamed_craft_still_has_a_designation() {
        let mut craft = Craft::at(CraftId(7), Kind::Probe, DVec3::ZERO);
        assert_eq!(craft.designation(), "Probe 7");
        craft.name = Some("Huygens".into());
        assert_eq!(craft.designation(), "Huygens");
    }
}
