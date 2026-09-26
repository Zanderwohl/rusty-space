//! The starting form, the built-in presets, and applying any preset to a ship.
//!
//! A preset is a [`Form`]. Applied as a design it is the target exactly. Applied as a layout it
//! keeps its arrangement and takes the ship's volumes, each part keeping its share of its kind's
//! total. See 29 §The starting form and §Your own presets.

use std::collections::{BTreeSet, HashMap};

use glam::{DVec2, DVec3};

use super::place::normal_frame;
use super::primitive::Shape;
use super::rules;
use super::{Form, FormError, Kind, Mount, Part, PartId, Placement, Primitive, SparMode};
use crate::fitting::Balance;

const MIND: PartId = PartId(0);

/// 19's slot, the reference 500 m ovoid's `π L³ / 50` over twenty: what one of its modules held,
/// and what the densities and a module-energy are still quoted per.
pub const SLOT_M3: f64 = 125_000.0 * std::f64::consts::PI;

/// 19's starting modules as volumes: each kind's count of slots.
pub(crate) struct Volumes {
    pub(crate) storage: f64,
    pub(crate) drone: f64,
    pub(crate) engine: f64,
    pub(crate) living: f64,
    pub(crate) data: f64,
}

pub(crate) const STARTING: Volumes =
    Volumes { storage: 6.0 * SLOT_M3, drone: 2.0 * SLOT_M3, engine: 5.0 * SLOT_M3, living: SLOT_M3, data: SLOT_M3 };

fn hang(id: u16, kind: Kind, primitive: Primitive, volume_m3: f64, parent: PartId, mount: Mount) -> Part {
    let placement = Placement { parent, mount, twist: 0.0, tilt: DVec2::ZERO, blend: 0.0, mirror: false };
    Part { id: PartId(id), kind, primitive, volume_m3, placement: Some(placement) }
}

fn on(anchor: DVec3, standoff: f64) -> Mount {
    Mount::Attached { anchor, standoff }
}

fn blended(mut part: Part, blend: f64) -> Part {
    part.placement.as_mut().expect("only the Mind is unplaced").blend = blend;
    part
}

/// The tilt that turns a child attached where its parent's normal is `normal` to lie along `along`,
/// both in the parent's frame. `libm`, so every machine building the form builds the same one.
/// `along` must not be parallel to `normal`, which leaves the turn's axis undefined.
fn lying(normal: DVec3, along: DVec3) -> DVec2 {
    debug_assert!(normal.cross(along).length_squared() > 1e-12, "no axis to turn about");
    let frame = normal_frame(normal);
    let cross = normal.cross(along);
    let turn = cross.normalize() * libm::atan2(cross.length(), normal.dot(along));
    DVec2::new(turn.dot(frame.y_axis), turn.dot(frame.z_axis))
}

/// Of the drone pod's radius, how far its top sinks into the hull above it.
const KEEL_EMBED: f64 = 0.35;

impl Form {
    /// 29's starting form, holding 19's starting modules. The frame is x the nose, z up.
    pub fn starting() -> Form {
        let b = Balance::DEFAULT;
        let v = STARTING;
        let hull = Primitive::Ellipsoid { axes: DVec3::new(5.0, 3.0, 1.0) };
        let pod = Primitive::Capsule { length: 2.0 };

        // The pod lies fore and aft under the keel, its aft end bolted on, so its center sits
        // below the Mind. Solved at these volumes; a layout that resizes it slides it a little.
        let Shape::Ellipsoid { semi_axes: a } = hull.at(hull.scale(v.storage)) else { unreachable!() };
        let Shape::Capsule { radius, length } = pod.at(pod.scale(v.drone)) else { unreachable!() };
        let reach = length / 2.0 + radius;
        let bolt = DVec3::new(reach, 0.0, -a.z * (1.0 - (reach / a.x).powi(2)).sqrt());
        let normal = hull.at(1.0).exit(bolt.normalize()).normal;
        let axis_z = -a.z - (1.0 - KEEL_EMBED) * radius;
        let mut drones = hang(3, Kind::Drone, pod, v.drone, PartId(1), on(bolt, (axis_z - bolt.z) / (normal.z * reach)));
        drones.placement.as_mut().unwrap().tilt = lying(normal, DVec3::NEG_X);

        // 31: an engine's aperture is its wide end, so the frustum flares aft.
        let bell = Primitive::Frustum { length: 2.0, taper: 1.5 };
        let deck = Primitive::Slab { edges: DVec3::new(1.2, 6.0, 4.0), corner: 0.25 };
        Form {
            parts: vec![
                Part::mind(MIND, b.min_part_m3),
                hang(1, Kind::Storage, hull, v.storage, MIND, Mount::Enclosing),
                hang(2, Kind::Engine, bell, v.engine, PartId(1), on(DVec3::NEG_X, -0.2)),
                drones,
                blended(hang(4, Kind::Living, deck, v.living, PartId(1), on(DVec3::Z, -0.6)), 0.2),
                hang(5, Kind::Data, Primitive::Capsule { length: 1.5 }, v.data, PartId(1), on(DVec3::X, -0.3)),
            ],
        }
    }
}

/// The layouts the editor offers, filled with the starting form's volumes so each is also a
/// design.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    /// One wide slab with everything on its faces: the largest shadow for its volume, and the
    /// slowest to turn.
    Plate,
    /// Capsules on one long axis: the smallest shadow nose-on, and the quickest to turn.
    Spindle,
    /// Separate bodies on spars about a core.
    Cluster,
}

/// The names [`named`] reads, as the console and `--form` spell them.
pub const NAMES: [&str; 4] = ["default", "plate", "spindle", "cluster"];

/// A form by name, any case: `default` for the starting form, or a builtin's. `scale` makes every
/// part but the Mind that many times longer at the same proportions.
pub fn named(name: &str, scale: f64) -> Option<Form> {
    let mut form = match name.eq_ignore_ascii_case("default") {
        true => Form::starting(),
        false => Builtin::ALL.into_iter().find(|b| b.name().eq_ignore_ascii_case(name)).map(Builtin::form)?,
    };
    for part in form.parts.iter_mut().filter(|p| p.placement.is_some()) {
        part.volume_m3 *= scale.powi(3);
    }
    Some(form)
}

impl Builtin {
    pub const ALL: [Builtin; 3] = [Builtin::Plate, Builtin::Spindle, Builtin::Cluster];

    pub fn name(self) -> &'static str {
        match self {
            Builtin::Plate => "Plate",
            Builtin::Spindle => "Spindle",
            Builtin::Cluster => "Cluster",
        }
    }

    pub fn form(self) -> Form {
        let b = Balance::DEFAULT;
        let v = STARTING;
        let mind = Part::mind(MIND, b.min_part_m3);
        let parts = match self {
            Builtin::Plate => {
                let plate = Primitive::Slab { edges: DVec3::new(9.0, 5.0, 0.5), corner: 0.3 };
                let tile = |edges: DVec3| Primitive::Slab { edges, corner: 0.2 };
                let bell = Primitive::Frustum { length: 3.0, taper: 1.2 };
                // Nearly edge-on, so the ray leaves through the top face well fore or aft.
                let fore = DVec3::new(1.0, 0.0, 0.12);
                let aft = DVec3::new(-1.0, 0.0, 0.12);
                vec![
                    mind,
                    hang(1, Kind::Storage, plate, v.storage, MIND, Mount::Enclosing),
                    hang(2, Kind::Engine, bell, v.engine / 2.0, PartId(1), on(DVec3::new(-1.0, 0.35, 0.0), -0.1)),
                    hang(3, Kind::Engine, bell, v.engine / 2.0, PartId(1), on(DVec3::new(-1.0, -0.35, 0.0), -0.1)),
                    hang(4, Kind::Drone, tile(DVec3::new(0.5, 4.0, 4.0)), v.drone, PartId(1), on(DVec3::NEG_Z, -0.5)),
                    hang(5, Kind::Living, tile(DVec3::new(0.5, 4.0, 3.0)), v.living, PartId(1), on(fore, -0.5)),
                    hang(6, Kind::Data, tile(DVec3::new(0.5, 4.0, 3.0)), v.data, PartId(1), on(aft, -0.5)),
                ]
            }
            Builtin::Spindle => {
                let capsule = Primitive::Capsule { length: 2.0 };
                vec![
                    mind,
                    hang(1, Kind::Storage, Primitive::Capsule { length: 4.0 }, v.storage, MIND, Mount::Enclosing),
                    hang(2, Kind::Engine, Primitive::Frustum { length: 4.0, taper: 1.1 }, v.engine, PartId(1), on(DVec3::NEG_X, -0.1)),
                    hang(3, Kind::Drone, capsule, v.drone, PartId(1), on(DVec3::X, -0.1)),
                    hang(4, Kind::Living, capsule, v.living, PartId(3), on(DVec3::X, -0.1)),
                    hang(5, Kind::Data, capsule, v.data, PartId(4), on(DVec3::X, -0.1)),
                ]
            }
            Builtin::Cluster => {
                // Embedded at both ends, so a saddle has something to be cut to.
                let spar = |id: u16, anchor: DVec3| {
                    let rod = Primitive::Cylinder { length: 10.0 };
                    hang(id, Kind::Spar(SparMode::Saddle), rod, CLUSTER_SPAR_M3, PartId(1), on(anchor, -0.3))
                };
                let pod = Primitive::Capsule { length: 1.0 };
                let end = on(DVec3::X, -0.1);
                vec![
                    mind,
                    hang(1, Kind::Storage, Primitive::Ellipsoid { axes: DVec3::new(1.5, 1.0, 1.0) }, 0.7 * v.storage, MIND, Mount::Enclosing),
                    spar(2, DVec3::NEG_X),
                    hang(3, Kind::Engine, Primitive::Frustum { length: 1.5, taper: 1.4 }, v.engine, PartId(2), end),
                    spar(4, DVec3::NEG_Z),
                    hang(5, Kind::Drone, pod, v.drone, PartId(4), end),
                    spar(6, DVec3::Z),
                    hang(7, Kind::Living, pod, v.living, PartId(6), end),
                    spar(8, DVec3::Y),
                    hang(9, Kind::Data, pod, v.data, PartId(8), end),
                    spar(10, DVec3::NEG_Y),
                    hang(11, Kind::Storage, Primitive::Ellipsoid { axes: DVec3::ONE }, 0.3 * v.storage, PartId(10), end),
                ]
            }
        };
        Form { parts }
    }
}

/// Each spoke of the cluster: about 100 m long and 20 m across.
const CLUSTER_SPAR_M3: f64 = 3.0e4;

/// The kinds whose totals a layout keeps. A spar holds nothing: it is the arrangement's own
/// structure, so it scales with the arrangement instead. The Mind is fixed.
const HELD: [Kind; 6] = [Kind::Storage, Kind::Drone, Kind::Engine, Kind::Living, Kind::Data, Kind::Bay];

fn totals(form: &Form) -> [f64; HELD.len()] {
    let mut sums = [0.0; HELD.len()];
    for part in &form.parts {
        if let Some(k) = HELD.iter().position(|&kind| kind == part.kind) {
            sums[k] += part.volume_m3;
        }
    }
    sums
}

/// A preset applied as a layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub form: Form,
    /// The preset's parts given nothing, by id: of a kind the ship lacks, or whose share would be
    /// below `min_part_m3`. Their children hang from the nearest ancestor that is kept.
    pub emptied: Vec<PartId>,
    /// Kinds the ship has that the preset has no part for, whose volume the layout leaves out. The
    /// editor names these as it names `emptied`.
    pub unplaced: Vec<Kind>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PresetError {
    Form(FormError),
    /// Below `min_part_m3`.
    TooSmall(PartId),
}

impl std::fmt::Display for PresetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Form(e) => e.fmt(f),
            Self::TooSmall(id) => write!(f, "{id} is smaller than the smallest part"),
        }
    }
}

impl std::error::Error for PresetError {}

impl From<FormError> for PresetError {
    fn from(e: FormError) -> Self {
        Self::Form(e)
    }
}

impl Form {
    /// The preset exactly, once it is known to be a form the ship could be.
    pub fn as_design(&self, min_part_m3: f64) -> Result<Form, PresetError> {
        self.validate()?;
        if let Some(part) = self.parts.iter().find(|p| rules::too_small(p, min_part_m3)) {
            return Err(PresetError::TooSmall(part.id));
        }
        Ok(self.clone())
    }

    /// This preset's arrangement holding `ship`'s volumes. Each kind's total is the ship's, split
    /// among the preset's parts of that kind in proportion to their volumes; spars grow by the
    /// ratio the held parts grew by.
    pub fn as_layout(&self, ship: &Form, min_part_m3: f64) -> Result<Layout, PresetError> {
        let preset = self.as_design(min_part_m3)?;
        let want = totals(ship);
        let mut volumes: HashMap<PartId, f64> = HashMap::new();
        let mut emptied = Vec::new();
        let mut unplaced = Vec::new();
        let (mut before, mut after) = (0.0, 0.0);
        for (kind, want) in HELD.into_iter().zip(want) {
            let mut kept: Vec<&Part> = preset.parts.iter().filter(|p| p.kind == kind).collect();
            kept.sort_by(|a, b| a.volume_m3.total_cmp(&b.volume_m3).then(a.id.cmp(&b.id)));
            // Smallest share first, so what is dropped is what matters least. Dropping it raises
            // every other share, so the check repeats.
            let sum = loop {
                let sum: f64 = kept.iter().map(|p| p.volume_m3).sum();
                match kept.first() {
                    Some(least) if want * least.volume_m3 / sum < min_part_m3 => {
                        emptied.push(least.id);
                        kept.remove(0);
                    }
                    _ => break sum,
                }
            };
            if want > 0.0 && kept.is_empty() {
                unplaced.push(kind);
            }
            for part in kept {
                let volume = want * part.volume_m3 / sum;
                volumes.insert(part.id, volume);
                before += part.volume_m3;
                after += volume;
            }
        }
        let grow = if before > 0.0 { after / before } else { 1.0 };

        let parent: HashMap<PartId, PartId> =
            preset.parts.iter().filter_map(|p| Some((p.id, p.placement?.parent))).collect();
        let gone: BTreeSet<PartId> = emptied.iter().copied().collect();
        let kept_ancestor = |mut id: PartId| {
            while gone.contains(&id) {
                id = parent[&id];
            }
            id
        };
        let parts = preset
            .parts
            .iter()
            .filter(|p| !gone.contains(&p.id))
            .map(|&part| {
                let volume_m3 = match part.kind {
                    Kind::Mind => part.volume_m3,
                    Kind::Spar(_) => (part.volume_m3 * grow).max(min_part_m3),
                    _ => volumes[&part.id],
                };
                let placement = part.placement.map(|p| Placement { parent: kept_ancestor(p.parent), ..p });
                Part { volume_m3, placement, ..part }
            })
            .collect();
        emptied.sort();
        let form = Form { parts };
        form.validate()?;
        Ok(Layout { form, emptied, unplaced })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitting::{ONBOARD_DATA_BYTES, STARTING_DRY_KG};
    use crate::flight::{C_M_S, G0};
    use crate::form::capacity::{areal_density_for, dry_mass_kg, Capacities};
    use crate::form::place::{Poses, Side};

    const MIN: f64 = Balance::DEFAULT.min_part_m3;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        ((a - b) / b).abs() < tolerance
    }

    fn part(form: &Form, id: u16) -> &Part {
        form.parts.iter().find(|p| p.id == PartId(id)).unwrap()
    }

    fn of_kind(form: &Form, kind: Kind) -> Vec<&Part> {
        form.parts.iter().filter(|p| p.kind == kind).collect()
    }

    fn everything() -> Vec<(&'static str, Form)> {
        let mut all = vec![("starting", Form::starting())];
        all.extend(Builtin::ALL.map(|b| (b.name(), b.form())));
        all
    }

    /// Every primitive used here is star-shaped about its center, so a point is inside when it is
    /// no farther out than the surface along its own ray.
    fn inside(shape: &Shape, local: DVec3) -> bool {
        let r = local.length();
        r == 0.0 || r <= shape.exit(local / r).point.length()
    }

    fn placed(form: &Form) -> Poses {
        form.place(MIN).unwrap()
    }

    #[test]
    fn the_starting_volumes_are_29s_figures() {
        let b = Balance::DEFAULT;
        let form = Form::starting();
        let volume = |kind| of_kind(&form, kind).iter().map(|p| p.volume_m3).sum::<f64>();
        for (kind, printed) in [
            (Kind::Storage, "2.36e6"),
            (Kind::Engine, "1.96e6"),
            (Kind::Drone, "7.85e5"),
            (Kind::Living, "3.93e5"),
            (Kind::Data, "3.93e5"),
        ] {
            assert_eq!(of_kind(&form, kind).len(), 1, "{kind:?} is one part");
            assert_eq!(format!("{:.2e}", volume(kind)), printed, "{kind:?}");
        }
        let c = Capacities::of(&form, &b);
        assert_eq!(format!("{:.0}", c.storage_j / b.module_energy_j()), "30", "30 ME");
    }

    #[test]
    fn the_starting_form_has_nineteens_capacities_and_pulls_five_g_full() {
        let b = Balance::DEFAULT;
        let c = Capacities::of(&Form::starting(), &b);
        // A slot of each is what one of 19's modules gave.
        assert!(close(c.storage_j, 6.0 * SLOT_M3 * b.storage_density * b.module_energy_j(), 1e-12));
        assert!(close(c.building_w, 2.0 * SLOT_M3 * b.drone_density_w, 1e-12));
        assert!(close(c.drain_w, SLOT_M3 * b.living_density_w, 1e-12));
        assert!(close(c.data_b, ONBOARD_DATA_BYTES + SLOT_M3 * b.data_density_b, 1e-12));
        assert!(c.data_b > ONBOARD_DATA_BYTES);
        let full_kg = dry_mass_kg(&Form::starting(), &b) + c.storage_j / (C_M_S * C_M_S);
        let g = c.aperture_w / C_M_S / full_kg / G0;
        assert!(close(g, 5.0, 1e-9), "{g}");
    }

    /// Anchored on 19's dry mass: every module, the data module at half, and the frame over all
    /// twenty slots, the five empty ones included. Storage empty.
    #[test]
    fn hull_areal_density_is_anchored_on_nineteens_dry_starting_ship() {
        let b = Balance::DEFAULT;
        let target = STARTING_DRY_KG;
        let solved = areal_density_for(&Form::starting(), &b, target).unwrap();
        assert!(close(b.hull_areal_density, solved, 1e-9), "DEFAULT has {}, the starting form solves to {solved}", b.hull_areal_density);
        assert!(close(dry_mass_kg(&Form::starting(), &b), target, 1e-9));
        assert_eq!(format!("{target:.2e}"), "2.65e9", "19's table");
    }

    #[test]
    fn the_starting_form_validates_and_places() {
        let form = Form::starting();
        assert_eq!(form.validate(), Ok(()));
        assert_eq!(form.as_design(MIN), Ok(form.clone()));
        assert_eq!(placed(&form).len(), form.parts.len());
    }

    /// R1 photographs this, so each part must be where 29 says it is.
    #[test]
    fn each_starting_part_is_on_its_side_of_the_ship() {
        let form = Form::starting();
        let poses = placed(&form);
        let pose = |id: u16| *poses.get(PartId(id), Side::Original).unwrap();
        let hull = part(&form, 1).shape(MIN);
        let Shape::Ellipsoid { semi_axes: a } = hull else { unreachable!() };
        let axial = |v: DVec3| v.y.abs() < 1e-9 * a.x && v.z.abs() < 1e-9 * a.x;

        assert_eq!(pose(0).position, DVec3::ZERO);
        assert!(inside(&hull, DVec3::splat(5.0)), "the storage encloses the Mind's corners");

        let engines = pose(2);
        assert!(engines.position.x < -a.x, "aft of the hull: {}", engines.position);
        assert!(axial(engines.position), "on the nose axis: {}", engines.position);
        assert!((engines.axis() - DVec3::NEG_X).length() < 1e-12, "pointing aft: {}", engines.axis());

        let drones = pose(3);
        let Shape::Capsule { radius, .. } = part(&form, 3).shape(MIN) else { unreachable!() };
        assert!(drones.position.z < -a.z, "under the keel: {}", drones.position);
        assert!(drones.position.x.abs() < 0.1 * a.x, "amidships: {}", drones.position);
        assert!(drones.position.y.abs() < 1e-9 * a.x);
        assert!(drones.axis().x.abs() > 1.0 - 1e-12, "lying fore and aft: {}", drones.axis());
        assert!(drones.position.z + radius > -a.z, "its top in the hull, so it touches");

        let living = pose(4);
        assert!(living.position.z > 0.0 && living.position.x.abs() < 1e-9 * a.x, "dorsal: {}", living.position);
        assert!((living.axis() - DVec3::Z).length() < 1e-12, "flat across the top");

        let data = pose(5);
        assert!(data.position.x > a.x * 0.8 && axial(data.position), "forward: {}", data.position);
    }

    #[test]
    fn every_built_in_validates_places_and_is_a_design() {
        for (name, form) in everything() {
            assert_eq!(form.as_design(MIN), Ok(form.clone()), "{name}");
            placed(&form);
            for kind in [Kind::Storage, Kind::Drone, Kind::Engine, Kind::Living, Kind::Data] {
                assert!(!of_kind(&form, kind).is_empty(), "{name} has {kind:?}");
            }
            assert!(of_kind(&form, Kind::Drone).iter().map(|p| p.volume_m3).sum::<f64>() >= Balance::DEFAULT.min_drone_m3);
        }
        assert!(Builtin::Cluster.form().parts.iter().any(|p| matches!(p.kind, Kind::Spar(_))));
    }

    #[test]
    fn every_built_in_passes_the_placement_rules() {
        for (name, form) in everything() {
            if let Err(faults) = rules::check(&form, &Balance::DEFAULT) {
                panic!("{name}: {faults:?}");
            }
        }
    }

    fn assert_keeps_totals(layout: &Layout, ship: &Form, what: &str) {
        let (got, want) = (totals(&layout.form), totals(ship));
        for (k, kind) in HELD.iter().enumerate() {
            if layout.unplaced.contains(kind) {
                continue;
            }
            let ok = if want[k] == 0.0 { got[k] == 0.0 } else { close(got[k], want[k], 1e-12) };
            assert!(ok, "{what}: {kind:?} {} against {}", got[k], want[k]);
        }
        for part in &layout.form.parts {
            assert!(part.kind == Kind::Mind || part.volume_m3 >= MIN, "{what}: {} at {}", part.id, part.volume_m3);
        }
        placed(&layout.form);
    }

    /// Something other than a preset's own volumes: the starting ship with far more engine than
    /// storage, eight times over.
    fn odd_ship() -> Form {
        let mut ship = Form::starting();
        for part in &mut ship.parts {
            part.volume_m3 *= if part.kind == Kind::Engine { 20.0 } else { 8.0 };
        }
        ship
    }

    #[test]
    fn a_layout_keeps_every_kinds_total_and_each_parts_share() {
        for ship in [Form::starting(), odd_ship()] {
            for (name, preset) in everything() {
                let layout = preset.as_layout(&ship, MIN).unwrap();
                assert_eq!((layout.emptied.as_slice(), layout.unplaced.as_slice()), (&[][..], &[][..]), "{name}");
                assert_keeps_totals(&layout, &ship, name);
                // Same arrangement: ids, kinds, shapes and placements are the preset's.
                assert_eq!(layout.form.parts.len(), preset.parts.len());
                for (got, was) in layout.form.parts.iter().zip(&preset.parts) {
                    assert_eq!(Part { volume_m3: was.volume_m3, ..*got }, *was, "{name}");
                }
            }
        }
        // The cluster's two storage parts keep 70 : 30 of whatever the ship holds.
        let layout = Builtin::Cluster.form().as_layout(&odd_ship(), MIN).unwrap();
        let (core, pod) = (part(&layout.form, 1).volume_m3, part(&layout.form, 11).volume_m3);
        assert!(close(core / (core + pod), 0.7, 1e-12));
    }

    #[test]
    fn spars_grow_with_the_arrangement_and_the_mind_does_not() {
        let preset = Builtin::Cluster.form();
        let mut ship = Form::starting();
        ship.parts.iter_mut().for_each(|p| p.volume_m3 *= 8.0);
        let layout = preset.as_layout(&ship, MIN).unwrap();
        assert!(close(part(&layout.form, 2).volume_m3, 8.0 * CLUSTER_SPAR_M3, 1e-12));
        assert_eq!(part(&layout.form, 0), part(&preset, 0));
        // Every length doubles, so the pods stand twice as far out.
        let far = |form: &Form| placed(form).get(PartId(5), Side::Original).unwrap().position.z;
        assert!(close(far(&layout.form), 2.0 * far(&preset), 1e-12));
    }

    #[test]
    fn a_kind_the_ship_lacks_empties_its_parts_and_their_children_move_up() {
        // No storage at all: the cluster's core and its storage pod are given nothing, and every
        // spar the core held now hangs from the Mind.
        let mut ship = Form::starting();
        ship.parts.retain(|p| p.kind != Kind::Storage);
        for p in &mut ship.parts {
            if let Some(placement) = &mut p.placement {
                placement.parent = MIND;
            }
        }
        let layout = Builtin::Cluster.form().as_layout(&ship, MIN).unwrap();
        assert_eq!(layout.emptied, [PartId(1), PartId(11)]);
        assert_eq!(layout.unplaced, []);
        assert!(of_kind(&layout.form, Kind::Storage).is_empty());
        for id in [2, 4, 6, 8, 10] {
            assert_eq!(part(&layout.form, id).placement.unwrap().parent, MIND, "spar {id}");
        }
        assert_keeps_totals(&layout, &ship, "no storage");
    }

    #[test]
    fn a_kind_the_preset_lacks_is_named_as_unplaced() {
        let mut ship = Form::starting();
        ship.parts.push(hang(9, Kind::Bay, Primitive::Cylinder { length: 1.0 }, 5.0e4, PartId(1), on(DVec3::Y, 0.0)));
        let layout = Builtin::Spindle.form().as_layout(&ship, MIN).unwrap();
        assert_eq!(layout.unplaced, [Kind::Bay]);
        assert_eq!(layout.emptied, []);
        assert_keeps_totals(&layout, &ship, "a bay");
    }

    #[test]
    fn a_share_below_the_smallest_part_is_dropped_and_the_rest_take_its_volume() {
        let mut preset = Form::starting();
        // A second data part with a hundredth of the kind.
        let data = part(&preset, 5).volume_m3;
        preset.parts.push(hang(6, Kind::Data, Primitive::Capsule { length: 1.0 }, data / 99.0, PartId(1), on(DVec3::Y, 0.0)));
        let mut ship = Form::starting();
        let small = 50.0 * MIN;
        ship.parts.iter_mut().find(|p| p.kind == Kind::Data).unwrap().volume_m3 = small;
        let layout = preset.as_layout(&ship, MIN).unwrap();
        assert_eq!(layout.emptied, [PartId(6)]);
        assert_eq!(part(&layout.form, 5).volume_m3, small);
        assert_keeps_totals(&layout, &ship, "a dropped share");

        // With room for both, both stay, 99 : 1.
        ship.parts.iter_mut().find(|p| p.kind == Kind::Data).unwrap().volume_m3 = 1000.0 * MIN;
        let layout = preset.as_layout(&ship, MIN).unwrap();
        assert_eq!(layout.emptied, []);
        assert!(close(part(&layout.form, 6).volume_m3, 10.0 * MIN, 1e-12));
    }

    #[test]
    fn a_design_is_the_preset_exactly_and_a_bad_one_is_refused() {
        let mut preset = Builtin::Cluster.form();
        preset.parts[3].volume_m3 *= 3.7;
        preset.parts[5].placement.as_mut().unwrap().twist = 0.4;
        assert_eq!(preset.as_design(MIN).unwrap(), preset);

        let mut dust = preset.clone();
        dust.parts[5].volume_m3 = MIN * 0.99;
        assert_eq!(dust.as_design(MIN), Err(PresetError::TooSmall(dust.parts[5].id)));
        assert_eq!(dust.as_layout(&Form::starting(), MIN), Err(PresetError::TooSmall(dust.parts[5].id)));
        // The Mind's stored volume is ignored.
        let mut mind = preset.clone();
        mind.parts[0].volume_m3 = 1.0;
        assert!(mind.as_design(MIN).is_ok());

        let mut orphan = preset.clone();
        orphan.parts[4].placement.as_mut().unwrap().parent = PartId(99);
        let refused = Err(PresetError::Form(FormError::MissingParent { part: orphan.parts[4].id, parent: PartId(99) }));
        assert_eq!(orphan.as_design(MIN), refused);
    }
}
