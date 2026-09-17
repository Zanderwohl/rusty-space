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
    /// How loud it was on arrival, in the strength units the gate compares against a noise
    /// floor. `None` for one this ship sent, and for one read back from a transcript: the store
    /// keeps what was *said*, and how loudly it landed is a fact about one receiver.
    pub strength: Option<f32>,
}

impl Line {
    /// The arrival strength as decibels, or `None` for a message this ship sent.
    ///
    /// Referred to one strength unit, which is the same arbitrary scale the noise floor is
    /// quoted in — so the number is meaningful *compared to another signal*, which is the only
    /// way anybody reads a dB figure anyway. Silence is not `0 dB`; it is no reading at all.
    pub fn decibels(&self) -> Option<f32> {
        match self.strength {
            Some(strength) if strength > 0.0 => Some(10.0 * strength.log10()),
            _ => None,
        }
    }
}

/// Everything said to and by one craft.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conversation {
    pub name: String,
    /// In the order this ship learnt of them, which for a conversation across light delay is
    /// not the order they were sent in: a reply can be composed before the message it crosses.
    ///
    /// **Not everything that arrived.** A message with nothing in it is an acknowledgement and
    /// nothing else, and it is absorbed into [`Conversation::acked`] rather than shown: there
    /// is nothing to read, and a log of empty lines is a log nobody can read either.
    pub lines: Vec<Line>,
    /// Every event identifier the other end has named, from any message they sent.
    ///
    /// Kept here rather than read back off the lines because the message that carries an
    /// acknowledgement is usually one with nothing else in it — and that one is not a line.
    /// Losing the evidence along with the clutter would make every message look unanswered for
    /// ever.
    acked: std::collections::BTreeSet<i64>,
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
        line.event_ids.iter().any(|id| self.acked.contains(id))
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
    /// Keyed by the other craft's identifier. What order they are *shown* in is
    /// [`Chat::conversations`]'s business.
    conversations: BTreeMap<i64, Conversation>,
    /// Open messages this ship broadcast, which are in nobody's conversation.
    ///
    /// Something said to no one in particular belongs to no one in particular. A *received*
    /// broadcast is not here — it came from a craft, so it files under that craft and can be
    /// answered — and appears in the public log the same way any other open message does.
    broadcasts: Vec<Line>,
    /// Craft this ship answers automatically. See [`Chat::auto_acks`].
    auto_ack: std::collections::BTreeSet<i64>,
    /// Whose public keys this ship holds, and can therefore encrypt a message to.
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
    pub fn public(&self) -> Vec<(Option<ShipId>, &str, &Line)> {
        let mut out: Vec<(Option<ShipId>, &str, &Line)> = self
            .conversations
            .iter()
            .flat_map(|(id, c)| {
                c.lines
                    .iter()
                    .filter(|line| !line.sealed)
                    .map(move |line| (Some(ShipId(*id)), c.name.as_str(), line))
            })
            // No counterpart to name: `None` is what says so.
            .chain(self.broadcasts.iter().map(|line| (None, "", line)))
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

    /// Whether this ship answers `of` automatically.
    pub fn auto_acks(&self, of: ShipId) -> bool {
        self.auto_ack.contains(&of.0)
    }

    /// Answer this craft automatically, or stop. Off for everyone until it is asked for: a ship
    /// that replied to every signal it heard would announce its position to everything in range
    /// the moment anyone pinged it.
    pub fn set_auto_ack(&mut self, of: ShipId, on: bool) {
        match on {
            true => self.auto_ack.insert(of.0),
            false => self.auto_ack.remove(&of.0),
        };
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
        self.broadcasts.clear();
        self.keys = keys.into_iter().map(|k| k.0).collect();
        for said in messages {
            let Some(with) = said.with else {
                // A broadcast this ship sent. Nobody's conversation, and the public log's.
                restore_into(&mut self.broadcasts, said);
                continue;
            };
            let conversation = self.conversations.entry(with.0).or_default();
            conversation.name = said.with_name.clone();
            // Only what the *other* end named. This ship's own acknowledgements say nothing
            // about whether its own messages arrived, and folding them in would mark every
            // message it ever sent as delivered.
            if !said.mine {
                conversation.acked.extend(said.acks.iter().copied());
            }
            restore_into(&mut conversation.lines, said);
        }
    }

    /// A transmission whose light has just landed.
    ///
    /// `name` is what this ship currently calls the sender, when it can see one. A craft that
    /// has since gone out of sight keeps the name the conversation already had rather than
    /// reverting to its number, because a contact list is not the only place a name comes from.
    /// A transmission whose light has just landed, and what answering it automatically would
    /// take — `None` when nothing is owed.
    ///
    /// **Only a message with something in it is answered.** A bare acknowledgement is not, and
    /// that is not a nicety: two ships each answering the other's answers would trade light
    /// for ever, at whatever the round trip between them is, without either pilot present.
    #[allow(clippy::too_many_arguments)]
    pub fn received(
        &mut self,
        from: ShipId,
        name: Option<&str>,
        event_id: i64,
        spoken: Spoken,
        key: bool,
        sent_s: f64,
        arrive_s: f64,
        strength: f32,
        bearing: [f64; 3],
    ) -> Option<lc_proto::Aim> {
        if key {
            self.keys.insert(from.0);
        }
        let conversation = self.conversations.entry(from.0).or_default();
        if let Some(name) = name {
            conversation.name = name.to_string();
        } else if conversation.name.is_empty() {
            conversation.name = format!("ship {}", from.0);
        }
        // **What it acknowledges is kept whatever becomes of the message itself.** Absorbed
        // first, because the usual carrier of an acknowledgement is a message that is about to
        // be dropped for having nothing in it.
        conversation.acked.extend(spoken.acks.iter().copied());
        // Idempotent twice over. A reconnection replays — `ResumeFrom` winds the delivery
        // cursor back and everything since arrives again — and a sender may have *resent*,
        // which is a genuinely different pulse of light carrying the same message.
        if conversation.lines.iter().any(|line| line.event_ids.contains(&event_id)) {
            return None;
        }
        if !key
            && let Some(line) = same_message(&mut conversation.lines, spoken.idem, false)
        {
            line.event_ids.push(event_id);
            // Already answered when the first copy landed. Answering a resend as well would
            // send one reply per attempt at reaching us.
            return None;
        }
        let worth_answering = !key && spoken.body.as_deref().is_some_and(|b| !b.is_empty());
        if bare_acknowledgement(key, spoken.body.as_deref()) {
            // Nothing to read, so nothing to show. Its acknowledgements are already kept, and
            // it is not answered either: an acknowledgement is the end of the exchange, not
            // the middle of one.
            return None;
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
            strength: Some(strength),
        });
        if !worth_answering || !self.auto_ack.contains(&from.0) {
            return None;
        }
        // In the mode it was spoken in. A beam is answered down the bearing it arrived on,
        // which a dish knows without knowing who sent it — and which is therefore aimed where
        // they *were*, not where they will be. See `lc_proto::Aim::Bearing`.
        Some(match spoken.beamed {
            true => lc_proto::Aim::Bearing(bearing),
            false => lc_proto::Aim::Omni,
        })
    }

    /// A message this ship has just had accepted. Recorded against the identifier the server
    /// minted, which is the only thing an acknowledgement will ever name it by.
    #[allow(clippy::too_many_arguments)]
    pub fn sent(
        &mut self,
        to: Option<ShipId>,
        name: Option<&str>,
        event_id: i64,
        idem: lc_proto::MessageKey,
        body: Option<String>,
        sealed: bool,
        key: bool,
        sent_s: f64,
    ) {
        // Something said to nobody in particular belongs to nobody in particular: it is in the
        // public log and in no conversation, because there is no conversation it is part of.
        let lines = match to {
            Some(to) => {
                let conversation = self.conversations.entry(to.0).or_default();
                if conversation.name.is_empty() {
                    conversation.name =
                        name.map(str::to_string).unwrap_or_else(|| format!("ship {}", to.0));
                }
                &mut conversation.lines
            }
            None => &mut self.broadcasts,
        };
        if lines.iter().any(|line| line.event_ids.contains(&event_id)) {
            return;
        }
        // This ship's own acknowledgements are as empty as anybody's. Shown, a conversation
        // with auto-ack on would be half blank lines from this end.
        if bare_acknowledgement(key, body.as_deref()) {
            return;
        }
        // A resend joins the line it repeats. The line keeps the time it was *first* said,
        // because that is when the thing was said; the resends are how hard it was tried.
        if !key
            && let Some(line) = same_message(lines, idem, true)
        {
            line.event_ids.push(event_id);
            return;
        }
        lines.push(Line {
            event_ids: vec![event_id],
            idem,
            mine: true,
            key,
            sealed,
            body,
            acks: Vec::new(),
            sent_s,
            arrive_s: None,
            strength: None,
        });
    }
}

/// Fold one entry of a transcript into a list of lines.
///
/// A backlog carries one entry per *transmission*, so a message that was resent arrives here
/// several times and is one line with several events behind it — the same rule that applies
/// when it is live, in the one other place a line can be made.
fn restore_into(lines: &mut Vec<Line>, said: Said) {
    if bare_acknowledgement(said.key, said.body.as_deref()) {
        return;
    }
    if let Some(line) = same_message(lines, said.idem, said.mine) {
        if !line.event_ids.contains(&said.event_id) {
            line.event_ids.push(said.event_id);
        }
        // The acknowledgements of the copy that actually landed are the ones worth keeping,
        // and a later copy can only know more.
        if said.acks.len() > line.acks.len() {
            line.acks = said.acks;
        }
        return;
    }
    lines.push(Line {
        event_ids: vec![said.event_id],
        idem: said.idem,
        mine: said.mine,
        key: said.key,
        sealed: said.sealed,
        body: said.body,
        acks: said.acks,
        sent_s: said.sent_t as f64 * 1.0e-6,
        arrive_s: said.arrive_t.map(|t| t as f64 * 1.0e-6),
        // Not kept by the store: how loudly a signal landed is a fact about one receiver, and
        // what is written down is what was said.
        strength: None,
    });
}

/// Whether a message is an acknowledgement and nothing else.
///
/// An empty body with no key offer behind it: its whole content is the identifiers riding in
/// its payload, which are kept on the conversation. There is nothing to read, so there is
/// nothing to show, and nothing to answer either — this is the end of an exchange rather than
/// the middle of one.
///
/// A message this ship cannot read is **not** this. Its body is `None` rather than empty, and
/// that somebody in earshot is talking in private is exactly the kind of thing worth showing.
fn bare_acknowledgement(key: bool, body: Option<&str>) -> bool {
    !key && body == Some("")
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
        Spoken { to: Some(to), beamed: false, idem: 0, sealed, body: body.map(Into::into), acks }
    }

    fn keyed(to: i64, body: &str, idem: u64, acks: Vec<i64>) -> Spoken {
        Spoken { to: Some(to), beamed: false, idem, sealed: false, body: Some(body.into()), acks }
    }

    /// A line by its position, for a test that wants to talk about one.
    fn line(chat: &Chat, with: i64, at: usize) -> Line {
        chat.get(ShipId(with)).expect("a conversation").lines[at].clone()
    }

    /// The only delivery report there is, and it is a report about a *specific* message.
    #[test]
    fn a_message_is_delivered_only_when_the_far_end_names_it() {
        let mut chat = Chat::default();
        chat.sent(Some(ShipId(7)), Some("Ada"), 100, 1, Some("first".into()), false, false, 1.0);
        chat.sent(Some(ShipId(7)), Some("Ada"), 101, 2, Some("second".into()), false, false, 2.0);
        let (first, second) = (line(&chat, 7, 0), line(&chat, 7, 1));
        assert!(!chat.get(ShipId(7)).unwrap().delivered(&first));

        chat.received(ShipId(7), Some("Ada"), 200, spoken(1, Some("got it"), false, vec![100]), false, 9.0, 12.0, 1.0, [1.0, 0.0, 0.0]);
        let conversation = chat.get(ShipId(7)).unwrap();
        assert!(conversation.delivered(&first));
        assert!(!conversation.delivered(&second), "an unnamed message was reported delivered");
    }

    /// **A resend is the same message, said twice.** Two pulses of light and two events — both
    /// really happened — and one line, because one thing was said.
    #[test]
    fn a_resend_joins_the_line_it_repeats_rather_than_making_a_second() {
        let mut chat = Chat::default();
        chat.sent(Some(ShipId(7)), Some("Ada"), 100, 42, Some("are you there".into()), false, false, 1.0);
        chat.sent(Some(ShipId(7)), Some("Ada"), 108, 42, Some("are you there".into()), false, false, 9.0);

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
        chat.sent(Some(ShipId(7)), Some("Ada"), 100, 42, Some("again".into()), false, false, 1.0);
        chat.sent(Some(ShipId(7)), Some("Ada"), 108, 42, Some("again".into()), false, false, 9.0);
        chat.received(ShipId(7), Some("Ada"), 200, keyed(1, "heard the second one", 5, vec![108]), false, 10.0, 12.0, 1.0, [1.0, 0.0, 0.0]);

        let sent = line(&chat, 7, 0);
        assert!(chat.get(ShipId(7)).unwrap().delivered(&sent));
    }

    /// The receiving end of the same rule: two arrivals carrying one key are one line.
    #[test]
    fn a_message_heard_twice_is_shown_once() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), Some("Ada"), 200, keyed(1, "hello", 77, vec![]), false, 1.0, 4.0, 1.0, [1.0, 0.0, 0.0]);
        chat.received(ShipId(7), Some("Ada"), 209, keyed(1, "hello", 77, vec![]), false, 6.0, 9.0, 1.0, [1.0, 0.0, 0.0]);
        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1, "a resend was shown twice");
        assert_eq!(conversation.lines[0].event_ids, vec![200, 209]);
    }

    /// A key of zero is "not keyed" — a key offer, or a row from before there were keys — and
    /// must never fold. Folding them all together would make every unkeyed message one message.
    #[test]
    fn unkeyed_messages_never_fold_into_each_other() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), None, 200, spoken(1, Some("one"), false, vec![]), false, 1.0, 2.0, 1.0, [1.0, 0.0, 0.0]);
        chat.received(ShipId(7), None, 201, spoken(1, Some("two"), false, vec![]), false, 3.0, 4.0, 1.0, [1.0, 0.0, 0.0]);
        assert_eq!(chat.get(ShipId(7)).unwrap().lines.len(), 2);
    }

    /// A reconnection replays the delivery stream, so the same arrival can be folded twice. It
    /// must not become two lines.
    #[test]
    fn the_same_arrival_folded_twice_is_one_line() {
        let mut chat = Chat::default();
        let said = spoken(1, Some("hello"), false, Vec::new());
        chat.received(ShipId(7), Some("Ada"), 200, said.clone(), false, 1.0, 4.0, 1.0, [1.0, 0.0, 0.0]);
        chat.received(ShipId(7), Some("Ada"), 200, said, false, 1.0, 4.0, 1.0, [1.0, 0.0, 0.0]);
        assert_eq!(chat.get(ShipId(7)).unwrap().lines.len(), 1);
    }

    /// A key is held from the moment its offer *lands*, and the client learns it the same way
    /// the server does: by the arrival, not by the sending.
    #[test]
    fn a_key_offer_arriving_is_what_puts_a_key_in_the_ring() {
        let mut chat = Chat::default();
        assert!(!chat.holds_key(ShipId(7)));
        chat.received(ShipId(7), None, 300, spoken(1, Some(""), false, Vec::new()), true, 1.0, 5.0, 1.0, [1.0, 0.0, 0.0]);
        assert!(chat.holds_key(ShipId(7)));
        // And sending one away teaches this ship nothing about anybody.
        chat.sent(Some(ShipId(8)), None, 301, 0, None, false, true, 6.0);
        assert!(!chat.holds_key(ShipId(8)), "offering a key taught the offerer something");
    }

    /// A sealed message somebody else's is kept, with no body. It was heard; it cannot be read.
    #[test]
    fn a_sealed_message_for_somebody_else_is_a_line_with_nothing_in_it() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), None, 400, spoken(99, None, true, Vec::new()), false, 1.0, 3.0, 1.0, [1.0, 0.0, 0.0]);
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
        chat.received(ShipId(7), Some("Ada"), 500, spoken(1, Some("here"), false, vec![]), false, 1.0, 2.0, 1.0, [1.0, 0.0, 0.0]);
        chat.received(ShipId(7), None, 501, spoken(1, Some("still here"), false, vec![]), false, 3.0, 9.0, 1.0, [1.0, 0.0, 0.0]);
        assert_eq!(chat.get(ShipId(7)).unwrap().name, "Ada");
    }

    /// **The public log is a view, and encryption is what keeps something out of it** — from
    /// either end. This ship's own sealed message is as absent as anybody else's, because a
    /// private message in a public log is one somebody can read over your shoulder.
    #[test]
    fn the_public_log_holds_what_was_said_in_the_open_and_nothing_else() {
        let mut chat = Chat::default();
        chat.received(ShipId(7), Some("Ada"), 200, spoken(1, Some("in the open"), false, vec![]), false, 1.0, 2.0, 1.0, [1.0, 0.0, 0.0]);
        chat.received(ShipId(7), Some("Ada"), 201, spoken(99, None, true, vec![]), false, 3.0, 4.0, 1.0, [1.0, 0.0, 0.0]);
        chat.sent(Some(ShipId(8)), Some("Bry"), 300, 5, Some("mine, open".into()), false, false, 5.0);
        chat.sent(Some(ShipId(8)), Some("Bry"), 301, 6, Some("mine, private".into()), true, false, 6.0);

        let public = chat.public();
        let bodies: Vec<&str> =
            public.iter().filter_map(|(_, _, l)| l.body.as_deref()).collect();
        assert_eq!(bodies, vec!["in the open", "mine, open"]);
        assert!(public.iter().all(|(_, _, l)| !l.sealed));
        // And each line still says who it is with, which a mixed log needs to be answerable.
        assert_eq!(public[0].0, Some(ShipId(7)));
        assert_eq!(public[1].0, Some(ShipId(8)));
    }

    /// Oldest first, by when *this ship* learnt of each — which for a conversation across light
    /// delay is not the order they were sent in.
    #[test]
    fn the_public_log_is_ordered_by_when_this_ship_learnt_of_each() {
        let mut chat = Chat::default();
        // Sent early, heard late: a long crossing.
        chat.received(ShipId(7), Some("Ada"), 200, spoken(1, Some("slow"), false, vec![]), false, 1.0, 90.0, 1.0, [1.0, 0.0, 0.0]);
        chat.sent(Some(ShipId(8)), Some("Bry"), 300, 5, Some("quick".into()), false, false, 50.0);
        let order: Vec<&str> = chat.public().iter().filter_map(|(_, _, l)| l.body.as_deref()).collect();
        assert_eq!(order, vec!["quick", "slow"], "ordered by sending, not by learning");
    }

    /// The list puts whoever spoke last at the top, which is how a player finds the
    /// conversation they are in the middle of.
    #[test]
    fn conversations_are_listed_most_recently_heard_first() {
        let mut chat = Chat::default();
        chat.sent(Some(ShipId(7)), Some("Ada"), 100, 1, Some("early".into()), false, false, 1.0);
        chat.sent(Some(ShipId(8)), Some("Bry"), 200, 2, Some("later".into()), false, false, 5.0);
        let order: Vec<ShipId> = chat.conversations().into_iter().map(|(id, _)| id).collect();
        assert_eq!(order, vec![ShipId(8), ShipId(7)]);

        // And it moves when something lands.
        chat.received(ShipId(7), Some("Ada"), 300, spoken(1, Some("back"), false, vec![]), false, 6.0, 9.0, 1.0, [1.0, 0.0, 0.0]);
        let order: Vec<ShipId> = chat.conversations().into_iter().map(|(id, _)| id).collect();
        assert_eq!(order, vec![ShipId(7), ShipId(8)]);
    }

    /// **The quiet half of an acknowledgement.** A message with nothing in it is not shown —
    /// there is nothing to read — and what it acknowledges is kept anyway. Dropping both would
    /// make every message look unanswered for ever, which is the failure worth guarding.
    #[test]
    fn a_bare_acknowledgement_is_not_shown_and_still_marks_the_message_delivered() {
        let mut chat = Chat::default();
        chat.sent(Some(ShipId(7)), Some("Ada"), 100, 1, Some("are you there".into()), false, false, 1.0);
        let sent = line(&chat, 7, 0);
        assert!(!chat.get(ShipId(7)).unwrap().delivered(&sent));

        let bare = Spoken {
            to: Some(1),
            beamed: false,
            idem: 5,
            sealed: false,
            body: Some(String::new()),
            acks: vec![100],
        };
        chat.received(ShipId(7), Some("Ada"), 200, bare, false, 9.0, 12.0, 1.0, [1.0, 0.0, 0.0]);

        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1, "an empty acknowledgement was shown as a line");
        assert!(conversation.delivered(&sent), "the acknowledgement was dropped with the line");
    }

    /// This ship's own empty acknowledgements are as quiet as anybody's. With auto-ack on, a
    /// conversation would otherwise be half blank lines from this end.
    #[test]
    fn this_ships_own_bare_acknowledgements_are_not_shown_either() {
        let mut chat = Chat::default();
        chat.sent(Some(ShipId(7)), Some("Ada"), 300, 9, Some(String::new()), false, false, 1.0);
        assert!(chat.get(ShipId(7)).is_none_or(|c| c.lines.is_empty()));
    }

    /// A message this ship cannot *read* is not a message with nothing in it. That somebody in
    /// earshot is talking in private is exactly the kind of thing worth showing.
    #[test]
    fn an_unreadable_message_is_not_mistaken_for_an_acknowledgement() {
        let mut chat = Chat::default();
        let sealed = Spoken {
            to: Some(99),
            beamed: false,
            idem: 3,
            sealed: true,
            body: None,
            acks: vec![],
        };
        chat.received(ShipId(7), None, 400, sealed, false, 1.0, 3.0, 1.0, [1.0, 0.0, 0.0]);
        assert_eq!(chat.get(ShipId(7)).unwrap().lines.len(), 1);
    }

    /// A key offer's body is empty too, and it is not an acknowledgement.
    #[test]
    fn a_key_offer_is_shown_despite_having_no_body() {
        let mut chat = Chat::default();
        let offer = Spoken {
            to: Some(1),
            beamed: false,
            idem: 0,
            sealed: false,
            body: Some(String::new()),
            acks: vec![],
        };
        chat.received(ShipId(7), None, 500, offer, true, 1.0, 3.0, 1.0, [1.0, 0.0, 0.0]);
        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1);
        assert!(conversation.lines[0].key);
    }

    /// Off until it is asked for, and then it answers in the mode it was spoken to in.
    #[test]
    fn auto_ack_answers_only_when_it_is_turned_on() {
        let mut chat = Chat::default();
        let heard = |chat: &mut Chat, event_id: i64, beamed| {
            let said = Spoken {
                to: Some(1),
                beamed,
                idem: event_id as u64,
                sealed: false,
                body: Some("hello".into()),
                acks: vec![],
            };
            chat.received(ShipId(7), Some("Ada"), event_id, said, false, 1.0, 2.0, 1.0, [0.0, 1.0, 0.0])
        };
        assert_eq!(heard(&mut chat, 1, false), None, "answered without being asked to");

        chat.set_auto_ack(ShipId(7), true);
        assert_eq!(heard(&mut chat, 2, false), Some(lc_proto::Aim::Omni));
        // A beam is answered down the bearing it came in on, which is what a dish knows.
        assert_eq!(heard(&mut chat, 3, true), Some(lc_proto::Aim::Bearing([0.0, 1.0, 0.0])));

        chat.set_auto_ack(ShipId(7), false);
        assert_eq!(heard(&mut chat, 4, false), None, "it kept answering after being told to stop");
    }

    /// **The loop guard.** Two ships each answering the other automatically would trade light
    /// for ever, with no pilot present at either end. A bare acknowledgement is not answered.
    #[test]
    fn an_acknowledgement_is_never_itself_acknowledged() {
        let mut chat = Chat::default();
        chat.set_auto_ack(ShipId(7), true);
        let bare = Spoken {
            to: Some(1),
            beamed: false,
            idem: 5,
            sealed: false,
            body: Some(String::new()),
            acks: vec![100],
        };
        assert_eq!(
            chat.received(ShipId(7), None, 200, bare, false, 1.0, 2.0, 1.0, [1.0, 0.0, 0.0]),
            None,
            "an empty acknowledgement was answered with another one",
        );
        // And a key offer is not something to answer either.
        let key = Spoken {
            to: Some(1),
            beamed: false,
            idem: 0,
            sealed: false,
            body: Some(String::new()),
            acks: vec![],
        };
        assert_eq!(
            chat.received(ShipId(7), None, 201, key, true, 1.0, 2.0, 1.0, [1.0, 0.0, 0.0]),
            None,
        );
    }

    /// A resend is answered once, not once per attempt at reaching us.
    #[test]
    fn a_resend_does_not_earn_a_second_automatic_answer() {
        let mut chat = Chat::default();
        chat.set_auto_ack(ShipId(7), true);
        let said = |event_id| (event_id, Spoken {
            to: Some(1),
            beamed: false,
            idem: 42,
            sealed: false,
            body: Some("are you there".into()),
            acks: vec![],
        });
        let (first, a) = said(200);
        assert!(chat.received(ShipId(7), None, first, a, false, 1.0, 2.0, 1.0, [1.0, 0.0, 0.0]).is_some());
        let (again, b) = said(208);
        assert_eq!(
            chat.received(ShipId(7), None, again, b, false, 6.0, 9.0, 1.0, [1.0, 0.0, 0.0]),
            None,
            "a resend was answered a second time",
        );
    }

    /// A broadcast this ship sent is in the public log and in nobody's conversation, because
    /// there is no conversation it is part of.
    #[test]
    fn a_broadcast_is_in_the_public_log_and_in_no_conversation() {
        let mut chat = Chat::default();
        chat.sent(None, None, 300, 9, Some("to whoever is listening".into()), false, false, 1.0);
        assert!(chat.conversations().is_empty(), "a broadcast opened a conversation");
        let public = chat.public();
        assert_eq!(public.len(), 1);
        assert_eq!(public[0].0, None, "a broadcast named a counterpart");
        assert_eq!(public[0].2.body.as_deref(), Some("to whoever is listening"));
    }

    /// The reading a tooltip shows, and the one case where there is none.
    #[test]
    fn strength_reads_as_decibels_and_silence_reads_as_nothing() {
        let mut chat = Chat::default();
        let said = Spoken {
            to: Some(1),
            beamed: false,
            idem: 1,
            sealed: false,
            body: Some("loud".into()),
            acks: vec![],
        };
        chat.received(ShipId(7), None, 200, said, false, 1.0, 2.0, 100.0, [1.0, 0.0, 0.0]);
        let heard = &chat.get(ShipId(7)).unwrap().lines[0];
        assert!((heard.decibels().unwrap() - 20.0).abs() < 1.0e-4, "{:?}", heard.decibels());

        // Nothing this ship sent has a reading: it never heard it.
        chat.sent(Some(ShipId(7)), None, 301, 2, Some("mine".into()), false, false, 3.0);
        let mine = chat.get(ShipId(7)).unwrap().lines.last().unwrap();
        assert_eq!(mine.decibels(), None);
    }

    #[test]
    fn a_restored_transcript_replaces_rather_than_merges() {
        let mut chat = Chat::default();
        chat.sent(Some(ShipId(7)), None, 1, 9, Some("stale".into()), false, false, 0.0);
        chat.restore(
            vec![Said {
                event_id: 9,
                idem: 4,
                with: Some(ShipId(7)),
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
            with: Some(ShipId(7)),
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
