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
/// part's volume.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Primitive {
    /// Semi-axes along the part's x, y and z, relative to one another.
    Ellipsoid { axes: DVec3 },
    /// Length of the straight section over the radius.
    Capsule { length: f64 },
    /// Edges relative to one another, and the corner radius as a fraction of the shortest edge.
    Slab { edges: DVec3, corner: f64 },
    /// Length over radius.
    Cylinder { length: f64 },
    /// Major radius over minor.
    Torus { major: f64 },
    /// Length over the first end's radius, and the second end's radius over the first's.
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
    /// Radians, about the surface normal, or about the parent's axis when enclosing.
    pub twist: f64,
    /// The child's axis away from that normal or axis, as a rotation vector in the plane across
    /// it. Radians.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormError {
    TooManyParts { found: usize },
    DuplicateId(PartId),
    NoMind,
    SecondMind { first: PartId, second: PartId },
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
            Self::MindPlaced(id) => write!(f, "the Mind, {id}, has a parent"),
            Self::Unplaced(id) => write!(f, "{id} has no parent"),
            Self::MissingParent { part, parent } => write!(f, "{part} hangs from {parent}, which does not exist"),
            Self::Cycle(id) => write!(f, "{id} is its own ancestor"),
        }
    }
}
impl std::error::Error for FormError {}

impl Form {
    pub fn part(&self, id: PartId) -> Option<&Part> {
        self.parts.iter().find(|p| p.id == id)
    }

    pub fn mind(&self) -> Option<&Part> {
        self.parts.iter().find(|p| p.kind == Kind::Mind)
    }

    /// Structure only: one Mind at the root, one tree, unique ids, at most [`MAX_PARTS`].
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
    fn a_second_root_is_refused_by_name() {
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
    fn a_form_round_trips_through_serde() {
        let form = ship();
        let json = serde_json::to_string(&form).unwrap();
        assert_eq!(serde_json::from_str::<Form>(&json).unwrap(), form);
        let bytes = postcard::to_stdvec(&form).unwrap();
        assert_eq!(postcard::from_bytes::<Form>(&bytes).unwrap(), form);
    }
}
