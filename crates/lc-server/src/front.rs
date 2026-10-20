//! Light on its way, followed until it is too faint to matter.
//!
//! Every event's light is a sphere growing at `c`, followed until it is fainter than
//! [`crate::world::FAINTEST`] everywhere it could land. Each tick finds the craft the sphere passed
//! during the tick, whichever craft exist then and on the courses they actually flew.
//!
//! Light from a subluminal source reaches any point in the order it left, since `t + |x - w(t)|`
//! rises with `t`. So a source's fronts in emission order are in arrival order at every craft, and
//! one binary search per source finds a tick's landings. A jump breaks the order, so a front that
//! would break it starts a new run. See `lightcone/docs/02-event-store.md`.

use std::collections::HashMap;

use lc_world::craft::{Craft, CraftId, Fleet};
use lc_spacetime::Worldline;
use lc_world::signal::Beam;

use crate::journal::{Journal, JournalError};
use crate::radio::Utterance;
use crate::server::Server;
use crate::world::{Event, Scheduled, reach_us, schedule};

/// Sweeps between clearing faded fronts out of the buckets. Until then they are skipped.
const COMPACT_EVERY: u32 = 1_200;

/// One event's light.
#[derive(Clone, Debug)]
pub struct Front {
    pub event: Event,
    pub beam: Beam,
    /// Craft that see it whatever the beam: the ones an emission is already lighting.
    pub also: Vec<CraftId>,
    /// A transmitter does not hear its own transmission.
    pub deaf: Option<CraftId>,
    pub(crate) said: Option<Utterance>,
    /// Coordinate microseconds after which it is fainter than [`crate::world::FAINTEST`] everywhere.
    pub until_t: i64,
    /// Through when its landings have been found, if later than the last sweep, so a front crossed
    /// at launch is not found again.
    found_t: i64,
}

impl Front {
    pub(crate) fn new(event: Event, beam: Beam, also: Vec<CraftId>, deaf: Option<CraftId>, said: Option<Utterance>) -> Self {
        // The beam's own gain: `also` hears it omnidirectionally, which never carries further.
        let reach = reach_us(event.power_w, &beam);
        let until_t = event.t.saturating_add(reach.ceil() as i64);
        Self { event, beam, also, deaf, said, until_t, found_t: i64::MIN }
    }

    fn landing(&self, craft: &Craft) -> Option<Scheduled> {
        if self.deaf == Some(craft.id) {
            return None;
        }
        let beam = if self.also.contains(&craft.id) { &Beam::OMNI } else { &self.beam };
        schedule(&self.event, beam, craft)
    }
}

/// Every front still bright enough to matter.
#[derive(Default)]
pub struct Fronts {
    live: Vec<Front>,
    /// Indices into `live`, each run one source's fronts in order of emission and no two of them
    /// further apart than light goes between them, so in order of arrival everywhere.
    runs: Vec<Vec<usize>>,
    by_source: HashMap<i64, Vec<usize>>,
    /// Made since the last sweep. See [`Fronts::sweep_fresh`].
    fresh: Vec<Front>,
    unkept: Vec<Front>,
    crossed: Vec<Scheduled>,
    sweeps: u32,
}

/// A front's light landing on a craft: the front, by its place in [`Fronts::get`], and the landing.
pub type Crossing = (usize, Scheduled);

impl Fronts {
    pub fn launch(&mut self, front: Front) {
        self.unkept.push(front.clone());
        self.fresh.push(front);
    }

    pub fn restore(&mut self, fronts: Vec<Front>) {
        for front in fronts {
            self.insert(front);
        }
    }

    pub fn get(&self, index: usize) -> &Front {
        &self.live[index]
    }

    pub fn len(&self) -> usize {
        self.live.len() + self.fresh.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Take back fronts made this tick whose events were never sent, and their landings.
    pub fn withdraw(&mut self, event_ids: &[i64]) {
        self.fresh.retain(|f| !event_ids.contains(&f.event.id));
        self.unkept.retain(|f| !event_ids.contains(&f.event.id));
        self.crossed.retain(|d| !event_ids.contains(&d.event));
        for front in self.live.iter_mut().filter(|f| event_ids.contains(&f.event.id)) {
            front.until_t = i64::MIN;
        }
    }

    pub fn take_crossed(&mut self) -> Vec<Scheduled> {
        std::mem::take(&mut self.crossed)
    }

    pub fn take_unkept(&mut self) -> Vec<Front> {
        std::mem::take(&mut self.unkept)
    }

    /// Every landing in `(after_t, now_t]` on any craft in `fleet`, in arrival order.
    pub fn sweep(&mut self, fleet: &Fleet, after_t: i64, now_t: i64) -> Vec<Crossing> {
        let mut found = Vec::new();
        // A craft moves at most the window's width in the window, so the light that lands on it in
        // the window is what would land where it is now within that much either side.
        let width = (now_t - after_t).max(0) as f64;
        for craft in fleet.iter() {
            let here = craft.position_at(now_t as f64);
            let lands = |front: &Front| front.event.t as f64 + (here - front.event.at).length();
            // A jump in the window puts the craft further from where it is now than the width, so
            // for that tick it is asked of every front. Jumps are rare and a tick is short.
            let jumped = craft.worldline().breaks().iter().any(|&t| t > after_t as f64 && t <= now_t as f64);
            for run in &self.runs {
                let from = match jumped {
                    true => 0,
                    false => run.partition_point(|&i| lands(&self.live[i]) <= after_t as f64 - width),
                };
                for &index in &run[from..] {
                    let front = &self.live[index];
                    if !jumped && lands(front) > now_t as f64 + width {
                        break;
                    }
                    if front.until_t < after_t {
                        continue;
                    }
                    let opens = after_t.max(front.found_t);
                    if let Some(landing) = front.landing(craft).filter(|l| l.arrive_t > opens && l.arrive_t <= now_t) {
                        found.push((index, landing));
                    }
                }
            }
        }
        found.extend(self.sweep_fresh(fleet, now_t));
        self.sweeps += 1;
        if self.sweeps.is_multiple_of(COMPACT_EVERY) {
            found = self.compact(now_t, found);
        }
        found.sort_by_key(|(_, landing)| (landing.arrive_t, landing.observer.0, landing.event));
        found
    }

    /// Every landing by `now_t` of fronts made since the last sweep. Their window opens at their own
    /// emission, which may be before the last sweep: an order is stamped as far back as its
    /// client's cursor.
    pub fn sweep_fresh(&mut self, fleet: &Fleet, now_t: i64) -> Vec<Crossing> {
        let mut found = Vec::new();
        for mut front in std::mem::take(&mut self.fresh) {
            let opens = front.event.t.saturating_sub(1);
            front.found_t = now_t;
            let index = self.insert(front);
            for craft in fleet.iter() {
                if let Some(landing) = self.live[index].landing(craft).filter(|l| l.arrive_t > opens && l.arrive_t <= now_t) {
                    found.push((index, landing));
                }
            }
        }
        found.sort_by_key(|(_, landing)| (landing.arrive_t, landing.observer.0, landing.event));
        found
    }

    fn insert(&mut self, front: Front) -> usize {
        let index = self.live.len();
        self.live.push(front);
        let live = &self.live;
        let front = &live[index];
        // No faster than light between neighbors, which is what keeps a run in arrival order.
        let keeps_order = |a: usize, b: usize| {
            let (a, b) = (&live[a].event, &live[b].event);
            (b.at - a.at).length() <= (b.t - a.t) as f64
        };
        let runs = self.by_source.entry(front.event.source.0).or_default();
        let fits = runs.iter().copied().find_map(|run| {
            let members = &self.runs[run];
            let place = members.partition_point(|&i| live[i].event.t <= front.event.t);
            let after_previous = place == 0 || keeps_order(members[place - 1], index);
            let before_next = place == members.len() || keeps_order(index, members[place]);
            (after_previous && before_next).then_some((run, place))
        });
        match fits {
            Some((run, place)) => self.runs[run].insert(place, index),
            None => {
                runs.push(self.runs.len());
                self.runs.push(vec![index]);
            }
        }
        index
    }

    /// Drop what has faded, keeping `found`'s indices pointing at the same fronts.
    fn compact(&mut self, now_t: i64, found: Vec<Crossing>) -> Vec<Crossing> {
        let mut moved = vec![usize::MAX; self.live.len()];
        let live = std::mem::take(&mut self.live);
        self.runs.clear();
        self.by_source.clear();
        for (old, front) in live.into_iter().enumerate() {
            let needed = found.iter().any(|(i, _)| *i == old);
            if front.until_t >= now_t || needed {
                moved[old] = self.insert(front);
            }
        }
        found.into_iter().map(|(i, landing)| (moved[i], landing)).collect()
    }
}

/// What a kept front holds besides its event.
#[cfg(feature = "storage")]
#[derive(serde::Serialize, serde::Deserialize)]
struct Spread {
    /// Exactly, light-microseconds: the event's own row holds it rounded to the grid.
    at: [f64; 3],
    power_w: f64,
    axis: [f64; 3],
    half_angle_rad: f64,
    also: Vec<i64>,
    deaf: Option<i64>,
}

#[cfg(feature = "storage")]
impl Front {
    /// `None`, with a warning, when it cannot be kept.
    pub(crate) fn to_row(&self) -> Option<lc_store::fronts::Front> {
        let spread = Spread {
            at: self.event.at.to_array(),
            power_w: self.event.power_w,
            axis: self.beam.axis.to_array(),
            half_angle_rad: self.beam.half_angle_rad,
            also: self.also.iter().map(|c| c.0).collect(),
            deaf: self.deaf.map(|c| c.0),
        };
        let kept = || -> Option<lc_store::fronts::Front> { Some(lc_store::fronts::Front {
            event: lc_store::store::Event {
                id: lc_store::id::EventId::from_raw(self.event.id)?,
                source_id: self.event.source.0,
                t: self.event.t,
                g: self.event.at.round().as_i64vec3().to_array(),
                system_id: None,
                local: None,
                kind: self.event.kind,
                payload: self.event.payload.clone(),
            },
            until_t: self.until_t,
            spread: serde_json::to_string(&spread).ok()?,
        }) };
        let row = kept();
        if row.is_none() {
            eprintln!("WARNING: the light of event {} cannot be kept; a restart will not follow it", self.event.id);
        }
        row
    }

    pub(crate) fn from_row(row: &lc_store::fronts::Front, said: &HashMap<i64, Utterance>) -> Option<Self> {
        let spread: Spread = serde_json::from_str(&row.spread).ok()?;
        Some(Self {
            event: Event {
                id: row.event.id.get(),
                source: lc_proto::ShipId(row.event.source_id),
                t: row.event.t,
                at: glam::DVec3::from_array(spread.at),
                kind: row.event.kind,
                power_w: spread.power_w,
                payload: row.event.payload.clone(),
            },
            beam: Beam::along(glam::DVec3::from_array(spread.axis), spread.half_angle_rad),
            also: spread.also.into_iter().map(CraftId).collect(),
            deaf: spread.deaf.map(CraftId),
            said: said.get(&row.event.id.get()).cloned(),
            until_t: row.until_t,
            found_t: i64::MIN,
        })
    }
}

impl<J: Journal> Server<J> {
    /// Every landing in `(after_t, now]` of light put out before this tick.
    pub(crate) fn cross_fronts(&mut self, after_t: i64) {
        let found = self.fronts.sweep(&self.fleet, after_t, self.now_t);
        self.landed(found);
    }

    /// Send light on its way and land at once what lands by now: a collapse's spike must land in
    /// time order among the collapses still to come in its tick.
    pub(crate) fn launch(&mut self, front: Front) {
        self.fronts.launch(front);
        let found = self.fronts.sweep_fresh(&self.fleet, self.now_t);
        self.landed(found);
    }

    /// Everything a landing does, on any craft, connected or not.
    fn landed(&mut self, found: Vec<Crossing>) {
        for (index, landing) in found {
            let front = self.fronts.get(index).clone();
            let observer = CraftId(landing.observer.0);
            if let Some(said) = &front.said {
                let sender = CraftId(front.event.source.0);
                self.landed_on(front.event.id, sender, said, observer, landing.arrive_t, landing.strength);
                self.owe(sender, front.event.at, !front.beam.is_omni(), said, observer, landing.arrive_t);
            }
            self.queue_landings(&front.event, std::slice::from_ref(&landing));
            self.schedule_landings(std::slice::from_ref(&front.event), std::slice::from_ref(&landing));
            self.fronts.crossed.push(landing);
        }
    }

    /// A tick the journal refuses keeps none of its light, because its events are lost.
    pub(crate) async fn journal_tick(&mut self, events: &[Event]) -> Result<(), JournalError> {
        let deliveries = self.fronts.take_crossed();
        let made = self.fronts.take_unkept();
        if let Err(why) = self.journal.write(events, &deliveries, &made).await {
            self.fronts.withdraw(&events.iter().map(|e| e.id).collect::<Vec<_>>());
            return Err(why);
        }
        Ok(())
    }

    /// Read once, when a shard comes back.
    pub async fn resume_fronts(&mut self) -> Result<(), JournalError> {
        let fronts = self.journal.fronts(self.now_t).await?;
        self.fronts.restore(fronts);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_proto::{ClientId, Inbound, Intent, Order, Outbound, ShipId, Sighting};
    use lc_world::craft::CraftId;
    use lc_world::motion::LIGHT_US_PER_LY;

    use crate::journal::Memory;
    use crate::server::{KIND_TRANSMIT, Server};
    use crate::transport::Loopback;
    use crate::world::{FAINTEST, still};

    const LIGHT_HOUR_US: f64 = 3.6e9;

    fn transmit(power_w: f64) -> Inbound {
        Inbound::Act(Intent { ship_id: ShipId(1), order: Order::Transmit { power_w }, issued_at_client_t: i64::MAX })
    }

    /// Ship 1 at the origin transmits at `power_w`: when, and from where.
    async fn shout(server: &mut Server<Memory>, wire: &mut Loopback, power_w: f64) -> (i64, DVec3) {
        server.admit(ClientId(1), still(ShipId(1), DVec3::ZERO), 0.0);
        wire.client_says(ClientId(1), transmit(power_w));
        server.tick(wire).await.unwrap();
        let sent = server.journal().events.iter().find(|e| e.kind == KIND_TRANSMIT).expect("it went out");
        (sent.t, sent.at)
    }

    async fn run_until(server: &mut Server<Memory>, wire: &mut Loopback, t: f64) {
        while (server.now_t() as f64) < t {
            server.tick(wire).await.unwrap();
        }
    }

    fn heard(wire: &mut Loopback, client: ClientId) -> Vec<Sighting> {
        wire.take(client)
            .into_iter()
            .flat_map(|m| match m {
                Outbound::Sightings(list) => list.into_iter().map(|s| s.get().clone()).collect(),
                _ => Vec::new(),
            })
            .filter(|s| s.kind == KIND_TRANSMIT && s.source_id == 1)
            .collect()
    }

    /// A craft that did not exist when the light left hears it when the light gets there, and one
    /// that appears where it has already passed does not.
    #[tokio::test]
    async fn a_craft_made_after_the_light_left_hears_it_when_it_arrives() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let (t0, from) = shout(&mut server, &mut wire, 1.0e12).await;
        let (ahead, behind) = (DVec3::X * LIGHT_HOUR_US, DVec3::Y * 0.25 * LIGHT_HOUR_US);

        run_until(&mut server, &mut wire, t0 as f64 + 0.5 * LIGHT_HOUR_US).await;
        server.admit(ClientId(2), still(ShipId(2), ahead), 0.0);
        server.admit(ClientId(3), still(ShipId(3), behind), 0.0);
        run_until(&mut server, &mut wire, t0 as f64 + 1.5 * LIGHT_HOUR_US).await;

        let told = heard(&mut wire, ClientId(2));
        assert_eq!(told.len(), 1, "{told:?}");
        assert_eq!(told[0].arrive_t, (t0 as f64 + (ahead - from).length()).ceil() as i64);
        assert!(heard(&mut wire, ClientId(3)).is_empty(), "heard light that passed before it was made");
    }

    /// A craft that turns away while the light is on its way hears it where the light catches it,
    /// not where it would have been.
    #[tokio::test]
    async fn a_craft_that_turns_away_hears_it_where_the_light_catches_it() {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        let (t0, from) = shout(&mut server, &mut wire, 1.0e12).await;
        let far = DVec3::X * LIGHT_HOUR_US;
        server.admit(ClientId(2), still(ShipId(2), far), 0.0);
        let stayed_t = (t0 as f64 + (far - from).length()).ceil() as i64;

        run_until(&mut server, &mut wire, t0 as f64 + 0.25 * LIGHT_HOUR_US).await;
        let turned_s = server.now_t() as f64 * 1.0e-6;
        let craft = server.fleet.get_mut(CraftId(2)).unwrap();
        craft.drift_from(far / LIGHT_US_PER_LY, DVec3::X * 0.5, turned_s);
        let caught = lc_spacetime::arrival_time_at(t0 as f64, from, &craft.worldline()).unwrap();
        run_until(&mut server, &mut wire, caught + 0.1 * LIGHT_HOUR_US).await;

        let told = heard(&mut wire, ClientId(2));
        assert_eq!(told.len(), 1, "{told:?}");
        assert_eq!(told[0].arrive_t, caught.ceil() as i64);
        assert!(told[0].arrive_t > stayed_t + (0.5 * LIGHT_HOUR_US) as i64, "premise: the turn mattered");
    }

    /// The runs find exactly what checking every front against every craft finds, with sources and
    /// craft moving at 0.9c, a source that jumps, and a craft that jumps mid-tick.
    #[test]
    fn the_runs_find_what_checking_every_front_finds() {
        use lc_world::craft::Fleet;
        use lc_world::signal::Beam;

        use super::{Front, Fronts};
        use crate::world::{Event, schedule};

        let mut seed = 0x5eed_u64;
        let mut unit = move || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            (seed >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
        };
        let span = 4.0 * LIGHT_HOUR_US;
        let starts: Vec<DVec3> = (0..5).map(|_| DVec3::new(unit(), unit(), unit()) * span).collect();
        let mut fleet = Fleet::new();
        for id in 1..=6 {
            let at = DVec3::new(unit(), unit(), unit()) * span;
            match id % 2 {
                0 => fleet.insert(crate::world::coasting(ShipId(id), at, (starts[0] - at).normalize() * 0.9, 0)),
                _ => fleet.insert(still(ShipId(id), at)),
            }
        }
        let (step, tick) = (60_000_000_i64, 300_000_000_i64);
        let jumper = CraftId(1);
        let mut fronts = Vec::new();
        for source in 0..5_i64 {
            let mut at = starts[source as usize];
            let velocity = DVec3::new(unit(), unit(), unit()).normalize() * 0.9;
            for k in 0..40_i64 {
                let t = k * step;
                // The last source jumps three light-hours and back, five emissions apart.
                let jump = match k % 10 {
                    4 => 3.0,
                    9 => -3.0,
                    _ => 0.0,
                };
                at += if source == 4 && jump != 0.0 { DVec3::X * jump * LIGHT_HOUR_US } else { velocity * step as f64 };
                let event = Event { id: source * 1_000 + k, source: ShipId(100 + source), t, at, kind: 1, power_w: 1.0e30, payload: "{}".into() };
                fronts.push(Front::new(event, Beam::OMNI, Vec::new(), None, None));
            }
        }

        // A microsecond after light lands on it, in the same tick, so what landed before the jump
        // is somewhere the craft no longer is.
        let jump_t = fronts
            .iter()
            .filter_map(|f| schedule(&f.event, &Beam::OMNI, fleet.get(jumper).unwrap()))
            .map(|l| l.arrive_t + 1)
            .filter(|t| (t + 1).rem_euclid(tick) > 1)
            .min()
            .expect("premise: light lands on the jumper");
        let mut index = Fronts::default();
        let mut found = Vec::new();
        let mut now = -1;
        let end = 40 * step + 3 * span as i64;
        let mut launched = 0;
        while now < end {
            let after = now;
            now += tick;
            if jump_t > after && jump_t <= now {
                let craft = fleet.get_mut(jumper).unwrap();
                let landing_ly = (craft.position_at(jump_t as f64) + DVec3::Y * 2.0 * LIGHT_HOUR_US) / LIGHT_US_PER_LY;
                craft.teleport_drifting(None, landing_ly, DVec3::ZERO, jump_t as f64 * 1.0e-6);
            }
            for front in fronts.iter().filter(|f| f.event.t > after && f.event.t <= now) {
                index.launch(front.clone());
                launched += 1;
            }
            found.extend(index.sweep(&fleet, after, now).into_iter().map(|(_, l)| (l.observer.0, l.event, l.arrive_t)));
        }
        assert_eq!(launched, fronts.len());
        assert!(index.runs.len() > 5, "premise: the jumps split a run, {} runs", index.runs.len());

        let mut every: Vec<(i64, i64, i64)> = fronts
            .iter()
            .flat_map(|f| fleet.iter().filter_map(|c| schedule(&f.event, &Beam::OMNI, c)).map(|l| (l.observer.0, l.event, l.arrive_t)))
            .filter(|(.., arrive_t)| *arrive_t <= end)
            .collect();
        every.sort_unstable();
        found.sort_unstable();
        assert!(every.len() > 500, "premise: plenty lands, {}", every.len());
        assert_eq!(found, every);
    }

    /// Light is followed as far as it is bright enough to detect, and then let go.
    #[tokio::test]
    async fn light_too_faint_to_detect_is_not_delivered_and_is_let_go() {
        let power_w = 1.0;
        let reach_us = (power_w / FAINTEST as f64).sqrt();
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut wire = Loopback::new();
        server.admit(ClientId(2), still(ShipId(2), DVec3::X * 0.9 * reach_us), 0.0);
        server.admit(ClientId(3), still(ShipId(3), DVec3::Y * 1.1 * reach_us), 0.0);
        let (t0, _) = shout(&mut server, &mut wire, power_w).await;
        assert_eq!(server.fronts.len(), 1);

        run_until(&mut server, &mut wire, t0 as f64 + 1.2 * reach_us).await;
        assert_eq!(heard(&mut wire, ClientId(2)).len(), 1, "inside its reach");
        assert!(heard(&mut wire, ClientId(3)).is_empty(), "heard beyond its reach");
        for _ in 0..super::COMPACT_EVERY {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(server.fronts.is_empty(), "still followed after it faded");
    }
}
