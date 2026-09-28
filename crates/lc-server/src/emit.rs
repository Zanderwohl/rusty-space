//! Light a craft puts out, and where it lands. See `lightcone/docs/31-directed-energy.md` and
//! `lightcone/docs/30-the-field.md` §Proximity.
//!
//! **One path, whatever lit it.** An emission is an event where it lights, another wherever its
//! power changes, and one where it goes out, each fanned out along its cone by the event machinery
//! everything else uses. Every delivery to another craft is a [`Landing`] at its arrival. A
//! sustained emission's landing restates what that beam puts on the receiver, as intake, tells the
//! receiver's owner, and gives observers its `Glare`; a burst's is all heat at once. A collapse's
//! spike is the isotropic burst, riding the collapse event. What a landing needs is in its event's
//! payload as [`Emitted`], so the journal holds everything a restart needs.
//!
//! Three things come back after a restart: what each craft has lit and what lands on each craft
//! now, with the craft's checkpoint, and landings still in flight, from the journal's deliveries.
//!
//! A receiver takes its share of a beam as its light lands, from where it is then, and keeps it
//! until the light of the beam going out lands: a target that maneuvered after the beam left is
//! missed, and one that flies into a beam after its first light passed is not fed.

use std::collections::HashMap;

use glam::DVec3;
use lc_proto::{Aim, Apertures, Glare, Order, Outbound, Refusal, ShipId, Spectrum};
use lc_spacetime::LIGHT_MICROSECOND_M;
use lc_world::craft::CraftId;
use lc_world::emit::Boost;
use lc_world::field::Burst;
use lc_world::fitting::Lit;
use lc_world::flight::{C_M_S, G0};
use lc_world::signal::{Beam, Transmitter};
use serde::{Deserialize, Serialize};

use crate::field::shadow_toward_m2;
use crate::journal::{Journal, JournalError};
use crate::server::{KIND_COLLAPSE, Server};
use crate::transport::Transport;
use crate::world::{Event, Scheduled, schedule};

pub const KIND_EMIT: i16 = lc_proto::kind::EMIT;

/// The shortest wavelength an emit may ask for, and the longest: 31's 1 nm and the dish's 3 cm.
const WAVELENGTH_M: (f64, f64) = (1.0e-9, 0.03);

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
}

/// A craft's light, as its checkpoint carries it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub emitting: Vec<Lighting>,
    pub lit_by: Vec<Incoming>,
}

/// An emitting event's light on its way to one craft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Landing {
    pub(crate) observer: CraftId,
    /// Coordinate microseconds, from the fan-out's delivery.
    pub(crate) arrive_t: i64,
    source: ShipId,
    emitted: Emitted,
}

/// What is emitted and landing, kept on the server.
#[derive(Default)]
pub(crate) struct Emissions {
    pub(crate) emitting: HashMap<CraftId, Vec<Lighting>>,
    pub(crate) lit_by: HashMap<CraftId, Vec<Incoming>>,
    pub(crate) landings: Vec<Landing>,
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

    pub(crate) fn light_of(&self, id: CraftId) -> Light {
        Light {
            emitting: self.emitting.get(&id).cloned().unwrap_or_default(),
            lit_by: self.lit_by.get(&id).cloned().unwrap_or_default(),
        }
    }

    pub(crate) fn restore(&mut self, id: CraftId, light: Light) {
        if !light.emitting.is_empty() {
            self.emitting.insert(id, light.emitting);
        }
        if !light.lit_by.is_empty() {
            self.lit_by.insert(id, light.lit_by);
        }
    }

    pub(crate) fn is_emitting(&self, id: CraftId) -> bool {
        self.emitting.get(&id).is_some_and(|list| !list.is_empty())
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
        let Order::Emit { aim, apertures, power_w, wavelength_m, spread_rad, duration_s } = *order else {
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
        let axis = self.beam_for(id, &aim, at)?.axis;
        let spread_rad = Transmitter::new(wavelength_m, ends[0].diameter_m).spread_rad(spread_rad);
        let spectrum = Spectrum::Line { wavelength_m };
        let lighting = |axis: DVec3, lights_t: i64, out_t: i64| Lighting {
            lights_t,
            out_t,
            axis: axis.to_array(),
            half_angle_rad: spread_rad,
            spectrum,
            power_w,
            beam: None,
        };

        let lit = match apertures {
            Apertures::Both => {
                let lit = Lit { from_s: at_s, until_s: at_s + duration_s, power_w: 2.0 * power_w };
                if lit.power_w * duration_s > craft.free_j_at(at_s) {
                    return Err(Refusal::NoEnergy);
                }
                let craft = self.fleet.get_mut(id).ok_or(Refusal::NotYours)?;
                craft.adjust(at_s, |fitting| fitting.light(lit));
                let out_t = to_us(lit.until_s);
                vec![lighting(axis, at, out_t), lighting(-axis, at, out_t)]
            }
            Apertures::Fore | Apertures::Aft => {
                let (from_ly, beta0) = lc_world::motion::state_at(&craft.motion, craft.system.as_deref(), at_s)
                    .unwrap_or((craft.motion.position_ly, craft.motion.beta));
                let accel_g = power_w / (craft.mass_kg_at(at_s) * C_M_S * G0);
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
                self.pursuits.remove(&id);
                vec![lighting(axis, to_us(boost.lights_s()).max(at), to_us(boost.out_s()))]
            }
        };
        self.emissions.emitting.insert(id, lit);
        Ok(Order::Emit { aim, apertures, power_w, wavelength_m, spread_rad, duration_s })
    }

    /// Put out everything `id` has lit, at `at`: its draw stops, and the light of stopping follows
    /// what is already on its way.
    pub(crate) fn put_out(&mut self, id: CraftId, at: i64, events: &mut Vec<Event>, deliveries: &mut Vec<Scheduled>) {
        let Some(lit) = self.emissions.emitting.get_mut(&id) else { return };
        for lighting in lit.iter_mut() {
            lighting.out_t = lighting.out_t.min(at);
            lighting.lights_t = lighting.lights_t.min(lighting.out_t);
        }
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
            .map(|d| Landing { observer: CraftId(d.observer.0), arrive_t: d.arrive_t, source: event.source, emitted });
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
        let Landing { observer, arrive_t, source, emitted } = landing;
        let at_s = arrive_t as f64 * 1.0e-6;
        let Some(craft) = self.fleet.get(observer) else { return false };
        let from = DVec3::from_array(emitted.from);
        let offset = craft.position_at(arrive_t as f64) - from;
        let distance_m = offset.length() * LIGHT_MICROSECOND_M;
        let shadow_m2 = shadow_toward_m2(craft, -offset, at_s);
        let fraction = emitted.fraction(shadow_m2, distance_m);
        if emitted.burst_j > 0.0 {
            let arriving_j = emitted.burst_j * fraction;
            let Some(craft) = self.fleet.get_mut(observer) else { return false };
            craft.adjust(at_s, |fitting| fitting.take_burst(Burst::Arriving(arriving_j)));
            return self.after_landing(observer, arrive_t, wire, events, deliveries);
        }

        let covered = emitted.cone().covers(offset);
        let was = self.emissions.lit_by.get(&observer).and_then(|beams| beams.iter().find(|b| b.beam == emitted.beam));
        let was_w = was.map_or(0.0, |b| b.arriving_w);
        if was.is_none() && !(covered && emitted.power_w > 0.0) {
            return false;
        }
        let lit_by = self.emissions.lit_by.entry(observer).or_default();
        lit_by.retain(|b| b.beam != emitted.beam);
        let arriving_w = if covered { emitted.power_w * fraction } else { 0.0 };
        if covered && emitted.power_w > 0.0 {
            lit_by.push(Incoming {
                beam: emitted.beam,
                source,
                from: emitted.from,
                spectrum: emitted.spectrum,
                flux_w_m2: lc_world::emit::flux_w_m2(emitted.power_w, emitted.half_angle_rad, distance_m),
                arriving_w,
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
