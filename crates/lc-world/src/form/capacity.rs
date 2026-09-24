//! What a form holds and weighs: each kind's capacity from its volume, dry mass from volume and
//! surface, and the mass-energy a refit moves when a part changes.
//!
//! Every part is sized alone from its closed form, overlaps ignored, and a spar as the uncut
//! primitive it is cut from. See `lightcone/docs/29-ship-form.md` §Kinds and §Hull structure
//! follows area.

use super::{Form, Kind, Part};
use crate::fitting::{Balance, C2, ONBOARD_DATA_BYTES};

/// Sums of volume × density per kind. The Mind, bays and spars add nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Capacities {
    /// Joules storage can hold.
    pub storage_j: f64,
    /// Watts of drone building power.
    pub building_w: f64,
    /// Watts of engine aperture, fore and aft together. The rating, not thrust: a fore engine
    /// pushes against an aft one.
    pub aperture_w: f64,
    /// Watts living space drains, continuously.
    pub drain_w: f64,
    /// Bytes of raw log, the onboard baseline included.
    pub data_b: f64,
}

impl Capacities {
    pub fn of(form: &Form, balance: &Balance) -> Self {
        let mut storage_m3 = 0.0;
        let mut drone_m3 = 0.0;
        let mut engine_m3 = 0.0;
        let mut living_m3 = 0.0;
        let mut data_m3 = 0.0;
        for part in &form.parts {
            let v = part.volume_m3;
            match part.kind {
                Kind::Storage => storage_m3 += v,
                Kind::Drone => drone_m3 += v,
                Kind::Engine => engine_m3 += v,
                Kind::Living => living_m3 += v,
                Kind::Data => data_m3 += v,
                Kind::Mind | Kind::Bay | Kind::Spar(_) => {}
            }
        }
        Self {
            // `storage_density` is in module-energies.
            storage_j: storage_m3 * balance.storage_density * balance.module_energy_j(),
            building_w: drone_m3 * balance.drone_density_w,
            aperture_w: engine_m3 * balance.engine_density_w,
            drain_w: living_m3 * balance.living_density_w,
            data_b: ONBOARD_DATA_BYTES + data_m3 * balance.data_density_b,
        }
    }
}

/// Of module density.
pub fn mass_fraction(kind: Kind, balance: &Balance) -> f64 {
    match kind {
        Kind::Data => balance.data_mass_fraction,
        Kind::Bay => balance.bay_mass_fraction,
        Kind::Spar(_) => balance.spar_mass_fraction,
        Kind::Mind | Kind::Storage | Kind::Drone | Kind::Engine | Kind::Living => 1.0,
    }
}

/// What the part's contents weigh, structure excluded. Uses [`Part::shape`]'s volume, so the Mind
/// is the cube of `min_part_m3` whatever it stores.
pub fn body_kg(part: &Part, balance: &Balance) -> f64 {
    let volume = part.shape(balance.min_part_m3).volume();
    volume * balance.module_density_kg_m3 * mass_fraction(part.kind, balance)
}

/// m² of the part's own closed-form surface, which its structure covers.
pub fn area_m2(part: &Part, balance: &Balance) -> f64 {
    part.shape(balance.min_part_m3).area()
}

/// Contents and structure.
pub fn part_kg(part: &Part, balance: &Balance) -> f64 {
    body_kg(part, balance) + balance.hull_areal_density * area_m2(part, balance)
}

pub fn dry_mass_kg(form: &Form, balance: &Balance) -> f64 {
    form.parts.iter().map(|p| part_kg(p, balance)).sum()
}

/// The `hull_areal_density`, kg/m², at which `form` weighs `target_kg` dry. `None` when its
/// contents alone already weigh more, or it has no surface.
pub fn areal_density_for(form: &Form, balance: &Balance, target_kg: f64) -> Option<f64> {
    let body: f64 = form.parts.iter().map(|p| body_kg(p, balance)).sum();
    let area: f64 = form.parts.iter().map(|p| area_m2(p, balance)).sum();
    let density = (target_kg - body) / area;
    (density.is_finite() && density >= 0.0).then_some(density)
}

/// The mass-energy one step of a refit moves, as 19's loadout moves it for a module.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Transfer {
    /// Taken from storage, joules.
    Build { cost_j: f64 },
    /// `gross_j` taken apart, of which `returned_j` goes back to storage if there is room.
    Dismantle { gross_j: f64, returned_j: f64 },
}

impl Transfer {
    /// A part going from `from` to `to`, `None` where it is absent. Only size may differ between
    /// the two: a reshape or a change of kind is a dismantle of all of `from` then a build of
    /// all of `to`, so it is two calls.
    pub fn of(from: Option<&Part>, to: Option<&Part>, balance: &Balance) -> Self {
        if let (Some(from), Some(to)) = (from, to) {
            // Priced as a resize, a reshape would skip the loss on all of it.
            debug_assert!(from.kind == to.kind && from.primitive == to.primitive, "a reshape is two transfers");
        }
        let kg = |p: Option<&Part>| p.map_or(0.0, |p| part_kg(p, balance));
        // Structure goes as volume^(2/3), so this is not a volume difference at one density.
        let delta_j = (kg(to) - kg(from)) * C2;
        if delta_j >= 0.0 {
            Transfer::Build { cost_j: delta_j }
        } else {
            Transfer::Dismantle { gross_j: -delta_j, returned_j: balance.recovery * -delta_j }
        }
    }

    /// Joules into storage: negative for a build.
    pub fn net_j(&self) -> f64 {
        match *self {
            Transfer::Build { cost_j } => -cost_j,
            Transfer::Dismantle { returned_j, .. } => returned_j,
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::{DVec2, DVec3};

    use super::*;
    use crate::fitting::Loadout;
    use crate::form::{Mount, PartId, Placement, Primitive, SparMode};

    fn part(id: u16, kind: Kind, primitive: Primitive, volume_m3: f64) -> Part {
        let placement = Placement {
            parent: PartId(0),
            mount: Mount::Attached { anchor: DVec3::X, standoff: 0.0 },
            twist: 0.0,
            tilt: DVec2::ZERO,
            blend: 0.0,
            mirror: false,
        };
        Part { id: PartId(id), kind, primitive, volume_m3, placement: Some(placement) }
    }

    /// 19's starting ship as one part per module kind, each holding that kind's slots, in the
    /// primitives 29's starting form gives them.
    fn nineteen(balance: &Balance) -> Form {
        let start = Loadout::STARTING;
        let slot = balance.slot_volume_m3;
        let n = |count: u32| count as f64 * slot;
        Form {
            parts: vec![
                Part::mind(PartId(0), balance.min_part_m3),
                part(1, Kind::Storage, Primitive::Ellipsoid { axes: DVec3::new(5.0, 3.0, 1.0) }, n(start.storage)),
                part(2, Kind::Engine, Primitive::Frustum { length: 1.5, taper: 0.6 }, n(start.engines)),
                part(3, Kind::Drone, Primitive::Capsule { length: 4.0 }, n(start.drones)),
                part(4, Kind::Living, Primitive::Slab { edges: DVec3::new(4.0, 3.0, 0.5), corner: 0.2 }, n(start.living)),
                part(5, Kind::Data, Primitive::Capsule { length: 2.0 }, n(start.data)),
            ],
        }
    }

    fn close(a: f64, b: f64) -> bool {
        ((a - b) / b).abs() < 1e-9
    }

    #[test]
    fn nineteens_volumes_have_nineteens_capacities() {
        let b = Balance::DEFAULT;
        let start = Loadout::STARTING;
        let c = Capacities::of(&nineteen(&b), &b);
        assert!(close(c.storage_j, b.capacity_j(&start)), "{} vs {}", c.storage_j, b.capacity_j(&start));
        assert!(close(c.building_w, b.refit_power_w(&start)));
        assert!(close(c.drain_w, b.drain_w(&start)));
        assert!(close(c.data_b, b.data_capacity(&start)));
        assert!(close(c.aperture_w / crate::flight::C_M_S, start.engines as f64 * b.engine_thrust_n));
    }

    /// The comparison above cannot see a density that DEFAULT derives wrongly from a per-module
    /// value that is wrong the same way, so pin 29's table too.
    #[test]
    fn densities_and_fractions_are_29s() {
        let b = Balance::DEFAULT;
        let figures = |x: f64, want: f64| ((x - want) / want).abs() < 5e-3;
        let me = b.module_energy_j();
        assert!(figures(me, 1.397e25), "{me}");
        assert!(figures(b.storage_density, 1.27e-5));
        assert!(figures(b.drone_density_w, 5.88e13));
        assert!(figures(b.engine_density_w, 5.47e13));
        assert!(figures(b.living_density_w, 1.13e10));
        assert!(figures(b.data_density_b, 7.5));
        assert!(figures(b.module_density_kg_m3, 395.8));
        assert_eq!(mass_fraction(Kind::Data, &b), 0.5);
        assert_eq!(mass_fraction(Kind::Bay, &b), 0.1);
        assert_eq!(mass_fraction(Kind::Spar(SparMode::Strap), &b), 0.05);
        for kind in [Kind::Mind, Kind::Storage, Kind::Drone, Kind::Engine, Kind::Living] {
            assert_eq!(mass_fraction(kind, &b), 1.0);
        }
    }

    #[test]
    fn the_mind_bays_and_spars_hold_nothing() {
        let b = Balance::DEFAULT;
        let v = 1.0e6;
        let form = Form {
            parts: vec![
                Part::mind(PartId(0), b.min_part_m3),
                part(1, Kind::Bay, Primitive::Cylinder { length: 2.0 }, v),
                part(2, Kind::Spar(SparMode::Saddle), Primitive::Cylinder { length: 20.0 }, v),
            ],
        };
        assert_eq!(Capacities::of(&form, &b), Capacities { data_b: ONBOARD_DATA_BYTES, ..Default::default() });
    }

    /// With no structure, contents weigh what 19's modules do, plus the Mind, which 19 has no
    /// counterpart for. The anchor below is where it is absorbed.
    #[test]
    fn contents_weigh_what_nineteens_modules_do() {
        let b = Balance { hull_areal_density: 0.0, ..Balance::DEFAULT };
        let start = Loadout::STARTING;
        let modules_kg = b.dry_mass_kg(&start) - start.slots as f64 * b.slot_structure_kg();
        let mind_kg = b.min_part_m3 * b.module_density_kg_m3;
        assert!(close(dry_mass_kg(&nineteen(&b), &b), modules_kg + mind_kg));
    }

    #[test]
    fn structure_is_each_parts_own_area() {
        let b = Balance { hull_areal_density: 120.0, ..Balance::DEFAULT };
        let form = nineteen(&b);
        let area: f64 = form.parts.iter().map(|p| p.shape(b.min_part_m3).area()).sum();
        let bare = Balance { hull_areal_density: 0.0, ..b };
        assert!(close(dry_mass_kg(&form, &b) - dry_mass_kg(&form, &bare), 120.0 * area));
        // The Mind is the 10 m cube, whatever it stores.
        let mind = Part { volume_m3: 8.0e6, primitive: Primitive::Capsule { length: 3.0 }, ..form.parts[0] };
        assert!(close(part_kg(&mind, &b), 1000.0 * b.module_density_kg_m3 + 600.0 * 120.0));
    }

    #[test]
    fn a_spar_and_a_bay_weigh_their_fraction_of_their_uncut_primitive() {
        let b = Balance { hull_areal_density: 50.0, ..Balance::DEFAULT };
        let primitive = Primitive::Cylinder { length: 12.0 };
        let v = 2.0e5;
        let r = (v / (12.0 * std::f64::consts::PI)).cbrt();
        let area = 2.0 * std::f64::consts::PI * r * (12.0 * r + r);
        for (kind, fraction) in [(Kind::Spar(SparMode::Strap), 0.05), (Kind::Spar(SparMode::Saddle), 0.05), (Kind::Bay, 0.1)] {
            let kg = part_kg(&part(1, kind, primitive, v), &b);
            assert!(close(kg, fraction * v * b.module_density_kg_m3 + 50.0 * area), "{kind:?}");
        }
    }

    #[test]
    fn the_areal_density_is_solved_to_weigh_nineteens_starting_ship() {
        let b = Balance::DEFAULT;
        let form = nineteen(&b);
        let target = b.dry_mass_kg(&Loadout::STARTING);
        let density = areal_density_for(&form, &b, target).unwrap();
        let anchored = Balance { hull_areal_density: density, ..b };
        assert!(close(dry_mass_kg(&form, &anchored), target));
        // A structure of the right order: 19's frame over the surface of 15 slots of parts.
        assert!((100.0..10_000.0).contains(&density), "{density}");
        assert_eq!(areal_density_for(&form, &b, 1.0), None);
    }

    #[test]
    fn building_costs_the_mass_energy_and_dismantling_returns_recovery_of_it() {
        let b = Balance { hull_areal_density: 80.0, ..Balance::DEFAULT };
        let tank = part(1, Kind::Storage, Primitive::Ellipsoid { axes: DVec3::new(5.0, 3.0, 1.0) }, 1.0e6);
        let whole_j = part_kg(&tank, &b) * C2;
        assert_eq!(Transfer::of(None, Some(&tank), &b), Transfer::Build { cost_j: whole_j });
        let Transfer::Dismantle { gross_j, returned_j } = Transfer::of(Some(&tank), None, &b) else {
            panic!("removing is a dismantle");
        };
        assert_eq!(gross_j, whole_j);
        assert!(close(returned_j, 0.95 * whole_j));

        // Growing and shrinking by the same step are the same mass, and a round trip loses 5% of it.
        let bigger = Part { volume_m3: 3.0e6, ..tank };
        let grow = Transfer::of(Some(&tank), Some(&bigger), &b);
        let shrink = Transfer::of(Some(&bigger), Some(&tank), &b);
        let Transfer::Build { cost_j } = grow else { panic!("growing is a build") };
        assert!(close(cost_j, (part_kg(&bigger, &b) - part_kg(&tank, &b)) * C2));
        assert!(close(grow.net_j() + shrink.net_j(), -0.05 * cost_j));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a reshape is two transfers")]
    fn a_reshape_is_refused_as_one_transfer() {
        let b = Balance::DEFAULT;
        let rod = part(1, Kind::Spar(SparMode::Saddle), Primitive::Cylinder { length: 8.0 }, 1.0e5);
        let strap = Part { kind: Kind::Spar(SparMode::Strap), ..rod };
        Transfer::of(Some(&rod), Some(&strap), &b);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "a reshape is two transfers")]
    fn a_new_primitive_is_refused_as_one_transfer() {
        let b = Balance::DEFAULT;
        let rod = part(1, Kind::Storage, Primitive::Cylinder { length: 8.0 }, 1.0e5);
        let ball = Part { primitive: Primitive::Capsule { length: 0.0 }, ..rod };
        Transfer::of(Some(&rod), Some(&ball), &b);
    }

    #[test]
    fn a_modules_worth_of_volume_costs_what_building_a_module_does() {
        let b = Balance { hull_areal_density: 0.0, ..Balance::DEFAULT };
        for (kind, module) in [(Kind::Storage, crate::fitting::Module::Storage), (Kind::Data, crate::fitting::Module::Data)] {
            let one = part(1, kind, Primitive::Capsule { length: 1.0 }, b.slot_volume_m3);
            let Transfer::Build { cost_j } = Transfer::of(None, Some(&one), &b) else { panic!() };
            assert!(close(cost_j, b.build_energy_j(module)), "{kind:?}");
        }
    }
}
