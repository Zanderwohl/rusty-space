//! The shard as of an instant, so the world outlives the process running it.
//!
//! The rest of the store holds *events* and a worldline can in principle be reconstructed from
//! them. This is a checkpoint instead: replaying the history of the world to find out where a
//! ship is costs more every day the world runs, and a shard has to come back in seconds.
//!
//! **The split here is deliberate.** Everything in this module is pure — what to save and how to
//! read it back — and nothing in it touches a database. Where it goes is
//! [`lc_store::ships`], and the two are joined only in the binary. That is what lets the
//! interesting half be tested without one.
//!
//! What a craft is, on disk, is [`lc_proto::Motion`] and a little metadata: the same
//! representation a reconnect is handed, because a checkpoint and a re-acquire are the same
//! question asked by different things. See [`lc_world::resume`].
//!
//! It is written with **postcard, not JSON**, and that is not a taste. `serde_json` does not
//! round-trip every f64 — `-1.8149592025296526e-22` comes back `-1.8149592025296529e-22` — and
//! this model rests on two machines folding the same numbers and agreeing to the bit. A
//! checkpoint that perturbs a coordinate by one place every restart would be a slow leak in
//! exactly the property everything else is built to preserve. Postcard writes an f64 as its
//! eight bytes and reads them back.

use std::collections::HashMap;

use glam::DVec3;
use lc_proto::ShipId;
use lc_store::ships::Ship;
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::fitting::{Balance, Fitting, Loadout};
use serde::{Deserialize, Serialize};

use crate::chase::Pursuit;
use crate::journal::Journal;
use crate::server::{Server, TICK_US};

/// A craft, as JSON in [`Ship::state`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub kind: u8,
    pub name: Option<String>,
    pub noise_floor: f32,
    /// The hull's length, metres. Stored rather than derived from the kind, because two ships
    /// of one kind may be different sizes and a craft that came back a different size from the
    /// one that was saved would be a silent loss nobody would think to look for.
    pub length_m: f64,
    pub motion: lc_proto::Motion,
    /// The standing intercept it was flying, which outlives the process as it outlives the
    /// pilot's connection. Appended in format 3.
    pub pursuit: Option<lc_proto::Pursuit>,
    /// Modules and energy, settled when saved. The balance in it is not read back: a shard
    /// stamps its own. Appended in format 4.
    pub fitting: Option<lc_proto::Fitting>,
}

/// [`Saved`] as format 3 wrote it, before ships had modules.
#[derive(Deserialize)]
struct SavedV3 {
    kind: u8,
    name: Option<String>,
    noise_floor: f32,
    length_m: f64,
    motion: lc_proto::Motion,
    pursuit: Option<lc_proto::Pursuit>,
}

/// [`Saved`] as format 2 wrote it, before a pursuit was kept. Read and never written, so the
/// shards already running do not refuse every account they have.
#[derive(Deserialize)]
struct SavedV2 {
    kind: u8,
    name: Option<String>,
    noise_floor: f32,
    length_m: f64,
    motion: lc_proto::Motion,
}

/// What wrote a row's bytes, and a contract rather than a note.
///
/// Postcard is positional: it cannot notice that it is reading an older shape, so it would read
/// one wrong rather than fail. A row whose format is not this one is refused.
///
/// **Bump it when [`Saved`] or anything inside [`lc_proto::Motion`] changes shape, and never
/// otherwise** — deliberately not [`lc_proto::PROTOCOL_VERSION`], which moves for reasons that
/// have nothing to do with how a craft is stored. Bumping it makes every existing row
/// unreadable, which is the point and is also the cost.
pub const SAVE_FORMAT: i32 = 4;

/// The oldest format still read. See [`decode`].
pub const OLDEST_FORMAT: i32 = 2;

/// Everything a shard needs to come back: the clock, the counter, and the craft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Checkpoint {
    /// The world's clock. Without it every saved motive, which is stamped in absolute
    /// coordinate time, reads as one that has not happened yet.
    pub now_t: i64,
    pub next_ship: i64,
    pub ships: Vec<Ship>,
}

/// A craft whose row could not be read.
///
/// Kept rather than dropped, because the account is the problem: letting it sign in would mint
/// a *second* ship for an account that already has a row, and the next checkpoint would then
/// fail on the unique account for as long as the shard ran. One player refused loudly beats a
/// shard that silently stops saving.
#[derive(Clone, Debug, PartialEq)]
pub struct Unreadable {
    pub ship_id: i64,
    pub account: Option<String>,
    pub why: String,
}

/// Turn a craft, its account and its standing intercept into a row.
pub fn save(
    craft: &Craft,
    account: Option<&str>,
    pursuit: Option<lc_proto::Pursuit>,
    saved_t: i64,
) -> Ship {
    let saved = Saved {
        kind: kind_code(craft.kind),
        name: craft.name.clone(),
        noise_floor: craft.noise_floor,
        length_m: craft.length_m,
        motion: (&craft.motion.snapshot()).into(),
        pursuit,
        fitting: craft.fitting().map(Into::into),
    };
    Ship {
        ship_id: craft.id.0,
        account: account.map(Into::into),
        saved_t,
        state: lc_proto::encode(&saved),
        format: SAVE_FORMAT,
    }
}

/// Read a row back into a craft, **at the time it was saved**.
///
/// Not at the present. A station and a crossing are stamped absolutely and would restore
/// correctly either way, but a ballistic arc is re-solved from the position and velocity beside
/// it — and solving those as though they were current puts the ship on an orbit it is not on.
/// The caller advances from here to now; see [`crate::server::Server::adopt`].
pub fn load(row: &Ship, system: Option<&lc_world::system::LocalSystem>) -> Result<Craft, String> {
    let saved = decode(row)?;
    let kind = kind_of(saved.kind).ok_or_else(|| format!("unknown craft kind {}", saved.kind))?;
    let snapshot = lc_world::resume::Snapshot::from(&saved.motion);
    let mut craft = Craft::at(CraftId(row.ship_id), kind, snapshot.position_ly);
    craft.name = saved.name;
    craft.noise_floor = saved.noise_floor;
    craft.length_m = saved.length_m;
    // Assigned rather than changed, and it leaves the craft with no history — so the motive it
    // was saved on extrapolates backwards for any time before the save. That is the best answer
    // there is: the shard genuinely does not know what this craft was doing before it was
    // written down, and the saved motive is what it *was* doing at the moment it was. It is not
    // a leak either way, because nothing after the save is involved. From here on the craft
    // records its stretches like any other, and `catch_up` fills the gap to now with real ones.
    craft.motion = snapshot.restore(system, row.saved_t as f64 * 1.0e-6);
    // A player's ship from before modules existed is given the starting ones, full. Its hull
    // was the starting hull, so it keeps its size and, by how the engines were rated, its
    // acceleration.
    let fitting = match &saved.fitting {
        Some(fitting) => Some(Fitting::from(fitting)),
        None if row.format < 4 && row.account.is_some() => Some(Fitting::full(
            Loadout::STARTING,
            Balance::DEFAULT,
            row.saved_t as f64 * 1.0e-6,
        )),
        None => None,
    };
    craft.fit(fitting);
    Ok(craft)
}

/// A row's bytes, in whichever format wrote them.
pub fn decode(row: &Ship) -> Result<Saved, String> {
    match row.format {
        SAVE_FORMAT => lc_proto::decode(&row.state).map_err(|why| why.to_string()),
        OLDEST_FORMAT => {
            let old: SavedV2 = lc_proto::decode(&row.state).map_err(|why| why.to_string())?;
            Ok(Saved {
                kind: old.kind,
                name: old.name,
                noise_floor: old.noise_floor,
                length_m: old.length_m,
                motion: old.motion,
                pursuit: None,
                fitting: None,
            })
        }
        3 => {
            let old: SavedV3 = lc_proto::decode(&row.state).map_err(|why| why.to_string())?;
            Ok(Saved {
                kind: old.kind,
                name: old.name,
                noise_floor: old.noise_floor,
                length_m: old.length_m,
                motion: old.motion,
                pursuit: old.pursuit,
                fitting: None,
            })
        }
        other => Err(format!("format {other} is not {OLDEST_FORMAT} to {SAVE_FORMAT}")),
    }
}

/// The stored number for a craft kind.
///
/// Exhaustive on purpose, and the numbers are fixed forever: a kind the world gains has to be
/// given one here before this compiles, and changing one silently reinterprets every saved row.
fn kind_code(kind: Kind) -> u8 {
    match kind {
        Kind::Ship => 0,
        Kind::Probe => 1,
        Kind::Relay => 2,
        Kind::Beacon => 3,
    }
}

fn kind_of(code: u8) -> Option<Kind> {
    match code {
        0 => Some(Kind::Ship),
        1 => Some(Kind::Probe),
        2 => Some(Kind::Relay),
        3 => Some(Kind::Beacon),
        _ => None,
    }
}

impl<J: Journal> Server<J> {
    /// The whole shard as of now, for whoever is going to write it down.
    ///
    /// Every craft, not the connected ones: a ship exists whether or not anyone is flying it,
    /// which is the same reason the tick advances the whole fleet.
    pub fn checkpoint(&self) -> Checkpoint {
        let account_of: HashMap<ShipId, &str> =
            self.by_account.iter().map(|(account, ship)| (*ship, account.as_str())).collect();
        Checkpoint {
            now_t: self.now_t,
            next_ship: self.next_ship,
            ships: self
                .fleet
                .iter()
                .map(|craft| {
                    let account = account_of.get(&ShipId(craft.id.0)).copied();
                    let pursuit = self.pursuits.get(&craft.id).map(|p| lc_proto::Pursuit {
                        quarry: p.quarry,
                        closeness: p.closeness.into(),
                    });
                    save(craft, account, pursuit, self.now_t)
                })
                .collect(),
        }
    }

    /// Take a checkpoint as this shard's world. **Load the world first**, or a ballistic arc
    /// has no system to be re-solved against and comes back as a straight line.
    ///
    /// Each craft is read at the time it was saved and then advanced to the checkpoint's clock,
    /// in tick-sized steps. The steps matter for exactly one motive: a ballistic arc crosses
    /// spheres of influence, and each crossing is an event that has to be folded as it comes.
    /// Everything else is a closed form and would not notice a single leap.
    ///
    /// The clock resumes where it stopped rather than jumping forward by however long the
    /// process was down. A shard that fabricated the missing years would be asserting that
    /// things happened in them, when nothing was journalled and nobody was told.
    pub fn adopt(
        &mut self,
        checkpoint: Checkpoint,
    ) -> Vec<Unreadable> {
        self.now_t = checkpoint.now_t;
        self.next_ship = self.next_ship.max(checkpoint.next_ship);
        let mut unreadable = Vec::new();
        for row in &checkpoint.ships {
            let system = self.position_of(row).and_then(|at| self.world.system_at(at));
            match load(row, system.as_deref()) {
                Ok(mut craft) => {
                    if let Some(mut fitting) = craft.fitting().cloned() {
                        fitting.balance = self.balance;
                        craft.fit(Some(fitting));
                    }
                    craft.enter(system, row.saved_t as f64 * 1.0e-6);
                    catch_up(&mut craft, row.saved_t, checkpoint.now_t);
                    if let Some(account) = &row.account {
                        self.by_account.insert(account.clone(), ShipId(craft.id.0));
                    }
                    // Taken up again on the next tick, which plans as for a fresh order.
                    let pursuit = decode(row).ok().and_then(|saved| saved.pursuit);
                    if let Some(pursuit) = pursuit {
                        self.pursuits.insert(craft.id, Pursuit {
                            quarry: pursuit.quarry,
                            closeness: pursuit.closeness.into(),
                            last_plan_t: i64::MIN,
                            last_seen: None,
                        });
                    }
                    self.next_ship = self.next_ship.max(craft.id.0 + 1);
                    self.fleet.insert(craft);
                }
                Err(why) => {
                    if let Some(account) = &row.account {
                        self.blocked.insert(account.clone(), why.clone());
                    }
                    unreadable.push(Unreadable {
                        ship_id: row.ship_id,
                        account: row.account.clone(),
                        why,
                    });
                }
            }
        }
        unreadable
    }

    /// Where a saved craft is, without committing to being able to read the rest of it.
    fn position_of(&mut self, row: &Ship) -> Option<DVec3> {
        let saved = decode(row).ok()?;
        Some(DVec3::from_array(saved.motion.at_ly))
    }
}

/// Fly a restored craft from when it was saved to when the shard is now.
///
/// Tick-sized steps rather than one leap, because a ballistic arc folds a patch when it reaches
/// one and a single step past several would fold at most one of them. Capped, because the cost
/// is linear in the downtime and a shard that has been off for a month must still come back:
/// past the cap the remainder is taken in one step, which is exact for everything but a conic
/// that changes primary in it.
fn catch_up(craft: &mut Craft, from_t: i64, to_t: i64) {
    let mut at = from_t;
    let mut steps = 0;
    while at < to_t && steps < MAX_CATCHUP_TICKS {
        let next = (at + TICK_US).min(to_t);
        craft.advance(next as f64 * 1.0e-6, (next - at) as f64 * 1.0e-6);
        at = next;
        steps += 1;
    }
    if at < to_t {
        craft.advance(to_t as f64 * 1.0e-6, (to_t - at) as f64 * 1.0e-6);
    }
}

/// How many tick-sized steps a restored craft is flown in before the rest is taken at once.
///
/// Twenty thousand is about two and a half hours of downtime at the design rate, which covers a
/// restart, a deploy and an outage somebody slept through.
pub const MAX_CATCHUP_TICKS: usize = 20_000;

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;
    use lc_world::motion::Motive;

    fn a_craft() -> Craft {
        let mut craft = Craft::at(CraftId(5), Kind::Probe, DVec3::new(1.0, 2.0, 3.0));
        craft.name = Some("Ada".into());
        craft.noise_floor = 0.25;
        // Not its kind's default, which is the whole case worth saving: a length equal to the
        // default would round-trip just as well through a row that did not store one at all.
        craft.length_m = 12_345.0;
        craft
    }

    #[test]
    fn a_craft_saved_and_read_back_is_the_same_craft() {
        let craft = a_craft();
        let row = save(&craft, Some("acct-1"), None, 7_000_000);
        assert_eq!(row.ship_id, 5);
        assert_eq!(row.account.as_deref(), Some("acct-1"));

        let back = load(&row, None).expect("it reads");
        assert_eq!(back.id, craft.id);
        assert_eq!(back.kind, craft.kind);
        assert_eq!(back.name, craft.name);
        assert_eq!(back.noise_floor, craft.noise_floor);
        assert_eq!(back.length_m, craft.length_m, "the ship came back a different size");
        assert_ne!(back.length_m, back.kind.length_m(), "premise: it is not the kind's default");
        assert_eq!(back.motion.position_ly, craft.motion.position_ly);
        assert_eq!(back.motion.motive, craft.motion.motive);
    }

    /// The numbers are a storage format and may never move. A kind written as 2 has to read
    /// back as the kind that was written, for as long as any row exists.
    #[test]
    fn the_kind_codes_are_fixed_and_round_trip() {
        assert_eq!(
            [Kind::Ship, Kind::Probe, Kind::Relay, Kind::Beacon].map(kind_code),
            [0, 1, 2, 3],
        );
        for kind in [Kind::Ship, Kind::Probe, Kind::Relay, Kind::Beacon] {
            assert_eq!(kind_of(kind_code(kind)), Some(kind));
        }
        assert_eq!(kind_of(4), None, "an unknown code is a refusal, not a default ship");
    }

    /// **The format is exact, and JSON was not.**
    ///
    /// This value is the one that found it: written and read by `serde_json` it comes back
    /// `-1.8149592025296529e-22`, one place out. It reached a coordinate of a real crossing, so
    /// every restart would have moved every ship a little — in a model whose whole claim is that
    /// two machines folding the same numbers agree to the bit.
    #[test]
    fn a_coordinate_survives_the_round_trip_exactly() {
        let awkward = -1.8149592025296526e-22;
        let mut craft = a_craft();
        craft.motion.position_ly = DVec3::new(4.200079062537049, awkward, 0.0);
        craft.motion.beta = DVec3::new(awkward, 0.0, 1.0e-9);

        let back = load(&save(&craft, None, None, 0), None).expect("it reads");
        assert_eq!(back.motion.position_ly, craft.motion.position_ly);
        assert_eq!(back.motion.beta, craft.motion.beta);
        assert_eq!(
            back.motion.position_ly.y.to_bits(),
            awkward.to_bits(),
            "the stored form is not exact",
        );

        // And the fact behind the choice, stated where it will be read if anyone proposes JSON
        // again.
        let text = serde_json::to_string(&awkward).expect("it writes");
        let json: f64 = serde_json::from_str(&text).expect("it reads");
        assert_ne!(json.to_bits(), awkward.to_bits(), "serde_json round-trips this now; check why");
    }

    /// A row written by an older shape is refused, not misread. Postcard is positional and
    /// would happily read the wrong fields out of the right bytes.
    #[test]
    fn a_row_from_another_format_is_refused() {
        let mut row = save(&a_craft(), None, None, 0);
        row.format = OLDEST_FORMAT - 1;
        let why = load(&row, None).expect_err("it should refuse");
        assert!(why.contains("format"), "{why}");
    }

    /// A pursuit is written down with the craft, so a ship hanging about with another is still
    /// hanging about after a restart.
    #[test]
    fn a_pursuit_survives_the_round_trip() {
        let pursuit = lc_proto::Pursuit {
            quarry: lc_proto::ShipId(9),
            closeness: lc_proto::Closeness::Intimate,
        };
        let row = save(&a_craft(), None, Some(pursuit), 0);
        assert_eq!(decode(&row).expect("it reads").pursuit, Some(pursuit));
    }

    /// **Format 2 still reads**, as a craft with no pursuit. Refusing it would refuse every
    /// account on every shard already running, which is the cost the format bump exists to
    /// make visible and nothing here needs to pay.
    #[test]
    fn a_row_from_before_pursuits_were_kept_still_reads() {
        #[derive(Serialize)]
        struct Old {
            kind: u8,
            name: Option<String>,
            noise_floor: f32,
            length_m: f64,
            motion: lc_proto::Motion,
        }
        let craft = a_craft();
        let old = Old {
            kind: kind_code(craft.kind),
            name: craft.name.clone(),
            noise_floor: craft.noise_floor,
            length_m: craft.length_m,
            motion: (&craft.motion.snapshot()).into(),
        };
        let row = Ship {
            ship_id: 5,
            account: None,
            saved_t: 0,
            state: lc_proto::encode(&old),
            format: OLDEST_FORMAT,
        };
        let back = load(&row, None).expect("a format 2 row reads");
        assert_eq!(back.length_m, craft.length_m);
        assert_eq!(decode(&row).unwrap().pursuit, None);
    }

    #[test]
    fn a_ships_modules_and_energy_survive_the_round_trip() {
        let mut craft = Craft::at(CraftId(5), Kind::Ship, DVec3::ZERO);
        craft.fit(Some(Fitting::full(Loadout::STARTING, Balance::DEFAULT, 0.0)));
        craft.begin_refit(Loadout { engines: 7, ..Loadout::STARTING }, 10.0).unwrap();
        let back = load(&save(&craft, Some("acct"), None, 20_000_000), None).expect("it reads");
        assert_eq!(back.fitting(), craft.fitting());
        assert!(back.is_refitting(20.0));
    }

    /// **Format 3 still reads**, and a player's ship in it is given the starting modules.
    #[test]
    fn a_player_ship_from_before_modules_comes_back_with_the_starting_ones() {
        #[derive(Serialize)]
        struct Old {
            kind: u8,
            name: Option<String>,
            noise_floor: f32,
            length_m: f64,
            motion: lc_proto::Motion,
            pursuit: Option<lc_proto::Pursuit>,
        }
        let craft = Craft::at(CraftId(5), Kind::Ship, DVec3::ZERO);
        let old = Old {
            kind: kind_code(craft.kind),
            name: None,
            noise_floor: 0.0,
            length_m: 500.0,
            motion: (&craft.motion.snapshot()).into(),
            pursuit: None,
        };
        let row = |account: Option<&str>| Ship {
            ship_id: 5,
            account: account.map(Into::into),
            saved_t: 3_000_000,
            state: lc_proto::encode(&old),
            format: 3,
        };
        let player = load(&row(Some("acct")), None).expect("a format 3 row reads");
        let fitting = player.fitting().expect("a player's ship is fitted");
        assert_eq!(fitting.loadout, Loadout::STARTING);
        assert!((player.length_m - 500.0).abs() < 1.0e-9);
        assert!((player.rated_drive(3.0).accel_g - 5.0).abs() < 1.0e-9);
        assert!(load(&row(None), None).unwrap().fitting().is_none(), "a craft with no pilot is not");
    }

    /// A row that cannot be read is an error and never a fresh ship at the origin.
    #[test]
    fn an_unreadable_row_is_an_error() {
        let row = Ship {
            ship_id: 5,
            account: None,
            saved_t: 0,
            state: b"not postcard, and too short for this shape".to_vec(),
            format: SAVE_FORMAT,
        };
        assert!(load(&row, None).is_err());
    }

    /// What the crew's own clock is for: a ship's proper time is however long they have lived
    /// through, and a restart is not a thing that un-ages anybody.
    #[test]
    fn the_crews_clock_survives_the_round_trip() {
        let mut craft = a_craft();
        craft.motion.clock_s = 86_400.0 * 365.0;
        let back = load(&save(&craft, None, None, 0), None).expect("it reads");
        assert_eq!(back.motion.clock_s, craft.motion.clock_s);
    }

    #[test]
    fn a_drifting_ship_keeps_the_line_it_was_on() {
        let mut craft = a_craft();
        craft.motion.beta = DVec3::new(0.0, 0.1, 0.0);
        craft.motion.resume_drifting(DVec3::new(9.0, 0.0, 0.0), 1_234.0);
        let back = load(&save(&craft, None, None, 5_000_000), None).expect("it reads");
        match back.motion.motive {
            Motive::Drifting { from_ly, since_t } => {
                assert_eq!(from_ly, DVec3::new(9.0, 0.0, 0.0));
                assert_eq!(since_t, 1_234.0);
            }
            other => unreachable!("{other:?}"),
        }
        assert_eq!(back.motion.beta, craft.motion.beta);
    }
}
