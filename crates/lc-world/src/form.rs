//! A ship's form: a tree of parts, each one primitive of one kind, rooted at the Mind.
//!
//! This module is the types and their structure. Sizing, geometry, capacities and the placement
//! rules are its submodules. See `lightcone/docs/29-ship-form.md`.

pub mod capacity;
pub mod grid;
pub mod place;
pub mod presets;
pub mod primitive;
pub mod rules;
pub mod sdf;

use std::collections::HashMap;

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

pub const MAX_PARTS: usize = 256;

/// Assigned by whoever adds the part and kept across refits, so steps and animation can name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PartId(pub u16);

impl std::fmt::Display for PartId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "part {}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SparMode {
    /// Each tree neighbor, grown by `spar_gap`, is cut out of the spar.
    Saddle,
    /// The spar is kept only within `spar_thickness` of its parent's surface.
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

/// Shape without size: every field is a dimensionless ratio, and scale is solved from the
/// part's volume. A part's axis is its local x.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Primitive {
    /// Semi-axes along the part's x, y and z, relative to one another.
    Ellipsoid { axes: DVec3 },
    /// Length of the straight section over the radius. Zero is a sphere.
    Capsule { length: f64 },
    /// Edges relative to one another, and the corner radius as a fraction of the shortest edge.
    Slab { edges: DVec3, corner: f64 },
    /// Length over radius, along the axis.
    Cylinder { length: f64 },
    /// Major radius over minor, about the axis.
    Torus { major: f64 },
    /// Length over the radius of the end at −x, and the +x end's radius over it. Zero taper is a
    /// cone.
    Frustum { length: f64, taper: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Mount {
    /// On the parent's surface, where a ray from its center along `anchor` (parent frame) last
    /// leaves it. `standoff` is along the normal in multiples of the child's size; negative embeds.
    Attached { anchor: DVec3, standoff: f64 },
    /// Centered on the parent and containing it.
    Enclosing,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub parent: PartId,
    pub mount: Mount,
    /// Radians, about the surface normal, or about the parent's axis when enclosing. At zero the
    /// child's axis lies along that normal or axis and its y along the parent's y projected
    /// across it, or the parent's z where the y is parallel.
    pub twist: f64,
    /// A rotation vector across the normal or axis, in the child's y and z after twist, applied
    /// after twist. Radians.
    pub tilt: DVec2,
    /// Smooth-union radius with the parent, as a fraction of the smaller part. Ignored at a spar.
    pub blend: f64,
    /// Repeat the subtree reflected through the ship's port–starboard plane.
    pub mirror: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub id: PartId,
    pub kind: Kind,
    pub primitive: Primitive,
    pub volume_m3: f64,
    /// `None` for the Mind alone; its frame is the ship's, with the nose along its axis.
    pub placement: Option<Placement>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub parts: Vec<Part>,
}

/// A number in a part that can be malformed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl std::fmt::Display for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Volume => "volume",
            Self::Axes => "axes",
            Self::Length => "length",
            Self::Edges => "edges",
            Self::Corner => "corner",
            Self::Major => "major radius",
            Self::Taper => "taper",
            Self::Twist => "twist",
            Self::Tilt => "tilt",
            Self::Blend => "blend",
            Self::Anchor => "anchor",
            Self::Standoff => "standoff",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormError {
    TooManyParts { found: usize },
    DuplicateId(PartId),
    NoMind,
    SecondMind { first: PartId, second: PartId },
    /// Not finite, or out of the sign its meaning allows. Checked because a client sends forms.
    Malformed { part: PartId, number: Number },
    MindPlaced(PartId),
    /// A part other than the Mind hangs from nothing.
    Unplaced(PartId),
    MissingParent { part: PartId, parent: PartId },
    /// Names the lowest id on the cycle, whichever part the walk started from.
    Cycle(PartId),
}

impl std::fmt::Display for FormError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyParts { found } => write!(f, "{found} parts, at most {MAX_PARTS}"),
            Self::DuplicateId(id) => write!(f, "{id} appears twice"),
            Self::NoMind => write!(f, "no Mind"),
            Self::SecondMind { first, second } => write!(f, "{second} is a second Mind beside {first}"),
            Self::Malformed { part, number } => write!(f, "{part} has a malformed {number}"),
            Self::MindPlaced(id) => write!(f, "the Mind, {id}, has a parent"),
            Self::Unplaced(id) => write!(f, "{id} has no parent"),
            Self::MissingParent { part, parent } => write!(f, "{part} hangs from {parent}, which does not exist"),
            Self::Cycle(id) => write!(f, "{id} is its own ancestor"),
        }
    }
}

impl std::error::Error for FormError {}

impl Form {
    /// Structure and well-formed numbers only: one Mind at the root, one tree, unique ids, at
    /// most [`MAX_PARTS`], and nothing NaN, infinite or of the wrong sign. Sizes and the
    /// placement rules are [`rules`].
    pub fn validate(&self) -> Result<(), FormError> {
        if self.parts.len() > MAX_PARTS {
            return Err(FormError::TooManyParts { found: self.parts.len() });
        }
        let mut index = HashMap::with_capacity(self.parts.len());
        for (i, part) in self.parts.iter().enumerate() {
            if index.insert(part.id, i).is_some() {
                return Err(FormError::DuplicateId(part.id));
            }
        }

        let mut mind = None;
        for part in &self.parts {
            if part.kind == Kind::Mind {
                if let Some(first) = mind {
                    return Err(FormError::SecondMind { first, second: part.id });
                }
                mind = Some(part.id);
            }
        }
        if mind.is_none() {
            return Err(FormError::NoMind);
        }
        for part in &self.parts {
            if let Some(number) = part.malformed() {
                return Err(FormError::Malformed { part: part.id, number });
            }
        }

        for part in &self.parts {
            match (part.kind, part.placement) {
                (Kind::Mind, Some(_)) => return Err(FormError::MindPlaced(part.id)),
                (Kind::Mind, None) => {}
                (_, None) => return Err(FormError::Unplaced(part.id)),
                (_, Some(p)) if !index.contains_key(&p.parent) => {
                    return Err(FormError::MissingParent { part: part.id, parent: p.parent });
                }
                (_, Some(_)) => {}
            }
        }

        // Every part now has exactly one existing parent and only the Mind has none, so a walk
        // that has not reached the Mind in `len` steps is going round a cycle.
        let parent = |i: usize| self.parts[i].placement.map(|p| index[&p.parent]);
        for start in 0..self.parts.len() {
            let mut at = start;
            let mut steps = 0;
            while let Some(up) = parent(at) {
                at = up;
                steps += 1;
                if steps > self.parts.len() {
                    let mut lowest = self.parts[at].id;
                    let mut on = parent(at).expect("a cycle has no root");
                    while on != at {
                        lowest = lowest.min(self.parts[on].id);
                        on = parent(on).expect("a cycle has no root");
                    }
                    return Err(FormError::Cycle(lowest));
                }
            }
        }
        Ok(())
    }
}

impl Part {
    /// The first field that is not finite or has a sign its meaning forbids. `NaN > 0.0` is
    /// false, so every test is written to refuse NaN.
    fn malformed(&self) -> Option<Number> {
        let positive = |x: f64| x.is_finite() && x > 0.0;
        let non_negative = |x: f64| x.is_finite() && x >= 0.0;
        let all_positive = |v: DVec3| v.to_array().into_iter().all(positive);
        if !positive(self.volume_m3) {
            return Some(Number::Volume);
        }
        let shape = match self.primitive {
            Primitive::Ellipsoid { axes } => (!all_positive(axes)).then_some(Number::Axes),
            Primitive::Capsule { length } => (!non_negative(length)).then_some(Number::Length),
            Primitive::Slab { edges, corner } => {
                if !all_positive(edges) {
                    Some(Number::Edges)
                } else {
                    (!non_negative(corner)).then_some(Number::Corner)
                }
            }
            Primitive::Cylinder { length } => (!positive(length)).then_some(Number::Length),
            Primitive::Torus { major } => (!positive(major)).then_some(Number::Major),
            Primitive::Frustum { length, taper } => {
                if !positive(length) {
                    Some(Number::Length)
                } else {
                    (!non_negative(taper)).then_some(Number::Taper)
                }
            }
        };
        if shape.is_some() {
            return shape;
        }
        let place = self.placement?;
        if !place.twist.is_finite() {
            return Some(Number::Twist);
        }
        if !place.tilt.is_finite() {
            return Some(Number::Tilt);
        }
        if !non_negative(place.blend) {
            return Some(Number::Blend);
        }
        match place.mount {
            Mount::Attached { anchor, .. } if !(anchor.is_finite() && anchor.length_squared() > 0.0) => {
                Some(Number::Anchor)
            }
            Mount::Attached { standoff, .. } if !standoff.is_finite() => Some(Number::Standoff),
            _ => None,
        }
    }
}

impl From<lc_proto::form::PartId> for PartId {
    fn from(id: lc_proto::form::PartId) -> Self {
        Self(id.0)
    }
}

impl From<PartId> for lc_proto::form::PartId {
    fn from(id: PartId) -> Self {
        Self(id.0)
    }
}

impl From<lc_proto::form::Kind> for Kind {
    fn from(k: lc_proto::form::Kind) -> Self {
        use lc_proto::form::{Kind as K, SparMode as S};
        match k {
            K::Mind => Self::Mind,
            K::Storage => Self::Storage,
            K::Drone => Self::Drone,
            K::Engine => Self::Engine,
            K::Living => Self::Living,
            K::Data => Self::Data,
            K::Bay => Self::Bay,
            K::Spar(S::Saddle) => Self::Spar(SparMode::Saddle),
            K::Spar(S::Strap) => Self::Spar(SparMode::Strap),
        }
    }
}

impl From<Kind> for lc_proto::form::Kind {
    fn from(k: Kind) -> Self {
        use lc_proto::form::SparMode as S;
        match k {
            Kind::Mind => Self::Mind,
            Kind::Storage => Self::Storage,
            Kind::Drone => Self::Drone,
            Kind::Engine => Self::Engine,
            Kind::Living => Self::Living,
            Kind::Data => Self::Data,
            Kind::Bay => Self::Bay,
            Kind::Spar(SparMode::Saddle) => Self::Spar(S::Saddle),
            Kind::Spar(SparMode::Strap) => Self::Spar(S::Strap),
        }
    }
}

impl From<lc_proto::form::Primitive> for Primitive {
    fn from(p: lc_proto::form::Primitive) -> Self {
        use lc_proto::form::Primitive as P;
        match p {
            P::Ellipsoid { axes } => Self::Ellipsoid { axes: DVec3::from_array(axes) },
            P::Capsule { length } => Self::Capsule { length },
            P::Slab { edges, corner } => Self::Slab { edges: DVec3::from_array(edges), corner },
            P::Cylinder { length } => Self::Cylinder { length },
            P::Torus { major } => Self::Torus { major },
            P::Frustum { length, taper } => Self::Frustum { length, taper },
        }
    }
}

impl From<Primitive> for lc_proto::form::Primitive {
    fn from(p: Primitive) -> Self {
        match p {
            Primitive::Ellipsoid { axes } => Self::Ellipsoid { axes: axes.to_array() },
            Primitive::Capsule { length } => Self::Capsule { length },
            Primitive::Slab { edges, corner } => Self::Slab { edges: edges.to_array(), corner },
            Primitive::Cylinder { length } => Self::Cylinder { length },
            Primitive::Torus { major } => Self::Torus { major },
            Primitive::Frustum { length, taper } => Self::Frustum { length, taper },
        }
    }
}

impl From<lc_proto::form::Mount> for Mount {
    fn from(m: lc_proto::form::Mount) -> Self {
        match m {
            lc_proto::form::Mount::Attached { anchor, standoff } => {
                Self::Attached { anchor: DVec3::from_array(anchor), standoff }
            }
            lc_proto::form::Mount::Enclosing => Self::Enclosing,
        }
    }
}

impl From<Mount> for lc_proto::form::Mount {
    fn from(m: Mount) -> Self {
        match m {
            Mount::Attached { anchor, standoff } => Self::Attached { anchor: anchor.to_array(), standoff },
            Mount::Enclosing => Self::Enclosing,
        }
    }
}

impl From<lc_proto::form::Placement> for Placement {
    fn from(p: lc_proto::form::Placement) -> Self {
        Self {
            parent: p.parent.into(),
            mount: p.mount.into(),
            twist: p.twist,
            tilt: DVec2::from_array(p.tilt),
            blend: p.blend,
            mirror: p.mirror,
        }
    }
}

impl From<Placement> for lc_proto::form::Placement {
    fn from(p: Placement) -> Self {
        Self {
            parent: p.parent.into(),
            mount: p.mount.into(),
            twist: p.twist,
            tilt: p.tilt.to_array(),
            blend: p.blend,
            mirror: p.mirror,
        }
    }
}

impl From<lc_proto::form::Part> for Part {
    fn from(p: lc_proto::form::Part) -> Self {
        Self {
            id: p.id.into(),
            kind: p.kind.into(),
            primitive: p.primitive.into(),
            volume_m3: p.volume_m3,
            placement: p.placement.map(Into::into),
        }
    }
}

impl From<Part> for lc_proto::form::Part {
    fn from(p: Part) -> Self {
        Self {
            id: p.id.into(),
            kind: p.kind.into(),
            primitive: p.primitive.into(),
            volume_m3: p.volume_m3,
            placement: p.placement.map(Into::into),
        }
    }
}

impl From<&lc_proto::Form> for Form {
    fn from(f: &lc_proto::Form) -> Self {
        Self { parts: f.parts.iter().map(|&p| p.into()).collect() }
    }
}

impl From<&Form> for lc_proto::Form {
    fn from(f: &Form) -> Self {
        Self { parts: f.parts.iter().map(|&p| p.into()).collect() }
    }
}

impl From<lc_proto::form::Number> for Number {
    fn from(n: lc_proto::form::Number) -> Self {
        use lc_proto::form::Number as N;
        match n {
            N::Volume => Self::Volume,
            N::Axes => Self::Axes,
            N::Length => Self::Length,
            N::Edges => Self::Edges,
            N::Corner => Self::Corner,
            N::Major => Self::Major,
            N::Taper => Self::Taper,
            N::Twist => Self::Twist,
            N::Tilt => Self::Tilt,
            N::Blend => Self::Blend,
            N::Anchor => Self::Anchor,
            N::Standoff => Self::Standoff,
        }
    }
}

impl From<Number> for lc_proto::form::Number {
    fn from(n: Number) -> Self {
        match n {
            Number::Volume => Self::Volume,
            Number::Axes => Self::Axes,
            Number::Length => Self::Length,
            Number::Edges => Self::Edges,
            Number::Corner => Self::Corner,
            Number::Major => Self::Major,
            Number::Taper => Self::Taper,
            Number::Twist => Self::Twist,
            Number::Tilt => Self::Tilt,
            Number::Blend => Self::Blend,
            Number::Anchor => Self::Anchor,
            Number::Standoff => Self::Standoff,
        }
    }
}

/// One way only: a refusal is said by the server and read by a person.
impl From<FormError> for lc_proto::FormFault {
    fn from(e: FormError) -> Self {
        match e {
            FormError::TooManyParts { found } => Self::TooManyParts { found: u32::try_from(found).unwrap_or(u32::MAX) },
            FormError::DuplicateId(id) => Self::DuplicateId(id.into()),
            FormError::NoMind => Self::NoMind,
            FormError::SecondMind { first, second } => Self::SecondMind { first: first.into(), second: second.into() },
            FormError::Malformed { part, number } => Self::Malformed { part: part.into(), number: number.into() },
            FormError::MindPlaced(id) => Self::MindPlaced(id.into()),
            FormError::Unplaced(id) => Self::Unplaced(id.into()),
            FormError::MissingParent { part, parent } => {
                Self::MissingParent { part: part.into(), parent: parent.into() }
            }
            FormError::Cycle(id) => Self::Cycle(id.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mind(id: u16) -> Part {
        Part {
            id: PartId(id),
            kind: Kind::Mind,
            primitive: Primitive::Slab { edges: DVec3::ONE, corner: 0.0 },
            volume_m3: 1_000.0,
            placement: None,
        }
    }

    fn hung(id: u16, kind: Kind, parent: u16) -> Part {
        Part {
            id: PartId(id),
            kind,
            primitive: Primitive::Ellipsoid { axes: DVec3::new(2.0, 1.0, 1.0) },
            volume_m3: 50_000.0,
            placement: Some(Placement {
                parent: PartId(parent),
                mount: Mount::Attached { anchor: DVec3::Y, standoff: 0.0 },
                twist: 0.0,
                tilt: DVec2::ZERO,
                blend: 0.1,
                mirror: false,
            }),
        }
    }

    fn ship() -> Form {
        let mut core = hung(1, Kind::Storage, 0);
        core.placement.as_mut().unwrap().mount = Mount::Enclosing;
        Form {
            parts: vec![
                mind(0),
                core,
                hung(2, Kind::Engine, 1),
                hung(3, Kind::Spar(SparMode::Strap), 1),
                hung(4, Kind::Drone, 3),
            ],
        }
    }

    #[test]
    fn a_tree_rooted_at_the_mind_is_accepted() {
        assert_eq!(ship().validate(), Ok(()));
    }

    #[test]
    fn a_second_mind_is_refused_by_name() {
        let mut form = ship();
        form.parts.push(Part { id: PartId(9), ..mind(0) });
        assert_eq!(form.validate(), Err(FormError::SecondMind { first: PartId(0), second: PartId(9) }));
    }

    #[test]
    fn a_form_without_a_mind_is_refused() {
        let mut form = ship();
        form.parts.remove(0);
        assert_eq!(form.validate(), Err(FormError::NoMind));
    }

    #[test]
    fn the_mind_must_be_the_root() {
        let mut form = ship();
        form.parts[0].placement = hung(0, Kind::Mind, 1).placement;
        assert_eq!(form.validate(), Err(FormError::MindPlaced(PartId(0))));
    }

    #[test]
    fn a_part_with_no_parent_is_refused_by_name() {
        let mut form = ship();
        form.parts[2].placement = None;
        assert_eq!(form.validate(), Err(FormError::Unplaced(PartId(2))));
    }

    #[test]
    fn a_missing_parent_is_refused_by_name() {
        let mut form = ship();
        form.parts.push(hung(5, Kind::Data, 42));
        assert_eq!(form.validate(), Err(FormError::MissingParent { part: PartId(5), parent: PartId(42) }));
    }

    #[test]
    fn a_cycle_is_refused_by_name() {
        let mut form = ship();
        // 3 → 4 → 3, and 2 hangs from nothing that reaches the Mind either.
        form.parts[3].placement.as_mut().unwrap().parent = PartId(4);
        form.parts[2].placement.as_mut().unwrap().parent = PartId(4);
        assert_eq!(form.validate(), Err(FormError::Cycle(PartId(3))));
    }

    #[test]
    fn a_part_that_is_its_own_parent_is_a_cycle() {
        let mut form = ship();
        form.parts[4].placement.as_mut().unwrap().parent = PartId(4);
        assert_eq!(form.validate(), Err(FormError::Cycle(PartId(4))));
    }

    #[test]
    fn a_malformed_number_is_refused_by_part_and_field() {
        let cases: [(usize, fn(&mut Part), Number); 10] = [
            (2, |p| p.volume_m3 = f64::NAN, Number::Volume),
            (2, |p| p.volume_m3 = 0.0, Number::Volume),
            (2, |p| p.primitive = Primitive::Ellipsoid { axes: DVec3::new(1.0, -1.0, 1.0) }, Number::Axes),
            (2, |p| p.primitive = Primitive::Slab { edges: DVec3::ONE, corner: f64::NAN }, Number::Corner),
            (2, |p| p.primitive = Primitive::Frustum { length: f64::INFINITY, taper: 0.5 }, Number::Length),
            (2, |p| p.placement.as_mut().unwrap().tilt = DVec2::new(0.0, f64::NAN), Number::Tilt),
            (2, |p| p.placement.as_mut().unwrap().blend = -0.1, Number::Blend),
            (2, |p| p.placement.as_mut().unwrap().blend = f64::INFINITY, Number::Blend),
            (2, |p| p.placement.as_mut().unwrap().mount = Mount::Attached { anchor: DVec3::ZERO, standoff: 0.0 }, Number::Anchor),
            (0, |p| p.volume_m3 = f64::NAN, Number::Volume),
        ];
        for (index, spoil, number) in cases {
            let mut form = ship();
            spoil(&mut form.parts[index]);
            let part = form.parts[index].id;
            assert_eq!(form.validate(), Err(FormError::Malformed { part, number }), "{number}");
        }
    }

    #[test]
    fn edge_values_that_mean_something_are_accepted() {
        let mut form = ship();
        form.parts[2].primitive = Primitive::Capsule { length: 0.0 };
        form.parts[3].primitive = Primitive::Frustum { length: 2.0, taper: 0.0 };
        form.parts[4].placement.as_mut().unwrap().mount = Mount::Attached { anchor: DVec3::X, standoff: -0.5 };
        assert_eq!(form.validate(), Ok(()));
    }

    #[test]
    fn duplicate_ids_are_refused() {
        let mut form = ship();
        form.parts.push(hung(2, Kind::Living, 1));
        assert_eq!(form.validate(), Err(FormError::DuplicateId(PartId(2))));
    }

    #[test]
    fn at_most_max_parts() {
        let mut form = Form { parts: vec![mind(0)] };
        form.parts.extend((1..MAX_PARTS as u16).map(|i| hung(i, Kind::Storage, i - 1)));
        assert_eq!(form.validate(), Ok(()));
        form.parts.push(hung(MAX_PARTS as u16, Kind::Storage, 0));
        assert_eq!(form.validate(), Err(FormError::TooManyParts { found: MAX_PARTS + 1 }));
    }

    #[test]
    fn a_form_round_trips_through_the_wire() {
        let mut form = ship();
        form.parts[2].primitive = Primitive::Frustum { length: 2.0, taper: 0.5 };
        form.parts[3].primitive = Primitive::Slab { edges: DVec3::new(4.0, 2.0, 0.5), corner: 0.1 };
        form.parts[4].primitive = Primitive::Torus { major: 3.0 };
        form.parts.push(Part { primitive: Primitive::Capsule { length: 1.5 }, ..hung(5, Kind::Bay, 1) });
        let spar = hung(6, Kind::Spar(SparMode::Saddle), 1);
        form.parts.push(Part { primitive: Primitive::Cylinder { length: 2.5 }, ..spar });
        let place = form.parts[4].placement.as_mut().unwrap();
        place.twist = 0.3;
        place.tilt = DVec2::new(-0.1, 0.2);
        place.mirror = true;
        let wire = lc_proto::Form::from(&form);
        let back = Form::from(&lc_proto::decode::<lc_proto::Form>(&lc_proto::encode(&wire)).unwrap());
        assert_eq!(back, form);
    }

    #[test]
    fn a_refused_form_names_its_part_on_the_wire() {
        let mut form = ship();
        form.parts[2].placement.as_mut().unwrap().blend = f64::NAN;
        let fault = lc_proto::FormFault::from(form.validate().unwrap_err());
        let number = lc_proto::form::Number::Blend;
        assert_eq!(fault, lc_proto::FormFault::Malformed { part: lc_proto::form::PartId(2), number });
        assert_eq!(Number::from(number), Number::Blend);
    }

    #[test]
    fn a_form_round_trips_through_serde() {
        let form = ship();
        let json = serde_json::to_string(&form).unwrap();
        assert_eq!(serde_json::from_str::<Form>(&json).unwrap(), form);
        let bytes = postcard::to_stdvec(&form).unwrap();
        assert_eq!(postcard::from_bytes::<Form>(&bytes).unwrap(), form);
    }
}
