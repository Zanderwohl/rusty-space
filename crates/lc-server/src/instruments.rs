//! Instruments that run whether or not anybody is flying them, and the knowledge they fill.
//!
//! Every craft's knowledge lives here, not in its client. Duties advance and reports land with
//! nobody signed in; a connected client is sent what its craft learned as a report, which it
//! folds into its copy. See `lightcone/docs/24-standing-instruments.md`.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

use lc_proto::{Order, Outbound, Refusal, ShipId};
use lc_world::craft::CraftId;
use lc_world::fitting::ONBOARD_DATA_BYTES;
use lc_world::knowledge::observatory::{Observatory, Sky, Station};
use lc_world::knowledge::survey::Duty;
use lc_world::knowledge::prior::Prior;
use lc_world::knowledge::{ENTRIES_PER_REPORT, Knowledge, Mark, Report, Reporting, Subject, Witness};
use lc_world::motion::LIGHT_US_PER_LY;
use lc_world::sky::CatalogStar;

use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// Systems per page of a craft's knowledge sent to a client that has just signed in. A
/// surveyed sky is thousands of systems, so it arrives over a few ticks.
const PAGE: usize = 256;

/// Samples per page of a craft's own logs, before the byte bound shrinks it.
const LOG_PAGE: usize = 20_000;

/// Bytes a page may take, whichever stream. Far inside [`lc_proto::FRAME_LIMIT`]: a page over
/// that is refused by the client, which would reconnect and be sent the same page again.
pub(crate) const PAGE_BYTES: usize = 1 << 20;

/// Logs read per tick across the whole shard. Reading one is a search over thousands of
/// periods.
const READS_PER_TICK: usize = 1;

/// Orbit fits started per tick across the whole shard.
///
/// Solved off the tick by [`crate::fits`], which also caps how many run at once. Round-robin by
/// craft, and within a craft by which body waited longest.
const FITS_PER_TICK: usize = 1;

#[derive(Clone, Debug)]
pub(crate) struct Aboard {
    pub knowledge: Knowledge,
    pub observatory: Observatory,
    pub reporting: Reporting,
}

#[derive(Clone, Debug)]
pub(crate) struct Landing {
    observer: CraftId,
    arrive_t: i64,
    strength: f32,
    body: String,
}

#[derive(Default)]
pub(crate) struct Instruments {
    pub aboard: HashMap<CraftId, Aboard>,
    /// Built lazily and shared by every craft: a star's output does not depend on who looks.
    sky: Option<Sky>,
    /// The generator's planet population, which a log is read against.
    pub(crate) prior: PriorCell,
    /// Round-robin cursor for [`READS_PER_TICK`].
    reader: Option<CraftId>,
    /// Round-robin cursor for [`FITS_PER_TICK`].
    fitter: Option<CraftId>,
    fits: crate::fits::Fits,
    /// Round-robin cursor for the one full recount per tick.
    recounted: Option<CraftId>,
    landings: Vec<Landing>,
}

impl Instruments {
    /// Drop what was cached from the last world, and start measuring this one's prior.
    pub(crate) fn forget_sky(&mut self, stars: Arc<Vec<CatalogStar>>) {
        self.sky = None;
        self.prior = PriorCell::measuring(stars);
    }
}

/// [`Prior::measure`], run on its own thread from the moment a world is loaded.
///
/// Over a full catalog it generates fifteen hundred systems, and it used to do that on the tick
/// thread at the first log anyone read.
#[derive(Default)]
pub(crate) struct PriorCell {
    held: Option<Prior>,
    /// See [`crate::fits::Fits::done`] for why this is behind a `Mutex`.
    coming: Option<Mutex<Receiver<Prior>>>,
}

impl PriorCell {
    fn measuring(stars: Arc<Vec<CatalogStar>>) -> Self {
        let (send, coming) = channel();
        std::thread::spawn(move || {
            let _ = send.send(Prior::measure(stars.iter()));
        });
        Self { held: None, coming: Some(Mutex::new(coming)) }
    }

    /// The prior, or `None` while it is still being measured; whatever needs it waits a tick.
    /// Measured here and now if no world was ever loaded, which is where a test runs.
    ///
    /// A test waits for it instead, so what a tick does does not depend on how fast a thread is.
    pub(crate) fn get(&mut self, stars: &[CatalogStar]) -> Option<&Prior> {
        if self.held.is_none() {
            match &self.coming {
                #[cfg(not(test))]
                Some(coming) => {
                    self.held = coming.lock().unwrap_or_else(std::sync::PoisonError::into_inner).try_recv().ok()
                }
                #[cfg(test)]
                Some(coming) => {
                    self.held = coming.lock().unwrap_or_else(std::sync::PoisonError::into_inner).recv().ok()
                }
                None => self.held = Some(Prior::measure(stars.iter())),
            }
        }
        self.held.as_ref()
    }
}

pub(crate) fn witness(id: CraftId) -> Witness {
    Witness(id.0 as u64)
}

/// The largest page `build` makes that fits [`PAGE_BYTES`], and what to resume from. `None`
/// when there is nothing new.
///
/// A single item larger than a page is sent alone if it fits a frame. One that does not is
/// skipped (body `None`, mark moved past it), because sending it would close the connection on
/// every reconnection.
fn page<T>(mut limit: usize, build: impl Fn(usize) -> (Option<String>, Option<T>)) -> Option<(Option<String>, T)> {
    loop {
        let (body, through) = build(limit);
        let (body, through) = (body?, through?);
        if body.len() <= PAGE_BYTES || (limit == 1 && body.len() < lc_proto::FRAME_LIMIT / 2) {
            return Some((Some(body), through));
        }
        if limit == 1 {
            return Some((None, through));
        }
        limit /= 2;
    }
}

impl<J: Journal> Server<J> {
    fn station(&self, id: CraftId) -> Option<Station> {
        let craft = self.fleet.get(id)?;
        let position_ly = craft.position_at(self.now_t as f64) / LIGHT_US_PER_LY;
        Some(Station { position_ly, instrument: craft.sensor })
    }

    /// A craft's instruments. **A new craft knows nothing** -- not its home system's planets and
    /// not the stars around it either. Everything it holds it looked at or was told.
    ///
    /// This is where the charting office of `lightcone/docs/22-provenance.md` used to hand a new
    /// ship twenty light-years of star distances. The survey from inside replaced the argument
    /// for it: a ship that knows nothing is not idle, it is in a system full of unexamined
    /// planets that pay out in the first real minute. `observatory::issue_charts` stays for
    /// photographs and tests; nothing a player reaches calls it.
    pub(crate) fn aboard(&mut self, id: CraftId) -> &mut Aboard {
        self.instruments.aboard.entry(id).or_insert_with(|| Aboard {
            knowledge: Knowledge::new(witness(id)),
            observatory: Observatory::default(),
            reporting: Reporting::default(),
        })
    }

    /// Bytes: data modules plus the onboard store.
    fn data_capacity(&self, id: CraftId) -> f64 {
        let now_s = self.now_t as f64 * 1.0e-6;
        self.fleet
            .get(id)
            .and_then(|c| c.fitting())
            .map_or(ONBOARD_DATA_BYTES, |f| f.balance.data_capacity(&f.loadout_at(now_s)))
    }

    /// Recount what a craft's knowledge takes if its room changed, or if `recount`.
    ///
    /// Samples keep the count as they arrive; anything else is noticed only at a recount, which
    /// is a pass over every file, so it runs on change and otherwise one craft a tick.
    fn fit(&mut self, id: CraftId, recount: bool) {
        let capacity = self.data_capacity(id);
        if let Some(aboard) = self.instruments.aboard.get_mut(&id)
            && (recount || aboard.knowledge.capacity_bytes() != capacity)
        {
            aboard.knowledge.fit_to(capacity);
        }
    }

    pub(crate) fn run_instruments(&mut self) {
        let now_s = self.now_t as f64 * 1.0e-6;
        // The surveyed star comes along because loading its system needs the world mutably and
        // the tick below needs the instruments mutably.
        let busy: Vec<(CraftId, Option<lc_world::sky::StarId>)> = self
            .instruments
            .aboard
            .iter()
            .filter(|(_, a)| a.observatory.duty != Duty::Idle)
            .map(|(id, a)| (*id, a.observatory.duty.surveying()))
            .collect();
        let mut ids: Vec<CraftId> = self.instruments.aboard.keys().copied().collect();
        ids.sort_unstable_by_key(|id| id.0);
        let next = self.instruments.recounted.map_or(0, |r| ids.partition_point(|id| id.0 <= r.0));
        if let Some(&id) = ids.get(next).or(ids.first()) {
            self.fit(id, true);
            self.instruments.recounted = Some(id);
        }
        for (id, surveyed) in busy {
            self.fit(id, false);
            let Some(at) = self.station(id) else { continue };
            // The system the *duty* names, not the one the craft is in. A survey ordered from
            // outside is then refused by the physics -- the bodies are points in the star's
            // glare -- rather than by a silent special case here.
            let system = surveyed.and_then(|star| self.world.system_for(star, now_s));
            let stars = self.world.stars();
            let instruments = &mut self.instruments;
            let sky = instruments.sky.get_or_insert_with(|| Sky::new(stars));
            let Some(aboard) = instruments.aboard.get_mut(&id) else { continue };
            aboard.observatory.tick(sky, system.as_deref(), &mut aboard.knowledge, at, now_s);
        }
        self.stages.mark("observe");
        self.read_logs(now_s);
        self.stages.mark("read_logs");
        self.fit_orbits(now_s);
        self.stages.mark("fit_orbits");
    }

    /// File the fits that have finished, and start one more. See [`FITS_PER_TICK`].
    ///
    /// The star's *believed* position is what the bearings are put into the frame of, so a
    /// craft that has not measured its own sun's distance fits nothing -- which is the chain
    /// doc 25 describes, and the reason the survey measures the star every tick.
    fn fit_orbits(&mut self, now_s: f64) {
        let finished = self.instruments.fits.finished();
        self.file_fits(finished);
        let mut ids: Vec<CraftId> = self.instruments.aboard.keys().copied().collect();
        ids.sort_unstable_by_key(|id| id.0);
        let after = self.instruments.fitter.map_or(0, |r| ids.partition_point(|id| id.0 <= r.0));
        ids.rotate_left(after);
        let mut started = 0;
        for id in ids {
            if started == FITS_PER_TICK || self.instruments.fits.full() {
                break;
            }
            if self.instruments.fits.busy(id) {
                continue;
            }
            let Some(aboard) = self.instruments.aboard.get_mut(&id) else { continue };
            let Some(star) = aboard.observatory.duty.surveying() else { continue };
            let Some(star_ly) = aboard
                .knowledge
                .belief(lc_world::knowledge::Subject::Star(star))
                .and_then(|b| b.distance.position_ly())
            else {
                continue;
            };
            let Some(subject) = aboard.knowledge.unfitted(star) else { continue };
            let Some(job) = aboard.knowledge.fit_job(subject, star_ly, now_s) else { continue };
            self.instruments.fitter = Some(id);
            started += 1;
            self.instruments.fits.start(id, job);
        }
    }

    /// No recount after: a fit changes no samples, and a recount is a pass over every file.
    fn file_fits(&mut self, finished: Vec<(CraftId, lc_world::knowledge::primary::Solved)>) {
        for (id, solved) in finished {
            if let Some(aboard) = self.instruments.aboard.get_mut(&id) {
                aboard.knowledge.file_fit(solved);
            }
        }
    }

    /// Wait for every fit in flight and file it. A test's way to see a fit land.
    #[cfg(test)]
    pub(crate) fn settle_fits(&mut self) {
        let finished = self.instruments.fits.wait();
        self.file_fits(finished);
    }

    fn read_logs(&mut self, now_s: f64) {
        let mut ids: Vec<CraftId> = self.instruments.aboard.keys().copied().collect();
        ids.sort_unstable_by_key(|id| id.0);
        let after = self.instruments.reader.map_or(0, |r| ids.partition_point(|id| id.0 <= r.0));
        ids.rotate_left(after);
        let mut reads = 0;
        for id in ids {
            if reads == READS_PER_TICK {
                break;
            }
            let Some((subject, observer)) = self.instruments.aboard.get(&id).and_then(|a| a.knowledge.next_due())
            else {
                continue;
            };
            let stars = self.world.stars();
            let instruments = &mut self.instruments;
            let Some(prior) = instruments.prior.get(&stars) else { return };
            let Some(aboard) = instruments.aboard.get_mut(&id) else { continue };
            let settled = aboard
                .knowledge
                .read_log(subject, observer, prior, now_s)
                .as_ref()
                .and_then(crate::planets::settled_planet);
            instruments.reader = Some(id);
            reads += 1;
            // A settled transit is a body from here on: lettered, with the orbit its period
            // implies. See `crate::planets`, and doc 25's "Transits make bodies".
            if let Some(transit) = settled {
                self.found_by_transit(id, subject, &transit, now_s);
            }
            self.fit(id, true);
        }
    }

    /// Note where this tick's reports will land. Read off the deliveries, which already hold
    /// who was covered, when, and how loud.
    pub(crate) fn schedule_landings(&mut self, events: &[Event], deliveries: &[Scheduled]) {
        for event in events.iter().filter(|e| e.kind == lc_proto::kind::REPORT) {
            for scheduled in deliveries.iter().filter(|d| d.event == event.id) {
                // Redacted as a client is sent it, so a sealed report teaches only its addressee.
                let payload = crate::radio::redact(event.kind, &event.payload, scheduled.observer);
                let Ok(reported) = serde_json::from_str::<lc_proto::Reported>(&payload) else {
                    eprintln!("WARNING: report {} is not a report; it will not land", event.id);
                    continue;
                };
                if reported.format != lc_proto::REPORT_FORMAT {
                    eprintln!(
                        "WARNING: report {} is in format {}, not {}; it will not land",
                        event.id,
                        reported.format,
                        lc_proto::REPORT_FORMAT,
                    );
                    continue;
                }
                let Some(body) = reported.body else { continue };
                self.instruments.landings.push(Landing {
                    observer: CraftId(scheduled.observer.0),
                    arrive_t: scheduled.arrive_t,
                    strength: scheduled.strength,
                    body,
                });
            }
        }
    }

    /// Fold in every report whose light has arrived, signed in or not.
    pub(crate) fn land_reports(&mut self) {
        let now = self.now_t;
        let (due, pending): (Vec<Landing>, Vec<Landing>) =
            std::mem::take(&mut self.instruments.landings).into_iter().partition(|l| l.arrive_t <= now);
        self.instruments.landings = pending;
        for landing in due {
            // Under the noise floor it was not received.
            let floor = self.fleet.get(landing.observer).map(|c| c.noise_floor).unwrap_or(f32::INFINITY);
            if landing.strength < floor {
                continue;
            }
            let Ok(report) = serde_json::from_str::<Report>(&landing.body) else {
                eprintln!("WARNING: a report landing on craft {} would not parse", landing.observer.0);
                continue;
            };
            let arrived_s = landing.arrive_t as f64 * 1.0e-6;
            self.aboard(landing.observer);
            self.fit(landing.observer, false);
            self.aboard(landing.observer).knowledge.receive(&report, arrived_s);
        }
    }

    /// Send every connected client at most one page each of what its craft has learned and of
    /// its own samples.
    pub(crate) fn tell_learned(&mut self, wire: &mut impl Transport) {
        let now_s = self.now_t as f64 * 1.0e-6;
        let connected: Vec<(lc_proto::ClientId, ShipId, Mark, f64, bool)> = self
            .clients
            .iter()
            .map(|(c, state)| (*c, state.ship, state.learned, state.logged_s, state.retained_sent))
            .collect();
        for (client, ship, since, logged, retained_sent) in connected {
            let knowledge = &self.aboard(CraftId(ship.0)).knowledge;
            let learned = page(PAGE, |limit| {
                let (report, through) = knowledge.report_upto(since, now_s, limit);
                // Nothing new is the usual answer, and not worth encoding an empty report for.
                (through.and_then(|_| serde_json::to_string(&report).ok()), through)
            });
            let retained = knowledge.retained_subjects();
            let logs = page(LOG_PAGE, |limit| {
                let (logs, through) = knowledge.logs_upto(logged, limit);
                if through.is_none() {
                    return (None, None);
                }
                let page = lc_world::knowledge::Logs { logs, retained: retained.clone() };
                (serde_json::to_string(&page).ok(), through)
            });
            // Send what the craft keeps raw once even with no logs; later log pages and accepted
            // orders carry it after that.
            let logs = match logs {
                None if !retained_sent && !retained.is_empty() => serde_json::to_string(&lc_world::knowledge::Logs { logs: Vec::new(), retained })
                    .ok()
                    .map(|body| (Some(body), logged)),
                other => other,
            };
            if let Some(state) = self.clients.get_mut(&client) {
                state.retained_sent = true;
            }
            if let Some((body, through)) = learned {
                if let Some(report) = body {
                    wire.send(client, Outbound::Learned { report });
                }
                if let Some(state) = self.clients.get_mut(&client) {
                    state.learned = through;
                }
            }
            if let Some((body, through)) = logs {
                if let Some(logs) = body {
                    wire.send(client, Outbound::Logged { logs });
                }
                if let Some(state) = self.clients.get_mut(&client) {
                    state.logged_s = through;
                }
            }
        }
    }

    pub(crate) fn tell_observing(&mut self, wire: &mut impl Transport, client: lc_proto::ClientId, id: CraftId) {
        let observatory = &self.aboard(id).observatory;
        let observing = Outbound::Observing {
            duty: (&observatory.duty).into(),
            integration_s: observatory.integration_s,
        };
        wire.send(client, observing);
    }

    /// Instrument and knowledge orders. Nothing goes on the air, so they are not events. Returns
    /// the order as applied, with a sweep's or watch's actual start time.
    pub(crate) fn act_on_knowledge(&mut self, id: CraftId, order: &Order, at_s: f64) -> Result<Order, Refusal> {
        match order {
            Order::SetDuty { duty, integration_s } => {
                let integration_ok = (0.0..=lc_proto::INTEGRATION_MAX_S).contains(integration_s);
                if !integration_ok || !duty.is_valid() {
                    return Err(Refusal::Impossible);
                }
                // `Duty::is_valid` cannot check a star id, because `lc-proto` has no catalog
                // to check it against. Refused here instead: a duty pointed at a star nobody
                // has is a duty that finds nothing every tick forever, and a survey's does not
                // even cache the miss.
                let named = match lc_world::knowledge::survey::Duty::from(duty) {
                    lc_world::knowledge::survey::Duty::Survey { star, .. } => Some(star),
                    lc_world::knowledge::survey::Duty::Stare(star) => Some(star),
                    _ => None,
                };
                if let Some(star) = named
                    && !self.world.holds(star)
                {
                    return Err(Refusal::Impossible);
                }
                let aboard = self.aboard(id);
                aboard.observatory.integration_s = integration_s.max(1.0);
                aboard.observatory.take_up(duty.into(), at_s);
                Ok(Order::SetDuty {
                    duty: (&aboard.observatory.duty).into(),
                    integration_s: aboard.observatory.integration_s,
                })
            }
            Order::NameIt { subject, name } => {
                let name = name.trim();
                if name.is_empty() || name.len() > lc_proto::NAME_LIMIT {
                    return Err(Refusal::Impossible);
                }
                let subject = Subject::from(*subject);
                let here = self.fleet.get(id).and_then(|craft| craft.system.clone());
                let knowledge = &mut self.aboard(id).knowledge;
                if !knowledge.nameable(subject, here.as_deref()) {
                    return Err(Refusal::Impossible);
                }
                knowledge.name_it(subject, name, at_s);
                Ok(Order::NameIt { subject: subject.into(), name: name.to_string() })
            }
            Order::RetainRaw { subject, keep } => {
                let knowledge = &mut self.aboard(id).knowledge;
                if !knowledge.knows(Subject::from(*subject)) {
                    return Err(Refusal::Impossible);
                }
                knowledge.retain_raw(Subject::from(*subject), *keep);
                Ok(order.clone())
            }
            Order::Analyze => {
                self.aboard(id).knowledge.analyze();
                Ok(Order::Analyze)
            }
            _ => Err(Refusal::Impossible),
        }
    }

    /// The report a craft would send `to` now, and the mark it would advance to. Built from the
    /// shard's knowledge, never the client's.
    ///
    /// Shrunk until it fits [`lc_proto::REPORT_LIMIT`]: [`ENTRIES_PER_REPORT`] counts systems,
    /// and one system's file grows without bound over a long watch.
    pub(crate) fn report_for(&self, id: CraftId, to: Option<ShipId>, at_s: f64) -> Result<(String, Mark), Refusal> {
        let aboard = self.instruments.aboard.get(&id).ok_or(Refusal::NothingNew)?;
        let since = aboard.reporting.since(to.map_or(0, |t| t.0));
        let mut limit = ENTRIES_PER_REPORT;
        loop {
            let (report, through) = aboard.knowledge.report_upto(since, at_s, limit);
            let through = through.ok_or(Refusal::NothingNew)?;
            let body = serde_json::to_string(&report).map_err(|_| Refusal::Impossible)?;
            if body.len() <= lc_proto::REPORT_LIMIT {
                return Ok((body, through));
            }
            if limit == 1 {
                return Err(Refusal::Impossible);
            }
            limit /= 2;
        }
    }

    /// A report went out: this craft has now told `to` everything through `through`.
    pub(crate) fn reported(&mut self, id: CraftId, to: Option<ShipId>, through: Mark) {
        self.aboard(id).reporting.sent(to.map_or(0, |t| t.0), through);
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_proto::{ClientId, Inbound, Intent, PROTOCOL_VERSION};
    use lc_world::knowledge::{Bearing, Sighting};
    use lc_world::sky::{AuthoredStars, CatalogStar, StarId, StarProvider};

    use super::*;
    use crate::journal::Memory;
    use crate::server::TICK_US;
    use crate::testing::Broker;
    use crate::ticket::Trusted;
    use crate::transport::Loopback;
    use crate::world::World;

    const SHARD: &str = "shard-1";
    const MONTH_US: i64 = 30 * 86_400 * 1_000_000;

    /// A home star at the origin, where a new craft starts, and three more thirty light-years
    /// out along axes clear of the home star's glare. The charts reach twenty, so only a sweep
    /// finds the three.
    fn sky() -> Vec<CatalogStar> {
        let template = AuthoredStars::sample().stars()[1].clone();
        [DVec3::ZERO, DVec3::X * 30.0, DVec3::Y * 30.0, DVec3::Z * 30.0]
            .into_iter()
            .enumerate()
            .map(|(k, at)| {
                let mut star = template.clone();
                star.id = StarId::synthesize("instruments", k as u64);
                star.position_ly = at;
                star
            })
            .collect()
    }

    fn server(broker: &Broker) -> Server<Memory> {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut trusted = Trusted::new(SHARD);
        trusted.learn(&broker.jwks());
        server.trust(trusted);
        server.load_world(World::new(sky()));
        server
    }

    /// Sign in, and everything said on the welcoming tick, including the first knowledge page.
    async fn sign_in(
        server: &mut Server<Memory>,
        wire: &mut Loopback,
        client: ClientId,
        ticket: String,
    ) -> (ShipId, Vec<Outbound>) {
        wire.client_says(client, Inbound::Hello { protocol: PROTOCOL_VERSION, ticket });
        server.tick(wire).await.unwrap();
        let said = wire.take(client);
        let ship = said
            .iter()
            .find_map(|m| match m {
                Outbound::Welcome { ship_id, .. } => Some(*ship_id),
                _ => None,
            })
            .expect("welcomed");
        (ship, said)
    }

    /// Folds reports as the client does.
    fn replica(ship: ShipId, messages: &[Outbound]) -> Knowledge {
        let mut copy = Knowledge::new(witness(CraftId(ship.0)));
        for message in messages {
            if let Outbound::Learned { report } = message {
                copy.absorb(&serde_json::from_str(report).expect("a report"));
            }
        }
        copy
    }

    fn act(ship: ShipId, order: Order) -> Inbound {
        Inbound::Act(Intent { ship_id: ship, order, issued_at_client_t: i64::MAX })
    }

    /// **A new craft knows nothing** (decided 2026-09-22): not its home system's planets, and
    /// not the star it is standing next to either. Everything it holds it looked at or was
    /// told, and the first minutes of a new ship are looking.
    ///
    /// This is the charting office of `lightcone/docs/22-provenance.md` gone. It used to hand a
    /// new ship twenty light-years of star distances, and the argument was that a player who
    /// starts with nothing has no reason to fly anywhere. The survey from inside answers that:
    /// a ship that knows nothing is in a system full of unexamined planets.
    #[tokio::test]
    async fn a_new_craft_knows_nothing_at_all() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, said) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let copy = replica(ship, &said);
        assert!(!copy.knows(sky()[0].id), "not even the star it is standing next to");
        assert_eq!(copy.stars().count(), 0, "nor any other");

        // And looking is what changes that: one stare and it holds the star.
        wire.client_says(
            ClientId(1),
            act(ship, Order::SetDuty {
                duty: lc_proto::Duty::Stare { star: sky()[0].id.get() },
                integration_s: 1.0,
            }),
        );
        for _ in 0..4 {
            server.tick(&mut wire).await.unwrap();
        }
        assert!(
            server.instruments.aboard[&CraftId(ship.0)].knowledge.knows(sky()[0].id),
            "a stare finds what a chart used to be given"
        );
    }

    /// With nobody signed in, a craft keeps sweeping and receives a report when its light lands;
    /// signing back in hands the client all of it.
    #[tokio::test]
    async fn a_craft_keeps_observing_and_listening_while_nobody_is_signed_in() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let sweep = lc_proto::Duty::Sweep { center: [0.0, 0.0, 1.0], radius_rad: std::f64::consts::PI, dwell_s: 60.0, started_s: 0.0 };
        wire.client_says(ClientId(1), act(ship, Order::SetDuty { duty: sweep, integration_s: 1.0e4 }));
        server.tick(&mut wire).await.unwrap();
        assert!(wire.take(ClientId(1)).iter().any(|m| matches!(m, Outbound::Accepted { .. })));
        server.disconnected(ClientId(1));

        // A second craft a light-hour out, which knows one star the first does not.
        let bry = ClientId(2);
        let at = -DVec3::X * 3_600.0 * 1.0e6;
        server.admit(bry, crate::world::still(ShipId(90), at), 0.0);
        let secret = StarId::synthesize("instruments", 99);
        let now_s = server.now_t() as f64 * 1.0e-6;
        server.aboard(CraftId(90)).knowledge.sighted(
            secret,
            Sighting {
                witness: witness(CraftId(90)),
                observed_s: now_s,
                bearing: Bearing { observer_ly: DVec3::ZERO, toward: DVec3::Y, sigma_rad: 1e-9 },
                size: None,
                range_m: None,
                spin_s: None,
                band: em_spectra::Band::V,
                flux: 1e-12,
                flux_sigma: 1e-15,
                lineage: Vec::new(),
            },
        );
        wire.client_says(
            bry,
            act(ShipId(90), Order::SendReport { to: Some(ship), aim: lc_proto::Aim::Omni, secrecy: lc_proto::Secrecy::Open, idem: 5 }),
        );

        let until = server.now_t() + MONTH_US;
        while server.now_t() < until {
            server.tick(&mut wire).await.unwrap();
        }
        wire.take(bry);

        let again = ClientId(3);
        let (back, mut told) = sign_in(&mut server, &mut wire, again, broker.mint("acct-1", SHARD, 60, "j2")).await;
        assert_eq!(back, ship, "the same craft");
        for _ in 0..5 {
            server.tick(&mut wire).await.unwrap();
            told.extend(wire.take(again));
        }
        let copy = replica(ship, &told);
        for star in &sky()[1..] {
            let belief = copy.belief(star.id).expect("found by the sweep while nobody watched");
            assert_eq!(belief.hops, 0, "and found by this craft, not told");
        }
        let heard = copy.belief(secret).expect("the report landed while nobody was signed in");
        assert_eq!(heard.hops, 1);
        assert_eq!(heard.bearing.observer_ly, DVec3::ZERO);
        assert!(heard.learned_s > now_s + 3_000.0, "learned when its light landed, an hour on: {} vs {now_s}", heard.learned_s);
        assert!(
            told.iter().any(|m| matches!(m, Outbound::Observing { duty: lc_proto::Duty::Sweep { .. }, .. })),
            "and it is told what its telescope is still doing",
        );
    }

    /// A client-supplied start time is replaced by the shard's.
    #[tokio::test]
    async fn a_duty_starts_when_the_shard_applies_it() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let sweep = lc_proto::Duty::Sweep { center: [0.0, 0.0, 1.0], radius_rad: 1.0, dwell_s: 60.0, started_s: -1.0e12 };
        wire.client_says(ClientId(1), act(ship, Order::SetDuty { duty: sweep, integration_s: 1.0e4 }));
        server.tick(&mut wire).await.unwrap();
        let started = wire.take(ClientId(1)).iter().find_map(|m| match m {
            Outbound::Accepted { order: Order::SetDuty { duty: lc_proto::Duty::Sweep { started_s, .. }, .. }, .. } => Some(*started_s),
            _ => None,
        });
        let started = started.expect("accepted");
        assert!(started > 0.0 && started <= server.now_t() as f64 * 1.0e-6 + TICK_US as f64, "{started}");
    }

    /// **A survey ordered through the shard finds the bodies of a generated system.** A new
    /// craft starts `START_OFFSET_AU` from the first star, which is where the duty is for, and
    /// the whole path runs: the order, `World::system_for` loading the system, `visit::sources`
    /// placing its bodies, and the sightings landing under `Subject::Body`.
    #[tokio::test]
    async fn a_survey_finds_the_bodies_of_the_system_the_craft_is_in() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let star = sky()[0].id;

        let duty = lc_proto::Duty::Survey { star: star.get(), started_s: -1.0e12 };
        wire.client_says(ClientId(1), act(ship, Order::SetDuty { duty, integration_s: 1.0e4 }));
        server.tick(&mut wire).await.unwrap();
        assert!(
            wire.take(ClientId(1)).iter().any(|m| matches!(
                m,
                Outbound::Accepted { order: Order::SetDuty { duty: lc_proto::Duty::Survey { .. }, .. }, .. }
            )),
            "the survey was not accepted"
        );

        let before = server.instruments.aboard[&CraftId(ship.0)].knowledge.len();
        for _ in 0..40 {
            server.tick(&mut wire).await.unwrap();
        }
        let knowledge = &server.instruments.aboard[&CraftId(ship.0)].knowledge;
        let bodies = knowledge.bodies_of(star, server.now_t() as f64 * 1.0e-6);
        assert!(
            bodies.len() > 5,
            "forty ticks of surveying found {} bodies; it held {before} subjects to begin with",
            bodies.len()
        );
        // Under the star whose system it is, and nothing under any other.
        for other in &sky()[1..] {
            assert!(
                knowledge.bodies_of(other.id, server.now_t() as f64 * 1.0e-6).is_empty(),
                "a body turned up under {:?}",
                other.id
            );
        }
    }

    /// **The fitting chain is alive on the shard, and refuses where it should.** A drifting
    /// craft surveying its own system triangulates its sun, which is what puts the bearings in
    /// a frame at all, and the fit is then offered bodies whose arcs are hours long. Hours is
    /// nothing of any orbit, so it declines them -- and declining is the behavior worth
    /// pinning here, since `knowledge::arc` covers the arcs that do settle.
    /// **A duty pointed at a star nobody has is refused.** `Duty::is_valid` cannot check an
    /// id -- `lc-proto` carries no catalog to check it against -- so a bogus one was taken
    /// up and then looked for on every tick forever, and a survey's lookup caches no miss to
    /// remember it by.
    #[tokio::test]
    async fn a_duty_naming_a_star_nobody_has_is_refused() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) =
            sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;

        let nowhere = StarId::synthesize("no-such-catalog", 7);
        for duty in [
            lc_proto::Duty::Survey { star: nowhere.get(), started_s: 0.0 },
            lc_proto::Duty::Stare { star: nowhere.get() },
        ] {
            let order = Order::SetDuty { duty: duty.clone(), integration_s: 1.0e4 };
            assert!(
                matches!(
                    server.act_on_knowledge(CraftId(ship.0), &order, 0.0),
                    Err(lc_proto::Refusal::Impossible)
                ),
                "{duty:?} was taken up although no such star exists"
            );
        }

        // And a star the catalog does hold is taken up as before.
        let real = sky()[0].id;
        let order = Order::SetDuty {
            duty: lc_proto::Duty::Survey { star: real.get(), started_s: 0.0 },
            integration_s: 1.0e4,
        };
        assert!(server.act_on_knowledge(CraftId(ship.0), &order, 0.0).is_ok());
    }

    #[tokio::test]
    async fn a_surveying_craft_measures_its_sun_and_declines_the_short_arcs() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let star = sky()[0].id;

        // Adrift rather than at rest: a ship that holds still measures no parallax, so its own
        // sun has no distance and nothing downstream of that runs at all. Across the line of
        // sight and not along it -- a craft starts on the +X axis from its star, and drifting
        // straight out along it sweeps no baseline at all.
        if let Some(craft) = server.fleet_mut().get_mut(CraftId(ship.0)) {
            craft.motion.beta = DVec3::new(0.0, 1.0e-6, 0.0);
        }
        let duty = lc_proto::Duty::Survey { star: star.get(), started_s: 0.0 };
        wire.client_says(ClientId(1), act(ship, Order::SetDuty { duty, integration_s: 1.0e4 }));
        for _ in 0..80 {
            server.tick(&mut wire).await.unwrap();
        }
        // Fits are solved off the tick; the claims below are about what they conclude.
        server.settle_fits();

        let knowledge = &server.instruments.aboard[&CraftId(ship.0)].knowledge;
        let host = knowledge
            .belief(lc_world::knowledge::Subject::Star(star))
            .expect("its own sun is surveyed every tick");
        assert!(host.triangulated, "a drifting ship should measure it: {:?}", host.distance);

        // Bodies enough to fit, and no orbit from any of them yet.
        let now_s = server.now_t() as f64 * 1.0e-6;
        let bodies = knowledge.bodies_of(star, now_s);
        assert!(bodies.len() > 5, "only {} bodies", bodies.len());
        // Ten hours is a fraction of any orbit here but the innermost, so that is the only
        // one that settles -- and from inside the system, with the host star's distance
        // measured, it settles correctly. The next planet out is a twenty-seventh of an orbit
        // and gets nothing. It was: a circle assumed where the arc could not shape a conic
        // fitted a moon at 0.0097 AU to 63 AU, which is what `knowledge::arc` now refuses.
        let truth: Vec<f64> = lc_world::sky::generate::planets_of(&sky()[0])
            .iter()
            .map(|p| std::f64::consts::TAU * (p.semi_major_m.powi(3) / sky()[0].star.mu).sqrt())
            .collect();
        for fitted in bodies.iter().filter(|b| b.method.is_some()) {
            let (period, _) = fitted.period_s.expect("a fitted orbit states its period");
            let miss = truth.iter().map(|t| (period / t - 1.0).abs()).fold(f64::INFINITY, f64::min);
            assert!(miss < 0.05, "a {:.0} hour orbit that is nothing in this system", period / 3600.0);
            assert!(
                period < 15.0 * now_s,
                "an orbit of {:.0} hours was minted from {:.0} hours of arc",
                period / 3600.0,
                now_s / 3600.0
            );
        }
        assert!(knowledge.unfitted(star).is_some(), "the fitter has work and is being offered it");
    }

    #[tokio::test]
    async fn a_craft_names_only_what_it_knows() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        // One star looked at and one not, since a craft is handed nothing on creation and the
        // distinction this pins is between what it has seen and what it has not.
        wire.client_says(
            ClientId(1),
            act(ship, Order::SetDuty {
                duty: lc_proto::Duty::Stare { star: sky()[0].id.get() },
                integration_s: 1.0,
            }),
        );
        for _ in 0..4 {
            server.tick(&mut wire).await.unwrap();
        }
        let _ = wire.take(ClientId(1));

        let home = lc_proto::Subject::Star(sky()[0].id.get());
        let far = lc_proto::Subject::Star(sky()[1].id.get());
        wire.client_says(ClientId(1), act(ship, Order::NameIt { subject: home, name: "Hearth".into() }));
        wire.client_says(ClientId(1), act(ship, Order::NameIt { subject: far, name: "Yonder".into() }));
        server.tick(&mut wire).await.unwrap();
        let said = wire.take(ClientId(1));
        assert!(said.iter().any(|m| matches!(m, Outbound::Accepted { order: Order::NameIt { .. }, .. })));
        assert!(said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::Impossible, .. })));
        server.tick(&mut wire).await.unwrap();
        let mut all = said;
        all.extend(wire.take(ClientId(1)));
        assert_eq!(replica(ship, &all).name_of(sky()[0].id).as_deref(), Some("Hearth"));
    }

    /// A belt has no file until it is named, so what may be named is a belt of the system the
    /// craft is in — and not one around another star, or one past the end of the list.
    #[tokio::test]
    async fn a_craft_names_the_belts_of_its_own_system() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        server.tick(&mut wire).await.unwrap();
        let _ = wire.take(ClientId(1));
        let belts = server.ship(ship).and_then(|c| c.system.as_ref()).map(|s| s.populations.len());
        assert!(belts.is_some_and(|n| n > 0), "premise: the craft starts in a system with a belt");

        let belt = |star: StarId, index| lc_proto::Subject::Population { star: star.get(), index };
        let named = |subject| act(ship, Order::NameIt { subject, name: "Shoals".into() });
        wire.client_says(ClientId(1), named(belt(sky()[0].id, 0)));
        wire.client_says(ClientId(1), named(belt(sky()[1].id, 0)));
        wire.client_says(ClientId(1), named(belt(sky()[0].id, belts.unwrap() as u32)));
        server.tick(&mut wire).await.unwrap();
        let said = wire.take(ClientId(1));
        let accepted = said.iter().filter(|m| matches!(m, Outbound::Accepted { order: Order::NameIt { .. }, .. })).count();
        let refused = said.iter().filter(|m| matches!(m, Outbound::Refused { reason: Refusal::Impossible, .. })).count();
        assert_eq!((accepted, refused), (1, 2), "{said:?}");
        server.tick(&mut wire).await.unwrap();
        let mut all = said;
        all.extend(wire.take(ClientId(1)));
        let at_home = Subject::Population { star: sky()[0].id, index: 0 };
        assert_eq!(replica(ship, &all).name_of(at_home).as_deref(), Some("Shoals"));
    }

    /// A craft that knows more than one frame holds is paged all of it, every page inside
    /// [`PAGE_BYTES`], until its copy matches the original.
    #[tokio::test]
    async fn a_craft_that_knows_more_than_a_frame_is_paged_all_of_it() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, mut said) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let id = CraftId(ship.0);
        let now_s = server.now_t() as f64 * 1.0e-6;
        let knowledge = &mut server.aboard(id).knowledge;
        for k in 0..40_000u64 {
            let toward = DVec3::new((k as f64).sin(), (k as f64).cos(), 0.1).normalize();
            let sighting = Sighting {
                witness: witness(id),
                // After the sign-in page, and all at one instant, as a sweep tick or charts are.
                observed_s: now_s + 1.0,
                bearing: Bearing { observer_ly: DVec3::ZERO, toward, sigma_rad: 1e-6 },
                size: None,
                range_m: None,
                spin_s: None,
                band: em_spectra::Band::V,
                flux: 1e-12,
                flux_sigma: 1e-15,
                lineage: Vec::new(),
            };
            knowledge.sighted(StarId::synthesize("paging", k), sighting);
        }
        let watched = StarId::synthesize("paging", 0);
        // Kept raw, so the shard does not consume it while paging.
        knowledge.retain_raw(watched, true);
        for n in 0..200_000u64 {
            let sample = lc_world::knowledge::Sample { observed_s: now_s + n as f64 * 1e-3, deficit: 1e-4, sigma: 1e-5 };
            knowledge.measured(watched, witness(id), em_spectra::Band::V, sample);
        }
        let whole = serde_json::to_string(&knowledge.report(Mark::default(), now_s + 2.0)).unwrap().len();
        assert!(whole > lc_proto::FRAME_LIMIT, "the test needs more than a frame of files: {whole}");

        for _ in 0..200 {
            server.tick(&mut wire).await.unwrap();
            said.extend(wire.take(ClientId(1)));
        }
        let mut copy = Knowledge::new(witness(id));
        let mut pages = 0;
        for message in &said {
            let body = match message {
                Outbound::Learned { report } => {
                    copy.absorb(&serde_json::from_str(report).unwrap());
                    report
                }
                Outbound::Logged { logs } => {
                    let page: lc_world::knowledge::Logs = serde_json::from_str(logs).unwrap();
                    copy.copy_logs(&page.logs);
                    for subject in page.retained {
                        copy.retain_raw(subject, true);
                    }
                    logs
                }
                _ => continue,
            };
            pages += 1;
            assert!(body.len() <= PAGE_BYTES, "a page of {} bytes", body.len());
        }
        assert!(pages > 16, "it took pages, not one message: {pages}");
        let original = server.knowledge_of(ship).unwrap();
        assert!(original.own_series(watched, em_spectra::Band::V).unwrap().len() > 40_000, "a log longer than a page");
        assert_eq!(copy.len(), original.len(), "every file arrived");
        assert_eq!(copy.own_series(watched, em_spectra::Band::V), original.own_series(watched, em_spectra::Band::V), "and the whole log");
        assert!(copy.retained(watched), "and that it is kept raw");
    }

    /// Every number in a duty is checked; a NaN radius would pass `clamp` and finish an all-sky
    /// sweep in a tick.
    #[tokio::test]
    async fn a_duty_with_a_number_no_telescope_could_take_is_refused() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let sweep = |center: [f64; 3], radius_rad: f64, dwell_s: f64| lc_proto::Duty::Sweep { center, radius_rad, dwell_s, started_s: 0.0 };
        let bad = [
            (sweep([0.0, 0.0, 1.0], f64::NAN, 60.0), 1.0e4),
            (sweep([0.0, 0.0, 0.0], 1.0, 60.0), 1.0e4),
            (sweep([f64::INFINITY, 0.0, 1.0], 1.0, 60.0), 1.0e4),
            (sweep([0.0, 0.0, 1.0], 1.0, 0.0), 1.0e4),
            (sweep([0.0, 0.0, 1.0], 1.0, f64::NAN), 1.0e4),
            (lc_proto::Duty::Watch { stars: vec![1; lc_proto::WATCH_LIMIT + 1], dwell_s: 60.0, started_s: 0.0 }, 1.0e4),
            (lc_proto::Duty::Watch { stars: Vec::new(), dwell_s: 60.0, started_s: 0.0 }, 1.0e4),
            (lc_proto::Duty::Idle, f64::NAN),
            (lc_proto::Duty::Idle, 1.0e12),
        ];
        for (duty, integration_s) in bad {
            wire.client_says(ClientId(1), act(ship, Order::SetDuty { duty: duty.clone(), integration_s }));
            server.tick(&mut wire).await.unwrap();
            let said = wire.take(ClientId(1));
            assert!(
                said.iter().any(|m| matches!(m, Outbound::Refused { reason: Refusal::Impossible, .. })),
                "{duty:?} at {integration_s} was not refused",
            );
            assert_eq!(server.duty_of(ship), Some(&Duty::Idle), "and the telescope did not take it up");
        }
    }

    /// A report in flight across a restart lands on time, because the journal holds its delivery.
    #[tokio::test]
    async fn a_report_in_flight_across_a_restart_still_lands() {
        let mut old = Server::new(Memory::default(), 0, 1);
        old.load_world(World::new(sky()));
        let mut wire = Loopback::new();
        let (near, far) = (ShipId(40), ShipId(41));
        old.admit(ClientId(1), crate::world::still(near, DVec3::ZERO), 0.0);
        old.admit(ClientId(2), crate::world::still(far, DVec3::X * 3_600.0 * 1.0e6), 0.0);
        let secret = StarId::synthesize("instruments", 77);
        let now_s = old.now_t() as f64 * 1.0e-6;
        old.aboard(CraftId(far.0)).knowledge.sighted(
            secret,
            Sighting {
                witness: witness(CraftId(far.0)),
                observed_s: now_s,
                bearing: Bearing { observer_ly: DVec3::X, toward: DVec3::Y, sigma_rad: 1e-9 },
                size: None,
                range_m: None,
                spin_s: None,
                band: em_spectra::Band::V,
                flux: 1e-12,
                flux_sigma: 1e-15,
                lineage: Vec::new(),
            },
        );
        let report = Order::SendReport { to: Some(near), aim: lc_proto::Aim::Omni, secrecy: lc_proto::Secrecy::Open, idem: 3 };
        wire.client_says(ClientId(2), act(far, report));
        old.tick(&mut wire).await.unwrap();
        assert!(!old.knowledge_of(near).is_some_and(|k| k.knows(secret)), "an hour from landing");

        let checkpoint = old.checkpoint();
        let mut new = Server::new(std::mem::take(&mut old.journal), 0, 1);
        new.load_world(World::new(sky()));
        assert!(new.adopt(checkpoint).is_empty());
        new.resume_conversations().await.unwrap();
        let until = new.now_t() + 2 * 3_600 * 1_000_000;
        while new.now_t() < until {
            new.tick(&mut wire).await.unwrap();
        }
        let belief = new.knowledge_of(near).and_then(|k| k.belief(secret).cloned()).expect("it landed after the restart");
        assert!(belief.learned_s > now_s + 3_000.0, "when its light got there: {}", belief.learned_s);
    }

    /// A tick with a hundred craft sweeping, each holding ten thousand files. A timing, not a
    /// check: `cargo test -p lc-server --lib a_busy_tick -- --ignored --nocapture`. The figure is
    /// in `lightcone/docs/24-standing-instruments.md`.
    #[tokio::test]
    #[ignore]
    async fn a_busy_tick_is_measured() {
        let template = AuthoredStars::sample().stars()[1].clone();
        let stars: Vec<CatalogStar> = (0..2_000u64)
            .map(|k| {
                let mut star = template.clone();
                star.id = StarId::synthesize("busy", k);
                let u = (k as f64 * 0.618_034).fract() * std::f64::consts::TAU;
                star.position_ly = DVec3::new(u.cos(), u.sin(), (k as f64 * 0.414_2).fract() - 0.5) * (5.0 + k as f64 * 0.05);
                star
            })
            .collect();
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(World::new(stars));
        let mut wire = Loopback::new();
        let sweep = lc_proto::Duty::Sweep { center: [0.0, 0.0, 1.0], radius_rad: std::f64::consts::PI, dwell_s: 60.0, started_s: 0.0 };
        for n in 0..100i64 {
            let ship = ShipId(1_000 + n);
            server.admit(ClientId(n as u64 + 1), crate::world::still(ship, DVec3::X * n as f64 * 1.0e6), 0.0);
            let now_s = server.now_t() as f64 * 1.0e-6;
            let knowledge = &mut server.aboard(CraftId(ship.0)).knowledge;
            for k in 0..10_000u64 {
                let toward = DVec3::new((k as f64).sin(), (k as f64).cos(), 0.2).normalize();
                knowledge.sighted(
                    StarId::synthesize("known", k),
                    Sighting {
                        witness: witness(CraftId(ship.0)),
                        observed_s: now_s,
                        bearing: Bearing { observer_ly: DVec3::ZERO, toward, sigma_rad: 1e-6 },
                        size: None,
                        range_m: None,
                        spin_s: None,
                        band: em_spectra::Band::V,
                        flux: 1e-12,
                        flux_sigma: 1e-15,
                        lineage: Vec::new(),
                    },
                );
            }
            wire.client_says(ClientId(n as u64 + 1), act(ship, Order::SetDuty { duty: sweep.clone(), integration_s: 1.0e4 }));
        }
        // Past the sign-in pages, so the timing is of craft at work, not catching up.
        for _ in 0..80 {
            server.tick(&mut wire).await.unwrap();
            for n in 0..100u64 {
                wire.take(ClientId(n + 1));
            }
        }
        let started = std::time::Instant::now();
        const TICKS: u32 = 20;
        for _ in 0..TICKS {
            server.tick(&mut wire).await.unwrap();
        }
        let per_tick = started.elapsed() / TICKS;
        eprintln!("a tick with 100 craft sweeping, 10 000 files each: {per_tick:?}");
    }

    /// Ten craft surveying one system for a coordinate week, drifting so they measure parallax
    /// and fit orbits. A timing, not a check: `cargo test -p lc-server --lib a_surveying_shard
    /// -- --ignored --nocapture`. See `lightcone/docs/plans/server-tick-lag.md`.
    #[tokio::test]
    #[ignore]
    async fn a_surveying_shard_is_measured() {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(World::new(sky()));
        let mut wire = Loopback::new();
        let star = sky()[0].id;
        let at = sky()[0].position_ly;
        let duty = lc_proto::Duty::Survey { star: star.get(), started_s: 0.0 };
        const CRAFT: i64 = 10;
        for n in 0..CRAFT {
            let ship = ShipId(1_000 + n);
            // A few AU out, spread round the star, each drifting across its own line of sight.
            let u = n as f64 / CRAFT as f64 * std::f64::consts::TAU;
            let offset = DVec3::new(u.cos(), u.sin(), 0.0) * 3.0 * 1.58e-5;
            server.admit(ClientId(n as u64 + 1), crate::world::still(ship, at + offset), 0.0);
            if let Some(craft) = server.fleet_mut().get_mut(CraftId(ship.0)) {
                craft.motion.beta = DVec3::new(-u.sin(), u.cos(), 0.0) * 1.0e-6;
            }
            wire.client_says(ClientId(n as u64 + 1), act(ship, Order::SetDuty { duty: duty.clone(), integration_s: 1.0e4 }));
        }
        const TICKS: u32 = 1_400;
        let mut total = std::time::Duration::ZERO;
        let mut worst: Option<crate::timing::Stages> = None;
        let mut slow = 0;
        let mut checkpointed: Option<(std::time::Duration, usize, usize)> = None;
        for _ in 0..TICKS {
            server.tick(&mut wire).await.unwrap();
            for n in 0..CRAFT as u64 {
                wire.take(ClientId(n + 1));
            }
            let tick = server.last_tick();
            total += tick.total();
            slow += usize::from(tick.total() > std::time::Duration::from_millis(crate::server::TICK_MS as u64));
            if worst.as_ref().is_none_or(|w| tick.total() > w.total()) {
                worst = Some(tick.clone());
            }
            // The part of a checkpoint the tick waits for: the snapshot and the encoded files.
            if server.now_t() % (400 * TICK_US) == 0 {
                let started = std::time::Instant::now();
                let taken = server.checkpoint();
                let remembered = server.take_knowledge();
                let took = started.elapsed();
                let files = remembered.files.len();
                let bytes: usize = remembered.files.iter().map(|f| f.file.len()).sum::<usize>() + taken.ships.len();
                if checkpointed.is_none_or(|(worst, _, _)| took > worst) {
                    checkpointed = Some((took, files, bytes));
                }
            }
        }
        eprintln!(
            "{CRAFT} craft surveying for {TICKS} ticks: mean {:?}, {slow} over budget, worst {}",
            total / TICKS,
            worst.map_or_else(String::new, |w| w.to_string()),
        );
        if let Some((took, files, bytes)) = checkpointed {
            eprintln!("worst checkpoint snapshot {took:?}: {files} files, {bytes} bytes");
        }
    }
}
