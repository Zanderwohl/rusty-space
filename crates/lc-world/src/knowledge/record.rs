//! The records a craft holds: who measured what, when, and how it got here.
//!
//! Nothing here folds records into a belief; that is [`super::Knowledge`]. See
//! `lightcone/docs/22-provenance.md`.

use em_spectra::{Band, PerBand};
use glam::DVec3;
use serde::{Deserialize, Serialize};

use super::astrometry::{Bearing, Distance};
use super::subject::BodyId;

/// Whoever took a measurement: a ship, a probe, a telescope.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Witness(pub u64);

/// One handover of a measurement from whoever held it to whoever holds it now.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Hop {
    pub from: Witness,
    pub to: Witness,
    /// Coordinate seconds the report was transmitted.
    pub sent_s: f64,
    /// Coordinate seconds its light landed: at least `sent_s` plus the light time between them.
    pub received_s: f64,
}

/// The route a measurement took, oldest hop first. Empty for one this craft made itself.
pub type Lineage = Vec<Hop>;

/// When the holder learned something measured at `observed_s`.
pub fn learned_s(lineage: &Lineage, observed_s: f64) -> f64 {
    lineage.last().map(|h| h.received_s).unwrap_or(observed_s)
}

/// One detection of a star.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sighting {
    pub witness: Witness,
    /// Coordinate seconds the light arrived, not when it left.
    pub observed_s: f64,
    pub bearing: Bearing,
    /// Angular diameter and its sigma, radians, once the disc is resolved. `None` for a point
    /// source, which is everything interstellar and anything small enough or far enough inside
    /// a system. A radius needs this and a range; neither alone says anything.
    pub size: Option<(f64, f64)>,
    /// Meters and one sigma, from close enough to range it directly rather than infer it.
    ///
    /// The other two fields beside this one are what proximity buys, and this is the one that
    /// buys the most: a bearing with a range is a *position*, so an orbit fitted to ranged
    /// looks is not a search at all. See `survey::CLOSE_ELEMENTS`.
    pub range_m: Option<(f64, f64)>,
    /// Seconds for one turn, and one sigma, from watching features cross the disc.
    ///
    /// Only the period. The axis is not here because a close look does not settle it as
    /// cleanly, and a giant's moons give it better: the tilt of their orbits is the tilt of the
    /// planet. See `sky::generate`.
    pub spin_s: Option<(f64, f64)>,
    pub band: Band,
    /// Flux in `band`, W/m^2.
    pub flux: f64,
    pub flux_sigma: f64,
    pub lineage: Lineage,
}

impl Sighting {
    pub fn learned_s(&self) -> f64 {
        learned_s(&self.lineage, self.observed_s)
    }

    pub(crate) fn same_as(&self, other: &Self) -> bool {
        self.witness == other.witness && self.observed_s.to_bits() == other.observed_s.to_bits()
    }
}

/// One photometric measurement in a series.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub observed_s: f64,
    /// Signed fractional change against the bare star: positive is a deficit.
    pub deficit: f64,
    pub sigma: f64,
}

/// A run of photometry on one star, in one band, by one witness.
///
/// Kept per witness because two observers at different distances see different epochs of the
/// same star. Never sent to another craft: what a log says travels as a conclusion. Nothing
/// bounds it but the room aboard.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Series {
    pub witness: Witness,
    pub band: Band,
    samples: Vec<Sample>,
    /// Everything observed at or before this was consumed and is refused however it arrives.
    consumed_s: f64,
}

impl Series {
    pub fn new(witness: Witness, band: Band) -> Self {
        Self {
            witness,
            band,
            samples: Vec::new(),
            consumed_s: f64::NEG_INFINITY,
        }
    }

    pub fn samples(&self) -> &[Sample] {
        &self.samples
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn last(&self) -> Option<&Sample> {
        self.samples.last()
    }

    /// False if it was not kept: older than the last one held, or already consumed.
    pub fn push(&mut self, sample: Sample) -> bool {
        if sample.observed_s <= self.consumed_s
            || self
                .samples
                .last()
                .is_some_and(|s| sample.observed_s < s.observed_s)
        {
            return false;
        }
        self.samples.push(sample);
        true
    }

    /// Drop every sample observed at or before `through_s`, for good.
    pub fn consume_through(&mut self, through_s: f64) {
        self.samples.retain(|s| s.observed_s > through_s);
        self.consumed_s = self.consumed_s.max(through_s);
    }

    pub fn consumed_s(&self) -> f64 {
        self.consumed_s
    }

    /// What a stored file holds; the samples go to a log of their own. See
    /// `lightcone/docs/24-standing-instruments.md`.
    pub fn emptied(&self) -> Series {
        Series { samples: Vec::new(), ..self.clone() }
    }

    /// Emission times and deficits, for a plot. Without a light age the samples are against
    /// arrival time, and the caller must say so.
    pub fn against_emission(&self, light_age_s: Option<f64>) -> Vec<(f64, f64)> {
        let shift = light_age_s.unwrap_or(0.0);
        self.samples
            .iter()
            .map(|s| (s.observed_s - shift, s.deficit))
            .collect()
    }
}

/// What somebody calls something. A name travels like every other record, and two crews may
/// hold different names for the same light.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Naming {
    pub witness: Witness,
    pub name: String,
    pub kind: NameKind,
    pub stated_s: f64,
    pub lineage: Lineage,
}

/// Whether a name was chosen or merely assigned.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    /// Somebody decided to call it this.
    Given,
    /// What an instrument wrote down when it found it. Always beaten by a name somebody chose.
    Designation,
    /// Assigned, and read after whatever the holder calls the subject's star. Ranked as a
    /// designation.
    Relative,
}

impl NameKind {
    pub fn chosen(self) -> bool {
        matches!(self, Self::Given)
    }
}

/// A distance somebody states, for when the measurements behind it do not travel. A craft's own
/// triangulation overrides it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Claim {
    pub witness: Witness,
    pub distance: Distance,
    /// Coordinate seconds the claimant stated it.
    pub stated_s: f64,
    pub lineage: Lineage,
}

/// How much of an orbit's orientation is known.
///
/// `pole` and `node` are in **simulation axes**, never in the system's own plane. A longitude
/// measured from the plane's zero is a display quantity, derived when a panel asks: the plane is
/// itself a belief drawn from these orbits, so storing a longitude in it would define each orbit
/// against a frame its own value helps determine, and refining one orbit would silently move
/// every other one's stored number. See
/// `lightcone/docs/25-system-knowledge.md#the-systems-plane`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Orientation {
    Unknown,
    /// Seen to transit from `toward`: the pole lies somewhere on the great circle perpendicular
    /// to that line of sight, and the body was on the line at the orbit's epoch.
    ///
    /// Two craft that watched the same planet transit from different directions have two great
    /// circles, which cross at the pole up to its sign. Orientation from outside a system is a
    /// cooperative measurement.
    EdgeOnTo { toward: DVec3 },
    Known { pole: DVec3, sigma_rad: f64, node: f64, periapsis: f64 },
}

/// How an orbit was arrived at.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// From a settled transit: a period measured directly, and a distance through the host's
    /// mass. The error is mostly the mass's.
    Transit,
    /// Fitted to bearings taken from inside the system.
    Astrometric,
    /// Stated by a craft that sent no raw data. Cannot be re-solved or checked, exactly as a
    /// [`Claim`] cannot.
    Claim,
}

impl Method {
    /// How much an orbit found this way is worth against one found another way.
    ///
    /// A fit to bearings taken from inside the system has a measured distance and a solved
    /// orientation; a transit has a period and a distance no better than the host star's mass,
    /// and states the pole only as a circle. A claim has no raw data behind it at all.
    pub fn standing(self) -> u8 {
        match self {
            Self::Astrometric => 2,
            Self::Transit => 1,
            Self::Claim => 0,
        }
    }
}

/// What a body has been measured at, band by band, folded from every visit.
///
/// **The per-visit digest.** One visit is one row -- a flux in every band the instrument has --
/// and a row folded in is a row freed. Fixed in size per body, so eight planets cost the same
/// in month three as in month one, which is the property the fifteen-minute run rests on.
///
/// The orbit half of the digest is the decimated arc itself: [`super::BEARINGS_KEPT`] bearings
/// per witness, kept by widest spread, and `knowledge::arc` re-fits from them. That is doc 25's
/// second route, taken over accumulated normal equations, so the conditioning question that
/// route raised never had to be answered.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Colors {
    pub witness: Witness,
    /// Visits where this band *and* [`Colors::REFERENCE`] were both measured.
    pub visits: PerBand<u32>,
    /// Weighted mean of this band's flux over the reference band's, in the same visit.
    ///
    /// **A ratio, not a flux.** A body is visited from wherever the craft happens to be, so
    /// the raw flux carries the range and the phase as well as the body: averaging it measured
    /// the geometry of the visits rather than the surface. Two bands measured in the *same*
    /// visit share both exactly, so their ratio is free of either -- and a band detected only
    /// on near visits no longer biases the answer, because every value folded is already a
    /// color.
    pub mean: PerBand<f64>,
    /// Weighted sum of squared deviations, for the spread of that ratio.
    pub scatter: PerBand<f64>,
    /// Summed weights, per band. Kept because a weighted mean needs them to continue.
    pub weight: PerBand<f64>,
    /// Coordinate seconds of the first and last visit that measured something.
    pub spanned_s: (f64, f64),
    pub lineage: Lineage,
}

impl Colors {
    /// The band every other is measured against.
    ///
    /// One fixed band rather than the brightest, because a running fold cannot change what its
    /// values mean partway through. V is the middle of the optical run and what an instrument
    /// is likeliest to have; a visit that did not measure it folds nothing, which is honest --
    /// a color needs two bands at once and there is only one.
    pub const REFERENCE: Band = Band::V;

    pub fn new(witness: Witness) -> Self {
        Self {
            witness,
            visits: PerBand::splat(0),
            mean: PerBand::splat(0.0),
            scatter: PerBand::splat(0.0),
            weight: PerBand::splat(0.0),
            spanned_s: (f64::INFINITY, f64::NEG_INFINITY),
            lineage: Lineage::new(),
        }
    }

    /// Fold one visit in. Each entry is a flux and its one sigma; `None` for a band the
    /// instrument does not have or could not detect the body in, which is not a zero.
    pub fn fold(&mut self, at_s: f64, flux: &PerBand<Option<(f64, f64)>>) {
        let Some((reference, reference_sigma)) = flux[Self::REFERENCE] else { return };
        if !(reference.abs() > 0.0) {
            return;
        }
        let reference_part = reference_sigma / reference;
        let mut folded = false;
        for band in Band::ALL {
            let Some((measured, sigma)) = flux[band] else { continue };
            if !(measured.abs() > 0.0) {
                continue;
            }
            let ratio = measured / reference;
            // Both errors, because the reference carries its own into every color it makes.
            let part = ((sigma / measured).powi(2) + reference_part.powi(2)).sqrt();
            let spread = (ratio * part).abs();
            if !(spread > 0.0) || !spread.is_finite() {
                continue;
            }
            let w = 1.0 / (spread * spread);
            // Weighted Welford: a far, noisy visit must not weigh the same as a near one, and
            // with the range divided out that is the only thing left to tell them apart.
            let total = self.weight[band] + w;
            let step = ratio - self.mean[band];
            self.mean[band] += step * w / total;
            self.scatter[band] += w * step * (ratio - self.mean[band]);
            self.weight[band] = total;
            self.visits[band] = self.visits[band].saturating_add(1);
            folded = true;
        }
        // Only a visit that measured something has been spanned. A visit that saw nothing at
        // all is not evidence and must not stretch the record of when this was watched.
        if folded {
            self.spanned_s = (self.spanned_s.0.min(at_s), self.spanned_s.1.max(at_s));
        }
    }

    /// This band over the reference, and one sigma on that mean, or `None` where nothing was
    /// measured.
    pub fn against_reference(&self, band: Band) -> Option<(f64, f64)> {
        let n = self.visits[band];
        if n == 0 || !(self.weight[band] > 0.0) {
            return None;
        }
        // One visit gives a mean and no spread to judge it by, so the weight is all there is.
        let variance = if n > 1 {
            (self.scatter[band] / self.weight[band]).max(0.0) * n as f64 / (n - 1) as f64
        } else {
            0.0
        };
        let from_spread = (variance / n as f64).sqrt();
        Some((self.mean[band], from_spread.max((1.0 / self.weight[band]).sqrt())))
    }

    /// When this digest reached the craft holding it: the newest visit in it, carried forward
    /// by however many relays it crossed.
    pub fn learned_s(&self) -> f64 {
        learned_s(&self.lineage, self.spanned_s.1)
    }

    /// The color: this band's flux over that one's, and its fractional error.
    ///
    /// What a type hypothesis reads, and what no single band can say. The reference cancels,
    /// so any pair may be asked for however the digest was folded.
    pub fn color(&self, over: Band, under: Band) -> Option<(f64, f64)> {
        let ((a, sa), (b, sb)) = (self.against_reference(over)?, self.against_reference(under)?);
        (b.abs() > 0.0 && a.abs() > 0.0).then(|| {
            let ratio = a / b;
            (ratio, ratio.abs() * ((sa / a).powi(2) + (sb / b).powi(2)).sqrt())
        })
    }
}

/// Where somebody says a body orbits, as much of it as they have.
///
/// A statement, like a [`Claim`], because a craft outside a system infers an orbit from a period
/// or is told one. Letters are assigned against these.
///
/// Each element carries its own sigma, because they are not measured together: a transit gives a
/// period to a fraction of a percent and a distance only as well as it knows the star's mass.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Orbit {
    pub witness: Witness,
    /// What it goes round. `None` is the system's star, which is what a planet goes round.
    ///
    /// **The primary is not assumed, it is found.** A Keplerian orbit puts its primary at a
    /// focus, so a candidate that works as a focus *is* the primary, and the same test finds
    /// the star for a planet, the planet for a moon and the moon for a moon's moon. Nothing
    /// here is a special case for moons; see `knowledge::primary::fit_orbit`.
    pub about: Option<BodyId>,
    /// Seconds, and one sigma.
    pub period_s: (f64, f64),
    /// AU, and one sigma.
    pub semi_major_au: (f64, f64),
    /// `None` when nothing has constrained it, which a circle is not the same as.
    pub eccentricity: Option<(f64, f64)>,
    pub orientation: Orientation,
    /// A time the body was at a known place on the orbit: a transit's mid-time, or a fit's
    /// epoch. With a full orientation this is what places the body now.
    pub epoch_s: Option<f64>,
    pub method: Method,
    /// Coordinate seconds the witness stated it.
    pub stated_s: f64,
    pub lineage: Lineage,
}

impl Orbit {
    /// Kepler's third law, `P = 2 pi sqrt(a^3 / mu)`, with the fractional error the host mass
    /// carries into it.
    ///
    /// A third of the mass's fractional error, since the period goes as `mu^-1/2` and the axis
    /// as `mu^1/3`. The distance from a transit is never better than the mass prior, and saying
    /// so here keeps every producer from having to remember it.
    pub fn from_period(
        witness: Witness,
        period_s: (f64, f64),
        mu: f64,
        mu_fraction: f64,
        orientation: Orientation,
        epoch_s: Option<f64>,
        method: Method,
        stated_s: f64,
    ) -> Self {
        let a_m = em_foundations::kepler::semi_major_axis::third_law(mu, period_s.0);
        let a_au = a_m / crate::navigation::AU;
        let spread = (period_s.1 / period_s.0.max(f64::MIN_POSITIVE)).abs() * 2.0 / 3.0
            + mu_fraction.abs() / 3.0;
        Self {
            witness,
            about: None,
            period_s,
            semi_major_au: (a_au, a_au * spread),
            eccentricity: None,
            orientation,
            epoch_s,
            method,
            stated_s,
            lineage: Lineage::new(),
        }
    }
}

#[cfg(test)]
mod colors_tests {
    use super::*;

    fn seen(values: &[(Band, f64)], part: f64) -> PerBand<Option<(f64, f64)>> {
        let mut out = PerBand::splat(None);
        for (band, flux) in values {
            out[*band] = Some((*flux, flux * part));
        }
        out
    }

    /// **A digest measures the body, not the visits.** A body visited from four times the
    /// range returns a sixteenth of the light, and folding raw flux averaged that geometry
    /// into the answer. Two bands measured in the same visit share the range and the phase
    /// exactly, so what is folded is their ratio and the distance never enters.
    #[test]
    fn range_and_phase_divide_out_of_a_color() {
        let mut near = Colors::new(Witness(1));
        let mut far = Colors::new(Witness(1));
        // One body: twice as bright in R as in V, half as bright in B.
        for visit in 0..8 {
            let at = visit as f64;
            near.fold(at, &seen(&[(Band::B, 0.5), (Band::V, 1.0), (Band::R, 2.0)], 1.0e-3));
            // The same body from four times as far, at a third of the phase.
            let dim = 1.0 / 48.0;
            far.fold(at, &seen(&[(Band::B, 0.5 * dim), (Band::V, dim), (Band::R, 2.0 * dim)], 1.0e-3));
        }
        let (a, _) = near.color(Band::R, Band::B).expect("a color");
        let (b, _) = far.color(Band::R, Band::B).expect("and the same color");
        assert!((a - 4.0).abs() < 0.01, "R over B is {a}");
        assert!((a / b - 1.0).abs() < 1.0e-9, "{a} against {b} from four times the range");
    }

    /// A band detected only on the near visits used to drag the answer with it: its mean was
    /// of bright readings and the other band's mean included the dim ones. Folding colors
    /// leaves nothing for that to bias.
    #[test]
    fn a_band_seen_only_up_close_does_not_bias_the_rest() {
        let mut held = Colors::new(Witness(1));
        for visit in 0..6 {
            let dim = if visit < 3 { 1.0 } else { 1.0 / 40.0 };
            // B is only detected on the three near visits; V and R on all six.
            let mut row = vec![(Band::V, dim), (Band::R, 2.0 * dim)];
            if visit < 3 {
                row.push((Band::B, 0.5 * dim));
            }
            held.fold(visit as f64, &seen(&row, 1.0e-3));
        }
        let (ratio, _) = held.color(Band::R, Band::B).expect("a color from the overlap");
        assert!((ratio - 4.0).abs() < 0.05, "R over B is {ratio}, and the body's is 4");
        assert_eq!(held.visits[Band::B], 3);
        assert_eq!(held.visits[Band::R], 6);
    }

    /// A visit that measured nothing is not evidence, so it must not stretch the record of
    /// when the body was watched.
    #[test]
    fn a_visit_that_saw_nothing_spans_nothing() {
        let mut held = Colors::new(Witness(1));
        held.fold(100.0, &seen(&[(Band::V, 1.0), (Band::R, 2.0)], 1.0e-3));
        held.fold(900.0, &PerBand::splat(None));
        // And a visit that missed the reference band has no color in it either.
        held.fold(950.0, &seen(&[(Band::R, 2.0)], 1.0e-3));
        assert_eq!(held.spanned_s, (100.0, 100.0));
        assert_eq!(held.visits[Band::R], 1, "a reading with nothing to compare it against");
    }

    /// A better-measured visit counts for more, which is what a weighted fold is for: with the
    /// distance divided out there is nothing else left to tell two visits apart.
    #[test]
    fn a_precise_visit_outweighs_a_ragged_one() {
        let mut held = Colors::new(Witness(1));
        // One careful reading of the truth, and nine ragged ones a long way off it.
        held.fold(0.0, &seen(&[(Band::V, 1.0), (Band::R, 2.0)], 1.0e-4));
        for visit in 1..10 {
            held.fold(visit as f64, &seen(&[(Band::V, 1.0), (Band::R, 3.0)], 0.5));
        }
        let (ratio, _) = held.color(Band::R, Band::V).expect("a color");
        assert!((ratio - 2.0).abs() < 0.05, "the careful reading should win: {ratio}");
    }
}
