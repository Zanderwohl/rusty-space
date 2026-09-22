//! What craft know, as it is written down and read back.
//!
//! The same split as [`crate::persist`]: everything here is pure — what to write and how to
//! read it — and where it goes is [`lc_store::knowledge`], joined only in the binary. A craft's
//! knowledge is its files, rewritten when they change, and its photometric log, appended; see
//! `lightcone/docs/24-standing-instruments.md`.
//!
//! Postcard for the same reason as a craft's checkpoint: a bearing that moved one place in the
//! last digit every restart would be a parallax that drifted.

use std::collections::HashMap;

use em_spectra::Band;
use lc_store::knowledge::{Discarded, Filed, LogRow};
use lc_world::craft::CraftId;
use lc_world::knowledge::{Consumed, File, Knowledge, Logged, Sample, Subject, Witness};

use crate::instruments::witness;
use crate::journal::Journal;
use crate::server::Server;

/// What writes a file's bytes today. Older formats are read, not refused: see
/// [`lc_world::knowledge::formats`], where the shapes and the rule for bumping live.
pub const KNOWLEDGE_FORMAT: i32 = lc_world::knowledge::formats::FILE_FORMAT;

/// One craft's files and log, read back.
type Written = (Vec<(Subject, File)>, Vec<Logged>);

/// Everything written since the last time, for the store.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Remembered {
    pub files: Vec<Filed>,
    pub samples: Vec<LogRow>,
    /// Samples read and thrown away, to delete. After `samples`: a sample taken and consumed
    /// between two checkpoints is in neither.
    pub discarded: Vec<Discarded>,
    /// The same, as each craft's knowledge handed it over, for [`Server::untake_knowledge`] to
    /// put back if the write fails.
    drained: Vec<Drained>,
}

#[derive(Clone, Debug, PartialEq)]
struct Drained {
    craft: CraftId,
    files: Vec<Subject>,
    logs: Vec<Logged>,
    consumed: Vec<Consumed>,
}

pub fn file_row(ship: CraftId, subject: Subject, file: &File, saved_t: i64) -> Filed {
    Filed {
        ship_id: ship.0,
        subject: lc_proto::encode(&subject),
        format: KNOWLEDGE_FORMAT,
        file: lc_proto::encode(file),
        saved_t,
    }
}

pub fn discarded_row(ship: CraftId, consumed: &Consumed) -> Discarded {
    Discarded {
        ship_id: ship.0,
        subject: lc_proto::encode(&consumed.subject),
        witness: consumed.witness.0 as i64,
        band: consumed.band.index() as i16,
        through_s: consumed.through_s,
    }
}

pub fn log_row(ship: CraftId, logged: &Logged) -> LogRow {
    LogRow {
        ship_id: ship.0,
        subject: lc_proto::encode(&logged.subject),
        // The bit pattern: the charting office is u64::MAX.
        witness: logged.witness.0 as i64,
        band: logged.band.index() as i16,
        observed_s: logged.sample.observed_s,
        learned_t: (logged.learned_s * 1.0e6).round() as i64,
        deficit: logged.sample.deficit,
        sigma: logged.sample.sigma,
    }
}

/// A file row back into a subject and a file, or why not.
pub fn read_file(row: &Filed) -> Result<(Subject, File), String> {
    let subject = lc_proto::decode(&row.subject).map_err(|why| why.to_string())?;
    let file = lc_world::knowledge::formats::decode(row.format, &row.file)?;
    Ok((subject, file))
}

pub fn read_log(row: &LogRow) -> Result<Logged, String> {
    let band = *Band::ALL.get(row.band as usize).ok_or_else(|| format!("no band {}", row.band))?;
    Ok(Logged {
        subject: lc_proto::decode(&row.subject).map_err(|why| why.to_string())?,
        witness: Witness(row.witness as u64),
        band,
        sample: Sample { observed_s: row.observed_s, deficit: row.deficit, sigma: row.sigma },
        learned_s: row.learned_t as f64 * 1.0e-6,
    })
}

impl<J: Journal> Server<J> {
    /// What a craft knows, as the shard holds it. For the console, and for tests that restart
    /// a shard and need to compare before with after.
    pub fn knowledge_of(&self, ship: lc_proto::ShipId) -> Option<&Knowledge> {
        self.instruments.aboard.get(&CraftId(ship.0)).map(|a| &a.knowledge)
    }

    /// What a craft's telescope is doing, as the shard holds it.
    pub fn duty_of(&self, ship: lc_proto::ShipId) -> Option<&lc_world::knowledge::survey::Duty> {
        self.instruments.aboard.get(&CraftId(ship.0)).map(|a| &a.observatory.duty)
    }

    /// Everything every craft has learned since this was last called, ready to write.
    pub fn take_knowledge(&mut self) -> Remembered {
        let saved_t = self.now_t;
        let mut remembered = Remembered::default();
        for (id, aboard) in &mut self.instruments.aboard {
            let (files, logs) = aboard.knowledge.take_changes();
            remembered.files.extend(files.iter().map(|(subject, file)| file_row(*id, *subject, file, saved_t)));
            remembered.samples.extend(logs.iter().map(|logged| log_row(*id, logged)));
            let consumed = aboard.knowledge.take_consumed();
            remembered.discarded.extend(consumed.iter().map(|c| discarded_row(*id, c)));
            if !(files.is_empty() && logs.is_empty() && consumed.is_empty()) {
                let files = files.into_iter().map(|(subject, _)| subject).collect();
                remembered.drained.push(Drained { craft: *id, files, logs, consumed });
            }
        }
        remembered
    }

    /// Hand back what [`Server::take_knowledge`] took, because writing it failed. The next
    /// checkpoint writes it again, with whatever changed in between.
    pub fn untake_knowledge(&mut self, remembered: Remembered) {
        for drained in remembered.drained {
            if let Some(aboard) = self.instruments.aboard.get_mut(&drained.craft) {
                aboard.knowledge.untake(drained.files, drained.logs, drained.consumed);
            }
        }
    }

    /// Put back what craft knew. After [`Server::adopt`], which is what put the craft and their
    /// instruments back; a craft whose knowledge was written down is not issued charts again.
    ///
    /// A row that will not read is reported and skipped, not fatal: losing one file is losing
    /// what one craft knew about one star, which is recoverable by looking again.
    pub fn adopt_knowledge(&mut self, files: &[Filed], samples: &[LogRow]) -> Vec<String> {
        let mut problems = Vec::new();
        let mut by_craft: HashMap<CraftId, Written> = HashMap::new();
        for row in files {
            match read_file(row) {
                Ok(file) => by_craft.entry(CraftId(row.ship_id)).or_default().0.push(file),
                Err(why) => problems.push(format!("craft {}: {why}", row.ship_id)),
            }
        }
        for row in samples {
            match read_log(row) {
                Ok(logged) => by_craft.entry(CraftId(row.ship_id)).or_default().1.push(logged),
                Err(why) => problems.push(format!("craft {}: {why}", row.ship_id)),
            }
        }
        for (id, (files, logs)) in by_craft {
            // The store holds every craft ever saved; this shard only has the ones it adopted.
            if self.fleet.get(id).is_none() {
                continue;
            }
            let knowledge = Knowledge::restore(witness(id), files, logs);
            match self.instruments.aboard.get_mut(&id) {
                Some(aboard) => aboard.knowledge = knowledge,
                None => {
                    self.instruments.aboard.insert(id, crate::instruments::Aboard {
                        knowledge,
                        observatory: Default::default(),
                        reporting: Default::default(),
                    });
                }
            }
        }
        // A craft that came back with no knowledge written down — rows lost, or never checkpointed
        // since it launched — is issued its charts as a new craft is, rather than left knowing
        // nothing at all.
        let empty: Vec<CraftId> =
            self.instruments.aboard.iter().filter(|(_, a)| a.knowledge.is_empty()).map(|(id, _)| *id).collect();
        for id in empty {
            let Some(restored) = self.instruments.aboard.remove(&id) else { continue };
            // A fresh one is issued charts; the duty and the reporting marks are the checkpoint's.
            let aboard = self.aboard(id);
            aboard.observatory = restored.observatory;
            aboard.reporting = restored.reporting;
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use glam::DVec3;
    use lc_proto::{ClientId, Inbound, Intent, Order, ShipId};
    use lc_world::sky::{AuthoredStars, CatalogueStar, StarId, StarProvider};

    use super::*;
    use crate::journal::Memory;
    use crate::transport::Loopback;
    use crate::world::World;

    /// A home star, and three more thirty light-years out that the charts do not reach and a
    /// sweep has to find — see `crate::instruments`' tests for why these directions.
    fn sky() -> Vec<CatalogueStar> {
        let template = AuthoredStars::sample().stars()[1].clone();
        [DVec3::ZERO, DVec3::X * 30.0, DVec3::Y * 30.0, DVec3::Z * 30.0]
            .into_iter()
            .enumerate()
            .map(|(k, at)| {
                let mut star = template.clone();
                star.id = StarId::synthesise("archive", k as u64);
                star.position_ly = at;
                star
            })
            .collect()
    }

    fn a_shard() -> Server<Memory> {
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(World::new(sky()));
        server
    }

    const SHIP: ShipId = ShipId(3);

    /// A shard with one craft sweeping, a stare's worth of photometry in its log, and part of a
    /// pass done.
    async fn running() -> (Server<Memory>, Loopback) {
        let mut server = a_shard();
        let mut wire = Loopback::new();
        let at = server.world.start().unwrap() * lc_world::motion::LIGHT_US_PER_LY;
        server.admit(ClientId(1), crate::world::still(SHIP, at), 0.0);
        let order = |duty| Inbound::Act(Intent { ship_id: SHIP, order: Order::SetDuty { duty, integration_s: 1.0e4 }, issued_at_client_t: i64::MAX });
        // A stare on the home star first, so there is a log to keep.
        wire.client_says(ClientId(1), order(lc_proto::Duty::Stare { star: sky()[0].id.get() }));
        for _ in 0..200 {
            server.tick(&mut wire).await.unwrap();
        }
        let sweep = lc_proto::Duty::Sweep { center: [0.0, 0.0, 1.0], radius_rad: std::f64::consts::PI, dwell_s: 60.0, started_s: 0.0 };
        wire.client_says(ClientId(1), order(sweep));
        // About a third of a pass.
        for _ in 0..500 {
            server.tick(&mut wire).await.unwrap();
        }
        wire.take(ClientId(1));
        (server, wire)
    }

    /// Everything the binary does, without the database: checkpoint, write down what was
    /// learned, and bring both back in a new shard.
    fn restart(old: &mut Server<Memory>) -> Server<Memory> {
        let checkpoint = old.checkpoint();
        let remembered = old.take_knowledge();
        let mut new = a_shard();
        assert!(new.adopt(checkpoint).is_empty(), "every craft reads back");
        assert!(new.adopt_knowledge(&remembered.files, &remembered.samples).is_empty());
        new
    }

    /// The whole of 11c. A shard restarted mid-sweep comes back with the same map for every
    /// craft and the same sweep under way, and the sweep carries on finding what it had not yet.
    #[tokio::test]
    async fn a_shard_restarted_mid_sweep_resumes_it_with_the_same_map() {
        let (mut old, _) = running().await;
        let before = old.knowledge_of(SHIP).unwrap().clone();
        assert!(before.own_series(sky()[0].id, Band::V).is_some_and(|s| s.len() > 3), "a log to keep");
        let found_before = before.stars().count();
        assert!(found_before < sky().len(), "part of a pass: {found_before} of {}", sky().len());

        let mut new = restart(&mut old);
        assert_eq!(new.knowledge_of(SHIP), Some(&before), "the map after is the map before");
        assert_eq!(new.duty_of(SHIP), old.duty_of(SHIP), "and the sweep, with its start");

        let mut wire = Loopback::new();
        for _ in 0..3000 {
            new.tick(&mut wire).await.unwrap();
        }
        let after = new.knowledge_of(SHIP).unwrap();
        assert_eq!(after.stars().count(), sky().len(), "the sweep carried on and finished the pass");
        let home = sky()[0].id;
        let charts = after.file(home).unwrap().claims().len();
        assert_eq!(charts, 1, "a restored craft is not issued its charts a second time");
    }

    /// A second checkpoint writes only what changed since the first.
    #[tokio::test]
    async fn a_checkpoint_writes_only_what_changed() {
        let (mut server, mut wire) = running().await;
        let first = server.take_knowledge();
        assert!(!first.files.is_empty() && !first.samples.is_empty());
        let nothing = server.take_knowledge();
        assert!(nothing.files.is_empty() && nothing.samples.is_empty(), "nothing new");
        for _ in 0..10 {
            server.tick(&mut wire).await.unwrap();
        }
        let next = server.take_knowledge();
        assert!(next.files.len() < first.files.len(), "{} files, then {}", first.files.len(), next.files.len());
    }

    /// A red dwarf five light-years out whose innermost generated planet goes round in under five
    /// days, and the periods of all its planets, placed where they transit as seen from the origin.
    fn red_dwarf() -> (CatalogueStar, Vec<f64>) {
        let template = AuthoredStars::sample().stars()[0].clone();
        let luminosity: f64 = 0.01;
        let teff = 5772.0 * luminosity.powf(0.13);
        let mass = em_spectra::stellar::main_sequence_mass_solar(luminosity);
        (100_000..)
            .find_map(|key| {
                let mut star = template.clone();
                star.id = StarId::synthesise("archive-planet", key);
                // Edge-on to the origin, where the system's shared plane puts every planet across
                // its star.
                star.position_ly = lc_world::sky::generate::pole_for(star.seed()).any_orthonormal_vector() * 5.0;
                star.luminosity_solar = luminosity;
                star.mass_solar = mass;
                star.metallicity = 0.0;
                star.star.teff_k = teff;
                star.star.radius_m = em_spectra::stellar::radius_from_luminosity(
                    luminosity * em_spectra::stellar::SOLAR_LUMINOSITY,
                    teff,
                );
                star.star.mu = em_spectra::stellar::mu_from_mass_solar(mass);
                let periods: Vec<f64> = lc_world::sky::generate::ladder(star.seed(), luminosity, 0.0)
                    .iter()
                    .map(|r| std::f64::consts::TAU * (r.semi_major_m.powi(3) / star.star.mu).sqrt())
                    .collect();
                (*periods.first()? < 5.0 * 86_400.0).then_some((star, periods))
            })
            .unwrap()
    }

    /// The whole of 11d, on the shard. A craft left staring at a star the generator gave a close
    /// planet comes to believe in the planet, with its period, and the samples that told it are
    /// gone — from what the craft holds and from the store.
    #[tokio::test]
    async fn a_watched_planet_is_concluded_and_its_samples_deleted() {
        use lc_world::knowledge::conclusion::{Kind, SETTLED};

        let (target, periods) = red_dwarf();
        let mut server = Server::new(Memory::default(), 0, 1);
        server.load_world(World::new(vec![target.clone()]));
        let mut wire = Loopback::new();
        server.admit(ClientId(1), crate::world::still(SHIP, DVec3::ZERO), 0.0);
        let stare = Order::SetDuty { duty: lc_proto::Duty::Stare { star: target.id.get() }, integration_s: 1800.0 };
        wire.client_says(ClientId(1), Inbound::Act(Intent { ship_id: SHIP, order: stare, issued_at_client_t: i64::MAX }));

        let mut written = Remembered::default();
        let ticks = 60 * 86_400 * 1_000_000 / crate::server::TICK_US;
        for n in 0..ticks {
            server.tick(&mut wire).await.unwrap();
            if n % 500 == 0 {
                wire.take(ClientId(1));
                let taken = server.take_knowledge();
                written.samples.extend(taken.samples);
                written.discarded.extend(taken.discarded);
            }
        }
        let taken = server.take_knowledge();
        written.samples.extend(taken.samples);
        written.discarded.extend(taken.discarded);

        let knowledge = server.knowledge_of(SHIP).unwrap();
        let conclusion = knowledge.conclusion(target.id).expect("the log was read");
        let leading = conclusion.leading().unwrap();
        let Kind::Planet { transit, .. } = leading.kind else { panic!("{:?}", conclusion.transits) };
        assert!(leading.probability > SETTLED, "{:?}", conclusion.transits);
        let off = periods.iter().map(|p| (transit.period_s - p).abs()).fold(f64::INFINITY, f64::min);
        assert!(off < 3.0 * transit.period_sigma_s, "{} against {periods:?}", transit.period_s);

        // What the store is left holding, once it has deleted what it was told to, is exactly
        // what the craft still holds.
        assert!(!written.discarded.is_empty(), "the store is told to delete what was consumed");
        let left = written
            .samples
            .iter()
            .filter(|row| {
                !written.discarded.iter().any(|d| {
                    (d.ship_id, &d.subject, d.witness, d.band) == (row.ship_id, &row.subject, row.witness, row.band)
                        && row.observed_s <= d.through_s
                })
            })
            .count();
        let held: usize = knowledge.file(target.id).unwrap().series().iter().map(|s| s.len()).sum();
        assert_eq!(left, held);
        assert!(held < written.samples.len() / 2, "most of the log is gone: {held} of {}", written.samples.len());

        // A craft with no data modules watches on its onboard store, and reading its log into a
        // conclusion is what kept it from filling.
        assert_eq!(knowledge.capacity_bytes(), lc_world::fitting::ONBOARD_DATA_BYTES);
        assert!(knowledge.occupied_bytes() < knowledge.capacity_bytes() / 2.0);
        assert_eq!(knowledge.unkept(), 0);
    }

    /// Review item 20. A checkpoint whose write failed hands everything back, and the next one
    /// writes it all — nothing drained is lost to the failure.
    #[tokio::test]
    async fn a_failed_checkpoint_loses_nothing() {
        let (mut server, mut wire) = running().await;
        let failed = server.take_knowledge();
        assert!(!failed.files.is_empty() && !failed.samples.is_empty());
        server.untake_knowledge(failed.clone());
        server.tick(&mut wire).await.unwrap();
        let retried = server.take_knowledge();
        for file in &failed.files {
            assert!(retried.files.iter().any(|f| f.subject == file.subject), "a file was lost");
        }
        assert!(retried.samples.starts_with(&failed.samples), "and every sample, in order");
    }

    /// A craft whose knowledge rows are missing comes back with its charts, not knowing nothing.
    #[tokio::test]
    async fn a_craft_restored_without_knowledge_is_issued_its_charts() {
        let (mut old, _) = running().await;
        let checkpoint = old.checkpoint();
        let mut new = a_shard();
        assert!(new.adopt(checkpoint).is_empty());
        assert!(new.adopt_knowledge(&[], &[]).is_empty());
        let knowledge = new.knowledge_of(SHIP).expect("aboard");
        assert!(knowledge.knows(sky()[0].id), "the charts were issued");
        assert_eq!(new.duty_of(SHIP), old.duty_of(SHIP), "and the duty kept");
    }

    #[test]
    fn a_file_from_the_future_is_refused_rather_than_misread() {
        let star = Subject::Star(StarId::synthesise("archive", 1));
        let mut row = file_row(CraftId(1), star, &File::default(), 0);
        assert!(read_file(&row).is_ok());
        row.format = KNOWLEDGE_FORMAT + 1;
        assert!(read_file(&row).is_err(), "a newer shard's file, which this one cannot know the shape of");
    }

    #[test]
    fn a_log_row_reads_back_to_the_bit() {
        let logged = Logged {
            subject: Subject::Star(StarId::synthesise("archive", 1)),
            witness: lc_world::knowledge::observatory::CHARTS,
            band: Band::K,
            sample: Sample { observed_s: 1_234.567_891_23, deficit: -1.8149592025296526e-22, sigma: 1e-5 },
            learned_s: 1_300.0,
        };
        assert_eq!(read_log(&log_row(CraftId(1), &logged)).unwrap(), logged);
    }
}
