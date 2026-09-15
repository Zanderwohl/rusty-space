//! What goes over the wire, and the gate everything outbound passes through.
//!
//! Bandwidth here is low and the filter is everything. A bug that leaks an event to a client
//! before its light arrives is not a rendering glitch; it deletes the game. So the rule that
//! decides what a client may know is not a convention this crate documents — it is
//! [`Cleared::clear`], the only constructor of the only type the event channel can carry.
//!
//! The wire format is `postcard`: compact, `serde`-based, and not self-describing. Not
//! self-describing is the point rather than a cost — a stale client that half-understood a
//! message would be a client misreading a sighting, and in a game whose correctness *is* the
//! filter that is worse than a refused connection. So the version handshake is strict, and
//! [`golden`] pins the bytes a version encodes to.
//!
//! Transport is elsewhere and is deliberately not visible here. See
//! `lightcone/docs/08-networking.md`.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Bumped whenever anything below changes shape.
///
/// Clients lag server deploys — a browser tab left open across a release is the normal case —
/// so a connection states its version and is refused rather than misread.
pub const PROTOCOL_VERSION: u32 = 8;

/// Who is connected. Assigned by the server; a client never chooses its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ClientId(pub u64);

/// A thing with a worldline that the server owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShipId(pub i64);

/// Where a client is asking to go.
///
/// A mirror of the world's `Course` rather than the type itself. This crate is `serde` and
/// `postcard` and nothing else, and it stays that way: a wire type that was an alias for a
/// world type would make every change to the world model a change to the protocol, which is
/// the thing [`PROTOCOL_VERSION`] exists to make expensive.
///
/// The mirror is kept honest by the conversions in `lc_world::navigation`, which match
/// exhaustively in both directions — a course the world gains and the wire has not learned is
/// a compile error rather than a variant that silently cannot be asked for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Course {
    /// Burn, flip and burn to a point, light-years from the world origin.
    To([f64; 3]),
    /// Circular orbit at an altitude given in radii above the surface.
    Orbit { body: String, altitude_radii: f64, plane: Plane },
    /// The libration point itself.
    Lagrange { body: String, point: LagrangePoint },
    /// A libration orbit about the point, which is what a real mission flies.
    Hangout { body: String, point: LagrangePoint },
    /// Into a body's ring system, in its plane.
    Rings { body: String },
    /// Into a population's band, by its index in the system's list.
    Belt { index: u32 },
    /// Out along the system's axis until everything is behind.
    LeaveSystem,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Plane {
    #[default]
    Equatorial,
    Polar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LagrangePoint {
    L1,
    L2,
}

/// What a client asks its ship to do.
///
/// Deliberately few. Every order has to become an event with a coordinate, and an order that
/// cannot be placed at one instant is not an order, it is a plan.
///
/// A course is an order by that test and a crossing is not: "go to this orbit, at this
/// acceleration, now" happens at an instant, and the flight it implies is worked out from it by
/// the same code at both ends. Sending the solved crossing instead would put a second copy of
/// the answer on the wire, free to disagree with the one the receiver would have computed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Order {
    /// Put out a pulse. The clearest thing another client can be told about late.
    Transmit { power_w: f64 },
    /// Change velocity, as a fraction of `c` on each axis. The burn's start is the event.
    Burn { beta: [f64; 3] },
    /// Fly somewhere and hold there.
    ///
    /// The acceleration is asked for, not stated: it is clamped to what the craft's own drive
    /// can do, so a client cannot ask for a better ship than it has.
    SetCourse { course: Course, accel_g: f64 },
    /// Cross to another star, at this acceleration.
    ///
    /// The star is named by **catalogue id**, not by position. A position would let a client
    /// fly to somewhere it invented; an id can only name a star the server also holds, which
    /// it does because both ends are given the same packed catalogue — see the shard's
    /// `--sky` in `lightcone/docs/15-runbook.md`. The server resolves the id and folds a
    /// coordinate, so the two never plan against different places.
    ///
    /// Separate from [`Order::SetCourse`] because a `Course` names somewhere inside the local
    /// system and this is the one thing a ship does that is not about one.
    Cross { star: u64, accel_g: f64 },
    /// Cut the engine. Not a stop — whatever velocity it had, it keeps, on whatever conic that
    /// puts it on.
    CutDrive,
}

/// A client's request. Never authoritative about anything.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
/// The field is private, so nothing in this process can make one except the gate:
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
///
/// Deserialising one is not a hole in that, and it is worth being exact about why. A `Cleared`
/// read off the wire is a claim by whoever sent it — it says *a server released this*, not
/// *this passed our gate*. The invariant is about what a process emits, and a server only ever
/// emits ones it built with [`Cleared::clear`]; the client is the end that decodes them.
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
    Welcome {
        client_id: ClientId,
        protocol: u32,
        ship_id: ShipId,
        now_t: i64,
        name: String,
        /// Where the ship is, in light-years. Without it the client has no idea: it knows the
        /// clock and its own identity and would place its ship wherever it happened to start,
        /// which is the origin — empty space, and not where the server has it.
        ///
        /// Enough for a ship at rest, which is what a new one is. A ship found **mid-flight**
        /// needs its motive as well, and that is the resume problem rather than this one: see
        /// `Inbound::ResumeFrom` and `lightcone/docs/17-reconciliation.md`.
        ship_at: [f64; 3],
    },
    /// The event channel. Cleared, by construction.
    Sightings(Vec<Cleared<Sighting>>),
    /// What time it is, stated periodically.
    ///
    /// The client runs its own clock between these — it has to, because it draws frames far
    /// faster than this arrives — and that clock drifts. A browser tab in the background is the
    /// honest case: its frames are throttled, so its clock nearly stops while the world does
    /// not, and it comes back hours of coordinate time behind.
    ///
    /// Without this the client and the server disagree about *when* the ship is, which reads as
    /// disagreeing about **where** it is: an order the server stamps in the client's past folds
    /// as a manoeuvre that has already finished, so the ship appears to teleport to its
    /// destination, and the server goes on refusing orders about a system it does not think the
    /// ship has reached.
    Clock { now_t: i64 },
    /// An intent that stood, and **what was actually done with it** — which is not always
    /// what was asked for.
    ///
    /// The server silently clamps two things: the intent's timestamp, to the window the client
    /// can prove it is entitled to, and the acceleration, to the craft's own ceiling. A client
    /// predicting from what it *sent* would diverge by exactly those amounts with no way to
    /// see why. Carrying the order back as applied is what makes the difference observable
    /// rather than mysterious. See `lightcone/docs/17-reconciliation.md`.
    Accepted {
        ship_id: ShipId,
        /// The event this became, so a client can match its own act to the sighting of it that
        /// arrives later.
        event_id: i64,
        /// Coordinate time it took effect: the clamp, applied.
        at_t: i64,
        /// The order **as applied**, not as sent.
        order: Order,
    },
    /// An intent that did not survive validation, and why. Not an error: a client is allowed
    /// to ask for things it cannot have, and being told no is how it finds out.
    Refused { ship_id: ShipId, reason: Refusal },
    /// The protocol version did not match. The last thing sent on that connection.
    WrongProtocol { server: u32 },
    /// The ticket did not verify, or none was offered before something that needed one. The
    /// last thing sent on that connection: there is nobody to keep talking to.
    ///
    /// It says nothing about *why*. A client that learned whether its ticket was expired, or
    /// spent, or for another server, would have learned how close it got.
    Unauthenticated,
    /// This client is sending faster than the server will take, and the message was dropped
    /// unread. Not a disconnection: a client that hits this has a bug, and is told so it can be
    /// fixed. Nothing about the world leaks through it — it is a fact about the client's own
    /// sending and reveals nothing that was withheld.
    Throttled { retry_after_ticks: u32 },
}

/// Why an intent was not acted on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// No such ship, or it is not this client's.
    NotYours,
    /// The ticket did not verify: wrong audience, expired, already spent, or not signed by a
    /// key this server publishes trust in. **Deliberately one variant** — a client learning
    /// *which* is a client learning how close it got.
    NotYou,
    /// The order itself is impossible — a burn past `c`, a transmitter at negative power.
    Impossible,
}

/// Everything a client says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Inbound {
    /// The first message, and the only one accepted before it.
    ///
    /// The ticket is a signed assertion from the identity broker, audience-scoped to this
    /// server and worth sixty seconds. The server verifies it against a published key without
    /// calling the broker — see `lightcone/docs/16-identity.md`. A connection that sends
    /// anything else first is closed rather than refused: there is nobody to refuse.
    Hello { protocol: u32, ticket: String },
    Act(Intent),
    /// Reconnecting: replay from the last reception this client actually has.
    ResumeFrom { arrive_t: i64 },
}

/// Encode anything the protocol carries.
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    // Infallible for these types: they contain no map with non-string keys and no borrowed
    // data, which are the only shapes postcard rejects.
    postcard::to_stdvec(value).expect("the protocol's own types encode")
}

/// Decode. A failure means the peer is not speaking this version, whatever it claimed.
pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, postcard::Error> {
    postcard::from_bytes(bytes)
}

/// The bytes this protocol version encodes to.
///
/// A format that is not self-describing cannot notice a field that moved, so this is what
/// notices: change the shape of anything above without bumping [`PROTOCOL_VERSION`] and the
/// test on these fails. A deployed client would otherwise read the new shape as the old one and
/// be confidently wrong rather than refused.
pub mod golden {
    /// `Outbound::Welcome { client_id: 7, .., ship_id: 42, now_t: 1e6, ship_at: [4.2, 0, 0] }`
    pub const WELCOME: &[u8] = &[
        0, 7, 8, 84, 128, 137, 122, 3, 65, 100, 97, 205, 204, 204, 204, 204, 204, 16, 64, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    /// `Inbound::Act(Intent { ship_id: 42, order: Transmit { power_w: 1500.0 }, .. })`
    pub const ACT: &[u8] =
        &[1, 84, 0, 0, 0, 0, 0, 0, 112, 151, 64, 128, 137, 122];

    /// `Inbound::Act(Intent { ship_id: 42, order: SetCourse { Orbit of Earth, polar, 2 radii,
    /// 5 g }, .. })`
    ///
    /// Pinned as well as the other two because a course is the first thing on this wire with a
    /// *shape* — nested enums, a string, two floats — rather than a number. It is the message
    /// most able to move a field without anyone noticing.
    /// `Inbound::Hello { protocol: PROTOCOL_VERSION, ticket: "a.b.c" }`
    ///
    /// Pinned because it is now the message that decides whether anyone gets in at all. A
    /// field moving here is a server reading someone else's ticket as this one's.
    pub const HELLO: &[u8] = &[0, 8, 5, 97, 46, 98, 46, 99];

    pub const SET_COURSE: &[u8] = &[
        1, 84, 2, 1, 5, 69, 97, 114, 116, 104, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0, 0, 0, 0, 0, 0, 20,
        64, 128, 137, 122,
    ];

    /// `Inbound::Act(Intent { ship_id: 42, order: Cross { star: 0x0123456789abcdef, 3 g }, .. })`
    ///
    /// Pinned because a star id is the one field on this wire whose bytes nobody can eyeball:
    /// it is a hash, so a shifted field reads as a different star rather than as nonsense.
    pub const CROSS: &[u8] = &[
        1, 84, 3, 239, 155, 175, 205, 248, 172, 209, 145, 1, 0, 0, 0, 0, 0, 0, 8, 64, 128, 137,
        122,
    ];

    /// `Outbound::Accepted { ship_id: 42, event_id: 9, at_t: 1e6, order: SetCourse { .. 3 g } }`
    ///
    /// Pinned because it is the message a client reconciles against. A field moving here is a
    /// client folding the wrong number into where it believes its own ship is.
    pub const ACCEPTED: &[u8] = &[
        3, 84, 18, 128, 137, 122, 2, 1, 5, 69, 97, 114, 116, 104, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0,
        0, 0, 0, 0, 0, 8, 64,
    ];
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

    fn welcome() -> Outbound {
        Outbound::Welcome {
            client_id: ClientId(7),
            protocol: PROTOCOL_VERSION,
            ship_id: ShipId(42),
            now_t: 1_000_000,
            name: "Ada".into(),
            ship_at: [4.2, 0.0, 0.0],
        }
    }

    fn act() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::Transmit { power_w: 1500.0 },
            issued_at_client_t: 1_000_000,
        })
    }

    /// What a version encodes to, pinned.
    ///
    /// A format that is not self-describing cannot notice a field that moved, so this is what
    /// notices. If this fails, either the change was unintended or `PROTOCOL_VERSION` needs
    /// bumping and these bytes need replacing — and a deployed client needs to be refused
    /// rather than left reading the new shape as the old one.
    fn set_course() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::SetCourse {
                course: Course::Orbit {
                    body: "Earth".into(),
                    altitude_radii: 2.0,
                    plane: Plane::Polar,
                },
                accel_g: 5.0,
            },
            issued_at_client_t: 1_000_000,
        })
    }

    fn accepted() -> Outbound {
        Outbound::Accepted {
            ship_id: ShipId(42),
            event_id: 9,
            at_t: 1_000_000,
            // Not the acceleration that would have been asked for: the point of the message is
            // that this is the applied value.
            order: Order::SetCourse {
                course: Course::Orbit {
                    body: "Earth".into(),
                    altitude_radii: 2.0,
                    plane: Plane::Polar,
                },
                accel_g: 3.0,
            },
        }
    }

    fn cross() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::Cross { star: 0x0123_4567_89ab_cdef, accel_g: 3.0 },
            issued_at_client_t: 1_000_000,
        })
    }

    #[test]
    fn the_wire_format_for_this_version_has_not_moved() {
        assert_eq!(
            encode(&welcome()),
            golden::WELCOME,
            "Outbound::Welcome changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&act()),
            golden::ACT,
            "Inbound::Act changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&Inbound::Hello { protocol: PROTOCOL_VERSION, ticket: "a.b.c".into() }),
            golden::HELLO,
            "Inbound::Hello changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&set_course()),
            golden::SET_COURSE,
            "Order::SetCourse changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&cross()),
            golden::CROSS,
            "Order::Cross changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&accepted()),
            golden::ACCEPTED,
            "Outbound::Accepted changed shape at protocol version {PROTOCOL_VERSION}",
        );
    }

    #[test]
    fn everything_the_protocol_carries_survives_a_round_trip() {
        let out = [
            welcome(),
            Outbound::Sightings(vec![
                Cleared::clear(sighting(500, 2.5), 1_000, 0.0).unwrap(),
            ]),
            accepted(),
            Outbound::Clock { now_t: 1_000_000 },
            Outbound::Refused { ship_id: ShipId(-3), reason: Refusal::NotYours },
            Outbound::WrongProtocol { server: 9 },
        ];
        for message in out {
            let bytes = encode(&message);
            assert_eq!(decode::<Outbound>(&bytes).unwrap(), message);
        }
        let inbound = [
            Inbound::Hello { protocol: PROTOCOL_VERSION, ticket: "a.b.c".into() },
            Inbound::Hello { protocol: PROTOCOL_VERSION, ticket: String::new() },
            act(),
            cross(),
            Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: Order::Burn { beta: [0.1, -0.2, 0.3] },
                issued_at_client_t: i64::MIN,
            }),
            Inbound::ResumeFrom { arrive_t: -1 },
        ];
        for message in inbound {
            let bytes = encode(&message);
            assert_eq!(decode::<Inbound>(&bytes).unwrap(), message);
        }
    }

    /// Rubbish is a decode error, not a message. A format that is not self-describing will
    /// happily read the wrong shape, so this is only a check that it fails when it can.
    #[test]
    fn nonsense_does_not_decode_into_a_message() {
        assert!(decode::<Outbound>(&[255, 255, 255]).is_err());
        assert!(decode::<Inbound>(&[]).is_err());
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
