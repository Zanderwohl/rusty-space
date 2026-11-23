//! Where a craft has been: which system, over what span, and inside each stay the sightings that
//! put it there, each held by every witness that claims it. Never forgotten, but a run of
//! sightings on one [`Trajectory`] keeps only its latest in full and the times of the rest. See
//! `lightcone/docs/22-provenance.md` §Where craft are.

use glam::DVec3;
use lc_proto::Scope;
use serde::{Deserialize, Serialize};

use super::thread::Seal;
use super::trajectory::Trajectory;
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
    /// The path the fix lies on, once the witness has seen enough of it to fit one.
    #[serde(default)]
    pub trajectory: Option<Trajectory>,
    /// Earlier emissions by the same witness that lie on `trajectory`, oldest first. Each one's
    /// place is the trajectory's at that time, so its own fix is not kept.
    #[serde(default)]
    pub thinned_s: Vec<f64>,
}

impl Sighting {
    /// The first emission this sighting stands for, thinned ones included.
    pub fn earliest_s(&self) -> f64 {
        self.thinned_s.first().copied().unwrap_or(self.emitted_s)
    }

    /// Seen by `next`'s lone witness, by the same route: the only sightings a run of it may fold.
    fn same_hand(&self, next: &Sighting) -> bool {
        let ([held], [new]) = (self.witnesses.as_slice(), next.witnesses.as_slice()) else { return false };
        self.kind == Kind::Seen
            && next.kind == Kind::Seen
            && self.system == next.system
            && held.witness == new.witness
            && held.scope == new.scope
            && held.lineage == new.lineage
    }

    /// Whether `next` only carries this one on: the same trajectory, from the same hand.
    fn continued_by(&self, next: &Sighting) -> bool {
        self.trajectory.is_some() && self.trajectory == next.trajectory && self.same_hand(next)
    }

    /// Whether `next` fitted a trajectory that this one lies on, every time it stands for named
    /// among the times `next` folds: a first fit folding full sightings, or a refit over a run.
    fn folded_by(&self, next: &Sighting) -> bool {
        let named = |t: f64| next.thinned_s.binary_search_by(|n| n.total_cmp(&t)).is_ok();
        next.trajectory.is_some()
            && self.trajectory != next.trajectory
            && self.same_hand(next)
            && named(self.emitted_s)
            && self.thinned_s.iter().all(|&t| named(t))
    }

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
    /// once, by the first route it came. False when it added nothing.
    fn merge(&mut self, other: Sighting) -> bool {
        let before = (self.witnesses.len(), self.thinned_s.len());
        if self.trajectory.is_some() && self.trajectory == other.trajectory {
            self.thinned_s = joined(std::mem::take(&mut self.thinned_s), other.thinned_s, self.emitted_s.max(other.emitted_s));
        }
        // Thinned times are places on this sighting's trajectory, so a fix on another one may not
        // take their run over.
        if let (Fix::Place { sigma_ly: held, .. }, Fix::Place { sigma_ly: new, .. }) = (self.fix, other.fix)
            && new < held
            && (self.thinned_s.is_empty() || self.trajectory == other.trajectory)
        {
            self.fix = other.fix;
            self.emitted_s = other.emitted_s;
            self.trajectory = other.trajectory;
        }
        for w in other.witnesses {
            if !self.witnesses.iter().any(|h| h.witness == w.witness) {
                self.witnesses.push(w);
            }
        }
        (self.witnesses.len(), self.thinned_s.len()) != before
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
        let first = self.sightings.iter().map(Sighting::earliest_s).reduce(f64::min).unwrap_or(f64::NAN);
        let last = self.sightings.last().map_or(f64::NAN, |s| s.emitted_s);
        (first, last)
    }
}

/// Two ascending lists of times as one, each time once, all below `before`.
fn joined(mut a: Vec<f64>, b: Vec<f64>, before: f64) -> Vec<f64> {
    a.extend(b);
    a.retain(|&t| t < before);
    a.sort_by(f64::total_cmp);
    a.dedup_by(|x, y| x.to_bits() == y.to_bits());
    a
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
        let latest_s = self.stays.last().and_then(|s| s.sightings.last()).map(|s| s.emitted_s);
        match latest_s {
            Some(latest_s) if sighting.emitted_s < latest_s => self.regroup(sighting),
            _ => self.append(sighting),
        }
    }

    /// A sighting no older than any held, which is how the roster files them. Touches only the
    /// last stay, so holding one costs nothing like the size of the file.
    fn append(&mut self, mut sighting: Sighting) -> bool {
        // A fit folds the sightings it was made from, which then stand only as times on it.
        if let Some(stay) = self.stays.last_mut()
            && !sighting.thinned_s.is_empty()
        {
            stay.sightings.retain(|held| !held.folded_by(&sighting));
            if stay.sightings.is_empty() {
                self.stays.pop();
            }
        }
        let Some(stay) = self.stays.last_mut() else {
            self.stays.push(Stay { system: sighting.system, sightings: vec![sighting] });
            return true;
        };
        let floor_s = sighting.emitted_s - SAME_MOMENT_S;
        let same = stay
            .sightings
            .iter()
            .enumerate()
            .rev()
            .take_while(|(_, held)| held.emitted_s >= floor_s)
            .find(|(_, held)| held.same_moment(&sighting))
            .map(|(at, _)| at);
        if let Some(at) = same {
            let changed = stay.sightings[at].merge(sighting);
            stay.sightings.sort_by(|a, b| a.emitted_s.total_cmp(&b.emitted_s));
            return changed;
        }
        if stay.system != sighting.system {
            self.stays.push(Stay { system: sighting.system, sightings: vec![sighting] });
            return true;
        }
        if let Some(last) = stay.sightings.last_mut()
            && last.continued_by(&sighting)
        {
            let mut earlier = std::mem::take(&mut last.thinned_s);
            earlier.push(last.emitted_s);
            sighting.thinned_s = joined(earlier, std::mem::take(&mut sighting.thinned_s), sighting.emitted_s);
            *last = sighting;
            return true;
        }
        stay.sightings.push(sighting);
        true
    }

    /// A sighting older than the latest held: relayed from anywhere, so the stays are regrouped
    /// whole.
    fn regroup(&mut self, sighting: Sighting) -> bool {
        let mut all: Vec<Sighting> = std::mem::take(&mut self.stays).into_iter().flat_map(|s| s.sightings).collect();
        let changed = match all.iter_mut().find(|held| held.same_moment(&sighting)) {
            Some(held) => held.merge(sighting),
            None => {
                all.push(sighting);
                true
            }
        };
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
            trajectory: None,
            thinned_s: Vec::new(),
        }
    }

    fn on(trajectory: Trajectory, by: u64, at_s: f64) -> Sighting {
        Sighting { trajectory: Some(trajectory), ..seen(by, at_s, Some(star(1)), 1.0) }
    }

    fn line(x_ly: f64) -> Trajectory {
        Trajectory::Line { position_ly: DVec3::X * x_ly, beta: DVec3::ZERO, epoch_s: 0.0 }
    }

    #[test]
    fn a_fit_folds_the_sightings_it_was_made_from_and_no_others() {
        let mut held = Whereabouts::default();
        for at_s in [1_000.0, 2_000.0, 3_000.0] {
            held.add(seen(1, at_s, Some(star(1)), 1.0));
        }
        held.add(seen(2, 3_500.0, Some(star(1)), 1.0));
        let fitted = Sighting { thinned_s: vec![2_000.0, 3_000.0], ..on(line(1.0), 1, 4_000.0) };
        assert!(held.add(fitted));
        let left: Vec<(f64, usize)> = held.sightings().map(|s| (s.emitted_s, s.thinned_s.len())).collect();
        assert_eq!(left, vec![(1_000.0, 0), (3_500.0, 0), (4_000.0, 2)], "1 000 was not in the fit, 3 500 is another witness's");
        assert_eq!(held.stays()[0].span_s(), (1_000.0, 4_000.0));
    }

    #[test]
    fn a_run_on_one_track_keeps_the_latest_sighting_and_the_times_of_the_rest() {
        let mut held = Whereabouts::default();
        for at_s in [1_000.0, 2_000.0, 3_000.0] {
            assert!(held.add(on(line(1.0), 1, at_s)));
        }
        let all: Vec<_> = held.sightings().collect();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].emitted_s, 3_000.0);
        assert_eq!(all[0].thinned_s, vec![1_000.0, 2_000.0]);
        assert_eq!(held.stays()[0].span_s(), (1_000.0, 3_000.0));
    }

    #[test]
    fn another_track_or_another_witness_starts_a_new_run() {
        let mut held = Whereabouts::default();
        held.add(on(line(1.0), 1, 1_000.0));
        held.add(on(line(2.0), 1, 2_000.0));
        held.add(on(line(2.0), 2, 3_000.0));
        assert_eq!(held.sightings().count(), 3);
        assert!(held.sightings().all(|s| s.thinned_s.is_empty()));
    }

    #[test]
    fn a_run_relayed_twice_holds_each_time_once() {
        let mut sender = Whereabouts::default();
        let mut receiver = Whereabouts::default();
        for at_s in [1_000.0, 2_000.0] {
            sender.add(on(line(1.0), 1, at_s));
        }
        receiver.add(sender.sightings().next().unwrap().clone());
        sender.add(on(line(1.0), 1, 3_000.0));
        receiver.add(sender.sightings().next().unwrap().clone());
        assert!(!receiver.add(sender.sightings().next().unwrap().clone()));
        let all: Vec<_> = receiver.sightings().collect();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].thinned_s, vec![1_000.0, 2_000.0]);
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
