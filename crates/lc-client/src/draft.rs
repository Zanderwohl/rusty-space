//! The draft: the form being edited, beside the ship's own, and every edit to it as a value.
//!
//! An [`Edit`] is the parts it touches before and after, by id, so writing its `before` back
//! undoes it exactly, a removal's subtree and ids included. Only
//! [`Action::EditForm`](crate::action::Action::EditForm) writes the draft. See
//! `lightcone/docs/29-ship-form.md` §The editor.

use std::collections::{BTreeMap, BTreeSet};

use bevy::color::Color;
use glam::{DVec2, DVec3};
use lc_world::fitting::Balance;
use lc_world::form::{Form, FormError, Kind, Mount, Part, PartId, Placement, Primitive, SparMode, rules};
use lc_world::refit::rounds::{Change, changes};

use crate::snap;

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    /// In id order.
    pub form: Form,
    pub ship: Form,
    /// An enclosing part's last anchor and standoff, restored when it is attached again.
    remembered: BTreeMap<PartId, (DVec3, f64)>,
}

/// What an edit did, for the history to list it by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    Add,
    Remove,
    Resize,
    /// Primitive, proportions, a spar's mode, or kind.
    Reshape,
    /// Parent, mount, anchor, twist, tilt, standoff or blend.
    Move,
    Mirror,
    Whole,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub what: What,
    pub part: PartId,
    /// An id in only one of these is a part removed or added.
    pub before: Vec<Part>,
    pub after: Vec<Part>,
    /// False while a handle is held. Every frame's `before` is the part as the drag began, so the
    /// settled edit is the whole gesture.
    pub settled: bool,
}

impl Edit {
    pub fn inverse(&self) -> Edit {
        Edit { before: self.after.clone(), after: self.before.clone(), ..self.clone() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refused {
    NoSuchPart(PartId),
    Mind,
    Fault(FormError),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchPart(id) => write!(f, "there is no {id}"),
            Self::Mind => write!(f, "the Mind cannot be changed"),
            Self::Fault(e) => e.fmt(f),
        }
    }
}

/// How a part differs from the ship, as the round would change it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Build,
    /// In the ship and not in the draft.
    Dismantle,
    /// Smaller, or a mirrored copy fewer, and still there.
    Shrink,
    Move,
    /// A reshape or a change of kind.
    Rebuild,
}

impl Mark {
    pub fn word(self) -> &'static str {
        match self {
            Mark::Build => "build",
            Mark::Dismantle => "dismantle",
            Mark::Shrink => "shrink",
            Mark::Move => "move",
            Mark::Rebuild => "rebuild",
        }
    }

    /// `18-ui-style.md` §Refit marks. One hue per phase of the round, so a shrink wears the
    /// dismantle's.
    pub fn color(self) -> Color {
        match self {
            Mark::Build => em_ui::vfd::TEXT,
            Mark::Dismantle | Mark::Shrink => Color::srgb(1.0, 0.70, 0.78),
            Mark::Move => Color::srgb(0.45, 0.83, 1.0),
            Mark::Rebuild => Color::srgb(0.85, 0.76, 1.0),
        }
    }
}

/// A number the fields panel edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Volume,
    /// An ellipsoid's semi-axis ratios or a slab's edge ratios.
    Axis(usize),
    Length,
    Corner,
    Major,
    Taper,
    Anchor(usize),
    Standoff,
    /// Degrees.
    Twist,
    /// Degrees, along the twisted y and z.
    Tilt(usize),
    Blend,
    Parent,
}

/// The primitives the editor adds, each at proportions that read as that primitive.
pub const PRIMITIVES: [Primitive; 6] = [
    Primitive::Ellipsoid { axes: DVec3::new(2.0, 1.0, 1.0) },
    Primitive::Capsule { length: 2.0 },
    Primitive::Slab { edges: DVec3::new(1.0, 2.0, 2.0), corner: 0.2 },
    Primitive::Cylinder { length: 2.0 },
    Primitive::Torus { major: 3.0 },
    Primitive::Frustum { length: 2.0, taper: 1.5 },
];

pub const KINDS: [Kind; 7] =
    [Kind::Storage, Kind::Drone, Kind::Engine, Kind::Living, Kind::Data, Kind::Bay, Kind::Spar(SparMode::Saddle)];

/// Of the parent's volume, what an added part starts at, before snapping.
const ADDED_SHARE: f64 = 0.1;
/// Sunk a little into its parent, so it touches.
const ADDED_STANDOFF: f64 = -0.1;

pub fn primitive_name(primitive: &Primitive) -> &'static str {
    match primitive {
        Primitive::Ellipsoid { .. } => "ellipsoid",
        Primitive::Capsule { .. } => "capsule",
        Primitive::Slab { .. } => "slab",
        Primitive::Cylinder { .. } => "cylinder",
        Primitive::Torus { .. } => "torus",
        Primitive::Frustum { .. } => "frustum",
    }
}

pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Mind => "Mind",
        Kind::Storage => "storage",
        Kind::Drone => "drones",
        Kind::Engine => "engine",
        Kind::Living => "living",
        Kind::Data => "data",
        Kind::Bay => "bay",
        Kind::Spar(SparMode::Saddle) => "spar, saddle",
        Kind::Spar(SparMode::Strap) => "spar, strap",
    }
}

pub fn next_primitive(current: &Primitive) -> Primitive {
    let at = PRIMITIVES.iter().position(|p| primitive_name(p) == primitive_name(current)).unwrap_or(0);
    PRIMITIVES[(at + 1) % PRIMITIVES.len()]
}

pub fn next_kind(current: Kind) -> Kind {
    let same = |k: &Kind| std::mem::discriminant(k) == std::mem::discriminant(&current);
    let at = KINDS.iter().position(same).unwrap_or(0);
    KINDS[(at + 1) % KINDS.len()]
}

impl Draft {
    pub fn new(ship: Form) -> Self {
        let mut form = ship.clone();
        form.parts.sort_by_key(|p| p.id);
        Self { form, ship, remembered: BTreeMap::new() }
    }

    pub fn part(&self, id: PartId) -> Option<&Part> {
        self.form.parts.iter().find(|p| p.id == id)
    }

    fn editable(&self, id: PartId) -> Result<&Part, Refused> {
        let part = self.part(id).ok_or(Refused::NoSuchPart(id))?;
        if part.kind == Kind::Mind { Err(Refused::Mind) } else { Ok(part) }
    }

    pub fn children(&self, id: PartId) -> impl Iterator<Item = &Part> {
        self.form.parts.iter().filter(move |p| p.placement.is_some_and(|pl| pl.parent == id))
    }

    /// The part first.
    pub fn subtree(&self, id: PartId) -> Vec<Part> {
        let mut out: Vec<Part> = self.part(id).into_iter().copied().collect();
        let mut at = 0;
        while at < out.len() {
            let parent = out[at].id;
            out.extend(self.children(parent).copied());
            at += 1;
        }
        out
    }

    /// Depth first from the Mind, children in id order, each with its depth.
    pub fn tree(&self) -> Vec<(usize, &Part)> {
        let mut out = Vec::new();
        let mut stack: Vec<(usize, &Part)> =
            self.form.parts.iter().filter(|p| p.placement.is_none()).map(|p| (0, p)).collect();
        while let Some((depth, part)) = stack.pop() {
            out.push((depth, part));
            let mut children: Vec<&Part> = self.children(part.id).collect();
            children.reverse();
            stack.extend(children.into_iter().map(|c| (depth + 1, c)));
        }
        out
    }

    /// Refused, changing nothing, when the result is no form or has a part too small that was
    /// not too small before. Too few drones is left for Apply to refuse: a draft may pass through
    /// none on the way to a design.
    pub fn apply(&mut self, edit: &Edit, balance: &Balance) -> Result<(), Refused> {
        let named: BTreeSet<PartId> = edit.before.iter().chain(&edit.after).map(|p| p.id).collect();
        let mut next = self.form.clone();
        next.parts.retain(|p| !named.contains(&p.id));
        next.parts.extend(edit.after.iter().copied());
        next.parts.sort_by_key(|p| p.id);
        next.validate().map_err(Refused::Fault)?;
        let had = rules::sizes(&self.form, balance);
        let new = |f: &&FormError| !had.contains(f) && **f != FormError::TooFewDrones;
        if let Some(&fault) = rules::sizes(&next, balance).iter().find(new) {
            return Err(Refused::Fault(fault));
        }
        for was in &edit.before {
            let now = edit.after.iter().find(|p| p.id == was.id).and_then(|p| p.placement);
            if let (Some(Placement { mount: Mount::Attached { anchor, standoff }, .. }), Some(now)) = (was.placement, now)
                && now.mount == Mount::Enclosing
            {
                self.remembered.insert(was.id, (anchor, standoff));
            }
        }
        self.form = next;
        Ok(())
    }

    fn one(&self, what: What, before: Part, after: Part) -> Edit {
        Edit { what, part: before.id, before: vec![before], after: vec![after], settled: true }
    }

    pub fn resize(&self, id: PartId, volume_m3: f64) -> Result<Edit, Refused> {
        let part = *self.editable(id)?;
        Ok(self.one(What::Resize, part, Part { volume_m3, ..part }))
    }

    pub fn reshape(&self, id: PartId, primitive: Primitive) -> Result<Edit, Refused> {
        let part = *self.editable(id)?;
        Ok(self.one(What::Reshape, part, Part { primitive, ..part }))
    }

    pub fn set_kind(&self, id: PartId, kind: Kind) -> Result<Edit, Refused> {
        let part = *self.editable(id)?;
        if kind == Kind::Mind {
            return Err(Refused::Mind);
        }
        Ok(self.one(What::Reshape, part, Part { kind, ..part }))
    }

    pub fn spar_mode(&self, id: PartId, mode: SparMode) -> Result<Edit, Refused> {
        self.set_kind(id, Kind::Spar(mode))
    }

    pub fn place(&self, id: PartId, placement: Placement) -> Result<Edit, Refused> {
        let part = *self.editable(id)?;
        let what = match part.placement {
            Some(was) if Placement { mirror: placement.mirror, ..was } == placement => What::Mirror,
            _ => What::Move,
        };
        Ok(self.one(what, part, Part { placement: Some(placement), ..part }))
    }

    fn placement(&self, id: PartId) -> Result<Placement, Refused> {
        self.editable(id)?.placement.ok_or(Refused::Mind)
    }

    pub fn mirror(&self, id: PartId, on: bool) -> Result<Edit, Refused> {
        self.place(id, Placement { mirror: on, ..self.placement(id)? })
    }

    pub fn toggle_mount(&self, id: PartId) -> Result<Edit, Refused> {
        let placement = self.placement(id)?;
        let mount = match placement.mount {
            Mount::Attached { .. } => Mount::Enclosing,
            Mount::Enclosing => {
                let (anchor, standoff) = self.remembered.get(&id).copied().unwrap_or((DVec3::X, ADDED_STANDOFF));
                Mount::Attached { anchor, standoff }
            }
        };
        self.place(id, Placement { mount, ..placement })
    }

    /// `anchor` in the parent's frame. An enclosing part is left as it is.
    pub fn anchor(&self, id: PartId, anchor: DVec3) -> Result<Edit, Refused> {
        let placement = self.placement(id)?;
        let mount = match placement.mount {
            Mount::Attached { standoff, .. } => Mount::Attached { anchor, standoff },
            Mount::Enclosing => Mount::Enclosing,
        };
        self.place(id, Placement { mount, ..placement })
    }

    /// Refused where the part would no longer touch its parent.
    pub fn standoff(&self, id: PartId, standoff: f64, balance: &Balance) -> Result<Edit, Refused> {
        let placement = self.placement(id)?;
        let part = self.editable(id)?;
        let parent = self.part(placement.parent).ok_or(Refused::NoSuchPart(placement.parent))?;
        let moved = match placement.mount {
            Mount::Attached { anchor, .. } => Placement { mount: Mount::Attached { anchor, standoff }, ..placement },
            Mount::Enclosing => placement,
        };
        let min = balance.min_part_m3;
        if !touches(&parent.shape(min), &part.shape(min), &moved) {
            return Err(Refused::Fault(FormError::Detached(id)));
        }
        let mount = match placement.mount {
            Mount::Attached { anchor, .. } => Mount::Attached { anchor, standoff },
            Mount::Enclosing => Mount::Enclosing,
        };
        self.place(id, Placement { mount, ..placement })
    }

    pub fn twist(&self, id: PartId, twist: f64) -> Result<Edit, Refused> {
        self.place(id, Placement { twist, ..self.placement(id)? })
    }

    pub fn add(&self, parent: PartId, kind: Kind, primitive: Primitive, anchor: DVec3, balance: &Balance) -> Result<Edit, Refused> {
        let host = self.part(parent).ok_or(Refused::NoSuchPart(parent))?;
        if kind == Kind::Mind {
            return Err(Refused::Mind);
        }
        let host_m3 = host.shape(balance.min_part_m3).volume();
        let volume_m3 = snap::volume((host_m3 * ADDED_SHARE).max(balance.min_part_m3), balance.min_part_m3, false);
        let id = PartId(self.form.parts.iter().map(|p| p.id.0).max().map_or(0, |m| m + 1));
        let placement = Placement {
            parent,
            mount: Mount::Attached { anchor, standoff: ADDED_STANDOFF },
            twist: 0.0,
            tilt: DVec2::ZERO,
            blend: 0.0,
            mirror: false,
        };
        let part = Part { id, kind, primitive, volume_m3, placement: Some(placement) };
        Ok(Edit { what: What::Add, part: id, before: Vec::new(), after: vec![part], settled: true })
    }

    /// The part and its subtree. The last drones may go: see [`Draft::apply`].
    pub fn remove(&self, id: PartId) -> Result<Edit, Refused> {
        self.editable(id)?;
        let before = self.subtree(id);
        Ok(Edit { what: What::Remove, part: id, before, after: Vec::new(), settled: true })
    }

    pub fn reset(&self) -> Edit {
        self.replace(self.ship.clone())
    }

    pub fn replace(&self, form: Form) -> Edit {
        let mind = self.form.parts.iter().find(|p| p.kind == Kind::Mind).map_or(PartId(0), |p| p.id);
        Edit { what: What::Whole, part: mind, before: self.form.parts.clone(), after: form.parts, settled: true }
    }

    /// Not snapped. Angles in degrees.
    pub fn set_field(&self, id: PartId, field: Field, value: f64) -> Result<Edit, Refused> {
        let part = *self.editable(id)?;
        let placement = part.placement.ok_or(Refused::Mind)?;
        let deg = value.to_radians();
        let reshaped = |primitive| self.reshape(id, primitive);
        match (field, part.primitive) {
            (Field::Volume, _) => self.resize(id, value),
            (Field::Axis(i), Primitive::Ellipsoid { mut axes }) => {
                axes[i] = value;
                reshaped(Primitive::Ellipsoid { axes })
            }
            (Field::Axis(i), Primitive::Slab { mut edges, corner }) => {
                edges[i] = value;
                reshaped(Primitive::Slab { edges, corner })
            }
            (Field::Length, Primitive::Capsule { .. }) => reshaped(Primitive::Capsule { length: value }),
            (Field::Length, Primitive::Cylinder { .. }) => reshaped(Primitive::Cylinder { length: value }),
            (Field::Length, Primitive::Frustum { taper, .. }) => reshaped(Primitive::Frustum { length: value, taper }),
            (Field::Taper, Primitive::Frustum { length, .. }) => reshaped(Primitive::Frustum { length, taper: value }),
            (Field::Corner, Primitive::Slab { edges, .. }) => reshaped(Primitive::Slab { edges, corner: value }),
            (Field::Major, Primitive::Torus { .. }) => reshaped(Primitive::Torus { major: value }),
            (Field::Anchor(i), _) => match placement.mount {
                Mount::Attached { mut anchor, .. } => {
                    anchor[i] = value;
                    self.anchor(id, anchor)
                }
                Mount::Enclosing => Err(Refused::Fault(FormError::Malformed { part: id, number: lc_world::form::Number::Anchor })),
            },
            (Field::Standoff, _) => self.standoff(id, value, &Balance::DEFAULT),
            (Field::Twist, _) => self.twist(id, deg),
            (Field::Tilt(i), _) => {
                let mut tilt = placement.tilt;
                tilt[i] = deg;
                self.place(id, Placement { tilt, ..placement })
            }
            (Field::Blend, _) => self.place(id, Placement { blend: value, ..placement }),
            (Field::Parent, _) => {
                let parent = PartId(value.round().clamp(0.0, f64::from(u16::MAX)) as u16);
                self.place(id, Placement { parent, ..placement })
            }
            (field, primitive) => Err(Refused::Fault(FormError::Malformed { part: id, number: number_of(field, &primitive) })),
        }
    }

    /// Labeled figures: what the part does for its kind, then its mass, every copy counted. A
    /// store's wet mass is full.
    pub fn stats(&self, id: PartId, balance: &Balance) -> Vec<(&'static str, String)> {
        use lc_world::fitting::{C2, ONBOARD_DATA_BYTES};
        use lc_world::form::capacity::{Capacities, dry_mass_kg, part_kg};
        let Some(part) = self.part(id) else { return Vec::new() };
        let copies = f64::from(self.form.copies().get(&id).copied().unwrap_or(1));
        let alone = Form { parts: vec![Part { volume_m3: part.volume_m3 * copies, placement: None, ..*part }] };
        let held = Capacities::of(&alone, balance);
        let dry_kg = part_kg(part, balance) * copies;
        let mut out = match part.kind {
            Kind::Storage => vec![("capacity", format!("{} ME", figure(held.storage_j / balance.module_energy_j())))],
            Kind::Drone => vec![("building", format!("{} W", figure(held.building_w)))],
            Kind::Engine => {
                let thrust_n = held.aperture_w / lc_world::flight::C_M_S;
                let full_kg = dry_mass_kg(&self.form, balance) + Capacities::of(&self.form, balance).storage_j / C2;
                vec![
                    ("aperture", format!("{} W", figure(held.aperture_w))),
                    ("thrust", format!("{} N", figure(thrust_n))),
                    ("ship, full", format!("{} g", figure(thrust_n / full_kg / lc_world::flight::G0))),
                ]
            }
            Kind::Living => vec![("drain", format!("{} W", figure(held.drain_w)))],
            Kind::Data => vec![("capacity", format!("{} B", figure(held.data_b - ONBOARD_DATA_BYTES)))],
            Kind::Bay => {
                let mouth = part.shape(balance.min_part_m3).extent(glam::DMat3::IDENTITY, 0.0);
                vec![("mouth", format!("{} m", figure(2.0 * mouth.y.min(mouth.z))))]
            }
            Kind::Mind | Kind::Spar(_) => Vec::new(),
        };
        if copies > 1.0 {
            out.push(("copies", format!("{copies}")));
        }
        out.push(("dry mass", format!("{} kg", figure(dry_kg))));
        if part.kind == Kind::Storage {
            out.push(("wet mass", format!("{} kg", figure(dry_kg + held.storage_j / C2))));
        }
        out
    }

    /// By F8's own diff, so the marks are the round's steps. Includes parts only in the ship.
    pub fn marks(&self, balance: &Balance) -> BTreeMap<PartId, Mark> {
        let mut seen: BTreeMap<PartId, Vec<Change>> = BTreeMap::new();
        for (id, change) in changes(&self.ship, &self.form, balance) {
            seen.entry(id).or_default().push(change);
        }
        seen.into_iter()
            .map(|(id, changes)| {
                let has = |c: Change| changes.contains(&c);
                let in_draft = self.part(id).is_some();
                let in_ship = self.ship.parts.iter().any(|p| p.id == id);
                let mark = if has(Change::Remove) && has(Change::Add) && in_draft && in_ship {
                    Mark::Rebuild
                } else if has(Change::Add) || has(Change::Grow) {
                    Mark::Build
                } else if !in_draft {
                    Mark::Dismantle
                } else if has(Change::Remove) || has(Change::Shrink) {
                    Mark::Shrink
                } else {
                    Mark::Move
                };
                (id, mark)
            })
            .collect()
    }

}

/// `--draft`: `edits` for one of each mark on `ship`, or a preset by `--form`'s spelling.
pub fn staged(name: &str, ship: &Form, balance: &Balance) -> Option<Form> {
    if name != "edits" {
        return crate::parts::fixture(name);
    }
    let mut d = Draft::new(ship.clone());
    let by_kind = |d: &Draft, kind: Kind| d.form.parts.iter().find(|p| p.kind == kind).copied();
    let edits = [
        by_kind(&d, Kind::Drone).map(|p| d.reshape(p.id, PRIMITIVES[0])),
        by_kind(&d, Kind::Engine).map(|p| d.resize(p.id, snap::volume(p.volume_m3 * 0.6, balance.min_part_m3, false))),
        by_kind(&d, Kind::Data).map(|p| d.remove(p.id)),
        by_kind(&d, Kind::Storage).map(|p| d.add(p.id, Kind::Bay, PRIMITIVES[3], DVec3::new(0.0, -1.0, 0.0), balance)),
    ];
    for edit in edits.into_iter().flatten() {
        d.apply(&edit.ok()?, balance).ok()?;
    }
    if let Some(living) = by_kind(&d, Kind::Living) {
        d.apply(&d.anchor(living.id, DVec3::new(-1.0, 0.0, 1.0)).ok()?, balance).ok()?;
    }
    Some(d.form)
}

fn number_of(field: Field, primitive: &Primitive) -> lc_world::form::Number {
    use lc_world::form::Number;
    match (field, primitive) {
        (Field::Axis(_), Primitive::Slab { .. }) => Number::Edges,
        (Field::Axis(_), _) => Number::Axes,
        (Field::Length, _) => Number::Length,
        (Field::Corner, _) => Number::Corner,
        (Field::Major, _) => Number::Major,
        (Field::Taper, _) => Number::Taper,
        _ => Number::Proportions,
    }
}

const TOUCH_SAMPLES: usize = 256;
/// Of the child's smallest half-extent, how near its surface must come to the parent's to touch.
const TOUCH_TOLERANCE: f64 = 0.02;

/// Whether any sampled point of `child`'s surface is within [`TOUCH_TOLERANCE`] of `parent`. A
/// quicker reading of the grid rule Apply checks, so a handle can stop before the part comes away.
/// A positive standoff alone is not floating: tilt swings a part back into its parent.
pub fn touches(parent: &lc_world::form::primitive::Shape, child: &lc_world::form::primitive::Shape, placement: &Placement) -> bool {
    let pose = lc_world::form::place::relative(parent, child, placement);
    let tolerance = TOUCH_TOLERANCE * child.extent(glam::DMat3::IDENTITY, 0.0).min_element();
    // Fibonacci sphere.
    let golden = std::f64::consts::PI * (3.0 - 5f64.sqrt());
    (0..TOUCH_SAMPLES).any(|k| {
        let z = 1.0 - 2.0 * (k as f64 + 0.5) / TOUCH_SAMPLES as f64;
        let r = (1.0 - z * z).sqrt();
        let (s, c) = (golden * k as f64).sin_cos();
        let on = child.exit(DVec3::new(r * c, r * s, z)).point;
        parent.distance(pose.to_outer(on)) <= tolerance
    })
}

/// At fixed volume: volume is stored and scale solved, so a ratio alone cannot change it.
pub fn stretched(primitive: Primitive, axis: usize, factor: f64, fine: bool) -> Primitive {
    // Across the axis the radius grows, which is the length ratio shrinking.
    let along = |ratio: f64| {
        let f = if axis == 0 { factor } else { 1.0 / factor };
        snap::ratio(ratio.max(MIN_RATIO) * f, fine)
    };
    match primitive {
        Primitive::Ellipsoid { mut axes } => {
            axes[axis] = snap::ratio(axes[axis] * factor, fine);
            Primitive::Ellipsoid { axes }
        }
        Primitive::Slab { mut edges, corner } => {
            edges[axis] = snap::ratio(edges[axis] * factor, fine);
            Primitive::Slab { edges, corner }
        }
        Primitive::Capsule { length } => Primitive::Capsule { length: along(length) },
        Primitive::Cylinder { length } => Primitive::Cylinder { length: along(length) },
        Primitive::Frustum { length, taper } => Primitive::Frustum { length: along(length), taper },
        // The torus's axis is across its ring, so stretching along it fattens the tube.
        Primitive::Torus { major } => {
            let f = if axis == 0 { 1.0 / factor } else { factor };
            Primitive::Torus { major: snap::ratio(major * f, fine).max(1.0) }
        }
    }
}

/// Like [`stretched`], but keeping the other dimensions, so the volume changes.
pub fn grown(part: &Part, axis: usize, factor: f64, fine: bool) -> Part {
    let primitive = stretched(part.primitive, axis, factor, fine);
    let scale = part.primitive.scale(part.volume_m3);
    // Scale is the first semi-axis or edge, the radius, or a torus's minor radius.
    let kept = match (part.primitive, primitive) {
        (Primitive::Ellipsoid { axes: a }, Primitive::Ellipsoid { axes: b }) => scale * b.x / a.x,
        (Primitive::Slab { edges: a, .. }, Primitive::Slab { edges: b, .. }) => scale * b.x / a.x,
        (
            Primitive::Capsule { length: a } | Primitive::Cylinder { length: a } | Primitive::Frustum { length: a, .. },
            Primitive::Capsule { length: b } | Primitive::Cylinder { length: b } | Primitive::Frustum { length: b, .. },
        ) if axis != 0 && b > 0.0 => scale * a / b,
        (Primitive::Torus { major: a }, Primitive::Torus { major: b }) if axis == 0 => scale * a / b,
        _ => scale,
    };
    Part { primitive, volume_m3: primitive.volume(kept), ..*part }
}

/// So a capsule of length zero can still be stretched.
const MIN_RATIO: f64 = 0.1;

/// Three significant digits, for reading.
pub fn figure(x: f64) -> String {
    let trimmed = |s: String| s.trim_end_matches('0').trim_end_matches('.').to_string();
    if x == 0.0 || !x.is_finite() {
        return format!("{x}");
    }
    let magnitude = x.abs().log10().floor();
    if (-3.0..5.0).contains(&magnitude) {
        trimmed(format!("{x:.*}", (2.0 - magnitude).max(0.0) as usize))
    } else {
        let written = format!("{x:.2e}");
        let (mantissa, exponent) = written.split_once('e').unwrap_or((&written, "0"));
        format!("{}e{exponent}", trimmed(mantissa.to_string()))
    }
}

/// For a field: short, and exact enough to round-trip what a handle set.
pub fn shown(x: f64) -> String {
    let trimmed = |s: String| s.trim_end_matches('0').trim_end_matches('.').to_string();
    if x != 0.0 && (x.abs() >= 1.0e5 || x.abs() < 1.0e-3) {
        let written = format!("{x:.4e}");
        let (mantissa, exponent) = written.split_once('e').unwrap_or((&written, "0"));
        format!("{}e{exponent}", trimmed(mantissa.to_string()))
    } else {
        trimmed(format!("{x:.4}"))
    }
}

pub fn value(part: &Part, field: Field) -> Option<f64> {
    let placement = part.placement;
    let attached = placement.and_then(|p| match p.mount {
        Mount::Attached { anchor, standoff } => Some((anchor, standoff)),
        Mount::Enclosing => None,
    });
    match (field, part.primitive) {
        (Field::Volume, _) => Some(part.volume_m3),
        (Field::Axis(i), Primitive::Ellipsoid { axes }) => Some(axes[i]),
        (Field::Axis(i), Primitive::Slab { edges, .. }) => Some(edges[i]),
        (Field::Length, Primitive::Capsule { length } | Primitive::Cylinder { length } | Primitive::Frustum { length, .. }) => {
            Some(length)
        }
        (Field::Corner, Primitive::Slab { corner, .. }) => Some(corner),
        (Field::Major, Primitive::Torus { major }) => Some(major),
        (Field::Taper, Primitive::Frustum { taper, .. }) => Some(taper),
        (Field::Anchor(i), _) => attached.map(|(anchor, _)| anchor[i]),
        (Field::Standoff, _) => attached.map(|(_, standoff)| standoff),
        (Field::Twist, _) => placement.map(|p| p.twist.to_degrees()),
        (Field::Tilt(i), _) => placement.map(|p| p.tilt[i].to_degrees()),
        (Field::Blend, _) => placement.map(|p| p.blend),
        (Field::Parent, _) => placement.map(|p| f64::from(p.parent.0)),
        _ => None,
    }
}

/// A label and the fields on its line.
pub fn fields(part: &Part) -> Vec<(&'static str, Vec<Field>)> {
    let three = |make: fn(usize) -> Field| vec![make(0), make(1), make(2)];
    let mut out = vec![("volume m3", vec![Field::Volume])];
    out.extend(match part.primitive {
        Primitive::Ellipsoid { .. } => vec![("axes", three(Field::Axis))],
        Primitive::Slab { .. } => vec![("edges", three(Field::Axis)), ("corner", vec![Field::Corner])],
        Primitive::Capsule { .. } | Primitive::Cylinder { .. } => vec![("length", vec![Field::Length])],
        Primitive::Frustum { .. } => vec![("length", vec![Field::Length]), ("taper", vec![Field::Taper])],
        Primitive::Torus { .. } => vec![("major", vec![Field::Major])],
    });
    let Some(placement) = part.placement else { return out };
    out.push(("parent", vec![Field::Parent]));
    if matches!(placement.mount, Mount::Attached { .. }) {
        out.push(("anchor", three(Field::Anchor)));
        out.push(("standoff", vec![Field::Standoff]));
    }
    out.push(("twist deg", vec![Field::Twist]));
    out.push(("tilt deg", vec![Field::Tilt(0), Field::Tilt(1)]));
    out.push(("blend", vec![Field::Blend]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: Balance = Balance::DEFAULT;

    fn draft() -> Draft {
        Draft::new(Form::starting())
    }

    fn ids(parts: &[Part]) -> Vec<u16> {
        parts.iter().map(|p| p.id.0).collect()
    }

    #[test]
    fn the_draft_starts_as_the_ship_with_nothing_marked() {
        let d = draft();
        assert_eq!(d.form, d.ship);
        assert!(d.marks(&B).is_empty());
        assert_eq!(d.tree()[0].1.kind, Kind::Mind);
        assert_eq!(d.tree().len(), d.form.parts.len());
        assert_eq!(d.tree()[1], (1, d.part(PartId(1)).unwrap()), "the storage under the Mind");
    }

    #[test]
    fn the_mind_cannot_be_changed() {
        let d = draft();
        assert_eq!(d.resize(PartId(0), 5e3), Err(Refused::Mind));
        assert_eq!(d.remove(PartId(0)), Err(Refused::Mind));
        assert_eq!(d.twist(PartId(0), 1.0), Err(Refused::Mind));
        assert_eq!(d.add(PartId(0), Kind::Mind, PRIMITIVES[0], DVec3::X, &B), Err(Refused::Mind));
    }

    #[test]
    fn a_removal_carries_the_part_and_its_subtree_and_undoes_exactly() {
        let mut d = draft();
        let edit = d.remove(PartId(1)).unwrap();
        assert_eq!(ids(&edit.before), vec![1, 2, 3, 4, 5], "everything but the Mind hangs from the storage");
        let edit = d.remove(PartId(2)).unwrap();
        assert_eq!(ids(&edit.before), vec![2]);
        d.apply(&edit, &B).unwrap();
        assert!(d.part(PartId(2)).is_none());
        d.apply(&edit.inverse(), &B).unwrap();
        assert_eq!(d.form, d.ship);
    }

    /// A draft may have no drones on the way to a design; Apply is what refuses one.
    #[test]
    fn the_last_drones_can_be_deleted_from_a_draft() {
        let mut d = draft();
        d.apply(&d.remove(PartId(3)).unwrap(), &B).unwrap();
        assert!(d.form.parts.iter().all(|p| p.kind != Kind::Drone));
        assert_eq!(rules::sizes(&d.form, &B), vec![FormError::TooFewDrones], "which Apply will refuse");
        let tiny = d.add(PartId(1), Kind::Drone, PRIMITIVES[1], DVec3::Y, &B).unwrap();
        d.apply(&tiny, &B).unwrap();
    }

    #[test]
    fn an_edit_that_makes_a_part_too_small_is_refused_and_changes_nothing() {
        let mut d = draft();
        let edit = d.resize(PartId(5), 10.0).unwrap();
        assert_eq!(d.apply(&edit, &B), Err(Refused::Fault(FormError::TooSmall(PartId(5)))));
        assert_eq!(d.form, d.ship);
    }

    #[test]
    fn an_enclosing_part_keeps_its_last_anchor_and_standoff() {
        let mut d = draft();
        let was = d.part(PartId(4)).unwrap().placement.unwrap().mount;
        d.apply(&d.toggle_mount(PartId(4)).unwrap(), &B).unwrap();
        assert_eq!(d.part(PartId(4)).unwrap().placement.unwrap().mount, Mount::Enclosing);
        d.apply(&d.toggle_mount(PartId(4)).unwrap(), &B).unwrap();
        assert_eq!(d.part(PartId(4)).unwrap().placement.unwrap().mount, was);
        assert_eq!(d.form, d.ship);
    }

    #[test]
    fn each_change_is_marked_as_the_round_would_make_it() {
        let mut d = draft();
        let grow = d.resize(PartId(1), 3.0e6).unwrap();
        d.apply(&grow, &B).unwrap();
        d.apply(&d.twist(PartId(4), 0.5).unwrap(), &B).unwrap();
        d.apply(&d.reshape(PartId(5), PRIMITIVES[0]).unwrap(), &B).unwrap();
        d.apply(&d.remove(PartId(2)).unwrap(), &B).unwrap();
        d.apply(&d.add(PartId(1), Kind::Living, PRIMITIVES[2], DVec3::Y, &B).unwrap(), &B).unwrap();
        d.apply(&d.resize(PartId(3), 5.0e5).unwrap(), &B).unwrap();
        let marks = d.marks(&B);
        assert_eq!(marks[&PartId(3)], Mark::Shrink, "smaller, and still there");
        assert_eq!(marks[&PartId(1)], Mark::Build);
        assert_eq!(marks[&PartId(4)], Mark::Move);
        assert_eq!(marks[&PartId(5)], Mark::Rebuild);
        assert_eq!(marks[&PartId(2)], Mark::Dismantle, "gone from the draft");
        assert_eq!(marks[&PartId(6)], Mark::Build);
    }

    #[test]
    fn the_staged_draft_has_every_mark() {
        let mut d = draft();
        let staged = staged("edits", &d.ship, &B).unwrap();
        d.apply(&d.replace(staged), &B).unwrap();
        let marks: BTreeSet<&str> = d.marks(&B).values().map(|m| m.word()).collect();
        assert_eq!(marks, ["build", "dismantle", "shrink", "move", "rebuild"].into_iter().collect());
    }

    #[test]
    fn a_stretch_keeps_the_volume_and_snaps_the_ratio() {
        let Primitive::Ellipsoid { axes } = stretched(Primitive::Ellipsoid { axes: DVec3::new(5.0, 3.0, 1.0) }, 1, 1.3, false) else {
            panic!()
        };
        assert_eq!(axes, DVec3::new(5.0, 4.0, 1.0));
        assert_eq!(stretched(Primitive::Capsule { length: 2.0 }, 2, 2.0, false), Primitive::Capsule { length: 1.0 });
        assert_eq!(stretched(Primitive::Torus { major: 3.0 }, 0, 4.0, false), Primitive::Torus { major: 1.0 });
    }

    #[test]
    fn a_free_stretch_keeps_the_other_dimensions() {
        use lc_world::form::primitive::Shape;
        let min = B.min_part_m3;
        let part = |primitive| Part { id: PartId(9), kind: Kind::Storage, primitive, volume_m3: 1.0e6, placement: None };
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9 * b.abs().max(1.0);

        let egg = part(Primitive::Ellipsoid { axes: DVec3::new(2.0, 1.0, 1.0) });
        let Shape::Ellipsoid { semi_axes: a } = egg.shape(min) else { panic!() };
        let wider = grown(&egg, 1, 2.0, false);
        let Shape::Ellipsoid { semi_axes: b } = wider.shape(min) else { panic!() };
        assert!(close(b.x, a.x) && close(b.y, 2.0 * a.y) && close(b.z, a.z), "{a} to {b}");
        assert!(close(wider.volume_m3, 2.0 * egg.volume_m3));
        let Shape::Ellipsoid { semi_axes: c } = grown(&egg, 0, 2.0, false).shape(min) else { panic!() };
        assert!(close(c.x, 2.0 * a.x) && close(c.y, a.y), "{a} to {c}");

        let pod = part(Primitive::Capsule { length: 2.0 });
        let Shape::Capsule { radius, length } = pod.shape(min) else { panic!() };
        let Shape::Capsule { radius: r, length: l } = grown(&pod, 0, 2.0, false).shape(min) else { panic!() };
        assert!(close(r, radius) && close(l, 2.0 * length), "along: the radius is kept");
        let Shape::Capsule { radius: r, length: l } = grown(&pod, 2, 2.0, false).shape(min) else { panic!() };
        assert!(close(r, 2.0 * radius) && close(l, length), "across: the length is kept");

        let ring = part(Primitive::Torus { major: 2.0 });
        let Shape::Torus { major, minor } = ring.shape(min) else { panic!() };
        let Shape::Torus { major: m, minor: n } = grown(&ring, 1, 2.0, false).shape(min) else { panic!() };
        assert!(close(n, minor) && close(m, 2.0 * major), "across: a wider ring of the same tube");
        let Shape::Torus { major: m, minor: n } = grown(&ring, 0, 1.25, false).shape(min) else { panic!() };
        assert!(close(m, major) && n > minor, "along: a fatter tube on the same ring");
    }

    /// The starting pod's standoff is positive and its tilt keeps it touching.
    #[test]
    fn a_part_cannot_be_stood_off_its_parent() {
        let d = draft();
        assert_eq!(d.standoff(PartId(2), 0.3, &B), Err(Refused::Fault(FormError::Detached(PartId(2)))));
        assert!(d.standoff(PartId(2), 0.0, &B).is_ok() && d.standoff(PartId(2), -0.7, &B).is_ok());
        let Some(Placement { mount: Mount::Attached { standoff, .. }, .. }) = d.part(PartId(3)).unwrap().placement else { panic!() };
        assert!(standoff > 0.3, "the pod's own standoff is above zero");
        assert!(d.standoff(PartId(3), standoff, &B).is_ok());
        assert!(d.standoff(PartId(3), standoff + 1.0, &B).is_err());
    }

    #[test]
    fn a_typed_field_is_taken_exactly() {
        let d = draft();
        let edit = d.set_field(PartId(3), Field::Twist, 12.5).unwrap();
        assert!((edit.after[0].placement.unwrap().twist - 12.5f64.to_radians()).abs() < 1e-15);
        let edit = d.set_field(PartId(3), Field::Volume, 123_456.0).unwrap();
        assert_eq!(edit.after[0].volume_m3, 123_456.0);
        assert!(d.set_field(PartId(1), Field::Anchor(0), 1.0).is_err(), "an enclosing part has no anchor");
        assert!(d.set_field(PartId(1), Field::Length, 1.0).is_err(), "an ellipsoid has no length");
        for field in fields(d.part(PartId(2)).unwrap()).into_iter().flat_map(|(_, line)| line) {
            let v = value(d.part(PartId(2)).unwrap(), field).unwrap();
            let edit = d.set_field(PartId(2), field, v).unwrap();
            assert_eq!(edit.after[0].id, PartId(2), "{field:?}");
        }
    }

    #[test]
    fn a_part_is_described_by_what_it_does_for_its_kind() {
        let d = draft();
        let stats = |id| d.stats(PartId(id), &B).into_iter().collect::<BTreeMap<_, _>>();
        let storage = stats(1);
        assert_eq!(storage["capacity"], "30 ME", "{storage:?}");
        let (dry, wet): (f64, f64) = (storage["dry mass"].trim_end_matches(" kg").parse().unwrap(), storage["wet mass"].trim_end_matches(" kg").parse().unwrap());
        assert!(wet > 2.0 * dry, "full, the store weighs more than its structure");
        let engine = stats(2);
        let g: f64 = engine["ship, full"].trim_end_matches(" g").parse().unwrap();
        assert!((4.5..5.5).contains(&g), "29's starting ship does 5 g full: {g}");
        assert!(!engine.contains_key("wet mass"));
        assert!(stats(3).contains_key("building") && stats(4).contains_key("drain") && stats(5).contains_key("capacity"));
        assert_eq!(stats(0).keys().copied().collect::<Vec<_>>(), vec!["dry mass"], "the Mind only weighs");
    }

    #[test]
    fn numbers_read_short() {
        assert_eq!(shown(2_356_194.49), "2.3562e6");
        assert_eq!(shown(1.25e6), "1.25e6");
        assert_eq!(figure(1.0744e20), "1.07e20");
        assert_eq!(figure(4.9712), "4.97");
        assert_eq!(figure(30.0), "30");
        assert_eq!(shown(3.0e-5), "3e-5");
        assert_eq!(shown(0.25), "0.25");
    }
}
