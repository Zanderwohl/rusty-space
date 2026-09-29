//! Light a craft puts out, and where it lands. See `lightcone/docs/31-directed-energy.md` and
//! `lightcone/docs/30-the-field.md` §Proximity.
//!
//! **One path, whatever lit it.** An emission is an event where it lights and one where it goes
//! out, each fanned out along its cone by the event machinery
//! everything else uses. Every delivery to another craft is a [`Landing`] at its arrival. A
//! sustained emission's landing restates what that beam puts on the receiver, as intake, tells the
//! receiver's owner, and gives observers its `Glare`; a burst's is all heat at once. A collapse's
//! spike is the isotropic burst, riding the collapse event. What a landing needs is in its event's
//! payload as [`Emitted`], so the journal holds everything a restart needs.
//!
//! Three things come back after a restart: what each craft has lit, has said of it and what lands
//! on each craft now, with the craft's checkpoint, and landings still in flight, from the journal's
//! deliveries.
//!
//! **Restated as it goes.** An emission is said again, as an event of the same beam, whenever what
//! it sends has changed by a step: a lit drive's power as the ship lightens, or its axis as it
//! turns ([`drives`]). A receiver's share is taken again at every instant it changes by a step,
//! crosses the cone's edge or meets a statement's light ([`restate`]); each is a landing like any
//! other, so a craft that flies into a beam whose light is already passing is fed from when it
//! enters, and one that leaves stops.

use std::collections::HashMap;

use glam::DVec3;
use lc_proto::{Aim, Apertures, Glare, Lead, Order, Outbound, Refusal, ShipId, Spectrum};
use lc_spacetime::LIGHT_MICROSECOND_M;
use lc_world::craft::CraftId;
use lc_world::emit::{Boost, Jet};
use lc_world::field::Burst;
use lc_world::fitting::Lit;
use lc_world::flight::Drive;
use lc_world::motion::LIGHT_US_PER_LY;
use lc_world::signal::{Beam, Transmitter};
use serde::{Deserialize, Serialize};

use crate::field::shadow_toward_m2;
use crate::journal::{Journal, JournalError};
use crate::server::{KIND_COLLAPSE, Server};
use crate::transport::Transport;
use crate::world::{Event, Scheduled, schedule};

mod drives;
mod restate;

pub const KIND_EMIT: i16 = lc_proto::kind::EMIT;

/// Two shares of one statement this close are one, and the second is not news.
const SAME: f64 = 1.0e-4;

/// The shortest wavelength an emit may ask for, and the longest: 31's 1 nm and the dish's 3 cm.
const WAVELENGTH_M: (f64, f64) = (1.0e-9, 0.03);

/// Between the two sightings a burn is measured from. A steady burn reads the same over any.
const LEAD_BASELINE_US: i64 = 1_000_000;

/// What an emitting event carries to every craft it reaches, in its payload under `emission`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Emitted {
    /// The event that lit it, which names the beam to a receiver.
    pub beam: i64,
    /// Where it left, light-microseconds. The event's own position is rounded by the store.
    pub from: [f64; 3],
    pub axis: [f64; 3],
    pub half_angle_rad: f64,
    pub spectrum: Spectrum,
    /// Sustained watts from this event on. Zero is going out.
    pub power_w: f64,
    /// Joules let go of at once.
    pub burst_j: f64,
}

impl Emitted {
    fn cone(&self) -> Beam {
        Beam::along(DVec3::from_array(self.axis), self.half_angle_rad)
    }

    /// Of what leaves along the cone, the share a shadow of `shadow_m2` at `distance_m` intercepts.
    /// Isotropic light is capped at what a receiver in contact faces, as a neighbor's glow is.
    fn fraction(&self, shadow_m2: f64, distance_m: f64) -> f64 {
        if self.cone().is_omni() {
            lc_world::field::received_fraction(shadow_m2, distance_m)
        } else {
            lc_world::emit::received_fraction(self.half_angle_rad, shadow_m2, distance_m)
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Carrying {
    emission: Emitted,
}

/// `payload`, a JSON object, with the emission beside what it already says.
pub(crate) fn carrying(payload: serde_json::Value, emission: &Emitted) -> String {
    let mut payload = match payload {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    if let Ok(emission) = serde_json::to_value(emission) {
        payload.insert("emission".into(), emission);
    }
    serde_json::Value::Object(payload).to_string()
}

fn emitted(payload: &str) -> Option<Emitted> {
    serde_json::from_str::<Carrying>(payload).ok().map(|c| c.emission)
}

/// An emission its source has lit or will light, until the event of its going out is written.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lighting {
    /// Coordinate microseconds.
    pub lights_t: i64,
    pub out_t: i64,
    pub axis: [f64; 3],
    pub half_angle_rad: f64,
    pub spectrum: Spectrum,
    pub power_w: f64,
    /// The event that lit it, once written.
    pub beam: Option<i64>,
    /// A drive's, stated as it burns; `None` for an emit, lit and put out by its order.
    pub jet: Option<Jet>,
}

/// A beam whose light is landing on a craft now.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Incoming {
    pub beam: i64,
    pub source: ShipId,
    /// Where it left, light-microseconds.
    pub from: [f64; 3],
    pub spectrum: Spectrum,
    /// At the receiver, which is what an instrument there sees.
    pub flux_w_m2: f64,
    /// Onto its field, before absorptivity.
    pub arriving_w: f64,
    /// When the statement this share was taken from was said, coordinate microseconds.
    pub said_t: i64,
}

/// One statement of a beam, kept while its light may still reach a candidate receiver.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Said {
    /// Coordinate microseconds.
    pub t: i64,
    pub emitted: Emitted,
}

/// A craft's light, as its checkpoint carries it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub emitting: Vec<Lighting>,
    pub lit_by: Vec<Incoming>,
    pub said: Vec<Said>,
}

/// An emitting event's light on its way to one craft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Landing {
    pub(crate) observer: CraftId,
    /// Coordinate microseconds, from the fan-out's delivery.
    pub(crate) arrive_t: i64,
    source: ShipId,
    emitted: Emitted,
    /// When `emitted` was said.
    said_t: i64,
}

/// What is emitted and landing, kept on the server.
#[derive(Default)]
pub(crate) struct Emissions {
    pub(crate) emitting: HashMap<CraftId, Vec<Lighting>>,
    pub(crate) lit_by: HashMap<CraftId, Vec<Incoming>>,
    pub(crate) landings: Vec<Landing>,
    /// By source, oldest first.
    pub(crate) said: HashMap<CraftId, Vec<Said>>,
}

impl Emissions {
    /// Watts arriving at `id`'s field from every beam landing on it, before absorptivity.
    pub(crate) fn beamed_w(&self, id: CraftId) -> f64 {
        self.lit_by.get(&id).map_or(0.0, |beams| beams.iter().map(|b| b.arriving_w).sum())
    }

    /// `source` as `observer` sees it from inside its beams: their flux summed, in the spectrum of
    /// the brightest.
    pub(crate) fn glare(&self, observer: CraftId, source: ShipId) -> Option<Glare> {
        let beams = self.lit_by.get(&observer)?.iter().filter(|b| b.source == source);
        let brightest = beams.clone().max_by(|a, b| a.flux_w_m2.total_cmp(&b.flux_w_m2))?;
        Some(Glare { spectrum: brightest.spectrum, flux_w_m2: beams.map(|b| b.flux_w_m2).sum() })
    }

    #[cfg(feature = "storage")]
    pub(crate) fn light_of(&self, id: CraftId) -> Light {
        Light {
            emitting: self.emitting.get(&id).cloned().unwrap_or_default(),
            lit_by: self.lit_by.get(&id).cloned().unwrap_or_default(),
            said: self.said.get(&id).cloned().unwrap_or_default(),
        }
    }

    #[cfg(feature = "storage")]
    pub(crate) fn restore(&mut self, id: CraftId, light: Light) {
        if !light.emitting.is_empty() {
            self.emitting.insert(id, light.emitting);
        }
        if !light.lit_by.is_empty() {
            self.lit_by.insert(id, light.lit_by);
        }
        if !light.said.is_empty() {
            self.said.insert(id, light.said);
        }
    }

    /// Whether an emit is lit, which excludes lighting anything else. A drive's own light is not.
    pub(crate) fn is_emitting(&self, id: CraftId) -> bool {
        self.emitting.get(&id).is_some_and(|list| list.iter().any(|l| l.jet.is_none()))
    }

    /// Whether a later statement of `beam` than the one said at `said_t` has reached `here` by `at`.
    fn superseded(&self, source: CraftId, beam: i64, said_t: i64, here: DVec3, at: i64) -> bool {
        self.said.get(&source).is_some_and(|said| {
            said.iter()
                .filter(|s| s.emitted.beam == beam && s.t > said_t)
                .any(|s| s.t as f64 + here.distance(DVec3::from_array(s.emitted.from)) <= at as f64)
        })
    }

    /// Whether `observer` holds `beam`, or has its light on the way.
    fn holds(&self, observer: CraftId, beam: i64) -> bool {
        self.lit_by.get(&observer).is_some_and(|beams| beams.iter().any(|b| b.beam == beam))
            || self.landings.iter().any(|l| l.observer == observer && l.emitted.beam == beam && l.emitted.power_w > 0.0)
    }
}

fn to_us(s: f64) -> i64 {
    (s * 1.0e6).ceil() as i64
}

impl<J: Journal> Server<J> {
    /// Validate an [`Order::Emit`] and light it, as a burn when it has net thrust. What was lit:
    /// the spread never under the diffraction floor.
    pub(crate) fn order_emit(&mut self, id: CraftId, order: &Order, at: i64) -> Result<Order, Refusal> {
        let Order::Emit { aim, apertures, power_w, wavelength_m, spread_rad, duration_s, lead } = *order else {
            return Err(Refusal::Impossible);
        };
        let finite = [power_w, wavelength_m, spread_rad, duration_s].iter().all(|x| x.is_finite());
        if !finite || power_w <= 0.0 || duration_s <= 0.0 || spread_rad < 0.0 {
            return Err(Refusal::Impossible);
        }
        if !(WAVELENGTH_M.0..=WAVELENGTH_M.1).contains(&wavelength_m) {
            return Err(Refusal::Impossible);
        }
        let at_s = at as f64 * 1.0e-6;
        let craft = self.fleet.get(id).ok_or(Refusal::NotYours)?;
        if craft.is_refitting(at_s) {
            return Err(Refusal::Refitting);
        }
        if self.emissions.is_emitting(id) || craft.motion.is_under_way() {
            return Err(Refusal::UnderWay);
        }
        let fitting = craft.fitting().ok_or(Refusal::NoAperture)?;
        let (fore, aft) = lc_world::form::capacity::ends(fitting.form(), fitting.balance()).ok_or(Refusal::NoAperture)?;
        let ends = match apertures {
            Apertures::Fore => vec![fore],
            Apertures::Aft => vec![aft],
            Apertures::Both => vec![fore, aft],
        };
        if ends.iter().any(|end| end.rating_w <= 0.0) {
            return Err(Refusal::NoAperture);
        }
        if ends.iter().any(|end| power_w > end.rating_w) {
            return Err(Refusal::OverRating);
        }
        if matches!(aim, Aim::Omni) {
            return Err(Refusal::Impossible);
        }
        let axis = match (aim, lead) {
            (Aim::Ship(target), Lead::Burning) => self.lead_along_burn(id, target, at)?,
            _ => self.beam_for(id, &aim, at)?.axis,
        };
        // One spread for both ends of a balanced emit, never under either end's floor.
        let spread_rad = ends.iter().map(|end| Transmitter::new(wavelength_m, end.diameter_m).spread_rad(spread_rad)).fold(0.0, f64::max);
        let spectrum = Spectrum::Line { wavelength_m };
        let lighting = |axis: DVec3, lights_t: i64, out_t: i64| Lighting {
            lights_t,
            out_t,
            axis: axis.to_array(),
            half_angle_rad: spread_rad,
            spectrum,
            power_w,
            beam: None,
            jet: None,
        };

        let lit = match apertures {
            Apertures::Both => {
                let lit = Lit { from_s: at_s, until_s: at_s + duration_s, power_w: 2.0 * power_w };
                if lit.power_w * duration_s > craft.free_j_at(at_s) {
                    return Err(Refusal::NoEnergy);
                }
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                craft.adjust(at_s, |fitting| fitting.light(lit));
                self.pursuits.remove(&id);
                let out_t = to_us(lit.until_s);
                vec![lighting(axis, at, out_t), lighting(-axis, at, out_t)]
            }
            Apertures::Fore | Apertures::Aft => {
                let (from_ly, beta0) = lc_world::motion::state_at(&craft.motion, craft.system.as_deref(), at_s)
                    .unwrap_or((craft.motion.position_ly, craft.motion.beta));
                let accel_g = Drive::accel_g_at(craft.mass_kg_at(at_s), power_w);
                let nose = if apertures == Apertures::Fore { axis } else { -axis };
                let attitude0 = craft.facing_at(at_s).unwrap_or(nose);
                let boost = Boost::plan(from_ly, beta0, at_s, -axis, nose, accel_g, duration_s, attitude0, craft.slew_rate_rad_s());
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                let before = craft.clone();
                craft.boost(boost, at_s);
                if !crate::fitting::can_pay_for_its_plan(craft, at_s) {
                    *craft = before;
                    return Err(Refusal::NoEnergy);
                }
                // Unlike an emit from both ends, a boost moves the ship off its parking orbit.
                self.pursuits.remove(&id);
                self.parks.remove(&id);
                vec![lighting(axis, to_us(boost.lights_s()).max(at), to_us(boost.out_s()))]
            }
        };
        // Beside a drive whose cut this tick is not stated yet.
        self.emissions.emitting.entry(id).or_default().extend(lit);
        Ok(Order::Emit { aim, apertures, power_w, wavelength_m, spread_rad, duration_s, lead })
    }

    /// Where to point at `target` to meet it holding the burn `id` saw it in: its proper acceleration
    /// measured from two of `id`'s sightings [`LEAD_BASELINE_US`] apart, as an escort measures its
    /// quarry's, and coasting when its plume was dark or it has not been watched that long.
    fn lead_along_burn(&self, id: CraftId, target: ShipId, at: i64) -> Result<DVec3, Refusal> {
        let seen = crate::chase::sighting(&self.fleet, id, target, at).ok_or(Refusal::NotInSight)?;
        let previous = crate::chase::sighting(&self.fleet, id, target, at - LEAD_BASELINE_US);
        let accel = crate::chase::burn_of(&self.fleet, &seen, previous.as_ref()).unwrap_or(DVec3::ZERO);
        let from_ly = self.fleet.get(id).ok_or(Refusal::NotYours)?.position_at(at as f64) / LIGHT_US_PER_LY;
        lc_world::emit::lead(from_ly, at as f64 * 1.0e-6, &seen, accel).map(|led| led.axis).ok_or(Refusal::Impossible)
    }

    /// Put out what `id` has lit, at `at`, its drives too when `drives`: its draw stops, and the
    /// light of stopping follows what is already on its way. A drive cut by an order is not put out
    /// here but by the change of motion, which [`drives`] states.
    pub(crate) fn put_out(&mut self, id: CraftId, at: i64, drives: bool, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) {
        let Some(lit) = self.emissions.emitting.get_mut(&id) else { return };
        let mut withdrawn = Vec::new();
        lit.retain_mut(|lighting| {
            if !(drives || lighting.jet.is_none()) {
                return true;
            }
            // A burn still coming about never lit, and nor did a drive stated as lighting after it.
            let lit_by_then = match lighting.beam {
                Some(_) => lighting.lights_t <= at,
                None => lighting.lights_t < at,
            };
            lighting.out_t = lighting.out_t.min(at);
            withdrawn.extend(lighting.beam);
            lit_by_then
        });
        // Anything said after the instant it went out was never sent, though this tick said it.
        let unsaid = |source: ShipId, beam: i64, t: i64| source.0 == id.0 && withdrawn.contains(&beam) && t > at;
        if let Some(said) = self.emissions.said.get_mut(&id) {
            said.retain(|s| !unsaid(ShipId(id.0), s.emitted.beam, s.t));
        }
        self.emissions.landings.retain(|l| !unsaid(l.source, l.emitted.beam, l.said_t));
        let mut dropped = Vec::new();
        events.retain(|e| {
            let drop = e.kind == KIND_EMIT && emitted(&e.payload).is_some_and(|m| unsaid(e.source, m.beam, e.t));
            if drop {
                dropped.push(e.id);
            }
            !drop
        });
        deliveries.retain(|d| !dropped.contains(&d.event));
        if let Some(craft) = self.fleet.get_mut(id) {
            craft.adjust(at as f64 * 1.0e-6, |fitting| fitting.darken());
        }
        self.keep_emissions(events, deliveries);
    }

    /// Write every lighting and going out due by now.
    pub(crate) fn keep_emissions(&mut self, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) {
        let now = self.now_t;
        let ids: Vec<CraftId> = self.emissions.emitting.keys().copied().collect();
        for id in ids {
            let Some(mut lit) = self.emissions.emitting.remove(&id) else { continue };
            for lighting in &mut lit {
                if lighting.beam.is_none() && lighting.lights_t <= now {
                    lighting.beam = self.light_event(id, lighting, lighting.lights_t, lighting.power_w, events, deliveries);
                }
            }
            lit.retain(|lighting| {
                let done = lighting.beam.is_some() && lighting.out_t <= now;
                if done {
                    self.light_event(id, lighting, lighting.out_t, 0.0, events, deliveries);
                }
                !done && lighting.beam.is_some() || lighting.lights_t > now
            });
            if !lit.is_empty() {
                self.emissions.emitting.insert(id, lit);
            }
        }
        self.forget_said();
    }

    /// Drop each statement whose light has gone past every candidate: one followed by another said
    /// more than its reach ago, and a going out with nothing left before it.
    fn forget_said(&mut self) {
        let (now, balance) = (self.now_t, self.balance);
        for said in self.emissions.said.values_mut() {
            let before = said.clone();
            said.retain(|s| {
                let mut later = before.iter().filter(|n| n.emitted.beam == s.emitted.beam && n.t > s.t);
                match later.next() {
                    Some(next) => next.t as f64 + restate::reach_us(&s.emitted, &balance) >= now as f64,
                    None => s.emitted.power_w > 0.0 || before.iter().any(|b| b.emitted.beam == s.emitted.beam && b.t < s.t),
                }
            });
        }
        self.emissions.said.retain(|_, said| !said.is_empty());
    }

    /// An event of `lighting` at `at` putting out `power_w` from there on: the lighting itself when
    /// it has no beam yet. Its id.
    fn light_event(
        &mut self,
        id: CraftId,
        lighting: &Lighting,
        at: i64,
        power_w: f64,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Option<i64> {
        let from = self.fleet.get(id)?.position_at(at as f64);
        let emission = |beam: i64| Emitted {
            beam: lighting.beam.unwrap_or(beam),
            from: from.to_array(),
            axis: lighting.axis,
            half_angle_rad: lighting.half_angle_rad,
            spectrum: lighting.spectrum,
            power_w,
            burst_j: 0.0,
        };
        let cone = emission(0).cone();
        let beam = lighting.beam;
        let holding: Vec<CraftId> = match beam {
            Some(beam) => self.fleet.iter().map(|c| c.id).filter(|o| self.emissions.holds(*o, beam)).collect(),
            None => Vec::new(),
        };
        let payload = |event_id: i64| carrying(serde_json::json!({}), &emission(event_id));
        let event_id = self.fan_out(id, from, KIND_EMIT, power_w, payload, at, &cone, &holding, events, deliveries)?;
        self.queue_landings(events.last()?, deliveries);
        let said = self.emissions.said.entry(id).or_default();
        said.push(Said { t: at, emitted: emission(event_id) });
        said.sort_by_key(|s| s.t);
        Some(event_id)
    }

    /// Write an event and schedule it to every worldline its light reaches inside `cone`, and to
    /// `also` wherever they are. Its id, unless none could be minted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fan_out(
        &mut self,
        id: CraftId,
        from: DVec3,
        kind: i16,
        power_w: f64,
        payload: impl FnOnce(i64) -> String,
        at: i64,
        cone: &Beam,
        also: &[CraftId],
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> Option<i64> {
        let event_id = self.minter.mint(at)?.get();
        let event = Event { id: event_id, source: ShipId(id.0), t: at, at: from, kind, power_w, payload: payload(event_id) };
        for observer in self.fleet.iter() {
            let reach = if also.contains(&observer.id) { &Beam::OMNI } else { cone };
            if let Some(scheduled) = schedule(&event, reach, observer) {
                deliveries.push(scheduled);
            }
        }
        events.push(event);
        Some(event_id)
    }

    /// Queue a landing for every delivery of `event` to a craft other than its source, if it
    /// carries an emission.
    pub(crate) fn queue_landings(&mut self, event: &Event, deliveries: &[Scheduled]) {
        let Some(emitted) = emitted(&event.payload) else { return };
        let landings = deliveries
            .iter()
            .filter(|d| d.event == event.id && d.observer != event.source)
            .map(|d| Landing { observer: CraftId(d.observer.0), arrive_t: d.arrive_t, source: event.source, emitted, said_t: event.t });
        self.emissions.landings.extend(landings);
    }

    /// Land one emitting event's light on its craft, collapsing it there if a burst takes it to
    /// `Q_max`. Whether it did.
    pub(crate) fn land(
        &mut self,
        landing: Landing,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> bool {
        let Landing { observer, arrive_t, source, emitted, said_t } = landing;
        let at_s = arrive_t as f64 * 1.0e-6;
        let Some(craft) = self.fleet.get(observer).filter(|craft| craft.ended_s().is_none()) else { return false };
        let from = DVec3::from_array(emitted.from);
        let here = craft.position_at(arrive_t as f64);
        if emitted.burst_j <= 0.0 && self.emissions.superseded(CraftId(source.0), emitted.beam, said_t, here, arrive_t) {
            return false;
        }
        let offset = here - from;
        let distance_m = offset.length() * LIGHT_MICROSECOND_M;
        let shadow_m2 = shadow_toward_m2(craft, -offset, at_s);
        let fraction = emitted.fraction(shadow_m2, distance_m);
        if emitted.burst_j > 0.0 {
            let arriving_j = emitted.burst_j * fraction;
            let Some(craft) = self.fleet.get_mut(observer) else { return false };
            craft.adjust(at_s, |fitting| fitting.take_burst(Burst::Arriving(arriving_j)));
            return self.after_landing(observer, arrive_t, wire, events, deliveries);
        }

        let lit = emitted.cone().covers(offset) && emitted.power_w > 0.0;
        let was = self.emissions.lit_by.get(&observer).and_then(|beams| beams.iter().find(|b| b.beam == emitted.beam)).copied();
        let was_w = was.map_or(0.0, |b| b.arriving_w);
        let arriving_w = if lit { emitted.power_w * fraction } else { 0.0 };
        let unchanged = |b: Incoming| lit && b.said_t == said_t && (arriving_w - b.arriving_w).abs() <= SAME * b.arriving_w;
        let skip = match was {
            None => !lit,
            Some(b) => unchanged(b),
        };
        if skip {
            return false;
        }
        let lit_by = self.emissions.lit_by.entry(observer).or_default();
        lit_by.retain(|b| b.beam != emitted.beam);
        if lit {
            lit_by.push(Incoming {
                beam: emitted.beam,
                source,
                from: emitted.from,
                spectrum: emitted.spectrum,
                flux_w_m2: lc_world::emit::flux_w_m2(emitted.power_w, emitted.half_angle_rad, distance_m),
                arriving_w,
                said_t,
            });
        }
        if lit_by.is_empty() {
            self.emissions.lit_by.remove(&observer);
        }
        if let Some(owner) = self.owners.get(&observer).copied() {
            wire.send(owner, Outbound::Illuminated {
                ship_id: ShipId(observer.0),
                beam: emitted.beam,
                bearing: (-offset).normalize_or_zero().to_array(),
                spectrum: emitted.spectrum,
                power_w: arriving_w,
                arrive_t,
            });
        }
        let Some(craft) = self.fleet.get_mut(observer) else { return false };
        craft.adjust(at_s, |fitting| fitting.set_lit_w(fitting.lit_w() + arriving_w - was_w));
        self.after_landing(observer, arrive_t, wire, events, deliveries)
    }

    /// Collapse a craft a landing took to `Q_max`, or tell its owner what its field now holds.
    fn after_landing(
        &mut self,
        id: CraftId,
        at_t: i64,
        wire: &mut impl Transport,
        events: &mut Vec<Event>,
        deliveries: &mut Vec<Scheduled>,
    ) -> bool {
        let at_s = at_t as f64 * 1.0e-6;
        let Some(craft) = self.fleet.get(id) else { return false };
        let Some(fitting) = craft.fitting() else { return false };
        if fitting.heat_j_at(&craft.motion, at_s) < fitting.field().heat_max_j() {
            self.tell_fitted(wire, id);
            return false;
        }
        self.collapse(id, at_t, wire, events, deliveries);
        true
    }

    /// Landings whose light had not arrived when the last shard stopped, whatever lit them, from
    /// the journal's deliveries of events this shard's clock has reached.
    pub async fn resume_landings(&mut self) -> Result<(), JournalError> {
        for kind in [KIND_EMIT, KIND_COLLAPSE] {
            let flying = self.journal.in_flight(kind, self.now_t).await?;
            let (mut events, deliveries): (Vec<Event>, Vec<Scheduled>) = flying.into_iter().map(|(d, e)| (e, d)).unzip();
            events.sort_by_key(|e| e.id);
            events.dedup_by_key(|e| e.id);
            let now = self.now_t;
            for event in events.iter().filter(|e| e.t <= now) {
                self.queue_landings(event, &deliveries);
            }
        }
        Ok(())
    }
}

/// An isotropic burst of `burst_j` from `from`, light-microseconds, a blackbody at `temperature_k`.
pub(crate) fn burst(from: DVec3, burst_j: f64, temperature_k: f64) -> Emitted {
    Emitted {
        beam: 0,
        from: from.to_array(),
        axis: DVec3::X.to_array(),
        half_angle_rad: std::f64::consts::PI,
        spectrum: Spectrum::Blackbody { temperature_k },
        power_w: 0.0,
        burst_j,
    }
}

#[cfg(test)]
mod tests;
