//! Instruments that run whether or not anybody is flying them, and the knowledge they fill.
//!
//! Every craft's knowledge lives here, not in its client. A duty is advanced every tick for
//! every craft that has one, a report landing on a craft is folded in when its light arrives
//! whether or not anybody is signed in to it, and a connected client is sent what its craft
//! learned — a report from itself, which it folds into its copy. See
//! `lightcone/docs/24-standing-instruments.md`.

use std::collections::HashMap;

use lc_proto::{Order, Outbound, Refusal, ShipId};
use lc_world::craft::CraftId;
use lc_world::fitting::ONBOARD_DATA_BYTES;
use lc_world::knowledge::observatory::{self, CHARTED_LY, Observatory, Sky, Station};
use lc_world::knowledge::survey::Duty;
use lc_world::knowledge::prior::Prior;
use lc_world::knowledge::{ENTRIES_PER_REPORT, Knowledge, Mark, Report, Reporting, Subject, Witness};
use lc_world::motion::LIGHT_US_PER_LY;

use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;
use crate::world::{Event, Scheduled};

/// Systems per page of a craft's knowledge sent to a client that has just signed in. A
/// thoroughly surveyed sky is thousands, so it arrives over a few ticks rather than as one
/// message the size of the catalogue.
const PAGE: usize = 256;

/// Samples per page of a craft's own logs, before the byte bound shrinks it.
const LOG_PAGE: usize = 20_000;

/// Bytes a page may take, whichever stream. Far inside [`lc_proto::FRAME_LIMIT`]: a page over
/// that is one the client refuses, and it would reconnect and be sent the same page again.
pub(crate) const PAGE_BYTES: usize = 1 << 20;

/// Logs read per tick across the whole shard. Reading one is a search over thousands of
/// periods, so the shard takes them in turn rather than all at once.
const READS_PER_TICK: usize = 1;

/// What one craft holds and is doing with its instruments.
#[derive(Clone, Debug)]
pub(crate) struct Aboard {
    pub knowledge: Knowledge,
    pub observatory: Observatory,
    pub reporting: Reporting,
}

/// A report on its way to a craft, and when its light lands.
#[derive(Clone, Debug)]
pub(crate) struct Landing {
    observer: CraftId,
    arrive_t: i64,
    strength: f32,
    body: String,
}

/// Everything the server holds about what craft know.
#[derive(Default)]
pub(crate) struct Instruments {
    pub aboard: HashMap<CraftId, Aboard>,
    /// Built from the world's stars the first time anything looks at them, and shared by every
    /// craft: a star's output and its emission model do not depend on who is looking.
    sky: Option<Sky>,
    /// What the generator's planets look like across the world's stars, which is what a log is
    /// read against. Measured once, like the sky.
    prior: Option<Prior>,
    /// The craft whose log was read last, so the next read goes to the one after it.
    reader: Option<CraftId>,
    /// The craft whose room was last recounted from scratch, likewise.
    recounted: Option<CraftId>,
    landings: Vec<Landing>,
}

impl Instruments {
    pub(crate) fn forget_sky(&mut self) {
        self.sky = None;
        self.prior = None;
    }
}

pub(crate) fn witness(id: CraftId) -> Witness {
    Witness(id.0 as u64)
}

/// The largest page `build` makes that fits [`PAGE_BYTES`], halving `limit` until it does, and
/// what to resume from. `None` when there is nothing new.
///
/// A single item larger than a page is sent alone as long as it fits a frame. One that does not
/// is skipped — the body is `None` and the mark still moves past it — because sending it would
/// close the connection, and so would every reconnection after.
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
    fn sky(&mut self) -> &mut Sky {
        let stars = self.world.stars();
        self.instruments.sky.get_or_insert_with(|| Sky::new(stars))
    }

    /// Where a craft's instruments are and what they are, now.
    fn station(&self, id: CraftId) -> Option<Station> {
        let craft = self.fleet.get(id)?;
        let position_ly = craft.position_at(self.now_t as f64) / LIGHT_US_PER_LY;
        Some(Station { position_ly, instrument: craft.sensor })
    }

    /// A craft's knowledge, starting it with the charts of where it is if it has none yet.
    ///
    /// A new craft is not issued the sky. It is issued the charts of the volume it is in — the
    /// charting office's word, overridden the moment it measures anything itself — and has to
    /// find the rest. See `lightcone/docs/22-provenance.md`.
    pub(crate) fn aboard(&mut self, id: CraftId) -> &mut Aboard {
        let fresh = (!self.instruments.aboard.contains_key(&id)).then(|| {
            let mut knowledge = Knowledge::new(witness(id));
            if let Some(at) = self.station(id) {
                let now_s = self.now_t as f64 * 1.0e-6;
                observatory::issue_charts(self.sky(), &mut knowledge, at, CHARTED_LY, now_s);
            }
            Aboard { knowledge, observatory: Observatory::default(), reporting: Reporting::default() }
        });
        self.instruments.aboard.entry(id).or_insert_with(|| fresh.unwrap_or_else(|| Aboard {
            knowledge: Knowledge::new(witness(id)),
            observatory: Observatory::default(),
            reporting: Reporting::default(),
        }))
    }

    /// Room for knowledge aboard a craft now: its data modules and the onboard store.
    fn data_capacity(&self, id: CraftId) -> f64 {
        let now_s = self.now_t as f64 * 1.0e-6;
        self.fleet
            .get(id)
            .and_then(|c| c.fitting())
            .map_or(ONBOARD_DATA_BYTES, |f| f.balance.data_capacity(&f.loadout_at(now_s)))
    }

    /// Recount what a craft's knowledge takes if its room changed, or regardless.
    ///
    /// Samples keep the count as they arrive. Everything else — a sighting, a name, a log read
    /// and thrown away — is noticed at a recount, which costs a pass over every file and so is
    /// done when something changed and otherwise one craft a tick.
    fn fit(&mut self, id: CraftId, recount: bool) {
        let capacity = self.data_capacity(id);
        if let Some(aboard) = self.instruments.aboard.get_mut(&id)
            && (recount || aboard.knowledge.capacity_bytes() != capacity)
        {
            aboard.knowledge.fit_to(capacity);
        }
    }

    /// Advance every craft's duty to now.
    pub(crate) fn run_instruments(&mut self) {
        let now_s = self.now_t as f64 * 1.0e-6;
        let busy: Vec<CraftId> = self
            .instruments
            .aboard
            .iter()
            .filter(|(_, a)| a.observatory.duty != Duty::Idle)
            .map(|(id, _)| *id)
            .collect();
        let mut ids: Vec<CraftId> = self.instruments.aboard.keys().copied().collect();
        ids.sort_unstable_by_key(|id| id.0);
        let next = self.instruments.recounted.map_or(0, |r| ids.partition_point(|id| id.0 <= r.0));
        if let Some(&id) = ids.get(next).or(ids.first()) {
            self.fit(id, true);
            self.instruments.recounted = Some(id);
        }
        for id in busy {
            self.fit(id, false);
            let Some(at) = self.station(id) else { continue };
            let stars = self.world.stars();
            let instruments = &mut self.instruments;
            let sky = instruments.sky.get_or_insert_with(|| Sky::new(stars));
            let Some(aboard) = instruments.aboard.get_mut(&id) else { continue };
            aboard.observatory.tick(sky, &mut aboard.knowledge, at, now_s);
        }
        self.read_logs(now_s);
    }

    /// Read the logs that are due, a few a tick, taking the craft in turn.
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
            let Some((subject, observer)) =
                self.instruments.aboard.get(&id).and_then(|a| a.knowledge.due().first().copied())
            else {
                continue;
            };
            let stars = self.world.stars();
            let instruments = &mut self.instruments;
            let prior = instruments.prior.get_or_insert_with(|| Prior::measure(stars.iter()));
            let Some(aboard) = instruments.aboard.get_mut(&id) else { continue };
            aboard.knowledge.read_log(subject, observer, prior, now_s);
            instruments.reader = Some(id);
            reads += 1;
            self.fit(id, true);
        }
    }

    /// Note where this tick's reports will land, so they are folded in when their light does.
    ///
    /// Read off the deliveries rather than recomputed, because the deliveries already are the
    /// answer: who the beam covered, when the light gets there, and how loud it is.
    pub(crate) fn schedule_landings(&mut self, events: &[Event], deliveries: &[Scheduled]) {
        for event in events.iter().filter(|e| e.kind == lc_proto::kind::REPORT) {
            for scheduled in deliveries.iter().filter(|d| d.event == event.id) {
                // Redacted here exactly as a client is sent it, so a sealed report teaches
                // nobody but its addressee, whoever holds the craft it lands on.
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

    /// Fold in every report whose light has arrived, for every craft, signed in or not.
    pub(crate) fn land_reports(&mut self) {
        let now = self.now_t;
        let (due, pending): (Vec<Landing>, Vec<Landing>) =
            std::mem::take(&mut self.instruments.landings).into_iter().partition(|l| l.arrive_t <= now);
        self.instruments.landings = pending;
        for landing in due {
            // Under the noise floor it is light that went past, not a report received.
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

    /// Send every connected client what its craft has learned since the last time, and its own
    /// samples, a page of each at most. Paging carries on over the ticks that follow until the
    /// copy is caught up.
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
                (serde_json::to_string(&report).ok(), through)
            });
            let retained = knowledge.retained_subjects();
            let logs = page(LOG_PAGE, |limit| {
                let (logs, through) = knowledge.logs_upto(logged, limit);
                let page = lc_world::knowledge::Logs { logs, retained: retained.clone() };
                (serde_json::to_string(&page).ok(), through)
            });
            // A connection is told what its craft keeps raw once, logs or none; after that each
            // log page says so again, and an accepted order says when it changes.
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

    /// Tell a client what its craft's telescope is doing.
    pub(crate) fn tell_observing(&mut self, wire: &mut impl Transport, client: lc_proto::ClientId, id: CraftId) {
        let observatory = &self.aboard(id).observatory;
        let observing = Outbound::Observing {
            duty: (&observatory.duty).into(),
            integration_s: observatory.integration_s,
        };
        wire.send(client, observing);
    }

    /// The orders that change what a craft knows or is doing with its instruments, rather than
    /// where it is. Nothing is put on the air, so they are not events; the order comes back as
    /// applied, with a sweep's or a watch's start set to when it actually began.
    pub(crate) fn act_on_knowledge(&mut self, id: CraftId, order: &Order, at_s: f64) -> Result<Order, Refusal> {
        match order {
            Order::SetDuty { duty, integration_s } => {
                let integration_ok = (0.0..=lc_proto::INTEGRATION_MAX_S).contains(integration_s);
                if !integration_ok || !duty.is_valid() {
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
                let knowledge = &mut self.aboard(id).knowledge;
                // A name for something this craft has never heard of is a name for nothing.
                if !knowledge.knows(subject) {
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

    /// The report a craft would send `to` now, and what it would advance the mark to.
    ///
    /// Written here, from the knowledge the shard holds, never taken from the client. Shrunk
    /// until it fits a transmission: the slice is bounded by systems and a system's file is
    /// not, so a long watch on one star can make even a few of them too large.
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
    use lc_world::sky::{AuthoredStars, CatalogueStar, StarId, StarProvider};

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
    /// out along the axes that do not look back through the home star's glare. The charts reach
    /// twenty, so the three are what a sweep has to find for itself.
    fn sky() -> Vec<CatalogueStar> {
        let template = AuthoredStars::sample().stars()[1].clone();
        [DVec3::ZERO, DVec3::X * 30.0, DVec3::Y * 30.0, DVec3::Z * 30.0]
            .into_iter()
            .enumerate()
            .map(|(k, at)| {
                let mut star = template.clone();
                star.id = StarId::synthesise("instruments", k as u64);
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

    /// Sign in, and everything said on the tick that welcomed it — which already includes the
    /// first page of what its craft knows.
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

    /// Everything a client was told its craft knows, folded as the client folds it.
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

    /// A new craft is issued the charts of where it starts, and its client is sent them.
    #[tokio::test]
    async fn a_new_craft_starts_with_the_charts_of_where_it_is() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, said) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
        let copy = replica(ship, &said);
        let home = sky()[0].id;
        assert!(copy.knows(home), "the home star is charted");
        assert_eq!(copy.stars().count(), 1, "and nothing past twenty light-years is");
    }

    /// The whole of 11b. A craft set to sweep keeps sweeping with nobody signed in to it; a
    /// report sent to it meanwhile is folded in when its light lands; and signing back in hands
    /// the client all of it.
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

        // A second craft a light-hour out on the far side of the home star from where new craft
        // start, which knows one thing the first does not.
        let bry = ClientId(2);
        let at = -DVec3::X * 3_600.0 * 1.0e6;
        server.admit(bry, crate::world::still(ShipId(90), at), 0.0);
        let secret = StarId::synthesise("instruments", 99);
        let now_s = server.now_t() as f64 * 1.0e-6;
        server.aboard(CraftId(90)).knowledge.sighted(
            secret,
            Sighting {
                witness: witness(CraftId(90)),
                observed_s: now_s,
                bearing: Bearing { observer_ly: DVec3::ZERO, toward: DVec3::Y, sigma_rad: 1e-9 },
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

    /// A duty, like a name, is applied by the shard and comes back as applied.
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

    #[tokio::test]
    async fn a_craft_names_only_what_it_knows() {
        let broker = Broker::new([1u8; 32]);
        let mut server = server(&broker);
        let mut wire = Loopback::new();
        let (ship, _) = sign_in(&mut server, &mut wire, ClientId(1), broker.mint("acct-1", SHARD, 60, "j1")).await;
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

    /// The whole of review item 7. A craft that knows more than one frame can carry — a sky of
    /// files and a long log — signs in, and is paged all of it over the ticks that follow, every
    /// page inside the bound, until its copy is the original.
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
                // After the page the sign-in already sent, and all at one instant, as a sweep's
                // tick or a set of charts is.
                observed_s: now_s + 1.0,
                bearing: Bearing { observer_ly: DVec3::ZERO, toward, sigma_rad: 1e-6 },
                band: em_spectra::Band::V,
                flux: 1e-12,
                flux_sigma: 1e-15,
                lineage: Vec::new(),
            };
            knowledge.sighted(StarId::synthesise("paging", k), sighting);
        }
        let watched = StarId::synthesise("paging", 0);
        // Kept raw, so the shard does not read it into a conclusion while it is being paged.
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

    /// Review item 10. Every number in a duty is checked: a NaN radius, which `clamp` would
    /// pass through and which would finish an all-sky sweep in a tick, is refused, and so is a
    /// zero direction, a dwell out of range and a watch longer than the limit.
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

    /// Review item 8. A report a light-hour out when its shard stops still lands, at the time
    /// its light gets there, on the shard that comes back: the journal still holds its delivery.
    #[tokio::test]
    async fn a_report_in_flight_across_a_restart_still_lands() {
        let mut old = Server::new(Memory::default(), 0, 1);
        old.load_world(World::new(sky()));
        let mut wire = Loopback::new();
        let (near, far) = (ShipId(40), ShipId(41));
        old.admit(ClientId(1), crate::world::still(near, DVec3::ZERO), 0.0);
        old.admit(ClientId(2), crate::world::still(far, DVec3::X * 3_600.0 * 1.0e6), 0.0);
        let secret = StarId::synthesise("instruments", 77);
        let now_s = old.now_t() as f64 * 1.0e-6;
        old.aboard(CraftId(far.0)).knowledge.sighted(
            secret,
            Sighting {
                witness: witness(CraftId(far.0)),
                observed_s: now_s,
                bearing: Bearing { observer_ly: DVec3::X, toward: DVec3::Y, sigma_rad: 1e-9 },
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

    /// Review item 15's measurement: a tick with a hundred craft sweeping, each holding ten
    /// thousand files. Ignored because it is a timing, not a check; run it with
    /// `cargo test -p lc-server --lib a_busy_tick -- --ignored --nocapture`, and the figure is
    /// in doc 24.
    #[tokio::test]
    #[ignore]
    async fn a_busy_tick_is_measured() {
        let template = AuthoredStars::sample().stars()[1].clone();
        let stars: Vec<CatalogueStar> = (0..2_000u64)
            .map(|k| {
                let mut star = template.clone();
                star.id = StarId::synthesise("busy", k);
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
                    StarId::synthesise("known", k),
                    Sighting {
                        witness: witness(CraftId(ship.0)),
                        observed_s: now_s,
                        bearing: Bearing { observer_ly: DVec3::ZERO, toward, sigma_rad: 1e-6 },
                        band: em_spectra::Band::V,
                        flux: 1e-12,
                        flux_sigma: 1e-15,
                        lineage: Vec::new(),
                    },
                );
            }
            wire.client_says(ClientId(n as u64 + 1), act(ship, Order::SetDuty { duty: sweep.clone(), integration_s: 1.0e4 }));
        }
        // Past the sign-in pages, so what is timed is a craft at work rather than one catching up.
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
}
