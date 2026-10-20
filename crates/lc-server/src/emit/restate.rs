//! Each receiver's share of every beam it may be in, taken again as it goes. See
//! `lightcone/docs/31-directed-energy.md` §What arrives.
//!
//! Between the instants a beam's statements land, what a receiver takes of it changes as both
//! move: it flies into the cone or out, and its distance and shadow change. The share is taken
//! again at the first microsecond it has moved a step from what was last taken, at every crossing
//! of the cone's edge, and wherever a statement's light meets it. Each is found over the tick just
//! finished and queued as a landing, which the account settles as a change of input like any
//! other.
//!
//! **A pair costs the same whatever a tick spans.** It is looked at the tick's end and where its
//! straight-line path relative to the emitter passes nearest the axis, and a change between two
//! looks is bisected to its microsecond. Looking on a schedule of the flight's own instead cost a
//! flotilla at a year a minute thousands of looks a pair a tick. So a change is missed only if it
//! comes back within the tick off that straight line, and one leap and many ticks find the same
//! instants only while the motion between the two is straight.
//!
//! The light left from where the emitter's worldline was at the retarded instant, while the
//! emitter can still say, and otherwise from where the statement said. Only receivers within a
//! beam's reach are followed; beyond it one is restated only as each statement's light lands.

use glam::DVec3;
use lc_proto::ShipId;
use lc_spacetime::{LIGHT_MICROSECOND_M, Worldline};
use lc_world::craft::Craft;
use lc_world::fitting::Balance;

use super::drives::first;
use super::{Emitted, Landing, Said};
use crate::field::{radius_us, shadow_toward_m2};
use crate::journal::Journal;
use crate::server::Server;

/// Of the cooking flux, the flux at a beam's reach: a thousand times its cooking distance.
const REACH_FRACTION: f64 = 1.0e-6;
/// A share is taken again once it has moved this fraction from what was last taken.
const SHARE_STEP: f64 = 0.01;
/// Changes one pair may land in a tick. Past it, the share is taken at the tick's end.
pub(crate) const MOST_CHANGES: usize = 8;

/// How far a statement's light is followed, light-microseconds.
pub(super) fn reach_us(emitted: &Emitted, balance: &Balance) -> f64 {
    let flux_w_m2 = lc_world::courtesy::cooking_flux_w_m2(balance) * REACH_FRACTION;
    lc_world::emit::distance_at_flux_m(emitted.power_w, emitted.half_angle_rad, flux_w_m2) / LIGHT_MICROSECOND_M
}

/// A beam's light at a receiver at one instant.
#[derive(Clone, Copy)]
struct Seen {
    said_t: i64,
    /// From where this light left.
    emitted: Emitted,
    lit: bool,
    share_w: f64,
}

/// What `receiver` takes at `t` of the beam `said` states, if any of its light is there.
fn seen(emitter: Option<&Craft>, said: &[Said], receiver: &Craft, t: i64) -> Option<Seen> {
    let here = receiver.position_at(t as f64);
    let retarded = emitter.and_then(|emitter| {
        let line = emitter.worldline();
        let left = lc_spacetime::retarded_times_at(t as f64, here, &line).last().copied()?;
        let s = said.iter().rev().find(|s| s.t as f64 <= left)?;
        Some((s, line.position_at(left)))
    });
    let (s, from) = retarded.or_else(|| {
        let s = said.iter().rev().find(|s| s.t as f64 + here.distance(DVec3::from_array(s.emitted.from)) <= t as f64)?;
        Some((s, DVec3::from_array(s.emitted.from)))
    })?;
    let offset = here - from;
    let emitted = Emitted { from: from.to_array(), ..s.emitted };
    let shadow_m2 = shadow_toward_m2(receiver, -offset, t as f64 * 1.0e-6);
    let (cone, radius) = (emitted.cone(), radius_us(shadow_m2));
    let lit = emitted.power_w > 0.0 && cone.reaches(offset, radius);
    let distance_m = offset.length() * LIGHT_MICROSECOND_M;
    let share_w = if lit { emitted.power_w * emitted.fraction(shadow_m2, distance_m) } else { 0.0 };
    Some(Seen { said_t: s.t, emitted, lit, share_w })
}

/// What was last taken of a beam.
#[derive(Clone, Copy, Default)]
struct Taken {
    said_t: i64,
    lit: bool,
    share_w: f64,
}

fn taken(seen: Option<&Seen>) -> Taken {
    seen.filter(|s| s.lit).map_or(Taken::default(), |s| Taken { said_t: s.said_t, lit: true, share_w: s.share_w })
}

fn changed(was: Taken, now: Option<&Seen>) -> bool {
    let now = taken(now);
    now.lit != was.lit
        || now.lit && (now.said_t != was.said_t || (now.share_w - was.share_w).abs() > SHARE_STEP * was.share_w)
}

/// Where in `(after_t, now)` the receiver passes nearest the axis of `said`'s latest statement,
/// along the straight line between where it is relative to the emitter at either end.
fn nearest_axis_t(emitter: Option<&Craft>, said: &[Said], receiver: &Craft, after_t: i64, now: i64) -> Option<i64> {
    let (emitter, last) = (emitter?, said.last()?);
    let axis = DVec3::from_array(last.emitted.axis).try_normalize()?;
    let apart = |t: i64| receiver.position_at(t as f64) - emitter.position_at(t as f64);
    let (from, by) = (apart(after_t), apart(now) - apart(after_t));
    let (across, drift) = (from.cross(axis), by.cross(axis));
    let s = -across.dot(drift) / drift.length_squared();
    (s > 0.0 && s < 1.0).then(|| after_t + (s * (now - after_t) as f64) as i64)
}

/// Land what `there` is at `at`, and say what is taken now. `None` when there is nothing to land.
fn land(found: &mut Vec<(i64, i64, Emitted)>, said: &[Said], at: i64, there: Option<Seen>) -> Option<Taken> {
    let (said_t, emitted) = match there {
        Some(seen) => (seen.said_t, seen.emitted),
        // Gone dark: what is landed carries nothing.
        None => {
            let last = said.last()?;
            (last.t, Emitted { power_w: 0.0, ..last.emitted })
        }
    };
    found.push((at, said_t, emitted));
    Some(taken(there.as_ref()))
}

/// How many share steps lie between what was taken and `now`, when only the share has moved.
fn drift_steps(was: Taken, now: Option<&Seen>) -> Option<usize> {
    let now = now.filter(|n| was.lit && n.lit && n.said_t == was.said_t && was.share_w > 0.0)?;
    Some(((now.share_w / was.share_w).ln().abs() / SHARE_STEP.ln_1p()).ceil() as usize)
}

/// Every instant in `(after_t, now]` `receiver`'s share of the beam changes from `was`, and the
/// statement to land there, from where its light left.
///
/// At most [`MOST_CHANGES`]. A drift with more steps than are left lands that many, where its
/// share passes even ratios between its ends. Each is held to the next, so a falling share is
/// overstated by up to a step, and a rising one understated.
fn scan(emitter: Option<&Craft>, said: &[Said], receiver: &Craft, mut was: Taken, after_t: i64, now: i64) -> Vec<(i64, i64, Emitted)> {
    let look = |t: i64| seen(emitter, said, receiver, t);
    let share = |t: i64| look(t).filter(|s| s.lit).map_or(0.0, |s| s.share_w);
    let marks: Vec<i64> = nearest_axis_t(emitter, said, receiver, after_t, now).into_iter().chain([now]).collect();
    let mut found = Vec::new();
    let mut t = after_t;
    let mut left = MOST_CHANGES;
    while let Some(&next) = marks.iter().find(|m| **m > t) {
        let there = look(next);
        if !changed(was, there.as_ref()) {
            t = next;
            continue;
        }
        let at = match drift_steps(was, there.as_ref()) {
            Some(steps) if steps > left => {
                let (from_w, ratio) = (was.share_w, there.map_or(0.0, |s| s.share_w) / was.share_w);
                for k in 1..left {
                    let target_w = from_w * ratio.powf(k as f64 / left as f64);
                    let at = first(t, next, |m| (share(m) - target_w) * (ratio - 1.0) >= 0.0);
                    if land(&mut found, said, at, look(at)).is_none() {
                        break;
                    }
                    t = at;
                }
                next
            }
            _ if left > 1 => first(t, next, |m| changed(was, look(m).as_ref())),
            _ => next,
        };
        let there = if at == next { there } else { look(at) };
        let Some(now_taken) = land(&mut found, said, at, there) else { break };
        was = now_taken;
        t = at;
        left = MOST_CHANGES.saturating_sub(found.len()).max(1);
    }
    found
}

impl<J: Journal> Server<J> {
    /// Queue a landing at every instant in `(after_t, now]` a receiver's share of a beam changes.
    pub(crate) fn restate(&mut self, after_t: i64) {
        let now = self.now_t;
        let mut found = Vec::new();
        for (source, said) in &self.emissions.said {
            let emitter = self.fleet.get(*source);
            let mut beams: Vec<i64> = said.iter().map(|s| s.emitted.beam).collect();
            beams.sort_unstable();
            beams.dedup();
            for beam in beams {
                let said: Vec<Said> = said.iter().filter(|s| s.emitted.beam == beam).copied().collect();
                let reach = said.iter().map(|s| reach_us(&s.emitted, &self.balance)).fold(0.0, f64::max);
                let mut near: Vec<DVec3> = said.iter().map(|s| DVec3::from_array(s.emitted.from)).collect();
                near.extend(emitter.map(|e| e.position_at(now as f64)));
                for receiver in self.fleet.iter().filter(|c| c.id != *source && c.ended_s().is_none()) {
                    let here = receiver.position_at(now as f64);
                    if near.iter().all(|p| p.distance(here) > reach) {
                        continue;
                    }
                    let held = self.emissions.lit_by.get(&receiver.id).and_then(|b| b.iter().find(|b| b.beam == beam));
                    let was = held.map_or(Taken::default(), |b| Taken { said_t: b.said_t, lit: true, share_w: b.arriving_w });
                    for (arrive_t, said_t, emitted) in scan(emitter, &said, receiver, was, after_t, now) {
                        found.push(Landing { observer: receiver.id, arrive_t, source: ShipId(source.0), emitted, said_t });
                    }
                }
            }
        }
        self.emissions.landings.extend(found);
    }
}
