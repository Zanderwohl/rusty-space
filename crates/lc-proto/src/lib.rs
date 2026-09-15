//! What goes over the wire, and the gate everything outbound passes through.
//!
//! Bandwidth here is low and the filter is everything. A bug that leaks an event to a client
//! before its light arrives is not a rendering glitch; it deletes the game. So the rule that
//! decides what a client may know is not a convention this crate documents — it is
//! [`Cleared`], whose only constructors are its two `clear` gates and which is the only type
//! the channels carrying [`Sighting`] and [`Presence`] can hold.
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
pub const PROTOCOL_VERSION: u32 = 10;

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

/// What a ship's engine can do. Mirrors `lc_world::flight::Drive`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drive {
    /// Proper acceleration, in g.
    pub accel_g: f64,
    /// Speed cap as a fraction of `c`.
    pub max_beta: f64,
}

/// What an orbit is about.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Anchor {
    Star,
    Body(String),
}

/// A place a ship can be held, evaluated at any coordinate time.
///
/// Unlike [`Course`] this is a *resolved* place, not a request: the body has been looked up,
/// the altitude has become a radius and the plane has become a pole. A course is what a player
/// asks for and this is where the ship actually is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Waypoint {
    /// Light-years from the world origin.
    Fixed([f64; 3]),
    Orbit { about: Anchor, radius_m: f64, pole: [f64; 3], phase_rad: f64 },
    Lagrange { body: String, point: LagrangePoint },
    /// The orbit *about* a collinear point, which is what a craft there actually flies.
    Libration {
        body: String,
        point: LagrangePoint,
        standoff_m: f64,
        radial_m: f64,
        vertical_m: f64,
        planar_rate: f64,
        vertical_rate: f64,
        amplitude_ratio: f64,
        phase_rad: f64,
        vertical_phase_rad: f64,
        epoch_s: f64,
    },
}

/// How a ship is moving, as the parameters it was set from.
///
/// The recipe and never the trajectory, for the same reason [`Order`] carries a course rather
/// than the crossing it implies: a solved path on the wire is a second copy of an answer both
/// ends can compute, free to disagree with the one the receiver would have reached. So a
/// crossing travels as the arguments its planner takes, and a conic travels as nothing at all —
/// it is re-solved from the state beside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Motive {
    Crossing {
        from_ly: [f64; 3],
        /// The velocity the crossing was planned from. A burn does not begin from rest.
        beta0: [f64; 3],
        to_ly: [f64; 3],
        start_s: f64,
        drive: Drive,
        /// Where the crossing is *for*. Arriving becomes holding this.
        arrive_at: Option<Waypoint>,
        /// The ship's own clock when the crossing began, which is what its proper time is
        /// measured from.
        clock_base_s: f64,
    },
    /// Held on a station by thrust.
    Holding(Waypoint),
    /// Ballistic. Re-solved at the far end from the position and velocity in [`Motion`].
    Falling,
    /// A straight line, read from where and when it began rather than integrated.
    Drifting { from_ly: [f64; 3], since_t: f64 },
}

/// A ship's whole state of motion.
///
/// What a reconnect is handed. Position alone was not enough and the way that showed was a
/// player signing out of an orbit and signing back into a drift: the client placed its ship at
/// the point it was given, at rest, and then predicted a future the server did not share.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    /// Light-years from the world origin.
    pub at_ly: [f64; 3],
    /// Velocity as a fraction of `c`.
    pub beta: [f64; 3],
    /// Seconds on the ship's own clock, which no resynchronising may change.
    pub clock_s: f64,
    pub drive: Drive,
    pub motive: Motive,
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

/// One craft as another sees it: where it appeared to be, and when that light left.
///
/// An **appearance**, never a state. [`Motion`] is a recipe, and a recipe for someone else's
/// ship is a recipe a client can evaluate at its own clock — which is the whole of what the
/// light-cone gate exists to prevent, handed over in a different shape. So this carries one
/// sample of a worldline and nothing that can be run forward from it: a client drawing a
/// contact between updates has to hold it still or interpolate what it was already told, and
/// either way it cannot get ahead of the light.
///
/// [`Presence`] is therefore not a small [`Motion`] and must not grow into one. `beta` is here
/// because it is *measurable* at a distance — it is what the light arrives Doppler-shifted and
/// aberrated by — and `facing` because a hull's attitude is simply its silhouette.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    pub ship_id: ShipId,
    /// What to call it on screen.
    pub name: String,
    /// How long the hull is, metres. On the wire rather than derived from a kind, so craft
    /// varying in size costs no protocol version.
    pub length_m: f64,
    /// Light-years from the world origin, at the moment the light left.
    pub at_ly: [f64; 3],
    /// Velocity then, as a fraction of `c`.
    pub beta: [f64; 3],
    /// Unit vector the nose pointed along then.
    pub facing: [f64; 3],
    /// Coordinate microseconds the light left. Always earlier than [`Presence::arrive_t`].
    pub emitted_t: i64,
    /// Coordinate microseconds it arrives. Never later than the server's `t` when it is sent.
    pub arrive_t: i64,
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

impl Cleared<Presence> {
    /// **The same gate, for what a client is told about other craft.**
    ///
    /// One test rather than two: `arrive_t <= now_t` or the client is being shown a ship where
    /// it has not yet been seen to be. There is no second test here because there is no
    /// `strength` to compare — whether a hull is large enough to make out is a question about
    /// the observer and the distance, which the caller has and this does not, so the visibility
    /// cull happens before a `Presence` is built at all. Causality is what the type enforces.
    pub fn clear(presence: Presence, now_t: i64) -> Result<Self, Withheld> {
        if presence.arrive_t > now_t {
            return Err(Withheld::StillInFlight);
        }
        Ok(Self { inner: presence })
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
        /// The ship, whole: where it is, how fast, how old, and **what it is doing**.
        ///
        /// Without it the client knows only the clock and its own identity, and would place its
        /// ship wherever it happened to start — the origin, which is empty space. Carrying only
        /// the position was the same failure one step in: a ship found mid-flight came back at
        /// rest, so signing out of an orbit signed you back into a drift, and the two ends then
        /// predicted different futures for the same craft.
        ///
        /// This is the re-acquire of `lightcone/docs/17-reconciliation.md`: a state the client
        /// cannot reach by folding anything, so it is handed one and takes it.
        ship: Motion,
    },
    /// The event channel. Cleared, by construction.
    Sightings(Vec<Cleared<Sighting>>),
    /// Who else is in sight, and where they appeared to be. Cleared, by construction.
    ///
    /// Stated every tick rather than on change, because a contact's *position* is what moved
    /// and there is no event in that. A client hears nothing at all about craft it cannot see,
    /// which is how a system with nobody in it and a system whose traffic is all out of range
    /// look the same from inside.
    Present(Vec<Cleared<Presence>>),
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
    /// `Outbound::Welcome { .., ship: Motion { at [4.2, 0, 0], holding a 12 Mm orbit of Earth } }`
    pub const WELCOME: &[u8] = &[
        0, 7, 10, 84, 128, 137, 122, 3, 65, 100, 97, 205, 204, 204, 204, 204, 204, 16, 64, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 252, 169, 241, 210,
        77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 24, 245, 64, 0, 0, 0, 0, 0, 0,
        20, 64, 43, 135, 22, 217, 206, 247, 239, 63, 1, 1, 1, 5, 69, 97, 114, 116, 104, 0, 0, 0,
        0, 96, 227, 102, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        240, 63, 0, 0, 0, 0, 0, 0, 224, 63,
    ];

    /// `Inbound::Act(Intent { ship_id: 42, order: Transmit { power_w: 1500.0 }, .. })`
    pub const ACT: &[u8] = &[
        1, 84, 0, 0, 0, 0, 0, 0, 112, 151, 64, 128, 137, 122,
    ];

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
    pub const HELLO: &[u8] = &[
        0, 10, 5, 97, 46, 98, 46, 99,
    ];

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
        4, 84, 18, 128, 137, 122, 2, 1, 5, 69, 97, 114, 116, 104, 0, 0, 0, 0, 0, 0, 0, 64, 1, 0,
        0, 0, 0, 0, 0, 8, 64,
    ];
    /// `Outbound::Present([Presence { ship 42 "Ada", 500 m, at [4.2, 0, 0], nose +y }])`
    ///
    /// Pinned because it is the one message that says where somebody *else* is. A field moving
    /// here is a client drawing a contact somewhere its light never came from.
    pub const PRESENT: &[u8] = &[
        2, 1, 84, 3, 65, 100, 97, 0, 0, 0, 0, 0, 64, 127, 64, 205, 204, 204, 204, 204, 204, 16,
        64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 252, 169,
        241, 210, 77, 98, 80, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 240, 63, 0, 0, 0, 0, 0, 0, 0, 0, 192, 132, 61, 128, 137, 122,
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
            // Holding an orbit rather than at rest at a point. A ship doing something is the
            // shape worth pinning: it reaches through `Motion` into `Motive`, `Waypoint` and
            // `Anchor` at once, and those nested enums are where a field moves unnoticed.
            ship: Motion {
                at_ly: [4.2, 0.0, 0.0],
                beta: [0.0, 0.001, 0.0],
                clock_s: 86_400.0,
                drive: Drive { accel_g: 5.0, max_beta: 0.999 },
                motive: Motive::Holding(Waypoint::Orbit {
                    about: Anchor::Body("Earth".into()),
                    radius_m: 1.2e7,
                    pole: [0.0, 0.0, 1.0],
                    phase_rad: 0.5,
                }),
            },
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

    fn present() -> Outbound {
        Outbound::Present(vec![
            Cleared::<Presence>::clear(
                Presence {
                    ship_id: ShipId(42),
                    name: "Ada".into(),
                    length_m: 500.0,
                    at_ly: [4.2, 0.0, 0.0],
                    beta: [0.0, 0.001, 0.0],
                    facing: [0.0, 1.0, 0.0],
                    emitted_t: 500_000,
                    arrive_t: 1_000_000,
                },
                1_000_000,
            )
            .expect("its light has arrived"),
        ])
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
        assert_eq!(
            encode(&present()),
            golden::PRESENT,
            "Outbound::Present changed shape at protocol version {PROTOCOL_VERSION}",
        );
    }

    #[test]
    fn everything_the_protocol_carries_survives_a_round_trip() {
        let out = [
            welcome(),
            Outbound::Sightings(vec![
                Cleared::<Sighting>::clear(sighting(500, 2.5), 1_000, 0.0).unwrap(),
            ]),
            present(),
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

    /// The gate, for the channel that says where other people are.
    ///
    /// The same rule and the same reason: a contact drawn from light that has not arrived is a
    /// client seeing a ship move before it could have.
    #[test]
    fn no_contact_arrives_before_its_light_does() {
        let now = 1_000_000;
        let at = |arrive_t: i64| Presence {
            ship_id: ShipId(7),
            name: "Vela".into(),
            length_m: 500.0,
            at_ly: [1.0, 0.0, 0.0],
            beta: [0.0; 3],
            facing: [1.0, 0.0, 0.0],
            emitted_t: arrive_t - 1_000,
            arrive_t,
        };
        assert_eq!(
            Cleared::<Presence>::clear(at(now + 1), now),
            Err(Withheld::StillInFlight),
        );
        assert!(Cleared::<Presence>::clear(at(now), now).is_ok(), "exactly on the cone");
        assert!(Cleared::<Presence>::clear(at(now - 1), now).is_ok());
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
            Cleared::<Sighting>::clear(sighting(now + 1, 1.0), now, 0.0),
            Err(Withheld::StillInFlight),
            "a sighting one microsecond early was cleared",
        );
        // The boundary is inclusive: light arriving exactly now has arrived.
        assert!(Cleared::<Sighting>::clear(sighting(now, 1.0), now, 0.0).is_ok());
        assert!(Cleared::<Sighting>::clear(sighting(now - 1, 1.0), now, 0.0).is_ok());
    }

    /// Arrival is not detection, and the two refusals are distinguishable — a client that is
    /// told nothing must not be able to tell which of the two happened.
    #[test]
    fn arrival_is_not_detection() {
        let now = 1_000_000;
        assert_eq!(
            Cleared::<Sighting>::clear(sighting(now, 0.5), now, 1.0),
            Err(Withheld::BelowNoiseFloor),
        );
        assert!(Cleared::<Sighting>::clear(sighting(now, 1.0), now, 1.0).is_ok(), "exactly at the floor");
        // In flight *and* faint is reported as in flight: the causality test comes first and
        // is the one that may never be relaxed.
        assert_eq!(
            Cleared::<Sighting>::clear(sighting(now + 1, 0.0), now, 1.0),
            Err(Withheld::StillInFlight),
        );
    }
}
