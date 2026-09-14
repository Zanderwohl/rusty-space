//! What the client knows and is doing. No engine: the Bevy layer reads this.

use std::collections::HashMap;
use std::sync::Arc;

use em_spectra::{Band, BandMapping, PerBand, blackbody, presets};
use glam::{DVec3, Vec3};
use lc_spacetime::{Coord, Micros, frame::SystemFrame, units::Span};
use lc_world::craft::{Craft, CraftId, Fleet, Kind};
use lc_world::instrument::Instrument;
use lc_world::motion::{self, Motive};
use lc_world::observation::{Observation, Target, observe};
use lc_world::sky::{CatalogueStar, StarId, StarProvider, generate};

use crate::curve::LightCurve;
use crate::flight::{Cruise, STANDOFF_LY};
use crate::tonemap::{Shaded, ToneMap};

/// One in-game Julian year per real hour.
pub const TIME_RATE: f64 = 31_557_600.0 / 3600.0;

/// Light-microseconds per light-year, for putting the observer on the grid.
const LUS_PER_LY: f64 = 1.0 / LY_PER_LUS;

/// Light-years per light-microsecond, for reading catalogue positions onto the grid.
const LY_PER_LUS: f64 = 299.792458 / 9.460_730_472_580_8e15;

/// What the ship looks through.
///
/// Four square metres, every band, and cooled. Not [`Instrument::BASELINE`], which is a one
/// metre silicon camera at room temperature: it cannot reach the thermal infrared at all, and
/// its own 290 K housing glows straight into the band a swarm lives in. A ship that is a mind
/// with no eyes builds the sensor it needs.
pub const SHIP_SENSOR: Instrument = Instrument::SHIP;

/// How many nearby stars get a generated system and a full emission model.
///
/// The rest are drawn as bare blackbodies. Starlight is a function rather than an event
/// stream, so a star with nothing around it costs one evaluation and needs no model at all.
pub const MODELLED_STARS: usize = 12;

/// A star as the renderer wants it.
#[derive(Clone, Copy, Debug)]
pub struct SkyStar {
    pub id: StarId,
    /// Where it is relative to the observer, light-years, ecliptic axes.
    pub offset_ly: DVec3,
    /// Unit vector to draw along: [`offset_ly`] aberrated into the ship's frame. Equal to the
    /// true direction at rest, and swung toward the bow at speed.
    ///
    /// [`offset_ly`]: SkyStar::offset_ly
    pub apparent_dir: DVec3,
    pub shaded: Shaded,
    /// Light travel time from it, in seconds. Everything drawn is this stale.
    pub light_age_s: f64,
    /// Observed over emitted frequency. Above 1 is a blueshift.
    pub doppler: f64,
}

/// Solid angle of one pixel at ninety degrees across a 1080-line viewport, steradians.
///
/// Only a fallback for [`Scene::point_sr`] before a camera exists. It cancels out of a
/// star-only metering — see [`Session::expose_to_percentile`] — so its value is arbitrary
/// there; it matters only if bodies are metered without a camera, which the renderer never
/// does.
pub const NOMINAL_POINT_SR: f32 = 3.429_355e-6;

/// One body drawn as a disc.
#[derive(Clone, Debug, PartialEq)]
pub struct Disc {
    /// Radiance of its lit surface, per band.
    pub radiance: PerBand<f32>,
    /// How much sky it covers, steradians. Its weight in the metering.
    pub solid_angle_sr: f32,
}

/// What the exposure has to fit besides the star field.
///
/// Filled by the renderer, because which bodies are discs and how large they are drawn is a
/// fact about the camera. Empty between systems, which is most of the time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// Solid angle a point source is drawn at, steradians. Zero until a camera exists.
    pub point_sr: f32,
    /// Flux from each body still small enough to be drawn as a point, per band.
    pub points: Vec<PerBand<f32>>,
    pub discs: Vec<Disc>,
}

pub struct Session {
    pub stars: Vec<CatalogueStar>,
    pub observer: Coord,
    pub telescope: Instrument,
    pub mapping: BandMapping,
    pub tone: ToneMap,
    pub curve: LightCurve,
    pub pointing: Option<StarId>,
    /// The ship: where it is, how fast, and which of the four ways it is moving.
    ///
    /// `lc-world`'s, not the client's. The server is authoritative over this and the client
    /// predicts by running the same fold, so there is one implementation and this is a handle
    /// to it. [`Session::observer`] is its position rounded onto the grid.
    pub ship: Craft,
    /// The system the ship is inside, or `None` between stars.
    ///
    /// Here rather than beside the renderer because a course is set against it: an orbit, a
    /// libration point and a belt are all defined by bodies, and the action that chooses one
    /// has to be able to see them.
    pub system: Option<Arc<crate::system::LocalSystem>>,
    /// Everything else with a worldline: probes, relays, beacons. The ship is not in here —
    /// it is the one the player is flying, and it always exists.
    pub fleet: Fleet,
    /// What the renderer is about to draw besides the stars. Metered, never drawn from.
    pub scene: Scene,
    targets: HashMap<StarId, Target>,
}

impl Session {
    /// Take the nearest stars to the origin and model a few of them.
    pub fn new(provider: &dyn StarProvider, count: usize) -> Self {
        let mut stars: Vec<CatalogueStar> = provider.stars().to_vec();
        stars.sort_by(|a, b| a.position_ly.length().total_cmp(&b.position_ly.length()));
        stars.truncate(count.max(1));

        let mut targets = HashMap::new();
        for star in stars.iter().take(MODELLED_STARS) {
            targets.insert(star.id, build_target(star));
        }

        let mut session = Self {
            stars,
            observer: Coord::ORIGIN,
            telescope: SHIP_SENSOR,
            mapping: presets::natural(),
            tone: ToneMap::default(),
            curve: LightCurve::new(Band::V, 4000),
            pointing: None,
            ship: Craft::at(CraftId(0), Kind::Ship, DVec3::ZERO),
            scene: Scene::default(),
            system: None,
            fleet: Fleet::new(),
            targets,
        };
        session.retune();
        session.auto_expose();
        session
    }

    /// The star whose system the ship is inside, if it is inside one.
    pub fn local_star(&self) -> Option<&CatalogueStar> {
        self.stars.iter().find(|s| self.distance_to(s) < crate::starfield::LOCAL_SHELL_LY)
    }

    /// Load or drop the local system, and propagate it to now.
    ///
    /// Loading is the expensive part — two hundred and thirty bodies parsed out of a preset —
    /// so it happens only when the ship crosses into a different star's shell.
    pub fn sync_system(&mut self) {
        let here = self.local_star().map(|s| s.id);
        if self.system.as_ref().map(|s| s.star) == here {
            return;
        }
        self.system = self
            .local_star()
            .and_then(crate::system::LocalSystem::for_star)
            .map(Arc::new);
        // Shared, never propagated. Every motive is evaluated at the time asked for, so one
        // immutable copy serves the ship, every probe, and the renderer at once.
        let now = self.coordinate_time_s();
        self.ship.enter(self.system.clone(), now);
        for craft in self.fleet.iter_mut() {
            craft.enter(self.system.clone(), now);
        }
    }

    /// Point the band mapping at what the instrument can actually sense.
    ///
    /// A band the sensor cannot reach contributes nothing rather than reading as dark, which is
    /// the distinction `BandMask` exists for. Without this the sky is drawn through seven bands
    /// while the telescope reports four, and the two disagree about the same star.
    pub fn retune(&mut self) {
        self.mapping.available = self.telescope.bands;
    }

    /// Advance coordinate time by a span of real seconds, flying the ship along with it.
    /// Advance coordinate time by a span of real seconds, flying the ship along with it.
    ///
    /// Three ways to be moving and they are exclusive. Under thrust the crossing says where the
    /// ship is; on station the waypoint does; otherwise it is ballistic — a conic inside a
    /// system and a straight line between them. All three are *read* at the new time rather
    /// than integrated from the old one, so nothing drifts with the frame rate.
    pub fn advance(&mut self, real_seconds: f64) {
        let elapsed = real_seconds * TIME_RATE;
        let micros = (elapsed * 1e6) as i64;
        self.observer.t = self.observer.t + Span::new(micros);
        let now = self.coordinate_time_s();
        // Before anything is placed against it: a station and a conic are both positions in a
        // system, and one propagated to last frame would put the ship a frame behind.
        self.sync_system();

        self.ship.advance(now, elapsed);
        self.fleet.advance(now, elapsed);
        self.sync_observer();
    }

    /// How fast the ship is going, metres a second, world frame.
    pub fn velocity_m_s(&self) -> DVec3 {
        motion::velocity_m_s(&self.ship.motion, self.system.as_deref(), self.coordinate_time_s())
    }

    /// The crossing under way, if there is one.
    pub fn cruise(&self) -> Option<&Cruise> {
        match &self.ship.motion.motive {
            Motive::Crossing(cruise) => Some(cruise),
            _ => None,
        }
    }

    /// The place the ship is being held on, if it is.
    pub fn station(&self) -> Option<&crate::navigation::Waypoint> {
        match &self.ship.motion.motive {
            Motive::Holding(waypoint) => Some(waypoint),
            _ => None,
        }
    }

    /// The ballistic arc the ship is on, if it is on one.
    pub fn coast(&self) -> Option<&crate::coast::Coast> {
        match &self.ship.motion.motive {
            Motive::Falling(arc) => Some(arc),
            _ => None,
        }
    }

    /// Cut the engine and keep going.
    ///
    /// Not a stop. The ship keeps the velocity it had, which inside a system means it is now on
    /// whatever conic that velocity puts it on about whichever body holds it. Returns what it
    /// ended up on.
    pub fn cancel(&mut self) -> Option<crate::coast::Coast> {
        let event = motion::Event {
            ship: motion::ShipId(0),
            at_t: self.coordinate_time_s(),
            change: motion::Change::CutDrive,
        };
        // `Craft::apply` re-solves the patch: a new arc means the old answer to "when does
        // it leave" is about a different conic.
        let _ = self.ship.apply(&event);
        self.coast().cloned()
    }

    /// Put the continuous position back on the integer grid.
    ///
    /// Rounding is to the nearest light-microsecond, 300 metres. Retarded-time solving reads
    /// the grid, so this is what the light delay is actually computed against.
    fn sync_observer(&mut self) {
        let grid = self.ship.motion.position_ly * LUS_PER_LY;
        self.observer.x = grid.x as i64;
        self.observer.y = grid.y as i64;
        self.observer.z = grid.z as i64;
    }

    /// Begin a crossing to a star, stopping [`STANDOFF_LY`] short of it.
    ///
    /// The catalogue position is treated as fixed: nothing in this model has proper motion
    /// yet, so aiming at where it is recorded and aiming at where it will be are the same.
    pub fn fly_to(&mut self, id: StarId) -> Option<&Cruise> {
        let star = self.star(id)?;
        let target = star.position_ly;
        let approach = (target - self.ship.motion.position_ly).normalize_or_zero();
        let stop = target - approach * STANDOFF_LY;
        self.ship.motion.begin_crossing(
            Cruise::plan(self.ship.motion.position_ly, stop, self.coordinate_time_s(), self.ship.motion.drive),
            None,
        );
        self.cruise()
    }

    /// Set a course inside the local system, and hold there on arrival.
    ///
    /// Returns what to call the destination, or `None` if the system has nothing answering to
    /// it — a moon that is not there, rings on a body without any, a libration point of the
    /// star itself.
    pub fn set_course(&mut self, course: &crate::navigation::Course) -> Option<String> {
        let event = motion::Event {
            ship: motion::ShipId(0),
            at_t: self.coordinate_time_s(),
            change: motion::Change::SetCourse { course: course.clone(), drive: self.ship.motion.drive },
        };
        self.ship.apply(&event).ok()?;
        self.ship.motion.bound_for().map(|w| w.label())
    }


    /// Put the ship somewhere, cutting any crossing. Development only: there is no action for
    /// it and the server would never accept one.
    pub fn place_at(&mut self, position_ly: DVec3) {
        self.ship.motion.position_ly = position_ly;
        self.ship.motion.beta = DVec3::ZERO;
        self.ship.motion.set_adrift(self.coordinate_time_s());
        self.sync_observer();
    }

    /// Where a star is relative to the ship, light-years.
    pub fn offset_to(&self, star: &CatalogueStar) -> DVec3 {
        star.position_ly - self.ship.motion.position_ly
    }

    /// Distance to a star, light-years.
    pub fn distance_to(&self, star: &CatalogueStar) -> f64 {
        self.offset_to(star).length()
    }

    pub fn coordinate_time_s(&self) -> f64 {
        self.observer.t.get() as f64 * 1e-6
    }

    pub fn target(&self, id: StarId) -> Option<&Target> {
        self.targets.get(&id)
    }

    pub fn star(&self, id: StarId) -> Option<&CatalogueStar> {
        self.stars.iter().find(|s| s.id == id)
    }

    /// Point the telescope, clearing whatever it was watching.
    ///
    /// Builds the target's emission model if this is the first time anything has looked at it.
    /// [`MODELLED_STARS`] bounds what the *sky* evaluates every frame, which is a cost that
    /// scales with the field; the telescope looks at one star, and there is no reason a player
    /// should be unable to point it at the thirteenth-nearest.
    pub fn point_at(&mut self, id: Option<StarId>) {
        if self.pointing != id {
            self.curve.clear();
        }
        self.pointing = id;
        if let Some(id) = id {
            if !self.targets.contains_key(&id) {
                if let Some(star) = self.stars.iter().find(|s| s.id == id) {
                    let target = build_target(star);
                    self.targets.insert(id, target);
                }
            }
        }
    }

    /// Take one measurement of whatever the telescope is on.
    pub fn observe(&mut self, exposure_s: f64) -> Option<Observation> {
        let target = self.targets.get(&self.pointing?)?;
        let obs = observe(target, self.observer, &self.telescope, exposure_s, 0x10c)?;
        self.curve.record(&obs);
        Some(obs)
    }

    /// Observed over emitted frequency for one star, given the ship's velocity.
    pub fn doppler_to(&self, star: &CatalogueStar) -> f64 {
        let to_source = self.offset_to(star).normalize_or_zero();
        if to_source == DVec3::ZERO || self.ship.motion.beta == DVec3::ZERO {
            return 1.0;
        }
        lc_spacetime::doppler::doppler_factor(to_source, self.ship.motion.beta)
    }

    /// Band radiance arriving from one star, light delay and Doppler shift included.
    pub fn radiance_from(&self, star: &CatalogueStar) -> PerBand<f32> {
        let distance_m = self.distance_to(star) * M_PER_LY;
        // A blackbody seen with Doppler factor D is exactly a blackbody at D times the
        // temperature: B_nu/nu^3 is invariant and Planck's law depends only on nu/T. So the
        // shift needs no separate beaming term — integrating the observer's own bands against
        // the shifted temperature already carries the D^4 in the flux.
        let teff = (star.star.teff_k * self.doppler_to(star)).max(1.0);
        match self.targets.get(&star.id) {
            // A modelled system is evaluated properly, light delay and all.
            Some(target) => observe(target, self.observer, &full_spectrum(), 1.0, 0x5ee)
                .map(|o| received(&o, teff, star.star.radius_m, distance_m))
                .unwrap_or_default(),
            // A bare star is a function of its own parameters and nothing else.
            None => bare(teff, star.star.radius_m, distance_m),
        }
    }

    /// Every star, shaded for the current band mapping and exposure, in the ship's frame.
    pub fn sky(&self) -> Vec<SkyStar> {
        self.stars
            .iter()
            .map(|s| {
                let offset_ly = self.offset_to(s);
                let true_dir = offset_ly.normalize_or_zero();
                SkyStar {
                    id: s.id,
                    offset_ly,
                    apparent_dir: if self.ship.motion.beta == DVec3::ZERO || true_dir == DVec3::ZERO {
                        true_dir
                    } else {
                        lc_spacetime::doppler::apparent_source_direction(true_dir, self.ship.motion.beta)
                    },
                    shaded: self.tone.shade(&self.radiance_from(s), &self.mapping),
                    light_age_s: offset_ly.length() * M_PER_LY / 299_792_458.0,
                    doppler: self.doppler_to(s),
                }
            })
            .collect()
    }

    /// Put the brightest thing in the sky at the top of the displayed window.
    ///
    /// There is no absolute reference to use instead. A star's band radiance at
    /// interstellar range is of order 1e-10 in SI units, so any fixed reference is either
    /// thirty stops high or thirty stops low, and the picture is black or white accordingly.
    /// The window has to be placed by the scene, and it moves when the band mapping does —
    /// the thermal preset reads a different band and therefore a different brightness.
    pub fn auto_expose(&mut self) {
        self.expose_to_percentile(0.98);
    }

    /// Place the window so that `fraction` of the drawn sky falls below the top of it.
    ///
    /// A percentile rather than the maximum, because one star can be arbitrarily closer than
    /// the rest — the Sun is in the catalogue at about an astronomical unit — and exposing
    /// for it puts everything else thirty stops under and renders a black sky. Letting the
    /// brightest couple of percent clip is what a star map does anyway.
    ///
    /// The percentile is over *area*, not over count, which is what lets one pass meter a star
    /// field and a planet together. Every source is reduced to the brightness it has per unit
    /// of the sky it covers: a surface's own radiance, and for a point the flux it delivers
    /// divided by the solid angle the renderer spreads it over. Weighting each by that same
    /// solid angle makes the sum the power actually collected, so the rule reads as a light
    /// meter does — expose so that `1 - fraction` of the frame clips.
    ///
    /// Points all carry the same weight, so a sky with no bodies in it meters exactly as a
    /// count percentile over stars did, `point_sr` cancelling. A resolved planet does not: at
    /// a couple of hundred pixels across it outweighs six thousand stars together and takes
    /// the exposure with it, which is what a photograph of a planet looks like.
    pub fn expose_to_percentile(&mut self, fraction: f32) {
        let point_sr =
            if self.scene.point_sr > 0.0 { self.scene.point_sr } else { NOMINAL_POINT_SR };
        let mut samples: Vec<(f32, f32)> =
            Vec::with_capacity(self.stars.len() + self.scene.points.len() + self.scene.discs.len());
        let mut push = |brightness: f32, weight: f32| {
            if brightness > 0.0 && brightness.is_finite() && weight > 0.0 {
                samples.push((brightness, weight));
            }
        };
        for star in &self.stars {
            push(self.luminance_from(star) / point_sr, point_sr);
        }
        for flux in &self.scene.points {
            push(luminance_of(flux, &self.mapping) / point_sr, point_sr);
        }
        for disc in &self.scene.discs {
            // Floored at a point's weight: a body at the crossover is drawn at a pixel or two
            // whatever its true angle, and metering it at less than that would let a
            // just-resolved body count for nothing while being fully visible.
            push(luminance_of(&disc.radiance, &self.mapping), disc.solid_angle_sr.max(point_sr));
        }
        if samples.is_empty() {
            return;
        }
        samples.sort_by(|a, b| a.0.total_cmp(&b.0));
        let total: f32 = samples.iter().map(|(_, w)| w).sum();
        let want = total * fraction.clamp(0.0, 1.0);
        let mut below = 0.0;
        let mut at = samples.len() - 1;
        for (i, (_, weight)) in samples.iter().enumerate() {
            below += weight;
            if below >= want {
                at = i;
                break;
            }
        }
        self.tone.surface_reference = samples[at].0;
        self.tone.reference = samples[at].0 * point_sr;
    }

    /// Displayed luminance a star would contribute under the current mapping.
    pub fn luminance_from(&self, star: &CatalogueStar) -> f32 {
        luminance_of(&self.radiance_from(star), &self.mapping)
    }
}

/// Rec. 709 luminance of a per-band radiance under a mapping.
fn luminance_of(radiance: &PerBand<f32>, mapping: &BandMapping) -> f32 {
    let rgb = mapping.apply(radiance);
    rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722
}

/// An idealised instrument, for evaluating what leaves a system rather than what an
/// instrument would record of it.
fn full_spectrum() -> Instrument {
    Instrument::BASELINE
        .with_bands(em_spectra::BandMask::ALL)
        .cooled_to(20.0)
}

/// Metres in a light-year.
const M_PER_LY: f64 = 9.460_730_472_580_8e15;

fn geometry(radius_m: f64, distance_m: f64) -> f64 {
    std::f64::consts::PI * radius_m * radius_m / (distance_m * distance_m)
}

/// Band flux from a blackbody disc of `radius_m` seen from `distance_m`, W/m^2.
///
/// The whole of an unmodelled star's brightness, and — at the star's own temperature and a
/// body's effective radius — the whole of a body's reflected brightness.
pub fn bare(teff_k: f64, radius_m: f64, distance_m: f64) -> PerBand<f32> {
    let g = geometry(radius_m, distance_m);
    PerBand::new(std::array::from_fn(|i| {
        (blackbody::band_radiance(Band::ALL[i], teff_k) * g) as f32
    }))
}

/// The deficit is applied in the band it was measured in, which is only right at rest: under
/// a large shift the observer's V band samples what left the system somewhere else entirely.
/// Correcting it needs the emission model evaluated at shifted band centres, which the world
/// crate does not expose yet.
fn received(observation: &Observation, teff_k: f64, radius_m: f64, distance_m: f64) -> PerBand<f32> {
    let g = geometry(radius_m, distance_m);
    PerBand::new(std::array::from_fn(|i| {
        let band = Band::ALL[i];
        let full = blackbody::band_radiance(band, teff_k) * g;
        // Relative flux, not one minus the deficit: a warm population adds where a cold one
        // only subtracts, and in the thermal infrared the sum can exceed the bare star.
        let relative = observation.band(band).map(|m| m.relative_flux()).unwrap_or(1.0);
        (full * relative) as f32
    }))
}

fn build_target(star: &CatalogueStar) -> Target {
    let system = generate::system_for(star);
    let mut model = lc_world::emission::EmissionModel::new(star.star, star.seed());
    model.populations = system.populations;
    for planet in &system.planets {
        model.bodies.push(lc_world::emission::Body {
            occluder: lc_world::occluder::Occluder::new(planet.radius_m),
            motion: Box::new(lc_world::emission::CircularOrbit {
                radius_m: planet.semi_major_m,
                pole: DVec3::Z,
                phase0: planet.mean_anomaly_deg.to_radians(),
                mu: star.star.mu,
            }),
        });
    }
    let grid = star.position_ly / LY_PER_LUS / 9.460_730_472_580_8e15 * 299.792458;
    let origin = Coord::new(
        Micros::ORIGIN,
        (star.position_ly.x / LY_PER_LUS) as i64,
        (star.position_ly.y / LY_PER_LUS) as i64,
        (star.position_ly.z / LY_PER_LUS) as i64,
    )
    .unwrap_or(Coord::ORIGIN);
    let _ = grid;
    Target::new(SystemFrame::new(origin), model)
}

/// Stops of brightness a star field spreads over, beyond the tone map's own window.
pub const POINT_STOPS: f32 = 14.0;

/// How large a star should be drawn: its brightness across [`POINT_STOPS`], plus any glow.
pub fn point_size(shaded: &Shaded, base: f32) -> f32 {
    base * (0.35 + shaded.point_brightness(POINT_STOPS) + shaded.glow.clamp(0.0, 12.0) * 0.3)
}

/// Colour for a star drawn as a point.
pub fn point_colour(shaded: &Shaded) -> Vec3 {
    shaded.point_colour(POINT_STOPS)
}

#[cfg(test)]
mod tests {
    use lc_world::sky::AuthoredStars;

    use super::*;

    fn session() -> Session {
        Session::new(&AuthoredStars::sample(), 3)
    }

    #[test]
    fn a_session_sorts_its_stars_by_distance_and_models_the_near_ones() {
        let s = session();
        assert_eq!(s.stars.len(), 3);
        let d: Vec<f64> = s.stars.iter().map(|x| x.position_ly.length()).collect();
        assert!(d.windows(2).all(|w| w[0] <= w[1]), "{d:?}");
        assert!(s.target(s.stars[0].id).is_some());
    }

    #[test]
    fn the_clock_runs_a_year_an_hour() {
        let mut s = session();
        s.advance(3600.0);
        let years = s.coordinate_time_s() / 31_557_600.0;
        assert!((years - 1.0).abs() < 1e-6, "an hour should be {years} years");
    }

    #[test]
    fn every_star_in_the_sky_is_shaded_and_stale() {
        let s = session();
        let sky = s.sky();
        assert_eq!(sky.len(), 3);
        for star in &sky {
            assert!(star.shaded.colour().is_finite());
            assert!(star.light_age_s > 0.0, "everything drawn is old");
            // Four light-years is about four years of staleness.
            let years = star.light_age_s / 31_557_600.0;
            assert!(years > 4.0 && years < 30.0, "{years} years");
        }
    }

    /// The bug this exists for: the Sun sits in the catalogue about an astronomical unit
    /// away, and exposing for the brightest star renders everything else black.
    ///
    /// Not an artifact — the Sun at one AU outshines a star four light-years off by some
    /// thirty-six stops, which is why daylight hides the sky. What a percentile buys is that
    /// the outlier clips instead of setting the scale for everyone.
    #[test]
    fn one_very_close_star_does_not_black_out_the_field() {
        let template = AuthoredStars::sample().stars()[1].clone();
        let mut stars = Vec::new();
        for k in 0..200u64 {
            let mut s = template.clone();
            s.provenance.key = k;
            s.id = lc_world::sky::StarId::synthesise("many", k);
            let d = 4.0 + (k % 40) as f64;
            s.position_ly = glam::DVec3::new(d, (k % 7) as f64, (k % 11) as f64).normalize() * d;
            stars.push(s);
        }
        let mut sun = template.clone();
        sun.provenance.key = 9999;
        sun.id = lc_world::sky::StarId::synthesise("many", 9999);
        sun.position_ly = glam::DVec3::new(1.6e-5, 0.0, 0.0); // roughly an AU
        stars.insert(0, sun);

        let s = Session::new(&AuthoredStars::new("many", stars), 201);
        let visible = s.sky().iter().filter(|x| point_colour(&x.shaded).length() > 0.0).count();
        assert!(visible > 150, "only {visible} of 201 stars survived the exposure");

        // Exposing for the maximum instead is the failure: the field goes out.
        let mut naive = Session::new(&AuthoredStars::new("many", s.stars.clone()), 201);
        naive.expose_to_percentile(1.0);
        let left = naive.sky().iter().filter(|x| point_colour(&x.shaded).length() > 0.0).count();
        assert!(left < 5, "{left} stars should have survived exposing for the Sun");
    }

    // The world model moved to `lc-world`; these two stayed, because what they check is the
    // session driving it rather than the model itself.

    /// The whole of it, through the session rather than the pieces: ask for an orbit, fly, and
    /// still be in that orbit afterwards. The hold is what the screenshots cannot show — a ship
    /// parked at a point rather than following one drifts out of frame within the hour.
    #[test]
    fn a_course_is_flown_and_then_held() {
        let mut session = Session::new(
            &lc_world::sky::hyg::HygProvider::load(
                "../../assets/catalogs/hygdata_v42_dist_sort.csv",
            )
            .expect("the catalogue"),
            64,
        );
        session.sync_system();
        assert!(session.system.is_some(), "the ship starts inside the solar system");

        let course =
            lc_world::navigation::Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: lc_world::navigation::Plane::Equatorial };
        let label = session.set_course(&course).expect("a course to Earth");
        assert_eq!(label, "orbit of Earth");
        assert!(session.cruise().is_some(), "and a crossing to fly it");

        // Fly. A tenth of a real second a step, which at the design rate is fifteen minutes.
        let mut steps = 0;
        while session.cruise().is_some() {
            session.advance(0.1);
            session.sync_system();
            steps += 1;
            assert!(steps < 10_000, "the crossing never ended");
        }

        // Nothing holds it but itself: arriving turned the crossing into the station it was
        // flown for, and `advance` reads the place every step. This used to need a helper here
        // that did by hand what the model does.
        assert!(session.station().is_some(), "arriving did not become holding");
        // Earth read at the same instant as the ship. Reading it out of the arena, which no
        // longer advances, put the "altitude" at six hundred planetary radii.
        let altitude = |session: &Session| {
            let now = session.coordinate_time_s();
            let earth =
                session.system.as_ref().unwrap().body_position_at("Earth", now).unwrap();
            session.ship.motion.position_ly.distance(earth) * lc_world::system::M_PER_LY / 6.371e6
        };
        assert!((altitude(&session) - 3.0).abs() < 0.05, "arrived at {}", altitude(&session));

        // And an hour later, with Earth thirty thousand kilometres further round its year.
        for _ in 0..40 {
            session.advance(0.1);
        }
        assert!((altitude(&session) - 3.0).abs() < 0.05, "drifted to {}", altitude(&session));
    }

/// The whole of it through the session: fly a course, cut the engine partway, and end up
    /// on a real orbit that is then held without thrust.
    #[test]
    fn cancelling_a_crossing_leaves_the_ship_on_a_conic() {
        let provider =
            lc_world::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv");
        let Ok(provider) = provider else { return };
        let mut session = Session::new(&provider, 64);
        session.sync_system();
        let course = lc_world::navigation::Course::Orbit {
            body: "Earth".into(),
            altitude_radii: 2.0,
            plane: lc_world::navigation::Plane::Equatorial,
        };
        session.set_course(&course).expect("a course");

        // Partway: far enough to be moving, not so far as to have arrived.
        for _ in 0..20 {
            session.advance(0.05);
        }
        assert!(session.cruise().is_some(), "still under way");
        let moving = session.velocity_m_s();
        assert!(moving.length() > 1.0e3, "only {} m/s", moving.length());

        let coast = session.cancel().expect("an arc");
        assert!(session.cruise().is_none() && session.station().is_none());
        eprintln!(
            "cut at {:.0} km/s -> {} (e {:.3})",
            moving.length() / 1.0e3,
            crate::hud::arc(&coast),
            coast.elements.eccentricity,
        );

        // And it keeps going, ballistically, without any of the three modes fighting.
        let before = session.ship.motion.position_ly;
        for _ in 0..20 {
            session.advance(0.05);
        }
        assert!(session.ship.motion.position_ly != before, "a coasting ship is not parked");
        assert!(session.coast().is_some(), "and it is still on an arc");
    }


    /// A sky with nothing in it but stars must meter exactly as a count percentile did: every
    /// point carries the same weight, so the solid angle divides out of both the samples and
    /// the total. This is what lets the star field keep its tuning.
    #[test]
    fn a_sky_of_points_meters_by_count_whatever_a_pixel_subtends() {
        let mut s = session();
        s.scene.point_sr = NOMINAL_POINT_SR;
        s.auto_expose();
        let coarse = s.tone.reference;
        s.scene.point_sr = NOMINAL_POINT_SR * 1000.0;
        s.auto_expose();
        assert_eq!(s.tone.reference, coarse, "a point reference cannot depend on the zoom");
        assert!(
            (s.tone.surface_reference * s.scene.point_sr / s.tone.reference - 1.0).abs() < 1e-5,
            "and the two references are one metering, a solid angle apart",
        );
    }

    /// The rule the area percentile buys: how much of the frame a body covers is what decides
    /// whether it is the subject. Two per cent of the metered sky is the line, because that is
    /// what `auto_expose` lets clip.
    #[test]
    fn a_body_takes_the_exposure_once_it_is_more_than_a_crowd_of_stars() {
        let mut s = session();
        let crowd = 200;
        let faint = PerBand::splat(1.0e-12f32);
        let bright = PerBand::splat(1.0e3f32);
        let meter = |s: &mut Session, sr: f32| {
            s.scene = Scene {
                point_sr: NOMINAL_POINT_SR,
                points: vec![faint; crowd],
                discs: vec![Disc { radiance: bright, solid_angle_sr: sr }],
            };
            s.auto_expose();
            s.tone.surface_reference
        };
        // The crossover: with `n` equal points, a disc outweighs the top two per cent of the
        // frame at `n * (1 / 0.98 - 1)` of their area.
        let edge = (crowd + s.stars.len()) as f32 * (1.0 / 0.98 - 1.0) * NOMINAL_POINT_SR;
        let small = meter(&mut s, edge * 0.5);
        let large = meter(&mut s, edge * 2.0);
        assert!(small < bright[Band::V], "a body under the line leaves the field exposed");
        assert!(large > small * 1.0e6, "and over it the body is what is exposed for");
    }

    /// Faint points are still points: a body that has not resolved yet is metered by the flux
    /// it delivers, so it cannot take the exposure by being large.
    #[test]
    fn an_unresolved_body_is_metered_as_a_point() {
        let mut s = session();
        s.scene = Scene {
            point_sr: NOMINAL_POINT_SR,
            points: vec![PerBand::splat(1.0e3f32)],
            discs: Vec::new(),
        };
        s.auto_expose();
        let one_bright_point = s.tone.reference;
        s.scene.points = vec![PerBand::splat(1.0e3f32); 3];
        s.auto_expose();
        assert!(s.tone.reference >= one_bright_point, "more bright points, not a brighter one");
        assert!(s.tone.reference.is_finite() && s.tone.reference > 0.0);
    }

    #[test]
    fn auto_exposure_puts_the_brightest_star_at_the_top_of_the_window() {
        let s = session();
        let sky = s.sky();
        assert!(sky.iter().any(|x| x.shaded.colour().length() > 0.0), "something must be visible");
        let brightest = sky
            .iter()
            .map(|x| x.shaded.colour().max_element())
            .fold(0.0f32, f32::max);
        assert!(brightest > 0.99, "the brightest should fill the window, got {brightest}");
    }

    /// The bug this exists for: a fixed reference of 1.0 is thirty stops above a star's
    /// actual band radiance, and the whole sky renders black.
    #[test]
    fn a_fixed_reference_would_render_nothing() {
        let mut s = session();
        s.tone = ToneMap::default();
        assert!(
            s.sky().iter().all(|x| x.shaded.colour() == Vec3::ZERO),
            "an unexposed scene is black, which is why auto_expose exists"
        );
        s.auto_expose();
        assert!(s.sky().iter().any(|x| x.shaded.colour().length() > 0.0));
    }

    #[test]
    fn pointing_somewhere_new_discards_the_old_curve() {
        let mut s = session();
        let (a, b) = (s.stars[0].id, s.stars[1].id);
        s.point_at(Some(a));
        s.observe(1e4);
        assert!(!s.curve.is_empty());
        s.point_at(Some(b));
        assert!(s.curve.is_empty(), "a curve belongs to one target");
    }

    #[test]
    fn observing_nothing_returns_nothing() {
        let mut s = session();
        s.point_at(None);
        assert!(s.observe(1.0).is_none());
    }

    #[test]
    fn a_modelled_system_accumulates_a_curve_over_time() {
        let mut s = session();
        s.telescope = s.telescope.with_aperture(1e4).with_bands(em_spectra::BandMask::ALL);
        s.point_at(Some(s.stars[0].id));
        for _ in 0..200 {
            s.advance(6.0);
            s.observe(1e3);
        }
        assert_eq!(s.curve.len(), 200);
        let (first, last) = s.curve.span().unwrap();
        assert!(last > first, "the curve should advance through emission time");
    }

    /// Enough real seconds to finish any crossing in the sample sky.
    const LONG_ENOUGH: f64 = 40_000.0;

    /// The sample provider puts every star on +X, which makes any test about direction
    /// vacuously true. This spreads the same stars over three axes.
    fn spread() -> Session {
        let template = AuthoredStars::sample().stars()[0].clone();
        let mut stars = Vec::new();
        for (k, axis) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().enumerate() {
            let mut star = template.clone();
            star.provenance.key = k as u64;
            star.id = lc_world::sky::StarId::synthesise("spread", k as u64);
            star.position_ly = axis * 4.2;
            stars.push(star);
        }
        Session::new(&AuthoredStars::new("spread", stars), 3)
    }

    #[test]
    fn flying_to_a_star_arrives_at_the_standoff_and_stops() {
        let mut s = session();
        let id = s.stars[0].id;
        let before = s.distance_to(s.star(id).unwrap());
        s.fly_to(id);
        s.advance(LONG_ENOUGH);
        let after = s.distance_to(s.star(id).unwrap());
        assert!(after < before, "{before} -> {after} ly");
        assert!((after - STANDOFF_LY).abs() < 1e-6, "stopped {after} ly out, wanted {STANDOFF_LY}");
        assert!(s.cruise().is_none(), "the crossing should have ended");
        assert_eq!(s.ship.motion.beta, DVec3::ZERO, "and the ship should be at rest");
    }

    /// The grid is what retarded time is solved against, so it has to follow the ship.
    #[test]
    fn the_observer_coordinate_tracks_the_ship() {
        let mut s = session();
        assert_eq!((s.observer.x, s.observer.y, s.observer.z), (0, 0, 0));
        s.fly_to(s.stars[0].id);
        s.advance(LONG_ENOUGH);
        let grid = DVec3::new(s.observer.x as f64, s.observer.y as f64, s.observer.z as f64);
        let want = s.ship.motion.position_ly * LUS_PER_LY;
        // One light-microsecond of rounding, on a number of order 1e14.
        assert!((grid - want).max_element() < 2.0, "{grid:?} vs {want:?}");
    }

    /// Light from the destination gets younger as the ship closes on it. The whole premise.
    #[test]
    fn the_light_from_the_destination_gets_fresher() {
        let mut s = session();
        let id = s.stars[0].id;
        let age = |s: &Session| s.sky().into_iter().find(|x| x.id == id).unwrap().light_age_s;
        let before = age(&s);
        s.fly_to(id);
        s.advance(LONG_ENOUGH);
        assert!(age(&s) < before / 100.0, "{} should be far under {before}", age(&s));
    }

    #[test]
    fn flying_toward_a_star_blueshifts_it_and_one_abeam_shifts_less() {
        let mut s = spread();
        let (ahead, abeam) = (s.stars[0].id, s.stars[1].id);
        s.fly_to(ahead);
        s.advance(8_000.0);
        assert!(s.ship.motion.beta.length() > 0.5, "should be moving fast, got {}", s.ship.motion.beta.length());
        let (front, side) = (s.doppler_to(s.star(ahead).unwrap()), s.doppler_to(s.star(abeam).unwrap()));
        assert!(front > 1.0, "the destination must blueshift, got {front}");
        assert!(side < front, "a star abeam must shift less than one dead ahead: {side} vs {front}");
    }

    /// Pins the rule the shift is implemented by: a blackbody seen with Doppler factor D is
    /// exactly a blackbody at D times the temperature.
    ///
    /// Checked in the radio band, where a star of a few thousand kelvin is deep in the
    /// Rayleigh-Jeans tail and the radiance is therefore linear in temperature. So the band
    /// must brighten by exactly D — not by D^4, which is the *bolometric* factor and would
    /// only show up in an integral over all frequencies, not in one narrow window.
    #[test]
    fn a_shifted_blackbody_is_a_blackbody_at_the_shifted_temperature() {
        let mut s = spread();
        let id = s.stars[0].id;
        let radio = |s: &Session| s.radiance_from(s.star(id).unwrap())[Band::Radio] as f64;
        let at_rest = radio(&s);
        let distance_before = s.distance_to(s.star(id).unwrap());

        s.fly_to(id);
        s.advance(8_000.0);
        let d = s.doppler_to(s.star(id).unwrap());
        assert!(d > 1.5, "want a real shift, got {d}");

        // Closing the distance brightens it as well; divide that out first.
        let closing = (distance_before / s.distance_to(s.star(id).unwrap())).powi(2);
        let ratio = radio(&s) / (at_rest * closing);
        assert!((ratio / d - 1.0).abs() < 1e-3, "radio band rose {ratio}x, wanted D = {d}");
    }

    /// The sky compresses toward the bow. At speed a star abeam appears ahead of abeam.
    #[test]
    fn the_sky_aberrates_forward() {
        let mut s = spread();
        let (ahead, other) = (s.stars[0].id, s.stars[1].id);
        s.fly_to(ahead);
        s.advance(8_000.0);
        let bow = s.ship.motion.beta.normalize();
        let star = s.sky().into_iter().find(|x| x.id == other).unwrap();
        let true_angle = star.offset_ly.normalize().dot(bow).acos();
        let seen_angle = star.apparent_dir.dot(bow).acos();
        assert!(seen_angle < true_angle, "{seen_angle} should be inside {true_angle}");
    }

    #[test]
    fn cutting_the_drive_keeps_the_velocity_it_had() {
        let mut s = session();
        s.fly_to(s.stars[0].id);
        s.advance(8_000.0);
        let at = s.ship.motion.position_ly;
        let beta = s.ship.motion.beta;
        assert!(beta.length() > 0.01, "the crossing should be up to speed");

        s.cancel();
        assert!(s.cruise().is_none() && s.station().is_none());
        // Not exactly: the velocity goes out through metres a second and comes back, and a
        // multiply by `c` followed by a divide by `c` is not the identity in binary.
        assert!((s.ship.motion.beta - beta).length() < beta.length() * 1e-12, "cutting the engine is a brake");

        // Between stars there is no conic to fall onto, so it is a straight line at the speed
        // it had. A light-year is a year at `c`, so the distance is the beta times the years.
        s.advance(8_000.0);
        let gone = s.ship.motion.position_ly - at;
        let expected = beta * (8_000.0 * TIME_RATE) / crate::flight::JULIAN_YEAR_S;
        assert!(
            (gone - expected).length() < expected.length() * 1e-9,
            "{gone:?} against {expected:?}",
        );
    }

    /// The bug this exists for: the ship clock was stepped as `elapsed / gamma` using the
    /// velocity at the end of each step, so its reading depended on the frame rate, and the
    /// final step -- taken after the ship had already stopped -- ran at full rate.
    #[test]
    fn the_ship_clock_does_not_depend_on_how_finely_time_is_stepped() {
        // Exactly to arrival and no further: time spent coasting afterwards runs at the
        // coordinate rate and would swamp what is being measured.
        let crossing = |steps: usize| {
            let mut s = session();
            s.fly_to(s.stars[0].id);
            let cruise = s.cruise().cloned().unwrap();
            let (want, real) = (cruise.proper_duration_s(), cruise.duration_s() / TIME_RATE);
            for _ in 0..steps {
                s.advance(real / steps as f64);
            }
            (s.ship.motion.clock_s, want)
        };
        let (coarse, want) = crossing(4);
        let (fine, _) = crossing(4000);
        assert!((coarse - fine).abs() < 1.0, "{coarse} against {fine} seconds");
        // And both must agree with the closed form, not merely with each other.
        assert!((coarse - want).abs() / want < 1e-6, "{coarse} against {want}");
    }

    #[test]
    fn a_second_crossing_carries_on_from_the_first() {
        let mut s = spread();
        s.fly_to(s.stars[0].id);
        s.advance(LONG_ENOUGH);
        let after_one = s.ship.motion.clock_s;
        assert!(after_one > 0.0);
        s.fly_to(s.stars[1].id);
        s.advance(LONG_ENOUGH);
        assert!(s.ship.motion.clock_s > after_one, "the clock must not restart at zero");
    }

    #[test]
    fn a_ship_that_never_flies_keeps_the_coordinate_clock() {
        let mut s = session();
        s.advance(3600.0);
        assert!((s.ship.motion.clock_s - s.coordinate_time_s()).abs() < 1e-6);
    }

    #[test]
    fn flying_somewhere_that_is_not_in_the_sky_does_nothing() {
        let mut s = session();
        assert!(s.fly_to(lc_world::sky::StarId::synthesise("absent", 1)).is_none());
        assert!(s.cruise().is_none());
    }

    #[test]
    fn switching_the_band_mapping_changes_what_is_drawn() {
        let mut s = session();
        let natural: Vec<Vec3> = s.sky().iter().map(|x| x.shaded.colour()).collect();
        s.mapping = presets::thermal();
        // The window follows the mapping: a different band is a different brightness.
        s.auto_expose();
        let thermal: Vec<Vec3> = s.sky().iter().map(|x| x.shaded.colour()).collect();
        assert_ne!(natural, thermal, "the preset must reach the picture");
    }
}
