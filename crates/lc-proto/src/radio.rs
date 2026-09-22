//! What goes on the air, and what comes back down.
//!
//! Split out of the crate root because it is a subject rather than a section: where a
//! transmission is pointed, what a message carries, what a survey report carries, and the
//! transcript line a conversation is made of. Everything here is re-exported at the crate
//! root — the wire does not care which module a type was written in, and neither should a
//! caller.

use serde::{Deserialize, Serialize};

use crate::ShipId;

/// Where a transmission is pointed.
///
/// Not a power setting in disguise. An omnidirectional pulse and a beam of the same wattage
/// carry the same energy; the beam concentrates it, so it is heard further along its axis and
/// not at all off it. What that buys and what it costs is `lc_world::signal` and
/// `lightcone/docs/05-observation.md`.
///
/// Not `Eq`, because [`Aim::Bearing`] carries floats. Nothing compares two aims for equality;
/// the interface compares its own choice of aim, which is a separate type for that reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Aim {
    /// In every direction. Reaches everyone in range, and tells all of them where you are.
    #[default]
    Omni,
    /// At where a craft is **predicted** to be when the light lands.
    ///
    /// The prediction is built from the sender's own sighting of it, which is already old, and
    /// then extrapolated forward by the flight time. A quarry that maneuveres in between is
    /// missed. Refused with [`Refusal::NotInSight`] for a craft the sender has never seen.
    Ship(ShipId),
    /// At a star, by catalogue id: the whole system, for when you do not know where in it they
    /// are. A star does not maneuvere, so this always lands — on everybody there.
    Star(u64),
    /// Straight back along a bearing, as a unit vector in world axes.
    ///
    /// What a **directional antenna** can do that nothing else here can: answer a beam without
    /// knowing who sent it or where they are, because the dish already knows which way the
    /// signal came in. No sighting is needed and none is consulted.
    ///
    /// It is a bearing and not a target, so it is aimed at where the sender *was* when the
    /// light left them — not where they will be when the answer arrives. A craft that has been
    /// under thrust since is missed, and by more the further away it is. Appended last.
    Bearing([f64; 3]),
}

/// Whether anyone but the addressee can read it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Secrecy {
    /// Plain. Anyone whose receiver the signal reaches can read it, addressed to them or not.
    #[default]
    Open,
    /// For the addressee alone. Everyone else hears that *something* was sent and gets no
    /// body — which is the truth about a signal you cannot decrypt, not a courtesy.
    ///
    /// Requires the sender to hold the addressee's key. See [`Order::OfferKey`].
    Sealed,
}

/// What makes two transmissions the *same message*.
///
/// A resend is a second pulse of light and a second event — it really happened, at its own
/// coordinate, and the store records both. What it is not is a second thing somebody said, so
/// the receiver collapses them into one line by this.
///
/// **Not a sequence number.** The two ends do not agree about how many messages exist, because
/// half of them are in flight, so a counter would have to be reconciled and there is nothing to
/// reconcile it with. This only has to be unique to the sender, which a hash of who sent it,
/// when, and what it said already is.
pub type MessageKey = u64;

/// The longest message body, in bytes.
///
/// A bound on what one client can make a server store and fan out to every receiver in range,
/// not a claim about bandwidth. A signal that carries a megabyte and one that carries a
/// sentence take the same time to cross a light-year.
pub const MESSAGE_LIMIT: usize = 512;

/// The longest survey report, in bytes.
///
/// A report is not a sentence — it is sixty-four stars of bearings, curves and who said what
/// about them, which is tens of kilobytes. Same argument as [`MESSAGE_LIMIT`] and a different
/// number: it bounds what one client can make a shard fan out, and the sender splits a backlog
/// across as many transmissions as it takes. See `lightcone/docs/22-provenance.md`.
pub const REPORT_LIMIT: usize = 64 * 1024;

/// What a message holds, as one receiver has it.
///
/// An enum rather than a string whose emptiness means something: an acknowledgement and a key
/// offer both used to be an empty body, and every place that showed messages had to remember
/// to test for that.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Body {
    /// Something somebody typed. Never empty.
    Text(String),
    /// Nothing but its acknowledgements: a ship's automatic answer. Nothing to show.
    Ack,
    /// A public key handed over.
    Key,
    /// Sealed to somebody else. That it was said is all this receiver may know.
    Unreadable,
}

impl Body {
    pub fn text(&self) -> Option<&str> {
        match self {
            Body::Text(text) => Some(text),
            _ => None,
        }
    }
}

/// What a [`kind::MESSAGE`] or [`kind::KEY`] event carries, as JSON in the payload.
///
/// **Written once and redacted on the way out.** The event stored is the whole message; the
/// copy each receiver is handed has its body removed unless that receiver may read it. Doing it
/// at release rather than at write is what keeps one event one event — a sealed message
/// duplicated per receiver would be several events with one emission time, and the whole model
/// rests on an event being a point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spoken {
    /// Who it was addressed to, or `None` for a broadcast. Everyone else in earshot is an
    /// eavesdropper.
    pub to: Option<i64>,
    /// Whether it went out as a beam rather than in every direction.
    ///
    /// A byte the transmitter sets, which is how a receiver can answer in the mode it was
    /// spoken to in without knowing anything about the sender. Pair it with the bearing the
    /// signal arrived on — [`Sighting::direction`] — and a dish can reply down the same line it
    /// listened on. See [`Aim::Bearing`].
    #[serde(default)]
    pub beamed: bool,
    /// Which message this is, across however many times it was transmitted. See
    /// [`MessageKey`]; a receiver that has this one already shows one line, not two.
    #[serde(default)]
    pub idem: MessageKey,
    /// Whether it was sealed. With a [`Body::Text`], you are the addressee.
    pub sealed: bool,
    pub body: Body,
    /// Event ids of the addressee's last messages that the sender had received when this went
    /// out, newest last.
    ///
    /// By identifier rather than by count, because the sender and the addressee do not agree
    /// about how many messages exist: half of them are still in flight. An identifier names one
    /// message and means the same thing at both ends. See [`ACK_DEPTH`].
    #[serde(default)]
    pub acks: Vec<i64>,
}

/// A survey report on the air.
///
/// Shaped like [`Spoken`] and deliberately not the same type: a report is not something
/// somebody said. It is not filed in a conversation, it earns no acknowledgement, and a
/// receiver folds it into what it knows rather than reading it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reported {
    /// Who it was addressed to, or `None` for a broadcast to whoever is listening.
    pub to: Option<i64>,
    /// Whether it went out as a beam rather than in every direction.
    #[serde(default)]
    pub beamed: bool,
    /// Which report this is, across however many times it was transmitted.
    #[serde(default)]
    pub idem: MessageKey,
    pub sealed: bool,
    /// The serialized report, as `lc_world::knowledge::Report` writes it. Its shape is part of
    /// this protocol even though it travels as a string: a change to it is a version bump.
    /// `None` when this receiver may not read it.
    pub body: Option<String>,
    /// Which shape `body` is in: [`REPORT_FORMAT`] when written. A report outlives the process
    /// that sent it — it is in the journal until its light has passed everyone — so a shard
    /// reading one written by an older shard needs to know it cannot read it, rather than fail
    /// to parse it and say nothing. Zero is a report from before this was stated.
    #[serde(default)]
    pub format: u32,
}

/// The shape of a report's body today. Bump it when `lc_world::knowledge::Report` changes shape.
pub const REPORT_FORMAT: u32 = 2;

/// How many of the addressee's messages an outgoing one acknowledges.
///
/// Enough that a reply covers a burst, small enough that the payload stays a payload. There is
/// no retransmission behind it and there cannot be: a message that did not arrive is light that
/// went somewhere else, and nothing at either end can notice.
pub const ACK_DEPTH: usize = 10;

/// One message in a ship's own record of a conversation.
///
/// Both halves of it — what this ship said, and what reached it — because a transcript with one
/// side missing is not a transcript. The two are not symmetric and the type says so: a message
/// this ship sent has no arrival time, because a sender never hears its own signal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Said {
    pub event_id: i64,
    /// Which message this is. A backlog carries one entry per *transmission*, so a message
    /// sent three times is three entries sharing this and the client folds them into one.
    pub idem: MessageKey,
    /// The other craft in this conversation: who it went to, or who it came from. `None` for a
    /// broadcast this ship sent, which is in nobody's conversation and only in the public log.
    pub with: Option<ShipId>,
    /// Who it was **addressed** to, which is not always who it reached. `None` is a broadcast.
    ///
    /// Distinct from [`Said::with`] and carried alongside it, because a transcript holds
    /// everything that landed on this ship — traffic between two *other* craft included. For
    /// that, `with` is the sender and this is somebody else entirely, and telling the two apart
    /// is the difference between a conversation and something overheard. Appended last.
    pub to: Option<ShipId>,
    /// What to call them. Carried because a backlog names craft that are nowhere in sight, and
    /// there is no contact to read a name off.
    pub with_name: String,
    /// True when this ship sent it.
    pub mine: bool,
    pub sealed: bool,
    pub body: Body,
    pub acks: Vec<i64>,
    /// Coordinate microseconds it was transmitted.
    pub sent_t: i64,
    /// Coordinate microseconds the light landed. `None` for one this ship sent.
    pub arrive_t: Option<i64>,
    /// How loud it was when it landed, in the units the noise floor is compared against.
    /// `None` for one this ship sent, and for one recorded before the store kept the reading.
    /// Appended last.
    pub strength: Option<f32>,
}
