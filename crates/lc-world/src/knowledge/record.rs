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
    /// Per band: how many visits contributed, their mean flux in W/m^2, and the sum of squared
    /// deviations from it. Welford's, so a mean and a variance accumulate in one pass and
    /// neither needs the rows back.
    pub visits: PerBand<u32>,
    pub mean: PerBand<f64>,
    pub scatter: PerBand<f64>,
    /// Coordinate seconds of the first and last visit folded in.
    pub spanned_s: (f64, f64),
    pub lineage: Lineage,
}

impl Colors {
    pub fn new(witness: Witness) -> Self {
        Self {
            witness,
            visits: PerBand::splat(0),
            mean: PerBand::splat(0.0),
            scatter: PerBand::splat(0.0),
            spanned_s: (f64::INFINITY, f64::NEG_INFINITY),
            lineage: Lineage::new(),
        }
    }

    /// Fold one visit in. `flux` is `None` for a band the instrument does not have or could not
    /// detect the body in, which is not the same as a zero.
    pub fn fold(&mut self, at_s: f64, flux: &PerBand<Option<f64>>) {
        for band in Band::ALL {
            let Some(measured) = flux[band] else { continue };
            let n = self.visits[band].saturating_add(1);
            self.visits[band] = n;
            let step = measured - self.mean[band];
            self.mean[band] += step / n as f64;
            // Welford: the second term uses the *updated* mean, which is what keeps this stable
            // where `sum of squares minus square of sum` cancels away its own digits.
            self.scatter[band] += step * (measured - self.mean[band]);
        }
        self.spanned_s = (self.spanned_s.0.min(at_s), self.spanned_s.1.max(at_s));
    }

    /// Mean flux in a band and one sigma on that mean, or `None` where nothing was measured.
    pub fn flux_in(&self, band: Band) -> Option<(f64, f64)> {
        let n = self.visits[band];
        if n == 0 {
            return None;
        }
        // One visit gives a mean and no spread to judge it by, which is honestly nothing.
        let variance = if n > 1 { self.scatter[band] / (n - 1) as f64 } else { f64::INFINITY };
        Some((self.mean[band], (variance / n as f64).sqrt()))
    }

    /// The ratio of two bands' means, and its fractional error: the color, which is what a
    /// type hypothesis reads and what no single band can say.
    pub fn color(&self, over: Band, under: Band) -> Option<(f64, f64)> {
        let ((a, sa), (b, sb)) = (self.flux_in(over)?, self.flux_in(under)?);
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
