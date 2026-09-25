//! Presets, as the shard holds them: every account's, in memory, checkpointed beside the bookmarks.
//!
//! Not part of the world, so nothing here is cleared. A form is checked only for size; its
//! geometry is validated when a preset is applied, like any other target. See
//! `lightcone/docs/29-ship-form.md` §Your own presets.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use lc_proto::form::{MAX_PRESETS, PRESET_NAME_LIMIT};
use lc_proto::{ClientId, Form, FormFault, Outbound, Preset, Refusal};
use lc_world::form::MAX_PARTS;

use crate::journal::Journal;
use crate::server::Server;
use crate::transport::Transport;

/// A preset saved or deleted since the last checkpoint. `None` is a deletion.
pub type Change = (String, String, Option<Form>);

#[derive(Default)]
pub struct Presets {
    /// By name, so the list a client is sent is in a stable order.
    kept: HashMap<String, BTreeMap<String, Form>>,
    /// What to write. The state is read when it is taken, so the last change to a name wins.
    dirty: BTreeSet<(String, String)>,
}

impl Presets {
    /// What to send an account on connecting and after each change.
    pub fn for_account(&self, account: &str) -> Vec<Preset> {
        self.kept
            .get(account)
            .into_iter()
            .flatten()
            .map(|(name, form)| Preset { name: name.clone(), form: form.clone() })
            .collect()
    }

    /// Keep a form under a name, replacing whatever the account had under it.
    pub fn save(&mut self, account: &str, name: String, form: Form) -> Result<(), Refusal> {
        if name.is_empty() || name.len() > PRESET_NAME_LIMIT {
            return Err(Refusal::PresetName);
        }
        if form.parts.len() > MAX_PARTS {
            return Err(Refusal::Form(FormFault::TooManyParts { found: form.parts.len() as u32 }));
        }
        let kept = self.kept.entry(account.to_owned()).or_default();
        if kept.len() >= MAX_PRESETS && !kept.contains_key(&name) {
            return Err(Refusal::TooManyPresets);
        }
        self.dirty.insert((account.to_owned(), name.clone()));
        kept.insert(name, form);
        Ok(())
    }

    /// Whether there was one to delete.
    pub fn delete(&mut self, account: &str, name: &str) -> bool {
        let Some(kept) = self.kept.get_mut(account) else { return false };
        if kept.remove(name).is_none() {
            return false;
        }
        self.dirty.insert((account.to_owned(), name.to_owned()));
        true
    }

    /// Changes since the last time this was asked, and cleared by asking.
    pub fn take_dirty(&mut self) -> Vec<Change> {
        std::mem::take(&mut self.dirty)
            .into_iter()
            .map(|(account, name)| {
                let form = self.kept.get(&account).and_then(|kept| kept.get(&name)).cloned();
                (account, name, form)
            })
            .collect()
    }

    /// Mark these as needing writing again, because the write failed. Whatever they hold by the
    /// next checkpoint is what is written, so a change made since is not undone.
    pub fn redirty(&mut self, changes: &[Change]) {
        for (account, name, _) in changes {
            self.dirty.insert((account.clone(), name.clone()));
        }
    }

    /// Adopt what was saved, returning a line for each row that would not read.
    pub fn adopt(&mut self, rows: Vec<lc_store::presets::Preset>) -> Vec<String> {
        let mut problems = Vec::new();
        for row in rows {
            match lc_proto::decode::<Form>(&row.form) {
                Ok(form) => {
                    self.kept.entry(row.account).or_default().insert(row.name, form);
                }
                Err(why) => problems.push(format!("{}'s preset {:?}: {why}", row.account, row.name)),
            }
        }
        problems
    }
}

impl<J: Journal> Server<J> {
    /// Save a preset, or delete it when `form` is `None`, and answer with the account's whole list.
    ///
    /// Only to `from`: signing in displaces the account's earlier connection, so there is no
    /// other one to keep current, and the next machine is sent the list with its welcome.
    ///
    /// Ignored from a connection with no account, as a bookmark is: there is nowhere to keep it.
    pub(crate) fn keep_preset(&mut self, from: ClientId, name: String, form: Option<Form>, wire: &mut impl Transport) {
        let Some(state) = self.clients.get(&from) else { return };
        if state.account.is_empty() {
            return;
        }
        let (ship_id, account) = (state.ship, state.account.clone());
        match form {
            Some(form) => {
                if let Err(reason) = self.presets.save(&account, name, form) {
                    wire.send(from, Outbound::Refused { ship_id, reason });
                    return;
                }
            }
            None => {
                self.presets.delete(&account, &name);
            }
        }
        wire.send(from, Outbound::Presets(self.presets.for_account(&account)));
    }
}

/// Split changes into the rows to write and the `(account, name)` pairs to delete.
pub fn rows(changes: &[Change]) -> (Vec<lc_store::presets::Preset>, Vec<(String, String)>) {
    let mut saved = Vec::new();
    let mut deleted = Vec::new();
    for (account, name, form) in changes {
        match form {
            Some(form) => saved.push(lc_store::presets::Preset {
                account: account.clone(),
                name: name.clone(),
                form: lc_proto::encode(form),
            }),
            None => deleted.push((account.clone(), name.clone())),
        }
    }
    (saved, deleted)
}

#[cfg(test)]
mod tests {
    use lc_proto::form::{Kind, Part, PartId, Primitive};
    use lc_proto::{ClientId, Inbound, Outbound, PROTOCOL_VERSION};

    use super::*;
    use crate::journal::Memory;
    use crate::server::Server;
    use crate::testing::Broker;
    use crate::ticket::Trusted;
    use crate::transport::Loopback;

    const SHARD: &str = "shard-1";

    fn mind() -> Form {
        Form {
            parts: vec![Part {
                id: PartId(0),
                kind: Kind::Mind,
                primitive: Primitive::Ellipsoid { axes: [1.0; 3] },
                volume_m3: 10.0,
                placement: None,
            }],
        }
    }

    fn trusting(broker: &Broker) -> Server<Memory> {
        let mut server = Server::new(Memory::default(), 0, 1);
        let mut trusted = Trusted::new(SHARD);
        assert_eq!(trusted.learn(&broker.jwks()), 1);
        server.trust(trusted);
        server
    }

    async fn says(server: &mut Server<Memory>, wire: &mut Loopback, from: ClientId, message: Inbound) -> Vec<Outbound> {
        wire.client_says(from, message);
        server.tick(wire).await.unwrap();
        wire.take(from)
    }

    async fn signs_in(server: &mut Server<Memory>, wire: &mut Loopback, from: ClientId, ticket: String) -> Vec<Outbound> {
        says(server, wire, from, Inbound::Hello { protocol: PROTOCOL_VERSION, ticket }).await
    }

    fn presets(said: &[Outbound]) -> Option<Vec<Preset>> {
        said.iter().find_map(|m| match m {
            Outbound::Presets(presets) => Some(presets.clone()),
            _ => None,
        })
    }

    fn refused(said: &[Outbound]) -> Option<Refusal> {
        said.iter().find_map(|m| match m {
            Outbound::Refused { reason, .. } => Some(*reason),
            _ => None,
        })
    }

    fn save(name: &str, form: Form) -> Inbound {
        Inbound::SavePreset { name: name.into(), form }
    }

    #[tokio::test]
    async fn saving_and_deleting_each_answer_with_the_whole_list() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        let me = ClientId(1);

        let said = signs_in(&mut server, &mut wire, me, broker.mint("acct-1", SHARD, 60, "j1")).await;
        assert_eq!(presets(&said), Some(vec![]), "a new account is told it has none: {said:?}");

        let said = says(&mut server, &mut wire, me, save("Ring", mind())).await;
        assert_eq!(presets(&said).unwrap().len(), 1);
        let said = says(&mut server, &mut wire, me, save("Plate", Form::default())).await;
        let names: Vec<_> = presets(&said).unwrap().into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Plate", "Ring"]);

        let said = says(&mut server, &mut wire, me, save("Plate", mind())).await;
        let list = presets(&said).unwrap();
        assert_eq!(list.len(), 2, "the same name replaces");
        assert_eq!(list[0], Preset { name: "Plate".into(), form: mind() });

        let said = says(&mut server, &mut wire, me, Inbound::DeletePreset { name: "Ring".into() }).await;
        assert_eq!(presets(&said), Some(vec![Preset { name: "Plate".into(), form: mind() }]));

        // The same account on another socket.
        let said = signs_in(&mut server, &mut wire, ClientId(2), broker.mint("acct-1", SHARD, 60, "j2")).await;
        assert_eq!(presets(&said), Some(vec![Preset { name: "Plate".into(), form: mind() }]), "{said:?}");
    }

    #[tokio::test]
    async fn one_account_never_sees_another_s_presets() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        let (alice, bob) = (ClientId(1), ClientId(2));
        signs_in(&mut server, &mut wire, alice, broker.mint("acct-1", SHARD, 60, "j1")).await;
        signs_in(&mut server, &mut wire, bob, broker.mint("acct-2", SHARD, 60, "j2")).await;

        says(&mut server, &mut wire, alice, save("Plate", mind())).await;
        let said = says(&mut server, &mut wire, bob, save("Ring", Form::default())).await;
        assert_eq!(presets(&said), Some(vec![Preset { name: "Ring".into(), form: Form::default() }]));
        assert_eq!(presets(&wire.take(alice)), None, "bob's save was told to alice");

        // A delete of a name only the other account has leaves theirs alone.
        let said = says(&mut server, &mut wire, bob, Inbound::DeletePreset { name: "Plate".into() }).await;
        assert_eq!(presets(&said).unwrap().len(), 1);
        let said = signs_in(&mut server, &mut wire, ClientId(3), broker.mint("acct-1", SHARD, 60, "j3")).await;
        assert_eq!(presets(&said), Some(vec![Preset { name: "Plate".into(), form: mind() }]), "{said:?}");
    }

    #[tokio::test]
    async fn the_limits_are_refused_by_name_and_change_nothing() {
        let broker = Broker::new([1u8; 32]);
        let mut server = trusting(&broker);
        let mut wire = Loopback::new();
        let me = ClientId(1);
        signs_in(&mut server, &mut wire, me, broker.mint("acct-1", SHARD, 60, "j1")).await;

        let long = "x".repeat(PRESET_NAME_LIMIT + 1);
        // Multi-byte, so a count of characters rather than bytes would let it through.
        let wide = "é".repeat(PRESET_NAME_LIMIT / 2 + 1);
        for name in ["", long.as_str(), wide.as_str()] {
            let said = says(&mut server, &mut wire, me, save(name, mind())).await;
            assert_eq!(refused(&said), Some(Refusal::PresetName), "{name:?}");
            assert_eq!(presets(&said), None);
        }
        let said = says(&mut server, &mut wire, me, save(&"x".repeat(PRESET_NAME_LIMIT), mind())).await;
        assert_eq!(presets(&said).unwrap().len(), 1, "exactly the limit is a name");

        let mut huge = mind();
        huge.parts = vec![huge.parts[0]; MAX_PARTS + 1];
        let said = says(&mut server, &mut wire, me, save("Huge", huge)).await;
        assert_eq!(
            refused(&said),
            Some(Refusal::Form(FormFault::TooManyParts { found: MAX_PARTS as u32 + 1 }))
        );

        for k in 1..MAX_PRESETS {
            server.presets.save("acct-1", format!("p{k:02}"), Form::default()).unwrap();
        }
        let said = says(&mut server, &mut wire, me, save("One more", mind())).await;
        assert_eq!(refused(&said), Some(Refusal::TooManyPresets));
        let said = says(&mut server, &mut wire, me, save("p01", mind())).await;
        assert_eq!(presets(&said).unwrap().len(), MAX_PRESETS, "replacing one when full is not a 65th");
    }

    #[test]
    fn a_checkpoint_writes_the_last_change_to_each_name_and_reads_back() {
        let mut shard = Presets::default();
        shard.save("acct-1", "Plate".into(), mind()).unwrap();
        shard.save("acct-1", "Ring".into(), mind()).unwrap();
        shard.save("acct-2", "Ring".into(), Form::default()).unwrap();
        shard.delete("acct-1", "Ring");
        let changes = shard.take_dirty();
        assert!(shard.take_dirty().is_empty(), "asking clears it");

        let (saved, deleted) = rows(&changes);
        assert_eq!(deleted, [("acct-1".to_owned(), "Ring".to_owned())]);
        assert_eq!(saved.len(), 2);

        let mut restarted = Presets::default();
        assert!(restarted.adopt(saved).is_empty());
        assert_eq!(restarted.for_account("acct-1"), shard.for_account("acct-1"));
        assert_eq!(restarted.for_account("acct-2"), shard.for_account("acct-2"));

        shard.redirty(&changes);
        assert_eq!(shard.take_dirty().len(), 3, "a failed write is written again");
    }
}
