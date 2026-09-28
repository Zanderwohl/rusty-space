//! Which body a body goes round, and filing the orbit that says so.
//!
//! **A Keplerian orbit puts its primary at a focus**, so the candidate that works as a focus is
//! the primary. The same test finds the star for a planet, the planet for a moon and the moon
//! for a moon's moon, and nothing here is a moon case: [`super::arc`] fits an orbit about
//! whatever frame it is handed, and this decides which frames to hand it.
//!
//! See `lightcone/docs/25-system-knowledge.md`, "The primary is at a focus". Tested from
//! [`super::arc`], whose fixtures build the orbits these decide between.

use glam::DVec3;

use super::arc::{self, Fitted, Look, LOOKS_NEEDED, RANGED_NEEDED};
use super::{BodyId, Subject};
use crate::sky::StarId;

/// Primaries tried besides the star, in order of how nearly the body keeps station with them.
///
/// The mean bearing of a body over its orbit points at its primary: seen from outside, a
/// satellite's apparent path is a closed loop about the thing it goes round, and the middle of
/// that loop is the thing. So ranking candidates by the angle between their believed direction
/// and the body's own mean bearing puts the right one first, at both levels -- a planet's mean
/// bearing points at the star, a moon's at its planet.
const PRIMARIES_TRIED: usize = 3;

/// Hill-sphere bounds on a satellite: a period under the primary's own over sqrt 3, where the
/// masses cancel, and an axis under its own over cbrt 3, for any primary lighter than its star.
const HILL_PERIOD: f64 = 0.577_350_269_189_625_8;
const HILL_REACH: f64 = 0.693_361_274_350_634_7;

/// How far after the orbit it replaces a fit is stated, when both are filed at one instant.
const FILED_AFTER_S: f64 = 1.0e-3;

/// How much longer an arc must be than at the last attempt before a body is fitted again.
const REFIT_GROWTH: f64 = 1.5;

/// A fit attempted, how long an arc it had, and the last orbit fitted and the primary it is
/// about, which a refit starts from. See [`Knowledge::unfitted`] and [`arc::refit`].
///
/// Not saved: after a restart the first fit of each body searches from scratch, which costs
/// time and not correctness.
#[derive(Clone, Debug)]
pub(crate) struct Attempt {
    pub at_s: f64,
    pub span_s: f64,
    /// Ranged looks it had. More of them re-arms it whatever the span: see [`newly_ranged`].
    pub ranged: usize,
    /// When the looks behind the newest fit filed were taken. A fit on older looks that finishes
    /// after it is refused rather than filed over it.
    pub filed_taken_s: f64,
    pub last: Option<Held>,
}

/// The last orbit fitted, its primary, and every primary it was chosen over.
#[derive(Clone, Debug)]
pub(crate) struct Held {
    about: Option<BodyId>,
    fitted: Fitted,
    offered: Vec<Option<BodyId>>,
}

/// Seconds from the oldest look held to the newest. Decimation keeps the ends, so this only
/// grows.
fn span_s(sightings: &[super::Sighting]) -> f64 {
    let (first, last) = sightings
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(s.observed_s), hi.max(s.observed_s)));
    (last - first).max(0.0)
}

fn ranged(sightings: &[super::Sighting]) -> usize {
    sightings.iter().filter(|s| s.range_m.is_some()).count()
}

/// Whether a body has become solvable outright since it was last tried. A close pass can do
/// that in minutes of an arc hundreds of hours long, and waiting for the span to grow would
/// leave a planet the ship is looking at unplaced.
fn newly_ranged(now: usize, tried: Option<&Attempt>) -> bool {
    now >= RANGED_NEEDED && tried.is_none_or(|t| now > t.ranged)
}

impl crate::knowledge::Knowledge {
    /// The bearings held about a body, in the frame of a primary that was at `primary_at` at
    /// each look's own time.
    ///
    /// Believed, not true. The observer positions are the ship's own and exact; a primary's is
    /// a belief with its own error, and an error there shifts the looks and biases the orbit.
    pub fn looks_at(
        &self,
        subject: Subject,
        primary_at: &dyn Fn(f64) -> Option<DVec3>,
    ) -> Vec<Look> {
        self.file(subject).map_or_else(Vec::new, |file| {
            file.sightings()
                .iter()
                .filter_map(|seen| Some(Look::of(seen, primary_at(seen.observed_s)?)))
                .collect()
        })
    }

    /// The body of `star` most in need of an orbit: one whose arc has grown since it was last
    /// attempted, longest since that attempt first.
    ///
    /// **By attempt, not by statement.** An arc that cannot yet shape an orbit states nothing,
    /// so ranking by what was stated leaves that body at the front of the queue on every tick
    /// and nothing else in the system is ever fitted. Saturn on a three-month arc is enough to
    /// starve Venus, Earth, Mars and Jupiter indefinitely.
    ///
    /// **And gated on growth, not on a new look.** A survey adds a look to every body each
    /// rotation, so "anything newer" re-armed the whole system forever and a fit -- far more
    /// than a tick's work -- ran on every tick while anyone surveyed. The arc's span has to
    /// reach [`REFIT_GROWTH`] times what the last attempt saw, which is a handful of refits per
    /// decade of arc: a failed fit waits for its inputs to change, and a good one is revisited
    /// as the arc it stands on lengthens.
    ///
    /// **Except for new ranges**, which make a fit a solution rather than a search: a body with
    /// more of them than when last tried goes back in, ahead of the rest.
    ///
    /// **Or a surprise**: a look far from where the orbit said, which says the orbit is wrong
    /// however long its arc has been. See `knowledge::innovation`.
    pub fn unfitted(&self, star: StarId) -> Option<Subject> {
        let mine = |subject: Subject| -> Option<(bool, f64)> {
            let file = self.file(subject)?;
            if file.sightings().len() < LOOKS_NEEDED {
                return None;
            }
            let span = span_s(file.sightings());
            let newest = file.sightings().iter().map(|s| s.observed_s).fold(f64::MIN, f64::max);
            let stated = file
                .orbits()
                .iter()
                .filter(|o| o.witness == self.owner && o.method == crate::knowledge::Method::Astrometric)
                .map(|o| o.stated_s)
                .fold(f64::MIN, f64::max);
            let tried = self.tried.get(&subject);
            let fresh = newly_ranged(ranged(file.sightings()), tried) || self.surprised.contains(&subject);
            if !fresh && tried.is_some_and(|t| span < t.span_s * REFIT_GROWTH) {
                return None;
            }
            let tried_s = tried.map_or(f64::MIN, |t| t.at_s);
            (fresh || newest > stated).then_some((!fresh, stated.max(tried_s)))
        };
        self.members(star)
            .filter_map(|(subject, _)| match subject {
                Subject::Body { .. } => Some((mine(subject)?, subject)),
                _ => None,
            })
            .min_by(|a, b| a.0.0.cmp(&b.0.0).then(a.0.1.total_cmp(&b.0.1)).then(a.1.cmp(&b.1)))
            .map(|(_, subject)| subject)
    }

    /// Whether this craft has fitted an orbit to `subject` from its own bearings.
    pub fn orbited(&self, subject: Subject) -> bool {
        self.file(subject).is_some_and(|file| {
            file.orbits()
                .iter()
                .any(|o| o.witness == self.owner && o.method == crate::knowledge::Method::Astrometric)
        })
    }

    /// The mean direction a body was seen in, which points at whatever it goes round.
    fn mean_bearing(&self, subject: Subject) -> Option<DVec3> {
        let file = self.file(subject)?;
        let mean: DVec3 = file.sightings().iter().map(|s| s.bearing.toward).sum();
        (mean.length_squared() > 0.0).then(|| mean.normalize())
    }

    fn mean_flux(&self, subject: Subject, band: em_spectra::Band) -> Option<f64> {
        let (sum, count) = self
            .file(subject)?
            .sightings()
            .iter()
            .filter(|s| s.band == band)
            .fold((0.0, 0usize), |(sum, n), s| (sum + s.flux, n + 1));
        (count > 0).then(|| sum / count as f64)
    }

    /// A satellite and its primary are at one distance, so the brighter is the bigger. `false`
    /// with no band in common.
    fn outshines(&self, primary: Subject, satellite: Subject) -> bool {
        em_spectra::Band::ALL.into_iter().find_map(|band| {
            Some(self.mean_flux(primary, band)? > self.mean_flux(satellite, band)?)
        }) == Some(true)
    }

    pub fn beside_brighter(&self, subject: Subject, within_rad: f64) -> bool {
        let latest = |subject: Subject| {
            let seen = self.file(subject)?.sightings().iter().max_by(|a, b| a.observed_s.total_cmp(&b.observed_s))?;
            Some(seen.bearing.toward)
        };
        let (Some(star), Some(toward)) = (subject.star(), latest(subject)) else { return false };
        self.members(star).any(|(other, _)| {
            matches!(other, Subject::Body { .. })
                && other != subject
                && latest(other).is_some_and(|there| there.angle_between(toward) < within_rad)
                && self.outshines(other, subject)
        })
    }

    /// Candidate primaries for a body: the star, then the nearest few in the sky that outshine it.
    ///
    /// **Not a list of moons.** A Keplerian orbit puts its primary at a focus, so the candidate
    /// that works as a focus is the primary, and trying the star alongside the rest is what
    /// keeps a planet from being handed to one of its neighbors.
    ///
    /// Brighter only: six elements fit sixteen bearings about almost any point near the true
    /// primary, and a planet's nearest neighbors are its own moons. A strict order also rules
    /// out cycles.
    fn primaries(&self, star: StarId, subject: Subject, now_s: f64) -> Vec<Option<BodyId>> {
        let Some(toward) = self.mean_bearing(subject) else { return vec![None] };
        let mut near: Vec<(f64, BodyId)> = self
            .members(star)
            .filter_map(|(other, _)| match other {
                Subject::Body { body, .. } if other != subject && self.outshines(other, subject) => {
                    let seen = self.mean_bearing(other)?;
                    Some((seen.angle_between(toward), body))
                }
                _ => None,
            })
            .filter(|(angle, _)| angle.is_finite())
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        // Only ones this craft can actually place, since a frame needs a position.
        let mut tried: Vec<Option<BodyId>> = vec![None];
        for (_, body) in near {
            if tried.len() > PRIMARIES_TRIED {
                break;
            }
            if self.placed(star, body, now_s).is_some() {
                tried.push(Some(body));
            }
        }
        tried
    }

    /// At each look held, how far the believed place misses the ray the body was seen along, at
    /// the believed distance. Meters, in time order.
    ///
    /// Corrects a moon's frame: across the line of sight a planet's bearings place it to 75 km
    /// from 5 AU, where its orbit may be 0.005 AU out, wider than Europa's. A position rather
    /// than an angle, so it varies only as the orbit's error does and interpolates between the
    /// few looks kept.
    fn sightlines(&self, star: StarId, body: BodyId, star_ly: DVec3) -> Vec<(f64, DVec3)> {
        let Some(file) = self.file(Subject::Body { star, body }) else { return Vec::new() };
        let mut out: Vec<(f64, DVec3)> = file
            .sightings()
            .iter()
            .filter_map(|seen| {
                // Relative to the star: absolute meters lose tens of km at 30,000 ly.
                let believed = self.placed(star, body, seen.observed_s)?;
                let from = (seen.bearing.observer_ly - star_ly) * crate::system::M_PER_LY;
                let depth = (believed - from).length();
                arc::sound(depth).then(|| (seen.observed_s, from + seen.bearing.toward * depth - believed))
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    /// Where a body is believed to be, as an offset from its star in meters, or `None`.
    pub(crate) fn placed(&self, star: StarId, body: BodyId, at_s: f64) -> Option<DVec3> {
        match self.body_belief(star, body, at_s)?.position_now {
            crate::knowledge::Placed::Known { offset_au, .. } => {
                Some(offset_au * crate::navigation::AU)
            }
            _ => None,
        }
    }

    /// Fit an orbit to what is held about one body and file it, against each candidate primary
    /// in turn and keeping whichever explains the bearings best. `false` when none of them does.
    ///
    /// `Knowledge::orbits` keeps one statement per witness and the later wins, so a refit
    /// replaces the craft's own earlier one rather than piling up beside it.
    ///
    /// All three steps at once. A shard runs [`FitJob::solve`] off its tick instead, because a
    /// fit costs seconds.
    pub fn fit_orbit(&mut self, subject: Subject, star_ly: DVec3, now_s: f64) -> bool {
        let Some(job) = self.fit_job(subject, star_ly, now_s) else { return false };
        match job.solve() {
            Some(solved) => self.file_fit(solved, now_s),
            None => false,
        }
    }

    /// Everything a fit of `subject` needs, taken now and owned, and the attempt recorded so
    /// the queue moves on. See [`Knowledge::unfitted`].
    pub fn fit_job(&mut self, subject: Subject, star_ly: DVec3, now_s: f64) -> Option<FitJob> {
        let star = subject.star()?;
        let frames = self
            .primaries(star, subject, now_s)
            .into_iter()
            .map(|about| {
                let seen = about.map_or_else(Vec::new, |body| self.sightlines(star, body, star_ly));
                let at = |t: f64| match about {
                    None => Some(star_ly),
                    Some(body) => {
                        Some(star_ly + (self.placed(star, body, t)? + along(&seen, t)) / crate::system::M_PER_LY)
                    }
                };
                let held = about.and_then(|body| self.body_belief(star, body, now_s));
                let bound = |element: Option<(f64, f64)>, scale: f64| element.map_or(f64::INFINITY, |(v, _)| v * scale);
                let looks = self.looks_at(subject, &at);
                // The frame keeps the believed depth, whose error scales the whole orbit. The
                // sightlines' miss is only a floor: bearings pin depth far worse than the miss.
                let missed_m = (seen.iter().map(|(_, c)| c.length_squared()).sum::<f64>()
                    / seen.len().max(1) as f64)
                    .sqrt();
                let depth = looks.last().map_or(0.0, |look| {
                    let along_m = match held.as_ref().map(|b| b.position_now) {
                        Some(super::Placed::Known { error, .. }) => error.toward_au(-look.from_m) * crate::navigation::AU,
                        _ => 0.0,
                    };
                    along_m.max(missed_m) / look.from_m.length()
                });
                Frame {
                    about,
                    looks,
                    longest_s: bound(held.as_ref().and_then(|b| b.period_s), HILL_PERIOD),
                    widest_m: bound(held.as_ref().and_then(|b| b.semi_major_au), HILL_REACH * crate::navigation::AU),
                    depth,
                }
            })
            .collect();
        let (span_s, ranged) =
            self.file(subject).map_or((0.0, 0), |file| (span_s(file.sightings()), ranged(file.sightings())));
        let held = self.tried.get(&subject);
        let (last, filed_taken_s) =
            (held.and_then(|t| t.last.clone()), held.map_or(f64::NEG_INFINITY, |t| t.filed_taken_s));
        self.tried.insert(subject, Attempt { at_s: now_s, span_s, ranged, filed_taken_s, last: last.clone() });
        // A surprise is spent on this attempt. The contradicted orbit is still carried: it is the
        // best start there is, and a refit that cannot agree with the new look falls back to the
        // search. Dropping it had Mercury's search, from nothing, settle on Earth as its primary.
        self.surprised.remove(&subject);
        let star_sigma_m = match self.belief(Subject::Star(star)).map(|b| b.distance) {
            Some(super::Distance::Measured { sigma_ly, .. }) => sigma_ly * crate::system::M_PER_LY,
            _ => 0.0,
        };
        Some(FitJob { subject, owner: self.owner, frames, warm: last, star_sigma_m, taken_s: now_s })
    }

    /// File what a [`FitJob`] found, as learned at `now_s`. `false` if the body has since been
    /// forgotten, since filing would bring back a file the store let go of, or if a fit on later
    /// looks has already been filed.
    ///
    /// Stated when filed, not when its looks were taken: a report carries what was learned after
    /// the reader's mark, and a fit solved off the tick lands after every client's mark has passed
    /// the moment its job began.
    pub fn file_fit(&mut self, solved: Solved, now_s: f64) -> bool {
        if self.file(solved.subject).is_none() {
            return false;
        }
        if let Some(attempt) = self.tried.get_mut(&solved.subject) {
            if solved.taken_s < attempt.filed_taken_s {
                return false;
            }
            attempt.filed_taken_s = solved.taken_s;
            attempt.last = Some(Held { about: solved.about, fitted: solved.fitted, offered: solved.offered });
        }
        // Strictly after whatever it replaces, which `Knowledge::orbits` requires: two fits of
        // one body can land on one tick.
        let held_s = self
            .file(solved.subject)
            .into_iter()
            .flat_map(|file| file.orbits())
            .filter(|o| o.witness == self.owner && o.method == solved.orbit.method)
            .map(|o| o.stated_s)
            .fold(f64::NEG_INFINITY, f64::max);
        let stated_s = now_s.max(solved.taken_s).max(held_s + FILED_AFTER_S);
        self.orbits(solved.subject, super::Orbit { stated_s, ..solved.orbit });
        true
    }
}

/// A [`Knowledge::sightlines`] correction at `t`: linear between the looks either side, the
/// nearest one's beyond them.
fn along(seen: &[(f64, DVec3)], t: f64) -> DVec3 {
    let after = seen.partition_point(|(at, _)| *at < t);
    match (after.checked_sub(1).and_then(|i| seen.get(i)), seen.get(after)) {
        (Some((t0, c0)), Some((t1, c1))) if t1 > t0 => c0.lerp(*c1, (t - t0) / (t1 - t0)),
        (Some((_, c)), _) | (None, Some((_, c))) => *c,
        (None, None) => DVec3::ZERO,
    }
}

/// One candidate primary: the looks in its frame, and the longest period and widest axis a
/// satellite of it can have. See [`HILL_PERIOD`].
#[derive(Clone, Debug)]
struct Frame {
    about: Option<BodyId>,
    looks: Vec<Look>,
    longest_s: f64,
    widest_m: f64,
    /// Fractional error on the scale of anything fitted in this frame: the primary's depth error
    /// over its distance.
    depth: f64,
}

/// One body's fit, with nothing borrowed: the looks in each candidate primary's frame.
///
/// Carries the time the looks were taken, so that a fit overtaken by one on later looks is
/// refused by [`Knowledge::file_fit`] whichever order the two finish in.
#[derive(Clone, Debug)]
pub struct FitJob {
    pub subject: Subject,
    owner: super::Witness,
    frames: Vec<Frame>,
    /// The last orbit fitted, to carry onto this arc before searching for a new one.
    warm: Option<Held>,
    /// One sigma of the star's position along the line of sight to it, meters: see
    /// `settle::origin_covariance`.
    star_sigma_m: f64,
    taken_s: f64,
}

/// An orbit found by a [`FitJob`], for [`Knowledge::file_fit`].
#[derive(Clone, Debug)]
pub struct Solved {
    pub subject: Subject,
    about: Option<BodyId>,
    fitted: Fitted,
    orbit: super::Orbit,
    taken_s: f64,
    offered: Vec<Option<BodyId>>,
}

impl Solved {
    /// When the looks it was fitted to were taken.
    pub fn taken_s(&self) -> f64 {
        self.taken_s
    }
}

impl FitJob {
    /// The fit itself: pure, and the whole of the cost.
    ///
    /// A carried orbit competes with candidates offered since it was found, or a moon fitted
    /// before its planet was placed would stay about the star. Only those are searched: the full
    /// search fails on an arc many orbits long, which a moon's is within a day.
    pub fn solve(self) -> Option<Solved> {
        let offered: Vec<Option<BodyId>> = self.frames.iter().map(|f| f.about).collect();
        let bound = |frame: &Frame, fitted: Fitted| {
            (fitted.period_s <= frame.longest_s && fitted.semi_major_m <= frame.widest_m).then_some(fitted)
        };
        let carried = self.warm.as_ref().and_then(|held| {
            let frame = self.frames.iter().find(|f| f.about == held.about)?;
            let fitted = bound(frame, arc::refit(&held.fitted, &frame.looks)?)?;
            Some((frame.about, fitted, frame.looks.clone(), frame.depth))
        });
        let searched = |frame: Frame| {
            Some((frame.about, bound(&frame, arc::fit(&frame.looks)?)?, frame.looks, frame.depth))
        };
        let best = |found: &mut dyn Iterator<Item = (Option<BodyId>, Fitted, Vec<Look>, f64)>| {
            found.min_by(|a, b| a.1.residual_rad.total_cmp(&b.1.residual_rad))
        };
        let (about, fitted, looks, depth) = match (carried, &self.warm) {
            (Some(carried), Some(held)) => {
                let new = self.frames.into_iter().filter(|f| !held.offered.contains(&f.about));
                best(&mut std::iter::once(carried).chain(new.filter_map(searched)))?
            }
            _ => best(&mut self.frames.into_iter().filter_map(searched))?,
        };
        let mut orbit = fitted.stated(self.owner, about, &looks, self.taken_s);
        let (au, sigma_au) = orbit.semi_major_au;
        orbit.semi_major_au = (au, sigma_au.hypot(au * depth));
        // A depth error scales the whole orbit, which the axis is the log of.
        orbit.covariance = orbit.covariance.map(|c| c.scaled_by(depth));
        // An orbit about the star is only as good as the star's place; a moon's frame carries
        // its planet's, star and all, in `depth`.
        let toward_star = -looks.iter().map(|l| l.from_m).sum::<DVec3>().normalize_or_zero();
        if about.is_none() && arc::sound(self.star_sigma_m) {
            if let Some(extra) = super::settle::origin_covariance(&fitted, &looks, toward_star * self.star_sigma_m) {
                orbit.covariance = orbit.covariance.map(|c| c.with(&extra));
                let (au, sigma) = orbit.semi_major_au;
                orbit.semi_major_au = (au, sigma.hypot(au * extra[0][0].sqrt()));
                let (period, sigma) = orbit.period_s;
                orbit.period_s = (period, sigma.hypot(period * extra[6][6].sqrt()));
            }
        }
        Some(Solved { subject: self.subject, about, fitted, orbit, taken_s: self.taken_s, offered })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{Bearing, Knowledge, Lineage, Sighting, Witness};
    use em_spectra::Band;

    fn look(at_s: f64, range_m: Option<(f64, f64)>) -> Sighting {
        Sighting {
            witness: Witness(1),
            observed_s: at_s,
            bearing: Bearing { observer_ly: DVec3::ZERO, toward: DVec3::new(1.0, at_s * 1.0e-9, 0.0), sigma_rad: 1.0e-7 },
            size: None,
            range_m,
            spin_s: None,
            band: Band::V,
            flux: 1.0e-12,
            flux_sigma: 1.0e-15,
            lineage: Lineage::new(),
        }
    }

    /// A result as a shard's fitting thread hands one back, for looks taken at `taken_s`.
    fn solved(subject: Subject, taken_s: f64, semi_major_au: f64) -> Solved {
        let fitted = Fitted {
            semi_major_m: semi_major_au * crate::navigation::AU,
            eccentricity: 0.0,
            period_s: 3.0e7,
            pole: DVec3::Z,
            periapsis_rad: 0.0,
            epoch_s: 0.0,
            mu: 1.3e20,
            reach_m: f64::INFINITY,
            assumed_circular: true,
            residual_rad: 1.0e-7,
            looks: LOOKS_NEEDED,
        };
        let orbit = fitted.stated(Witness(1), None, &[], taken_s);
        Solved { subject, about: None, fitted, orbit, taken_s, offered: vec![None] }
    }

    /// A fit filed after the reader's mark reaches the reader, however long before it the looks
    /// were taken.
    #[test]
    fn a_fit_filed_late_is_still_news() {
        let star = StarId::synthesize("primary", 2);
        let subject = Subject::Body { star, body: BodyId::of(star, "late") };
        let mut k = Knowledge::new(Witness(1));
        for i in 0..LOOKS_NEEDED {
            k.sighted(subject, look(10.0 * i as f64, None));
        }
        let job = k.fit_job(subject, DVec3::ZERO, 100.0).expect("a job");
        // The client is told everything up to now while the fit is still running.
        let (_, mark) = k.report_upto(crate::knowledge::Mark::default(), 101.0, usize::MAX);
        let mark = mark.expect("sightings to report");

        assert!(k.file_fit(solved(job.subject, 100.0, 1.0), 105.0));
        let (report, _) = k.report_upto(mark, 105.0, usize::MAX);
        let sent = report.entries.iter().flat_map(|e| &e.parts).flat_map(|p| &p.orbits).count();
        assert_eq!(sent, 1, "the orbit never left the craft");
    }

    /// Two fits of one body, finishing out of order: the one on later looks stands.
    #[test]
    fn a_fit_on_older_looks_never_overwrites_a_newer_one() {
        let star = StarId::synthesize("primary", 3);
        let subject = Subject::Body { star, body: BodyId::of(star, "twice") };
        let mut k = Knowledge::new(Witness(1));
        for i in 0..LOOKS_NEEDED {
            k.sighted(subject, look(10.0 * i as f64, None));
        }
        k.fit_job(subject, DVec3::ZERO, 100.0);
        assert!(k.file_fit(solved(subject, 200.0, 2.0), 210.0));
        assert!(!k.file_fit(solved(subject, 100.0, 1.0), 220.0), "an older fit was filed over a newer one");
        // And two filed at one instant, in order, leave the later.
        assert!(k.file_fit(solved(subject, 300.0, 3.0), 400.0));
        assert!(k.file_fit(solved(subject, 310.0, 4.0), 400.0));
        let held = k.file(subject).unwrap().orbits().iter().find(|o| o.witness == Witness(1)).unwrap().semi_major_au.0;
        assert_eq!(held, 4.0);
    }

    /// A body is never offered a primary fainter than itself, however near it sits in the sky.
    #[test]
    fn only_something_brighter_is_gone_round() {
        let star = StarId::synthesize("primary", 4);
        let (planet, moonlet) = (BodyId::of(star, "planet"), BodyId::of(star, "moonlet"));
        let mut k = Knowledge::new(Witness(1));
        for (body, flux) in [(planet, 1.0e-9), (moonlet, 1.0e-12)] {
            let subject = Subject::Body { star, body };
            for i in 0..LOOKS_NEEDED {
                k.sighted(subject, Sighting { flux, ..look(10.0 * i as f64, None) });
            }
            k.fit_job(subject, DVec3::ZERO, 100.0);
            assert!(k.file_fit(solved(subject, 100.0, 5.0), 100.0));
        }
        let offered = |of: BodyId| k.primaries(star, Subject::Body { star, body: of }, 100.0);
        assert_eq!(offered(planet), vec![None], "a planet offered its own moon");
        assert_eq!(offered(moonlet), vec![None, Some(planet)]);
    }

    /// Bearings of an Earth-like orbit from a ship on a 5 AU circle, 24 looks over a fifth of a
    /// year, nudged by `noise` radians, as `arc`'s tests make them. Its period, seconds.
    fn circling(noise: f64) -> (Vec<Look>, f64) {
        use std::f64::consts::TAU;
        let (au, mu) = (crate::navigation::AU, 1.327_124_4e20);
        let period = TAU * (au.powi(3) / mu).sqrt();
        let truth = Fitted {
            semi_major_m: au,
            eccentricity: 0.0167,
            period_s: period,
            pole: DVec3::new(0.02, -0.03, 1.0).normalize(),
            periapsis_rad: 1.8,
            epoch_s: 4.0e6,
            mu,
            reach_m: f64::INFINITY,
            assumed_circular: false,
            residual_rad: 0.0,
            looks: 0,
        };
        let ship_period = TAU * ((5.0 * au).powi(3) / mu).sqrt();
        let looks = (0..24)
            .map(|i| {
                let t = i as f64 * period / 120.0;
                let phase = TAU * t / ship_period;
                let from = DVec3::new(phase.cos(), phase.sin(), 0.0) * 5.0 * au;
                let toward = (truth.at(t) - from).normalize();
                let (x, y) = toward.any_orthonormal_pair();
                let gauss = |k: u64| crate::rng::gaussian(crate::rng::hash(&[i as u64, k])) * noise;
                Look { from_m: from, toward: (toward + x * gauss(1) + y * gauss(2)).normalize(), at_s: t, sigma_rad: noise, range_m: None }
            })
            .collect();
        (looks, period)
    }

    fn job(frames: Vec<Frame>, warm: Option<Held>) -> FitJob {
        let star = StarId::synthesize("primary", 5);
        FitJob { subject: Subject::Body { star, body: BodyId::of(star, "fitted") }, owner: Witness(1), frames, warm, star_sigma_m: 0.0, taken_s: 0.0 }
    }

    fn frame(about: Option<BodyId>, looks: Vec<Look>, longest_s: f64, widest_m: f64) -> Frame {
        Frame { about, looks, longest_s, widest_m, depth: 0.0 }
    }

    /// An orbit outside its primary's Hill sphere, by period or by axis, is refused.
    #[test]
    fn a_satellite_stays_inside_the_hill_sphere() {
        let noise = 2.979e-7 * crate::knowledge::astrometry::CENTROID_FLOOR;
        let (looks, period) = circling(noise);
        let about = Some(BodyId::of(StarId::synthesize("primary", 5), "planet"));
        let au = crate::navigation::AU;
        let solve = |longest_s: f64, widest_m: f64| job(vec![frame(about, looks.clone(), longest_s, widest_m)], None).solve();
        assert!(solve(f64::INFINITY, f64::INFINITY).is_some(), "premise: the looks fit");
        assert!(solve(period * 0.9, f64::INFINITY).is_none(), "a period past the bound was kept");
        assert!(solve(f64::INFINITY, au * 0.9).is_none(), "an axis past the bound was kept");
    }

    /// A carried orbit loses to a primary offered since it was found, if that one fits better:
    /// a moon fitted about the star before its planet was placed moves to the planet.
    #[test]
    fn a_carried_orbit_yields_to_a_primary_offered_since() {
        let noise = 2.979e-7 * crate::knowledge::astrometry::CENTROID_FLOOR;
        let (clean, _) = circling(noise);
        // Far noisier than the clean fit's own stall, which is hundreds of times its noise.
        let (noisy, _) = circling(noise * 3000.0);
        let held = arc::fit(&noisy).expect("premise: the noisy looks fit");
        let planet = Some(BodyId::of(StarId::synthesize("primary", 5), "planet"));
        let warm = Held { about: None, fitted: held, offered: vec![None] };
        let frames = vec![
            frame(None, noisy, f64::INFINITY, f64::INFINITY),
            frame(planet, clean, f64::INFINITY, f64::INFINITY),
        ];
        let solved = job(frames, Some(warm)).solve().expect("fits");
        assert_eq!(solved.about, planet, "the carried orbit kept its primary");
        assert_eq!(solved.offered, vec![None, planet]);
    }

    /// A body tried on bearings alone waits for its arc to grow half again, but a close pass
    /// that ranges it puts it back at the front at once: three ranges are a solution.
    #[test]
    fn new_ranges_rearm_a_body_whose_arc_has_not_grown() {
        let star = StarId::synthesize("primary", 1);
        let (quiet, ranged) = (BodyId::of(star, "quiet"), BodyId::of(star, "ranged"));
        let mut k = Knowledge::new(Witness(1));
        for body in [quiet, ranged] {
            for i in 0..LOOKS_NEEDED {
                k.sighted(Subject::Body { star, body }, look(1000.0 * i as f64, None));
            }
        }
        while let Some(subject) = k.unfitted(star) {
            k.fit_job(subject, DVec3::ZERO, 10_000.0);
        }

        for i in 0..RANGED_NEEDED {
            k.sighted(Subject::Body { star, body: ranged }, look(4100.0 + i as f64, Some((1.0e11, 1.0e6))));
        }
        assert_eq!(k.unfitted(star), Some(Subject::Body { star, body: ranged }));
        k.fit_job(Subject::Body { star, body: ranged }, DVec3::ZERO, 10_001.0);
        assert_eq!(k.unfitted(star), None, "tried with those ranges, so not again until more come");
    }
}
