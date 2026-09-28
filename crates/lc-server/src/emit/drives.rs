//! Every lit drive as one of this module's emissions: lit at ignition, said again whenever its power
//! has moved a step as the ship lightens or its axis has turned, and put out at cutoff. An emit
//! flown as a burn throttles the same way and is said again alike. Found after the fact over the
//! tick just finished, as [`crate::drive`] finds the plume's transitions, at instants that depend
//! only on the flight: the plan's own transitions, and the first microsecond a step is reached.

use glam::DVec3;
use lc_proto::Spectrum;
use lc_world::craft::{Craft, CraftId};
use lc_world::emit::{Exhaust, Jet, aperture_temperature_k, exhaust, exhaust_face_m2, thrust_power_w};
use lc_world::fitting::Balance;
use lc_world::flight::G0;
use lc_world::motion::Motive;

use super::Lighting;
use crate::journal::Journal;
use crate::server::Server;
use crate::world::{Event, Scheduled};

/// A lit drive's power is said again once it has moved this fraction from what was last said.
const POWER_STEP: f64 = 0.01;
/// And its axis once it has turned this fraction of its half-angle.
const AXIS_STEP: f64 = 0.1;
/// Coordinate microseconds between the instants a lit drive is looked at for a drift, on a grid
/// so that a leap and a tick look at the same ones. A frame's turning or the mass falling moves a
/// step in hours; a change quicker than this is a transition of the plan, which is looked at too.
const LOOK_US: i64 = 60_000_000;

/// What a craft's lightings should say at one instant.
#[derive(Default)]
struct Wanted {
    jets: Vec<Exhaust>,
    /// An emit flown as a burn, at the rocket law's throttle.
    boost_w: Option<f64>,
}

fn wanted(craft: &Craft, balance: &Balance, t: i64) -> Wanted {
    let s = t as f64 * 1.0e-6;
    let boost_w = match &craft.motion_at(s).motive {
        Motive::Boosting(boost) if boost.thrust_at(s) != DVec3::ZERO => {
            Some(thrust_power_w(craft.mass_kg_at(s), boost.accel_g * G0))
        }
        _ => None,
    };
    Wanted { jets: exhaust(craft, balance, s), boost_w }
}

fn stepped(was_w: f64, now_w: f64) -> bool {
    (now_w - was_w).abs() > POWER_STEP * was_w
}

fn turned(lighting: &Lighting, jet: &Exhaust) -> bool {
    DVec3::from_array(lighting.axis).angle_between(jet.axis) > AXIS_STEP * jet.half_angle_rad
}

/// An emit flown as a burn, lit at `t`.
fn boosting(lighting: &Lighting, t: i64) -> bool {
    lighting.jet.is_none() && lighting.beam.is_some() && lighting.out_t > t
}

fn differs(stated: &[Lighting], wanted: &Wanted, t: i64) -> bool {
    let jet_differs = [Jet::Drive, Jet::Thrusters].iter().any(|kind| {
        let was = stated.iter().find(|l| l.jet == Some(*kind));
        match (was, wanted.jets.iter().find(|j| j.jet == *kind)) {
            (None, None) => false,
            (Some(l), Some(j)) => stepped(l.power_w, j.power_w) || turned(l, j),
            _ => true,
        }
    });
    let boost = stated.iter().find(|l| boosting(l, t));
    jet_differs || boost.zip(wanted.boost_w).is_some_and(|(l, w)| stepped(l.power_w, w))
}

/// The first microsecond in `(a, b]` where `p` holds, `p(b)` being true and `p(a)` false.
pub(super) fn first(mut a: i64, mut b: i64, p: impl Fn(i64) -> bool) -> i64 {
    while b - a > 1 {
        let m = a + (b - a) / 2;
        if p(m) { b = m } else { a = m }
    }
    b
}

impl<J: Journal> Server<J> {
    fn drive_differs(&self, id: CraftId, t: i64) -> bool {
        let Some(craft) = self.fleet.get(id) else { return false };
        let stated = self.emissions.emitting.get(&id).map_or(&[][..], Vec::as_slice);
        differs(stated, &wanted(craft, &self.balance, t), t)
    }

    /// State every lit drive over `(after_t, now]`. A wreck's were put out when it collapsed.
    pub(crate) fn light_drives(&mut self, after_t: i64, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) {
        let now = self.now_t;
        let (after_s, now_s) = (after_t as f64 * 1.0e-6, now as f64 * 1.0e-6);
        let ids: Vec<CraftId> = self.fleet.iter().filter(|c| c.ended_s().is_none()).map(|c| c.id).collect();
        for id in ids {
            let Some(craft) = self.fleet.get(id) else { continue };
            let mut marks: Vec<i64> = lc_world::ignition::transitions(craft, after_s, now_s)
                .iter()
                .map(|t| ((t.at_s * 1.0e6).ceil() as i64).clamp(after_t + 1, now))
                .collect();
            let stated = self.emissions.emitting.get(&id).is_some_and(|lit| !lit.is_empty());
            if marks.is_empty() && !stated && !self.drive_differs(id, now) {
                continue;
            }
            let grid = (after_t / LOOK_US + 1) * LOOK_US;
            marks.extend((grid..=now).step_by(LOOK_US as usize));
            marks.push(now);
            marks.sort_unstable();
            marks.dedup();

            let mut from = after_t;
            for mark in marks {
                while mark - 1 > from && self.drive_differs(id, mark - 1) {
                    let t = first(from, mark - 1, |t| self.drive_differs(id, t));
                    self.state_drive(id, t, events, deliveries);
                    from = t;
                    if self.drive_differs(id, t) {
                        break;
                    }
                }
                if self.drive_differs(id, mark) {
                    self.state_drive(id, mark, events, deliveries);
                }
                from = mark;
            }
        }
    }

    /// Say what `id`'s drives send at `t`, where it differs from what was last said.
    fn state_drive(&mut self, id: CraftId, t: i64, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) {
        let Some(craft) = self.fleet.get(id) else { return };
        let wanted = wanted(craft, &self.balance, t);
        let face_m2 = exhaust_face_m2(craft);
        let spectrum = |power_w: f64| Spectrum::Blackbody { temperature_k: aperture_temperature_k(power_w, face_m2) };
        let mut lit = self.emissions.emitting.remove(&id).unwrap_or_default();
        for kind in [Jet::Drive, Jet::Thrusters] {
            let at = lit.iter().position(|l| l.jet == Some(kind));
            match (at, wanted.jets.iter().find(|j| j.jet == kind)) {
                (Some(k), None) => {
                    let lighting = lit.remove(k);
                    self.light_event(id, &lighting, t, 0.0, events, deliveries);
                }
                (Some(k), Some(jet)) if stepped(lit[k].power_w, jet.power_w) || turned(&lit[k], jet) => {
                    let lighting = &mut lit[k];
                    lighting.axis = jet.axis.to_array();
                    lighting.power_w = jet.power_w;
                    lighting.spectrum = spectrum(jet.power_w);
                    let lighting = *lighting;
                    self.light_event(id, &lighting, t, jet.power_w, events, deliveries);
                }
                (None, Some(jet)) => {
                    let mut lighting = Lighting {
                        lights_t: t,
                        out_t: i64::MAX,
                        axis: jet.axis.to_array(),
                        half_angle_rad: jet.half_angle_rad,
                        spectrum: spectrum(jet.power_w),
                        power_w: jet.power_w,
                        beam: None,
                        jet: Some(kind),
                    };
                    lighting.beam = self.light_event(id, &lighting, t, jet.power_w, events, deliveries);
                    if lighting.beam.is_some() {
                        lit.push(lighting);
                    }
                }
                _ => {}
            }
        }
        let boost = lit.iter_mut().find(|l| boosting(l, t));
        if let (Some(lighting), Some(power_w)) = (boost, wanted.boost_w) {
            if stepped(lighting.power_w, power_w) {
                lighting.power_w = power_w;
                let lighting = *lighting;
                self.light_event(id, &lighting, t, power_w, events, deliveries);
            }
        }
        if !lit.is_empty() {
            self.emissions.emitting.insert(id, lit);
        }
    }
}
