//! The draft: the form being edited, beside the ship's own, and every edit to it as a value.
//!
//! An [`Edit`] is the parts it touches before and after, by id, so writing its `after` back makes
//! the edit and writing its `before` back undoes it: a removal keeps the part, its id and its
//! subtree. The handles and fields build edits here and send them as
//! [`Action::EditForm`](crate::action::Action::EditForm); nothing else writes the draft. See
//! `lightcone/docs/29-ship-form.md` §The editor.

use std::collections::{BTreeMap, BTreeSet};

use bevy::color::Color;
use glam::{DVec2, DVec3};
use lc_world::fitting::{Balance, C2, ONBOARD_DATA_BYTES};
use lc_world::flight::{C_M_S, G0};
use lc_world::form::capacity::{Capacities, dry_mass_kg};
use lc_world::form::{Form, FormError, Kind, Mount, Part, PartId, Placement, Primitive, SparMode, rules};
use lc_world::refit::rounds::{Change, changes};

use crate::snap;

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    /// In id order.
    pub form: Form,
    /// The ship as it is, which the draft is marked against.
    pub ship: Form,
    /// An enclosing part's last anchor and standoff, so switching it back to attached restores
    /// them. Editor state: the form has nowhere to keep them.
    remembered: BTreeMap<PartId, (DVec3, f64)>,
}

/// What an edit did, in the terms the history lists it by.
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
    /// The whole draft, as a reset to the ship does.
    Whole,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub what: What,
    pub part: PartId,
    /// The parts it touches as they were, and as they will be. An id in one and not the other is
    /// a part removed or added.
    pub before: Vec<Part>,
    pub after: Vec<Part>,
    /// False while a handle is still held. Each frame of a drag carries the part as it was when
    /// the drag began, so the settled one is the whole gesture.
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
    /// The Mind cannot be deleted, resized, reshaped or moved.
    Mind,
    /// Deleting it would leave fewer drones than a ship may keep, and it could never refit again.
    LastDrones,
    /// The draft would stop being a form, or a part would be too small.
    Fault(FormError),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSuchPart(id) => write!(f, "there is no {id}"),
            Self::Mind => write!(f, "the Mind cannot be changed"),
            Self::LastDrones => write!(f, "those are the last drones"),
            Self::Fault(e) => e.fmt(f),
        }
    }
}

/// How a part of the draft differs from the ship, as the round would do it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Build,
    Dismantle,
    Move,
    /// Taken apart and built again: a reshape or a change of kind.
    Rebuild,
}

impl Mark {
    pub fn word(self) -> &'static str {
        match self {
            Mark::Build => "build",
            Mark::Dismantle => "dismantle",
            Mark::Move => "move",
            Mark::Rebuild => "rebuild",
        }
    }

    /// `18-ui-style.md` §Refit marks: four hues at the phosphor green's lightness.
    pub fn color(self) -> Color {
        match self {
            Mark::Build => em_ui::vfd::TEXT,
            Mark::Dismantle => Color::srgb(1.0, 0.70, 0.78),
            Mark::Move => Color::srgb(0.45, 0.83, 1.0),
            Mark::Rebuild => Color::srgb(0.85, 0.76, 1.0),
        }
    }
}

/// Every number the fields panel shows, and a handle's field.
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
    /// Degrees in the field.
    Twist,
    /// Degrees in the field, along the twisted y and z.
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

/// The kinds a part may be added as. The Mind is not one.
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

/// The next of `list` after `current`, by what sort of thing it is, round again at the end.
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

    /// The part and everything hanging from it, the part first.
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

    /// The Mind first, then each part under its parent, children in id order, with its depth.
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

    /// Writes `edit.after` over the parts `edit` names. Refused, leaving the draft as it was,
    /// when the result is no form or makes a part too small; a fault the draft already had
    /// does not refuse an edit that leaves it.
    pub fn apply(&mut self, edit: &Edit, balance: &Balance) -> Result<(), Refused> {
        let named: BTreeSet<PartId> = edit.before.iter().chain(&edit.after).map(|p| p.id).collect();
        let mut next = self.form.clone();
        next.parts.retain(|p| !named.contains(&p.id));
        next.parts.extend(edit.after.iter().copied());
        next.parts.sort_by_key(|p| p.id);
        next.validate().map_err(Refused::Fault)?;
        let had = rules::sizes(&self.form, balance);
        if let Some(&fault) = rules::sizes(&next, balance).iter().find(|f| !had.contains(f)) {
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

    /// A spar cut the other way: a reshape, since the cut changes and the charged volume does not.
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

    /// Attached or enclosing. Back to attached takes the anchor and standoff it last had.
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

    /// Where an attached part hangs, as a direction in its parent's frame. An enclosing part has
    /// no anchor, and this leaves it as it is.
    pub fn anchor(&self, id: PartId, anchor: DVec3) -> Result<Edit, Refused> {
        let placement = self.placement(id)?;
        let mount = match placement.mount {
            Mount::Attached { standoff, .. } => Mount::Attached { anchor, standoff },
            Mount::Enclosing => Mount::Enclosing,
        };
        self.place(id, Placement { mount, ..placement })
    }

    pub fn standoff(&self, id: PartId, standoff: f64) -> Result<Edit, Refused> {
        let placement = self.placement(id)?;
        let mount = match placement.mount {
            Mount::Attached { anchor, .. } => Mount::Attached { anchor, standoff },
            Mount::Enclosing => Mount::Enclosing,
        };
        self.place(id, Placement { mount, ..placement })
    }

    pub fn twist(&self, id: PartId, twist: f64) -> Result<Edit, Refused> {
        self.place(id, Placement { twist, ..self.placement(id)? })
    }

    /// A new part of `kind`, attached to `parent` along `anchor`, a tenth of the parent's size.
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

    /// The part and its subtree.
    pub fn remove(&self, id: PartId, balance: &Balance) -> Result<Edit, Refused> {
        self.editable(id)?;
        let before = self.subtree(id);
        let gone: BTreeSet<PartId> = before.iter().map(|p| p.id).collect();
        let copies = self.form.copies();
        let drones_left: f64 = self
            .form
            .parts
            .iter()
            .filter(|p| p.kind == Kind::Drone && !gone.contains(&p.id))
            .map(|p| p.volume_m3 * f64::from(copies[&p.id]))
            .sum();
        let had_drones = before.iter().any(|p| p.kind == Kind::Drone);
        if had_drones && drones_left < balance.min_drone_m3 {
            return Err(Refused::LastDrones);
        }
        Ok(Edit { what: What::Remove, part: id, before, after: Vec::new(), settled: true })
    }

    /// Back to the ship as it is.
    pub fn reset(&self) -> Edit {
        let mind = self.form.parts.iter().find(|p| p.kind == Kind::Mind).map_or(PartId(0), |p| p.id);
        Edit { what: What::Whole, part: mind, before: self.form.parts.clone(), after: self.ship.parts.clone(), settled: true }
    }

    /// A typed number, exactly: fields do not snap. Angles arrive in degrees.
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
            (Field::Standoff, _) => self.standoff(id, value),
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

    /// Each changed part's mark, by F8's own diff. Parts the draft removed are marked too; they
    /// are in the ship, not the draft.
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
                } else if has(Change::Remove) || has(Change::Shrink) {
                    Mark::Dismantle
                } else {
                    Mark::Move
                };
                (id, mark)
            })
            .collect()
    }

    /// What a part does, as its handle says it: capacity by kind and, for an engine, what it
    /// pushes this ship at, full.
    pub fn role(&self, id: PartId, balance: &Balance) -> String {
        let Some(part) = self.part(id) else { return String::new() };
        let copies = f64::from(self.form.copies().get(&id).copied().unwrap_or(1));
        let alone = Form { parts: vec![Part { volume_m3: part.volume_m3 * copies, placement: None, ..*part }] };
        let held = Capacities::of(&alone, balance);
        match part.kind {
            Kind::Mind => "the Mind".into(),
            Kind::Storage => format!("storage, {} ME", sci(held.storage_j / balance.module_energy_j())),
            Kind::Drone => format!("drones, {} W of building", sci(held.building_w)),
            Kind::Engine => {
                let full = Capacities::of(&self.form, balance).storage_j / C2;
                let mass_kg = dry_mass_kg(&self.form, balance) + full;
                let g = held.aperture_w / C_M_S / mass_kg / G0;
                format!("drive section, {} W, {g:.1} g full on this ship", sci(held.aperture_w))
            }
            Kind::Living => format!("living, {} W of drain", sci(held.drain_w)),
            Kind::Data => format!("data, {}B", si(held.data_b - ONBOARD_DATA_BYTES)),
            Kind::Bay => {
                let mouth = part.shape(balance.min_part_m3).extent(glam::DMat3::IDENTITY, 0.0);
                format!("bay, a mouth {} m across", sci(2.0 * mouth.y.min(mouth.z)))
            }
            Kind::Spar(SparMode::Saddle) => "spar, cut to what it joins".into(),
            Kind::Spar(SparMode::Strap) => "spar, strapped to its parent".into(),
        }
    }
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

/// `primitive` stretched along its own `axis` by `factor` at fixed volume, the stretched ratio
/// snapped. Volume is stored and scale solved, so changing a ratio cannot change the volume.
pub fn stretched(primitive: Primitive, axis: usize, factor: f64, fine: bool) -> Primitive {
    // Along the axis a length grows; across it the radius does, which is the length shrinking.
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

/// So a sphere of a capsule, whose length is zero, can still be stretched into one.
const MIN_RATIO: f64 = 0.1;

/// `1.1 × 10^20`. Quantico has no superscript digits.
pub fn sci(x: f64) -> String {
    if x == 0.0 || !x.is_finite() {
        return format!("{x}");
    }
    let exponent = x.abs().log10().floor();
    if (-2.0..4.0).contains(&exponent) {
        return format!("{:.3}", x).trim_end_matches('0').trim_end_matches('.').to_string();
    }
    let mantissa = x / 10f64.powf(exponent);
    // Rounding can carry the mantissa to ten.
    let (mantissa, exponent) = if (mantissa.abs() * 10.0).round() >= 100.0 { (mantissa / 10.0, exponent + 1.0) } else { (mantissa, exponent) };
    format!("{mantissa:.1} × 10^{exponent}")
}

/// `2.9 M`, for bytes.
fn si(x: f64) -> String {
    const PREFIXES: [(f64, &str); 5] = [(1e12, "T"), (1e9, "G"), (1e6, "M"), (1e3, "k"), (1.0, "")];
    let (scale, prefix) = PREFIXES.into_iter().find(|(s, _)| x.abs() >= *s).unwrap_or((1.0, ""));
    format!("{:.1} {prefix}", x / scale)
}

/// A field's number as typed back to it: short, and exact enough to round-trip what a handle set.
pub fn shown(x: f64) -> String {
    if x != 0.0 && (x.abs() >= 1.0e5 || x.abs() < 1.0e-3) {
        format!("{x:.4e}")
    } else {
        format!("{:.4}", x).trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// The value `field` shows for `part`, if the part has one.
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

/// The fields `part` has, in the order the panel lists them.
pub fn fields(part: &Part) -> Vec<(Field, &'static str)> {
    let mut out = vec![(Field::Volume, "volume m3")];
    out.extend(match part.primitive {
        Primitive::Ellipsoid { .. } => vec![(Field::Axis(0), "axis x"), (Field::Axis(1), "axis y"), (Field::Axis(2), "axis z")],
        Primitive::Slab { .. } => {
            vec![(Field::Axis(0), "edge x"), (Field::Axis(1), "edge y"), (Field::Axis(2), "edge z"), (Field::Corner, "corner")]
        }
        Primitive::Capsule { .. } | Primitive::Cylinder { .. } => vec![(Field::Length, "length")],
        Primitive::Frustum { .. } => vec![(Field::Length, "length"), (Field::Taper, "taper")],
        Primitive::Torus { .. } => vec![(Field::Major, "major")],
    });
    let Some(placement) = part.placement else { return out };
    out.push((Field::Parent, "parent"));
    if matches!(placement.mount, Mount::Attached { .. }) {
        out.extend([(Field::Anchor(0), "anchor x"), (Field::Anchor(1), "anchor y"), (Field::Anchor(2), "anchor z")]);
        out.push((Field::Standoff, "standoff"));
    }
    out.extend([(Field::Twist, "twist deg"), (Field::Tilt(0), "tilt y deg"), (Field::Tilt(1), "tilt z deg"), (Field::Blend, "blend")]);
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
        assert_eq!(d.remove(PartId(0), &B), Err(Refused::Mind));
        assert_eq!(d.twist(PartId(0), 1.0), Err(Refused::Mind));
        assert_eq!(d.add(PartId(0), Kind::Mind, PRIMITIVES[0], DVec3::X, &B), Err(Refused::Mind));
    }

    /// A removal keeps the whole subtree, so writing its `before` back is exact.
    #[test]
    fn a_removal_carries_the_part_and_its_subtree_and_undoes_exactly() {
        let mut d = draft();
        let edit = d.remove(PartId(1), &B);
        assert_eq!(edit, Err(Refused::LastDrones), "the drones hang from the storage");
        let edit = d.remove(PartId(2), &B).unwrap();
        assert_eq!(ids(&edit.before), vec![2]);
        d.apply(&edit, &B).unwrap();
        assert!(d.part(PartId(2)).is_none());
        d.apply(&edit.inverse(), &B).unwrap();
        assert_eq!(d.form, d.ship);
    }

    #[test]
    fn the_last_drones_cannot_be_deleted_but_one_of_two_can() {
        let mut d = draft();
        assert_eq!(d.remove(PartId(3), &B), Err(Refused::LastDrones));
        let second = d.add(PartId(1), Kind::Drone, PRIMITIVES[1], DVec3::Y, &B).unwrap();
        d.apply(&second, &B).unwrap();
        assert!(d.remove(PartId(3), &B).is_ok());
    }

    #[test]
    fn an_edit_that_makes_a_part_too_small_is_refused_and_changes_nothing() {
        let mut d = draft();
        let edit = d.resize(PartId(5), 10.0).unwrap();
        assert_eq!(d.apply(&edit, &B), Err(Refused::Fault(FormError::TooSmall(PartId(5)))));
        assert_eq!(d.form, d.ship);
    }

    /// 29: switching an enclosing part back to attached restores its anchor and standoff.
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

    /// Marks come from the planner's own diff.
    #[test]
    fn each_change_is_marked_as_the_round_would_make_it() {
        let mut d = draft();
        let grow = d.resize(PartId(1), 3.0e6).unwrap();
        d.apply(&grow, &B).unwrap();
        d.apply(&d.twist(PartId(4), 0.5).unwrap(), &B).unwrap();
        d.apply(&d.reshape(PartId(5), PRIMITIVES[0]).unwrap(), &B).unwrap();
        d.apply(&d.remove(PartId(2), &B).unwrap(), &B).unwrap();
        d.apply(&d.add(PartId(1), Kind::Living, PRIMITIVES[2], DVec3::Y, &B).unwrap(), &B).unwrap();
        let marks = d.marks(&B);
        assert_eq!(marks[&PartId(1)], Mark::Build);
        assert_eq!(marks[&PartId(4)], Mark::Move);
        assert_eq!(marks[&PartId(5)], Mark::Rebuild);
        assert_eq!(marks[&PartId(2)], Mark::Dismantle);
        assert_eq!(marks[&PartId(6)], Mark::Build);
        assert!(!marks.contains_key(&PartId(3)));
    }

    #[test]
    fn a_stretch_keeps_the_volume_and_snaps_the_ratio() {
        let Primitive::Ellipsoid { axes } = stretched(Primitive::Ellipsoid { axes: DVec3::new(5.0, 3.0, 1.0) }, 1, 1.3, false) else {
            panic!()
        };
        assert_eq!(axes, DVec3::new(5.0, 4.0, 1.0));
        // Across a capsule's axis it is the radius that grows, which is the length shrinking.
        assert_eq!(stretched(Primitive::Capsule { length: 2.0 }, 2, 2.0, false), Primitive::Capsule { length: 1.0 });
        assert_eq!(stretched(Primitive::Torus { major: 3.0 }, 0, 4.0, false), Primitive::Torus { major: 1.0 });
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
        for (field, _) in fields(d.part(PartId(2)).unwrap()) {
            let v = value(d.part(PartId(2)).unwrap(), field).unwrap();
            let edit = d.set_field(PartId(2), field, v).unwrap();
            assert_eq!(edit.after[0].id, PartId(2), "{field:?}");
        }
    }

    #[test]
    fn a_handle_says_what_the_part_does() {
        let d = draft();
        let engine = d.role(PartId(2), &B);
        assert!(engine.starts_with("drive section, 1.1 × 10^20 W"), "{engine}");
        assert!(engine.contains("4.9 g full") || engine.contains("5.0 g full"), "{engine}");
        assert!(d.role(PartId(1), &B).contains("30 ME"), "{}", d.role(PartId(1), &B));
    }

    #[test]
    fn numbers_read_short() {
        assert_eq!(sci(1.07e20), "1.1 × 10^20");
        assert_eq!(sci(9.96e19), "1.0 × 10^20");
        assert_eq!(sci(30.0), "30");
        assert_eq!(shown(2_356_194.49), "2.3562e6");
        assert_eq!(shown(0.25), "0.25");
    }
}
