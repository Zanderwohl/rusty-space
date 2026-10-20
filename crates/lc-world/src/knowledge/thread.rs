//! What craft have written about a subject: notes and chosen names, appended by anyone, merged by
//! union, with no agreed order.
//!
//! Each entry names the entries its author held on the subject when writing, so the thread is a
//! graph, not a list. Two entries neither of which is the other's ancestor were each written
//! without knowing of the other, which is what light delay makes of a conversation. See
//! `lightcone/docs/23-factions.md` §Notes.

use std::collections::{BTreeMap, BTreeSet};

use lc_proto::{FactionId, Scope, Writing};
use serde::{Deserialize, Serialize};

use super::{Lineage, Witness, learned_s};

/// An entry by its content, so one entry is recognized however many routes it arrives by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntryId(pub u64);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub author: Witness,
    /// Coordinate seconds, at the author.
    pub written_s: f64,
    /// The thread's heads as the author held them, sorted.
    pub parents: Vec<EntryId>,
    pub scope: Scope,
    pub writing: Writing,
}

impl Entry {
    /// FNV-1a over the postcard encoding, which is fixed by the protocol's goldens.
    pub fn id(&self) -> EntryId {
        let bytes = lc_proto::encode(&(self.author.0, self.written_s.to_bits(), &self.parents, self.scope, &self.writing));
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        EntryId(h)
    }
}

impl Entry {
    /// What a craft says it is called, as a transmission carries it. `None` for a note.
    pub fn claim(&self) -> Option<lc_proto::Claim> {
        let Writing::Name(name) = &self.writing else { return None };
        Some(lc_proto::Claim { name: name.clone(), written_s: self.written_s, parents: self.parents.iter().map(|p| p.0).collect() })
    }

    /// The entry a claim heard from `sender` is: the sender's own, so every receiver files the
    /// same one.
    pub fn claimed(sender: Witness, claim: &lc_proto::Claim) -> Self {
        Self {
            author: sender,
            written_s: claim.written_s,
            parents: claim.parents.iter().copied().map(EntryId).collect(),
            scope: Scope::Open,
            writing: Writing::Name(claim.name.clone()),
        }
    }
}

/// An entry as one craft holds it, with how it got there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Held {
    pub entry: Entry,
    pub lineage: Lineage,
}

impl Held {
    pub fn learned_s(&self) -> f64 {
        learned_s(&self.lineage, self.entry.written_s)
    }
}

/// What a report is sealed to, for which entries it may carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seal {
    /// Open, or sealed to one craft's ship key.
    Plain,
    Faction(FactionId),
    /// The craft's own copy, for its own client: everything, its private entries included.
    Whole,
}

impl Seal {
    pub fn admits(self, scope: Scope) -> bool {
        match (self, scope) {
            (Seal::Whole, _) => true,
            (_, Scope::Own) => false,
            (_, Scope::Open) => true,
            (seal, Scope::Faction(key)) => seal == Seal::Faction(key),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Thread {
    held: BTreeMap<EntryId, Held>,
}

impl Thread {
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    pub fn get(&self, id: EntryId) -> Option<&Held> {
        self.held.get(&id)
    }

    /// Hold an entry. False when it was already held; the first route it came by is kept.
    pub fn add(&mut self, held: Held) -> bool {
        let id = held.entry.id();
        if self.held.contains_key(&id) {
            return false;
        }
        self.held.insert(id, held);
        true
    }

    /// Entries no held entry names as a parent: what a new entry's parents are.
    pub fn heads(&self) -> Vec<EntryId> {
        let named: BTreeSet<EntryId> = self.held.values().flat_map(|h| h.entry.parents.iter().copied()).collect();
        self.held.keys().copied().filter(|id| !named.contains(id)).collect()
    }

    /// Whether `later`'s author held `earlier` when writing it, directly or through other entries.
    /// Only through entries held here: an ancestor reached through one this craft lacks is not
    /// seen, so the answer can be no when the truth is yes, never the other way round.
    pub fn knew(&self, later: EntryId, earlier: EntryId) -> bool {
        let mut seen = BTreeSet::new();
        let mut open = vec![later];
        while let Some(id) = open.pop() {
            let Some(held) = self.held.get(&id) else { continue };
            for parent in &held.entry.parents {
                if *parent == earlier {
                    return true;
                }
                if seen.insert(*parent) {
                    open.push(*parent);
                }
            }
        }
        false
    }

    /// Neither written knowing of the other.
    pub fn concurrent(&self, a: EntryId, b: EntryId) -> bool {
        a != b && !self.knew(a, b) && !self.knew(b, a)
    }

    /// Parents before children; otherwise by when written, then by id so every holder agrees.
    pub fn ordered(&self) -> Vec<(EntryId, &Held)> {
        let mut out: Vec<(EntryId, &Held)> = Vec::with_capacity(self.held.len());
        let mut placed: BTreeSet<EntryId> = BTreeSet::new();
        let mut waiting: Vec<(EntryId, &Held)> = self.held.iter().map(|(id, h)| (*id, h)).collect();
        waiting.sort_by(|a, b| a.1.entry.written_s.total_cmp(&b.1.entry.written_s).then(a.0.cmp(&b.0)));
        // A parent not held here never arrives, so it does not hold its children back.
        let ready = |held: &Held, placed: &BTreeSet<EntryId>| {
            held.entry.parents.iter().all(|p| placed.contains(p) || !self.held.contains_key(p))
        };
        while !waiting.is_empty() {
            let at = waiting.iter().position(|(_, h)| ready(h, &placed)).unwrap_or(0);
            let (id, held) = waiting.remove(at);
            placed.insert(id);
            out.push((id, held));
        }
        out
    }

    /// The name that stands, and on whose word. See [`Thread::standing`].
    pub fn name(&self, owner: Witness) -> Option<&str> {
        self.standing(owner).and_then(|(_, held)| match &held.entry.writing {
            Writing::Name(name) => Some(name.as_str()),
            Writing::Note(_) => None,
        })
    }

    /// Of the names no later name supersedes: `owner`'s own, else one a faction vouched for, else
    /// anybody's; the most recently written within each. A faction-scoped entry is held only by
    /// way of a key held, so holding one is the vouching.
    pub fn standing(&self, owner: Witness) -> Option<(EntryId, &Held)> {
        let rank = |held: &Held| (held.entry.author == owner, matches!(held.entry.scope, Scope::Faction(_)));
        self.standing_names().into_iter().max_by(|a, b| {
            rank(a.1)
                .cmp(&rank(b.1))
                .then(a.1.entry.written_s.total_cmp(&b.1.entry.written_s))
                .then(a.0.cmp(&b.0))
        })
    }

    /// Names no later name supersedes. More than one is a disagreement nobody has settled.
    pub fn standing_names(&self) -> Vec<(EntryId, &Held)> {
        let names: Vec<(EntryId, &Held)> =
            self.held.iter().filter(|(_, h)| matches!(h.entry.writing, Writing::Name(_))).map(|(id, h)| (*id, h)).collect();
        names.iter().copied().filter(|(id, _)| !names.iter().any(|(other, _)| self.knew(*other, *id))).collect()
    }

    /// What a report under `seal` may carry.
    pub fn sealed(&self, seal: Seal) -> impl Iterator<Item = &Held> {
        self.held.values().filter(move |h| seal.admits(h.entry.scope))
    }

    /// When each entry was learned, for the backlog.
    pub fn learned_times(&self) -> impl Iterator<Item = f64> + '_ {
        self.held.values().map(Held::learned_s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(author: u64, written_s: f64, parents: &[EntryId], writing: Writing) -> Held {
        let mut parents = parents.to_vec();
        parents.sort();
        Held { entry: Entry { author: Witness(author), written_s, parents, scope: Scope::Open, writing }, lineage: Vec::new() }
    }

    fn note(text: &str) -> Writing {
        Writing::Note(text.into())
    }

    #[test]
    fn an_entry_is_the_same_entry_however_it_arrives() {
        let mut thread = Thread::default();
        let first = entry(1, 10.0, &[], note("hot"));
        assert!(thread.add(first.clone()));
        let relayed = Held { lineage: vec![super::super::Hop { from: Witness(1), to: Witness(2), sent_s: 11.0, received_s: 12.0 }], ..first };
        assert!(!thread.add(relayed));
        assert_eq!(thread.heads().len(), 1);
    }

    #[test]
    fn two_authors_who_had_not_heard_each_other_are_concurrent() {
        let mut thread = Thread::default();
        let root = entry(1, 0.0, &[], note("found it"));
        let root_id = root.entry.id();
        thread.add(root);
        let near = entry(2, 5.0, &[root_id], note("it flickers"));
        let far = entry(3, 4.0, &[root_id], note("it is a binary"));
        let (near_id, far_id) = (near.entry.id(), far.entry.id());
        thread.add(near);
        thread.add(far);
        assert!(thread.concurrent(near_id, far_id));
        assert!(thread.knew(near_id, root_id));
        let reply = entry(1, 9.0, &[near_id, far_id], note("both, then"));
        let reply_id = reply.entry.id();
        thread.add(reply);
        assert_eq!(thread.heads(), vec![reply_id]);
        assert!(!thread.concurrent(reply_id, far_id));
    }

    #[test]
    fn parents_come_first_whatever_the_clocks_say() {
        let mut thread = Thread::default();
        // Written "later" by a clock that runs behind the replier's.
        let first = entry(1, 50.0, &[], note("first"));
        let first_id = first.entry.id();
        thread.add(first);
        thread.add(entry(2, 10.0, &[first_id], note("reply")));
        let order: Vec<&Writing> = thread.ordered().into_iter().map(|(_, h)| &h.entry.writing).collect();
        assert_eq!(order, vec![&note("first"), &note("reply")]);
    }

    #[test]
    fn a_rename_supersedes_the_names_its_author_had_seen_and_a_disagreement_stands() {
        let mut thread = Thread::default();
        let kettle = entry(1, 0.0, &[], Writing::Name("Kettle".into()));
        let kettle_id = kettle.entry.id();
        thread.add(kettle);
        let hearth = entry(2, 5.0, &[kettle_id], Writing::Name("Hearth".into()));
        thread.add(hearth);
        assert_eq!(thread.name(Witness(9)), Some("Hearth"));
        assert_eq!(thread.standing_names().len(), 1);

        // Ship 1, not having heard of Hearth, renames again.
        let spout = entry(1, 6.0, &[kettle_id], Writing::Name("Spout".into()));
        thread.add(spout);
        assert_eq!(thread.standing_names().len(), 2, "two names, neither aware of the other");
        assert_eq!(thread.name(Witness(2)), Some("Hearth"), "a craft's own stands for it");
        assert_eq!(thread.name(Witness(9)), Some("Spout"), "otherwise the latest written");
    }

    #[test]
    fn a_name_a_faction_gave_beats_a_newer_one_from_anybody() {
        let mut thread = Thread::default();
        let mut vouched = entry(2, 1.0, &[], Writing::Name("Wren".into()));
        vouched.entry.scope = Scope::Faction(FactionId(5));
        thread.add(vouched);
        thread.add(entry(3, 9.0, &[], Writing::Name("Not Wren".into())));
        assert_eq!(thread.name(Witness(1)), Some("Wren"));
        thread.add(entry(1, 0.5, &[], Writing::Name("My Wren".into())));
        assert_eq!(thread.name(Witness(1)), Some("My Wren"), "and one's own beats both");
    }

    #[test]
    fn a_report_carries_only_what_its_seal_admits() {
        let mut thread = Thread::default();
        let mut scoped = |scope, text: &str| {
            let mut held = entry(1, 0.0, &[], note(text));
            held.entry.scope = scope;
            thread.add(held);
        };
        scoped(Scope::Own, "mine");
        scoped(Scope::Open, "everyone");
        scoped(Scope::Faction(FactionId(5)), "wake");
        let carried = |seal| thread.sealed(seal).count();
        assert_eq!(carried(Seal::Plain), 1);
        assert_eq!(carried(Seal::Faction(FactionId(5))), 2);
        assert_eq!(carried(Seal::Faction(FactionId(6))), 1);
    }

    #[test]
    fn an_entry_whose_parent_never_arrived_is_still_placed() {
        let mut thread = Thread::default();
        thread.add(entry(1, 3.0, &[EntryId(42)], note("re: something private")));
        thread.add(entry(2, 1.0, &[], note("unrelated")));
        assert_eq!(thread.ordered().len(), 2);
    }
}
