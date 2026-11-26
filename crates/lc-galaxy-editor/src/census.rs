//! Every baked star's system, counted on a worker thread after each rebuild, for Statistics and
//! Charts. Runs only while one of them is open.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use lc_galaxy::FeatureId;
use lc_galaxy::sample::default_threads;
use lc_world::sky::generate::census::Census;

use crate::doc::Doc;

pub struct Counted {
    pub population: Option<FeatureId>,
    pub feh: f64,
    pub initial_mass_solar: f64,
    /// `None` for a star the sky leaves out, which has no system.
    pub census: Option<Census>,
}

type Outcome = Result<Arc<Vec<Counted>>, &'static str>;

#[derive(Default)]
pub enum CensusJob {
    #[default]
    NotDispatched,
    /// `previous` stays on screen until this one finishes, so a drag does not blank the panels.
    Waiting { generation: u64, slot: Arc<OnceLock<Outcome>>, previous: Option<(u64, Outcome)> },
    Ready { generation: u64, results: Outcome },
}

impl CensusJob {
    /// Picks up a finished census, and starts one of the current bake if none is running. One runs
    /// at a time: a drag rebuilds every few frames, and only the newest bake is counted after it.
    pub fn update(&mut self, doc: &Doc) {
        if let CensusJob::Waiting { generation, slot, .. } = self
            && let Some(outcome) = slot.get()
        {
            *self = CensusJob::Ready { generation: *generation, results: outcome.clone() };
        }
        match self {
            CensusJob::Waiting { .. } => {}
            CensusJob::Ready { generation, .. } if *generation == doc.generation => {}
            _ => {
                let previous = match std::mem::take(self) {
                    CensusJob::Ready { generation, results } => Some((generation, results)),
                    _ => None,
                };
                *self = dispatch(doc, doc.generation, previous);
            }
        }
    }

    /// The newest finished census and the generation it counted, for anything cached on it.
    pub fn latest(&self) -> Option<(u64, &Outcome)> {
        match self {
            CensusJob::NotDispatched => None,
            CensusJob::Waiting { previous, .. } => previous.as_ref().map(|(g, p)| (*g, p)),
            CensusJob::Ready { generation, results } => Some((*generation, results)),
        }
    }

    /// Whether a census newer than [`Self::latest`] is running.
    pub fn running(&self) -> bool {
        matches!(self, CensusJob::Waiting { .. })
    }
}

fn dispatch(doc: &Doc, generation: u64, previous: Option<(u64, Outcome)>) -> CensusJob {
    let compiled = doc.result.as_ref().map(|r| &r.compiled);
    let mut kinematics = HashMap::new();
    let inputs: Vec<_> = doc
        .stars()
        .iter()
        .map(|s| {
            let a = &s.attributes;
            let k = a.population.and_then(|p| {
                kinematics.entry(p).or_insert_with(|| compiled.and_then(|c| lc_galaxy::attributes::kinematics(c, p))).clone()
            });
            (s.clone(), k)
        })
        .collect();
    let slot = Arc::new(OnceLock::new());
    let filled = slot.clone();
    let spawned = std::thread::Builder::new().name("census".into()).spawn(move || {
        // Caught so the slot is always filled: an empty one would leave the panels waiting.
        let outcome = std::panic::catch_unwind(|| {
            let share = inputs.len().div_ceil(default_threads()).max(1);
            std::thread::scope(|scope| {
                let parts: Vec<_> = inputs
                    .chunks(share)
                    .map(|part| {
                        scope.spawn(move || {
                            part.iter()
                                .map(|(s, k)| Counted {
                                    population: s.attributes.population,
                                    feh: s.attributes.feh,
                                    initial_mass_solar: s.attributes.initial_mass_solar,
                                    census: k.as_ref().and_then(|k| lc_galaxy::stats::census_of(s, k)),
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                parts.into_iter().flat_map(|p| p.join().expect("a census thread panicked")).collect::<Vec<_>>()
            })
        })
        .map(Arc::new)
        .map_err(|_| "taking the census panicked");
        let _ = filled.set(outcome);
    });
    match spawned {
        Ok(_) => CensusJob::Waiting { generation, slot, previous },
        Err(_) => CensusJob::Ready { generation, results: Err("no worker thread could be started") },
    }
}
