//! A ship's form as the wire carries it, and what the server solves from one.
//!
//! Mirrors of `lc_world::form`, with glam's vectors as arrays and the conversions on the far side
//! of the boundary, in `lc-world`, as [`crate::Course`] has them. See
//! `lightcone/docs/29-ship-form.md`.

use serde::{Deserialize, Serialize};

/// The most presets one account may keep.
pub const MAX_PRESETS: usize = 64;

/// The longest name a preset may have, in bytes.
pub const PRESET_NAME_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PartId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SparMode {
    Saddle,
    Strap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Mind,
    Storage,
    Drone,
    Engine,
    Living,
    Data,
    Bay,
    Spar(SparMode),
}

/// Relative proportions only; each part's scale is solved from its volume.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Primitive {
    Ellipsoid { axes: [f64; 3] },
    Capsule { length: f64 },
    Slab { edges: [f64; 3], corner: f64 },
    Cylinder { length: f64 },
    Torus { major: f64 },
    Frustum { length: f64, taper: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Mount {
    Attached { anchor: [f64; 3], standoff: f64 },
    Enclosing,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub parent: PartId,
    pub mount: Mount,
    pub twist: f64,
    pub tilt: [f64; 2],
    pub blend: f64,
    pub mirror: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub id: PartId,
    pub kind: Kind,
    pub primitive: Primitive,
    pub volume_m3: f64,
    pub placement: Option<Placement>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub parts: Vec<Part>,
}

/// A form an account has kept under a name. Not a fact about any craft, so it is not cleared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub form: Form,
}

/// What the authority solved from a ship's form. The client solves the same for its preview and
/// takes these when they arrive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hull {
    pub form: Form,
    /// One per part, in `form.parts` order: the length its proportions are multiplied by, meters.
    pub scales_m: Vec<f64>,
    pub capacities: Capacities,
    pub geometry: Geometry,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Capacities {
    pub storage_j: f64,
    pub drone_w: f64,
    /// Aperture ratings. Only the aft one drives.
    pub aft_w: f64,
    pub fore_w: f64,
    pub living_w: f64,
    pub data_b: f64,
    pub dry_mass_kg: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Geometry {
    /// Projected area toward each vertex of a twice-subdivided icosahedron, in its vertex order.
    pub shadow_m2: Vec<f64>,
    /// Ship frame. The roll turns the ship about it to present the broadside.
    pub broadside: [f64; 3],
    pub broadside_roll_rad: f64,
    pub envelope_area_m2: f64,
    pub envelope_volume_m3: f64,
    /// The inertia tensor about the center of mass in the ship's frame: xx, yy, zz, xy, xz, yz.
    /// Only port–starboard symmetry is guaranteed, so xz is generally not zero.
    pub inertia_kg_m2: [f64; 6],
    pub extent_m: f64,
}

/// A number in a part that [`FormFault::Malformed`] can name. Mirrors `lc_world::form::Number`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Number {
    Volume,
    Axes,
    Length,
    Edges,
    Corner,
    Major,
    Taper,
    Twist,
    Tilt,
    Blend,
    Anchor,
    Standoff,
    Proportions,
}

/// Why a target form was refused, naming the part where there is one. The structural checks
/// mirror `lc_world::form::FormError`; the placement rules follow them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FormFault {
    TooManyParts { found: u32 },
    DuplicateId(PartId),
    NoMind,
    SecondMind { first: PartId, second: PartId },
    Malformed { part: PartId, number: Number },
    MindPlaced(PartId),
    Unplaced(PartId),
    MissingParent { part: PartId, parent: PartId },
    Cycle(PartId),
    /// An engine not pointing fore or aft.
    EngineOffAxis(PartId),
    /// Something in an engine's clear cone.
    EngineBlocked(PartId),
    BayBlocked(PartId),
    /// An attached part not touching its parent.
    Detached(PartId),
    /// An enclosing part not containing its parent.
    Uncontained(PartId),
    /// The envelope's extent is outside `LENGTH_RANGE_M`.
    Extent,
    TooSmall(PartId),
    TooFewDrones,
}
