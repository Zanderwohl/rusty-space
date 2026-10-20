//! Light on its way, followed until it is too faint to matter.
//!
//! Every event's light is a sphere growing at `c` from where it happened, for as long as it is
//! brighter than [`crate::world::FAINTEST`] somewhere it could land: its reach, which its power and
//! beam fix the moment it happens. Each tick finds the craft the sphere's surface passed during it —
//! whichever craft there are by then, on the courses they actually flew — and those are its
//! deliveries. Working them out when the event was written was exact only for a craft that already
//! existed and never changed course, and light can be thirty-six thousand years on the way.
//!
//! The search rests on one fact: light from a source slower than light reaches any point in the order
//! it left, because `t + |x - w(t)|` rises with `t` along a subluminal worldline. So each source's
//! fronts, in order of emission, are in order of arrival at every craft too, and a craft finds what
//! lands on it this tick by a binary search per source rather than a look at every front. A jump
//! breaks that order, so a front that would break it starts a new run. A spatial index cannot do
//! the same: the question is a thin shell about the craft's past light cone, which a tree over
//! where and when cuts through at `f^(3/4)` of its leaves. See `lightcone/docs/02-event-store.md`.

use std::collections::HashMap;

use lc_world::craft::{Craft, CraftId, Fleet};
use lc_world::signal::Beam;

use crate::journal::{Journal, JournalError};
use crate::radio::Utterance;
use crate::server::Server;
use crate::world::{Event, Scheduled, reach_us, schedule};

/// Sweeps between clearing faded fronts out of the buckets. Until then they are skipped.
const COMPACT_EVERY: u32 = 1_200;

/// One event's light, and everything that hearing it does.
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
    /// Through when its landings have been found, when that is later than the last sweep: one
    /// crossed as it was made is not found again by the next.
    found_t: i64,
}

impl Front {
    pub(crate) fn new(event: Event, beam: Beam, also: Vec<CraftId>, deaf: Option<CraftId>, said: Option<Utterance>) -> Self {
        // The beam's own gain: `also` hears it omnidirectionally, which never carries further.
        let reach = reach_us(event.power_w, &beam);
        let until_t = event.t.saturating_add(reach.ceil() as i64);
        Self { event, beam, also, deaf, said, until_t, found_t: i64::MIN }
    }

    /// Where and when its light lands on `craft`, if it is bright enough there to be detected.
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
    /// Each source's runs, by its id.
    by_source: HashMap<i64, Vec<usize>>,
    /// Made since the last sweep. See [`Fronts::sweep_fresh`].
    fresh: Vec<Front>,
    /// Made since the journal last kept them.
    unkept: Vec<Front>,
    /// Landings found since the journal last wrote them.
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

    /// Put back fronts a journal kept, as a shard comes back.
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

    /// Take back fronts made this tick whose events were never sent after all, and what they
    /// were found to have delivered.
    pub fn withdraw(&mut self, event_ids: &[i64]) {
        self.fresh.retain(|f| !event_ids.contains(&f.event.id));
        self.unkept.retain(|f| !event_ids.contains(&f.event.id));
        self.crossed.retain(|d| !event_ids.contains(&d.event));
        for front in self.live.iter_mut().filter(|f| event_ids.contains(&f.event.id)) {
            front.until_t = i64::MIN;
        }
    }

    /// The landings found since this was last called, for the journal.
    pub fn take_crossed(&mut self) -> Vec<Scheduled> {
        std::mem::take(&mut self.crossed)
    }

    pub fn take_unkept(&mut self) -> Vec<Front> {
        std::mem::take(&mut self.unkept)
    }

    /// Hand back what the journal could not keep, to be tried with the next.
    pub fn unkeep(&mut self, mut fronts: Vec<Front>) {
        fronts.append(&mut self.unkept);
        self.unkept = fronts;
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
            for run in &self.runs {
                let from = run.partition_point(|&i| lands(&self.live[i]) <= after_t as f64 - width);
                for &index in &run[from..] {
                    let front = &self.live[index];
                    if lands(front) > now_t as f64 + width {
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

    /// Every landing by `now_t` of the fronts made since the last sweep, which then join the rest.
    /// Their first window opens at their own emission: an order is stamped as far back as its
    /// client's cursor, and a collapse's light can land on a neighbor in the tick it happens.
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
    pub(crate) fn to_row(&self) -> Option<lc_store::fronts::Front> {
        let spread = Spread {
            at: self.event.at.to_array(),
            power_w: self.event.power_w,
            axis: self.beam.axis.to_array(),
            half_angle_rad: self.beam.half_angle_rad,
            also: self.also.iter().map(|c| c.0).collect(),
            deaf: self.deaf.map(|c| c.0),
        };
        Some(lc_store::fronts::Front {
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
        })
    }

    /// The front a kept row was, with what it said if it was speech.
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
    /// Every landing in `(after_t, now]` of light put out before this tick, on whatever craft it
    /// landed on.
    pub(crate) fn cross_fronts(&mut self, after_t: i64) {
        let found = self.fronts.sweep(&self.fleet, after_t, self.now_t);
        self.landed(found);
    }

    /// Send light on its way, and land at once what of it lands by now: a neighbor's spike has to
    /// land in time order among the collapses still to come in the tick that made it.
    pub(crate) fn launch(&mut self, front: Front) {
        self.fronts.launch(front);
        let found = self.fronts.sweep_fresh(&self.fleet, self.now_t);
        self.landed(found);
    }

    /// Everything light did as it landed: a delivery, and for speech its receipt and any answer
    /// owed, for an emission its energy, for a report what it teaches.
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

    /// Hand the journal the fronts made this tick, keeping them to try again if it will not take
    /// them.
    pub(crate) async fn keep_fronts(&mut self) -> Result<(), JournalError> {
        let made = self.fronts.take_unkept();
        if let Err(why) = self.journal.keep_fronts(&made).await {
            self.fronts.unkeep(made);
            return Err(why);
        }
        Ok(())
    }

    /// Follow on the light the last shard was following. Read once, when a shard comes back.
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

    /// The runs are an index and nothing more: swept a tick at a time they find exactly the landings
    /// a check of every front against every craft finds, with sources scattered through deep
    /// space, moving at up to most of `c`, and one that jumps.
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
        let mut fleet = Fleet::new();
        for id in 1..=6 {
            fleet.insert(still(ShipId(id), DVec3::new(unit(), unit(), unit()) * span));
        }
        let (step, tick) = (60_000_000_i64, 300_000_000_i64);
        let mut fronts = Vec::new();
        for source in 0..5_i64 {
            let mut at = DVec3::new(unit(), unit(), unit()) * span;
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

        let mut index = Fronts::default();
        let mut found = Vec::new();
        let mut now = -1;
        let end = 40 * step + 3 * span as i64;
        let mut launched = 0;
        while now < end {
            let after = now;
            now += tick;
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
