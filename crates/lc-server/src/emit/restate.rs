//! Each receiver's share of every beam it may be in, taken again as it goes. See
//! `lightcone/docs/31-directed-energy.md` §What arrives.
//!
//! Between the instants a beam's statements land, what a receiver takes of it changes as both
//! move: it flies into the cone or out, and its distance and shadow change. The share is taken
//! again at the first microsecond it has moved a step from what was last taken, at every crossing
//! of the cone's edge, and wherever a statement's light meets it. Each is found over the tick just
//! finished and queued as a landing, which the account settles as a change of input like any
//! other. The instants are the flight's and not the tick's, so one leap and many ticks find the
//! same ones.
//!
//! The light left from where the emitter's worldline was at the retarded instant, while the
//! emitter can still say, and otherwise from where the statement said. Only receivers within a
//! beam's reach are followed; beyond it one is restated only as each statement's light lands.

use std::sync::atomic::{AtomicBool, Ordering};

use glam::DVec3;
use lc_proto::ShipId;
use lc_spacetime::{LIGHT_MICROSECOND_M, Worldline};
use lc_world::craft::Craft;
use lc_world::fitting::Balance;

use super::drives::first;
use super::{Emitted, Landing, Said};
use crate::field::shadow_toward_m2;
use crate::journal::Journal;
use crate::server::Server;

/// Of the cooking flux, the flux at a beam's reach: a thousand times its cooking distance.
const REACH_FRACTION: f64 = 1.0e-6;
/// A share is taken again once it has moved this fraction from what was last taken.
const SHARE_STEP: f64 = 0.01;
/// Of the time a step or the cone's edge could be reached in at the speed between the two, how
/// far ahead the next look is: the speed is taken at the look, and either may be accelerating.
const LOOK_AHEAD: f64 = 0.5;
const SHORTEST_LOOK_US: i64 = 1_000;
const LONGEST_LOOK_US: i64 = 10_000_000;
/// A backstop on the looks one pair takes in a tick: past it, the rest of the tick is taken from
/// where the next tick starts, and a leap and many ticks may differ.
const MOST_LOOKS: usize = 100_000;

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
    /// Light-microseconds.
    distance: f64,
    /// Angle to the cone's edge, from whichever side.
    to_edge_rad: f64,
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
    let lit = emitted.power_w > 0.0 && emitted.cone().covers(offset);
    let distance_m = offset.length() * LIGHT_MICROSECOND_M;
    let share_w = if lit {
        let shadow_m2 = shadow_toward_m2(receiver, -offset, t as f64 * 1.0e-6);
        emitted.power_w * emitted.fraction(shadow_m2, distance_m)
    } else {
        0.0
    };
    let to_edge_rad = (emitted.half_angle_rad - DVec3::from_array(emitted.axis).angle_between(offset)).abs();
    Some(Seen { said_t: s.t, emitted, lit, share_w, distance: offset.length(), to_edge_rad })
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

/// How far ahead to look from `seen` at `t` so a step or an edge between the looks is not missed.
fn look_us(emitter: Option<&Craft>, receiver: &Craft, seen: Option<&Seen>, t: i64) -> i64 {
    let Some(seen) = seen else { return LONGEST_LOOK_US };
    let theirs = emitter.map_or(DVec3::ZERO, |e| e.worldline().velocity_at(t as f64 - seen.distance));
    let beta = (receiver.worldline().velocity_at(t as f64) - theirs).length();
    if beta <= 0.0 {
        return LONGEST_LOOK_US;
    }
    // Light-microseconds over a speed in c is microseconds. A share goes as the inverse square.
    let to_step = if seen.lit { 0.5 * SHARE_STEP * seen.distance } else { f64::INFINITY };
    let to_edge = seen.to_edge_rad * seen.distance;
    let ahead = LOOK_AHEAD * to_step.min(to_edge) / beta;
    (ahead as i64).clamp(SHORTEST_LOOK_US, LONGEST_LOOK_US)
}

/// Every instant in `(after_t, now]` `receiver`'s share of the beam changes from `was`, and the
/// statement to land there, from where its light left.
fn scan(emitter: Option<&Craft>, said: &[Said], receiver: &Craft, mut was: Taken, after_t: i64, now: i64) -> Vec<(i64, i64, Emitted)> {
    let look = |t: i64| seen(emitter, said, receiver, t);
    let mut found = Vec::new();
    let mut t = after_t;
    for _ in 0..MOST_LOOKS {
        if t >= now {
            return found;
        }
        let next = (t + look_us(emitter, receiver, look(t).as_ref(), t)).min(now);
        if !changed(was, look(next).as_ref()) {
            t = next;
            continue;
        }
        let at = first(t, next, |m| changed(was, look(m).as_ref()));
        let there = look(at);
        let (said_t, emitted) = match there {
            Some(seen) => (seen.said_t, seen.emitted),
            // Gone dark: what is landed carries nothing.
            None => match said.last() {
                Some(last) => (last.t, Emitted { power_w: 0.0, ..last.emitted }),
                None => break,
            },
        };
        found.push((at, said_t, emitted));
        was = taken(there.as_ref());
        t = at;
    }
    if t < now && !CUT_SHORT.swap(true, Ordering::Relaxed) {
        eprintln!("WARNING: a receiver's share was followed only to {t} of a tick ending {now}; the rest is taken next tick");
    }
    found
}

static CUT_SHORT: AtomicBool = AtomicBool::new(false);

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
