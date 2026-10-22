//! Faction keys: who minted which, and who holds which from when.
//!
//! A faction is a key and its chat, with no roster; see `lightcone/docs/23-factions.md`. Holding
//! is learned like the ship keyring in [`crate::radio`]: as the light of a readable gift lands, on
//! every craft it lands on, signed in or not.

use std::collections::{BTreeMap, HashMap};

use lc_proto::{Faction, FactionId, NAME_LIMIT, Refusal, ShipId};
use lc_store::chat::{Founded, Holds};
use lc_world::craft::CraftId;

#[derive(Default)]
pub struct Factions {
    named: HashMap<FactionId, Faction>,
    held: HashMap<CraftId, BTreeMap<FactionId, i64>>,
    /// Written by the next flush, then cleared.
    pub(crate) founded: Vec<Founded>,
    pub(crate) taught: Vec<Holds>,
}

impl Factions {
    /// Mint a key for `founder`, held from `at`: the next generation of `replaces`, which only a
    /// holder of it may mint, or a first.
    pub fn found(&mut self, founder: CraftId, name: &str, replaces: Option<FactionId>, at: i64) -> Result<Faction, Refusal> {
        let name = name.trim();
        if name.is_empty() || name.len() > NAME_LIMIT {
            return Err(Refusal::Impossible);
        }
        let lineage = match replaces {
            None => Vec::new(),
            Some(old) if !self.holds(founder, old, at) => return Err(Refusal::NoKey),
            Some(old) => {
                let older = self.named.get(&old).map(|f| f.lineage.clone()).unwrap_or_default();
                std::iter::once(old).chain(older).collect()
            }
        };
        // Hashed rather than counted, so an identifier says nothing about how many factions
        // there are or which was founded first.
        let mut salt = 0u64;
        let key = loop {
            let h = lc_world::rng::hash(&[founder.0 as u64, at as u64, salt]);
            let key = FactionId((h >> 1) as i64);
            if key.0 != 0 && !self.named.contains_key(&key) {
                break key;
            }
            salt += 1;
        };
        let faction = Faction { key, name: name.to_string(), founder: ShipId(founder.0), lineage };
        self.named.insert(key, faction.clone());
        self.founded.push(Founded {
            faction: key.0,
            founder: founder.0,
            name: faction.name.clone(),
            lineage: faction.lineage.iter().map(|k| k.0).collect(),
            founded_t: at,
        });
        self.learn(founder, key, at);
        Ok(faction)
    }

    /// `holder` holds `key` from `at`. The earliest landing wins.
    pub fn learn(&mut self, holder: CraftId, key: FactionId, at: i64) {
        let learned = self.held.entry(holder).or_default().entry(key).or_insert(at);
        if at <= *learned {
            *learned = at;
            self.taught.push(Holds { holder: holder.0, faction: key.0, learned_t: at });
        }
    }

    /// Whether `holder` can read what is sealed to `key` and lands at `at`.
    pub fn holds(&self, holder: CraftId, key: FactionId, at: i64) -> bool {
        self.held.get(&holder).and_then(|keys| keys.get(&key)).is_some_and(|learned| *learned <= at)
    }

    pub fn faction(&self, key: FactionId) -> Option<Faction> {
        self.named.get(&key).cloned()
    }

    /// Every key `holder` holds by `now`, oldest held first.
    pub fn held_by(&self, holder: CraftId, now: i64) -> Vec<Faction> {
        let mut held: Vec<(i64, Faction)> = self
            .held
            .get(&holder)
            .into_iter()
            .flatten()
            .filter(|(_, learned)| **learned <= now)
            .filter_map(|(key, learned)| Some((*learned, self.faction(*key)?)))
            .collect();
        held.sort_by_key(|(learned, f)| (*learned, f.key));
        held.into_iter().map(|(_, f)| f).collect()
    }

    /// Take up what a previous run wrote. Nothing here is written again.
    pub fn resume(&mut self, founded: Vec<Founded>, holds: Vec<Holds>) {
        for f in founded {
            let key = FactionId(f.faction);
            let lineage = f.lineage.into_iter().map(FactionId).collect();
            self.named.insert(key, Faction { key, name: f.name, founder: ShipId(f.founder), lineage });
        }
        for h in holds {
            let learned = self.held.entry(CraftId(h.holder)).or_default().entry(FactionId(h.faction)).or_insert(h.learned_t);
            *learned = (*learned).min(h.learned_t);
        }
    }

    pub(crate) fn is_dirty(&self) -> bool {
        !self.founded.is_empty() || !self.taught.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_founder_holds_its_key_from_the_moment_it_is_minted_and_not_before() {
        let mut factions = Factions::default();
        let wake = factions.found(CraftId(1), " Wake ", None, 1_000).unwrap();
        assert_eq!(wake.name, "Wake");
        assert!(factions.holds(CraftId(1), wake.key, 1_000));
        assert!(!factions.holds(CraftId(1), wake.key, 999));
        assert!(!factions.holds(CraftId(2), wake.key, 1_000));
    }

    #[test]
    fn two_foundings_at_once_are_two_keys() {
        let mut factions = Factions::default();
        let a = factions.found(CraftId(1), "A", None, 1_000).unwrap();
        let b = factions.found(CraftId(1), "A", None, 1_000).unwrap();
        assert_ne!(a.key, b.key);
    }

    #[test]
    fn the_earliest_landing_is_when_a_key_is_held() {
        let mut factions = Factions::default();
        let wake = factions.found(CraftId(1), "Wake", None, 0).unwrap();
        factions.learn(CraftId(2), wake.key, 900);
        factions.learn(CraftId(2), wake.key, 500);
        factions.learn(CraftId(2), wake.key, 700);
        assert!(factions.holds(CraftId(2), wake.key, 500));
        assert!(!factions.holds(CraftId(2), wake.key, 499));
        assert_eq!(factions.held_by(CraftId(2), 400), Vec::new());
        assert_eq!(factions.held_by(CraftId(2), 500), vec![wake]);
    }

    #[test]
    fn a_name_must_say_something() {
        let mut factions = Factions::default();
        assert!(factions.found(CraftId(1), "  ", None, 0).is_err());
        assert!(factions.found(CraftId(1), &"x".repeat(NAME_LIMIT + 1), None, 0).is_err());
    }

    #[test]
    fn a_key_knows_every_key_it_replaces_and_only_a_holder_may_replace_one() {
        let mut factions = Factions::default();
        let first = factions.found(CraftId(1), "Wake", None, 0).unwrap();
        assert_eq!(factions.found(CraftId(2), "Wake", Some(first.key), 10), Err(Refusal::NoKey));
        let second = factions.found(CraftId(1), "Wake", Some(first.key), 10).unwrap();
        let third = factions.found(CraftId(1), "Wake", Some(second.key), 20).unwrap();
        assert_eq!(third.generation(), 3);
        assert!(third.supersedes(first.key), "without anyone holding the second");
        assert_eq!(third.lineage, vec![second.key, first.key]);
    }

    /// Nothing reconciles a fork: two holders replacing one key make two keys, and each says who
    /// made it. No shard could pick a winner without information nobody in the game has yet.
    #[test]
    fn two_holders_replacing_one_key_fork_it_and_each_branch_names_its_founder() {
        let mut factions = Factions::default();
        let wake = factions.found(CraftId(1), "Wake", None, 0).unwrap();
        factions.learn(CraftId(2), wake.key, 5);
        let loyal = factions.found(CraftId(1), "Wake", Some(wake.key), 10).unwrap();
        let rival = factions.found(CraftId(2), "Wake", Some(wake.key), 10).unwrap();
        assert_ne!(loyal.key, rival.key);
        assert_eq!((loyal.generation(), rival.generation()), (2, 2));
        assert!(loyal.supersedes(wake.key) && rival.supersedes(wake.key));
        assert_eq!((loyal.founder, rival.founder), (ShipId(1), ShipId(2)));
    }

    mod play {
        use glam::DVec3;
        use lc_proto::{Aim, Body, ClientId, Inbound, Intent, Order, Outbound, Secrecy, ShipId, Spoken};

        use super::*;
        use crate::journal::Memory;
        use crate::server::{Server, TICK_US};
        use crate::transport::Loopback;

        const TWO_LIGHT_HOURS: f64 = 7_200.0 * 1_000_000.0;
        const ADA: ClientId = ClientId(1);
        const BRY: ClientId = ClientId(2);
        const CY: ClientId = ClientId(3);

        /// Ada at the origin; Bry two light-hours out, and Cy close beside Bry.
        fn three() -> (Server<Memory>, Loopback) {
            let mut server = Server::new(Memory::default(), 0, 1);
            let far = DVec3::new(TWO_LIGHT_HOURS, 0.0, 0.0);
            server.admit(ADA, crate::world::still(ShipId(1), DVec3::ZERO), 0.0);
            server.admit(BRY, crate::world::still(ShipId(2), far), 0.0);
            server.admit(CY, crate::world::still(ShipId(3), far + DVec3::new(0.0, 1.0e8, 0.0)), 0.0);
            (server, Loopback::new())
        }

        fn tell(server: &Server<Memory>, wire: &mut Loopback, client: ClientId, order: Order) {
            let ship_id = ShipId(client.0 as i64);
            wire.client_says(client, Inbound::Act(Intent { ship_id, order, issued_at_client_t: server.now_t() }));
        }

        async fn run_for(server: &mut Server<Memory>, wire: &mut Loopback, us: f64) {
            let until = server.now_t() + us as i64 + TICK_US;
            while server.now_t() < until {
                server.tick(wire).await.unwrap();
            }
        }

        async fn found(server: &mut Server<Memory>, wire: &mut Loopback, client: ClientId, name: &str) -> FactionId {
            tell(server, wire, client, Order::Found { name: name.into(), key: FactionId(0), replaces: None });
            server.tick(wire).await.unwrap();
            wire.take(client)
                .into_iter()
                .find_map(|m| match m {
                    Outbound::Accepted { order: Order::Found { key, .. }, .. } => Some(key),
                    _ => None,
                })
                .expect("founding was refused")
        }

        fn chat(key: FactionId, body: &str) -> Order {
            Order::Say { to: None, aim: Aim::Omni, secrecy: Secrecy::Faction(key), body: body.into(), idem: body.len() as u64 }
        }

        /// What reached a client, by sender, as it may read it.
        fn heard(messages: &[Outbound]) -> Vec<(i64, Spoken)> {
            messages
                .iter()
                .flat_map(|m| match m {
                    Outbound::Sightings(list) => list.iter().map(|c| c.get()).collect::<Vec<_>>(),
                    _ => Vec::new(),
                })
                .filter(|s| s.kind == lc_proto::kind::MESSAGE || s.kind == lc_proto::kind::KEY)
                .filter_map(|s| Some((s.source_id, serde_json::from_str(&s.payload).ok()?)))
                .collect()
        }

        fn text_under(heard: &[(i64, Spoken)], key: FactionId) -> Vec<Body> {
            heard.iter().filter(|(_, s)| s.under == Some(key)).map(|(_, s)| s.body.clone()).collect()
        }

        /// 11e's done-when. Cy is left off the new key: it still reads the old chat, cannot read
        /// the new one, and Ada sees Bry speaking under the old key until the new one reaches Bry.
        #[tokio::test]
        async fn a_member_left_off_a_new_key_reads_the_old_chat_and_not_the_new() {
            let (mut server, mut wire) = three();
            // Bry's ship key, so Ada can seal the new faction key to Bry alone.
            tell(&server, &mut wire, BRY, Order::OfferKey { to: Some(ShipId(1)), aim: Aim::Omni });
            let old = found(&mut server, &mut wire, ADA, "Wake").await;
            // Open: everyone it reaches holds it, which is how Cy got in.
            tell(&server, &mut wire, ADA, Order::GiveKey { key: old, to: None, aim: Aim::Omni, secrecy: Secrecy::Open });
            run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;
            for client in [BRY, CY] {
                let gave: Vec<Body> = heard(&wire.take(client)).into_iter().map(|(_, s)| s.body).collect();
                assert!(gave.iter().any(|b| matches!(b, Body::Faction(f) if f.key == old && f.name == "Wake")), "{gave:?}");
            }
            wire.take(ADA);

            tell(&server, &mut wire, ADA, Order::Found { name: "Wake".into(), key: FactionId(0), replaces: Some(old) });
            server.tick(&mut wire).await.unwrap();
            let new = wire
                .take(ADA)
                .into_iter()
                .find_map(|m| match m {
                    Outbound::Accepted { order: Order::Found { key, .. }, .. } => Some(key),
                    _ => None,
                })
                .expect("re-keying was refused");
            assert!(server.factions.faction(new).unwrap().supersedes(old));
            tell(&server, &mut wire, ADA, Order::GiveKey { key: new, to: Some(ShipId(2)), aim: Aim::Omni, secrecy: Secrecy::Sealed });
            server.tick(&mut wire).await.unwrap();
            tell(&server, &mut wire, ADA, chat(new, "new key"));
            tell(&server, &mut wire, BRY, chat(old, "still old"));
            // Bry cannot speak under a key whose light has not reached it.
            tell(&server, &mut wire, BRY, chat(new, "too soon"));
            server.tick(&mut wire).await.unwrap();
            assert!(wire.take(BRY).iter().any(|m| matches!(m, Outbound::Refused { reason: lc_proto::Refusal::NoKey, .. })));
            run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;

            let cy = heard(&wire.take(CY));
            assert_eq!(text_under(&cy, old), vec![Body::Text("still old".into())], "the old chat is still Cy's");
            assert_eq!(text_under(&cy, new), vec![Body::Unreadable], "the new chat is not");
            assert!(cy.iter().any(|(from, s)| *from == 1 && s.sealed && s.body == Body::Unreadable), "the key went past Cy sealed");

            let bry = heard(&wire.take(BRY));
            assert_eq!(text_under(&bry, new), vec![Body::Text("new key".into())]);
            assert!(server.factions.holds(CraftId(2), new, server.now_t()));
            assert!(!server.factions.holds(CraftId(3), new, server.now_t()));

            let ada = heard(&wire.take(ADA));
            assert_eq!(text_under(&ada, old), vec![Body::Text("still old".into())], "Bry, still on the old key");
        }

        /// A key opens what lands after it does, and nothing heard before.
        #[tokio::test]
        async fn a_key_does_not_open_what_was_heard_before_it_arrived() {
            let (mut server, mut wire) = three();
            let key = found(&mut server, &mut wire, ADA, "Wake").await;
            tell(&server, &mut wire, ADA, chat(key, "before"));
            server.tick(&mut wire).await.unwrap();
            tell(&server, &mut wire, ADA, Order::GiveKey { key, to: None, aim: Aim::Omni, secrecy: Secrecy::Open });
            server.tick(&mut wire).await.unwrap();
            tell(&server, &mut wire, ADA, chat(key, "after"));
            run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;
            let bry = heard(&wire.take(BRY));
            assert_eq!(text_under(&bry, key), vec![Body::Unreadable, Body::Text("after".into())]);

            let Some(Outbound::Backlog { messages, .. }) = server.backlog(ShipId(2), server.now_t()).await.unwrap() else {
                panic!("no backlog");
            };
            let chat: Vec<Body> = messages.into_iter().filter(|m| m.under == Some(key)).map(|m| m.body).collect();
            assert_eq!(chat, vec![Body::Unreadable, Body::Text("after".into())], "the backlog agrees with what landed");
        }

        #[tokio::test]
        async fn a_faction_chat_is_addressed_to_nobody_and_needs_the_key() {
            let (mut server, mut wire) = three();
            let key = found(&mut server, &mut wire, ADA, "Wake").await;
            let to_bry = Order::Say { to: Some(ShipId(2)), aim: Aim::Omni, secrecy: Secrecy::Faction(key), body: "hi".into(), idem: 1 };
            tell(&server, &mut wire, ADA, to_bry);
            tell(&server, &mut wire, BRY, chat(key, "not mine"));
            tell(&server, &mut wire, BRY, Order::GiveKey { key, to: None, aim: Aim::Omni, secrecy: Secrecy::Open });
            server.tick(&mut wire).await.unwrap();
            assert!(wire.take(ADA).iter().any(|m| matches!(m, Outbound::Refused { reason: lc_proto::Refusal::Impossible, .. })));
            let refused = wire.take(BRY).into_iter().filter(|m| matches!(m, Outbound::Refused { reason: lc_proto::Refusal::NoKey, .. })).count();
            assert_eq!(refused, 2, "neither speaking under a key nor giving one is open to a craft without it");
        }

        /// A note under a faction key reaches a member in a report under that key, and a note
        /// kept for its author goes nowhere. See `lightcone/docs/23-factions.md` §Notes.
        #[tokio::test]
        async fn a_faction_note_reaches_who_holds_the_key_and_a_private_one_stays_home() {
            use lc_proto::{Scope, Writing};
            use lc_world::knowledge::{Bearing, Sighting, Subject, Witness};
            let (mut server, mut wire) = three();
            // About light delay, not seeing: these craft are too dark or too far to detect.
            server.see_everything(true);
            let star = lc_world::sky::StarId::synthesize("notes", 1);
            server.aboard(CraftId(1)).knowledge.sighted(star, Sighting {
                witness: Witness(1),
                observed_s: 0.0,
                bearing: Bearing { observer_ly: DVec3::ZERO, toward: DVec3::X, sigma_rad: 1e-9 },
                size: None,
                range_m: None,
                spin_s: None,
                band: em_spectra::Band::V,
                flux: 1e-12,
                flux_sigma: 1e-15,
                lineage: Vec::new(),
            });
            let key = found(&mut server, &mut wire, ADA, "Wake").await;
            tell(&server, &mut wire, ADA, Order::GiveKey { key, to: Some(ShipId(2)), aim: Aim::Ship(ShipId(2)), secrecy: Secrecy::Open });
            run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;
            assert!(server.factions.holds(CraftId(2), key, server.now_t()), "premise: Bry holds the key");

            let write = |writing, scope| Order::Write { subject: lc_proto::Subject::Star(star.get()), writing, scope };
            tell(&server, &mut wire, ADA, write(Writing::Note("mine alone".into()), Scope::Own));
            tell(&server, &mut wire, ADA, write(Writing::Note("muster here".into()), Scope::Faction(key)));
            server.tick(&mut wire).await.unwrap();
            tell(&server, &mut wire, ADA, Order::SendReport { to: None, aim: Aim::Omni, secrecy: Secrecy::Faction(key), idem: 7 });
            server.tick(&mut wire).await.unwrap();
            assert!(wire.take(ADA).iter().any(|m| matches!(m, Outbound::Accepted { order: Order::SendReport { .. }, .. })));
            run_for(&mut server, &mut wire, TWO_LIGHT_HOURS).await;

            let notes = |server: &mut Server<Memory>, craft| -> Vec<String> {
                let knowledge = &server.aboard(CraftId(craft)).knowledge;
                knowledge.file(Subject::Star(star)).map(|f| {
                    f.thread().ordered().into_iter().map(|(_, h)| match &h.entry.writing {
                        Writing::Note(t) | Writing::Name(t) => t.clone(),
                        Writing::Was(_) => String::new(),
                    }).collect()
                }).unwrap_or_default()
            };
            assert_eq!(notes(&mut server, 2), vec!["muster here"]);
            assert!(notes(&mut server, 3).is_empty(), "Cy heard the report and holds no key");
            assert_eq!(notes(&mut server, 1).len(), 2, "both stay with their author");
        }

        #[tokio::test]
        async fn a_returning_pilot_is_told_the_keys_it_holds() {
            let (mut server, mut wire) = three();
            let key = found(&mut server, &mut wire, ADA, "Wake").await;
            let mut told = Loopback::new();
            server.welcome(ClientId(9), ShipId(1), "Ada".into(), "", &mut told);
            assert!(told.take(ClientId(9)).contains(&Outbound::Factions {
                ship_id: ShipId(1),
                factions: vec![Faction { key, name: "Wake".into(), founder: ShipId(1), lineage: vec![] }],
            }));
        }
    }
}
