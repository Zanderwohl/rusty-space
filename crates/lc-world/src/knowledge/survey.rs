//! Finding stars in the first place: where the telescope is pointed, what it picks up there,
//! and what it cannot pick up because something brighter is in the way.

use em_spectra::{Band, blackbody};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::instrument::{Instrument, PHOTOMETRY_FLOOR};
use crate::knowledge::astrometry::{self, Bearing};
use crate::knowledge::{Sighting, Subject, Witness};
use crate::rng;
use crate::sky::StarId;
use crate::star::Star;

pub const DETECTION_SNR: f64 = 5.0;

/// Fraction of a source's light the optics spread into wings rather than into its own image.
///
/// The wings fall as the cube of the separation, near what is measured off a real mirror and,
/// unlike a shallower fall, integrating to something finite. So the blind spot around a bright
/// source is not one radius: it depends on what is being looked for. A planet at the same range
/// is lost only where it is behind the star. The faintest star the instrument can reach at all
/// is lost across degrees. See [`glare_radius_rad`].
pub const SCATTER: f64 = 1e-3;

/// Radians: two degrees, a wide-field survey camera.
pub const FIELD_RAD: f64 = 0.035;

/// Coordinate seconds a sweep spends on each field.
pub const DWELL_S: f64 = 60.0;

/// Coordinate seconds a survey spends on each body.
///
/// A body inside the system is bright and resolved, so this is not a depth: a planet gives
/// 1e13 counts in a minute from 5 AU. It is how long the telescope is committed elsewhere,
/// which is what sets how often anything comes round.
pub const SURVEY_DWELL_S: f64 = 60.0;

/// Resolution elements across a disc past which the instruments measure it directly rather
/// than infer it from across a system: a range, and how fast it turns.
///
/// **Proximity is the instrument.** There is no lidar module any more than there is a telescope
/// module; what a ship has is one sensor, and how much it can do with a body depends on how
/// much of the sky that body fills. Past a thousand elements the disc is a map rather than a
/// dot: features can be followed across it, and the return of a ranging pulse is worth having.
///
/// A thousand is chosen so that a survey from `START_OFFSET_AU` does *not* get it and a visit
/// does. From 5 AU a ship's telescope puts Jupiter at 630 elements, Saturn at 290, Earth at 57
/// and Mars at 30, so every one of them has to be approached: Jupiter inside 3.1 AU, Earth
/// inside 0.29, Mars inside 0.15. From an orbit about any of them it is millions.
pub const CLOSE_ELEMENTS: f64 = 1.0e3;

/// Best fractional precision on a range, however close. Ranging is a time-of-flight and a clock
/// is the least of a ship's problems; what limits it is knowing where its own antenna is.
pub const RANGE_FLOOR: f64 = 1.0e-6;

/// Bodies a survey measures in one tick, so a long gap costs a bounded amount.
///
/// At the design rate a tick is 438 coordinate seconds, so seven bodies fit in one and a
/// system of Sol's two hundred takes about four game hours to come round. A generated system
/// of eight planets and their moons takes minutes. The doc's "about once a game hour" was
/// written before anybody counted the bodies in the preset.
pub const VISITS_PER_TICK: usize = 32;

/// A telescope, or several acting as one.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Optics {
    pub instrument: Instrument,
    /// Widest separation between elements combining into one image; a lone telescope's is its
    /// own aperture.
    pub baseline_m: f64,
}

impl Optics {
    pub fn of(instrument: Instrument) -> Self {
        let baseline_m = astrometry::diameter_m(instrument.aperture_m2);
        Self {
            instrument,
            baseline_m,
        }
    }

    /// The collecting area adds; the baseline is how far apart they are.
    pub fn joined(instrument: Instrument, elements: f64, baseline_m: f64) -> Self {
        let instrument = instrument.with_aperture(instrument.aperture_m2 * elements.max(1.0));
        Self {
            instrument,
            baseline_m: baseline_m.max(astrometry::diameter_m(instrument.aperture_m2)),
        }
    }

    /// V where the instrument has it, otherwise whatever it does have.
    pub fn band(&self) -> Option<Band> {
        Band::ALL
            .into_iter()
            .find(|b| self.instrument.sees(*b) && *b == Band::V)
            .or_else(|| Band::ALL.into_iter().find(|b| self.instrument.sees(*b)))
    }

    pub fn resolution_rad(&self, band: Band) -> f64 {
        astrometry::resolution_rad(band.center_m(), self.baseline_m)
    }

    /// Photon statistics against the instrument's own glow.
    pub fn snr(&self, band: Band, flux_w_m2: f64, exposure_s: f64) -> f64 {
        self.snr_over(band, flux_w_m2, exposure_s, 0.0)
    }

    /// The same, with `glare_counts` of another source's wings falling in the same resolution
    /// element.
    ///
    /// Photon statistics, uncapped: detection and centroiding really do improve with every
    /// photon, and centroiding has its own floor in `astrometry::CENTROID_FLOOR`. What does not
    /// is the flux, which [`look`] holds at [`PHOTOMETRY_FLOOR`].
    pub fn snr_over(&self, band: Band, flux_w_m2: f64, exposure_s: f64, glare_counts: f64) -> f64 {
        let source = self
            .instrument
            .counts_from_flux(band, flux_w_m2, exposure_s);
        let background =
            self.instrument.self_emission_counts(band, exposure_s) + glare_counts.max(0.0);
        if source <= 0.0 {
            return 0.0;
        }
        source / (source + background).sqrt()
    }

    pub fn counts(&self, band: Band, flux_w_m2: f64, exposure_s: f64) -> f64 {
        self.instrument.counts_from_flux(band, flux_w_m2, exposure_s)
    }

    /// Counts the wings of a source of `counts` put into one resolution element this far off it.
    ///
    /// Surface brightness `A / theta^3` whose integral from one resolution element outwards is
    /// `SCATTER` of the source. Inside that first element there is no halo, only the image.
    pub fn halo_counts(&self, band: Band, counts: f64, separation_rad: f64) -> f64 {
        let resolution = self.resolution_rad(band);
        let ratio = resolution / separation_rad.max(resolution);
        counts * SCATTER * ratio * ratio * ratio / std::f64::consts::TAU
    }
}

/// One source as it arrives at the observer, in the band being surveyed.
///
/// A star and a planet are the same kind of thing here on purpose. The host star glares on its
/// own planets and a planet can pass in front of a star behind it, so both belong in one sky
/// and both go through the same glare and blend rules rather than down parallel paths.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub subject: Subject,
    /// Unit vector, aberration already removed.
    pub toward: DVec3,
    pub flux_w_m2: f64,
    /// Angular diameter, radians. Effectively zero for anything interstellar; a body inside the
    /// system, and the system's own star, are discs.
    pub diameter_rad: f64,
    /// Meters. Truth, and used only to turn the diameter into a range the way a ranging pulse
    /// would: a source's range is `2 * radius / diameter`, so carrying the radius rather than
    /// the range keeps the two from disagreeing.
    pub radius_m: f64,
    /// Seconds for one turn. Truth, for a close look to time against. `None` for a star: the
    /// catalogue states no rotation for one, and following a sunspot round is a later thing.
    pub spin_s: Option<f64>,
}

/// W/m^2. The same geometry `observation::observe` uses.
pub fn flux_from(star: &Star, band: Band, distance_m: f64) -> f64 {
    if distance_m <= 0.0 {
        return 0.0;
    }
    let radiance = blackbody::band_radiance(band, star.teff_k);
    std::f64::consts::PI * star.radius_m * star.radius_m * radiance / (distance_m * distance_m)
}

/// How close a source giving `faint_counts` may come to one giving `bright_counts` before the
/// wings of the brighter bury it, radians. Never inside one resolution element, where the two
/// are one image whatever their brightness.
///
/// Both arguments are counts over the same exposure, so the answer is what the detector can do
/// rather than what arrives. The cube root is the wings' falloff read backwards: nine orders of
/// magnitude of contrast cost three of separation, which is why a bright planet is lost only
/// behind its star while a faint one of 25 counts is lost across degrees.
pub fn glare_radius_rad(resolution_rad: f64, bright_counts: f64, faint_counts: f64) -> f64 {
    let noise = faint_counts / DETECTION_SNR;
    if bright_counts <= 0.0 || noise <= 0.0 {
        return resolution_rad;
    }
    let widening = bright_counts * SCATTER / (std::f64::consts::TAU * noise * noise);
    resolution_rad * widening.cbrt().max(1.0)
}

/// Counts every brighter source's wings put where `index` is.
pub fn glare_counts(
    optics: &Optics,
    band: Band,
    sky: &[Source],
    index: usize,
    exposure_s: f64,
) -> f64 {
    let Some(target) = sky.get(index) else { return 0.0 };
    sky.iter()
        .enumerate()
        .filter(|(i, s)| *i != index && s.flux_w_m2 > target.flux_w_m2)
        .map(|(_, s)| {
            let counts = optics.counts(band, s.flux_w_m2, exposure_s);
            optics.halo_counts(band, counts, s.toward.angle_between(target.toward))
        })
        .sum()
}

/// A brighter source closer than one resolution element, with which `index` is one image.
///
/// Not glare. The fainter source's photons land inside the brighter one's own image, so there
/// is one measurement to make and not two, and no exposure and no contrast separates them.
/// [`glare_counts`] cannot say this: the wings it models start where this ends.
pub fn blended_with(
    optics: &Optics,
    band: Band,
    sky: &[Source],
    index: usize,
) -> Option<Subject> {
    let target = *sky.get(index)?;
    let resolution = optics.resolution_rad(band);
    sky.iter()
        .filter(|s| s.subject != target.subject && s.flux_w_m2 > target.flux_w_m2)
        .find(|s| s.toward.angle_between(target.toward) < resolution)
        .map(|s| s.subject)
}

/// The source most in the way of `index`, when glare is what loses it: it would be detected in
/// a clean sky and is not in this one.
///
/// [`look`] does not consult this. The glare is background there, so a source near something
/// bright degrades before it disappears; this answers *which* source is in the way, for a
/// reader who wants to be told.
pub fn hidden_by(
    optics: &Optics,
    band: Band,
    sky: &[Source],
    index: usize,
    exposure_s: f64,
) -> Option<Subject> {
    let target = *sky.get(index)?;
    if let Some(blend) = blended_with(optics, band, sky, index) {
        return Some(blend);
    }
    let glare = glare_counts(optics, band, sky, index, exposure_s);
    if optics.snr(band, target.flux_w_m2, exposure_s) < DETECTION_SNR
        || optics.snr_over(band, target.flux_w_m2, exposure_s, glare) >= DETECTION_SNR
    {
        return None;
    }
    sky.iter()
        .filter(|s| s.subject != target.subject && s.flux_w_m2 > target.flux_w_m2)
        .max_by(|a, b| {
            let halo = |s: &Source| {
                optics.halo_counts(
                    band,
                    optics.counts(band, s.flux_w_m2, exposure_s),
                    s.toward.angle_between(target.toward),
                )
            };
            halo(a).total_cmp(&halo(b))
        })
        .map(|s| s.subject)
}

/// Noise is seeded from the witness, the star and the arrival time, so a server can recompute
/// exactly what this instrument saw. See `lightcone/docs/05-observation.md`.
#[allow(clippy::too_many_arguments)]
pub fn look(
    optics: &Optics,
    sky: &[Source],
    index: usize,
    exposure_s: f64,
    observer_ly: DVec3,
    observed_s: f64,
    witness: Witness,
) -> Option<Sighting> {
    let band = optics.band()?;
    let target = *sky.get(index)?;
    if blended_with(optics, band, sky, index).is_some() {
        return None;
    }
    // Glare is background, not a veto: a source beside something bright comes back with a
    // worse bearing and a worse flux, and only disappears once that noise swallows it.
    let glare = glare_counts(optics, band, sky, index, exposure_s);
    let snr = optics.snr_over(band, target.flux_w_m2, exposure_s, glare);
    if snr < DETECTION_SNR {
        return None;
    }
    let resolution = optics.resolution_rad(band);

    let seed = rng::hash(&[witness.0, target.subject.key(), observed_s.to_bits()]);
    let sigma_rad = astrometry::centroid_sigma_rad(resolution, snr);
    let (x, y) = target.toward.any_orthonormal_pair();
    let scatter = x * rng::gaussian(rng::hash(&[seed, 1])) * sigma_rad
        + y * rng::gaussian(rng::hash(&[seed, 2])) * sigma_rad;
    // Photons bound the position; calibration bounds the brightness. The ship's own sun
    // arrives with enough photons to claim a part in 1e11 and is worth a part in a thousand.
    let flux_sigma = target.flux_w_m2 * (1.0 / snr).max(PHOTOMETRY_FLOOR);
    Some(Sighting {
        witness,
        observed_s,
        bearing: Bearing {
            observer_ly,
            toward: (target.toward + scatter).normalize(),
            sigma_rad,
        },
        size: disc(target.diameter_rad, resolution, sigma_rad, rng::hash(&[seed, 4])),
        range_m: ranged(&target, resolution, exposure_s, rng::hash(&[seed, 5])),
        spin_s: target.spin_s.and_then(|spin| {
            spun(target.diameter_rad, resolution, spin, exposure_s, rng::hash(&[seed, 6]))
        }),
        band,
        flux: target.flux_w_m2 + rng::gaussian(rng::hash(&[seed, 3])) * flux_sigma,
        flux_sigma,
        lineage: Vec::new(),
    })
}

/// A ranging measurement, for a body whose disc fills enough of the sky to be worth aiming a
/// pulse at. `None` otherwise, which is everything seen from across a system.
///
/// `range_rad` is not a field of a [`Source`]: the range comes from the diameter and the body's
/// own radius, both of which the source knows, and a source with no size has no range either.
fn ranged(target: &Source, resolution_rad: f64, exposure_s: f64, seed: u64) -> Option<(f64, f64)> {
    let elements = target.diameter_rad / resolution_rad;
    if !(elements.is_finite() && elements >= CLOSE_ELEMENTS && target.radius_m > 0.0) {
        return None;
    }
    let range = 2.0 * target.radius_m / target.diameter_rad;
    // A pulse resolves the near limb to about the same fraction of the body that one resolution
    // element is, and a longer look averages more pulses.
    let sigma = (range / elements / (exposure_s.max(1.0)).sqrt()).max(range * RANGE_FLOOR);
    Some((range + rng::gaussian(seed) * sigma, sigma))
}

/// How fast a body turns, from following features across a disc close enough to have any.
/// `None` otherwise.
///
/// The rate, not a whole revolution: features move a measurable fraction of the way round in a
/// dwell, and the period follows. So a first close look already states a period, which is why
/// nothing here has to remember how long a body has been watched.
fn spun(
    diameter_rad: f64,
    resolution_rad: f64,
    spin_s: f64,
    exposure_s: f64,
    seed: u64,
) -> Option<(f64, f64)> {
    let elements = diameter_rad / resolution_rad;
    if !(elements.is_finite() && elements >= CLOSE_ELEMENTS && spin_s > 0.0) {
        return None;
    }
    // A feature is placed to one element in `elements`, twice over, against how far round it
    // went in the dwell. A dwell longer than the period is one turn's worth and no better.
    let turned = (exposure_s / spin_s).clamp(f64::MIN_POSITIVE, 1.0);
    let sigma = (spin_s * std::f64::consts::SQRT_2 / elements / turned).max(spin_s * RANGE_FLOOR);
    Some((spin_s + rng::gaussian(seed) * sigma, sigma))
}

/// The measured angular diameter of a resolved disc, or `None` for a point source.
///
/// A diameter is the separation of two limbs, each found the way a centroid is, hence the
/// `sqrt(2)`. It is held at [`astrometry::LIMB_FLOOR`] of itself, because the limb of a real
/// body is not a step and how it darkens toward the edge is a model rather than a measurement.
fn disc(diameter_rad: f64, resolution_rad: f64, sigma_rad: f64, seed: u64) -> Option<(f64, f64)> {
    if diameter_rad <= resolution_rad {
        return None;
    }
    let sigma = (std::f64::consts::SQRT_2 * sigma_rad).max(diameter_rad * astrometry::LIMB_FLOOR);
    Some((diameter_rad + rng::gaussian(seed) * sigma, sigma))
}

/// A region of sky visited field by field in a fixed order, each for its dwell: a star is found
/// when the sweep reaches its field.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    /// In the observer's frame.
    pub center: DVec3,
    pub radius_rad: f64,
    pub field_rad: f64,
    pub dwell_s: f64,
    pub started_s: f64,
}

impl Sweep {
    pub fn all_sky(started_s: f64) -> Self {
        Self {
            center: DVec3::Z,
            radius_rad: std::f64::consts::PI,
            field_rad: FIELD_RAD,
            dwell_s: DWELL_S,
            started_s,
        }
    }

    pub fn region(center: DVec3, radius_rad: f64, started_s: f64) -> Self {
        Self {
            center: center.normalize_or(DVec3::Z),
            // A NaN is refused upstream; if one got here, a clamp would pass it through.
            radius_rad: if radius_rad.is_nan() { std::f64::consts::PI } else { radius_rad.clamp(FIELD_RAD, std::f64::consts::PI) },
            field_rad: FIELD_RAD,
            dwell_s: DWELL_S,
            started_s,
        }
    }

    pub fn with_dwell(mut self, dwell_s: f64) -> Self {
        self.dwell_s = dwell_s.max(1.0);
        self
    }

    /// Worth holding on to: a tick asks which field each of thousands of stars is in.
    pub fn plan(&self) -> Plan {
        let count = (self.radius_rad / self.field_rad).ceil().max(1.0) as u64;
        let mut rings = Vec::with_capacity(count as usize);
        let mut fields = 0;
        for j in 0..count {
            let middle = (j as f64 + 0.5) * self.field_rad;
            let around = (std::f64::consts::TAU * middle.sin() / self.field_rad).round();
            let n = (around as u64).max(1);
            rings.push((fields, n));
            fields += n;
        }
        Plan {
            sweep: *self,
            rings,
            fields,
            basis: self.center.any_orthonormal_pair(),
        }
    }

    pub fn fields(&self) -> u64 {
        self.plan().fields
    }

    /// Coordinate seconds.
    pub fn pass_s(&self) -> f64 {
        self.fields() as f64 * self.dwell_s
    }

    /// Per star per pass: the dwell, not the pass.
    pub fn exposure_s(&self) -> f64 {
        self.dwell_s
    }

    pub fn field_of(&self, toward: DVec3) -> Option<u64> {
        self.plan().field_of(toward)
    }

    pub fn observed_between(&self, toward: DVec3, from_s: f64, to_s: f64) -> Option<f64> {
        self.plan().observed_between(toward, from_s, to_s).first().copied()
    }

    pub fn progress(&self, now_s: f64) -> (u64, f64) {
        let passes = ((now_s - self.started_s) / self.pass_s()).max(0.0);
        (passes as u64, passes.fract())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    sweep: Sweep,
    /// Each ring of polar angle: its first field, and how many it holds.
    rings: Vec<(u64, u64)>,
    fields: u64,
    basis: (DVec3, DVec3),
}

impl Plan {
    pub fn sweep(&self) -> &Sweep {
        &self.sweep
    }

    pub fn fields(&self) -> u64 {
        self.fields
    }

    pub fn field_of(&self, toward: DVec3) -> Option<u64> {
        let toward = toward.normalize_or_zero();
        let polar = toward.dot(self.sweep.center).clamp(-1.0, 1.0).acos();
        if polar > self.sweep.radius_rad {
            return None;
        }
        let j = ((polar / self.sweep.field_rad) as usize).min(self.rings.len().saturating_sub(1));
        let &(offset, n) = self.rings.get(j)?;
        let (x, y) = self.basis;
        let azimuth = toward
            .dot(y)
            .atan2(toward.dot(x))
            .rem_euclid(std::f64::consts::TAU);
        let i = ((azimuth / std::f64::consts::TAU) * n as f64) as u64;
        Some(offset + i.min(n - 1))
    }

    /// Oldest first, at most [`PASSES_PER_CALL`]. Each is stamped at the end of its dwell,
    /// when the exposure it reports exists.
    pub fn observed_between(&self, toward: DVec3, from_s: f64, to_s: f64) -> Vec<f64> {
        let Some(field) = self.field_of(toward) else { return Vec::new() };
        let per_pass = self.fields as f64;
        let elapsed = (from_s - self.sweep.started_s) / self.sweep.dwell_s;
        let first = ((elapsed - field as f64 - 1.0) / per_pass).floor().max(0.0);
        let finished = |pass: f64| self.sweep.started_s + (pass * per_pass + field as f64 + 1.0) * self.sweep.dwell_s;
        (0..PASSES_PER_CALL + 2)
            .map(|k| finished(first - 1.0 + k as f64))
            .filter(|at| *at > from_s && *at <= to_s)
            .take(PASSES_PER_CALL)
            .collect()
    }
}

/// So a long gap costs a bounded amount.
pub const PASSES_PER_CALL: usize = 8;

/// What a telescope is committed to; one thing at a time.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub enum Duty {
    #[default]
    Idle,
    Stare(StarId),
    Sweep(Sweep),
    /// Each target in turn, for `dwell_s` apiece: a light curve with gaps, which the period
    /// finders in `05-observation.md` are written to survive.
    Watch {
        targets: Vec<StarId>,
        dwell_s: f64,
        started_s: f64,
    },
    /// Every body of one star's system in turn, brightest first, for [`SURVEY_DWELL_S`] each.
    ///
    /// Unlike a [`Duty::Watch`] this does not name its targets, because the craft does not know
    /// them: finding them is the duty. It rotates over whatever the system holds, so a body too
    /// faint or too close to the star is simply not detected on that pass and is there on a
    /// later one when the geometry has moved.
    Survey {
        star: StarId,
        started_s: f64,
    },
}

impl From<&Duty> for lc_proto::Duty {
    fn from(duty: &Duty) -> Self {
        match duty {
            Duty::Idle => Self::Idle,
            Duty::Stare(star) => Self::Stare { star: star.get() },
            Duty::Sweep(sweep) => Self::Sweep {
                center: sweep.center.to_array(),
                radius_rad: sweep.radius_rad,
                dwell_s: sweep.dwell_s,
                started_s: sweep.started_s,
            },
            Duty::Watch { targets, dwell_s, started_s } => Self::Watch {
                stars: targets.iter().map(|s| s.get()).collect(),
                dwell_s: *dwell_s,
                started_s: *started_s,
            },
            Duty::Survey { star, started_s } => {
                Self::Survey { star: star.get(), started_s: *started_s }
            }
        }
    }
}

impl From<&lc_proto::Duty> for Duty {
    fn from(duty: &lc_proto::Duty) -> Self {
        match duty {
            lc_proto::Duty::Idle => Self::Idle,
            lc_proto::Duty::Stare { star } => Self::Stare(StarId::from_raw(*star)),
            lc_proto::Duty::Sweep { center, radius_rad, dwell_s, started_s } => {
                Self::Sweep(Sweep::region(DVec3::from_array(*center), *radius_rad, *started_s).with_dwell(*dwell_s))
            }
            lc_proto::Duty::Watch { stars, dwell_s, started_s } => Self::Watch {
                targets: stars.iter().map(|s| StarId::from_raw(*s)).collect(),
                dwell_s: dwell_s.max(1.0),
                started_s: *started_s,
            },
            lc_proto::Duty::Survey { star, started_s } => {
                Self::Survey { star: StarId::from_raw(*star), started_s: *started_s }
            }
        }
    }
}

impl Duty {
    pub fn target_at(&self, now_s: f64) -> Option<StarId> {
        match self {
            Self::Stare(id) => Some(*id),
            Self::Watch { targets, .. } => targets
                .get(self.slot_at(now_s)? as usize % targets.len())
                .copied(),
            _ => None,
        }
    }

    pub fn slot_at(&self, now_s: f64) -> Option<i64> {
        match self {
            Self::Watch {
                targets,
                dwell_s,
                started_s,
            } if !targets.is_empty() => {
                Some(((now_s - started_s) / dwell_s.max(1.0)).floor() as i64)
            }
            _ => None,
        }
    }

    pub fn sweep(&self) -> Option<&Sweep> {
        match self {
            Self::Sweep(sweep) => Some(sweep),
            _ => None,
        }
    }

    /// The star whose system a survey is of. `None` for every other duty.
    pub fn surveying(&self) -> Option<StarId> {
        match self {
            Self::Survey { star, .. } => Some(*star),
            _ => None,
        }
    }

    /// Which bodies of a sorted list a survey measures between two times, as indices into it.
    ///
    /// Brightest first and rotating, so the first tick of a new survey lands on the brightest
    /// things in the system: from 5 AU every major planet is in the first handful, which is how
    /// each of them has a position inside the first real second. Read from the clock rather
    /// than from a stored cursor, so two sides that ticked differently agree.
    pub fn visits(&self, count: usize, from_s: f64, to_s: f64) -> Vec<usize> {
        let Self::Survey { started_s, .. } = self else { return Vec::new() };
        if count == 0 || to_s <= from_s {
            return Vec::new();
        }
        // Each turn is stamped at the *end* of its dwell, when the exposure it reports exists,
        // as a sweep's fields are. Turn zero is the brightest body and ends one dwell in, so
        // counting from the elapsed time instead would skip it on the first tick and not come
        // back to it for a whole cycle.
        let ends = |k: i64| started_s + (k + 1) as f64 * SURVEY_DWELL_S;
        let first = ((from_s - started_s) / SURVEY_DWELL_S).max(0.0).floor() as i64;
        (first..=(first + VISITS_PER_TICK as i64))
            .filter(|k| ends(*k) > from_s && ends(*k) <= to_s)
            .take(VISITS_PER_TICK)
            .map(|k| k.rem_euclid(count as i64) as usize)
            .collect()
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Stare(_) => "staring",
            Self::Sweep(_) => "sweeping",
            Self::Watch { .. } => "watching",
            Self::Survey { .. } => "surveying",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use em_spectra::BandMask;

    const M_PER_LY: f64 = crate::system::M_PER_LY;
    const AU_M: f64 = 1.495_978_707e11;

    fn sun() -> Star {
        Star {
            radius_m: 6.957e8,
            teff_k: 5772.0,
            mu: 1.327e20,
            limb_darkening: (0.4, 0.26),
        }
    }

    fn source(key: u64, toward: DVec3, flux: f64) -> Source {
        Source {
            subject: Subject::Star(StarId::synthesise("test", key)),
            toward: toward.normalize(),
            flux_w_m2: flux,
            diameter_rad: 0.0,
            radius_m: 0.0,
            spin_s: None,
        }
    }

    #[test]
    fn a_sweep_covers_every_direction_exactly_once_per_pass() {
        let sweep = Sweep::all_sky(0.0);
        let fields = sweep.fields();
        assert!(
            fields > 8000 && fields < 15000,
            "{fields} fields at two degrees"
        );

        let mut seen = std::collections::HashSet::new();
        for i in 0..2000 {
            let u = rng::uniform(rng::hash(&[i, 1])) * 2.0 - 1.0;
            let phi = rng::uniform(rng::hash(&[i, 2])) * std::f64::consts::TAU;
            let dir = DVec3::new(
                (1.0f64 - u * u).sqrt() * phi.cos(),
                (1.0f64 - u * u).sqrt() * phi.sin(),
                u,
            );
            seen.insert(sweep.field_of(dir).expect("all-sky misses nothing"));
        }
        assert!(
            seen.len() > 1500,
            "{} distinct fields out of 2000 looks",
            seen.len()
        );
    }

    #[test]
    fn a_region_is_a_cone_and_the_rest_of_the_sky_is_not_in_it() {
        let sweep = Sweep::region(DVec3::X, 0.2, 0.0);
        assert!(sweep.field_of(DVec3::X).is_some());
        assert!(sweep.field_of(DVec3::new(1.0, 0.1, 0.0)).is_some());
        assert!(sweep.field_of(DVec3::Y).is_none());
        assert!(
            sweep.pass_s() < Sweep::all_sky(0.0).pass_s() / 50.0,
            "a cone comes round often"
        );
    }

    #[test]
    fn a_field_comes_round_once_a_pass() {
        let sweep = Sweep::region(DVec3::X, 0.2, 0.0);
        let dir = DVec3::new(1.0, 0.05, 0.02);
        let pass = sweep.pass_s();
        let first = sweep
            .observed_between(dir, 0.0, pass)
            .expect("covered in the first pass");
        assert!(first > 0.0 && first <= pass);
        assert!(
            sweep
                .observed_between(dir, first, first + pass * 0.5)
                .is_none()
                || sweep.observed_between(dir, first, first + pass).is_some()
        );
        let second = sweep
            .observed_between(dir, first, first + pass)
            .expect("and the next");
        assert!(
            (second - first - pass).abs() < sweep.dwell_s * 2.0,
            "{first} then {second}"
        );
        assert!(sweep.observed_between(DVec3::Y, 0.0, pass * 3.0).is_none());
    }

    /// With a long exposure a four square meter mirror detects a sun across the galaxy.
    #[test]
    fn depth_is_bought_with_exposure() {
        let optics = Optics::of(Instrument::SHIP);
        let sky = [source(
            1,
            DVec3::X,
            flux_from(&sun(), Band::V, 1.0e6 * M_PER_LY),
        )];
        assert!(look(&optics, &sky, 0, 10.0, DVec3::ZERO, 0.0, Witness(1)).is_none());
        assert!(look(&optics, &sky, 0, 1.0e4, DVec3::ZERO, 0.0, Witness(1)).is_some());
        let near = [source(
            1,
            DVec3::X,
            flux_from(&sun(), Band::V, 1.0e4 * M_PER_LY),
        )];
        assert!(look(&optics, &near, 0, DWELL_S, DVec3::ZERO, 0.0, Witness(1)).is_some());
    }

    #[test]
    fn a_detection_carries_a_bearing_good_to_its_signal_to_noise() {
        let optics = Optics::of(Instrument::SHIP);
        let flux = flux_from(&sun(), Band::V, 10.0 * M_PER_LY);
        let truth = DVec3::new(0.3, 0.9, -0.2).normalize();
        let sky = [source(1, truth, flux)];
        let seen = look(&optics, &sky, 0, 60.0, DVec3::ZERO, 1.0e6, Witness(1)).unwrap();
        assert_eq!(seen.band, Band::V);
        let error = seen.bearing.toward.angle_between(truth);
        assert!(
            error < 5.0 * seen.bearing.sigma_rad,
            "{error} vs {}",
            seen.bearing.sigma_rad
        );
        assert!(
            seen.bearing.sigma_rad < 1e-6,
            "a four square meter mirror resolves better"
        );
        assert!((seen.flux / flux - 1.0).abs() < 0.1);
    }

    /// The same seed gives the same measurement, so a server can check a claim.
    #[test]
    fn noise_is_addressed_not_drawn() {
        let optics = Optics::of(Instrument::SHIP);
        let sky = [source(
            1,
            DVec3::X,
            flux_from(&sun(), Band::V, 10.0 * M_PER_LY),
        )];
        let once = look(&optics, &sky, 0, 60.0, DVec3::ZERO, 7.0, Witness(4)).unwrap();
        let again = look(&optics, &sky, 0, 60.0, DVec3::ZERO, 7.0, Witness(4)).unwrap();
        assert_eq!(once.bearing.toward, again.bearing.toward);
        let elsewhen = look(&optics, &sky, 0, 60.0, DVec3::ZERO, 8.0, Witness(4)).unwrap();
        assert_ne!(once.bearing.toward, elsewhen.bearing.toward);
        let somebody = look(&optics, &sky, 0, 60.0, DVec3::ZERO, 7.0, Witness(5)).unwrap();
        assert_ne!(once.bearing.toward, somebody.bearing.toward);
    }

    /// A blind spot is not one radius. The same sun that loses a barely detectable star across
    /// degrees loses a planet beside it only where the two are one image.
    #[test]
    fn a_blind_spot_is_sized_by_what_is_being_looked_for() {
        let optics = Optics::of(Instrument::SHIP);
        let resolution = optics.resolution_rad(Band::V);
        let host = optics.counts(Band::V, flux_from(&sun(), Band::V, 5.0 * AU_M), DWELL_S);

        // The faintest thing the instrument reaches at all: DETECTION_SNR against its own
        // photons, so 25 counts.
        let faintest = glare_radius_rad(resolution, host, DETECTION_SNR * DETECTION_SNR);
        assert!(
            faintest.to_degrees() > 1.0 && faintest.to_degrees() < 20.0,
            "{} degrees around the local sun",
            faintest.to_degrees()
        );

        // Jupiter's reflected V flux from 5 AU off, worked from the same sun. A geometric
        // albedo is defined against a flat disc of the body's own radius, so this is
        // `incident * albedo * (radius / range)^2` -- `visit::of` is where it belongs, and
        // getting the factor wrong here would have understated the hole by four.
        let sunlight = flux_from(&sun(), Band::V, 5.203 * AU_M);
        let jupiter = sunlight * 0.52 * (6.991e7 / (5.0 * AU_M)) * (6.991e7 / (5.0 * AU_M));
        let hole = glare_radius_rad(resolution, host, optics.counts(Band::V, jupiter, DWELL_S));
        assert!(
            (hole - resolution).abs() < resolution * 1e-6,
            "a planet is lost only behind its star, not {} rad out",
            hole
        );

        // And a comparable star a few light-years off is no sun in the eyepiece.
        let neighbor = optics.counts(Band::V, flux_from(&sun(), Band::V, 4.0 * M_PER_LY), DWELL_S);
        let far = optics.counts(Band::V, flux_from(&sun(), Band::V, 100.0 * M_PER_LY), DWELL_S);
        assert!(
            glare_radius_rad(resolution, neighbor, far).to_degrees() < 1e-3,
            "a star is not a sun in the eyepiece"
        );
    }

    /// Glare costs precision before it costs the detection, which one hard disc could not say.
    #[test]
    fn glare_degrades_a_bearing_before_it_loses_it() {
        let optics = Optics::of(Instrument::SHIP);
        let near = flux_from(&sun(), Band::V, AU_M);
        let far = flux_from(&sun(), Band::V, 100.0 * M_PER_LY);
        let resolution = optics.resolution_rad(Band::V);
        let blind = glare_radius_rad(
            resolution,
            optics.counts(Band::V, near, 600.0),
            optics.counts(Band::V, far, 600.0),
        );
        assert!(blind > resolution, "the glare reaches past one element");

        let sky = [
            source(1, DVec3::X, near),
            source(2, DVec3::new(1.0, blind * 0.5, 0.0), far),
            source(3, DVec3::new(1.0, blind * 8.0, 0.0), far),
        ];
        assert!(look(&optics, &sky, 1, 600.0, DVec3::ZERO, 0.0, Witness(1)).is_none());
        let clear = look(&optics, &sky, 2, 600.0, DVec3::ZERO, 0.0, Witness(1)).expect("well off");
        let host = look(&optics, &sky, 0, 600.0, DVec3::ZERO, 0.0, Witness(1))
            .expect("nothing hides the brightest thing in the sky");
        assert!(host.bearing.sigma_rad <= clear.bearing.sigma_rad);
        assert_eq!(
            hidden_by(&optics, Band::V, &sky, 1, 600.0),
            Some(sky[0].subject)
        );
        assert_eq!(hidden_by(&optics, Band::V, &sky, 2, 600.0), None);
        assert_eq!(hidden_by(&optics, Band::V, &sky, 0, 600.0), None);
    }

    /// Contrast is not the point when two things are one image, and no exposure fixes it.
    #[test]
    fn two_sources_inside_one_resolution_element_are_one_image() {
        let optics = Optics::of(Instrument::SHIP);
        let resolution = optics.resolution_rad(Band::V);
        let bright = flux_from(&sun(), Band::V, 10.0 * M_PER_LY);
        let sky = [
            source(1, DVec3::X, bright),
            source(2, DVec3::new(1.0, resolution * 0.5, 0.0), bright * 0.5),
            source(3, DVec3::new(1.0, resolution * 4.0, 0.0), bright * 0.5),
        ];
        assert_eq!(blended_with(&optics, Band::V, &sky, 1), Some(sky[0].subject));
        assert_eq!(blended_with(&optics, Band::V, &sky, 2), None);
        assert_eq!(blended_with(&optics, Band::V, &sky, 0), None, "the brighter one is the image");

        assert!(look(&optics, &sky, 1, 1.0e6, DVec3::ZERO, 0.0, Witness(1)).is_none());
        assert!(look(&optics, &sky, 2, DWELL_S, DVec3::ZERO, 0.0, Witness(1)).is_some());
        assert_eq!(hidden_by(&optics, Band::V, &sky, 1, DWELL_S), Some(sky[0].subject));
    }

    #[test]
    fn a_swarm_sees_into_a_glare_one_telescope_cannot() {
        let lone = Optics::of(Instrument::SHIP);
        let swarm = Optics::joined(Instrument::SHIP, 100.0, 1.0e5);
        let near = flux_from(&sun(), Band::V, AU_M);
        let far = flux_from(&sun(), Band::V, 100.0 * M_PER_LY);
        let separation = glare_radius_rad(
            lone.resolution_rad(Band::V),
            lone.counts(Band::V, near, 600.0),
            lone.counts(Band::V, far, 600.0),
        ) * 0.5;
        let sky = [
            source(1, DVec3::X, near),
            source(2, DVec3::new(1.0, separation, 0.0), far),
        ];
        assert!(look(&lone, &sky, 1, 600.0, DVec3::ZERO, 0.0, Witness(1)).is_none());
        assert!(look(&swarm, &sky, 1, 600.0, DVec3::ZERO, 0.0, Witness(1)).is_some());
        assert_eq!(
            hidden_by(&lone, Band::V, &sky, 1, 600.0),
            Some(sky[0].subject)
        );
    }

    /// The ship's own sun is the one source whose distance everything else hangs off, and the
    /// glare that loses everything near it is its own: nothing outshines it.
    #[test]
    fn the_host_star_is_measured_however_bright_it_is() {
        let optics = Optics::of(Instrument::SHIP);
        let host = flux_from(&sun(), Band::V, 5.0 * AU_M);
        let sky = [
            source(1, DVec3::X, host),
            source(2, DVec3::Y, flux_from(&sun(), Band::V, 10.0 * M_PER_LY)),
        ];
        let seen = look(&optics, &sky, 0, DWELL_S, DVec3::ZERO, 0.0, Witness(1))
            .expect("a saturated star still gives a centroid");
        assert!(seen.bearing.sigma_rad.is_finite() && seen.bearing.sigma_rad > 0.0);
        // Its flux comes back at the calibration floor rather than to a part in 1e11.
        let precision = seen.flux_sigma / host;
        assert!(
            (precision - PHOTOMETRY_FLOOR).abs() < PHOTOMETRY_FLOOR * 1e-9,
            "{precision} against a floor of {PHOTOMETRY_FLOOR}"
        );
    }

    /// A resolved disc is a measurement; a point source is not, and claiming one would give a
    /// radius out of nothing. The ship's own sun is the only star that is ever a disc.
    #[test]
    fn a_resolved_disc_is_measured_and_a_point_source_is_not() {
        let optics = Optics::of(Instrument::SHIP);
        let resolution = optics.resolution_rad(Band::V);
        let host = flux_from(&sun(), Band::V, 5.0 * AU_M);

        // The Sun's disc from 5 AU: 1.86e-3 rad, six thousand resolution elements across.
        let disc = 2.0 * sun().radius_m / (5.0 * AU_M);
        assert!(disc / resolution > 6000.0, "{} elements", disc / resolution);
        let sky = [Source {
            subject: Subject::Star(StarId::synthesise("test", 1)),
            toward: DVec3::X,
            flux_w_m2: host,
            diameter_rad: disc,
            radius_m: sun().radius_m,
            spin_s: None,
        }];
        let seen = look(&optics, &sky, 0, DWELL_S, DVec3::ZERO, 0.0, Witness(1)).expect("seen");
        let (measured, sigma) = seen.size.expect("a resolved star has a size");
        assert!((measured / disc - 1.0).abs() < 5.0 * sigma / disc, "{measured} against {disc}");
        // Held at the limb floor, not at the centroid's, so it is a part in a thousand and not
        // the part in ten million the photons alone would claim.
        assert!(
            (sigma / disc - astrometry::LIMB_FLOOR).abs() < astrometry::LIMB_FLOOR * 1e-9,
            "{} against a floor of {}",
            sigma / disc,
            astrometry::LIMB_FLOOR
        );

        // The same star ten light-years off is a point and reports no size at all.
        let far = [source(2, DVec3::X, flux_from(&sun(), Band::V, 10.0 * M_PER_LY))];
        let point = look(&optics, &far, 0, DWELL_S, DVec3::ZERO, 0.0, Witness(1)).expect("seen");
        assert_eq!(point.size, None);
    }

    /// **Proximity is the instrument.** The same telescope that can only bear and weigh a
    /// planet from across a system ranges it and times its turning from close up, and the line
    /// between is how much of the sky the disc fills.
    #[test]
    fn closeness_buys_a_range_and_a_rotation_and_distance_takes_them_away() {
        let optics = Optics::of(Instrument::SHIP);
        let resolution = optics.resolution_rad(Band::V);
        let earth_m = 6.371e6;
        let spin_s = 86_164.0;

        let at = |range_m: f64| Source {
            subject: Subject::Body {
                star: StarId::synthesise("t", 1),
                body: crate::knowledge::BodyId::of(StarId::synthesise("t", 1), "Earth"),
            },
            toward: DVec3::X,
            // Bright enough to be detected at any of these ranges; the point here is the disc.
            flux_w_m2: 1.0e-7,
            diameter_rad: 2.0 * earth_m / range_m,
            radius_m: earth_m,
            spin_s: Some(spin_s),
        };

        // From five AU an Earth is 57 resolution elements across: a disc, and nothing more.
        let far = at(5.0 * AU_M);
        assert!(far.diameter_rad / resolution < CLOSE_ELEMENTS);
        let seen = look(&optics, &[far], 0, DWELL_S, DVec3::ZERO, 0.0, Witness(1)).expect("seen");
        assert!(seen.size.is_some(), "the disc is resolved even so");
        assert_eq!(seen.range_m, None, "no ranging from across a system");
        assert_eq!(seen.spin_s, None, "and no features to follow");

        // From a low orbit it is millions of elements across, and both come for free.
        let close = at(earth_m + 4.0e5);
        assert!(close.diameter_rad / resolution > 1.0e6);
        let seen = look(&optics, &[close], 0, DWELL_S, DVec3::ZERO, 0.0, Witness(1)).expect("seen");
        let (range, range_sigma) = seen.range_m.expect("a close pass ranges it");
        assert!(
            (range / (earth_m + 4.0e5) - 1.0).abs() < 1.0e-5,
            "{range} m against {}",
            earth_m + 4.0e5
        );
        assert!(range_sigma / range <= 1.0e-5, "{}", range_sigma / range);
        let (spin, spin_sigma) = seen.spin_s.expect("a close pass times its turning");
        assert!((spin / spin_s - 1.0).abs() < 1.0e-3, "{spin} s against {spin_s}");
        assert!(spin_sigma > 0.0);

        // And the radius follows from the two together, which is the whole reason both are on
        // the one record.
        let radius = seen.size.expect("resolved").0 * range / 2.0;
        assert!((radius / earth_m - 1.0).abs() < 1.0e-3, "{radius} m against {earth_m}");
    }

    /// The threshold is a thousand elements because that is where a survey from the standard
    /// standoff stops getting it and a visit starts. Worth pinning: it is the one number that
    /// decides whether flying somewhere is worth anything.
    #[test]
    fn nothing_at_the_standard_standoff_is_close_enough() {
        let optics = Optics::of(Instrument::SHIP);
        let resolution = optics.resolution_rad(Band::V);
        let elements = |radius_m: f64, range_au: f64| 2.0 * radius_m / (range_au * AU_M) / resolution;

        for (name, radius_m, seen_from) in [
            ("Jupiter", 6.991e7, 5.0),
            ("Saturn", 5.823e7, 9.0),
            ("Earth", 6.371e6, 5.0),
            ("Mars", 3.390e6, 5.0),
        ] {
            let across = elements(radius_m, seen_from);
            assert!(across < CLOSE_ELEMENTS, "{name} is already close at {across:.0} elements");
            assert!(across > 20.0, "{name} should still be a disc, not {across:.0} elements");
        }
        // Jupiter needs approaching to about three AU, Earth to under a third of one.
        assert!(elements(6.991e7, 3.0) > CLOSE_ELEMENTS);
        assert!(elements(6.371e6, 0.25) > CLOSE_ELEMENTS);
    }

    #[test]
    fn a_watch_gives_each_target_its_turn() {
        let ids: Vec<StarId> = (0..3).map(|k| StarId::synthesise("test", k)).collect();
        let duty = Duty::Watch {
            targets: ids.clone(),
            dwell_s: 100.0,
            started_s: 0.0,
        };
        assert_eq!(duty.target_at(50.0), Some(ids[0]));
        assert_eq!(duty.target_at(150.0), Some(ids[1]));
        assert_eq!(duty.target_at(250.0), Some(ids[2]));
        assert_eq!(duty.target_at(350.0), Some(ids[0]), "and round again");
        assert_ne!(duty.slot_at(50.0), duty.slot_at(150.0));
        assert_eq!(Duty::Idle.target_at(0.0), None);
        assert_eq!(Duty::Stare(ids[0]).target_at(1e9), Some(ids[0]));
    }

    /// Turns are read from the clock and not from a cursor, so two sides that ticked
    /// differently agree; each turn is stamped at the end of its dwell, so turn zero -- the
    /// brightest body -- is measured on the first tick rather than a whole cycle later.
    #[test]
    fn a_survey_takes_the_bodies_in_turn_from_the_clock() {
        let duty = Duty::Survey { star: StarId::synthesise("t", 1), started_s: 0.0 };
        let tick = 438.3;

        let first = duty.visits(213, 0.0, tick);
        assert_eq!(first, vec![0, 1, 2, 3, 4, 5, 6], "seven turns of a minute fit in a tick");
        let second = duty.visits(213, tick, tick * 2.0);
        assert_eq!(second, vec![7, 8, 9, 10, 11, 12, 13], "no gap and no repeat");

        // Round again, and a body comes back where it started.
        let wrapped = duty.visits(5, 0.0, tick);
        assert_eq!(wrapped, vec![0, 1, 2, 3, 4, 0, 1], "a short list comes round often");

        // A gap costs a bounded amount rather than every turn it covers.
        let gap = duty.visits(213, 0.0, tick * 1000.0);
        assert_eq!(gap.len(), VISITS_PER_TICK);

        assert!(duty.visits(0, 0.0, tick).is_empty(), "no bodies, no turns");
        assert!(duty.visits(10, tick, 0.0).is_empty(), "time does not run backwards");
        assert!(Duty::Idle.visits(10, 0.0, tick).is_empty(), "only a survey has turns");
        assert_eq!(duty.surveying(), Some(StarId::synthesise("t", 1)));
        assert_eq!(Duty::Idle.surveying(), None);
    }

    #[test]
    fn an_instrument_surveys_in_a_band_it_has() {
        assert_eq!(Optics::of(Instrument::SHIP).band(), Some(Band::V));
        let radio = Instrument::BASELINE.with_bands(BandMask::of(&[Band::Radio]));
        assert_eq!(Optics::of(radio).band(), Some(Band::Radio));
        assert!(
            Optics::of(Instrument::BASELINE.with_bands(BandMask::EMPTY))
                .band()
                .is_none()
        );
    }
}
