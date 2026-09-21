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
    /// floor. `None` for one this ship sent — it never heard it — and for one recorded before
    /// the store kept the reading.
    pub strength: Option<f32>,
}

impl Line {
    /// What an unreadable message looks like: fixed-length noise, and the same noise every
    /// time this line is drawn.
    ///
    /// **Fixed length whatever the message was**, which is the point rather than a detail. A
    /// run of gibberish whose length tracked the plaintext would leak the one thing about a
    /// sealed message that is still readable — a long one is a long one — and a player could
    /// read a conversation's shape without reading a word of it.
    ///
    /// Derived from when it was heard, so it is stable across frames and reconnections and
    /// there is nothing in it to decode. It is not ciphertext and does not pretend to be; it is
    /// what a receiver has instead of a message.
    pub fn ciphertext(&self) -> String {
        const GLYPHS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
        const LENGTH: usize = 24;
        let seed = self.event_ids.first().copied().unwrap_or_default() as u64;
        (0..LENGTH)
            .map(|k| {
                let h = lc_world::rng::hash(&[seed, k as u64]);
                GLYPHS[(h % GLYPHS.len() as u64) as usize] as char
            })
            .collect()
    }

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

/// Where a transmission belongs, decided by who it was addressed to.
///
/// One rule with two readers — what [`Chat::received`] files it as, and what the events box
/// says about it — and it is a named answer rather than a predicate at each of them so the two
/// cannot drift into disagreeing about what a message *is*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filing {
    /// Addressed to this ship: a conversation with whoever sent it.
    Conversation,
    /// Addressed to nobody: a broadcast.
    Broadcast,
    /// Addressed to another craft, with this ship merely in earshot.
    Overheard,
}

/// A transmission that belongs to no conversation.
///
/// Two kinds, told apart by [`Loose::to`]: a **broadcast**, said to nobody in particular, and
/// something **overheard**, said by one craft to another with this ship merely in earshot.
/// Neither is a conversation this ship is having, and filing them as one would put words in
/// somebody's mouth — a log of "what Ada said to me" containing what Ada said to Bry.
#[derive(Clone, Debug, PartialEq)]
pub struct Loose {
    /// Who transmitted it, or `None` when this ship did.
    pub from: Option<ShipId>,
    /// What to call them. Empty for this ship's own.
    pub from_name: String,
    /// Who it was addressed to. `None` is a broadcast; anything else is overheard.
    pub to: Option<ShipId>,
    pub line: Line,
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
        self.lines.last().map_or(f64::NEG_INFINITY, |line| {
            line.arrive_s.unwrap_or(line.sent_s)
        })
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
    /// Everything in no conversation: broadcasts, and other people's mail.
    ///
    /// One list rather than two, because the only thing that separates them is who each was
    /// addressed to, and that is a field on the row. See [`Loose`].
    loose: Vec<Loose>,
    /// Which craft this ship is, so a message addressed to it can be told from one that merely
    /// reached it.
    ///
    /// `None` until a welcome says. Everything is filed as a conversation until then, which is
    /// the offline case and the frame before the server answers: better to show a message in
    /// the wrong tab than to decide it was somebody else's on no evidence.
    me: Option<ShipId>,
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
        let mut out: Vec<(ShipId, &Conversation)> = self
            .conversations
            .iter()
            .map(|(id, c)| (ShipId(*id), c))
            .collect();
        out.sort_by(|a, b| {
            b.1.last_at()
                .total_cmp(&a.1.last_at())
                .then_with(|| a.0.0.cmp(&b.0.0))
        });
        out
    }

    /// Which craft this ship is. Stated by the welcome; see [`Chat::me`].
    pub fn i_am(&mut self, me: ShipId) {
        self.me = Some(me);
    }

    /// Where a transmission addressed to `to` belongs.
    ///
    /// Until a welcome says which craft this is, anything with an addressee is taken to be a
    /// conversation: better to show a message in the wrong tab than to decide it was somebody
    /// else's on no evidence.
    pub fn filing(&self, to: Option<i64>) -> Filing {
        match (to, self.me) {
            (None, _) => Filing::Broadcast,
            (Some(to), Some(me)) if to != me.0 => Filing::Overheard,
            (Some(_), _) => Filing::Conversation,
        }
    }

    /// Everything said to nobody in particular, oldest first — sent and heard.
    ///
    /// **Broadcasts and nothing else.** A message addressed to one craft is not public however
    /// openly it was sent: anyone in range can read it, but it was still somebody's mail, and
    /// [`Chat::overheard`] is where being in earshot of it belongs.
    pub fn public(&self) -> Vec<&Loose> {
        self.loose_where(|loose| loose.to.is_none())
    }

    /// Traffic between other craft that this ship happened to be in range of, oldest first.
    ///
    /// Open ones can be read, which is what "open" means and is the cost of shouting. Encrypted
    /// ones cannot, and are shown as the noise they are — the fact of them is real and is worth
    /// seeing, because a run of traffic between two craft says something even when none of it
    /// can be read.
    pub fn overheard(&self) -> Vec<&Loose> {
        self.loose_where(|loose| loose.to.is_some())
    }

    fn loose_where(&self, keep: impl Fn(&Loose) -> bool) -> Vec<&Loose> {
        let mut out: Vec<&Loose> = self.loose.iter().filter(|l| keep(l)).collect();
        out.sort_by(|a, b| when(&a.line).total_cmp(&when(&b.line)));
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
        self.loose.clear();
        self.keys = keys.into_iter().map(|k| k.0).collect();
        for said in messages {
            // The same rule the live path uses, and it has to be: a transcript replayed into
            // different tabs from the ones it arrived in would be a different transcript.
            // A message this ship *sent* is its own conversation whoever it was for, which is
            // the one way the replay differs from the live path: nothing this ship said is
            // something it overheard.
            let for_us = said.mine || self.filing(said.to.map(|to| to.0)) == Filing::Conversation;
            let Some(with) = said.with.filter(|_| for_us) else {
                if bare_acknowledgement(said.key, said.body.as_deref()) {
                    continue;
                }
                let from = said.with.filter(|_| !said.mine);
                self.loose.push(Loose {
                    from,
                    from_name: from.map(|_| said.with_name.clone()).unwrap_or_default(),
                    to: said.to,
                    line: line_of(said),
                });
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
        // **Who it was addressed to decides where it is filed**, not who sent it. Only a
        // message addressed to this ship is a conversation with the craft that sent it; a
        // broadcast was said to nobody, and traffic between two other craft merely reached
        // this antenna. Filing either as a conversation would put words in somebody's mouth.
        //
        // Until a welcome says which craft this is, everything is a conversation: better to
        // show a message in the wrong tab than to decide it was somebody else's on no evidence.
        if self.filing(spoken.to) != Filing::Conversation {
            return self.overhear(
                from, name, event_id, spoken, key, sent_s, arrive_s, strength,
            );
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
        if conversation
            .lines
            .iter()
            .any(|line| line.event_ids.contains(&event_id))
        {
            return None;
        }
        if !key && let Some(line) = same_message(&mut conversation.lines, spoken.idem, false) {
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

    /// File a transmission that is in no conversation: a broadcast, or somebody else's mail.
    ///
    /// Never answered automatically, whatever auto-ack is set to. Acknowledging a broadcast
    /// would answer everybody at once, and acknowledging a message meant for a third craft
    /// would tell its sender that somebody they were not talking to is listening.
    #[allow(clippy::too_many_arguments)]
    fn overhear(
        &mut self,
        from: ShipId,
        name: Option<&str>,
        event_id: i64,
        spoken: Spoken,
        key: bool,
        sent_s: f64,
        arrive_s: f64,
        strength: f32,
    ) -> Option<lc_proto::Aim> {
        if self
            .loose
            .iter()
            .any(|l| l.line.event_ids.contains(&event_id))
        {
            return None;
        }
        if !key
            && let Some(loose) = self
                .loose
                .iter_mut()
                .find(|l| l.from == Some(from) && l.line.idem == spoken.idem)
            && spoken.idem != 0
        {
            loose.line.event_ids.push(event_id);
            return None;
        }
        if bare_acknowledgement(key, spoken.body.as_deref()) {
            return None;
        }
        // The name it is known by, or its number. There may be no conversation to borrow one
        // from — this is a craft that has never spoken to this ship.
        let from_name = name
            .map(str::to_string)
            .or_else(|| {
                self.conversations
                    .get(&from.0)
                    .map(|c| c.name.clone())
                    .filter(|n| !n.is_empty())
            })
            .unwrap_or_else(|| format!("ship {}", from.0));
        self.loose.push(Loose {
            from: Some(from),
            from_name,
            to: spoken.to.map(ShipId),
            line: Line {
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
            },
        });
        None
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
                    conversation.name = name
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("ship {}", to.0));
                }
                &mut conversation.lines
            }
            None => {
                if self
                    .loose
                    .iter()
                    .any(|l| l.line.event_ids.contains(&event_id))
                {
                    return;
                }
                self.loose.push(Loose {
                    from: None,
                    from_name: String::new(),
                    to: None,
                    line: Line {
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
                    },
                });
                return;
            }
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
        if !key && let Some(line) = same_message(lines, idem, true) {
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
    lines.push(line_of(said));
}

/// One transcript entry as a line.
fn line_of(said: Said) -> Line {
    Line {
        event_ids: vec![said.event_id],
        idem: said.idem,
        mine: said.mine,
        key: said.key,
        sealed: said.sealed,
        body: said.body,
        acks: said.acks,
        sent_s: said.sent_t as f64 * 1.0e-6,
        arrive_s: said.arrive_t.map(|t| t as f64 * 1.0e-6),
        // Kept by the store now. It was not, on the reasoning that a transcript holds what was
        // *said* — true, and it did not follow: the receipts table it is read from is the
        // per-receiver one, and the arrival time beside this is the same kind of fact. Without
        // it a reconnection, which is the normal case, lost the reading on everything.
        strength: said.strength,
    }
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
    lines
        .iter_mut()
        .find(|line| line.mine == mine && line.idem == idem && !line.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spoken(to: i64, body: Option<&str>, sealed: bool, acks: Vec<i64>) -> Spoken {
        Spoken {
            to: Some(to),
            beamed: false,
            idem: 0,
            sealed,
            body: body.map(Into::into),
            acks,
        }
    }

    fn broadcast(body: &str) -> Spoken {
        Spoken {
            to: None,
            beamed: false,
            idem: 0,
            sealed: false,
            body: Some(body.into()),
            acks: vec![],
        }
    }

    fn keyed(to: i64, body: &str, idem: u64, acks: Vec<i64>) -> Spoken {
        Spoken {
            to: Some(to),
            beamed: false,
            idem,
            sealed: false,
            body: Some(body.into()),
            acks,
        }
    }

    /// A line by its position, for a test that wants to talk about one.
    fn line(chat: &Chat, with: i64, at: usize) -> Line {
        chat.get(ShipId(with)).expect("a conversation").lines[at].clone()
    }

    /// The only delivery report there is, and it is a report about a *specific* message.
    #[test]
    fn a_message_is_delivered_only_when_the_far_end_names_it() {
        let mut chat = Chat::default();
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            100,
            1,
            Some("first".into()),
            false,
            false,
            1.0,
        );
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            101,
            2,
            Some("second".into()),
            false,
            false,
            2.0,
        );
        let (first, second) = (line(&chat, 7, 0), line(&chat, 7, 1));
        assert!(!chat.get(ShipId(7)).unwrap().delivered(&first));

        chat.received(
            ShipId(7),
            Some("Ada"),
            200,
            spoken(1, Some("got it"), false, vec![100]),
            false,
            9.0,
            12.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        let conversation = chat.get(ShipId(7)).unwrap();
        assert!(conversation.delivered(&first));
        assert!(
            !conversation.delivered(&second),
            "an unnamed message was reported delivered"
        );
    }

    /// **A resend is the same message, said twice.** Two pulses of light and two events — both
    /// really happened — and one line, because one thing was said.
    #[test]
    fn a_resend_joins_the_line_it_repeats_rather_than_making_a_second() {
        let mut chat = Chat::default();
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            100,
            42,
            Some("are you there".into()),
            false,
            false,
            1.0,
        );
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            108,
            42,
            Some("are you there".into()),
            false,
            false,
            9.0,
        );

        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(
            conversation.lines.len(),
            1,
            "a resend became a second message"
        );
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
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            100,
            42,
            Some("again".into()),
            false,
            false,
            1.0,
        );
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            108,
            42,
            Some("again".into()),
            false,
            false,
            9.0,
        );
        chat.received(
            ShipId(7),
            Some("Ada"),
            200,
            keyed(1, "heard the second one", 5, vec![108]),
            false,
            10.0,
            12.0,
            1.0,
            [1.0, 0.0, 0.0],
        );

        let sent = line(&chat, 7, 0);
        assert!(chat.get(ShipId(7)).unwrap().delivered(&sent));
    }

    /// The receiving end of the same rule: two arrivals carrying one key are one line.
    #[test]
    fn a_message_heard_twice_is_shown_once() {
        let mut chat = Chat::default();
        chat.received(
            ShipId(7),
            Some("Ada"),
            200,
            keyed(1, "hello", 77, vec![]),
            false,
            1.0,
            4.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        chat.received(
            ShipId(7),
            Some("Ada"),
            209,
            keyed(1, "hello", 77, vec![]),
            false,
            6.0,
            9.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1, "a resend was shown twice");
        assert_eq!(conversation.lines[0].event_ids, vec![200, 209]);
    }

    /// A key of zero is "not keyed" — a key offer, or a row from before there were keys — and
    /// must never fold. Folding them all together would make every unkeyed message one message.
    #[test]
    fn unkeyed_messages_never_fold_into_each_other() {
        let mut chat = Chat::default();
        chat.received(
            ShipId(7),
            None,
            200,
            spoken(1, Some("one"), false, vec![]),
            false,
            1.0,
            2.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        chat.received(
            ShipId(7),
            None,
            201,
            spoken(1, Some("two"), false, vec![]),
            false,
            3.0,
            4.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        assert_eq!(chat.get(ShipId(7)).unwrap().lines.len(), 2);
    }

    /// A reconnection replays the delivery stream, so the same arrival can be folded twice. It
    /// must not become two lines.
    #[test]
    fn the_same_arrival_folded_twice_is_one_line() {
        let mut chat = Chat::default();
        let said = spoken(1, Some("hello"), false, Vec::new());
        chat.received(
            ShipId(7),
            Some("Ada"),
            200,
            said.clone(),
            false,
            1.0,
            4.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        chat.received(
            ShipId(7),
            Some("Ada"),
            200,
            said,
            false,
            1.0,
            4.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        assert_eq!(chat.get(ShipId(7)).unwrap().lines.len(), 1);
    }

    /// A key is held from the moment its offer *lands*, and the client learns it the same way
    /// the server does: by the arrival, not by the sending.
    #[test]
    fn a_key_offer_arriving_is_what_puts_a_key_in_the_ring() {
        let mut chat = Chat::default();
        assert!(!chat.holds_key(ShipId(7)));
        chat.received(
            ShipId(7),
            None,
            300,
            spoken(1, Some(""), false, Vec::new()),
            true,
            1.0,
            5.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        assert!(chat.holds_key(ShipId(7)));
        // And sending one away teaches this ship nothing about anybody.
        chat.sent(Some(ShipId(8)), None, 301, 0, None, false, true, 6.0);
        assert!(
            !chat.holds_key(ShipId(8)),
            "offering a key taught the offerer something"
        );
    }

    /// A sealed message somebody else's is kept, with no body. It was heard; it cannot be read.
    #[test]
    fn a_sealed_message_for_somebody_else_is_a_line_with_nothing_in_it() {
        let mut chat = Chat::default();
        chat.received(
            ShipId(7),
            None,
            400,
            spoken(99, None, true, Vec::new()),
            false,
            1.0,
            3.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
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
        chat.received(
            ShipId(7),
            Some("Ada"),
            500,
            spoken(1, Some("here"), false, vec![]),
            false,
            1.0,
            2.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        chat.received(
            ShipId(7),
            None,
            501,
            spoken(1, Some("still here"), false, vec![]),
            false,
            3.0,
            9.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        assert_eq!(chat.get(ShipId(7)).unwrap().name, "Ada");
    }

    /// The one rule that decides where everything goes, stated once so two readers cannot
    /// drift apart about what a message is.
    #[test]
    fn where_a_message_belongs_is_decided_by_who_it_was_addressed_to() {
        let mut chat = Chat::default();
        // Before a welcome says which craft this is, anything addressed is a conversation.
        assert_eq!(chat.filing(Some(9)), Filing::Conversation);
        assert_eq!(chat.filing(None), Filing::Broadcast);

        chat.i_am(ShipId(1));
        assert_eq!(chat.filing(Some(1)), Filing::Conversation);
        assert_eq!(chat.filing(Some(9)), Filing::Overheard);
        assert_eq!(chat.filing(None), Filing::Broadcast);
    }

    /// **Public is broadcasts and nothing else.** A message addressed to one craft is not
    /// public however openly it was sent: anyone in range can read it, but it was still
    /// somebody's mail.
    #[test]
    fn the_public_log_holds_broadcasts_and_nothing_else() {
        let mut chat = Chat::default();
        chat.i_am(ShipId(1));
        // Heard, addressed to nobody: a broadcast.
        chat.received(
            ShipId(7),
            Some("Ada"),
            200,
            broadcast("to whoever"),
            false,
            1.0,
            2.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        // Heard, addressed to this ship: a conversation.
        chat.received(
            ShipId(7),
            Some("Ada"),
            201,
            spoken(1, Some("for you"), false, vec![]),
            false,
            3.0,
            4.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        // Heard, addressed to somebody else: overheard.
        chat.received(
            ShipId(7),
            Some("Ada"),
            202,
            spoken(9, Some("for Bry"), false, vec![]),
            false,
            5.0,
            6.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        // Sent by this ship, to one craft and to nobody.
        chat.sent(
            Some(ShipId(8)),
            Some("Bry"),
            300,
            5,
            Some("mine, to Bry".into()),
            false,
            false,
            7.0,
        );
        chat.sent(
            None,
            None,
            301,
            6,
            Some("mine, to nobody".into()),
            false,
            false,
            8.0,
        );

        let bodies = |v: Vec<&Loose>| -> Vec<String> {
            v.iter().filter_map(|l| l.line.body.clone()).collect()
        };
        assert_eq!(bodies(chat.public()), vec!["to whoever", "mine, to nobody"]);
        assert_eq!(bodies(chat.overheard()), vec!["for Bry"]);
        assert_eq!(
            chat.get(ShipId(7)).unwrap().lines.len(),
            1,
            "the conversation held something that was not between us",
        );
        assert_eq!(
            chat.get(ShipId(7)).unwrap().lines[0].body.as_deref(),
            Some("for you")
        );
    }

    /// An encrypted message meant for somebody else is *heard* and cannot be read. Both halves
    /// matter: it is in the log, and there is nothing in it.
    #[test]
    fn an_encrypted_message_for_somebody_else_is_overheard_as_noise() {
        let mut chat = Chat::default();
        chat.i_am(ShipId(1));
        chat.received(
            ShipId(7),
            Some("Ada"),
            400,
            spoken(9, None, true, vec![]),
            false,
            1.0,
            3.0,
            1.0,
            [1.0, 0.0, 0.0],
        );

        let overheard = chat.overheard();
        assert_eq!(overheard.len(), 1);
        assert_eq!(overheard[0].to, Some(ShipId(9)), "it kept who it was for");
        assert_eq!(overheard[0].line.body, None);
        assert!(
            chat.get(ShipId(7)).is_none(),
            "it became a conversation with the sender"
        );
    }

    /// **Fixed length whatever was said**, or a player could read a conversation's shape
    /// without reading a word of it. Stable, too: it is drawn every frame.
    #[test]
    fn unreadable_messages_are_the_same_length_and_the_same_noise_every_time() {
        let mut chat = Chat::default();
        chat.i_am(ShipId(1));
        chat.received(
            ShipId(7),
            None,
            400,
            spoken(9, None, true, vec![]),
            false,
            1.0,
            3.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        chat.received(
            ShipId(7),
            None,
            401,
            spoken(9, None, true, vec![]),
            false,
            4.0,
            6.0,
            1.0,
            [1.0, 0.0, 0.0],
        );

        let overheard = chat.overheard();
        let first = overheard[0].line.ciphertext();
        let second = overheard[1].line.ciphertext();
        assert_eq!(first.len(), second.len(), "the noise leaked a length");
        assert_eq!(
            first,
            overheard[0].line.ciphertext(),
            "it changed between draws"
        );
        assert_ne!(first, second, "two messages produced the same noise");
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        );
    }

    /// Nothing loose is ever answered automatically. Acknowledging a broadcast would answer
    /// everybody at once, and acknowledging somebody else's mail would tell its sender that a
    /// craft they were not talking to is listening.
    #[test]
    fn nothing_overheard_is_ever_answered_automatically() {
        let mut chat = Chat::default();
        chat.i_am(ShipId(1));
        chat.set_auto_ack(ShipId(7), true);
        assert_eq!(
            chat.received(
                ShipId(7),
                None,
                200,
                broadcast("hello all"),
                false,
                1.0,
                2.0,
                1.0,
                [1.0, 0.0, 0.0]
            ),
            None,
            "a broadcast was answered",
        );
        assert_eq!(
            chat.received(
                ShipId(7),
                None,
                201,
                spoken(9, Some("hello Bry"), false, vec![]),
                false,
                3.0,
                4.0,
                1.0,
                [1.0, 0.0, 0.0]
            ),
            None,
            "somebody else's mail was answered",
        );
        // And one actually addressed here still is.
        assert!(
            chat.received(
                ShipId(7),
                None,
                202,
                spoken(1, Some("hello you"), false, vec![]),
                false,
                5.0,
                6.0,
                1.0,
                [1.0, 0.0, 0.0]
            )
            .is_some()
        );
    }

    /// Oldest first, by when *this ship* learnt of each — which for a conversation across light
    /// delay is not the order they were sent in.
    #[test]
    fn the_public_log_is_ordered_by_when_this_ship_learnt_of_each() {
        let mut chat = Chat::default();
        chat.i_am(ShipId(1));
        // Sent early, heard late: a long crossing.
        chat.received(
            ShipId(7),
            Some("Ada"),
            200,
            broadcast("slow"),
            false,
            1.0,
            90.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        chat.sent(None, None, 300, 5, Some("quick".into()), false, false, 50.0);
        let order: Vec<String> = chat
            .public()
            .iter()
            .filter_map(|l| l.line.body.clone())
            .collect();
        assert_eq!(
            order,
            vec!["quick", "slow"],
            "ordered by sending, not by learning"
        );
    }

    /// The list puts whoever spoke last at the top, which is how a player finds the
    /// conversation they are in the middle of.
    #[test]
    fn conversations_are_listed_most_recently_heard_first() {
        let mut chat = Chat::default();
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            100,
            1,
            Some("early".into()),
            false,
            false,
            1.0,
        );
        chat.sent(
            Some(ShipId(8)),
            Some("Bry"),
            200,
            2,
            Some("later".into()),
            false,
            false,
            5.0,
        );
        let order: Vec<ShipId> = chat.conversations().into_iter().map(|(id, _)| id).collect();
        assert_eq!(order, vec![ShipId(8), ShipId(7)]);

        // And it moves when something lands.
        chat.received(
            ShipId(7),
            Some("Ada"),
            300,
            spoken(1, Some("back"), false, vec![]),
            false,
            6.0,
            9.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
        let order: Vec<ShipId> = chat.conversations().into_iter().map(|(id, _)| id).collect();
        assert_eq!(order, vec![ShipId(7), ShipId(8)]);
    }

    /// **The quiet half of an acknowledgement.** A message with nothing in it is not shown —
    /// there is nothing to read — and what it acknowledges is kept anyway. Dropping both would
    /// make every message look unanswered for ever, which is the failure worth guarding.
    #[test]
    fn a_bare_acknowledgement_is_not_shown_and_still_marks_the_message_delivered() {
        let mut chat = Chat::default();
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            100,
            1,
            Some("are you there".into()),
            false,
            false,
            1.0,
        );
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
        chat.received(
            ShipId(7),
            Some("Ada"),
            200,
            bare,
            false,
            9.0,
            12.0,
            1.0,
            [1.0, 0.0, 0.0],
        );

        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(
            conversation.lines.len(),
            1,
            "an empty acknowledgement was shown as a line"
        );
        assert!(
            conversation.delivered(&sent),
            "the acknowledgement was dropped with the line"
        );
    }

    /// This ship's own empty acknowledgements are as quiet as anybody's. With auto-ack on, a
    /// conversation would otherwise be half blank lines from this end.
    #[test]
    fn this_ships_own_bare_acknowledgements_are_not_shown_either() {
        let mut chat = Chat::default();
        chat.sent(
            Some(ShipId(7)),
            Some("Ada"),
            300,
            9,
            Some(String::new()),
            false,
            false,
            1.0,
        );
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
        chat.received(
            ShipId(7),
            None,
            400,
            sealed,
            false,
            1.0,
            3.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
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
        chat.received(
            ShipId(7),
            None,
            500,
            offer,
            true,
            1.0,
            3.0,
            1.0,
            [1.0, 0.0, 0.0],
        );
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
            chat.received(
                ShipId(7),
                Some("Ada"),
                event_id,
                said,
                false,
                1.0,
                2.0,
                1.0,
                [0.0, 1.0, 0.0],
            )
        };
        assert_eq!(
            heard(&mut chat, 1, false),
            None,
            "answered without being asked to"
        );

        chat.set_auto_ack(ShipId(7), true);
        assert_eq!(heard(&mut chat, 2, false), Some(lc_proto::Aim::Omni));
        // A beam is answered down the bearing it came in on, which is what a dish knows.
        assert_eq!(
            heard(&mut chat, 3, true),
            Some(lc_proto::Aim::Bearing([0.0, 1.0, 0.0]))
        );

        chat.set_auto_ack(ShipId(7), false);
        assert_eq!(
            heard(&mut chat, 4, false),
            None,
            "it kept answering after being told to stop"
        );
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
            chat.received(
                ShipId(7),
                None,
                200,
                bare,
                false,
                1.0,
                2.0,
                1.0,
                [1.0, 0.0, 0.0]
            ),
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
            chat.received(
                ShipId(7),
                None,
                201,
                key,
                true,
                1.0,
                2.0,
                1.0,
                [1.0, 0.0, 0.0]
            ),
            None,
        );
    }

    /// A resend is answered once, not once per attempt at reaching us.
    #[test]
    fn a_resend_does_not_earn_a_second_automatic_answer() {
        let mut chat = Chat::default();
        chat.set_auto_ack(ShipId(7), true);
        let said = |event_id| {
            (
                event_id,
                Spoken {
                    to: Some(1),
                    beamed: false,
                    idem: 42,
                    sealed: false,
                    body: Some("are you there".into()),
                    acks: vec![],
                },
            )
        };
        let (first, a) = said(200);
        assert!(
            chat.received(
                ShipId(7),
                None,
                first,
                a,
                false,
                1.0,
                2.0,
                1.0,
                [1.0, 0.0, 0.0]
            )
            .is_some()
        );
        let (again, b) = said(208);
        assert_eq!(
            chat.received(
                ShipId(7),
                None,
                again,
                b,
                false,
                6.0,
                9.0,
                1.0,
                [1.0, 0.0, 0.0]
            ),
            None,
            "a resend was answered a second time",
        );
    }

    /// A broadcast this ship sent is in the public log and in nobody's conversation, because
    /// there is no conversation it is part of.
    #[test]
    fn a_broadcast_is_in_the_public_log_and_in_no_conversation() {
        let mut chat = Chat::default();
        chat.sent(
            None,
            None,
            300,
            9,
            Some("to whoever is listening".into()),
            false,
            false,
            1.0,
        );
        assert!(
            chat.conversations().is_empty(),
            "a broadcast opened a conversation"
        );
        let public = chat.public();
        assert_eq!(public.len(), 1);
        assert_eq!(
            public[0].from, None,
            "a broadcast this ship sent named a transmitter"
        );
        assert_eq!(public[0].to, None, "a broadcast named an addressee");
        assert_eq!(
            public[0].line.body.as_deref(),
            Some("to whoever is listening")
        );
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
        chat.received(
            ShipId(7),
            None,
            200,
            said,
            false,
            1.0,
            2.0,
            100.0,
            [1.0, 0.0, 0.0],
        );
        let heard = &chat.get(ShipId(7)).unwrap().lines[0];
        assert!(
            (heard.decibels().unwrap() - 20.0).abs() < 1.0e-4,
            "{:?}",
            heard.decibels()
        );

        // Nothing this ship sent has a reading: it never heard it.
        chat.sent(
            Some(ShipId(7)),
            None,
            301,
            2,
            Some("mine".into()),
            false,
            false,
            3.0,
        );
        let mine = chat.get(ShipId(7)).unwrap().lines.last().unwrap();
        assert_eq!(mine.decibels(), None);
    }

    #[test]
    fn a_restored_transcript_replaces_rather_than_merges() {
        let mut chat = Chat::default();
        chat.sent(
            Some(ShipId(7)),
            None,
            1,
            9,
            Some("stale".into()),
            false,
            false,
            0.0,
        );
        chat.restore(
            vec![Said {
                event_id: 9,
                idem: 4,
                with: Some(ShipId(7)),
                to: Some(ShipId(1)),
                with_name: "Ada".into(),
                mine: false,
                key: false,
                sealed: false,
                body: Some("from the store".into()),
                acks: vec![1],
                sent_t: 1_000_000,
                arrive_t: Some(4_000_000),
                strength: Some(4.0),
            }],
            vec![ShipId(7)],
        );
        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1);
        assert_eq!(conversation.name, "Ada");
        assert_eq!(conversation.lines[0].sent_s, 1.0);
        assert!(chat.holds_key(ShipId(7)));
    }

    /// **The reading survives a transcript.** It did not, and a reconnection is the normal
    /// case — so in practice every message a client had came back with no strength at all.
    #[test]
    fn a_restored_message_keeps_how_loudly_it_landed() {
        let mut chat = Chat::default();
        chat.i_am(ShipId(1));
        chat.restore(
            vec![Said {
                event_id: 9,
                idem: 4,
                with: Some(ShipId(7)),
                to: Some(ShipId(1)),
                with_name: "Ada".into(),
                mine: false,
                key: false,
                sealed: false,
                body: Some("from the store".into()),
                acks: vec![],
                sent_t: 1_000_000,
                arrive_t: Some(4_000_000),
                strength: Some(100.0),
            }],
            Vec::new(),
        );
        let line = &chat.get(ShipId(7)).unwrap().lines[0];
        assert!(
            (line.decibels().unwrap() - 20.0).abs() < 1.0e-4,
            "{:?}",
            line.decibels()
        );
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
            to: Some(ShipId(7)),
            with_name: "Ada".into(),
            mine: true,
            key: false,
            sealed: false,
            body: Some("are you there".into()),
            acks: Vec::new(),
            sent_t,
            arrive_t: None,
            strength: None,
        };
        chat.restore(vec![said(1, 1_000_000), said(2, 9_000_000)], Vec::new());
        let conversation = chat.get(ShipId(7)).unwrap();
        assert_eq!(conversation.lines.len(), 1);
        assert_eq!(conversation.lines[0].event_ids, vec![1, 2]);
    }
}
