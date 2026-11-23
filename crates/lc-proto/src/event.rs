//! What an event carries, by kind.
//!
//! Stored and sent as JSON beside its [`kind`](crate::kind) number, because the store's column is
//! `jsonb` and [`Sighting::payload`](crate::Sighting::payload) is text. Each variant's JSON is the
//! shape the journal already holds, so rows written before this type existed still decode.

use serde::{Deserialize, Serialize};

use crate::{DriveChange, Released, Reported, ShadeChange, Spectrum, Spoken, kind};

/// What an emitting event carries to every craft it reaches.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Emitted {
    /// The event that lit it, which names the beam to a receiver.
    pub beam: i64,
    /// Where it left, light-microseconds. The event's own position is rounded by the store.
    pub from: [f64; 3],
    pub axis: [f64; 3],
    pub half_angle_rad: f64,
    pub spectrum: Spectrum,
    /// Sustained watts from this event on. Zero is going out.
    pub power_w: f64,
    /// Joules let go of at once.
    #[serde(default)]
    pub burst_j: f64,
}

/// What a [`kind::BURN`] says was ordered. At most one of `beta`, `cross` and `intercept`; a
/// burn with none was a course, a park or the director's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Burned {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beta: Option<[f64; 3]>,
    /// The star crossed to, by catalog id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intercept: Option<i64>,
    /// The acceleration flown, after the clamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accel_g: Option<f64>,
}

/// What a [`kind::CUT`] says was ordered: `Some` for starting (`true`) or abandoning a refit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Cut {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refit: Option<bool>,
}

/// One payload per [`kind`]. [`Payload::kind`] is the only place the two are paired.
#[derive(Clone, Debug, PartialEq)]
pub enum Payload {
    Transmit { power_w: f64 },
    Burn(Burned),
    Cut(Cut),
    Drive(DriveChange),
    Message(Spoken),
    /// A public key or a faction key on the air: a [`Spoken`] with a key body.
    Key(Spoken),
    Report(Reported),
    Vanish,
    Appear,
    /// The emission is the spike; rows from before it was carried have none.
    Collapse { released: Released, emission: Option<Emitted> },
    Shade(ShadeChange),
    Emit(Emitted),
}

/// Why a stored or received payload is not one this build understands.
#[derive(Clone, Debug, PartialEq)]
pub enum PayloadError {
    UnknownKind(i16),
    Malformed { kind: i16, why: String },
}

impl std::fmt::Display for PayloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PayloadError::UnknownKind(kind) => write!(f, "no event kind {kind}"),
            PayloadError::Malformed { kind, why } => write!(f, "kind {kind}: {why}"),
        }
    }
}

impl std::error::Error for PayloadError {}

#[derive(Serialize, Deserialize)]
struct Power {
    power_w: f64,
}

#[derive(Serialize, Deserialize)]
struct Carrying {
    emission: Emitted,
}

#[derive(Serialize, Deserialize)]
struct Collapsing {
    released_j: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    emission: Option<Emitted>,
}

#[derive(Serialize, Deserialize)]
struct Nothing {}

impl Payload {
    pub fn kind(&self) -> i16 {
        match self {
            Payload::Transmit { .. } => kind::TRANSMIT,
            Payload::Burn(_) => kind::BURN,
            Payload::Cut(_) => kind::CUT,
            Payload::Drive(_) => kind::DRIVE,
            Payload::Message(_) => kind::MESSAGE,
            Payload::Key(_) => kind::KEY,
            Payload::Report(_) => kind::REPORT,
            Payload::Vanish => kind::VANISH,
            Payload::Appear => kind::APPEAR,
            Payload::Collapse { .. } => kind::COLLAPSE,
            Payload::Shade(_) => kind::SHADE,
            Payload::Emit(_) => kind::EMIT,
        }
    }

    /// As the journal stores it and a [`Sighting`](crate::Sighting) carries it.
    pub fn to_json(&self) -> String {
        let json = match self {
            Payload::Transmit { power_w } => serde_json::to_string(&Power { power_w: *power_w }),
            Payload::Burn(burned) => serde_json::to_string(burned),
            Payload::Cut(cut) => serde_json::to_string(cut),
            Payload::Drive(change) => serde_json::to_string(change),
            Payload::Message(spoken) | Payload::Key(spoken) => serde_json::to_string(spoken),
            Payload::Report(reported) => serde_json::to_string(reported),
            Payload::Vanish | Payload::Appear => serde_json::to_string(&Nothing {}),
            Payload::Collapse { released, emission } => {
                serde_json::to_string(&Collapsing { released_j: released.released_j, emission: *emission })
            }
            Payload::Shade(change) => serde_json::to_string(change),
            Payload::Emit(emission) => serde_json::to_string(&Carrying { emission: *emission }),
        };
        // Infallible: no map with non-string keys, and serde_json writes a non-finite float as null.
        json.expect("a payload is always JSON")
    }

    pub fn from_json(kind: i16, json: &str) -> Result<Self, PayloadError> {
        fn read<'a, T: Deserialize<'a>>(kind: i16, json: &'a str) -> Result<T, PayloadError> {
            serde_json::from_str(json).map_err(|why| PayloadError::Malformed { kind, why: why.to_string() })
        }
        Ok(match kind {
            kind::TRANSMIT => Payload::Transmit { power_w: read::<Power>(kind, json)?.power_w },
            kind::BURN => Payload::Burn(read(kind, json)?),
            kind::CUT => Payload::Cut(read(kind, json)?),
            kind::DRIVE => Payload::Drive(read(kind, json)?),
            kind::MESSAGE => Payload::Message(read(kind, json)?),
            kind::KEY => Payload::Key(read(kind, json)?),
            kind::REPORT => Payload::Report(read(kind, json)?),
            kind::VANISH => {
                read::<Nothing>(kind, json)?;
                Payload::Vanish
            }
            kind::APPEAR => {
                read::<Nothing>(kind, json)?;
                Payload::Appear
            }
            kind::COLLAPSE => {
                let Collapsing { released_j, emission } = read(kind, json)?;
                Payload::Collapse { released: Released { released_j }, emission }
            }
            kind::SHADE => Payload::Shade(read(kind, json)?),
            kind::EMIT => Payload::Emit(read::<Carrying>(kind, json)?.emission),
            other => return Err(PayloadError::UnknownKind(other)),
        })
    }

    /// The light it puts out that lands as energy, for the kinds that carry one.
    pub fn emission(&self) -> Option<&Emitted> {
        match self {
            Payload::Emit(emission) => Some(emission),
            Payload::Collapse { emission, .. } => emission.as_ref(),
            _ => None,
        }
    }

    /// Whom a transmission is to, whether it is sealed, and the faction key it is under: what
    /// decides who may read it. `None` for anything that is not a transmission.
    pub fn addressing(&self) -> Option<Addressing> {
        let (to, sealed, under) = match self {
            Payload::Message(s) | Payload::Key(s) => (s.to, s.sealed, s.under),
            Payload::Report(r) => (r.to, r.sealed, r.under),
            _ => return None,
        };
        Some(Addressing { to, sealed, under })
    }

    /// What its sender calls itself, readable by anyone who hears it.
    pub fn claim(&self) -> Option<&crate::Claim> {
        match self {
            Payload::Message(s) | Payload::Key(s) => s.claim.as_ref(),
            Payload::Report(r) => r.claim.as_ref(),
            _ => None,
        }
    }

    /// Where the sender says it is, as anyone may read it.
    pub fn stated(&self) -> Option<&crate::Stated> {
        match self {
            Payload::Message(s) | Payload::Key(s) => s.stated.as_ref(),
            Payload::Report(r) => r.stated.as_ref(),
            _ => None,
        }
    }

    /// As a receiver who may not read it gets it: the body and the stated place removed, and who
    /// sent it to whom left. Anything but a transmission is returned unchanged.
    pub fn unreadable(mut self) -> Self {
        match &mut self {
            Payload::Message(s) | Payload::Key(s) => {
                s.body = crate::Body::Unreadable;
                s.stated = None;
            }
            Payload::Report(r) => {
                r.body = None;
                r.stated = None;
            }
            _ => {}
        }
        self
    }
}

/// See [`Payload::addressing`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Addressing {
    pub to: Option<i64>,
    pub sealed: bool,
    pub under: Option<crate::FactionId>,
}

impl Addressing {
    /// Sealed to somebody else, or under a key `observer` does not hold, is unreadable to it.
    pub fn readable_by(&self, observer: i64, holds: impl FnOnce(crate::FactionId) -> bool) -> bool {
        !(self.sealed && self.to != Some(observer)) && self.under.is_none_or(holds)
    }
}

/// A transmission as `observer` may have it: **the sealing, and the whole of it.** One event is
/// stored with its body intact and each receiver is handed a copy with the body removed unless they
/// are the addressee, so one transmission stays one event. The shard and a catch-up apply this one
/// rule. Anything but a transmission passes through untouched.
///
/// `holds` answers whether the observer holds a faction key as of this landing.
pub fn redact(payload: &Payload, observer: crate::ShipId, holds: impl FnOnce(crate::FactionId) -> bool) -> Payload {
    match payload.addressing() {
        Some(addressing) if !addressing.readable_by(observer.0, holds) => payload.clone().unreadable(),
        _ => payload.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows already in the journal, as the shard wrote them before payloads were typed. Each must
    /// decode as its kind, and re-encode to JSON that decodes the same.
    #[test]
    fn rows_written_before_the_type_still_decode() {
        let emission = r#"{"beam":7,"from":[1.0,2.0,3.0],"axis":[0.0,0.0,1.0],"half_angle_rad":0.1,"spectrum":{"Line":{"wavelength_m":1e-6}},"power_w":5.0,"burst_j":0.0}"#;
        let rows: Vec<(i16, String)> = vec![
            (kind::TRANSMIT, r#"{"power_w":1000000}"#.into()),
            (kind::BURN, r#"{"beta":[0.0,0.001,0.0]}"#.into()),
            (kind::BURN, r#"{"accel_g":0.5}"#.into()),
            (kind::BURN, r#"{"cross":42,"accel_g":1}"#.into()),
            (kind::BURN, r#"{"intercept":3}"#.into()),
            (kind::BURN, "{}".into()),
            (kind::CUT, "{}".into()),
            (kind::CUT, r#"{"refit":true}"#.into()),
            (kind::DRIVE, r#"{"power_w":1e12,"facing":[1.0,0.0,0.0]}"#.into()),
            (kind::MESSAGE, r#"{"to":null,"sealed":false,"body":{"Text":"hello"}}"#.into()),
            (kind::KEY, r#"{"to":2,"sealed":false,"body":"Key"}"#.into()),
            (kind::REPORT, r#"{"to":null,"sealed":true,"body":null}"#.into()),
            (kind::VANISH, "{}".into()),
            (kind::APPEAR, "{}".into()),
            (kind::COLLAPSE, format!(r#"{{"emission":{emission},"released_j":3.6e24}}"#)),
            (kind::COLLAPSE, r#"{"released_j":3.6e24}"#.into()),
            (kind::SHADE, r#"{"shade":"Clear"}"#.into()),
            (kind::EMIT, format!(r#"{{"emission":{emission}}}"#)),
        ];
        for (kind, json) in rows {
            let payload = Payload::from_json(kind, &json).unwrap_or_else(|why| panic!("{json}: {why}"));
            assert_eq!(payload.kind(), kind, "{json}");
            assert_eq!(Payload::from_json(kind, &payload.to_json()), Ok(payload), "{json}");
        }
    }

    #[test]
    fn a_payload_of_the_wrong_kind_is_refused() {
        assert_eq!(Payload::from_json(99, "{}"), Err(PayloadError::UnknownKind(99)));
        assert!(matches!(Payload::from_json(kind::TRANSMIT, "{}"), Err(PayloadError::Malformed { .. })));
        assert!(matches!(Payload::from_json(kind::EMIT, "{}"), Err(PayloadError::Malformed { .. })));
        assert!(matches!(Payload::from_json(kind::DRIVE, "not json"), Err(PayloadError::Malformed { .. })));
    }
}
