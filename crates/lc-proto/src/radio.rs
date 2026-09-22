//! What goes on the air: aim, messages, survey reports, and transcript lines. Everything here is
//! re-exported at the crate root.

use serde::{Deserialize, Serialize};

use crate::ShipId;

/// Where a transmission is pointed. A beam carries the same energy as an omni pulse, concentrated
/// along its axis; see `lc_world::signal` and `lightcone/docs/05-observation.md`.
///
/// Not `Eq`, because [`Aim::Bearing`] carries floats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Aim {
    #[default]
    Omni,
    /// At where a craft is predicted to be when the light lands, extrapolated from the sender's
    /// own already-old sighting. Refused with [`Refusal::NotInSight`](crate::Refusal::NotInSight) for a craft never seen.
    Ship(ShipId),
    /// At a star, by catalog id: the whole system.
    Star(u64),
    /// Back along a bearing, as a unit vector in world axes. Lets a dish answer a beam without a
    /// sighting. Aimed where the sender was when the light left, so a craft that has thrust since
    /// is missed. Appended last.
    Bearing([f64; 3]),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Secrecy {
    #[default]
    Open,
    /// For the addressee alone; others learn only that something was sent. Requires the sender
    /// to hold the addressee's key. See [`Order::OfferKey`](crate::Order::OfferKey).
    Sealed,
}

/// What makes two transmissions the same message, so a receiver shows a resend as one line.
///
/// Not a sequence number: the two ends cannot agree on a count while messages are in flight. A
/// hash of sender, time and text only has to be unique to the sender.
pub type MessageKey = u64;

/// The longest message body, in bytes. Bounds what one client can make a server store and fan
/// out; not a bandwidth limit.
pub const MESSAGE_LIMIT: usize = 512;

/// The longest survey report, in bytes. Sixty-four stars of data is tens of kilobytes; the
/// sender splits a larger backlog across transmissions. See `lightcone/docs/22-provenance.md`.
pub const REPORT_LIMIT: usize = 64 * 1024;

/// What a message holds, as one receiver has it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Body {
    /// Never empty.
    Text(String),
    /// A ship's automatic answer, carrying only acknowledgements.
    Ack,
    Key,
    /// Sealed to somebody else.
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

/// What a [`kind::MESSAGE`](crate::kind::MESSAGE) or [`kind::KEY`](crate::kind::KEY) event carries, as JSON in the payload.
///
/// Stored whole and redacted per receiver on release, not duplicated per receiver at write: an
/// event must stay one point with one emission time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spoken {
    /// `None` for a broadcast.
    pub to: Option<i64>,
    /// Lets a receiver reply in the same mode; with [`Sighting::direction`](crate::Sighting::direction) a dish can answer
    /// down the arrival bearing. See [`Aim::Bearing`].
    #[serde(default)]
    pub beamed: bool,
    #[serde(default)]
    pub idem: MessageKey,
    /// With a [`Body::Text`], you are the addressee.
    pub sealed: bool,
    pub body: Body,
    /// Event ids of the addressee's messages the sender had received, newest last. Ids rather
    /// than a count because the ends disagree on the count. See [`ACK_DEPTH`].
    #[serde(default)]
    pub acks: Vec<i64>,
}

/// A survey report on the air. Not a [`Spoken`]: it is not filed in a conversation and earns no
/// acknowledgement.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reported {
    /// `None` for a broadcast.
    pub to: Option<i64>,
    #[serde(default)]
    pub beamed: bool,
    #[serde(default)]
    pub idem: MessageKey,
    pub sealed: bool,
    /// The serialized `lc_world::knowledge::Report`. Its shape is part of this protocol: a change
    /// to it is a version bump. `None` when this receiver may not read it.
    pub body: Option<String>,
    /// [`REPORT_FORMAT`] when written. Reports outlive the shard that wrote them in the journal,
    /// so a reader must detect a format it cannot parse. Zero predates the field.
    #[serde(default)]
    pub format: u32,
}

/// Bump it when `lc_world::knowledge::Report` changes shape.
pub const REPORT_FORMAT: u32 = 2;

/// How many of the addressee's messages an outgoing one acknowledges. There is no
/// retransmission behind it: a lost message is undetectable at either end.
pub const ACK_DEPTH: usize = 10;

/// One message in a ship's own transcript, sent or received.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Said {
    pub event_id: i64,
    /// A backlog has one entry per transmission; the client folds entries sharing this.
    pub idem: MessageKey,
    /// Who it went to or came from. `None` for a broadcast this ship sent.
    pub with: Option<ShipId>,
    /// Who it was addressed to; `None` is a broadcast. Differs from [`Said::with`] for
    /// overheard traffic between two other craft. Appended last.
    pub to: Option<ShipId>,
    /// Carried because a backlog names craft not in sight.
    pub with_name: String,
    pub mine: bool,
    pub sealed: bool,
    pub body: Body,
    pub acks: Vec<i64>,
    /// Coordinate microseconds.
    pub sent_t: i64,
    /// Coordinate microseconds. `None` for one this ship sent.
    pub arrive_t: Option<i64>,
    /// In the units the noise floor is compared against. `None` for one this ship sent, and for
    /// one recorded before the store kept the reading. Appended last.
    pub strength: Option<f32>,
}
