//! Event kinds, as [`Sighting::kind`](crate::Sighting::kind) carries them. Named here because both
//! ends read them.

pub const TRANSMIT: i16 = 1;
/// An order that lights the drive, stamped when it was given.
pub const BURN: i16 = 2;
pub const CUT: i16 = 3;
/// The drive lit, went out or changed power, stamped when it did. The payload is a
/// [`super::DriveChange`] as JSON.
pub const DRIVE: i16 = 4;
/// Somebody said something. The payload is a [`super::Spoken`] as JSON, **redacted per
/// receiver**: a sealed message reaches an eavesdropper as [`super::Body::Unreadable`].
pub const MESSAGE: i16 = 5;
/// Somebody sent what they have learned. The payload is a [`super::Reported`] as JSON,
/// **redacted per receiver** exactly as a message is: a sealed report reaches an
/// eavesdropper as the fact that a report went out, with nothing in it.
pub const REPORT: i16 = 7;
/// Somebody put their public key on the air. The payload is a [`super::Spoken`] too, with
/// [`super::Body::Key`] — it lands in the same conversation, because that is where a player
/// looks for it. Receiving one is what puts the source in the receiver's keyring.
pub const KEY: i16 = 6;
/// A teleport's departure, stamped where the craft was.
pub const VANISH: i16 = 8;
/// Its arrival, at the same coordinate time. Each end is seen at its own light delay.
pub const APPEAR: i16 = 9;
/// A field reached `Q_max` and the ship is gone, stamped when and where. The payload is a
/// [`super::Released`] as JSON.
pub const COLLAPSE: i16 = 10;
/// A field switch completed, stamped when and where: what it absorbs and reflects changed then.
/// The payload is a [`super::ShadeChange`] as JSON.
pub const SHADE: i16 = 11;
/// An emission lit or went out, stamped when and where. Its payload says
/// what it sends and along which cone; only craft inside that cone are delivered it.
pub const EMIT: i16 = 12;
