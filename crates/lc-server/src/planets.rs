//! Turning a settled transit into a body a craft believes in.
//!
//! The shard's job rather than the knowledge layer's, for one reason: which body a transit *is*
//! cannot be answered from the light curve. A transit measures a period, and only the arena
//! knows whether a planet with that period exists. Every *number* here still comes from what the
//! craft knows — the period from its own search, the distance through its own mass prior — and
//! truth is consulted for identity alone. See
//! `lightcone/docs/25-system-knowledge.md#which-body-a-transit-is`.

use lc_world::craft::CraftId;
use lc_world::knowledge::conclusion::{Kind, SETTLED, TRANSITS_TO_SETTLE};
use lc_world::knowledge::prior::Prior;
use lc_world::knowledge::record::{Method, Orbit, Orientation};
use lc_world::knowledge::transit::Candidate;
use lc_world::knowledge::{BodyId, Conclusion, Subject, Witness};
use lc_world::navigation::Kind as BodyKind;
use lc_world::navigation::Target;
use lc_world::sky::StarId;
use lc_world::system::LocalSystem;

use crate::journal::Journal;
use crate::server::Server;

/// How many sigma of the measured period a true planet may sit away and still be called the
/// same body.
///
/// Three, because the alternative to matching is inventing a body nobody else can confirm, and
/// a near miss is far more likely to be the planet than a coincidence: a system's planets are
/// decades apart in period, so there is nothing else within three sigma to be confused with.
const MATCH_SIGMA: f64 = 3.0;

/// The floor under that window, as a fraction of the period.
///
/// A box-least-squares period from a long log can have a sigma of minutes, which no truth lookup
/// deserves to be held to: the search's own grid is coarser than its formal error.
const MATCH_FLOOR: f64 = 0.02;

/// A settled transiting planet in a conclusion, or `None`.
///
/// Settled is both tests the search applies: probable past [`SETTLED`] *and* seen to transit
/// [`TRANSITS_TO_SETTLE`] times. One without the other is a candidate, which the map draws
/// faintly and this does not mint.
pub fn settled_planet(conclusion: &Conclusion) -> Option<Candidate> {
    let leading = conclusion.leading()?;
    let Kind::Planet { transit, .. } = leading.kind else {
        return None;
    };
    (leading.probability >= SETTLED && transit.transits >= TRANSITS_TO_SETTLE).then_some(transit)
}

/// Which body a settled transit is.
///
/// On a period match, the planet's own id, so a craft that later images the same planet folds
/// into one body rather than two. A candidate that matches nothing is a **false positive**, and
/// gets an id derived from the star, the witness and the period: this craft's own later transits
/// of its own phantom land on the same id, and no other craft can ever agree with it, which is
/// correct — they have no shared object to agree about.
pub fn identify(
    system: &LocalSystem,
    star: StarId,
    witness: Witness,
    transit: &Candidate,
) -> BodyId {
    let window = (MATCH_SIGMA * transit.period_sigma_s).max(MATCH_FLOOR * transit.period_s);
    let best = system
        .inventory()
        .iter()
        .filter(|entry| entry.kind == BodyKind::Planet)
        .filter_map(|entry| {
            let Target::Body(key) = &entry.target else { return None };
            // The orbit's own period, from its semi-major axis. Not from how far out the body
            // happened to be when the inventory was taken: those differ by `1 +- e`, so the
            // period is out by `(1 +- e)^1.5`, which is thirty per cent for Mercury and enough
            // past an eccentricity of 0.013 to miss this window entirely -- and a real planet
            // that misses it is minted as a phantom that never merges with the surveyed body.
            let period = system.period_of_target(key)?;
            let miss = (period - transit.period_s).abs();
            (miss <= window).then_some((miss, key))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    match best {
        Some((_, key)) => BodyId::of(star, key),
        // Bucketed rather than raw, so the same phantom found twice is one body. Log-spaced,
        // because what "the same period" means scales with the period.
        None => {
            let bucket = (transit.period_s.max(1.0).ln() / MATCH_FLOOR).round() as i64;
            BodyId::phantom(star, witness, bucket)
        }
    }
}

impl<J: Journal> Server<J> {
    /// File a settled transit as a planet, letter it, and give it the orbit the transit implies.
    ///
    /// The orbit is `EdgeOnTo` the line of sight it was seen along: a transit says the pole is
    /// perpendicular to that line, somewhere on a great circle, and the mid-transit time puts
    /// the planet on the line at a known instant. Crossing that with another craft's is what
    /// settles a pole, which is why orientation from outside a system is cooperative.
    ///
    /// Returns the letter assigned, for a test or a notification to read.
    pub(crate) fn found_by_transit(
        &mut self,
        id: CraftId,
        subject: Subject,
        transit: &Candidate,
        now_s: f64,
    ) -> Option<String> {
        let star = subject.as_star()?;
        let witness = crate::instruments::witness(id);

        // Everything measured comes from the craft's own belief: the mass prior it would have
        // used, and the bearing it watched along. Truth is asked which body this is and
        // nothing else.
        let belief = self.instruments.aboard.get(&id)?.knowledge.belief(subject)?;
        let toward = belief.bearing.toward;
        let luminosity_w = belief.luminosity_w()?;
        let band = belief.band;
        let stars = self.world.stars();
        let prior = self.instruments.prior.get_or_insert_with(|| Prior::measure(stars.iter()));
        let (mu, mu_fraction) = prior.host_mass(band, luminosity_w)?;
        let luminosity_solar = luminosity_w / em_spectra::stellar::SOLAR_LUMINOSITY;

        let system = self.world.system_for(star)?;
        let body = identify(&system, star, witness, transit);

        let orbit = Orbit::from_period(
            witness,
            (transit.period_s, transit.period_sigma_s),
            mu,
            mu_fraction,
            Orientation::EdgeOnTo { toward },
            Some(transit.epoch_s),
            Method::Transit,
            now_s,
        );
        let aboard = self.instruments.aboard.get_mut(&id)?;
        Some(aboard.knowledge.found_planet(star, body, orbit, luminosity_solar, now_s))
    }
}

#[cfg(test)]
mod tests {
    use lc_world::knowledge::conclusion::{Class, Evidence, Hypothesis};
    use lc_world::knowledge::record::Lineage;
    use lc_world::sky::{AuthoredStars, StarProvider};

    use super::*;

    /// The authored star whose generated system has planets.
    fn system() -> (lc_world::sky::CatalogueStar, LocalSystem) {
        let stars = AuthoredStars::sample();
        let star = StarProvider::stars(&stars)[2].clone();
        let system = LocalSystem::for_star(&star).expect("a generated system");
        (star, system)
    }

    fn candidate(period_s: f64, transits: u32) -> Candidate {
        Candidate {
            period_s,
            period_sigma_s: period_s * 1.0e-4,
            epoch_s: 0.0,
            duration_s: 3600.0,
            depth: 1.0e-3,
            depth_sigma: 1.0e-5,
            delta_chi2: 1.0e4,
            transits,
        }
    }

    fn conclusion(probability: f64, transit: Candidate) -> Conclusion {
        Conclusion {
            witness: Witness(1),
            observer: Witness(1),
            from_ly: None,
            stated_s: 0.0,
            lineage: Lineage::new(),
            transits: vec![Hypothesis {
                probability,
                kind: Kind::Planet { class: Class::Rocky, transit },
            }],
            populations: Vec::new(),
            evidence: Evidence {
                samples: 1,
                bands: 1,
                ln_bayes: 0.0,
                prior: 0.0,
                periods_s: None,
                completeness: 0.0,
                jitter: 0.0,
            },
            covering: lc_world::knowledge::conclusion::Covering {
                observed_s: (0.0, 1.0),
                light_age_s: None,
            },
            discarded_s: None,
        }
    }

    /// **Both tests, not either.** A planet probable enough but seen once is a candidate the map
    /// draws faintly, and minting it would put a body on the chart off one dip in a light curve.
    #[test]
    fn a_planet_is_minted_only_when_probable_and_seen_enough() {
        let enough = candidate(3.0e6, TRANSITS_TO_SETTLE);
        assert!(settled_planet(&conclusion(SETTLED, enough)).is_some());
        assert!(settled_planet(&conclusion(SETTLED - 0.01, enough)).is_none(), "not probable");
        assert!(
            settled_planet(&conclusion(SETTLED, candidate(3.0e6, TRANSITS_TO_SETTLE - 1))).is_none(),
            "not seen enough times"
        );

        // A quiet star mints nothing whatever its probability.
        let mut quiet = conclusion(1.0, enough);
        quiet.transits = vec![Hypothesis { probability: 1.0, kind: Kind::Quiet }];
        assert!(settled_planet(&quiet).is_none());
    }

    /// A transit whose period is a real planet's is *that* planet, so a craft that images it
    /// later folds into one body rather than two.
    ///
    /// **Every planet, and by its own period.** This checked one, with a period worked out the
    /// same wrong way the code did -- from how far out the body happened to be rather than from
    /// its semi-major axis -- so the two agreed and the test could not see that both were
    /// wrong. A body's distance and its axis differ by `1 +- e`.
    #[test]
    fn a_matching_period_is_the_planet_it_matches() {
        let (star, system) = system();
        let planets: Vec<_> = system
            .inventory()
            .iter()
            .filter(|e| e.kind == BodyKind::Planet)
            .cloned()
            .collect();
        assert!(planets.len() > 3, "only {} planets to match", planets.len());

        for planet in &planets {
            let Target::Body(key) = &planet.target else { panic!("a planet is a body") };
            let period = system.period_of_target(key).expect("a planet has a period");
            let found = identify(&system, star.id, Witness(1), &candidate(period, 3));
            assert_eq!(found, BodyId::of(star.id, key), "{key} did not match its own period");

            // And a little off is still it: the search's period is never exact.
            let near = identify(&system, star.id, Witness(1), &candidate(period * 1.005, 3));
            assert_eq!(near, found, "{key}: a half-percent miss is the same planet");
        }
    }

    /// **The distance is not the axis.** A period taken from where a body happens to be is out
    /// by `(1 +- e)^1.5`, which past an eccentricity of about 0.013 is wider than the window a
    /// transit is matched in -- so the real planet missed its own entry and was minted as a
    /// phantom that no later imaging could ever merge with.
    #[test]
    fn an_eccentric_planet_still_matches_itself() {
        let (star, system) = system();
        let mut checked = 0;
        for entry in system.inventory().iter().filter(|e| e.kind == BodyKind::Planet) {
            let Target::Body(key) = &entry.target else { continue };
            let truth = system.period_of_target(key).expect("a planet has a period");
            // What the old reading gave: the third law on the distance at the inventory epoch.
            let from_distance =
                em_foundations::kepler::period::third_law(entry.orbit_radius_m, star.star.mu);
            if (from_distance / truth - 1.0).abs() < 0.02 {
                continue;
            }
            checked += 1;
            // Its own period finds it.
            assert_eq!(
                identify(&system, star.id, Witness(1), &candidate(truth, 3)),
                BodyId::of(star.id, key),
                "{key} does not match its own period"
            );
            // The distance's period is a different body or none, which is the whole defect.
            assert_ne!(
                identify(&system, star.id, Witness(1), &candidate(from_distance, 3)),
                BodyId::of(star.id, key),
                "{key}: the distance's period should not have found it"
            );
        }
        assert!(checked > 0, "no planet here is eccentric enough to show it");
    }

    /// A transit matching no planet is a false positive: a body only this craft believes in,
    /// which nobody else can ever confirm.
    #[test]
    fn a_period_matching_nothing_is_one_crafts_phantom() {
        let (star, system) = system();
        let mu = star.star.mu;
        // Far inside the innermost planet, where the generator puts nothing.
        let nonsense = candidate(600.0, 3);

        let mine = identify(&system, star.id, Witness(1), &nonsense);
        let theirs = identify(&system, star.id, Witness(2), &nonsense);
        assert_ne!(mine, theirs, "two craft's phantoms must never merge");

        // But one craft's own repeated transits of its own phantom are one body.
        assert_eq!(mine, identify(&system, star.id, Witness(1), &nonsense));
        let again = identify(&system, star.id, Witness(1), &candidate(601.0, 4));
        assert_eq!(mine, again, "the same phantom found twice is one body");

        // And it is not any real planet.
        for entry in system.inventory().iter() {
            if let Target::Body(key) = &entry.target {
                assert_ne!(mine, BodyId::of(star.id, key), "a phantom collided with {key}");
            }
        }
    }
}
