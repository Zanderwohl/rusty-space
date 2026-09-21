//! Finding stars in the first place: where the telescope is pointed, what it picks up there,
//! and what it cannot pick up because something brighter is in the way.

use em_spectra::{Band, blackbody};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::instrument::Instrument;
use crate::knowledge::astrometry::{self, Bearing};
use crate::knowledge::{Sighting, Witness};
use crate::rng;
use crate::sky::StarId;
use crate::star::Star;

/// Signal to noise a source must clear to be recorded at all.
pub const DETECTION_SNR: f64 = 5.0;

/// Fraction of a source's light that optics spread into a halo around it rather than into its
/// own image. Baffles and clean mirrors lower it; nothing removes it.
///
/// This one number is why a sky has blind spots. A faint star is lost wherever the halo of a
/// brighter one outshines it, which is a disc of radius `resolution * sqrt(SCATTER * ratio)`
/// — arcseconds around a comparable star, tens of degrees around the sun the telescope is
/// sitting next to.
pub const SCATTER: f64 = 1e-3;

/// A field of view, radians. Two degrees across is a wide-field survey camera.
pub const FIELD_RAD: f64 = 0.035;

/// Coordinate seconds a sweep spends on each field before moving to the next.
pub const DWELL_S: f64 = 60.0;

/// What a telescope, or several acting as one, can resolve and collect.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Optics {
    pub instrument: Instrument,
    /// Widest separation between elements combining into one image. A lone telescope's is its
    /// own aperture; a swarm's is how far apart the swarm is spread, which is why a swarm sees
    /// past a glare a single mirror cannot.
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

    /// Several instruments observing as one: the collecting area adds, the baseline is how far
    /// apart they are.
    pub fn joined(instrument: Instrument, elements: f64, baseline_m: f64) -> Self {
        let instrument = instrument.with_aperture(instrument.aperture_m2 * elements.max(1.0));
        Self {
            instrument,
            baseline_m: baseline_m.max(astrometry::diameter_m(instrument.aperture_m2)),
        }
    }

    /// Which band the survey works in: V where the instrument has it, otherwise whatever it
    /// does have. A detection is in one band and the record says which.
    pub fn band(&self) -> Option<Band> {
        Band::ALL
            .into_iter()
            .find(|b| self.instrument.sees(*b) && *b == Band::V)
            .or_else(|| Band::ALL.into_iter().find(|b| self.instrument.sees(*b)))
    }

    pub fn resolution_rad(&self, band: Band) -> f64 {
        astrometry::resolution_rad(band.center_m(), self.baseline_m)
    }

    /// Signal to noise on a point source, photon statistics against the instrument's own glow.
    pub fn snr(&self, band: Band, flux_w_m2: f64, exposure_s: f64) -> f64 {
        let source = self
            .instrument
            .counts_from_flux(band, flux_w_m2, exposure_s);
        let background = self.instrument.self_emission_counts(band, exposure_s);
        if source <= 0.0 {
            return 0.0;
        }
        source / (source + background).sqrt()
    }
}

/// One point source as it arrives at the observer, in the band being surveyed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Source {
    pub star: StarId,
    /// Unit vector from the observer toward it, aberration already removed.
    pub toward: DVec3,
    pub flux_w_m2: f64,
}

/// Band flux from a star at a distance, W/m^2. The same geometry `observation::observe` uses.
pub fn flux_from(star: &Star, band: Band, distance_m: f64) -> f64 {
    if distance_m <= 0.0 {
        return 0.0;
    }
    let radiance = blackbody::band_radiance(band, star.teff_k);
    std::f64::consts::PI * star.radius_m * star.radius_m * radiance / (distance_m * distance_m)
}

/// How close to a brighter source a fainter one is lost, radians.
pub fn glare_radius_rad(resolution_rad: f64, bright: f64, faint: f64) -> f64 {
    if faint <= 0.0 || bright <= faint {
        return resolution_rad;
    }
    resolution_rad * (SCATTER * bright / faint).sqrt().max(1.0)
}

/// Whether anything in `sky` hides `sky[index]`.
pub fn hidden_by(sky: &[Source], index: usize, resolution_rad: f64) -> Option<StarId> {
    let target = sky[index];
    sky.iter()
        .enumerate()
        .filter(|(i, s)| *i != index && s.flux_w_m2 > target.flux_w_m2)
        .find(|(_, s)| {
            let separation = s.toward.angle_between(target.toward) as f64;
            separation < glare_radius_rad(resolution_rad, s.flux_w_m2, target.flux_w_m2)
        })
        .map(|(_, s)| s.star)
}

/// Point at `sky[index]` for `exposure_s` and record what comes back.
///
/// `None` when the source is under the detection threshold or inside something brighter's
/// halo. The noise is seeded from the witness, the star and the arrival time, so a server can
/// recompute exactly what this instrument saw — see `lightcone/docs/05-observation.md`.
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
    let snr = optics.snr(band, target.flux_w_m2, exposure_s);
    if snr < DETECTION_SNR {
        return None;
    }
    let resolution = optics.resolution_rad(band);
    if hidden_by(sky, index, resolution).is_some() {
        return None;
    }

    let seed = rng::hash(&[witness.0, target.star.get(), observed_s.to_bits()]);
    let sigma_rad = astrometry::centroid_sigma_rad(resolution, snr);
    let (x, y) = target.toward.any_orthonormal_pair();
    let scatter = x * rng::gaussian(rng::hash(&[seed, 1])) * sigma_rad
        + y * rng::gaussian(rng::hash(&[seed, 2])) * sigma_rad;
    let flux_sigma = target.flux_w_m2 / snr;
    Some(Sighting {
        witness,
        observed_s,
        bearing: Bearing {
            observer_ly,
            toward: (target.toward + scatter).normalize(),
            sigma_rad,
        },
        band,
        flux: target.flux_w_m2 + rng::gaussian(rng::hash(&[seed, 3])) * flux_sigma,
        flux_sigma,
        lineage: Vec::new(),
    })
}

/// A telescope working its way across a region of sky, field by field.
///
/// The sky is not observed at once. Fields are visited in a fixed order and each gets its
/// dwell, so coverage is a function of how long the instrument has been at it — and a star is
/// found when the sweep reaches the field it happens to be in, not when the player looks.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    /// Center of the region, in the observer's frame.
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

    /// A cone of sky: everything within `radius_rad` of a direction.
    pub fn region(center: DVec3, radius_rad: f64, started_s: f64) -> Self {
        Self {
            center: center.normalize_or(DVec3::Z),
            radius_rad: radius_rad.clamp(FIELD_RAD, std::f64::consts::PI),
            field_rad: FIELD_RAD,
            dwell_s: DWELL_S,
            started_s,
        }
    }

    pub fn with_dwell(mut self, dwell_s: f64) -> Self {
        self.dwell_s = dwell_s.max(1.0);
        self
    }

    /// Fields per ring of polar angle, and where each ring starts in the visiting order.
    fn rings(&self) -> Vec<(u64, u64)> {
        let count = (self.radius_rad / self.field_rad).ceil().max(1.0) as u64;
        let mut rings = Vec::with_capacity(count as usize);
        let mut offset = 0;
        for j in 0..count {
            let middle = (j as f64 + 0.5) * self.field_rad;
            let around = (std::f64::consts::TAU * middle.sin() / self.field_rad).round();
            let n = (around as u64).max(1);
            rings.push((offset, n));
            offset += n;
        }
        rings
    }

    /// Fields in one pass over the region.
    pub fn fields(&self) -> u64 {
        self.rings()
            .last()
            .map(|(offset, n)| offset + n)
            .unwrap_or(1)
    }

    /// Coordinate seconds one pass takes.
    pub fn pass_s(&self) -> f64 {
        self.fields() as f64 * self.dwell_s
    }

    /// Exposure any one star gets per pass: the dwell, not the pass.
    pub fn exposure_s(&self) -> f64 {
        self.dwell_s
    }

    /// Which field a direction falls in, or `None` outside the region.
    pub fn field_of(&self, toward: DVec3) -> Option<u64> {
        let toward = toward.normalize_or_zero();
        let polar = toward.dot(self.center).clamp(-1.0, 1.0).acos();
        if polar > self.radius_rad {
            return None;
        }
        let rings = self.rings();
        let j = ((polar / self.field_rad) as usize).min(rings.len() - 1);
        let (offset, n) = rings[j];
        let (x, y) = self.center.any_orthonormal_pair();
        let azimuth = toward
            .dot(y)
            .atan2(toward.dot(x))
            .rem_euclid(std::f64::consts::TAU);
        let i = ((azimuth / std::f64::consts::TAU) * n as f64) as u64;
        Some(offset + i.min(n - 1))
    }

    /// When a direction's field was last finished inside `(from_s, to_s]`, if it was.
    ///
    /// A field's measurement is stamped at the end of its dwell, because that is when the
    /// exposure it reports actually exists.
    pub fn observed_between(&self, toward: DVec3, from_s: f64, to_s: f64) -> Option<f64> {
        let field = self.field_of(toward)?;
        let per_pass = self.fields() as f64;
        let elapsed = (from_s - self.started_s) / self.dwell_s;
        let first = ((elapsed - field as f64 - 1.0) / per_pass).floor();
        for pass in [first - 1.0, first, first + 1.0] {
            if pass < 0.0 {
                continue;
            }
            let at = self.started_s + (pass * per_pass + field as f64 + 1.0) * self.dwell_s;
            if at > from_s {
                return (at <= to_s).then_some(at);
            }
        }
        None
    }

    /// Completed passes, and how far through the current one the sweep is.
    pub fn progress(&self, now_s: f64) -> (u64, f64) {
        let passes = ((now_s - self.started_s) / self.pass_s()).max(0.0);
        (passes as u64, passes.fract())
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
            star: StarId::synthesise("test", key),
            toward: toward.normalize(),
            flux_w_m2: flux,
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

    /// Depth is bought with time, and a four square meter mirror has plenty of it to spend:
    /// a sun-like star stays over the threshold to the far side of the galaxy. Nearby, what
    /// limits a survey is how much sky it has got round to, not how faint it can go.
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
        let error = seen.bearing.toward.angle_between(truth) as f64;
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

    /// The same seed gives the same measurement, which is what lets a server check a claim.
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

    /// The sun a telescope is sitting beside hides a wedge of sky degrees across. A star of
    /// the same kind a few light-years off hides only arcseconds, which is the same formula
    /// and the whole difference between a blind spot and a close pair.
    #[test]
    fn a_bright_star_blots_out_a_disc_around_itself() {
        let optics = Optics::of(Instrument::SHIP);
        let near = flux_from(&sun(), Band::V, AU_M);
        let far = flux_from(&sun(), Band::V, 100.0 * M_PER_LY);
        let blind = glare_radius_rad(optics.resolution_rad(Band::V), near, far);
        assert!(
            blind.to_degrees() > 1.0,
            "{} degrees of blind spot",
            blind.to_degrees()
        );
        let neighbour = glare_radius_rad(
            optics.resolution_rad(Band::V),
            flux_from(&sun(), Band::V, 4.0 * M_PER_LY),
            far,
        );
        assert!(
            neighbour.to_degrees() < 1e-3,
            "a star is not a sun in the eyepiece"
        );

        let behind = DVec3::new(1.0, blind * 0.5, 0.0);
        let beside = DVec3::new(1.0, blind * 2.0, 0.0);
        let sky = [
            source(1, DVec3::X, near),
            source(2, behind, far),
            source(3, beside, far),
        ];
        assert!(look(&optics, &sky, 1, 600.0, DVec3::ZERO, 0.0, Witness(1)).is_none());
        assert!(look(&optics, &sky, 2, 600.0, DVec3::ZERO, 0.0, Witness(1)).is_some());
        assert!(
            look(&optics, &sky, 0, 600.0, DVec3::ZERO, 0.0, Witness(1)).is_some(),
            "nothing hides the bright one"
        );
    }

    #[test]
    fn a_swarm_sees_into_a_glare_one_telescope_cannot() {
        let lone = Optics::of(Instrument::SHIP);
        let swarm = Optics::joined(Instrument::SHIP, 100.0, 1.0e5);
        let near = flux_from(&sun(), Band::V, AU_M);
        let far = flux_from(&sun(), Band::V, 100.0 * M_PER_LY);
        let separation = glare_radius_rad(lone.resolution_rad(Band::V), near, far) * 0.5;
        let sky = [
            source(1, DVec3::X, near),
            source(2, DVec3::new(1.0, separation, 0.0), far),
        ];
        assert!(look(&lone, &sky, 1, 600.0, DVec3::ZERO, 0.0, Witness(1)).is_none());
        assert!(look(&swarm, &sky, 1, 600.0, DVec3::ZERO, 0.0, Witness(1)).is_some());
        assert_eq!(
            hidden_by(&sky, 1, lone.resolution_rad(Band::V)),
            Some(sky[0].star)
        );
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
