//! Running a telescope: turning a duty and elapsed time into records.
//!
//! Engine-free so a shard runs it for every craft every tick, flown or not. A client runs it
//! only with no shard: a test or a headless snapshot. See
//! `lightcone/docs/24-standing-instruments.md`.

use std::collections::HashMap;
use std::sync::Arc;

use em_spectra::Band;
use glam::DVec3;
use lc_spacetime::{Coord, Micros, frame::SystemFrame};

use super::survey::{self, Duty, Optics, Source, Sweep};
use crate::system::LocalSystem;
use super::{Bearing, Claim, Distance, Hop, Knowledge, NameKind, Naming, Sample, Sighting, Subject, Witness};
use crate::instrument::Instrument;
use crate::observation::{Target, observe};
use crate::rng;
use crate::sky::{CatalogueStar, StarId, generate};
use crate::system::M_PER_LY;

/// Who the charts a ship launches with came from: not this ship, nor any craft it will meet.
pub const CHARTS: Witness = Witness(u64::MAX);

/// Fractional error on a charted distance: small, and still visible as scatter on the map.
pub const CHART_ERROR: f64 = 0.01;

/// Light-years.
pub const CHARTED_LY: f64 = 20.0;

/// Watch turns measured in one tick at most.
const TURNS_PER_TICK: i64 = 64;

const LUS_PER_LY: f64 = M_PER_LY / 299.792458;

/// Nothing cached here depends on who is looking, so one serves every craft a shard runs.
pub struct Sky {
    stars: Arc<Vec<CatalogueStar>>,
    /// Watts, in the order of `stars`.
    luminosity: HashMap<Band, Vec<f64>>,
    targets: HashMap<StarId, Target>,
}

impl Sky {
    pub fn new(stars: Arc<Vec<CatalogueStar>>) -> Self {
        Self { stars, luminosity: HashMap::new(), targets: HashMap::new() }
    }

    pub fn stars(&self) -> &[CatalogueStar] {
        &self.stars
    }

    /// In the order of [`Sky::stars`]: a detection is indexed into this, and the glare test
    /// needs the whole sky.
    pub fn sources(&mut self, band: Band, here: DVec3) -> Vec<Source> {
        let stars = &self.stars;
        let luminosity = self.luminosity.entry(band).or_insert_with(|| {
            stars
                .iter()
                .map(|star| {
                    let unit = survey::flux_from(&star.star, band, M_PER_LY);
                    4.0 * std::f64::consts::PI * M_PER_LY * M_PER_LY * unit
                })
                .collect()
        });
        stars
            .iter()
            .zip(luminosity.iter())
            .map(|(star, luminosity)| {
                let offset = star.position_ly - here;
                let distance_m = offset.length() * M_PER_LY;
                let flux = if distance_m > 0.0 {
                    luminosity / (4.0 * std::f64::consts::PI * distance_m * distance_m)
                } else {
                    0.0
                };
                Source {
                    subject: Subject::Star(star.id),
                    toward: offset.normalize_or_zero(),
                    flux_w_m2: flux,
                    // A star is a point at any interstellar range and a disc from inside its
                    // own system, which is the ship's own sun and nothing else.
                    diameter_rad: if distance_m > 0.0 {
                        2.0 * star.star.radius_m / distance_m
                    } else {
                        0.0
                    },
                    radius_m: star.star.radius_m,
                    spin_s: None,
                }
            })
            .collect()
    }

    /// Built the first time anything points at it.
    pub fn target(&mut self, id: StarId) -> Option<&Target> {
        if !self.targets.contains_key(&id) {
            let star = self.stars.iter().find(|s| s.id == id)?;
            self.targets.insert(id, build_target(star));
        }
        self.targets.get(&id)
    }
}

pub fn build_target(star: &CatalogueStar) -> Target {
    let system = generate::system_for(star);
    let mut model = crate::emission::EmissionModel::new(star.star, star.seed());
    model.populations = system.populations;
    for planet in &system.planets {
        model.bodies.push(crate::emission::Body {
            occluder: crate::occluder::Occluder::new(planet.radius_m),
            motion: Box::new(crate::emission::CircularOrbit {
                radius_m: planet.semi_major_m,
                pole: system.pole,
                phase0: planet.mean_anomaly_deg.to_radians(),
                mu: star.star.mu,
            }),
        });
    }
    let at = star.position_ly * LUS_PER_LY;
    let origin =
        Coord::new(Micros::ORIGIN, at.x as i64, at.y as i64, at.z as i64).unwrap_or(Coord::ORIGIN);
    Target::new(SystemFrame::new(origin), model)
}

#[derive(Clone, Copy, Debug)]
pub struct Station {
    pub position_ly: DVec3,
    pub instrument: Instrument,
}

impl Station {
    fn observer(&self, now_s: f64) -> Coord {
        let at = self.position_ly * LUS_PER_LY;
        Coord::new(Micros::new((now_s * 1.0e6) as i64), at.x as i64, at.y as i64, at.z as i64)
            .unwrap_or(Coord::ORIGIN)
    }

    pub fn optics(&self) -> Optics {
        Optics::of(self.instrument)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Observatory {
    pub duty: Duty,
    /// Coordinate seconds.
    pub integration_s: f64,
    /// Last photometric sample and last sweep check: how elapsed time becomes exposure rather
    /// than one measurement per tick.
    sampled_s: f64,
    swept_s: f64,
    slot: i64,
    pointing: Option<StarId>,
}

impl Default for Observatory {
    fn default() -> Self {
        Self {
            duty: Duty::Idle,
            integration_s: 1.0e4,
            sampled_s: 0.0,
            swept_s: 0.0,
            slot: i64::MIN,
            pointing: None,
        }
    }
}

impl Observatory {
    pub fn pointing(&self) -> Option<StarId> {
        self.pointing
    }

    /// A sweep or a watch starts at `now_s`, whatever start it was handed.
    pub fn take_up(&mut self, mut duty: Duty, now_s: f64) {
        match &mut duty {
            Duty::Sweep(sweep) => sweep.started_s = now_s,
            Duty::Watch { started_s, .. } | Duty::Survey { started_s, .. } => *started_s = now_s,
            _ => {}
        }
        self.sampled_s = now_s;
        self.swept_s = now_s;
        self.slot = i64::MIN;
        self.pointing = duty.target_at(now_s);
        self.duty = duty;
    }

    /// Exposure is elapsed coordinate time, so no measurement claims an integration it did not
    /// get.
    ///
    /// `system` is the one the craft is inside, `None` between the stars. Optional for the same
    /// reason `motion::state_at` takes it that way: a survey of a system nobody has loaded has
    /// no bodies to point at and no answer to give, and the other duties never look at it.
    pub fn tick(
        &mut self,
        sky: &mut Sky,
        system: Option<&LocalSystem>,
        knowledge: &mut Knowledge,
        at: Station,
        now_s: f64,
    ) {
        match self.duty.clone() {
            Duty::Idle => {}
            Duty::Stare(id) => {
                let elapsed = now_s - self.sampled_s;
                if elapsed >= self.integration_s.max(1.0) {
                    self.pointing = Some(id);
                    photometry(sky, knowledge, at, id, elapsed, now_s);
                    fix(sky, knowledge, at, id, elapsed, now_s);
                    self.sampled_s = now_s;
                }
            }
            duty @ Duty::Watch { .. } => {
                let Duty::Watch { dwell_s, started_s, .. } = duty else { return };
                let dwell = dwell_s.max(1.0);
                let Some(slot) = duty.slot_at(now_s) else { return };
                let previous = std::mem::replace(&mut self.slot, slot);
                // Every turn that ended since the last tick, each measured when it ended; on the
                // first tick no turn has a full dwell behind it. Capped, so a long gap is bounded.
                if previous != i64::MIN {
                    for turn in previous.max(slot - TURNS_PER_TICK)..slot {
                        let ended_s = started_s + (turn + 1) as f64 * dwell;
                        if let Some(id) = duty.target_at(ended_s - dwell * 0.5) {
                            photometry(sky, knowledge, at, id, dwell, ended_s);
                            fix(sky, knowledge, at, id, dwell, ended_s);
                        }
                    }
                }
                self.pointing = duty.target_at(now_s);
            }
            Duty::Sweep(sweep) => {
                self.pointing = None;
                sweep_between(sky, knowledge, at, &sweep, self.swept_s, now_s);
                self.swept_s = now_s;
            }
            duty @ Duty::Survey { star, .. } => {
                self.pointing = Some(star);
                if let Some(system) = system.filter(|s| s.star == star) {
                    survey_between(sky, system, knowledge, at, &duty, self.swept_s, now_s);
                }
                self.swept_s = now_s;
            }
        }
    }
}

/// One tick of a survey: the bodies whose turn it is, measured against a sky that holds the
/// system's own star as well as its bodies.
///
/// The star belongs in that list and not beside it. It is the brightest thing by nine orders of
/// magnitude, so it is the only meaningful glare in the system, and `survey::look` can only be
/// asked about it if it is a source like any other. The rest of the catalogue is left out: a
/// star light-years off cannot outshine a planet at 5 AU, so it can neither glare on one nor
/// hide behind one.
pub fn survey_between(
    sky: &mut Sky,
    system: &LocalSystem,
    knowledge: &mut Knowledge,
    at: Station,
    duty: &Duty,
    from_s: f64,
    to_s: f64,
) {
    let optics = at.optics();
    let Some(band) = optics.band() else { return };
    let mut sources = host_source(sky, system, band, at.position_ly)
        .into_iter()
        .collect::<Vec<Source>>();
    let star_last = sources.len();
    sources.extend(crate::visit::sources(system, band, at.position_ly, to_s));
    // Brightest first over the bodies only, so the star keeps its place at the front and the
    // rotation below is over a list whose order is a property of the system.
    sources[star_last..].sort_unstable_by(|a, b| b.flux_w_m2.total_cmp(&a.flux_w_m2));

    let bodies = sources.len() - star_last;
    let witness = knowledge.owner;

    // The star every tick, and not as a turn in the rotation. It is not a target of the survey;
    // it is the reference the survey is measured against, in every frame because it is the
    // brightest thing in the sky and what the phase angle of everything else is reckoned from.
    // So it costs no dwell of its own, and its parallax accumulates with the ship's motion --
    // without which nothing here has a distance and no mass prior runs.
    if star_last > 0
        && let Some(seen) = survey::look(
            &optics,
            &sources,
            0,
            survey::SURVEY_DWELL_S,
            at.position_ly,
            to_s,
            witness,
        )
    {
        knowledge.sighted(Subject::Star(system.star), seen);
    }

    for slot in duty.visits(bodies, from_s, to_s) {
        let index = star_last + slot;
        if let Some(seen) =
            survey::look(&optics, &sources, index, survey::SURVEY_DWELL_S, at.position_ly, to_s, witness)
            && let Some(source) = sources.get(index)
        {
            knowledge.sighted(source.subject, seen);
        }
    }
}

/// The system's own star as a source, worked the way the catalogue path works it so the two
/// cannot disagree about how bright the ship's own sun is.
fn host_source(sky: &mut Sky, system: &LocalSystem, band: Band, from: DVec3) -> Option<Source> {
    let index = sky.stars().iter().position(|s| s.id == system.star)?;
    sky.sources(band, from).get(index).copied()
}

/// Noise is seeded from the witness and the star, so two craft see different noise and a shard
/// can recompute exactly what either saw.
pub fn photometry(
    sky: &mut Sky,
    knowledge: &mut Knowledge,
    at: Station,
    id: StarId,
    exposure_s: f64,
    now_s: f64,
) {
    let witness = knowledge.owner;
    let Some(target) = sky.target(id) else { return };
    let seed = rng::hash(&[witness.0, id.get()]);
    let Some(obs) = observe(target, at.observer(now_s), &at.instrument, exposure_s, seed) else {
        return;
    };
    for band in Band::ALL {
        let Some(m) = obs.band(band) else { continue };
        let sample = Sample { observed_s: now_s, deficit: m.measured_deficit, sigma: m.uncertainty };
        knowledge.measured(id, witness, band, sample);
    }
}

/// A bearing to a star the instrument is already pointed at.
pub fn fix(sky: &mut Sky, knowledge: &mut Knowledge, at: Station, id: StarId, exposure_s: f64, now_s: f64) {
    let optics = at.optics();
    let Some(band) = optics.band() else { return };
    let sources = sky.sources(band, at.position_ly);
    let Some(index) = sources.iter().position(|s| s.subject == Subject::Star(id)) else { return };
    let witness = knowledge.owner;
    if let Some(seen) =
        survey::look(&optics, &sources, index, exposure_s, at.position_ly, now_s, witness)
    {
        knowledge.sighted(id, seen);
    }
}

pub fn sweep_between(
    sky: &mut Sky,
    knowledge: &mut Knowledge,
    at: Station,
    sweep: &Sweep,
    from_s: f64,
    to_s: f64,
) {
    let optics = at.optics();
    let Some(band) = optics.band() else { return };
    let plan = sweep.plan();
    let due: Vec<(usize, f64)> = sky
        .stars()
        .iter()
        .enumerate()
        .filter_map(|(i, star)| {
            let toward = (star.position_ly - at.position_ly).normalize_or_zero();
            let passes = plan.observed_between(toward, from_s, to_s);
            (!passes.is_empty()).then(|| passes.into_iter().map(move |when| (i, when)))
        })
        .flatten()
        .collect();
    if due.is_empty() {
        return;
    }
    let sources = sky.sources(band, at.position_ly);
    let witness = knowledge.owner;
    for (index, when) in due {
        if let Some(seen) =
            survey::look(&optics, &sources, index, sweep.exposure_s(), at.position_ly, when, witness)
            && let Some(source) = sources.get(index)
        {
            knowledge.sighted(source.subject, seen);
        }
    }
}

/// The charting office's designation for a star, the same for every ship. The catalogue name is
/// the generator's and is never shown.
pub fn chart_number(id: StarId) -> String {
    let raw = id.get();
    format!("HC {:04X}-{:02X}", raw >> 48, (raw >> 40) & 0xFF)
}

/// The charts a ship leaves port with: claims from [`CHARTS`] out to `reach_ly`, held on the
/// office's word until the ship measures a star itself.
pub fn issue_charts(sky: &mut Sky, knowledge: &mut Knowledge, at: Station, reach_ly: f64, now_s: f64) {
    let Some(band) = at.optics().band() else { return };
    let from = at.position_ly;
    let hop = Hop { from: CHARTS, to: knowledge.owner, sent_s: now_s, received_s: now_s };
    let sources = sky.sources(band, from);
    for (star, source) in sky.stars().iter().zip(sources.iter()) {
        let offset = star.position_ly - from;
        let distance = offset.length();
        if distance > reach_ly || distance <= 0.0 {
            continue;
        }
        // A catalogue distance is a parallax like any other, so its error grows with range.
        let sigma_ly = CHART_ERROR * distance;
        let slip = rng::gaussian(rng::hash(&[star.id.get(), 0x0_c_4_a_7]));
        knowledge.sighted(
            star.id,
            Sighting {
                witness: CHARTS,
                observed_s: now_s,
                bearing: Bearing { observer_ly: from, toward: source.toward, sigma_rad: sigma_ly / distance },
                // A chart gives a place and a brightness and nothing a close look would.
                size: None,
                range_m: None,
                spin_s: None,
                band,
                flux: source.flux_w_m2,
                flux_sigma: source.flux_w_m2 * CHART_ERROR,
                lineage: vec![hop],
            },
        );
        knowledge.named(
            star.id,
            Naming {
                witness: CHARTS,
                name: chart_number(star.id),
                kind: NameKind::Designation,
                stated_s: now_s,
                lineage: vec![hop],
            },
        );
        knowledge.told(
            star.id,
            Claim {
                witness: CHARTS,
                distance: Distance::Measured {
                    position_ly: from + offset.normalize() * (distance + slip * sigma_ly),
                    sigma_ly,
                },
                stated_s: now_s,
                lineage: vec![hop],
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sky::{AuthoredStars, StarProvider};

    const YEAR_S: f64 = crate::flight::JULIAN_YEAR_S;
    const AU_M: f64 = 1.495_978_707e11;
    /// Coordinate seconds in one tick at the design rate: 50 ms of real time at 8766x.
    const TICK_S: f64 = 438.3;

    /// Three stars at 4.2 ly, one along each axis, so none hides behind another.
    fn spread() -> Sky {
        let template = AuthoredStars::sample().stars()[0].clone();
        let stars = [DVec3::X, DVec3::Y, DVec3::Z]
            .into_iter()
            .enumerate()
            .map(|(k, axis)| {
                let mut star = template.clone();
                star.id = StarId::synthesise("spread", k as u64);
                star.position_ly = axis * 4.2;
                star
            })
            .collect();
        Sky::new(Arc::new(stars))
    }

    fn at(position_ly: DVec3) -> Station {
        Station { position_ly, instrument: Instrument::SHIP }
    }

    /// Sol loaded, and a sky with only its own star in it.
    fn sol() -> Option<(Sky, LocalSystem)> {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .ok()?;
        let sun = provider
            .stars()
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some(crate::system::SOL))?
            .clone();
        let system = LocalSystem::for_star(&sun)?;
        Some((Sky::new(Arc::new(vec![sun])), system))
    }

    /// **What phase 6 is for.** A ship five AU out, told to survey its own system, comes to
    /// hold its planets: each a subject with a bearing, a brightness and a measured disc, under
    /// the same `BodyId` a navigation order would name.
    ///
    /// The claim pinned is the done-when's: *every one of them has a position within the first
    /// real second*. At the design rate that is twenty ticks of 438 coordinate seconds.
    #[test]
    fn a_survey_finds_the_planets_of_the_system_it_is_in() {
        let Some((mut sky, system)) = sol() else { return };
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory::default();
        let from = system.star_position_ly() + DVec3::X * 5.0 * AU_M / M_PER_LY;
        o.take_up(Duty::Survey { star: system.star, started_s: 0.0 }, 0.0);
        assert_eq!(o.duty.label(), "surveying");

        // One tick brings round seven bodies, brightest first, plus the star, which is measured
        // every tick rather than taking a turn. The first body is the brightest thing in the
        // system: Jupiter, from here.
        o.tick(&mut sky, Some(&system), &mut k, at(from), TICK_S);
        let first = k.len();
        assert_eq!(first, 8, "seven turns of SURVEY_DWELL_S, and the star besides");
        assert!(k.belief(Subject::Star(system.star)).is_some(), "the star is measured first of all");
        let jupiter = Subject::Body {
            star: system.star,
            body: crate::knowledge::BodyId::of(system.star, "Jupiter"),
        };
        assert!(k.file(jupiter).is_some(), "the brightest body comes round first");

        // A real second, at which point the done-when wants every major planet placed.
        for step in 2..=20 {
            o.tick(&mut sky, Some(&system), &mut k, at(from), TICK_S * step as f64);
        }
        for name in ["Venus", "Earth", "Mars", "Jupiter", "Saturn"] {
            let subject = Subject::Body {
                star: system.star,
                body: crate::knowledge::BodyId::of(system.star, name),
            };
            let file = k.file(subject).unwrap_or_else(|| panic!("{name} has no position yet"));
            let seen = file.sightings().first().unwrap_or_else(|| panic!("{name} has no sighting"));
            let (diameter, sigma) = seen.size.unwrap_or_else(|| panic!("{name} has no disc"));
            assert!(diameter > 0.0 && sigma > 0.0 && sigma < diameter);
        }

        // Mars is not among the first seven, and that is the physics rather than a fault: from
        // five AU the Galilean moons and Titan are all brighter than it is.
        let held = k.len();
        assert!(held > 100, "only {held} subjects after a real second");
        // Everything but the star itself is a body, and every one of them reads back.
        assert_eq!(k.bodies_of(system.star, TICK_S * 20.0).len(), held - 1, "a body did not read back");
    }

    /// **A moving ship's bearings on a moving planet are not a distance.** `triangulate` fits a
    /// static point to whatever it is given, and a body's bearings are all taken from inside its
    /// own system where it moves appreciably between them. Before this was guarded, a ship on a
    /// 5 AU orbit surveying Sol put Jupiter at 1.63 AU plus or minus 9e-7 -- sixteen million
    /// sigma from where it was, which is worse than no answer.
    #[test]
    fn a_body_never_gets_a_distance_from_being_watched_move() {
        let Some((mut sky, system)) = sol() else { return };
        let mut k = Knowledge::new(Witness(2));
        let mut o = Observatory::default();
        o.take_up(Duty::Survey { star: system.star, started_s: 0.0 }, 0.0);

        // A circular 5 AU orbit about a solar mass, which is what `Course::Orbit` would fly.
        let period_s = std::f64::consts::TAU * ((5.0 * AU_M).powi(3) / 1.327e20f64).sqrt();
        let orbit = |t: f64| {
            let phase = std::f64::consts::TAU * t / period_s;
            system.star_position_ly()
                + DVec3::new(phase.cos(), phase.sin(), 0.0) * 5.0 * AU_M / M_PER_LY
        };
        for step in 1..=120 {
            let t = TICK_S * step as f64;
            o.tick(&mut sky, Some(&system), &mut k, at(orbit(t)), t);
        }

        let mut checked = 0;
        for name in ["Venus", "Earth", "Mars", "Jupiter", "Saturn"] {
            let subject = Subject::Body {
                star: system.star,
                body: crate::knowledge::BodyId::of(system.star, name),
            };
            let Some(belief) = k.belief(subject) else { continue };
            assert!(belief.sightings > 1, "{name} was only seen once");
            assert_eq!(
                belief.distance,
                crate::knowledge::Distance::Unknown,
                "{name} was given a distance by watching it move"
            );
            assert!(!belief.triangulated);
            checked += 1;
        }
        assert_eq!(checked, 5, "only {checked} planets came round twice in 120 ticks");

        // The ship's own sun, from the same bearings, is measured: it is the one thing in the
        // system that holds still, which is the whole difference.
        let host = k.belief(Subject::Star(system.star)).expect("the sun was surveyed too");
        assert!(host.triangulated, "{:?}", host.distance);
    }

    /// A survey of a system the craft has not been handed does nothing rather than inventing
    /// it, which is the same reason `motion::state_at` takes its system as an `Option`.
    #[test]
    fn a_survey_without_its_system_learns_nothing() {
        let Some((mut sky, system)) = sol() else { return };
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory::default();
        let from = system.star_position_ly() + DVec3::X * 5.0 * AU_M / M_PER_LY;
        o.take_up(Duty::Survey { star: system.star, started_s: 0.0 }, 0.0);
        for step in 1..=8 {
            o.tick(&mut sky, None, &mut k, at(from), 438.3 * step as f64);
        }
        assert!(k.is_empty(), "it found {} bodies out of nothing", k.len());
    }

    /// A survey takes its start from when it was told, not from the number in the order: an
    /// instrument cannot have begun before it was ordered to.
    #[test]
    fn a_survey_starts_when_it_is_taken_up() {
        let mut o = Observatory::default();
        o.take_up(Duty::Survey { star: StarId::synthesise("t", 1), started_s: -1.0e9 }, 400.0);
        let Duty::Survey { started_s, .. } = o.duty else { panic!("the duty did not take") };
        assert_eq!(started_s, 400.0);
    }

    #[test]
    fn an_idle_instrument_learns_nothing() {
        let mut sky = spread();
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory::default();
        o.tick(&mut sky, None, &mut k, at(DVec3::ZERO), YEAR_S);
        assert!(k.is_empty());
    }

    #[test]
    fn a_sweep_finds_stars_field_by_field() {
        let mut sky = spread();
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory::default();
        o.take_up(Duty::Sweep(Sweep::all_sky(0.0)), 0.0);
        let pass = Sweep::all_sky(0.0).pass_s();
        let mut found = Vec::new();
        for step in 1..=40 {
            o.tick(&mut sky, None, &mut k, at(DVec3::ZERO), pass * step as f64 / 20.0);
            found.push(k.len());
        }
        assert_eq!(k.len(), 3, "two passes reach every field");
        assert!(found.windows(2).all(|w| w[1] >= w[0]), "a sweep only ever adds");
        assert!(found[..10].iter().any(|n| *n < 3), "and it takes time to get there");
    }

    /// A duty taken up now starts now, whatever start it was handed.
    #[test]
    fn a_sweep_starts_when_it_is_taken_up() {
        let mut o = Observatory::default();
        o.take_up(Duty::Sweep(Sweep::all_sky(-5.0e9)), 100.0);
        assert_eq!(o.duty.sweep().unwrap().started_s, 100.0);
    }

    #[test]
    fn a_stare_records_photometry_and_a_bearing_once_its_integration_is_done() {
        let mut sky = spread();
        let id = sky.stars()[0].id;
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory { integration_s: 1.0e4, ..Default::default() };
        o.take_up(Duty::Stare(id), 0.0);
        o.tick(&mut sky, None, &mut k, at(DVec3::ZERO), 5.0e3);
        assert!(k.own_series(id, Band::V).is_none(), "half an integration is not a sample");
        o.tick(&mut sky, None, &mut k, at(DVec3::ZERO), 1.0e4);
        assert_eq!(k.own_series(id, Band::V).unwrap().len(), 1);
        assert_eq!(k.belief(id).unwrap().sightings, 1);
        assert_eq!(o.pointing(), Some(id));
    }

    /// One place gives a direction; moving between looks gives a distance.
    #[test]
    fn a_stare_from_a_moving_ship_measures_a_distance() {
        let mut sky = spread();
        let id = sky.stars()[0].id;
        let truth = sky.stars()[0].position_ly;
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory::default();
        o.take_up(Duty::Stare(id), 0.0);
        for step in 1..=6 {
            let t = step as f64 * 2.0e4;
            o.tick(&mut sky, None, &mut k, at(DVec3::Z * 0.02 * step as f64), t);
        }
        let belief = k.belief(id).unwrap();
        assert!(belief.triangulated, "{:?}", belief.distance);
        assert!(belief.distance.position_ly().unwrap().distance(truth) < 0.1);
    }

    #[test]
    fn a_watch_samples_each_of_its_targets_in_turn() {
        let mut sky = spread();
        let ids: Vec<StarId> = sky.stars().iter().map(|s| s.id).collect();
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory::default();
        o.take_up(Duty::Watch { targets: ids.clone(), dwell_s: 4000.0, started_s: 0.0 }, 0.0);
        for step in 1..=30 {
            o.tick(&mut sky, None, &mut k, at(DVec3::ZERO), step as f64 * 4000.0);
        }
        for id in &ids {
            assert!(k.own_series(*id, Band::V).is_some_and(|s| !s.is_empty()));
        }
    }

    /// Ticked once after nine turns, a watch measures every turn, each stamped when it ended.
    #[test]
    fn a_watch_measures_every_turn_that_ended_across_a_gap() {
        let mut sky = spread();
        let ids: Vec<StarId> = sky.stars().iter().map(|s| s.id).collect();
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory::default();
        let dwell = 4000.0;
        o.take_up(Duty::Watch { targets: ids.clone(), dwell_s: dwell, started_s: YEAR_S }, YEAR_S);
        o.tick(&mut sky, None, &mut k, at(DVec3::ZERO), YEAR_S + 1.0);
        o.tick(&mut sky, None, &mut k, at(DVec3::ZERO), YEAR_S + 9.5 * dwell);
        for (n, id) in ids.iter().enumerate() {
            let times: Vec<f64> = k.own_series(*id, Band::V).unwrap().samples().iter().map(|s| s.observed_s).collect();
            let expected: Vec<f64> = (0..9).filter(|t| t % 3 == n).map(|t| YEAR_S + (t + 1) as f64 * dwell).collect();
            assert_eq!(times, expected, "target {n}");
        }
    }

    /// Different witnesses see different noise; the same witness sees the same, so a shard can
    /// check a claim.
    #[test]
    fn noise_belongs_to_the_witness() {
        let run = |witness: u64| {
            let mut sky = spread();
            let id = sky.stars()[0].id;
            let mut k = Knowledge::new(Witness(witness));
            photometry(&mut sky, &mut k, at(DVec3::ZERO), id, 1.0e4, 1.0e4);
            k.own_series(id, Band::V).unwrap().samples()[0].deficit
        };
        assert_eq!(run(1), run(1));
        assert_ne!(run(1), run(2));
    }

    #[test]
    fn charts_cover_what_they_cover_and_no_further() {
        let mut sky = spread();
        let mut k = Knowledge::new(Witness(1));
        issue_charts(&mut sky, &mut k, at(DVec3::ZERO), 5.0, 0.0);
        assert_eq!(k.len(), 3);
        let id = sky.stars()[0].id;
        let belief = k.belief(id).unwrap();
        assert!(!belief.triangulated, "a chart is somebody's word");
        assert_eq!(belief.hops, 1);
        let mut none = Knowledge::new(Witness(1));
        issue_charts(&mut sky, &mut none, at(DVec3::ZERO), 1.0, 0.0);
        assert!(none.is_empty());
    }
}
