//! The gate everything a client learns by light passes through. See the crate docs.

use serde::{Deserialize, Serialize};

use crate::{Presence, ShipId, Sighting, Spectrum};

/// A beam's power at a ship changed at `arrive_t`, told when that light arrives and not before:
/// first when it lands, again whenever the emitter's power changes, and with `power_w` zero when it
/// stops. `power_w` reaches the field before its absorptivity.
///
/// No end time: an emitter can stop early, and when it meant to stop is its own business until the
/// light of stopping arrives. `beam` is the emit's event id, so two beams on one bearing stay
/// apart. `bearing` is a unit vector toward the source, in world axes.
///
/// Field order is the wire format: it was a struct variant of [`crate::Outbound`] with these fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Illumination {
    pub ship_id: ShipId,
    pub beam: i64,
    pub bearing: [f64; 3],
    pub spectrum: Spectrum,
    pub power_w: f64,
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
/// Every channel that tells a client something because light reached it holds one, a beam
/// included:
///
/// ```compile_fail
/// # use lc_proto::{Illumination, Outbound};
/// fn send(lit: Illumination) -> Outbound {
///     Outbound::Illuminated(lit)
/// }
/// ```
///
/// Deserializing one is not a hole in that, and it is worth being exact about why. A `Cleared`
/// read off the wire is a claim by whoever sent it — it says *a server released this*, not
/// *this passed our gate*. The invariant is about what a process emits, and a server only ever
/// emits ones it built with [`Cleared::arrived`]; the client is the end that decodes them.
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

/// What a client is told because light reached it: the instant that light lands, in coordinate
/// microseconds. Implemented by every payload [`Cleared`] can carry, so each passes the same test.
pub trait Arrives {
    fn arrive_t(&self) -> i64;
}

impl Arrives for Sighting {
    fn arrive_t(&self) -> i64 {
        self.arrive_t
    }
}

impl Arrives for Presence {
    fn arrive_t(&self) -> i64 {
        self.arrive_t
    }
}

impl Arrives for Illumination {
    fn arrive_t(&self) -> i64 {
        self.arrive_t
    }
}

impl<T: Arrives> Cleared<T> {
    /// **The gate.** `arrive_t <= now_t` or the client is being told something it could not know,
    /// and no threshold, subscription or optimization may reverse it. Everything else a payload
    /// is checked against is a detection rule, applied after this.
    pub fn arrived(value: T, now_t: i64) -> Result<Self, Withheld> {
        if value.arrive_t() > now_t {
            return Err(Withheld::StillInFlight);
        }
        Ok(Self { inner: value })
    }
}

impl Cleared<Sighting> {
    /// The light cone, then the noise floor: arrival is not detection.
    pub fn clear(sighting: Sighting, now_t: i64, noise_floor: f32) -> Result<Self, Withheld> {
        let passed = Self::arrived(sighting, now_t)?;
        if passed.inner.strength < noise_floor {
            return Err(Withheld::BelowNoiseFloor);
        }
        Ok(passed)
    }
}

impl Cleared<Presence> {
    /// The light cone alone. Whether a hull is large enough to make out needs the observer and
    /// the distance, which the caller has, so that cull happens before a `Presence` is built.
    pub fn clear(presence: Presence, now_t: i64) -> Result<Self, Withheld> {
        Self::arrived(presence, now_t)
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
