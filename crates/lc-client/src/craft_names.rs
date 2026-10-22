//! What each craft is called on this screen, and which are borrowing another's name.
//!
//! A craft is called what this ship named it, else what a faction it holds a key to named it,
//! else what anybody else named it, else what it says it is. A craft whose name stands only on
//! its own word, when another craft already goes by that name on a better one, is a claimant,
//! and is shown as one. See `lightcone/docs/23-factions.md` §Craft that are not ours.

use std::collections::BTreeMap;

use lc_proto::{Scope, ShipId, Writing};
use lc_world::knowledge::{Knowledge, Subject, Witness};

/// On whose word a craft's name stands, weakest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Word {
    /// Its own: what it says it is.
    Claim,
    /// Somebody else's, outside any faction this ship holds.
    Other,
    /// Under a faction key this ship holds.
    Vouched,
    /// This ship's.
    Own,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub name: String,
    pub word: Word,
    /// The craft whose name this one borrows, when it does.
    pub claimant_of: Option<ShipId>,
}

impl Label {
    pub fn text(&self, id: ShipId) -> String {
        match self.claimant_of {
            Some(_) => format!("{} (claimant {})", self.name, id.tag()),
            None => self.name.clone(),
        }
    }
}

/// What a craft last said it is called, in what this ship holds of it.
pub fn claimed(knowledge: &Knowledge, id: ShipId) -> Option<String> {
    let thread = knowledge.file(Subject::Craft(id.0))?.thread();
    // Its newest whether or not somebody's name now stands over it: a claim is what it says.
    thread
        .ordered()
        .into_iter()
        .filter(|(_, held)| held.entry.author == Witness(id.0 as u64) && matches!(held.entry.writing, Writing::Name(_)))
        .max_by(|a, b| a.1.entry.written_s.total_cmp(&b.1.entry.written_s))
        .and_then(|(_, held)| match &held.entry.writing {
            Writing::Name(name) => Some(name.clone()),
            Writing::Note(_) | Writing::Was(_) => None,
        })
}

/// A craft nobody has named, shown without its id.
pub fn unnamed(id: ShipId) -> String {
    format!("ship {}", id.tag())
}

/// Every named craft among `ships` and the ones `knowledge` holds a thread on.
pub fn labels(knowledge: &Knowledge, ships: impl IntoIterator<Item = ShipId>) -> BTreeMap<ShipId, Label> {
    let owner = knowledge.owner;
    let mut ids: Vec<ShipId> = ships.into_iter().chain(knowledge.crafts().map(ShipId)).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut named: BTreeMap<ShipId, (Label, f64)> = BTreeMap::new();
    for id in ids {
        let Some(file) = knowledge.file(Subject::Craft(id.0)) else { continue };
        let Some((_, held)) = file.thread().standing(owner) else { continue };
        let Writing::Name(name) = &held.entry.writing else { continue };
        let word = if held.entry.author == owner {
            Word::Own
        } else if matches!(held.entry.scope, Scope::Faction(_)) {
            Word::Vouched
        } else if held.entry.author == Witness(id.0 as u64) {
            Word::Claim
        } else {
            Word::Other
        };
        named.insert(id, (Label { name: name.clone(), word, claimant_of: None }, held.learned_s()));
    }
    // Who a name belongs to: the strongest word, then whoever was called it first.
    let mut holders: BTreeMap<String, (ShipId, Word, f64)> = BTreeMap::new();
    for (id, (label, learned_s)) in &named {
        let key = label.name.trim().to_lowercase();
        let better = |held: &(ShipId, Word, f64)| (label.word, -learned_s, -id.0) > (held.1, -held.2, -held.0.0);
        if holders.get(&key).is_none_or(better) {
            holders.insert(key, (*id, label.word, *learned_s));
        }
    }
    named
        .into_iter()
        .map(|(id, (mut label, _))| {
            let holder = holders.get(&label.name.trim().to_lowercase()).map(|h| h.0);
            if label.word == Word::Claim && holder.is_some_and(|h| h != id) {
                label.claimant_of = holder;
            }
            (id, label)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use lc_proto::{Claim, FactionId};

    use super::*;

    fn claim(k: &mut Knowledge, ship: i64, name: &str, at_s: f64) {
        let hop = lc_world::knowledge::Hop { from: Witness(ship as u64), to: k.owner, sent_s: at_s, received_s: at_s };
        k.heard_claim(Witness(ship as u64), &Claim { name: name.into(), written_s: at_s, parents: vec![] }, vec![hop]);
    }

    #[test]
    fn a_second_craft_claiming_a_known_name_is_a_claimant_of_the_first() {
        let mut k = Knowledge::new(Witness(1));
        claim(&mut k, 7, "Wren", 10.0);
        claim(&mut k, 9, "Wren", 20.0);
        let labels = labels(&k, []);
        assert_eq!(labels[&ShipId(7)].claimant_of, None);
        assert_eq!(labels[&ShipId(9)].claimant_of, Some(ShipId(7)));
        assert_eq!(labels[&ShipId(9)].text(ShipId(9)), format!("Wren (claimant {})", ShipId(9).tag()));
    }

    #[test]
    fn a_name_this_ship_gave_holds_it_against_an_earlier_claim() {
        let mut k = Knowledge::new(Witness(1));
        claim(&mut k, 9, "Wren", 1.0);
        claim(&mut k, 7, "Wren", 10.0);
        k.write(Subject::Craft(7), Writing::Name("Wren".into()), Scope::Own, 11.0);
        let labels = labels(&k, []);
        assert_eq!(labels[&ShipId(7)].word, Word::Own);
        assert_eq!(labels[&ShipId(9)].claimant_of, Some(ShipId(7)), "the one this ship vouches for keeps the name");
    }

    #[test]
    fn a_faction_name_is_vouched_and_beats_a_claim() {
        let mut k = Knowledge::new(Witness(1));
        claim(&mut k, 7, "Mallory", 1.0);
        let entry = lc_world::knowledge::thread::Entry {
            author: Witness(3),
            written_s: 2.0,
            parents: vec![],
            scope: Scope::Faction(FactionId(5)),
            writing: Writing::Name("Wren".into()),
        };
        k.heard_entry(Subject::Craft(7), lc_world::knowledge::thread::Held { entry, lineage: vec![] });
        let labels = labels(&k, []);
        assert_eq!(labels[&ShipId(7)].name, "Wren");
        assert_eq!(labels[&ShipId(7)].word, Word::Vouched);
    }

    #[test]
    fn an_unnamed_craft_is_its_tag_not_its_id() {
        assert!(!unnamed(ShipId(2001)).contains("2001"));
        assert_eq!(unnamed(ShipId(2001)), unnamed(ShipId(2001)));
    }
}
