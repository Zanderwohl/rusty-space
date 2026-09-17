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
    /// Every event this message was transmitted as, oldest first.
    ///
    /// More than one when it was resent. A resend is a second pulse of light and a second
    /// event — it really happened — but it is not a second thing somebody said, so it joins the
    /// line it repeats rather than making a new one. An acknowledgement of *any* of them
    /// acknowledges the message.
    pub event_ids: Vec<i64>,
    /// Which message this is, across those transmissions. See [`lc_proto::MessageKey`].
    pub idem: lc_proto::MessageKey,
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
    ///
    /// Any transmission of it counts. A resent message that is acknowledged by its second copy
    /// arrived, and which pulse of light got there is not a thing the sender needs to know.
    pub fn delivered(&self, line: &Line) -> bool {
        self.lines
            .iter()
            .filter(|reply| !reply.mine)
            .any(|reply| line.event_ids.iter().any(|id| reply.acks.contains(id)))
    }

    /// When this ship last learnt anything here, coordinate seconds. What the list sorts on.
    pub fn last_at(&self) -> f64 {
        self.lines.last().map_or(f64::NEG_INFINITY, |line| line.arrive_s.unwrap_or(line.sent_s))
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
    /// Every conversation, by the craft it is with, **most recently heard from first**.
    ///
    /// Recency rather than a stable key, because the list is how a player finds the
    /// conversation they are in the middle of. The cost is real and worth naming: a list that
    /// reorders itself is a list that can be misclicked, and it reorders exactly when a message
    /// lands. Nothing here moves under the cursor between frames unless the world moved it.
    pub fn conversations(&self) -> Vec<(ShipId, &Conversation)> {
        let mut out: Vec<(ShipId, &Conversation)> =
            self.conversations.iter().map(|(id, c)| (ShipId(*id), c)).collect();
        out.sort_by(|a, b| {
            b.1.last_at().total_cmp(&a.1.last_at()).then_with(|| a.0.0.cmp(&b.0.0))
        });
        out
    }

    /// Everything that was said in the open, from every conversation, oldest first.
    ///
    /// Not a conversation and deliberately not stored as one: it is a *view*, the answer to
    /// "what has been going on" rather than "what did we two say". A sealed message is absent
    /// whichever end it came from — including this ship's own, because a private message listed
    /// in a public log is a private message on a screen somebody can read over your shoulder.
    ///
    /// Each entry carries the craft the line is with, because a public log mixes them and a
    /// line with no name against it is a line nobody can answer.
    pub fn public(&self) -> Vec<(ShipId, &str, &Line)> {
        let mut out: Vec<(ShipId, &str, &Line)> = self
            .conversations
            .iter()
            .flat_map(|(id, c)| {
                c.lines
                    .iter()
                    .filter(|line| !line.sealed)
                    .map(move |line| (ShipId(*id), c.name.as_str(), line))
            })
            .collect();
        out.sort_by(|a, b| when(a.2).total_cmp(&when(b.2)));
        out
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
            // A backlog carries one entry per *transmission*, so a resent message arrives here
            // several times. They are one line with several events behind it.
            if let Some(line) = same_message(&mut conversation.lines, said.idem, said.mine) {
                if !line.event_ids.contains(&said.event_id) {
                    line.event_ids.push(said.event_id);
                }
                // The acknowledgements of the copy that actually landed are the ones worth
                // keeping, and a later copy can only know more.
                if said.acks.len() > line.acks.len() {
                    line.acks = said.acks;
                }
                continue;
            }
            conversation.lines.push(Line {
                event_ids: vec![said.event_id],
                idem: said.idem,
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
        // Idempotent twice over. A reconnection replays — `ResumeFrom` winds the delivery
        // cursor back and everything since arrives again — and a sender may have *resent*,
        // which is a genuinely different pulse of light carrying the same message.
        if conversation.lines.iter().any(|line| line.event_ids.contains(&event_id)) {
            return;
        }
        if !key
            && let Some(line) = same_message(&mut conversation.lines, spoken.idem, false)
        {
            line.event_ids.push(event_id);
            if spoken.acks.len() > line.acks.len() {
                line.acks = spoken.acks;
            }
            return;
        }
        conversation.lines.push(Line {
            event_ids: vec![event_id],
            idem: spoken.idem,
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
    #[allow(clippy::too_many_arguments)]
    pub fn sent(
        &mut self,
        to: ShipId,
        name: Option<&str>,
        event_id: i64,
        idem: lc_proto::MessageKey,
        body: Option<String>,
        sealed: bool,
        key: bool,
        sent_s: f64,
    ) {
        let conversation = self.conversations.entry(to.0).or_default();
        if conversation.name.is_empty() {
            conversation.name = name.map(str::to_string).unwrap_or_else(|| format!("ship {}", to.0));
        }
        if conversation.lines.iter().any(|line| line.event_ids.contains(&event_id)) {
            return;
        }
        // A resend joins the line it repeats. The line keeps the time it was *first* said,
        // because that is when the thing was said; the resends are how hard it was tried.
        if !key
            && let Some(line) = same_message(&mut conversation.lines, idem, true)
        {
            line.event_ids.push(event_id);
            return;
        }
        conversation.lines.push(Line {
            event_ids: vec![event_id],
            idem,
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

/// When this ship learnt of a line: the arrival for something heard, the sending for its own.
fn when(line: &Line) -> f64 {
    line.arrive_s.unwrap_or(line.sent_s)
}

/// The line a transmission belongs to, when this ship already has one for that message.
///
/// A key of zero means "not keyed" — a key offer, or a row written before there were keys — and
/// never matches, because folding all of those together would make every unkeyed message the
/// same message.
fn same_message(lines: &mut [Line], idem: lc_proto::MessageKey, mine: bool) -> Option<&mut Line> {
    if idem == 0 {
        return None;
    }
    lines.iter_mut().find(|line| line.mine == mine && line.idem == idem && !line.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(to: i64, body: Option<&str>, sealed: bool, acks: Vec<i64>) -> Spoken {
        Spoken { to, idem: 0, sealed, body: body.map(Into::into), acks }
    }

    fn keyed(to: i64, body: &str, idem: u64, acks: Vec<i64>) -> Spoken {
        Spoken { to, idem, sealed: false, body: Some(body.into()), acks }
    }

    /// A line by its position, for a test that wants to talk about one.
    fn line(chat: &Chat, with: i64, at: usize) -> Line {
        chat.get(ShipId(with)).expect("a conversation").lines[at].clone()
    }

    /// The only delivery report there is, and it is a report about a *specific* message.
    #[test]
    fn a_message_is_delivered_only_when_the_far_end_names_it() {
        let mut chat = Chat::default();
        chat.sent(ShipId(7), Some("Ada"), 100, 1, Some("first".into()), false, false, 1.0);
        chat.sent(ShipId(7), Some("Ada"), 101, 2, Some("second".into()), false, false, 2.0);
        let (first, second) = (line(&chat, 7, 0), line(&chat, 7, 1));
        assert!(!chat.get(ShipId(7)).unwrap().delivered(&first));

        chat.received(ShipId(7), Some("Ada"), 200, spoken(1, Some("got it"), false, vec![100]), false, 9.0, 12.0);
        let conversation = chat.get(ShipId(7)).unwrap();
        assert!(conversation.delivered(&first));
        assert!(!conversation.delivered(&second), "an unnamed message was reported delivered");
    }

    /// **A resend is the same message, said twice.** Two pulses of light and two events — both
    /// really happened — and one line, because one thing was said.
    #[test]
    fn a_resend_joins_the_line_it_repeats_rather_than_making_a_second() {
        let mut chat = Chat::default();
        chat.sent(ShipId(7), Some("Ada"), 100, 42, Some("are you there".into()), false, false, 1.0);
        chat.sent(ShipId(7), Some("Ada"), 108, 42, Some("are you there".into()), false, false, 9.0);

        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1, "a resend became a second message");
        assert_eq!(conversation.lines[0].event_ids, vec![100, 108]);
        // The time it was first said, because that is when the thing was said. The resends are
        // how hard it was tried.
        assert_eq!(conversation.lines[0].sent_s, 1.0);
    }

    /// An acknowledgement of *any* transmission acknowledges the message. Which pulse of light
    /// got there is not a thing the sender needs to know.
    #[test]
    fn acknowledging_the_second_copy_acknowledges_the_message() {
        let mut chat = Chat::default();
        chat.sent(ShipId(7), Some("Ada"), 100, 42, Some("again".into()), false, false, 1.0);
        chat.sent(ShipId(7), Some("Ada"), 108, 42, Some("again".into()), false, false, 9.0);
        chat.received(ShipId(7), Some("Ada"), 200, keyed(1, "heard the second one", 5, vec![108]), false, 10.0, 12.0);

        let sent = line(&chat, 7, 0);
        assert!(chat.get(ShipId(7)).unwrap().delivered(&sent));
    }

    /// The receiving end of the same rule: two arrivals carrying one key are one line.
    #[test]
    fn a_message_heard_twice_is_shown_once() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), Some("Ada"), 200, keyed(1, "hello", 77, vec![]), false, 1.0, 4.0);
        chat.received(ShipId(7), Some("Ada"), 209, keyed(1, "hello", 77, vec![]), false, 6.0, 9.0);
        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1, "a resend was shown twice");
        assert_eq!(conversation.lines[0].event_ids, vec![200, 209]);
    }

    /// A key of zero is "not keyed" — a key offer, or a row from before there were keys — and
    /// must never fold. Folding them all together would make every unkeyed message one message.
    #[test]
    fn unkeyed_messages_never_fold_into_each_other() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), None, 200, spoken(1, Some("one"), false, vec![]), false, 1.0, 2.0);
        chat.received(ShipId(7), None, 201, spoken(1, Some("two"), false, vec![]), false, 3.0, 4.0);
        assert_eq!(chat.get(ShipId(7)).unwrap().lines.len(), 2);
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
        chat.sent(ShipId(8), None, 301, 0, None, false, true, 6.0);
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

    /// **The public log is a view, and encryption is what keeps something out of it** — from
    /// either end. This ship's own sealed message is as absent as anybody else's, because a
    /// private message in a public log is one somebody can read over your shoulder.
    #[test]
    fn the_public_log_holds_what_was_said_in_the_open_and_nothing_else() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), Some("Ada"), 200, spoken(1, Some("in the open"), false, vec![]), false, 1.0, 2.0);
        chat.received(ShipId(7), Some("Ada"), 201, spoken(99, None, true, vec![]), false, 3.0, 4.0);
        chat.sent(ShipId(8), Some("Bry"), 300, 5, Some("mine, open".into()), false, false, 5.0);
        chat.sent(ShipId(8), Some("Bry"), 301, 6, Some("mine, private".into()), true, false, 6.0);

        let public = chat.public();
        let bodies: Vec<&str> =
            public.iter().filter_map(|(_, _, l)| l.body.as_deref()).collect();
        assert_eq!(bodies, vec!["in the open", "mine, open"]);
        assert!(public.iter().all(|(_, _, l)| !l.sealed));
        // And each line still says who it is with, which a mixed log needs to be answerable.
        assert_eq!(public[0].0, ShipId(7));
        assert_eq!(public[1].0, ShipId(8));
    }

    /// Oldest first, by when *this ship* learnt of each — which for a conversation across light
    /// delay is not the order they were sent in.
    #[test]
    fn the_public_log_is_ordered_by_when_this_ship_learnt_of_each() {
        let mut chat = Chat::default();
        // Sent early, heard late: a long crossing.
        chat.received(ShipId(7), Some("Ada"), 200, spoken(1, Some("slow"), false, vec![]), false, 1.0, 90.0);
        chat.sent(ShipId(8), Some("Bry"), 300, 5, Some("quick".into()), false, false, 50.0);
        let order: Vec<&str> = chat.public().iter().filter_map(|(_, _, l)| l.body.as_deref()).collect();
        assert_eq!(order, vec!["quick", "slow"], "ordered by sending, not by learning");
    }

    /// The list puts whoever spoke last at the top, which is how a player finds the
    /// conversation they are in the middle of.
    #[test]
    fn conversations_are_listed_most_recently_heard_first() {
        let mut chat = Chat::default();
        chat.sent(ShipId(7), Some("Ada"), 100, 1, Some("early".into()), false, false, 1.0);
        chat.sent(ShipId(8), Some("Bry"), 200, 2, Some("later".into()), false, false, 5.0);
        let order: Vec<ShipId> = chat.conversations().into_iter().map(|(id, _)| id).collect();
        assert_eq!(order, vec![ShipId(8), ShipId(7)]);

        // And it moves when something lands.
        chat.received(ShipId(7), Some("Ada"), 300, spoken(1, Some("back"), false, vec![]), false, 6.0, 9.0);
        let order: Vec<ShipId> = chat.conversations().into_iter().map(|(id, _)| id).collect();
        assert_eq!(order, vec![ShipId(7), ShipId(8)]);
    }

    #[test]
    fn a_restored_transcript_replaces_rather_than_merges() {
        let mut chat = Chat::default();
        chat.sent(ShipId(7), None, 1, 9, Some("stale".into()), false, false, 0.0);
        chat.restore(
            vec![Said {
                event_id: 9,
                idem: 4,
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

    /// A backlog carries one entry per *transmission*, so a resent message comes back several
    /// times and has to fold on the way in, exactly as it did when it was live.
    #[test]
    fn a_restored_transcript_folds_a_message_that_was_resent() {
        let mut chat = Chat::default();
        let said = |event_id, sent_t| Said {
            event_id,
            idem: 77,
            with: ShipId(7),
            with_name: "Ada".into(),
            mine: true,
            key: false,
            sealed: false,
            body: Some("are you there".into()),
            acks: Vec::new(),
            sent_t,
            arrive_t: None,
        };
        chat.restore(vec![said(1, 1_000_000), said(2, 9_000_000)], Vec::new());
        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1);
        assert_eq!(conversation.lines[0].event_ids, vec![1, 2]);
    }
}
