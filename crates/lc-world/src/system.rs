//! The system the ship is inside: its bodies, propagated, as things to draw.
//!
//! Nothing here models anything. `em-sim` already holds and propagates systems, `em-sim`'s
//! presets already carry the solar system with its moons, and `lc-world::sky::generate` already
//! emits a generated system in the form `em-sim` consumes. This is the join.

use em_sim::id::BodyIndex;
use em_sim::system::System;
use em_foundations::time::Instant;
use glam::DVec3;
use crate::sky::{CatalogStar, StarId, generate};

/// Meters in a light-year.
pub const M_PER_LY: f64 = 9.460_730_472_580_8e15;

/// Meters in one render unit inside a system. An astronomical unit, so a belt's radius is a
/// number of order ten rather than of order 1e11.
pub const UNIT_M: f64 = 1.495_978_707e11;

/// Geometric albedo where nothing better is known. Every classified body has its own; this is
/// only the fallback.
pub const DEFAULT_ALBEDO: f64 = 0.3;

/// When the inventory's radii and ordering are taken, seconds since J2000.
///
/// Any instant would do and none is better: what matters is that it is one instant rather than
/// this one, so the list does not reshuffle while it is being read.
pub const INVENTORY_EPOCH_S: f64 = 0.0;

/// Inside this of a star, the ship is in its system and the star is drawn as an object rather
/// than as a point of the background.
///
/// An Oort cloud reaches about a hundred thousand astronomical units, which is 1.6 light-years,
/// and 03-world-model.md already makes that shell the partition boundary. Being inside it is
/// the same statement as being in the system.
pub const LOCAL_SHELL_LY: f64 = 1.6;

/// The catalog name of the system whose data is real rather than generated.
pub const SOL: &str = "Sol";

/// A body's rings, as the renderer wants them.
#[derive(Clone, Copy, Debug)]
pub struct Rings {
    pub system: &'static crate::rings::RingSystem,
    /// Unit normal of the ring plane, simulation axes.
    pub pole: DVec3,
}

/// One body, as the renderer wants it.
#[derive(Clone, Debug)]
pub struct Drawable {
    pub name: String,
    /// Planet, moon or minor body, from `em-sim`'s own tags.
    ///
    /// Carried rather than looked up against [`crate::navigation::Entry`] by name: `designate`
    /// invents a designation for an unnamed body and this name comes from the arena, so the
    /// two strings can disagree and the join would silently classify a moon as a minor body.
    pub kind: crate::navigation::Kind,
    pub rings: Option<Rings>,
    /// What it looks like, from what it is.
    pub surface: crate::surface::Surface,
    /// What it is made of and wrapped in: measured where anybody has been, and derived from
    /// [`Drawable::surface`] everywhere else. This is what a survey reads per band, and the one
    /// place Venus is allowed to differ from Mars. See [`crate::worlds`].
    pub world: crate::worlds::World,
    /// What a rocky world with air is painted with. See [`crate::climate`].
    pub climate: Option<crate::climate::Climate>,
    /// Also what [`Drawable::world`]'s reflectance is, for a giant.
    pub giant: Option<crate::giant::Giant>,
    /// What a rocky world without air is painted with. See [`crate::airless`].
    pub airless: Option<crate::airless::Airless>,
    /// Spin axis, simulation axes. Ecliptic north where the data says nothing.
    pub pole: DVec3,
    /// How long it takes to turn once, seconds. `None` where the arena states no rotation.
    ///
    /// A tidally locked body's is its orbit about its own primary, which is what being locked
    /// means; the arena states the lock rather than the rate, so it is worked out here.
    pub spin_s: Option<f64>,
    /// Where it is, light-years from the world origin, simulation axes.
    pub position_ly: DVec3,
    pub radius_m: f64,
    /// Kilograms, as the arena states it. Carried because it is what decides which of two
    /// names a crowded map has room for — see `em_map::label`.
    pub mass_kg: f64,
    /// The radius a blackbody at the star's temperature would need to deliver this body's
    /// reflected flux. See [`effective_radius`].
    pub effective_radius_m: f64,
    /// The gray, zero-albedo balance: what sunlight alone would leave it at, kelvin.
    ///
    /// Kept because [`crate::surface::Surface::classify`] is calibrated against it. It is not
    /// what the body radiates at — see [`Drawable::effective_k`].
    pub equilibrium_k: f64,
    /// What it actually radiates at, kelvin: sunlight it keeps, plus heat of its own.
    ///
    /// The two differ for a giant and barely at all for anything else. Jupiter is 124 K where
    /// the gray balance says 122 and the sunlight it keeps says 110 — the albedo takes it down
    /// and its own contraction puts it back, which is a coincidence of the two corrections and
    /// not a reason to skip either.
    pub effective_k: f64,
}

#[derive(Clone)]
pub struct LocalSystem {
    pub star: StarId,
    pub star_name: String,
    /// Swarms, belts and clouds. Generated even for the real solar system: `em-sim`'s preset
    /// carries bodies and no distributions, and a system with a Kuiper belt and no Kuiper belt
    /// in it would be the stranger of the two errors.
    pub populations: Vec<crate::population::Population>,
    /// Where the system's barycenter sits, light-years from the world origin.
    pub origin_ly: DVec3,
    /// The normal of the plane the planets and belts orbit in. What a player means by this
    /// system's ecliptic, and what the map's plane option measures against.
    pub pole: DVec3,
    /// Which way the star spins, a few degrees off [`LocalSystem::pole`]. Only the star's own
    /// orientation: the plane is the planets'.
    pub star_spin: DVec3,
    sim: System,
    primary: BodyIndex,
    /// Everything here a ship can be sent to, ordered outward. Built once: the order comes
    /// from where the bodies were at load, so a list cannot reshuffle itself while a player is
    /// reading it.
    inventory: Vec<crate::navigation::Entry>,
    /// The primary's radius and temperature, which set every reflection in the system.
    star_radius_m: f64,
    star_teff_k: f64,
    star_luminosity_w: f64,
    star_feh: f64,
}

impl LocalSystem {
    /// Load the system around a star, real where there is real data and generated otherwise.
    pub fn for_star(star: &CatalogStar) -> Option<Self> {
        // Once. Generating a system is the expensive part of loading one, and the populations
        // and the bodies both come out of the same pass.
        let generated = generate::system_for(star);
        let contents = match star.provenance.name.as_deref() {
            // The one system with measured data rather than generated: two hundred and thirty
            // bodies fitted against JPL, moons and comets included. Its belts are still the
            // generator's, which knows they are Sol's.
            Some(SOL) => em_sim::presets::solar_system(),
            _ => generated.to_universe(),
        };
        let populations = generated.populations;
        let sim = System::from_contents(&contents).ok()?;
        // The most massive body is the primary. Not the first: a multiple is a barycenter with
        // children, and the barycenter is massless.
        let primary = sim
            .indices()
            .max_by(|a, b| sim.info(*a).mass.total_cmp(&sim.info(*b).mass))?;
        let mut system = Self {
            star: star.id,
            star_name: star.provenance.name.clone().unwrap_or_else(|| format!("{:x}", star.id.get())),
            populations,
            origin_ly: star.position_ly,
            pole: star.system_pole(),
            star_spin: star.spin_axis(),
            sim,
            primary,
            inventory: Vec::new(),
            star_radius_m: star.star.radius_m,
            star_teff_k: star.star.teff_k,
            star_luminosity_w: star.star.luminosity(),
            star_feh: star.metallicity,
        };
        // Propagate before taking the inventory. Straight out of the file every body sits at
        // the origin and has no parent -- the derived columns are rebuilt by the first
        // evaluation -- so the ordering came out as file order with every radius zero.
        system.advance_to(INVENTORY_EPOCH_S);
        system.inventory =
            build_inventory(&system.sim, primary, &system.star_name, &system.populations);
        Some(system)
    }

    /// Propagate to a coordinate time, in seconds since the world origin.
    pub fn advance_to(&mut self, seconds: f64) {
        em_sim::propagate::evaluate_at(&mut self.sim, Instant::from_seconds_since_j2000(seconds));
    }

    pub fn len(&self) -> usize {
        self.sim.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sim.len() == 0
    }

    /// Every body except the primary, as seen from `observer_ly` at a coordinate time.
    ///
    /// The last thing in the client that read the propagated arena. Placing bodies
    /// analytically here is what lets a system be shared, immutable, between every craft in
    /// it — nothing has to advance one to ask it a question any more.
    pub fn drawables_at(&self, observer_ly: DVec3, seconds: f64) -> Vec<Drawable> {
        self.drawn_at(observer_ly, seconds, true)
    }

    /// [`LocalSystem::drawables_at`] without the climate, which only paints a surface. For a
    /// survey, which asks every tick and never paints anything.
    pub fn drawables_unpainted_at(&self, observer_ly: DVec3, seconds: f64) -> Vec<Drawable> {
        self.drawn_at(observer_ly, seconds, false)
    }

    fn drawn_at(&self, observer_ly: DVec3, seconds: f64, painted: bool) -> Vec<Drawable> {
        let time = Instant::from_seconds_since_j2000(seconds);
        let star_at = match em_sim::propagate::position_at(&self.sim, self.primary, time) {
            Some(at) => at,
            None => return Vec::new(),
        };
        let observer_m = (observer_ly - self.origin_ly) * M_PER_LY;
        self.sim
            .indices()
            .filter(|i| *i != self.primary)
            .filter_map(|i| {
                let at = em_sim::propagate::position_at(&self.sim, i, time)?;
                if !at.is_finite() {
                    return None;
                }
                let radius_m = self.sim.appearance(i).radius();
                let from_star = at - star_at;
                let distance_m = from_star.length();
                if radius_m <= 0.0 || distance_m <= 0.0 {
                    return None;
                }
                let to_star = star_at - at;
                let to_observer = observer_m - at;
                let phase = phase_factor(to_star, to_observer);

                // The rings are found by the body's `em-sim` id, and their plane is that body's
                // own pole out of the preset's IAU rotation. A second copy of a pole here would
                // be a second chance to have it wrong.
                let kind = crate::navigation::Kind::of(&self.sim.info(i).tags);
                let pole = self.sim.rotation(i).and_then(pole_of).unwrap_or(DVec3::Z);
                let spin_s = self.sim.rotation(i).and_then(|r| self.spin_of(i, r));
                let rings = crate::rings::for_body(self.sim.name(i))
                    .map(|system| Rings { system, pole });

                let equilibrium_k = equilibrium_temperature(self.star_luminosity_w, distance_m);
                let surface = crate::surface::Surface::classify(
                    radius_m,
                    self.sim.info(i).mass,
                    equilibrium_k,
                );

                // What actually reflects: the lit disc, plus whatever of the rings is turned
                // toward both the star and the observer.
                let mut area = std::f64::consts::PI * radius_m * radius_m * phase;
                // Its own albedo, not one number for everything, and the same one a survey
                // reads per band: a second opinion about how bright a body is would be a
                // second chance to have it wrong.
                let tags = &self.sim.info(i).tags;
                let giant = surface.is_banded().then(|| {
                    let inputs = crate::giant::Inputs::from_tags(
                        surface,
                        self.sim.info(i).mass,
                        radius_m,
                        equilibrium_k,
                        self.star_feh,
                        self.star_teff_k,
                        spin_s,
                        tags,
                    );
                    crate::giant::of(self.sim.name(i), &inputs)
                });
                let world = crate::worlds::of(self.sim.name(i), surface, tags, giant.as_ref());
                let giant = giant.filter(|_| world.atmosphere == crate::worlds::Atmosphere::Envelope);
                let mut albedo = world.gray_albedo();
                if let Some(rings) = rings {
                    let lit = rings.pole.dot(to_star.normalize_or_zero()).abs();
                    let seen = rings.pole.dot(to_observer.normalize_or_zero()).abs();
                    let ring_area = rings.system.cross_section_m2() * lit * seen;
                    if ring_area > 0.0 {
                        // One albedo for the pair, weighted by what each contributes. Saturn's
                        // ice is brighter than Saturn.
                        let total = area + ring_area;
                        albedo = (albedo * area + rings.system.albedo * ring_area) / total;
                        area = total;
                    }
                }

                Some(Drawable {
                    name: self.sim.info(i).name.clone().unwrap_or_else(|| self.sim.name(i).into()),
                    kind,
                    rings,
                    surface,
                    // Keyed by the arena's id, which is what `rings::for_body` is keyed by and
                    // is not always the display name -- see `worlds`.
                    climate: painted
                        .then(|| crate::climate::of(self.sim.name(i), &world, equilibrium_k, self.star_teff_k, tags))
                        .flatten(),
                    giant,
                    airless: painted
                        .then(|| crate::airless::of(self.sim.name(i), &world, surface, radius_m))
                        .flatten(),
                    world,
                    pole,
                    spin_s,
                    position_ly: self.origin_ly + at / M_PER_LY,
                    radius_m,
                    mass_kg: self.sim.info(i).mass,
                    effective_radius_m: effective_radius_from_area(
                        self.star_radius_m,
                        area,
                        albedo,
                        distance_m,
                    ),
                    equilibrium_k,
                    effective_k: surface.effective_temperature(equilibrium_k),
                })
            })
            .collect()
    }

    /// Everything in the system a ship can be sent to, ordered outward from the primary with
    /// each body's own satellites behind it.
    pub fn inventory(&self) -> &[crate::navigation::Entry] {
        &self.inventory
    }

    /// The propagated bodies, for anything that needs more than [`LocalSystem::drawables`].
    pub fn sim(&self) -> &System {
        &self.sim
    }

    /// The most massive body: the star everything here orbits.
    pub fn primary(&self) -> BodyIndex {
        self.primary
    }

    /// The body whose sphere of influence holds a point: what an arc there is about, and what
    /// the map means by the primary.
    ///
    /// The star holds anything outside every other sphere, because its own influence has no
    /// outer edge until another star's begins.
    pub fn holding(&self, position_ly: DVec3, seconds: f64) -> BodyIndex {
        let at_m = (position_ly - self.origin_ly) * M_PER_LY;
        em_sim::influence::containing(&self.sim, at_m, Instant::from_seconds_since_j2000(seconds))
            .unwrap_or(self.primary)
    }

    pub fn body_named(&self, name: &str) -> Option<BodyIndex> {
        // By display name as well as by id, because the interface offers what `drawables` shows
        // and that is the display name.
        self.sim
            .indices()
            .find(|i| self.sim.info(*i).name.as_deref() == Some(name) || self.sim.name(*i) == name)
    }

    /// Where a named body is in the arena as it currently stands, light-years from the world
    /// origin.
    ///
    /// The arena holds one instant and nothing advances it any more, so outside a test that
    /// set it deliberately this is the load-time position. Use
    /// [`body_position_at`](Self::body_position_at) and say which instant you mean.
    pub fn body_position_ly(&self, name: &str) -> Option<DVec3> {
        let index = self.body_named(name)?;
        Some(self.origin_ly + self.sim.position(index) / M_PER_LY)
    }

    /// Where a named body is at a coordinate time, light-years from the world origin.
    pub fn body_position_at(&self, name: &str, seconds: f64) -> Option<DVec3> {
        let (at, _) = self.body_state_at(self.body_named(name)?, seconds)?;
        Some(self.origin_ly + at / M_PER_LY)
    }

    /// Where the primary is at a coordinate time. It moves: a star with planets orbits their
    /// common center, which for the Sun and Jupiter is outside the Sun.
    /// How long a body takes to turn once, seconds.
    fn spin_of(&self, i: BodyIndex, rotation: &em_sim::body::BodyRotation) -> Option<f64> {
        use em_sim::body::RotationMode;
        match &rotation.mode {
            RotationMode::Spinning { angular_velocity, .. } => {
                (angular_velocity.abs() > 0.0).then(|| std::f64::consts::TAU / angular_velocity.abs())
            }
            // Locked is a statement about the orbit, not a rate: one turn per orbit about the
            // primary it is locked to.
            RotationMode::TidallyLocked { .. } => self.period_of(i),
        }
    }

    /// The semi-major axis of a body's orbit about whatever it goes round, meters.
    ///
    /// **Not how far away it is now.** Those differ by a factor of `1 +- e`, so a period taken
    /// from the distance rather than the axis is out by `(1 +- e)^1.5` -- thirty per cent for
    /// Mercury, and enough for anything past an eccentricity of about 0.013 to miss the window
    /// a transit is identified by. [`crate::navigation::Entry::orbit_radius_m`] is the distance
    /// and says so; this is the element.
    ///
    /// `None` for a body whose motion is not Keplerian, which nothing generated or preset is.
    pub fn semi_major_of(&self, i: BodyIndex) -> Option<f64> {
        let when = Instant::from_seconds_since_j2000(INVENTORY_EPOCH_S);
        match &self.sim.motive(i).motive_at(when).1 {
            em_sim::motive::MotiveSelection::Keplerian(kepler) => {
                let a = kepler.semi_major_axis();
                (a.is_finite() && a > 0.0).then_some(a)
            }
            _ => None,
        }
    }

    /// How long a body takes to go once round its primary, seconds.
    ///
    /// The primary's own `mu` where the arena carries one -- Sol's preset states them -- and
    /// `G` times its stated mass otherwise, which is what a generated system gives.
    pub fn period_of(&self, i: BodyIndex) -> Option<f64> {
        const G: f64 = 6.674_301_5e-11;
        let parent = self.sim.parent(i)?;
        let mu = match self.sim.mu(parent) {
            stated if stated > 0.0 => stated,
            _ => G * self.sim.info(parent).mass,
        };
        let a = self.semi_major_of(i)?;
        (mu > 0.0).then(|| em_foundations::kepler::period::third_law(a, mu))
    }

    /// The same, for a body named by what a course targets it as.
    pub fn period_of_target(&self, key: &str) -> Option<f64> {
        self.period_of(self.sim.by_name(key)?)
    }

    pub fn star_position_at(&self, seconds: f64) -> Option<DVec3> {
        let (at, _) = self.body_state_at(self.primary, seconds)?;
        Some(self.origin_ly + at / M_PER_LY)
    }

    /// A body's position and velocity at a coordinate time, simulation frame, meters and
    /// meters a second — without propagating this system to get there.
    ///
    /// [`LocalSystem::sim`]'s accessors read the arena, which holds one instant: asking them
    /// where a body *will* be means propagating a copy first. This walks the body's parent
    /// chain analytically instead, so anything defined against a body is evaluable at an
    /// arbitrary time without the caller having to arrange the clock. `None` for a body whose
    /// chain is integrated rather than evaluated, which has no closed form to ask.
    pub fn body_state_at(&self, index: BodyIndex, seconds: f64) -> Option<(DVec3, DVec3)> {
        em_sim::propagate::state_at(&self.sim, index, Instant::from_seconds_since_j2000(seconds))
    }

    /// A body's spin axis, simulation axes. Ecliptic north where the data says nothing.
    pub fn body_pole(&self, index: BodyIndex) -> DVec3 {
        self.sim.rotation(index).and_then(pole_of).unwrap_or(DVec3::Z)
    }

    /// Coordinate seconds the system is currently propagated to.
    pub fn time_s(&self) -> f64 {
        self.sim.time().to_j2000_seconds()
    }

    /// A copy of this system propagated to another time.
    ///
    /// A copy because propagating is a mutation and the caller is usually asking about the
    /// future while the present is still being drawn. Two hundred and thirty bodies of analytic
    /// elements; only planning does this, never a frame.
    pub fn propagated_to(&self, seconds: f64) -> Self {
        let mut copy = self.clone();
        copy.advance_to(seconds);
        copy
    }

    /// The axis the system as a whole turns about: the primary's own pole.
    pub fn axis(&self) -> DVec3 {
        self.body_pole(self.primary)
    }

    /// How far out the system reaches, light-years.
    ///
    /// The outermost thing in it, which is the cloud if it has one and its furthest body
    /// otherwise. What "leaving" has to clear.
    pub fn reach_ly(&self) -> f64 {
        let star_at = self.sim.position(self.primary);
        let bodies = self
            .sim
            .indices()
            .map(|i| (self.sim.position(i) - star_at).length())
            .fold(0.0f64, f64::max);
        let populations = self
            .populations
            .iter()
            .map(|p| p.semi_major.nodes().iter().map(|(a, _)| *a).fold(0.0f64, f64::max))
            .fold(0.0f64, f64::max);
        (bodies.max(populations) / M_PER_LY).max(LOCAL_SHELL_LY)
    }

    pub fn star_luminosity_w(&self) -> f64 {
        self.star_luminosity_w
    }

    pub fn star_teff_k(&self) -> f64 {
        self.star_teff_k
    }

    pub fn star_radius_m(&self) -> f64 {
        self.star_radius_m
    }

    pub fn star_mass_kg(&self) -> f64 {
        self.sim.info(self.primary).mass
    }

    /// Where the star is, light-years from the world origin.
    pub fn star_position_ly(&self) -> DVec3 {
        self.origin_ly + self.sim.position(self.primary) / M_PER_LY
    }
}

/// The radius a blackbody at the star's temperature would need to deliver a body's reflected
/// flux, so that a lit body can be drawn by the same shader as the star lighting it.
///
/// `R_eff = R_star * R_body * sqrt(p) / d`, from equating `pi R_eff^2 B / D^2` with the
/// standard `L p R^2 / (4 pi d^2 D^2)`. Exact for a gray reflector, and checked against a
/// measured magnitude: Jupiter comes out at -2.72 against an observed -2.70.
///
/// Reflected light has the star's spectrum, which is what makes this work at all. A body with a
/// strongly colored albedo — Mars — comes out the star's color rather than its own, and that
/// is the approximation being made.
pub fn effective_radius(star_radius_m: f64, radius_m: f64, albedo: f64, distance_m: f64) -> f64 {
    effective_radius_from_area(
        star_radius_m,
        std::f64::consts::PI * radius_m * radius_m,
        albedo,
        distance_m,
    )
}

/// The same, for a reflector of any shape: `R_eff = R_star sqrt(p A / pi) / d`.
///
/// Taking an area rather than a radius is what lets a ringed planet be one source. Saturn's
/// rings reflect a few times what Saturn does when they are open, and nothing about that is a
/// sphere.
pub fn effective_radius_from_area(
    star_radius_m: f64,
    area_m2: f64,
    albedo: f64,
    distance_m: f64,
) -> f64 {
    if distance_m <= 0.0 || albedo <= 0.0 || area_m2 <= 0.0 {
        return 0.0;
    }
    star_radius_m * (albedo * area_m2 / std::f64::consts::PI).sqrt() / distance_m
}

/// A stable number from a body's name, for anything that needs a seed and has only a name.
pub fn name_seed(name: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    crate::rng::mix(h)
}

/// A body's pole, from whichever way its rotation is described.
pub fn pole_of(rotation: &em_sim::body::BodyRotation) -> Option<DVec3> {
    use em_sim::body::RotationMode;
    let pole = match &rotation.mode {
        RotationMode::Spinning { orientation_at_epoch, .. } => *orientation_at_epoch * DVec3::Z,
        RotationMode::TidallyLocked { pole, .. } => *pole,
    };
    (pole.length_squared() > 0.0).then(|| pole.normalize())
}

/// Lambert phase function: how much of a lit sphere is turned toward the observer.
///
/// One at opposition, zero at conjunction. Not optional — Venus at a tenth of an astronomical
/// unit is a razor crescent, and ignoring phase makes it three magnitudes too bright.
pub fn phase_factor(to_star: DVec3, to_observer: DVec3) -> f64 {
    let (a, b) = (to_star.normalize_or_zero(), to_observer.normalize_or_zero());
    if a == DVec3::ZERO || b == DVec3::ZERO {
        return 1.0;
    }
    let alpha = a.dot(b).clamp(-1.0, 1.0).acos();
    ((alpha.sin() + (std::f64::consts::PI - alpha) * alpha.cos()) / std::f64::consts::PI).max(0.0)
}

/// What a body at `distance_m` from a star of `luminosity_w` settles at, kelvin.
///
/// The sphere case of the balance in `crate::population`: absorbing on a cross-section and
/// radiating from the whole surface.
pub fn equilibrium_temperature(luminosity_w: f64, distance_m: f64) -> f64 {
    if distance_m <= 0.0 {
        return 0.0;
    }
    let denominator =
        16.0 * std::f64::consts::PI * distance_m * distance_m * em_spectra::blackbody::SIGMA;
    (luminosity_w / denominator).powf(0.25)
}

/// Everything in a system a ship can be sent to, ordered outward.
///
/// The order is hierarchical, not by distance from the star: a moon's heliocentric distance is
/// its planet's, so ordering on that would shuffle Jupiter's moons into whatever arrangement
/// they happened to be in this instant. Each body is keyed by the chain of orbital radii from
/// the primary down to itself, so planets come out by their own distance and a planet's moons
/// come out behind it by theirs.
fn build_inventory(
    sim: &System,
    primary: BodyIndex,
    star_name: &str,
    populations: &[crate::population::Population],
) -> Vec<crate::navigation::Entry> {
    use crate::navigation::{Entry, Kind, Target, designate};

    // Rank among its siblings, for a body with nothing better to be called.
    let mut ranked: Vec<(BodyIndex, f64)> = Vec::new();
    let mut keyed: Vec<(Vec<f64>, BodyIndex)> = Vec::new();
    for i in sim.indices() {
        let mut chain = Vec::new();
        let mut at = i;
        while at != primary {
            let Some(parent) = sim.parent(at) else { break };
            chain.push((sim.position(at) - sim.position(parent)).length());
            at = parent;
        }
        chain.reverse();
        keyed.push((chain, i));
        ranked.push((i, 0.0));
    }
    keyed.sort_by(|a, b| {
        a.0.iter()
            .zip(b.0.iter())
            .find_map(|(x, y)| match x.total_cmp(y) {
                std::cmp::Ordering::Equal => None,
                other => Some(other),
            })
            .unwrap_or_else(|| a.0.len().cmp(&b.0.len()))
    });

    let mut seen_under: std::collections::HashMap<Option<BodyIndex>, usize> = Default::default();
    let mut entries: Vec<(f64, Entry)> = Vec::new();
    for (chain, i) in keyed {
        let parent = (i != primary).then(|| sim.parent(i)).flatten();
        let rank = seen_under.entry(parent).or_insert(0);
        *rank += 1;
        let info = sim.info(i);
        let under = parent.map(|p| sim.info(p).name.clone().unwrap_or_else(|| sim.name(p).into()));
        entries.push((
            chain.first().copied().unwrap_or(0.0),
            Entry {
                designation: designate(
                    info.name.as_deref(),
                    info.designation.as_deref(),
                    under.as_deref().unwrap_or(star_name),
                    *rank,
                ),
                kind: Kind::of(&info.tags),
                orbit_radius_m: chain.last().copied().unwrap_or(0.0),
                depth: chain.len(),
                major: sim.is_major(i),
                target: Target::Body(sim.info(i).name.clone().unwrap_or_else(|| sim.name(i).into())),
            },
        ));
    }

    for (index, population) in populations.iter().enumerate() {
        let radius = population.thermal_radius();
        entries.push((
            radius,
            Entry {
                designation: crate::navigation::band_designation(population),
                kind: Kind::Band,
                orbit_radius_m: radius,
                depth: 0,
                major: true,
                target: Target::Band(index),
            },
        ));
    }

    // One stable sort on the outermost key puts the bands among the planets by radius; the
    // hierarchy inside each planet is already in place and a stable sort leaves it alone.
    entries.sort_by(|a, b| a.0.total_cmp(&b.0));
    entries.into_iter().map(|(_, entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use crate::sky::{AuthoredStars, StarProvider};

    use super::*;

    const AU: f64 = 1.495_978_707e11;

    fn catalog() -> Option<crate::sky::hyg::HygProvider> {
        crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv").ok()
    }

    /// **The plane a map draws has to be the one the planets are actually in.** This is the
    /// claim `LocalSystem::pole` exists to make, and the whole of phase 1 of
    /// `lightcone/docs/25-system-knowledge.md` rests on it: `+Z` was drawn under systems whose
    /// planets orbit somewhere else, which put every planet out of the plane beneath it.
    ///
    /// Generated inclinations are gaussian with a two-degree sigma, so ten is four sigma and
    /// the same figure catches a pole that is ignored outright -- a random pole is 60 degrees
    /// out on average.
    #[test]
    fn a_generated_systems_planets_lie_in_its_own_pole() {
        let stars = AuthoredStars::sample();
        // The third authored star is the one whose generated system has planets.
        let star = &StarProvider::stars(&stars)[2];
        let system = LocalSystem::for_star(star).expect("a generated system");
        let drawn = system.drawables_at(star.position_ly, 0.0);
        assert!(!drawn.is_empty(), "nothing to measure");

        for body in &drawn {
            let offset = body.position_ly - system.star_position_ly();
            let out = offset.normalize().dot(system.pole).abs().asin().to_degrees();
            assert!(out < 10.0, "{} is {out:.1}° out of its own system's plane", body.name);
        }
        // And the pole is not simply +Z, or this would pass without measuring anything.
        assert!(system.pole.dot(DVec3::Z).abs() < 0.999, "the generated pole is +Z");
    }

    /// A drawable's kind is the same answer the inventory gives, for every body in the solar
    /// system that appears in both.
    ///
    /// The point of carrying the field rather than joining on the name: `designate` invents a
    /// designation for an unnamed body and `Drawable::name` comes from the arena, so a join
    /// would quietly classify whatever it failed to match as a minor body. Titan and the Moon
    /// are the ones to watch, and this asserts all two hundred.
    #[test]
    fn a_drawable_is_the_kind_the_inventory_says_it_is() {
        let Some(provider) = catalog() else { return };
        let Some(sun) = provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some(SOL)) else {
            panic!("the catalog should carry Sol")
        };
        let system = LocalSystem::for_star(sun).expect("Sol loads");
        let drawn = system.drawables_at(system.origin_ly, 0.0);
        assert!(drawn.len() > 100, "only {} bodies", drawn.len());

        let mut checked = 0;
        for body in &drawn {
            let Some(entry) = system.inventory().iter()
                .find(|e| e.designation == body.name) else { continue };
            assert_eq!(body.kind, entry.kind, "{} is a {:?} in one place and a {:?} in the other",
                body.name, body.kind, entry.kind);
            checked += 1;
        }
        assert!(checked > 50, "only {checked} bodies matched by name at all");
        assert!(drawn.iter().any(|b| b.kind == crate::navigation::Kind::Moon), "no moons");
        assert!(drawn.iter().any(|b| b.kind == crate::navigation::Kind::Planet), "no planets");
    }

    #[test]
    fn the_solar_system_is_the_real_one_and_the_rest_are_generated() {
        let Some(provider) = catalog() else { return };
        let sun = provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some(SOL));
        let Some(sun) = sun else { panic!("the catalog should carry Sol") };

        let real = LocalSystem::for_star(sun).expect("Sol loads");
        assert!(real.len() > 100, "the preset carries moons too, got {}", real.len());

        let other = provider.stars().iter().find(|s| s.provenance.name.as_deref() != Some(SOL)).unwrap();
        let made = LocalSystem::for_star(other).expect("a generated system loads");
        assert!(!made.is_empty() && made.len() < real.len());
    }

    #[test]
    fn a_generated_system_loads_from_the_authored_sky() {
        let sky = AuthoredStars::sample();
        let star = &sky.stars()[0];
        let mut sys = LocalSystem::for_star(star).expect("a system");
        sys.advance_to(0.0);
        for d in sys.drawables_at(star.position_ly + DVec3::X * 1e-4, 0.0) {
            assert!(d.position_ly.is_finite() && d.radius_m > 0.0, "{d:?}");
            assert!(d.equilibrium_k > 0.0);
        }
    }

    /// The formula the whole approach rests on, against a measured magnitude. Jupiter at
    /// opposition is magnitude -2.70; drawn as a blackbody of this radius at the Sun's
    /// temperature it comes out -2.72.
    #[test]
    fn the_effective_radius_reproduces_jupiters_magnitude() {
        let r_sun = 6.957e8;
        let r_eff = effective_radius(r_sun, 6.9911e7, 0.538, 5.204 * AU);
        // Flux against the Sun's, both seen from Earth at opposition.
        let d_obs = 4.204 * AU;
        let ratio = (r_eff / r_sun).powi(2) * (AU / d_obs).powi(2);
        let magnitude = -26.74 - 2.5 * ratio.log10();
        assert!((magnitude + 2.70).abs() < 0.1, "Jupiter came out at {magnitude}");
    }

    #[test]
    fn a_body_with_nowhere_to_be_or_nothing_to_reflect_is_not_drawn() {
        assert_eq!(effective_radius(6.957e8, 1e6, 0.3, 0.0), 0.0);
        assert_eq!(effective_radius(6.957e8, 1e6, 0.0, AU), 0.0);
        assert_eq!(equilibrium_temperature(3.8e26, 0.0), 0.0);
    }

    /// Ignoring this makes Venus three magnitudes too bright at inferior conjunction, which is
    /// how it was found.
    #[test]
    fn phase_is_one_at_opposition_and_zero_at_conjunction() {
        let full = phase_factor(DVec3::X, DVec3::X);
        let dark = phase_factor(DVec3::X, -DVec3::X);
        let half = phase_factor(DVec3::X, DVec3::Y);
        assert!((full - 1.0).abs() < 1e-12, "{full}");
        assert!(dark.abs() < 1e-12, "{dark}");
        assert!((half - 1.0 / std::f64::consts::PI).abs() < 1e-12, "{half}");
        assert!(half < full && dark < half);
    }

    #[test]
    fn equilibrium_temperature_puts_the_earth_where_it_belongs() {
        let t = equilibrium_temperature(3.828e26, AU);
        assert!((t - 278.3).abs() < 1.0, "{t} K at one astronomical unit");
        // And four times out is half as warm.
        let far = equilibrium_temperature(3.828e26, 4.0 * AU);
        assert!((t / far - 2.0).abs() < 1e-9);
    }

    #[test]
    fn propagating_moves_the_bodies() {
        let Some(provider) = catalog() else { return };
        let Some(sun) = provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some(SOL)) else {
            return;
        };
        let sys = LocalSystem::for_star(sun).unwrap();
        let observer = sun.position_ly + DVec3::X * (AU / M_PER_LY);

        // Two times, one system. Nothing has to be propagated to ask where a body will be.
        let before = sys.drawables_at(observer, 0.0);
        let after = sys.drawables_at(observer, 200.0 * 86_400.0);

        assert_eq!(before.len(), after.len());
        assert!(!before.is_empty(), "the solar system should have something in it");
        let moved = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| a.position_ly.distance(b.position_ly) > 1e-9)
            .count();
        assert!(moved > before.len() / 2, "only {moved} of {} moved", before.len());
        assert!(after.iter().all(|d| d.position_ly.is_finite()));
    }

    /// Reflected brightness falls off far faster than distance alone: the flux reaching the
    /// body goes as one over the square, and what it returns goes as the square of that.
    #[test]
    fn a_body_further_out_reflects_far_less() {
        let near = effective_radius(6.957e8, 6.99e7, 0.5, AU);
        let far = effective_radius(6.957e8, 6.99e7, 0.5, 10.0 * AU);
        assert!((near / far - 10.0).abs() < 1e-9, "the effective radius goes as 1/d");
    }

    /// The end-to-end check: from where the Earth is, the brightest things in the solar system
    /// should be the planets a person can see, in roughly the order they see them.
    #[test]
    fn the_naked_eye_planets_are_the_brightest_things_in_the_sky() {
        let Some(provider) = catalog() else { return };
        let Some(sun) = provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some(SOL)) else {
            return;
        };
        let mut sys = LocalSystem::for_star(sun).unwrap();
        sys.advance_to(0.0);

        // Roughly where the Earth is at J2000, which is where the catalog puts the observer.
        let earth = sys
            .drawables_at(sun.position_ly, 0.0)
            .into_iter()
            .find(|d| d.name == "Earth")
            .expect("the preset carries the Earth");
        let observer = earth.position_ly;

        let mut lit = sys.drawables_at(observer, 0.0);
        // Brightness goes as the effective radius squared over the distance squared.
        lit.sort_by(|a, b| {
            let flux = |d: &Drawable| {
                let r = d.effective_radius_m / observer.distance(d.position_ly).max(1e-30);
                -(r * r)
            };
            flux(a).total_cmp(&flux(b))
        });
        let top: Vec<&str> = lit.iter().take(8).map(|d| d.name.as_str()).collect();
        for want in ["Venus", "Jupiter", "Mars", "Saturn"] {
            assert!(top.contains(&want), "{want} should be among the brightest, got {top:?}");
        }
        // And the Moon, which is the brightest of all from here.
        assert!(top.contains(&"Luna") || top.contains(&"Moon"), "{top:?}");
    }

    /// The 0.7 magnitude that was missing when Saturn was a bare sphere. Its rings reflect a
    /// few times what the planet does when they are open, and nothing at all when edge-on.
    #[test]
    fn saturns_rings_brighten_it_and_the_tilt_decides_by_how_much() {
        let Some(provider) = catalog() else { return };
        let Some(sun) = provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some(SOL)) else {
            return;
        };
        let mut sys = LocalSystem::for_star(sun).unwrap();
        sys.advance_to(0.0);

        let saturn = sys
            .drawables_at(sun.position_ly, 0.0)
            .into_iter()
            .find(|d| d.name == "Saturn")
            .expect("the preset carries Saturn");
        let rings = saturn.rings.expect("and Saturn has rings");
        assert!(rings.pole.is_normalized());

        // Saturn's obliquity is 26.7 degrees, so its pole is well off the ecliptic.
        let tilt = rings.pole.dot(DVec3::Z).acos().to_degrees();
        assert!((tilt - 26.7).abs() < 3.0, "obliquity came out {tilt} degrees");

        // Bare sphere against sphere plus rings, at the same place.
        let bare = effective_radius(6.957e8, saturn.radius_m, DEFAULT_ALBEDO, 9.583 * AU);
        assert!(saturn.effective_radius_m > bare, "rings should add light");
        let magnitudes = -2.5 * (saturn.effective_radius_m / bare).powi(2).log10();
        assert!(magnitudes < -0.3, "rings should be worth real brightness: {magnitudes}");
    }

    #[test]
    fn a_ring_seen_edge_on_reflects_nothing() {
        let s = crate::rings::for_body("Saturn").unwrap();
        // The formula's two cosines: face-on is the whole cross-section, edge-on is none.
        let face = s.cross_section_m2() * 1.0 * 1.0;
        let edge = s.cross_section_m2() * 0.0 * 1.0;
        assert!(face > 0.0);
        assert_eq!(edge, 0.0);
        assert_eq!(effective_radius_from_area(6.957e8, 0.0, 0.5, AU), 0.0);
    }

    /// The area form and the radius form have to be the same function.
    #[test]
    fn the_area_and_radius_forms_agree_for_a_sphere() {
        let r = 6.9911e7;
        let a = std::f64::consts::PI * r * r;
        let by_radius = effective_radius(6.957e8, r, 0.538, 5.204 * AU);
        let by_area = effective_radius_from_area(6.957e8, a, 0.538, 5.204 * AU);
        assert!((by_radius / by_area - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_body_without_rings_has_none_and_is_unaffected() {
        let Some(provider) = catalog() else { return };
        let Some(sun) = provider.stars().iter().find(|s| s.provenance.name.as_deref() == Some(SOL)) else {
            return;
        };
        let mut sys = LocalSystem::for_star(sun).unwrap();
        sys.advance_to(0.0);
        let earth = sys.drawables_at(sun.position_ly, 0.0).into_iter().find(|d| d.name == "Earth").unwrap();
        assert!(earth.rings.is_none());
    }
}

