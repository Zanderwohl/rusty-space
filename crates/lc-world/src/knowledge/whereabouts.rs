//! Where a craft has been: which system, over what span, and inside each stay the sightings that
//! put it there, each held by every witness that claims it. Never forgotten. See
//! `lightcone/docs/22-provenance.md` §Where craft are.

use glam::DVec3;
use lc_proto::Scope;
use serde::{Deserialize, Serialize};

use super::thread::Seal;
use super::{Bearing, Lineage, Witness, learned_s};
use crate::flight::JULIAN_YEAR_S;
use crate::sky::StarId;

/// Seen sightings this close in emission time, and agreeing in place, are one moment witnessed
/// twice. Two observers at different ranges never catch exactly the same instant.
pub const SAME_MOMENT_S: f64 = 60.0;

/// How fast a believed place spreads with age, as a fraction of c: what a craft could have done
/// with its drive since. Without it a month-old fix reads as sharp as a fresh one.
pub const WANDER_BETA: f64 = 1e-4;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Its light, detected.
    Seen,
    /// A transmission or burn landing from out of sight.
    Heard,
    /// The craft's own word about itself, which may be a lie.
    Stated,
    /// A collapse's light where it was: the spike, or the afterglow cooling from the field's
    /// limit. Dated to the collapse, which the cooling says.
    Collapse,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub enum Fix {
    /// Where it was and how it was moving: light-years in the shard's frame, and velocity as a
    /// fraction of c.
    Place { position_ly: DVec3, beta: DVec3, sigma_ly: f64 },
    /// A direction from the witness, and the flux that came down it, W/m².
    Toward { bearing: Bearing, flux: f64 },
}

/// One witness's hold on a sighting.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Witnessed {
    pub witness: Witness,
    /// Coordinate seconds it reached the witness.
    pub observed_s: f64,
    /// Where this witness's claim may travel. Seen and heard are open; a stated place goes only
    /// where it was said.
    pub scope: Scope,
    pub lineage: Lineage,
}

impl Witnessed {
    pub fn learned_s(&self) -> f64 {
        learned_s(&self.lineage, self.observed_s)
    }
}

/// One moment of one craft, however many witnesses claim it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sighting {
    pub kind: Kind,
    /// Coordinate seconds the light, or the statement, left the craft.
    pub emitted_s: f64,
    /// `None` for the space between systems, or a bearing that settles none.
    pub system: Option<StarId>,
    pub fix: Fix,
    pub witnesses: Vec<Witnessed>,
}

impl Sighting {
    fn same_moment(&self, other: &Self) -> bool {
        if self.kind != other.kind || self.system != other.system {
            return false;
        }
        match (self.kind, self.fix, other.fix) {
            (Kind::Seen, Fix::Place { position_ly: a, sigma_ly: sa, .. }, Fix::Place { position_ly: b, sigma_ly: sb, .. }) => {
                (self.emitted_s - other.emitted_s).abs() <= SAME_MOMENT_S && a.distance(b) <= 3.0 * sa.hypot(sb)
            }
            // One collapse, dated by its light, wherever each witness put it.
            (Kind::Collapse, _, _) => self.emitted_s.to_bits() == other.emitted_s.to_bits(),
            // One statement or one transmission, landed on several.
            _ => self.emitted_s.to_bits() == other.emitted_s.to_bits() && self.fix == other.fix,
        }
    }

    /// Fold another claim of the same moment in. The sharper place stands; each witness is held
    /// once, by the first route it came.
    fn merge(&mut self, other: Sighting) {
        if let (Fix::Place { sigma_ly: held, .. }, Fix::Place { sigma_ly: new, .. }) = (self.fix, other.fix)
            && new < held
        {
            self.fix = other.fix;
            self.emitted_s = other.emitted_s;
        }
        for w in other.witnesses {
            if !self.witnesses.iter().any(|h| h.witness == w.witness) {
                self.witnesses.push(w);
            }
        }
    }

    /// Only the witnesses `seal` admits and `fresh` accepts, or `None` if none is left.
    fn sealed(&self, seal: Seal, fresh: impl Fn(f64) -> bool) -> Option<Sighting> {
        let witnesses: Vec<Witnessed> =
            self.witnesses.iter().filter(|w| seal.admits(w.scope) && fresh(w.learned_s())).cloned().collect();
        (!witnesses.is_empty()).then(|| Sighting { witnesses, ..self.clone() })
    }

    /// Where it was at coordinate time `at_s`, carried forward on the motion seen, and how far
    /// off that may be. `None` for a bearing alone.
    pub fn place_at(&self, at_s: f64) -> Option<(DVec3, f64)> {
        let Fix::Place { position_ly, beta, sigma_ly } = self.fix else { return None };
        let years = (at_s - self.emitted_s) / JULIAN_YEAR_S;
        Some((position_ly + beta * years, sigma_ly + WANDER_BETA * years.abs()))
    }

    /// Which way light sent from `from_ly` at `now_s` must go to meet the craft where this
    /// reckons it will be when the light gets there. `None` for a bearing alone.
    pub fn aim_from(&self, from_ly: DVec3, now_s: f64) -> Option<DVec3> {
        let (mut at, _) = self.place_at(now_s)?;
        // Converges because the craft is slower than its light: each pass shrinks the error by beta.
        for _ in 0..8 {
            let lands_s = now_s + from_ly.distance(at) * JULIAN_YEAR_S;
            at = self.place_at(lands_s)?.0;
        }
        Some((at - from_ly).normalize_or_zero())
    }
}

/// A craft in one system, or between systems, over the span its sightings cover.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Stay {
    pub system: Option<StarId>,
    /// In emission order; never empty.
    pub sightings: Vec<Sighting>,
}

impl Stay {
    /// First and last emission known to be here.
    pub fn span_s(&self) -> (f64, f64) {
        let first = self.sightings.first().map_or(f64::NAN, |s| s.emitted_s);
        let last = self.sightings.last().map_or(f64::NAN, |s| s.emitted_s);
        (first, last)
    }
}

/// Everything held on where one craft has been, oldest stay first.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Whereabouts {
    stays: Vec<Stay>,
}

impl Whereabouts {
    pub fn is_empty(&self) -> bool {
        self.stays.is_empty()
    }

    pub fn stays(&self) -> &[Stay] {
        &self.stays
    }

    pub fn sightings(&self) -> impl Iterator<Item = &Sighting> {
        self.stays.iter().flat_map(|s| s.sightings.iter())
    }

    /// Hold a sighting. False when it added nothing: the moment and every witness of it were
    /// already held.
    pub fn add(&mut self, sighting: Sighting) -> bool {
        let mut all: Vec<Sighting> = std::mem::take(&mut self.stays).into_iter().flat_map(|s| s.sightings).collect();
        let changed = match all.iter_mut().find(|held| held.same_moment(&sighting)) {
            Some(held) => {
                let before = held.witnesses.len();
                held.merge(sighting);
                held.witnesses.len() != before
            }
            None => {
                all.push(sighting);
                true
            }
        };
        // Sightings arrive out of order, relayed from anywhere, so the stays are regrouped whole.
        all.sort_by(|a, b| a.emitted_s.total_cmp(&b.emitted_s));
        for sighting in all {
            match self.stays.last_mut() {
                Some(stay) if stay.system == sighting.system => stay.sightings.push(sighting),
                _ => self.stays.push(Stay { system: sighting.system, sightings: vec![sighting] }),
            }
        }
        changed
    }

    /// The last stay and its best sighting: the latest, except that a craft's word about itself
    /// gives way to a sighting of it from the same moment.
    pub fn last_known(&self) -> Option<(&Stay, &Sighting)> {
        let stay = self.stays.last()?;
        let latest = stay.sightings.last()?;
        let best = match latest.kind {
            Kind::Stated => stay
                .sightings
                .iter()
                .rev()
                .take_while(|s| latest.emitted_s - s.emitted_s <= SAME_MOMENT_S)
                .find(|s| s.kind == Kind::Seen)
                .unwrap_or(latest),
            _ => latest,
        };
        Some((stay, best))
    }

    /// The latest sighting that places the craft, which a bearing heard since does not.
    pub fn last_placed(&self) -> Option<&Sighting> {
        self.sightings().filter(|s| matches!(s.fix, Fix::Place { .. })).max_by(|a, b| a.emitted_s.total_cmp(&b.emitted_s))
    }

    pub(super) fn sealed(&self, seal: Seal, fresh: impl Fn(f64) -> bool) -> Vec<Sighting> {
        self.sightings().filter_map(|s| s.sealed(seal, &fresh)).collect()
    }

    pub(super) fn learned_times(&self) -> impl Iterator<Item = f64> + '_ {
        self.sightings().flat_map(|s| s.witnesses.iter().map(Witnessed::learned_s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn star(key: u64) -> StarId {
        StarId::synthesize("test", key)
    }

    fn seen(by: u64, at_s: f64, system: Option<StarId>, x_ly: f64) -> Sighting {
        Sighting {
            kind: Kind::Seen,
            emitted_s: at_s,
            system,
            fix: Fix::Place { position_ly: DVec3::X * x_ly, beta: DVec3::ZERO, sigma_ly: 1e-9 },
            witnesses: vec![Witnessed { witness: Witness(by), observed_s: at_s + 10.0, scope: Scope::Open, lineage: Lineage::new() }],
        }
    }

    #[test]
    fn two_witnesses_of_one_moment_are_one_sighting_held_by_both() {
        let mut held = Whereabouts::default();
        assert!(held.add(seen(1, 100.0, Some(star(1)), 1.0)));
        assert!(held.add(seen(2, 130.0, Some(star(1)), 1.0)));
        assert!(!held.add(seen(2, 130.0, Some(star(1)), 1.0)));
        let all: Vec<_> = held.sightings().collect();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].witnesses.len(), 2);
    }

    #[test]
    fn stays_follow_the_systems_in_emission_order_whatever_order_they_arrive_in() {
        let (a, b) = (star(1), star(2));
        let mut held = Whereabouts::default();
        held.add(seen(1, 3_000.0, Some(b), 5.0));
        held.add(seen(1, 1_000.0, Some(a), 1.0));
        held.add(seen(1, 2_000.0, None, 3.0));
        held.add(seen(1, 1_500.0, Some(a), 1.0));
        let systems: Vec<_> = held.stays().iter().map(|s| s.system).collect();
        assert_eq!(systems, vec![Some(a), None, Some(b)]);
        assert_eq!(held.stays()[0].span_s(), (1_000.0, 1_500.0));
        assert_eq!(held.last_known().unwrap().1.emitted_s, 3_000.0);
    }

    #[test]
    fn a_place_spreads_as_it_ages() {
        let s = seen(1, 0.0, None, 1.0);
        let (_, fresh) = s.place_at(0.0).unwrap();
        let (_, old) = s.place_at(JULIAN_YEAR_S).unwrap();
        assert!((old - fresh - WANDER_BETA).abs() < 1e-12);
    }

    #[test]
    fn an_aim_leads_a_moving_craft_by_its_light_time() {
        let mut s = seen(1, 0.0, None, 1.0);
        s.fix = Fix::Place { position_ly: DVec3::X, beta: DVec3::Y * 0.1, sigma_ly: 0.0 };
        let aim = s.aim_from(DVec3::ZERO, 0.0).unwrap();
        // Light takes t years to cover the hypotenuse to (1, 0.1 t): t² = 1 + 0.01 t².
        let t = (1.0f64 / 0.99).sqrt();
        assert!(aim.distance(DVec3::new(1.0, 0.1 * t, 0.0).normalize()) < 1e-9, "{aim}");
    }

    #[test]
    fn a_stated_place_goes_only_where_it_was_said() {
        let key = lc_proto::FactionId(7);
        let mut stated = seen(1, 0.0, None, 1.0);
        stated.kind = Kind::Stated;
        stated.witnesses[0].scope = Scope::Faction(key);
        let mut held = Whereabouts::default();
        held.add(stated);
        held.add(seen(2, 10_000.0, None, 2.0));
        assert_eq!(held.sealed(Seal::Plain, |_| true).len(), 1);
        assert_eq!(held.sealed(Seal::Faction(key), |_| true).len(), 2);
    }
}
