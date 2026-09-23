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

use super::arc::{self, Look, LOOKS_NEEDED};
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

    /// The body of `star` most in need of an orbit: one whose bearings have grown since its
    /// orbit was last stated, longest since its last attempt first.
    ///
    /// Round-robin by age rather than by any measure of promise. A fit costs about a tick, so
    /// what matters is that every body gets its turn and none is starved.
    ///
    /// **By attempt, not by statement.** An arc that cannot yet shape an orbit states nothing,
    /// so ranking by what was stated leaves that body at the front of the queue on every tick
    /// and nothing else in the system is ever fitted. Saturn on a three-month arc is enough to
    /// starve Venus, Earth, Mars and Jupiter indefinitely.
    pub fn unfitted(&self, star: StarId) -> Option<Subject> {
        let mine = |subject: Subject| -> Option<f64> {
            let file = self.file(subject)?;
            if file.sightings().len() < LOOKS_NEEDED {
                return None;
            }
            let newest = file.sightings().iter().map(|s| s.observed_s).fold(f64::MIN, f64::max);
            let stated = file
                .orbits()
                .iter()
                .filter(|o| o.witness == self.owner && o.method == crate::knowledge::Method::Astrometric)
                .map(|o| o.stated_s)
                .fold(f64::MIN, f64::max);
            let tried = self.tried.get(&subject).copied().unwrap_or(f64::MIN);
            (newest > stated).then_some(stated.max(tried))
        };
        self.members(star)
            .filter_map(|(subject, _)| match subject {
                Subject::Body { .. } => Some((mine(subject)?, subject)),
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, subject)| subject)
    }

    /// The mean direction a body was seen in, which points at whatever it goes round.
    fn mean_bearing(&self, subject: Subject) -> Option<DVec3> {
        let file = self.file(subject)?;
        let mean: DVec3 = file.sightings().iter().map(|s| s.bearing.toward).sum();
        (mean.length_squared() > 0.0).then(|| mean.normalize())
    }

    /// Candidate primaries for a body, the star first and then the nearest few in the sky.
    ///
    /// **Not a list of moons.** A Keplerian orbit puts its primary at a focus, so the candidate
    /// that works as a focus is the primary, and trying the star alongside the rest is what
    /// keeps a planet from being handed to one of its neighbors.
    fn primaries(&self, star: StarId, subject: Subject, now_s: f64) -> Vec<Option<BodyId>> {
        let Some(toward) = self.mean_bearing(subject) else { return vec![None] };
        let mut near: Vec<(f64, BodyId)> = self
            .members(star)
            .filter_map(|(other, _)| match other {
                Subject::Body { body, .. } if other != subject => {
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
    pub fn fit_orbit(&mut self, subject: Subject, star_ly: DVec3, now_s: f64) -> bool {
        let Some(star) = subject.star() else { return false };
        let best = self
            .primaries(star, subject, now_s)
            .into_iter()
            .filter_map(|about| {
                let at = |t: f64| match about {
                    None => Some(star_ly),
                    Some(body) => Some(
                        star_ly + self.placed(star, body, t)? / crate::system::M_PER_LY,
                    ),
                };
                let looks = self.looks_at(subject, &at);
                let fitted = arc::fit(&looks)?;
                Some((about, fitted, looks))
            })
            .min_by(|a, b| a.1.residual_rad.total_cmp(&b.1.residual_rad));
        // Recorded whatever comes of it, so the queue moves on. See [`Knowledge::unfitted`].
        self.tried.insert(subject, now_s);
        let Some((about, fitted, looks)) = best else { return false };
        let orbit = fitted.stated(self.owner, about, &looks, now_s);
        self.orbits(subject, orbit);
        true
    }
}

