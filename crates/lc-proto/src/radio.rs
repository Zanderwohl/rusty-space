//! What goes on the air: aim, messages, survey reports, and transcript lines. Everything here is
//! re-exported at the crate root.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::de::DeserializeOwned;
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
    /// For whoever holds this faction key when it lands, which files it in that faction's chat.
    /// Addressed to nobody. Appended last.
    Faction(FactionId),
}

/// A faction key. Minted by the shard, so nobody can found a faction that is somebody else's.
/// See `lightcone/docs/23-factions.md`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FactionId(pub i64);

/// A faction key with the name its founder gave it, which travels with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Faction {
    pub key: FactionId,
    pub name: String,
    /// Who minted it. Two holders can each replace one key, so a lineage forks, and this is
    /// how a receiver tells the branches apart. True, because identity is not forgeable yet.
    pub founder: ShipId,
    /// The keys this one replaces, the one it directly replaces first. Whole rather than one
    /// link, so a holder of generation 3 knows it supersedes 1 without ever having held 2.
    pub lineage: Vec<FactionId>,
}

impl Faction {
    /// 1 for a key that replaces nothing.
    pub fn generation(&self) -> u32 {
        self.lineage.len() as u32 + 1
    }

    /// Whether this key is a later generation of `older`.
    pub fn supersedes(&self, older: FactionId) -> bool {
        self.lineage.contains(&older)
    }
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
    /// A ship's automatic answer, carrying only acknowledgments.
    Ack,
    Key,
    /// Sealed to somebody else.
    Unreadable,
    /// A faction's key, given. Appended last.
    Faction(Faction),
    /// A survey report, in a transcript: how many stars it carried, or `None` sealed to somebody
    /// else. Appended last.
    Report { stars: Option<u32> },
}

impl Body {
    pub fn text(&self) -> Option<&str> {
        match self {
            Body::Text(text) => Some(text),
            _ => None,
        }
    }
}

/// The name a transmitter calls itself: its own newest name entry about itself, with what a
/// receiver needs to file the identical entry, author being the sender. See
/// `lightcone/docs/23-factions.md` §Craft that are not ours.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub name: String,
    /// Coordinate seconds, at the sender.
    pub written_s: f64,
    /// Entry ids, sorted.
    pub parents: Vec<u64>,
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
    /// The faction key it is sealed to. Shown to everyone who hears it, so two messages under
    /// one key are seen to be.
    #[serde(default)]
    pub under: Option<FactionId>,
    /// What the sender calls itself. Readable by anyone who hears it, sealed or not.
    #[serde(default)]
    pub claim: Option<Claim>,
    pub body: Body,
    /// Event ids of the addressee's messages the sender had received, newest last. Ids rather
    /// than a count because the ends disagree on the count. See [`ACK_DEPTH`].
    #[serde(default)]
    pub acks: Vec<i64>,
}

/// A survey report on the air. Not a [`Spoken`]: it is not filed in a conversation and earns no
/// acknowledgment.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Reported {
    /// `None` for a broadcast.
    pub to: Option<i64>,
    #[serde(default)]
    pub beamed: bool,
    #[serde(default)]
    pub idem: MessageKey,
    pub sealed: bool,
    #[serde(default)]
    pub under: Option<FactionId>,
    #[serde(default)]
    pub claim: Option<Claim>,
    /// The `lc_world::knowledge::Report`, as [`encode_report`] writes it. Its shape is part of
    /// this protocol: a change to it is a [`REPORT_FORMAT`] bump. `None` when this receiver may
    /// not read it.
    pub body: Option<String>,
    /// [`REPORT_FORMAT`] when written. Reports outlive the shard that wrote them in the journal,
    /// so a reader must detect a format it cannot parse. Zero predates the field.
    #[serde(default)]
    pub format: u32,
}

/// Bump it when `lc_world::knowledge::Report` changes shape, or how [`encode_report`] writes it.
pub const REPORT_FORMAT: u32 = 12;

/// A report as [`Reported::body`] carries it: postcard, then base64 so it rides in the JSON a
/// journaled payload must be.
/// Not JSON, which has no infinity: an orbit states an unbounded element's error as infinite.
pub fn encode_report<T: Serialize>(report: &T) -> String {
    STANDARD.encode(crate::encode(report))
}

/// The report [`encode_report`] wrote, or why not.
pub fn decode_report<T: DeserializeOwned>(body: &str) -> Result<T, String> {
    let bytes = STANDARD.decode(body).map_err(|why| why.to_string())?;
    crate::decode(&bytes).map_err(|why| why.to_string())
}

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
    /// The faction key it was sealed to: a line in that faction's chat. Appended last.
    pub under: Option<FactionId>,
    /// What the sender called itself when it sent this. Appended last.
    pub claim: Option<String>,
}
