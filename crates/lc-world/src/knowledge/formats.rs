//! A file as it is written down, in this format and every one before it.
//!
//! Postcard is positional and cannot notice an older shape, so each format ever written has its
//! shape here and a way up to the current one. See `lightcone/docs/24-standing-instruments.md`.
//!
//! **Bump [`FILE_FORMAT`] when [`File`], or anything inside it, changes shape**, and add the
//! shape that was current as a new reader.

use serde::{Deserialize, Serialize};

use super::conclusion::{Class, Conclusion, Covering, Digest, Evidence, Hypothesis, Kind, Settled};
use super::transit::{Candidate, Fold};
use super::{Claim, File, Lineage, Naming, Orbit, Sample, Series, Sighting, Witness};

/// What writes a file's bytes today.
pub const FILE_FORMAT: i32 = 4;

/// The oldest format still read. Anything older is refused.
pub const OLDEST_FILE_FORMAT: i32 = 1;

/// A file's bytes, in whichever format wrote them.
pub fn decode(format: i32, bytes: &[u8]) -> Result<File, String> {
    match format {
        FILE_FORMAT => lc_proto::decode(bytes).map_err(|why| why.to_string()),
        3 => lc_proto::decode::<FileV3>(bytes).map_err(|why| why.to_string()).map(Into::into),
        2 => lc_proto::decode::<FileV2>(bytes).map_err(|why| why.to_string()).map(Into::into),
        1 => lc_proto::decode::<FileV1>(bytes).map_err(|why| why.to_string()).map(Into::into),
        other => Err(format!("knowledge format {other} is not {OLDEST_FILE_FORMAT} to {FILE_FORMAT}")),
    }
}

/// A series as formats 1 to 3 wrote it. Its lineage is dropped, and its samples, which a stored
/// file logs apart.
#[derive(Serialize, Deserialize)]
struct SeriesV3 {
    witness: Witness,
    band: em_spectra::Band,
    lineage: Lineage,
    samples: Vec<Sample>,
    consumed_s: f64,
}

#[derive(Serialize, Deserialize)]
struct SeriesV1 {
    witness: Witness,
    band: em_spectra::Band,
    lineage: Lineage,
    samples: Vec<Sample>,
}

fn series(witness: Witness, band: em_spectra::Band, samples: Vec<Sample>, consumed_s: f64) -> Series {
    let mut series = Series::new(witness, band);
    if consumed_s.is_finite() {
        series.consume_through(consumed_s);
    }
    for sample in samples {
        series.push(sample);
    }
    series
}

/// Format 3: a conclusion did not say where it was drawn from.
#[derive(Serialize, Deserialize)]
struct FileV3 {
    sightings: Vec<Sighting>,
    series: Vec<SeriesV3>,
    claims: Vec<Claim>,
    names: Vec<Naming>,
    orbits: Vec<Orbit>,
    conclusions: Vec<ConclusionV3>,
    digests: Vec<Digest>,
    retained: bool,
}

#[derive(Serialize, Deserialize)]
struct ConclusionV3 {
    witness: Witness,
    observer: Witness,
    stated_s: f64,
    lineage: Lineage,
    transits: Vec<Hypothesis>,
    populations: Vec<Hypothesis>,
    evidence: Evidence,
    covering: Covering,
    discarded_s: Option<f64>,
}

impl From<ConclusionV3> for Conclusion {
    fn from(c: ConclusionV3) -> Self {
        Conclusion {
            witness: c.witness,
            observer: c.observer,
            from_ly: None,
            stated_s: c.stated_s,
            lineage: c.lineage,
            transits: c.transits,
            populations: c.populations,
            evidence: c.evidence,
            covering: c.covering,
            discarded_s: c.discarded_s,
        }
    }
}

impl From<FileV3> for File {
    fn from(f: FileV3) -> Self {
        File {
            sightings: f.sightings,
            series: f.series.into_iter().map(|s| series(s.witness, s.band, s.samples, s.consumed_s)).collect(),
            claims: f.claims,
            names: f.names,
            orbits: f.orbits,
            conclusions: f.conclusions.into_iter().map(Into::into).collect(),
            digests: f.digests,
            retained: f.retained,
        }
    }
}

/// Format 2: transits only, with no completeness, and a digest without moments.
#[derive(Serialize, Deserialize)]
struct FileV2 {
    sightings: Vec<Sighting>,
    series: Vec<SeriesV3>,
    claims: Vec<Claim>,
    names: Vec<Naming>,
    orbits: Vec<Orbit>,
    conclusions: Vec<ConclusionV2>,
    digests: Vec<DigestV2>,
    retained: bool,
}

#[derive(Serialize, Deserialize)]
struct ConclusionV2 {
    witness: Witness,
    observer: Witness,
    stated_s: f64,
    lineage: Lineage,
    hypotheses: Vec<HypothesisV2>,
    evidence: EvidenceV2,
    covering: Covering,
    discarded_s: Option<f64>,
}

#[derive(Serialize, Deserialize)]
struct HypothesisV2 {
    probability: f64,
    kind: KindV2,
}

#[derive(Serialize, Deserialize)]
enum KindV2 {
    Quiet,
    Planet { class: Class, transit: Candidate },
}

#[derive(Serialize, Deserialize)]
struct EvidenceV2 {
    samples: u64,
    bands: u16,
    ln_bayes: f64,
    prior: f64,
    periods_s: (f64, f64),
    jitter: f64,
}

#[derive(Serialize, Deserialize)]
struct DigestV2 {
    observer: Witness,
    samples: u64,
    bands: u16,
    observed_s: (f64, f64),
    ln_bayes: f64,
    delta_chi2: f64,
    prior: f64,
    periods_s: (f64, f64),
    jitter: f64,
    folds: Vec<Fold>,
}

impl From<ConclusionV2> for Conclusion {
    fn from(c: ConclusionV2) -> Self {
        // "Quiet" then meant no planet in the periods searched, not no planet at all: the log's
        // completeness was never measured, so it reads as a planet the log could not have found.
        let transits = c
            .hypotheses
            .into_iter()
            .map(|h| Hypothesis {
                probability: h.probability,
                kind: match h.kind {
                    KindV2::Quiet => Kind::Unsearched,
                    KindV2::Planet { class, transit } => Kind::Planet { class, transit },
                },
            })
            .collect();
        Conclusion {
            witness: c.witness,
            observer: c.observer,
            from_ly: None,
            stated_s: c.stated_s,
            lineage: c.lineage,
            transits,
            populations: Vec::new(),
            evidence: Evidence {
                samples: c.evidence.samples,
                bands: c.evidence.bands,
                ln_bayes: c.evidence.ln_bayes,
                prior: c.evidence.prior,
                periods_s: Some(c.evidence.periods_s),
                completeness: 0.0,
                jitter: c.evidence.jitter,
            },
            covering: c.covering,
            discarded_s: c.discarded_s,
        }
    }
}

impl From<DigestV2> for Digest {
    fn from(d: DigestV2) -> Self {
        // A format-2 digest existed only for a settled log, and kept folds only for a planet.
        let planet = (!d.folds.is_empty()).then(|| Settled {
            ln_bayes: d.ln_bayes,
            delta_chi2: d.delta_chi2,
            prior: d.prior,
            periods_s: d.periods_s,
            folds: d.folds,
        });
        Digest {
            observer: d.observer,
            samples: d.samples,
            bands: d.bands,
            observed_s: d.observed_s,
            jitter: d.jitter,
            moments: Default::default(),
            completeness: 0.0,
            planet,
        }
    }
}

impl From<FileV2> for File {
    fn from(f: FileV2) -> Self {
        File {
            sightings: f.sightings,
            series: f.series.into_iter().map(|s| series(s.witness, s.band, s.samples, s.consumed_s)).collect(),
            claims: f.claims,
            names: f.names,
            orbits: f.orbits,
            conclusions: f.conclusions.into_iter().map(Into::into).collect(),
            digests: f.digests.into_iter().map(Into::into).collect(),
            retained: f.retained,
        }
    }
}

/// Format 1: before logs were read into conclusions.
#[derive(Serialize, Deserialize)]
struct FileV1 {
    sightings: Vec<Sighting>,
    series: Vec<SeriesV1>,
    claims: Vec<Claim>,
    names: Vec<Naming>,
    orbits: Vec<Orbit>,
}

impl From<FileV1> for File {
    fn from(f: FileV1) -> Self {
        File {
            sightings: f.sightings,
            series: f.series.into_iter().map(|s| series(s.witness, s.band, s.samples, f64::NEG_INFINITY)).collect(),
            claims: f.claims,
            names: f.names,
            orbits: f.orbits,
            ..File::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use em_spectra::Band;

    use super::*;
    use crate::knowledge::NameKind;

    fn naming() -> Naming {
        Naming { witness: Witness(4), name: "Kettle".into(), kind: NameKind::Given, stated_s: 7.0, lineage: Vec::new() }
    }

    fn claim() -> Claim {
        Claim { witness: Witness(9), distance: crate::knowledge::Distance::AtLeast(3.0), stated_s: 1.0, lineage: Vec::new() }
    }

    fn candidate() -> Candidate {
        Candidate {
            period_s: 3.0e5,
            period_sigma_s: 10.0,
            epoch_s: 1.0,
            duration_s: 3600.0,
            depth: 1e-3,
            depth_sigma: 1e-5,
            delta_chi2: 1e4,
            transits: 4,
        }
    }

    fn covering() -> Covering {
        Covering { observed_s: (1.0, 2.0), light_age_s: None }
    }

    /// Everything a format-1 file said is still said: names, claims, and how far a log went.
    #[test]
    fn format_1_reads() {
        let old = FileV1 {
            sightings: Vec::new(),
            series: vec![SeriesV1 { witness: Witness(4), band: Band::V, lineage: Vec::new(), samples: Vec::new() }],
            claims: vec![claim()],
            names: vec![naming()],
            orbits: Vec::new(),
        };
        let file = decode(1, &lc_proto::encode(&old)).expect("format 1 reads");
        assert_eq!(file.names(), [naming()]);
        assert_eq!(file.claims(), [claim()]);
        assert_eq!((file.series()[0].witness, file.series()[0].band), (Witness(4), Band::V));
    }

    /// A format-2 "quiet" reads as unsearched, and its settled planet keeps its folds.
    #[test]
    fn format_2_reads() {
        let old = FileV2 {
            sightings: Vec::new(),
            series: vec![SeriesV3 { witness: Witness(4), band: Band::V, lineage: Vec::new(), samples: Vec::new(), consumed_s: 50.0 }],
            claims: Vec::new(),
            names: vec![naming()],
            orbits: Vec::new(),
            conclusions: vec![ConclusionV2 {
                witness: Witness(4),
                observer: Witness(4),
                stated_s: 9.0,
                lineage: Vec::new(),
                hypotheses: vec![
                    HypothesisV2 { probability: 0.99, kind: KindV2::Planet { class: Class::Rocky, transit: candidate() } },
                    HypothesisV2 { probability: 0.01, kind: KindV2::Quiet },
                ],
                evidence: EvidenceV2 { samples: 500, bands: 3, ln_bayes: 40.0, prior: 0.05, periods_s: (1.0, 2.0), jitter: 0.0 },
                covering: covering(),
                discarded_s: Some(50.0),
            }],
            digests: vec![DigestV2 {
                observer: Witness(4),
                samples: 500,
                bands: 3,
                observed_s: (1.0, 50.0),
                ln_bayes: 40.0,
                delta_chi2: 1e4,
                prior: 0.05,
                periods_s: (1.0, 2.0),
                jitter: 0.0,
                folds: vec![Fold::new(3.0e5, 1.0)],
            }],
            retained: true,
        };
        let file = decode(2, &lc_proto::encode(&old)).expect("format 2 reads");
        let conclusion = &file.conclusions()[0];
        assert!(matches!(conclusion.transits[0].kind, Kind::Planet { class: Class::Rocky, .. }));
        assert!(matches!(conclusion.transits[1].kind, Kind::Unsearched));
        assert_eq!(conclusion.evidence.completeness, 0.0);
        assert_eq!(file.digests()[0].planet.as_ref().map(|p| p.folds.len()), Some(1));
        assert_eq!(file.series()[0].consumed_s(), 50.0, "and what was thrown away stays thrown away");
        assert!(file.retained);
    }

    #[test]
    fn format_3_reads() {
        let conclusion = ConclusionV3 {
            witness: Witness(4),
            observer: Witness(5),
            stated_s: 9.0,
            lineage: Vec::new(),
            transits: vec![Hypothesis { probability: 1.0, kind: Kind::Unsearched }],
            populations: vec![Hypothesis { probability: 1.0, kind: Kind::Belts { excess: None } }],
            evidence: Evidence { samples: 1, bands: 1, ln_bayes: 0.0, prior: 0.0, periods_s: None, completeness: 0.0, jitter: 0.0 },
            covering: covering(),
            discarded_s: None,
        };
        let old = FileV3 {
            sightings: Vec::new(),
            series: Vec::new(),
            claims: Vec::new(),
            names: vec![naming()],
            orbits: Vec::new(),
            conclusions: vec![conclusion],
            digests: Vec::new(),
            retained: false,
        };
        let file = decode(3, &lc_proto::encode(&old)).expect("format 3 reads");
        assert_eq!((file.conclusions()[0].observer, file.conclusions()[0].from_ly), (Witness(5), None));
        assert_eq!(file.names(), [naming()]);
    }

    #[test]
    fn the_current_format_round_trips_and_older_than_the_oldest_is_refused() {
        let mut file = File::default();
        file.names.push(naming());
        assert_eq!(decode(FILE_FORMAT, &lc_proto::encode(&file)).unwrap(), file);
        assert!(decode(0, &lc_proto::encode(&file)).is_err());
        assert!(decode(FILE_FORMAT + 1, &lc_proto::encode(&file)).is_err());
    }
}
