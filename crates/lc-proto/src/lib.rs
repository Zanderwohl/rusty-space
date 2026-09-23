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
pub const PROTOCOL_VERSION: u32 = 36;

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

/// What a craft can do under power. Mirrors `lc_world::flight::Drive`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drive {
    /// Proper acceleration, in g.
    pub accel_g: f64,
    /// Speed cap as a fraction of `c`.
    pub max_beta: f64,
    /// How fast it throws its reaction mass, meters a second.
    ///
    /// On the wire because a client draws its own ship's plume, and what a burn looks like is
    /// `½ F v` — the one number a trajectory does not depend on and an exhaust does.
    pub exhaust_v_m_s: f64,
    /// How fast the hull can swing its nose, radians a second.
    ///
    /// On the wire because a crossing's coast is held open long enough for the flip, so two ends
    /// that disagree about how fast a ship turns would re-plan the same recipe into two
    /// different trajectories.
    pub slew_rate_rad_s: f64,
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
        /// The velocity the crossing ends *on*, which is the station's where there is one to
        /// join. A burn does not have to end at rest either.
        arrive_beta: [f64; 3],
        start_s: f64,
        drive: Drive,
        /// Where the crossing is *for*. Arriving becomes holding this.
        arrive_at: Option<Waypoint>,
        /// The ship's own clock when the crossing began, which is what its proper time is
        /// measured from.
        clock_base_s: f64,
    },
    /// A crossing flown in a *body's* frame: one orbit of it to another.
    ///
    /// Every coordinate here is relative to that body, so the name is not a label — without it
    /// the numbers mean nothing. Both ends place the body from their own copy of the system,
    /// which is why the frame does not have to be sent.
    Transfer {
        about: String,
        from_ly: [f64; 3],
        beta0: [f64; 3],
        to_ly: [f64; 3],
        arrive_beta: [f64; 3],
        start_s: f64,
        drive: Drive,
        arrive_at: Option<Waypoint>,
        clock_base_s: f64,
    },
    /// Closing on another craft and matching its velocity.
    ///
    /// The arguments of the approach, like [`Motive::Crossing`], and re-solved at the far end
    /// by the same planner. Every number is either *relative* to the quarry or a **sighting**
    /// of it — where it was seen, how fast, and when the light left — so a receiver learns
    /// nothing here it could not have seen for itself. That is what makes it safe to hand a
    /// client whose own ship is the pursuer.
    Rendezvous {
        /// Relative to the quarry's reckoned position, at `start_s`.
        from_ly: [f64; 3],
        /// Relative to the quarry's velocity, at `start_s`.
        beta0: [f64; 3],
        /// Where the approach ends, relative: a standoff short of the quarry.
        to_ly: [f64; 3],
        start_s: f64,
        drive: Drive,
        /// The sighting the frame is anchored at.
        frame_from_ly: [f64; 3],
        frame_beta: [f64; 3],
        since_t: f64,
        /// Who is being closed on.
        target: ShipId,
        clock_base_s: f64,
    },
    /// Held on a station by thrust.
    Holding(Waypoint),
    /// Ballistic. Re-solved at the far end from the position and velocity in [`Motion`].
    Falling,
    /// A straight line, read from where and when it began rather than integrated.
    Drifting { from_ly: [f64; 3], since_t: f64 },
    /// Closing on, or holding station beside, a craft that is itself under thrust.
    ///
    /// A [`Motive::Rendezvous`] with one more number: the acceleration the pursuer measured
    /// from two sightings. The approach is solved in the frame that accelerates with the quarry,
    /// so `from_ly`, `beta0` and `to_ly` are relative to it and `start_s` is the quarry's own
    /// proper time since the sighting. Appended last, so no earlier motive's bytes move.
    Escort {
        from_ly: [f64; 3],
        beta0: [f64; 3],
        to_ly: [f64; 3],
        start_s: f64,
        /// The drive the approach has — the pursuer's, less the quarry's acceleration.
        drive: Drive,
        frame_from_ly: [f64; 3],
        frame_beta: [f64; 3],
        /// The quarry's proper acceleration, light-seconds per second squared.
        accel: [f64; 3],
        since_t: f64,
        target: ShipId,
        clock_base_s: f64,
    },
    /// Closing on, or holding station beside, a craft reckoned along its conic about a body.
    ///
    /// The [`Motive::Rendezvous`] numbers, read differently: the frame is the conic through the
    /// sighting, which the receiver re-solves against its own copy of the system as it does a
    /// [`Motive::Falling`], and the offsets are Galilean in it. Appended last.
    Consort {
        from_ly: [f64; 3],
        beta0: [f64; 3],
        to_ly: [f64; 3],
        start_s: f64,
        drive: Drive,
        frame_from_ly: [f64; 3],
        frame_beta: [f64; 3],
        since_t: f64,
        target: ShipId,
        clock_base_s: f64,
    },
}

/// How close a craft hangs about once it has matched with its quarry. Mirrors
/// `lc_world::pursuit::Closeness`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Closeness {
    /// Formation flying, a few combined hull lengths off.
    #[default]
    Company,
    /// A kilometer of clear space between the hulls.
    Intimate,
}

/// A standing intercept: who, and how close.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pursuit {
    pub quarry: ShipId,
    pub closeness: Closeness,
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
    /// Which way the nose points. Carried because a turn takes time and is as often as not
    /// half finished — nothing in a trajectory says where a nose had got to.
    pub attitude: [f64; 3],
    /// Seconds on the ship's own clock, which no resynchronizing may change.
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
    /// can do, so a client cannot ask for a better ship than it has. The speed cap likewise, to
    /// what its stored energy can pay for.
    SetCourse { course: Course, accel_g: f64, max_beta: f64 },
    /// Cross to another star, at this acceleration.
    ///
    /// The star is named by **catalog id**, not by position. A position would let a client
    /// fly to somewhere it invented; an id can only name a star the server also holds, which
    /// it does because both ends are given the same packed catalog — see the shard's
    /// `--sky` in `lightcone/docs/15-runbook.md`. The server resolves the id and folds a
    /// coordinate, so the two never plan against different places.
    ///
    /// Separate from [`Order::SetCourse`] because a `Course` names somewhere inside the local
    /// system and this is the one thing a ship does that is not about one.
    Cross { star: u64, accel_g: f64, max_beta: f64 },
    /// Cut the engine. Not a stop — whatever velocity it had, it keeps, on whatever conic that
    /// puts it on.
    CutDrive,
    /// Close on another craft, match its velocity, and hold station alongside it.
    ///
    /// A **standing** order, unlike every other one here, and that is the interesting thing
    /// about it. The rest are events: they happen at an instant and a trajectory follows. This
    /// one is a policy — the authority re-solves it whenever what the pursuer can see of its
    /// quarry stops agreeing with the plan it is flying — and each of those re-solutions is an
    /// ordinary event both ends fold the usual way. The standing part lives only on the
    /// authority, so nothing about how a trajectory is agreed on has changed.
    ///
    /// The quarry is named by its identifier, which a client can only have because it was told
    /// about it — see [`Presence`]. There is no way to spell an intercept of a craft whose
    /// light has not arrived.
    ///
    /// Sent again for the same quarry with another closeness, it closes in or stands off.
    Intercept { ship_id: ShipId, closeness: Closeness },
    /// Give up a standing [`Order::Intercept`], with no further corrections: the drive is cut
    /// and the ship keeps whatever velocity the approach or the station left it with, on whatever
    /// conic that is.
    BreakOff,
    /// Rebuild toward this loadout. Refused while under way.
    Refit { target: Loadout },
    /// Stop a refit where it is; the step in progress is reversed.
    CancelRefit,
    /// Put a message on the air, for `to`, pointed `aim`, readable by `secrecy`.
    ///
    /// **Three independent choices, and keeping them independent is the whole design.** Who it
    /// is addressed to says where the reply goes and whose acknowledgments ride with it.
    /// Where it is pointed says who *hears* it. Whether it is sealed says who can *read* it.
    /// A player who conflates them broadcasts a private message in clear across a system, which
    /// is a mistake the interface should let them make.
    ///
    /// `to` is **`None` for a broadcast**: something said to nobody in particular, which is
    /// what the public channel sends. Everything else is addressed to one craft even when it is
    /// shouted omnidirectionally — a chat is with somebody, and the bystanders who hear an open
    /// one are eavesdroppers who can see that they are.
    ///
    /// A broadcast cannot be sealed. There is nobody for it to be sealed *to*, and the server
    /// refuses the combination rather than quietly sending it in the open. Appended last.
    Say { to: Option<ShipId>, aim: Aim, secrecy: Secrecy, body: String, idem: MessageKey },
    /// Put your public key on the air, so `to` can seal messages to you.
    ///
    /// A message like any other, and that is the mechanic rather than an implementation note:
    /// it travels at `c`, so a key sent across four light-years is usable four years later, and
    /// an omnidirectional offer hands it to everyone in range at the same time. Nobody starts
    /// holding anybody's key — first contact is loud by necessity. Appended last.
    /// `to` is `None` to offer it to **whoever hears it**, which is what the public channel
    /// does: anyone in range can answer in private from then on.
    OfferKey { to: Option<ShipId>, aim: Aim },
    /// Send what this craft has learned since it last reported to `to`.
    ///
    /// A transmission like any other — aimed, sealed or not, and subject to the same light
    /// delay — but not a conversation. It is not filed in a transcript, it is never
    /// acknowledged automatically, and what it does at the far end is fold into the receiver's
    /// knowledge.
    ///
    /// **The shard writes the report**, from the knowledge it holds for this craft. A client
    /// that wrote its own could report anything it liked. Appended last.
    SendReport { to: Option<ShipId>, aim: Aim, secrecy: Secrecy, idem: MessageKey },
    /// Put the telescope on a duty. Appended last.
    SetDuty { duty: Duty, integration_s: f64 },
    /// Call something by a name of this craft's own. Appended last.
    NameIt { subject: Subject, name: String },
    /// Keep a subject's raw logs whatever the shard concludes from them, or stop keeping them.
    /// Appended last.
    RetainRaw { subject: Subject, keep: bool },
    /// Answer `with` automatically from now on, or stop.
    ///
    /// A standing order kept by the server, so it answers whether or not anyone is flying the
    /// ship. It puts nothing on the air and is answered by [`Outbound::AutoAcking`], not
    /// `Accepted`. Appended last.
    AutoAck { with: ShipId, on: bool },
    /// Read every log this craft holds into a conclusion and consume it, to make room. Kept
    /// raw logs are left alone. Appended last.
    Analyze,
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

/// Event kinds, as [`Sighting::kind`] carries them. Named here because both ends read them.
pub mod kind {
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
    /// A craft was taken from here by fiat, stamped where it was. See [`super::Inbound::Command`].
    pub const VANISH: i16 = 8;
    /// The same craft put down, stamped where it landed and at the same coordinate time. The two
    /// are seen apart, each at its own light delay: nothing flew between them.
    pub const APPEAR: i16 = 9;
}

/// What a craft's drive became at a [`kind::DRIVE`] event.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriveChange {
    /// Watts into the exhaust from this instant. Zero is the drive going out.
    pub power_w: f64,
    /// Unit vector the nose pointed along.
    pub facing: [f64; 3],
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
/// sample of a worldline and not the motive behind it. A client reckons that sample forward
/// ballistically to the light arriving now (`lc_world::sighted`), which is wrong about any
/// maneuvere since until the next statement — the light of it has not been delivered.
///
/// [`Presence`] is therefore not a small [`Motion`] and must not grow into one. `beta` is here
/// because it is *measurable* at a distance — it is what the light arrives Doppler-shifted and
/// aberrated by — and `facing` because a hull's attitude is simply its silhouette.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Presence {
    pub ship_id: ShipId,
    /// What to call it on screen.
    pub name: String,
    /// How long the hull is, meters. On the wire rather than derived from a kind, so craft
    /// varying in size costs no protocol version.
    pub length_m: f64,
    /// Light-years from the world origin, at the moment the light left.
    pub at_ly: [f64; 3],
    /// Velocity then, as a fraction of `c`.
    pub beta: [f64; 3],
    /// Unit vector the nose pointed along then.
    pub facing: [f64; 3],
    /// What its drive was putting into its exhaust then, watts. Zero when it was coasting.
    ///
    /// Sent rather than derived, and it is worth saying why this is not the "second copy of an
    /// answer" the rest of this file refuses. A receiver *cannot* work it out: the power of a
    /// burn is the craft's mass times its acceleration times its exhaust speed, and a client
    /// knows none of the three about somebody else's ship. What it is, is the one thing about a
    /// burn that is plainly observable — a plume's brightness is exactly this — so a receiver
    /// is being told what it can see rather than what it could have computed.
    pub jet_power_w: f64,
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
/// Deserializing one is not a hole in that, and it is worth being exact about why. A `Cleared`
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
    /// is no threshold, subscription or optimization that may be allowed to reverse it. The
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

/// A book on the shelf.
///
/// A mirror of the catalog rather than the catalog's own type, for the reason the rest of
/// this crate is a mirror: `lc-books` is a zip and an XML parser, and a protocol that borrowed
/// its types would put both in every client's wire layer and make every change to how a book is
/// parsed a change to the protocol. The two agree by being converted at the edge.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Book {
    pub id: String,
    pub title: String,
    pub authors: Vec<Writer>,
    pub year: Option<i32>,
    pub subjects: Vec<String>,
    /// The file under the shelf's base, which the client composes a URL from and never receives
    /// one for. A server that could send a URL could send any URL.
    pub file: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Writer {
    pub name: String,
    pub sort: Option<String>,
}

/// Where a player is in a book.
///
/// **A character offset, not a page.** A page is a fact about a window at a size; this survives
/// a font change, a resize and a different client. See `lightcone/docs/19-library.md`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    pub book: String,
    pub spine: u32,
    pub char_offset: u32,
    /// How far through, in the book's own thousand-character locations. **Reported by the
    /// client and not checked**, because the server does not have the book and there is nothing
    /// to win by lying: the shelf reads "34%" and no rule anywhere depends on it.
    pub location: u32,
    pub locations: u32,
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
        /// How fast this world runs, as a multiple of the design rate — one Julian year an
        /// hour, the 8766 a client's own clock already counts in.
        ///
        /// Stated rather than assumed. The client used to hold a constant of its own that
        /// happened to agree, which is a different thing from being told: a shard running at
        /// any other rate would have been joined by a client confidently running at this one.
        /// Carrying it is also what lets a development shard stage a scene at sixty times the
        /// design rate, where a three-month chase is fifteen seconds of watching.
        rate: f64,
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
    /// as a maneuvere that has already finished, so the ship appears to teleport to its
    /// destination, and the server goes on refusing orders about a system it does not think the
    /// ship has reached.
    ///
    /// The rate is restated with it because a welcome happens once and a rate does not have
    /// to: a shard that stages a scene changes how fast the world runs, and a client still
    /// ticking at the old one would run away from it exactly as an unstated rate did.
    Clock { now_t: i64, rate: f64 },
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
    /// **Your ship is now doing this.** What the authority did to a craft that its owner did
    /// not order.
    ///
    /// Every other change to a ship is an order the client sent and can fold for itself. A
    /// standing [`Order::Intercept`] is the exception and the reason this exists: the authority
    /// re-solves it against sightings the client cannot reproduce, whenever it likes, and
    /// without this the two ends would quietly fly different ships — the server closing on a
    /// quarry while the client's copy drifted where it was left.
    ///
    /// The same [`Motion`] a welcome carries, and applied the same way, because a re-acquire
    /// and "here is what you are doing now" are one question asked by two things. See
    /// `lightcone/docs/17-reconciliation.md`.
    ///
    /// It says nothing a client is not entitled to: a `Motive::Rendezvous` is relative
    /// offsets and one sighting, which is what its own eyes gave it.
    Flying { ship_id: ShipId, ship: Motion },
    /// This client is sending faster than the server will take, and the message was dropped
    /// unread. Not a disconnection: a client that hits this has a bug, and is told so it can be
    /// fixed. Nothing about the world leaks through it — it is a fact about the client's own
    /// sending and reveals nothing that was withheld.
    Throttled { retry_after_ticks: u32 },
    /// The standing intercept a ship has, stated on sign-in.
    ///
    /// Every other change to it is an order the client sent and saw accepted. A pursuit
    /// outlives the connection that ordered it — a ship goes on hanging about with its quarry
    /// while its pilot is away — so a client coming back has to be told there is one, or it
    /// has no way to break it off. Appended last.
    Pursuing { ship_id: ShipId, pursuit: Pursuit },
    /// The ship's modules and energy, as settled by the authority. Sent on sign-in and whenever
    /// the account changes other than by the passage of time. Appended last.
    Fitted { ship_id: ShipId, fitting: Fitting },
    /// Everything this ship has ever said or been told, and whose keys it holds.
    ///
    /// Sent once, shortly after a welcome. A conversation outlives the connection it happened
    /// on — the whole premise is that an answer can take years — so a client that came back to
    /// an empty transcript would be a client that had lost the game's slowest and most valuable
    /// state. The events are still in the journal either way; this is the ship's own copy of
    /// the ones it is party to, which is a different question from what the journal holds.
    ///
    /// It says nothing this ship is not entitled to: every message here either left it or
    /// landed on it, and a sealed one it is not the addressee of has no body, exactly as it had
    /// none when it arrived. Appended last.
    Backlog { messages: Vec<Said>, keys: Vec<ShipId> },
    /// What there is to read, and where the shelf is.
    ///
    /// Appended last, like every variant added since: the discriminants above are what the
    /// goldens are pinned at, and a version bump is not a license to renumber them.
    ///
    /// **Not cleared, and deliberately.** Everything else the server says about the world goes
    /// through [`Cleared`], because delivering an event before its light arrives would delete
    /// the game. A book is not an event, nobody observes one across a light-hour, and the shelf
    /// is the same for every player at every distance. Said here so the next reader takes it for
    /// a decision rather than an omission.
    Library {
        /// What `file` hangs off: a CDN prefix, stated by the shard so the shelf can move
        /// without a client release.
        base: String,
        books: Vec<Book>,
    },
    /// Where this account left off in each book it has opened.
    ///
    /// **Most recently read first.** That ordering is the only record of recency on the wire,
    /// which is what lets the shelf offer "recently read" without either end having to agree
    /// about whose clock a timestamp would be in.
    Reading(Vec<Bookmark>),
    /// What this craft has learned since the last of these: a serialized
    /// `lc_world::knowledge::Report` from the craft itself, which the client folds into its
    /// copy without adding a hop. The whole of a craft's knowledge arrives this way, in pages,
    /// when it signs in. Appended last.
    Learned { report: String },
    /// What the telescope is committed to, as the shard has it. Said on sign-in and whenever it
    /// changes. Appended last.
    Observing { duty: Duty, integration_s: f64 },
    /// This craft's own photometry since the last of these: a serialized
    /// `lc_world::knowledge::Logs`, with the subjects it keeps raw. A report never carries logs, so a client's copy of its
    /// own curves arrives this way, in pages. Appended last.
    Logged { logs: String },
    /// Every craft this ship answers automatically, whole. Sent on signing in and after each
    /// [`Order::AutoAck`]. Appended last.
    AutoAcking { ship_id: ShipId, with: Vec<ShipId> },
    /// What became of [`Inbound::Command`] number `seq`, to the connection that sent it and no
    /// other. `text` is for a person to read and nothing parses it. Appended last.
    Answered { seq: u32, ok: bool, text: String },
}

/// The longest command line a shard will read, in bytes.
pub const COMMAND_LIMIT: usize = 1024;

/// The largest frame and message either end of a connection accepts, bytes. Stated rather than
/// left to the library's default, so that the shard's pages can be bounded against the same
/// number the client enforces.
pub const FRAME_LIMIT: usize = 16 << 20;

/// Why an intent was not acted on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// No such ship, or it is not this client's.
    NotYours,
    /// There is no such craft in sight. Deliberately the same answer for a ship that does not
    /// exist, one in another system, and one whose light has not arrived — a client that could
    /// tell those apart could probe for craft it has not been told about.
    NotInSight,
    /// The quarry is moving too fast for an approach to be solved the way this one is. See
    /// `lc_world::pursuit`.
    TooFast,
    /// The ticket did not verify: wrong audience, expired, already spent, or not signed by a
    /// key this server publishes trust in. **Deliberately one variant** — a client learning
    /// *which* is a client learning how close it got.
    NotYou,
    /// The order itself is impossible — a burn past `c`, a transmitter at negative power.
    Impossible,
    /// Not enough stored energy for even the slowest version of this.
    NoEnergy,
    /// A refit is running, and the drive cannot be lit until it is done or canceled.
    Refitting,
    /// The ship is under way, and cannot refit until it has stopped.
    UnderWay,
    /// The refit cannot reach its target from here.
    Short(Shortfall),
    /// This ship does not hold the addressee's key, so it cannot seal anything to them.
    ///
    /// Safe to say plainly, unlike most of these: it is a fact about the sender's own keyring,
    /// which the sender already has. Appended last.
    NoKey,
    /// A report was asked for and this craft has learned nothing since it last reported to that
    /// recipient. Appended last.
    NothingNew,
}

/// Why a refit cannot be done. Mirrors `lc_world::refit::Shortage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Shortfall {
    Unbuildable,
    Energy,
    NoDrones,
    CannotBuild(Module),
    CannotDismantle(Module),
}

/// Mirrors `lc_world::fitting::Module`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Module {
    Storage,
    Drone,
    Living,
    Engine,
    Data,
}

/// Module counts and hull slots. Mirrors `lc_world::fitting::Loadout`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Loadout {
    pub storage: u32,
    pub drones: u32,
    pub living: u32,
    pub engines: u32,
    pub slots: u32,
    pub data: u32,
}

/// The shard's tunables. Mirrors `lc_world::fitting::Balance`; stated so a client's refit
/// preview uses the numbers the authority does.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Balance {
    pub drive_efficiency: f64,
    pub recovery: f64,
    pub storage_per_module: f64,
    pub engine_thrust_n: f64,
    pub drone_power_w: f64,
    pub living_drain_w: f64,
    pub hull_density_kg_m3: f64,
    pub slot_volume_m3: f64,
    pub module_density_kg_m3: f64,
    pub solar_efficiency: f64,
    pub solar_gain: f64,
    pub data_per_module: f64,
    pub data_mass_fraction: f64,
    pub data_work_factor: f64,
}

/// A refit as the arguments it is planned from. Mirrors `lc_world::refit::Order`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RefitOrder {
    pub from: Loadout,
    pub target: Loadout,
    pub stored_j: f64,
    pub start_s: f64,
}

/// A ship's energy account, settled at `since_s`. Mirrors `lc_world::fitting::Account`, with
/// the balance it is read under.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fitting {
    pub balance: Balance,
    pub loadout: Loadout,
    pub stored_j: f64,
    pub since_s: f64,
    pub rapidity_since: f64,
    pub committed_j: f64,
    /// Starlight being collected in the segment that began at `since_s`, watts.
    pub solar_w: f64,
    pub refit: Option<RefitOrder>,
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
    /// Put a named scene in the world.
    ///
    /// Appended last on purpose: every other variant keeps the discriminant its golden was
    /// pinned at. Refused outright by a shard, which is not started for this — a client that
    /// could stage a scene could put a craft wherever it liked, which is the one thing no
    /// client may do. See `lc_server::director`.
    Stage { scenario: String },
    /// Put energy in this client's ship. Development only, refused by a shard for the reason
    /// `Stage` is. Appended last.
    Grant { joules: f64 },
    /// Where the player has got to. Debounced by the client: a page turn every few seconds must
    /// not be a message every few seconds.
    SetReading(Bookmark),
    /// A line typed into the console, exactly as typed.
    ///
    /// **Text, not a parsed command.** Parsing, permission and validation are the shard's, so a
    /// client that sent a structure could send one no parser would ever have produced. `seq` is
    /// the client's own count, echoed in [`Outbound::Answered`] so the answer finds its line.
    /// Appended last.
    Command { seq: u32, line: String },
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
/// In its own file because it is *data*, not definition: a hundred and sixty lines of pinned
/// byte arrays beside the types they pin would bury the types. What it is for is unchanged —
/// a format that is not self-describing cannot notice a field that moved, so this is what
/// notices.
pub mod golden;
mod knowing;
mod radio;

pub use knowing::{DWELL_MAX_S, DWELL_MIN_S, Duty, INTEGRATION_MAX_S, NAME_LIMIT, Subject, WATCH_LIMIT};

pub use radio::{
    ACK_DEPTH, Aim, Body, MESSAGE_LIMIT, MessageKey, REPORT_FORMAT, REPORT_LIMIT, Reported, Said, Secrecy, Spoken,
};

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
            rate: 1.0,
            // Holding an orbit rather than at rest at a point. A ship doing something is the
            // shape worth pinning: it reaches through `Motion` into `Motive`, `Waypoint` and
            // `Anchor` at once, and those nested enums are where a field moves unnoticed.
            ship: Motion {
                at_ly: [4.2, 0.0, 0.0],
                beta: [0.0, 0.001, 0.0],
                attitude: [1.0, 0.0, 0.0],
                clock_s: 86_400.0,
                drive: Drive {
                    accel_g: 5.0,
                    max_beta: 0.999,
                    exhaust_v_m_s: 1.5e7,
                    slew_rate_rad_s: 0.05,
                },
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
                max_beta: 0.999,
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
                max_beta: 0.25,
            },
        }
    }

    fn cross() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::Cross { star: 0x0123_4567_89ab_cdef, accel_g: 3.0, max_beta: 0.5 },
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
                    jet_power_w: 7.2e17,
                    emitted_t: 500_000,
                    arrive_t: 1_000_000,
                },
                1_000_000,
            )
            .expect("its light has arrived"),
        ])
    }

    /// A ship closing on another, which is the one motive whose numbers are all about
    /// somebody else. Pinned because a field moving in it is a pursuer flying at a point its
    /// quarry was never at.
    fn rendezvous() -> Outbound {
        Outbound::Welcome {
            client_id: ClientId(7),
            protocol: PROTOCOL_VERSION,
            ship_id: ShipId(42),
            now_t: 1_000_000,
            name: "Ada".into(),
            rate: 1.0,
            ship: Motion {
                at_ly: [4.2, 0.0, 0.0],
                beta: [0.0, 0.001, 0.0],
                attitude: [1.0, 0.0, 0.0],
                clock_s: 86_400.0,
                drive: Drive {
                    accel_g: 5.0,
                    max_beta: 0.999,
                    exhaust_v_m_s: 1.5e7,
                    slew_rate_rad_s: 0.05,
                },
                motive: Motive::Rendezvous {
                    from_ly: [1.0e-9, 0.0, 0.0],
                    beta0: [0.0, -0.001, 0.0],
                    to_ly: [1.0e-12, 0.0, 0.0],
                    start_s: 900_000.0,
                    drive: Drive {
                        accel_g: 5.0,
                        max_beta: 0.999,
                        exhaust_v_m_s: 1.5e7,
                        slew_rate_rad_s: 0.05,
                    },
                    frame_from_ly: [4.2, 1.0e-9, 0.0],
                    frame_beta: [0.0, 0.001, 0.0],
                    since_t: 899_000.0,
                    target: ShipId(7),
                    clock_base_s: 86_000.0,
                },
            },
        }
    }

    /// A ship holding station on a quarry under thrust: the rendezvous numbers and one more.
    fn escort() -> Outbound {
        let Outbound::Welcome { client_id, protocol, ship_id, now_t, name, rate, ship } =
            rendezvous()
        else {
            unreachable!("the rendezvous fixture is a welcome")
        };
        Outbound::Welcome {
            client_id,
            protocol,
            ship_id,
            now_t,
            name,
            rate,
            ship: Motion {
                motive: Motive::Escort {
                    from_ly: [-1.0e-9, 0.0, 0.0],
                    beta0: [0.0, 0.0, 0.0],
                    to_ly: [-1.0e-12, 0.0, 0.0],
                    start_s: 900_000.0,
                    drive: Drive {
                        accel_g: 5.0,
                        max_beta: 0.999,
                        exhaust_v_m_s: 1.5e7,
                        slew_rate_rad_s: 0.05,
                    },
                    frame_from_ly: [4.2, 1.0e-9, 0.0],
                    frame_beta: [0.3, 0.0, 0.0],
                    accel: [1.6e-7, 0.0, 0.0],
                    since_t: 899_000.0,
                    target: ShipId(7),
                    clock_base_s: 86_000.0,
                },
                ..ship
            },
        }
    }

    /// The rendezvous numbers again, in a frame that falls.
    fn consort() -> Outbound {
        let Outbound::Welcome { client_id, protocol, ship_id, now_t, name, rate, ship } =
            rendezvous()
        else {
            unreachable!("the rendezvous fixture is a welcome")
        };
        let Motive::Rendezvous {
            from_ly, beta0, to_ly, start_s, drive, frame_from_ly, frame_beta, since_t, target,
            clock_base_s,
        } = ship.motive.clone()
        else {
            unreachable!("the rendezvous fixture is a rendezvous")
        };
        Outbound::Welcome {
            client_id,
            protocol,
            ship_id,
            now_t,
            name,
            rate,
            ship: Motion {
                motive: Motive::Consort {
                    from_ly, beta0, to_ly, start_s, drive, frame_from_ly, frame_beta, since_t,
                    target, clock_base_s,
                },
                ..ship
            },
        }
    }

    fn fitted() -> Outbound {
        let loadout = Loadout { storage: 6, drones: 2, living: 2, engines: 5, slots: 20, data: 0 };
        Outbound::Fitted {
            ship_id: ShipId(42),
            fitting: Fitting {
                balance: Balance {
                    drive_efficiency: 1.0,
                    recovery: 0.95,
                    storage_per_module: 5.0,
                    engine_thrust_n: 7.2e10,
                    drone_power_w: 2.3e19,
                    living_drain_w: 4.4e15,
                    hull_density_kg_m3: 50.0,
                    slot_volume_m3: 392_699.0,
                    module_density_kg_m3: 395.8,
                    solar_efficiency: 0.7,
                    solar_gain: 1.18e9,
                    data_per_module: 2.1e6,
                    data_mass_fraction: 0.5,
                    data_work_factor: 3.0,
                },
                loadout,
                stored_j: 4.2e26,
                since_s: 1.0e6,
                rapidity_since: 0.125,
                committed_j: 1.0e24,
                solar_w: 2.5e17,
                refit: Some(RefitOrder {
                    from: loadout,
                    target: Loadout { engines: 7, ..loadout },
                    stored_j: 4.2e26,
                    start_s: 1.0e6,
                }),
            },
        }
    }

    fn intercept() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::Intercept {
                ship_id: ShipId(7),
                closeness: Closeness::Intimate,
            },
            issued_at_client_t: 1_000_000,
        })
    }

    /// The knowledge orders and messages, each field a different value so a swap shows.
    fn set_duty() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::SetDuty {
                duty: Duty::Sweep { center: [0.25, 0.5, 0.75], radius_rad: 1.5, dwell_s: 60.0, started_s: 7.0 },
                integration_s: 1.0e4,
            },
            issued_at_client_t: 1_000_000,
        })
    }

    fn name_it() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::NameIt { subject: Subject::Body { star: 7, body: 9 }, name: "Kettle".into() },
            issued_at_client_t: 1_000_000,
        })
    }

    fn observing() -> Outbound {
        Outbound::Observing { duty: Duty::Watch { stars: vec![3, 4], dwell_s: 90.0, started_s: 5.0 }, integration_s: 2.0e3 }
    }

    fn surveying() -> Outbound {
        Outbound::Observing { duty: Duty::Survey { star: 11, started_s: 5.0 }, integration_s: 2.0e3 }
    }

    fn learned() -> Outbound {
        Outbound::Learned { report: "{}".into() }
    }

    fn send_report() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::SendReport {
                to: Some(ShipId(7)),
                aim: Aim::Ship(ShipId(7)),
                secrecy: Secrecy::Open,
                idem: 0x0fed_cba9_8765_4321,
            },
            issued_at_client_t: 1_000_000,
        })
    }

    fn say() -> Inbound {
        Inbound::Act(Intent {
            ship_id: ShipId(42),
            order: Order::Say {
                to: Some(ShipId(7)),
                aim: Aim::Ship(ShipId(7)),
                secrecy: Secrecy::Sealed,
                body: "well?".into(),
                idem: 0x1234_5678_9abc_def0,
            },
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
        assert_eq!(
            encode(&present()),
            golden::PRESENT,
            "Outbound::Present changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&rendezvous()),
            golden::RENDEZVOUS,
            "Motive::Rendezvous changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&escort()),
            golden::ESCORT,
            "Motive::Escort changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&consort()),
            golden::CONSORT,
            "Motive::Consort changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&intercept()),
            golden::INTERCEPT,
            "Order::Intercept changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&fitted()),
            golden::FITTED,
            "Outbound::Fitted changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&say()),
            golden::SAY,
            "Order::Say changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&send_report()),
            golden::SEND_REPORT,
            "Order::SendReport changed shape at protocol version {PROTOCOL_VERSION}",
        );
        for (what, bytes, pinned) in [
            ("Order::SetDuty", encode(&set_duty()), golden::SET_DUTY),
            ("Order::NameIt", encode(&name_it()), golden::NAME_IT),
            ("Outbound::Observing", encode(&observing()), golden::OBSERVING),
            ("Outbound::Observing (survey)", encode(&surveying()), golden::SURVEYING),
            ("Outbound::Learned", encode(&learned()), golden::LEARNED),
        ] {
            assert_eq!(bytes, pinned, "{what} changed shape at protocol version {PROTOCOL_VERSION}");
        }

        // The shelf. Appended variants, so their discriminants are the only new numbers here.
        let shelf = Outbound::Library {
            base: "https://cdn.example/library/".to_owned(),
            books: vec![Book {
                id: "the-gilded-age".to_owned(),
                title: "The Gilded Age: A Tale of Today".to_owned(),
                authors: vec![Writer {
                    name: "Mark Twain".to_owned(),
                    sort: Some("Twain, Mark".to_owned()),
                }],
                year: Some(1873),
                subjects: vec!["Satire".to_owned()],
                file: "The Gilded Age A Tale of Today.epub".to_owned(),
            }],
        };
        assert_eq!(
            encode(&shelf),
            golden::LIBRARY,
            "Outbound::Library changed shape at protocol version {PROTOCOL_VERSION}",
        );
        let mark = Bookmark {
            book: "the-gilded-age".to_owned(),
            spine: 2,
            char_offset: 41_580,
            location: 41,
            locations: 878,
        };
        assert_eq!(
            encode(&Outbound::Reading(vec![mark.clone()])),
            golden::READING,
            "Outbound::Reading changed shape at protocol version {PROTOCOL_VERSION}",
        );
        assert_eq!(
            encode(&Inbound::SetReading(mark)),
            golden::SET_READING,
            "Inbound::SetReading changed shape at protocol version {PROTOCOL_VERSION}",
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
            rendezvous(),
            escort(),
            accepted(),
            Outbound::Clock { now_t: 1_000_000, rate: 1.0 },
            Outbound::Refused { ship_id: ShipId(-3), reason: Refusal::NotYours },
            Outbound::WrongProtocol { server: 9 },
            Outbound::Pursuing {
                ship_id: ShipId(42),
                pursuit: Pursuit { quarry: ShipId(7), closeness: Closeness::Intimate },
            },
            consort(),
            fitted(),
            Outbound::Refused { ship_id: ShipId(1), reason: Refusal::Short(Shortfall::Energy) },
            Outbound::Backlog {
                messages: vec![Said {
                    event_id: 9,
                    idem: 99,
                    with: Some(ShipId(7)),
                    to: Some(ShipId(42)),
                    with_name: "Ada".into(),
                    mine: false,
                    sealed: true,
                    body: Body::Text("well?".into()),
                    acks: vec![3, 5],
                    sent_t: 500_000,
                    arrive_t: Some(1_000_000),
                    strength: Some(0.25),
                }],
                keys: vec![ShipId(7)],
            },
            Outbound::Refused { ship_id: ShipId(42), reason: Refusal::NoKey },
            Outbound::Refused { ship_id: ShipId(42), reason: Refusal::NothingNew },
            Outbound::Learned { report: "{}".into() },
            Outbound::Observing { duty: Duty::Stare { star: 3 }, integration_s: 1.0e4 },
            Outbound::Observing { duty: Duty::Idle, integration_s: 0.0 },
            Outbound::AutoAcking { ship_id: ShipId(42), with: vec![ShipId(7), ShipId(9)] },
            Outbound::AutoAcking { ship_id: ShipId(42), with: Vec::new() },
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
            intercept(),
            Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: Order::BreakOff,
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: Order::Burn { beta: [0.1, -0.2, 0.3] },
                issued_at_client_t: i64::MIN,
            }),
            Inbound::ResumeFrom { arrive_t: -1 },
            Inbound::Grant { joules: 1.5e25 },
            Inbound::Act(Intent {
                ship_id: ShipId(1),
                order: Order::Refit { target: Loadout { storage: 6, drones: 2, living: 2, engines: 5, slots: 20, data: 0 } },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent { ship_id: ShipId(1), order: Order::CancelRefit, issued_at_client_t: 0 }),
            say(),
            send_report(),
            Inbound::Act(Intent {
                ship_id: ShipId(42),
                order: Order::SetDuty {
                    duty: Duty::Sweep { center: [0.0, 0.0, 1.0], radius_rad: 0.35, dwell_s: 60.0, started_s: 0.0 },
                    integration_s: 1.0e4,
                },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent {
                ship_id: ShipId(42),
                order: Order::SetDuty {
                    duty: Duty::Watch { stars: vec![1, 2, u64::MAX], dwell_s: 400.0, started_s: 9.0 },
                    integration_s: 0.0,
                },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent {
                ship_id: ShipId(42),
                order: Order::SetDuty {
                    duty: Duty::Survey { star: 11, started_s: 0.0 },
                    integration_s: 1.0e4,
                },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent {
                ship_id: ShipId(42),
                order: Order::NameIt { subject: Subject::Body { star: 7, body: 9 }, name: "Kettle".into() },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent {
                ship_id: ShipId(42),
                order: Order::SendReport {
                    to: None,
                    aim: Aim::Omni,
                    secrecy: Secrecy::Open,
                    idem: 0,
                },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent {
                ship_id: ShipId(42),
                order: Order::Say {
                    to: None,
                    aim: Aim::Star(0x0123_4567_89ab_cdef),
                    secrecy: Secrecy::Open,
                    body: String::new(),
                    idem: 7,
                },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent {
                ship_id: ShipId(42),
                order: Order::OfferKey { to: Some(ShipId(7)), aim: Aim::Omni },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent {
                ship_id: ShipId(42),
                order: Order::AutoAck { with: ShipId(7), on: true },
                issued_at_client_t: 0,
            }),
            Inbound::Act(Intent { ship_id: ShipId(42), order: Order::Analyze, issued_at_client_t: 0 }),
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
            jet_power_w: 0.0,
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

