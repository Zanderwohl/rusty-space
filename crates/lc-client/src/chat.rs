//! What has been said, to whom, and whether it got there.
//!
//! One conversation per craft, and a conversation is not a chat log: half of it is still in
//! flight. A message this ship sent exists the moment it is sent and arrives years later, and
//! there is nothing at either end that can notice it landing — so the only evidence a message
//! got through is the **acknowledgement** that rides back with the next one, naming it by the
//! identifier the server minted. [`Line::delivered`] is that, and it is the whole of it.
//!
//! No engine here and no socket: `crate::uplink` folds the wire into this, and
//! `crate::panels` draws it.

use std::collections::BTreeMap;

use lc_proto::{Said, ShipId, Spoken};

/// One message, from either end.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    /// The event the server minted for it, which is what an acknowledgement names.
    pub event_id: i64,
    pub mine: bool,
    /// A key offer rather than something somebody typed.
    pub key: bool,
    pub sealed: bool,
    /// `None` for a sealed message this ship is not the addressee of. It was heard and cannot
    /// be read, which is a different thing from not having been heard.
    pub body: Option<String>,
    /// What this message said it had received, newest last.
    pub acks: Vec<i64>,
    /// Coordinate seconds it was transmitted.
    pub sent_s: f64,
    /// Coordinate seconds its light landed here. `None` for one this ship sent — a sender never
    /// hears its own signal, and never learns when it arrived.
    pub arrive_s: Option<f64>,
}

/// Everything said to and by one craft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub name: String,
    /// In the order this ship learnt of them, which for a conversation across light delay is
    /// not the order they were sent in: a reply can be composed before the message it crosses.
    pub lines: Vec<Line>,
}

impl Conversation {
    /// Whether a message this ship sent has been acknowledged by the other end.
    ///
    /// The only evidence there is. Nothing reports a delivery, because nothing at either end
    /// can observe one — the light either fell on an antenna or went past it, and the far end
    /// is the only place that knows which.
    pub fn delivered(&self, event_id: i64) -> bool {
        self.lines.iter().any(|line| !line.mine && line.acks.contains(&event_id))
    }

    /// The newest line, for a list that wants to show what a conversation is about.
    pub fn last(&self) -> Option<&Line> {
        self.lines.last()
    }
}

/// Every conversation this ship is in, and whose keys it holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chat {
    /// Keyed by the other craft's identifier, so the dropdown has a stable order that is not
    /// "whoever spoke last" — a list that reorders itself under a cursor is a list that gets
    /// misclicked.
    conversations: BTreeMap<i64, Conversation>,
    /// Whose public keys this ship holds, and can therefore seal a message to.
    ///
    /// Learnt when a key offer's **light arrives**, never when it is sent. The client's copy of
    /// the server's rule; the server refuses a sealed message either way, and this is what stops
    /// the interface from offering one it knows would be refused.
    keys: std::collections::BTreeSet<i64>,
}

impl Chat {
    /// Every conversation, by the craft it is with.
    pub fn conversations(&self) -> impl Iterator<Item = (ShipId, &Conversation)> {
        self.conversations.iter().map(|(id, c)| (ShipId(*id), c))
    }

    pub fn get(&self, with: ShipId) -> Option<&Conversation> {
        self.conversations.get(&with.0)
    }

    pub fn holds_key(&self, of: ShipId) -> bool {
        self.keys.contains(&of.0)
    }

    pub fn is_empty(&self) -> bool {
        self.conversations.is_empty()
    }

    /// Start a conversation with a craft nothing has been said to yet, so the window has
    /// somewhere to put the first message.
    pub fn open(&mut self, with: ShipId, name: &str) {
        let conversation = self.conversations.entry(with.0).or_default();
        if conversation.name.is_empty() {
            conversation.name = name.to_string();
        }
    }

    /// The transcript the server hands over on signing in. Replaces whatever is here.
    ///
    /// Wholesale rather than merged, because it is the authority's copy of a record this client
    /// only ever had a partial view of — and because a merge would have to decide what to do
    /// about a line in both, which is a question with no interesting answer.
    pub fn restore(&mut self, messages: Vec<Said>, keys: Vec<ShipId>) {
        self.conversations.clear();
        self.keys = keys.into_iter().map(|k| k.0).collect();
        for said in messages {
            let conversation = self.conversations.entry(said.with.0).or_default();
            conversation.name = said.with_name;
            conversation.lines.push(Line {
                event_id: said.event_id,
                mine: said.mine,
                key: said.key,
                sealed: said.sealed,
                body: said.body,
                acks: said.acks,
                sent_s: said.sent_t as f64 * 1.0e-6,
                arrive_s: said.arrive_t.map(|t| t as f64 * 1.0e-6),
            });
        }
    }

    /// A transmission whose light has just landed.
    ///
    /// `name` is what this ship currently calls the sender, when it can see one. A craft that
    /// has since gone out of sight keeps the name the conversation already had rather than
    /// reverting to its number, because a contact list is not the only place a name comes from.
    pub fn received(
        &mut self,
        from: ShipId,
        name: Option<&str>,
        event_id: i64,
        spoken: Spoken,
        key: bool,
        sent_s: f64,
        arrive_s: f64,
    ) {
        if key {
            self.keys.insert(from.0);
        }
        let conversation = self.conversations.entry(from.0).or_default();
        if let Some(name) = name {
            conversation.name = name.to_string();
        } else if conversation.name.is_empty() {
            conversation.name = format!("ship {}", from.0);
        }
        // Idempotent, because a reconnection replays: `ResumeFrom` winds the delivery cursor
        // back and everything since arrives a second time.
        if conversation.lines.iter().any(|line| line.event_id == event_id) {
            return;
        }
        conversation.lines.push(Line {
            event_id,
            mine: false,
            key,
            sealed: spoken.sealed,
            body: spoken.body,
            acks: spoken.acks,
            sent_s,
            arrive_s: Some(arrive_s),
        });
    }

    /// A message this ship has just had accepted. Recorded against the identifier the server
    /// minted, which is the only thing an acknowledgement will ever name it by.
    pub fn sent(
        &mut self,
        to: ShipId,
        name: Option<&str>,
        event_id: i64,
        body: Option<String>,
        sealed: bool,
        key: bool,
        sent_s: f64,
    ) {
        let conversation = self.conversations.entry(to.0).or_default();
        if conversation.name.is_empty() {
            conversation.name = name.map(str::to_string).unwrap_or_else(|| format!("ship {}", to.0));
        }
        if conversation.lines.iter().any(|line| line.event_id == event_id) {
            return;
        }
        conversation.lines.push(Line {
            event_id,
            mine: true,
            key,
            sealed,
            body,
            acks: Vec::new(),
            sent_s,
            arrive_s: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(to: i64, body: Option<&str>, sealed: bool, acks: Vec<i64>) -> Spoken {
        Spoken { to, sealed, body: body.map(Into::into), acks }
    }

    /// The only delivery report there is, and it is a report about a *specific* message.
    #[test]
    fn a_message_is_delivered_only_when_the_far_end_names_it() {
        let mut chat = Chat::default();
        chat.sent(ShipId(7), Some("Ada"), 100, Some("first".into()), false, false, 1.0);
        chat.sent(ShipId(7), Some("Ada"), 101, Some("second".into()), false, false, 2.0);
        let with = |chat: &Chat| chat.get(ShipId(7)).cloned().unwrap();
        assert!(!with(&chat).delivered(100));

        chat.received(ShipId(7), Some("Ada"), 200, spoken(1, Some("got it"), false, vec![100]), false, 9.0, 12.0);
        assert!(with(&chat).delivered(100));
        assert!(!with(&chat).delivered(101), "an unnamed message was reported delivered");
    }

    /// A reconnection replays the delivery stream, so the same arrival can be folded twice. It
    /// must not become two lines.
    #[test]
    fn the_same_arrival_folded_twice_is_one_line() {
        let mut chat = Chat::default();
        let said = spoken(1, Some("hello"), false, Vec::new());
        chat.received(ShipId(7), Some("Ada"), 200, said.clone(), false, 1.0, 4.0);
        chat.received(ShipId(7), Some("Ada"), 200, said, false, 1.0, 4.0);
        assert_eq!(chat.get(ShipId(7)).unwrap().lines.len(), 1);
    }

    /// A key is held from the moment its offer *lands*, and the client learns it the same way
    /// the server does: by the arrival, not by the sending.
    #[test]
    fn a_key_offer_arriving_is_what_puts_a_key_in_the_ring() {
        let mut chat = Chat::default();
        assert!(!chat.holds_key(ShipId(7)));
        chat.received(ShipId(7), None, 300, spoken(1, Some(""), false, Vec::new()), true, 1.0, 5.0);
        assert!(chat.holds_key(ShipId(7)));
        // And sending one away teaches this ship nothing about anybody.
        chat.sent(ShipId(8), None, 301, None, false, true, 6.0);
        assert!(!chat.holds_key(ShipId(8)), "offering a key taught the offerer something");
    }

    /// A sealed message somebody else's is kept, with no body. It was heard; it cannot be read.
    #[test]
    fn a_sealed_message_for_somebody_else_is_a_line_with_nothing_in_it() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), None, 400, spoken(99, None, true, Vec::new()), false, 1.0, 3.0);
        let line = &chat.get(ShipId(7)).unwrap().lines[0];
        assert!(line.sealed);
        assert_eq!(line.body, None);
        assert_eq!(line.arrive_s, Some(3.0), "it still arrived");
    }

    /// A contact that has gone out of sight keeps the name it had. Falling back to its number
    /// would rename a conversation the moment the other ship left the system.
    #[test]
    fn a_name_survives_the_contact_that_supplied_it() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), Some("Ada"), 500, spoken(1, Some("here"), false, vec![]), false, 1.0, 2.0);
        chat.received(ShipId(7), None, 501, spoken(1, Some("still here"), false, vec![]), false, 3.0, 9.0);
        assert_eq!(chat.get(ShipId(7)).unwrap().name, "Ada");
    }

    #[test]
    fn a_restored_transcript_replaces_rather_than_merges() {
        let mut chat = Chat::default();
        chat.sent(ShipId(7), None, 1, Some("stale".into()), false, false, 0.0);
        chat.restore(
            vec![Said {
                event_id: 9,
                with: ShipId(7),
                with_name: "Ada".into(),
                mine: false,
                key: false,
                sealed: false,
                body: Some("from the store".into()),
                acks: vec![1],
                sent_t: 1_000_000,
                arrive_t: Some(4_000_000),
            }],
            vec![ShipId(7)],
        );
        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1);
        assert_eq!(conversation.name, "Ada");
        assert_eq!(conversation.lines[0].sent_s, 1.0);
        assert!(chat.holds_key(ShipId(7)));
    }
}
