//! What goes over the wire, and the gate everything outbound passes through.
//!
//! Bandwidth here is low and the filter is everything. A bug that leaks an event to a client
//! before its light arrives is not a rendering glitch; it deletes the game. So the rule that
//! decides what a client may know is not a convention this crate documents — it is
//! [`Cleared::clear`], the only constructor of the only type the event channel can carry.
//!
//! No transport and no serialisation format: both are still open, and the types derive `serde`
//! so that neither choice reaches back into this. See `lightcone/docs/08-networking.md`.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Bumped whenever anything below changes shape.
///
/// Clients lag server deploys — a browser tab left open across a release is the normal case —
/// so a connection states its version and is refused rather than misread.
pub const PROTOCOL_VERSION: u32 = 1;

/// Who is connected. Assigned by the server; a client never chooses its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClientId(pub u64);

/// A thing with a worldline that the server owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShipId(pub i64);

/// What a client asks its ship to do.
///
/// Deliberately few. Every order has to become an event with a coordinate, and an order that
/// cannot be placed at one instant is not an order, it is a plan.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Order {
    /// Put out a pulse. The clearest thing another client can be told about late.
    Transmit { power_w: f64 },
    /// Change velocity, as a fraction of `c` on each axis. The burn's start is the event.
    Burn { beta: [f64; 3] },
}

/// A client's request. Never authoritative about anything.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Intent {
    pub ship_id: ShipId,
    pub order: Order,
    /// When the client believes it issued this, coordinate microseconds.
    ///
    /// Advisory, and clamped on arrival. A client that stamps an intent earlier than the last
    /// event it can prove it received is claiming to have acted on information it did not have.
    pub issued_at_client_t: i64,
}

/// One event arriving at one observer: what the client is actually told.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sighting {
    pub event_id: i64,
    pub source_id: i64,
    /// Coordinate time the light arrives. Never later than the server's `t` when it is sent.
    pub arrive_t: i64,
    /// Coordinate time it was emitted. Always earlier, and usually by a great deal.
    pub emitted_t: i64,
    /// Unit vector toward where the source appears to be.
    pub direction: [f64; 3],
    pub strength: f32,
    pub kind: i16,
    pub payload: String,
}

/// A sighting that has passed the gate, and the only thing the event channel can carry.
///
/// The point of the wrapper is that it cannot be built any other way. "Every outbound message
/// passes one gate" is then a fact the compiler enforces rather than a rule a reviewer has to
/// notice being broken: there is no second path to a socket because there is no second way to
/// make one of these.
///
/// The field is private, so from outside this crate there is no route round [`Cleared::clear`]:
///
/// ```compile_fail
/// # use lc_proto::{Cleared, Sighting};
/// let forged: Cleared<Sighting> = Cleared { inner: todo!() };
/// ```
///
/// and none through a pattern either:
///
/// ```compile_fail
/// # use lc_proto::{Cleared, Sighting};
/// fn unwrap(c: Cleared<Sighting>) -> Sighting {
///     let Cleared { inner } = c;
///     inner
/// }
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cleared<T> {
    inner: T,
}

/// Why a sighting was not cleared. Returned rather than logged, so a caller cannot ignore it
/// by accident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Withheld {
    /// Its light has not reached the observer yet. The hard gate.
    StillInFlight,
    /// It reached the observer, below the noise floor. Arrival is not detection.
    BelowNoiseFloor,
}

impl Cleared<Sighting> {
    /// **The gate.** The only way a sighting becomes sendable.
    ///
    /// Two tests, in the order they matter. The light cone is the one that cannot be relaxed:
    /// `arrive_t <= now_t` or the client is being told something it could not know, and there
    /// is no threshold, subscription or optimisation that may be allowed to reverse it. The
    /// noise floor is second and prunes far more, but it is a detection rule rather than a
    /// causality one.
    pub fn clear(sighting: Sighting, now_t: i64, noise_floor: f32) -> Result<Self, Withheld> {
        if sighting.arrive_t > now_t {
            return Err(Withheld::StillInFlight);
        }
        if sighting.strength < noise_floor {
            return Err(Withheld::BelowNoiseFloor);
        }
        Ok(Self { inner: sighting })
    }
}

impl<T> Cleared<T> {
    pub fn get(&self) -> &T {
        &self.inner
    }

    pub fn into_inner(self) -> T {
        self.inner
    }
}

/// Everything the server says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Outbound {
    /// The control channel's first message. A client that gets anything else first is talking
    /// to a server it does not understand.
    Welcome { client_id: ClientId, protocol: u32, ship_id: ShipId, now_t: i64 },
    /// The event channel. Cleared, by construction.
    Sightings(Vec<Cleared<Sighting>>),
    /// An intent that did not survive validation, and why. Not an error: a client is allowed
    /// to ask for things it cannot have, and being told no is how it finds out.
    Refused { ship_id: ShipId, reason: Refusal },
    /// The protocol version did not match. The last thing sent on that connection.
    WrongProtocol { server: u32 },
}

/// Why an intent was not acted on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// No such ship, or it is not this client's.
    NotYours,
    /// The order itself is impossible — a burn past `c`, a transmitter at negative power.
    Impossible,
}

/// Everything a client says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Inbound {
    Hello { protocol: u32 },
    Act(Intent),
    /// Reconnecting: replay from the last reception this client actually has.
    ResumeFrom { arrive_t: i64 },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sighting(arrive_t: i64, strength: f32) -> Sighting {
        Sighting {
            event_id: 1,
            source_id: 2,
            arrive_t,
            emitted_t: arrive_t - 1_000,
            direction: [1.0, 0.0, 0.0],
            strength,
            kind: 0,
            payload: "{}".into(),
        }
    }

    /// The negative case, which is the one that matters. Everything else in this crate is
    /// plumbing; this is the rule the game is made of.
    #[test]
    fn nothing_still_in_flight_gets_through() {
        let now = 1_000_000;
        assert_eq!(
            Cleared::clear(sighting(now + 1, 1.0), now, 0.0),
            Err(Withheld::StillInFlight),
            "a sighting one microsecond early was cleared",
        );
        // The boundary is inclusive: light arriving exactly now has arrived.
        assert!(Cleared::clear(sighting(now, 1.0), now, 0.0).is_ok());
        assert!(Cleared::clear(sighting(now - 1, 1.0), now, 0.0).is_ok());
    }

    /// Arrival is not detection, and the two refusals are distinguishable — a client that is
    /// told nothing must not be able to tell which of the two happened.
    #[test]
    fn arrival_is_not_detection() {
        let now = 1_000_000;
        assert_eq!(
            Cleared::clear(sighting(now, 0.5), now, 1.0),
            Err(Withheld::BelowNoiseFloor),
        );
        assert!(Cleared::clear(sighting(now, 1.0), now, 1.0).is_ok(), "exactly at the floor");
        // In flight *and* faint is reported as in flight: the causality test comes first and
        // is the one that may never be relaxed.
        assert_eq!(
            Cleared::clear(sighting(now + 1, 0.0), now, 1.0),
            Err(Withheld::StillInFlight),
        );
    }
}
