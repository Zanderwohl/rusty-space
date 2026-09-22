//! Running a telescope: turning a duty and elapsed time into records.
//!
//! Engine-free so a shard runs it for every craft every tick, whether or not anybody is flying
//! them — the universe runs when nobody is watching. A client runs the same code only when it
//! has no shard, which is a test or a headless snapshot. See
//! `lightcone/docs/24-standing-instruments.md`.

use std::collections::HashMap;
use std::sync::Arc;

use em_spectra::Band;
use glam::DVec3;
use lc_spacetime::{Coord, Micros, frame::SystemFrame};

use super::survey::{self, Duty, Optics, Source, Sweep};
use super::{Bearing, Claim, Distance, Hop, Knowledge, NameKind, Naming, Sample, Sighting, Witness};
use crate::instrument::Instrument;
use crate::observation::{Target, observe};
use crate::rng;
use crate::sky::{CatalogueStar, StarId, generate};
use crate::system::M_PER_LY;

/// Who the charts a ship launches with came from. Not this ship, and not any craft it will ever
/// meet: a name for "somebody else measured this and we are taking their word".
pub const CHARTS: Witness = Witness(u64::MAX);

/// Fractional error on a charted distance: a percent at the edge of the charted volume, which is
/// a good office and still visible as a scatter on the map.
pub const CHART_ERROR: f64 = 0.01;

/// How far the charts a ship starts with reach, light-years. Past this, the sky is unsurveyed.
pub const CHARTED_LY: f64 = 20.0;

/// Light-microseconds per light-year, for putting an observer on the grid.
const LUS_PER_LY: f64 = M_PER_LY / 299.792458;

/// The sky an instrument can point at, and what is cached about it.
///
/// Shared: a star's band luminosity and its emission model do not depend on who is looking, so
/// one of these serves every craft a shard runs.
pub struct Sky {
    stars: Arc<Vec<CatalogueStar>>,
    /// Band luminosity per star, watts, in the order of `stars`. Only the distance changes with
    /// the observer, so the rest is computed once and divided by `r^2`.
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

    /// Every star as a point source arriving at `here`, in `band`, in the order of
    /// [`Sky::stars`] — a detection is indexed into this, and the glare test needs the whole sky.
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
                Source { star: star.id, toward: offset.normalize_or_zero(), flux_w_m2: flux }
            })
            .collect()
    }

    /// A star's emission model, built the first time anything points at it.
    pub fn target(&mut self, id: StarId) -> Option<&Target> {
        if !self.targets.contains_key(&id) {
            let star = self.stars.iter().find(|s| s.id == id)?;
            self.targets.insert(id, build_target(star));
        }
        self.targets.get(&id)
    }
}

/// A star's system as the photometry sees it: the star, its planets as occluders, and its
/// populations.
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

/// Where an instrument is and what it is.
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

/// What one instrument is committed to, and how far through it it is.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Observatory {
    pub duty: Duty,
    /// Coordinate seconds a stare integrates for before it records a sample.
    pub integration_s: f64,
    /// When the last photometric sample was taken, and when the sweep was last asked what it
    /// had covered. How a duty turns elapsed time into exposure rather than producing one
    /// measurement per tick.
    sampled_s: f64,
    swept_s: f64,
    /// Which turn of a watch rotation the last sample came from.
    slot: i64,
    /// Where it is pointed right now.
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

    /// Take up a duty, starting its clock at `now_s`. A sweep or a watch starts where it is
    /// taken up, whatever start time it was handed: the instrument cannot have begun earlier.
    pub fn take_up(&mut self, mut duty: Duty, now_s: f64) {
        match &mut duty {
            Duty::Sweep(sweep) => sweep.started_s = now_s,
            Duty::Watch { started_s, .. } => *started_s = now_s,
            _ => {}
        }
        self.sampled_s = now_s;
        self.swept_s = now_s;
        self.slot = i64::MIN;
        self.pointing = duty.target_at(now_s);
        self.duty = duty;
    }

    /// Run the duty for however much coordinate time has passed since the last call.
    ///
    /// Exposure is elapsed coordinate time, not a number typed into a panel: a measurement
    /// labeled with an integration it did not get is a lie about its own error bars.
    pub fn tick(&mut self, sky: &mut Sky, knowledge: &mut Knowledge, at: Station, now_s: f64) {
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
                let Duty::Watch { dwell_s, .. } = duty else { return };
                let Some(slot) = duty.slot_at(now_s) else { return };
                let previous = std::mem::replace(&mut self.slot, slot);
                // The turn that just ended is the only one with a full dwell behind it, and on
                // the first tick there is no such turn.
                if previous != slot
                    && previous != i64::MIN
                    && let Some(id) = duty.target_at(now_s - dwell_s.max(1.0))
                {
                    photometry(sky, knowledge, at, id, dwell_s, now_s);
                    fix(sky, knowledge, at, id, dwell_s, now_s);
                }
                self.pointing = duty.target_at(now_s);
            }
            Duty::Sweep(sweep) => {
                self.pointing = None;
                sweep_between(sky, knowledge, at, &sweep, self.swept_s, now_s);
                self.swept_s = now_s;
            }
        }
    }
}

/// One photometric sample of a star, in every band the instrument has.
///
/// Noise is seeded from the witness and the star as well as the time, so two craft watching
/// the same star see different noise and a shard can recompute exactly what either saw.
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
///
/// A stare measures where something is as well as how bright it is, which is why watching one
/// star from a moving ship eventually gives its distance without any survey at all.
pub fn fix(sky: &mut Sky, knowledge: &mut Knowledge, at: Station, id: StarId, exposure_s: f64, now_s: f64) {
    let optics = at.optics();
    let Some(band) = optics.band() else { return };
    let sources = sky.sources(band, at.position_ly);
    let Some(index) = sources.iter().position(|s| s.star == id) else { return };
    let witness = knowledge.owner;
    if let Some(seen) =
        survey::look(&optics, &sources, index, exposure_s, at.position_ly, now_s, witness)
    {
        knowledge.sighted(id, seen);
    }
}

/// Whatever fields a sweep finished between two coordinate times.
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
            plan.observed_between(toward, from_s, to_s).map(|when| (i, when))
        })
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
            knowledge.sighted(source.star, seen);
        }
    }
}

/// Issue the charts a ship leaves port with.
///
/// A craft does not start from nothing — it starts from somebody else's parallax program,
/// which is exactly as good as whoever ran it and does not extend past where they were looking.
/// So the nearby sky arrives as claims from a charting office the ship has never met, held on
/// that office's word until the ship measures one for itself, and everything beyond `reach_ly`
/// is sky nobody aboard has ever detected.
/// What the charting office calls a star: a number of its own, the same for every ship it
/// charts for. Not the catalogue's name, which is the generator's and never shown.
pub fn chart_number(id: StarId) -> String {
    let raw = id.get();
    format!("HC {:04X}-{:02X}", raw >> 48, (raw >> 40) & 0xFF)
}

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

    /// The three sample stars at 4.2 ly, one along each axis, so nothing hides behind anything.
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

    #[test]
    fn an_idle_instrument_learns_nothing() {
        let mut sky = spread();
        let mut k = Knowledge::new(Witness(1));
        let mut o = Observatory::default();
        o.tick(&mut sky, &mut k, at(DVec3::ZERO), YEAR_S);
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
            o.tick(&mut sky, &mut k, at(DVec3::ZERO), pass * step as f64 / 20.0);
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
        o.tick(&mut sky, &mut k, at(DVec3::ZERO), 5.0e3);
        assert!(k.own_series(id, Band::V).is_none(), "half an integration is not a sample");
        o.tick(&mut sky, &mut k, at(DVec3::ZERO), 1.0e4);
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
            o.tick(&mut sky, &mut k, at(DVec3::Z * 0.02 * step as f64), t);
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
            o.tick(&mut sky, &mut k, at(DVec3::ZERO), step as f64 * 4000.0);
        }
        for id in &ids {
            assert!(k.own_series(*id, Band::V).is_some_and(|s| !s.is_empty()));
        }
    }

    /// Two craft staring at one star see different noise, and a craft seeing it twice sees the
    /// same — what lets a shard check a claim.
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
