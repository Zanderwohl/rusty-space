//! 29 §Placement rules: what a target must satisfy beyond its structure, each failure naming the
//! part.
//!
//! [`check`] is the one entry point, for the server's refusal and the editor's marks alike. It
//! returns every fault, because the editor marks each part it can; the server sends the first.
//! Faults come in the order 29 lists the rules, and by part id within a rule. A mirrored part is
//! named once, whichever copy broke the rule.
//!
//! Geometry is judged on the grid's cell centers, in its fixed order, so the server and a client
//! agree to the bit, and to its resolution: a cell's half diagonal. Below that the grid cannot see
//! a gap, so neither can anything the server states from it.

use std::collections::BTreeSet;

use glam::DVec3;

use super::grid::{FormGrid, Shadow};
use super::primitive::Shape;
use super::{Form, FormError, Kind, Mount, Part, PartId};
use crate::craft::LENGTH_RANGE_M;
use crate::fitting::Balance;

/// Of the unit axis, the part across the nose that still counts as along it. Placement's rotations
/// are exact to rounding, so this only forgives the last few ulps of a composed tilt.
const AXIS_TOLERANCE: f64 = 1e-9;

/// Every rule, on a form that is about to be the ship. The grid comes back on success so a
/// caller that states geometry does not build it twice.
pub fn check(form: &Form, balance: &Balance) -> Result<FormGrid, Vec<FormError>> {
    // Nothing below can be judged on a form that is not one tree.
    form.validate().map_err(|e| vec![e])?;
    let mut faults = Vec::new();
    let grid = match FormGrid::new(form, balance) {
        Ok(grid) => {
            faults.extend(geometry(form, &grid, balance));
            let extent = grid.extent_m();
            if !(LENGTH_RANGE_M.0..=LENGTH_RANGE_M.1).contains(&extent) {
                faults.push(FormError::Extent);
            }
            Some(grid)
        }
        Err(e) => {
            faults.push(e);
            None
        }
    };
    faults.extend(sizes(form, balance));
    match grid {
        Some(grid) if faults.is_empty() => Ok(grid),
        _ => Err(faults),
    }
}

/// The rules that need no geometry: every part at least `min_part_m3` and the drones at least
/// `min_drone_m3`, every copy counted. The refit planner refuses on these without a grid.
pub fn sizes(form: &Form, balance: &Balance) -> Vec<FormError> {
    let mut faults: Vec<FormError> = {
        let small: BTreeSet<PartId> =
            form.parts.iter().filter(|p| too_small(p, balance.min_part_m3)).map(|p| p.id).collect();
        small.into_iter().map(FormError::TooSmall).collect()
    };
    let copies = form.copies();
    let drone_m3: f64 =
        form.parts.iter().filter(|p| p.kind == Kind::Drone).map(|p| p.volume_m3 * f64::from(copies[&p.id])).sum();
    if drone_m3 < balance.min_drone_m3 {
        faults.push(FormError::TooFewDrones);
    }
    faults
}

/// Per copy. The Mind's stored volume is ignored, so it is never too small.
pub fn too_small(part: &Part, min_part_m3: f64) -> bool {
    part.kind != Kind::Mind && part.volume_m3 < min_part_m3
}

fn geometry(form: &Form, grid: &FormGrid, balance: &Balance) -> Vec<FormError> {
    let sdf = grid.sdf();
    let pieces = sdf.pieces();
    let named = |test: &dyn Fn(usize) -> bool| -> BTreeSet<PartId> {
        (0..pieces.len()).filter(|&i| test(i)).map(|i| pieces[i].part).collect()
    };
    let engine = |i: usize| pieces[i].kind == Kind::Engine;
    let tan = libm::tan(balance.engine_clear_half_angle_rad);

    let off_axis = named(&|i| {
        let a = pieces[i].pose.axis();
        engine(i) && (a.y * a.y + a.z * a.z).sqrt() > AXIS_TOLERANCE
    });
    // The exhaust leaves the whole face, so the cone widens from its rim rather than its center.
    let blocked = named(&|i| engine(i) && obstructed(grid, i, |s, r, radius| s > 0.0 && r <= radius + s * tan));
    let bay = named(&|i| {
        pieces[i].kind == Kind::Bay && obstructed(grid, i, |s, r, radius| s > 0.0 && s <= 2.0 * radius && r <= radius)
    });

    let mount = |id: PartId| form.parts.iter().find(|p| p.id == id).and_then(|p| p.placement).map(|p| p.mount);
    let detached = named(&|i| matches!(mount(pieces[i].part), Some(Mount::Attached { .. })) && !touches(grid, i));
    let uncontained = named(&|i| matches!(mount(pieces[i].part), Some(Mount::Enclosing)) && !contains(grid, i));

    let mut faults = Vec::new();
    faults.extend(off_axis.into_iter().map(FormError::EngineOffAxis));
    faults.extend(blocked.into_iter().map(FormError::EngineBlocked));
    faults.extend(bay.into_iter().map(FormError::BayBlocked));
    let mut placed: Vec<FormError> = detached
        .into_iter()
        .map(FormError::Detached)
        .chain(uncontained.into_iter().map(FormError::Uncontained))
        .collect();
    placed.sort_by_key(|e| match *e {
        FormError::Detached(id) | FormError::Uncontained(id) => id,
        _ => unreachable!("only placement faults"),
    });
    faults.extend(placed);
    faults
}

/// The face an engine fires from or a bay opens through: where it crosses the part's axis, in
/// its own x, and its radius. 31 makes an engine's aperture a frustum's wide end; every other
/// primitive opens at +x, the end away from the foot it hangs by. A torus opens through its hole.
fn face(shape: &Shape) -> (f64, f64) {
    match *shape {
        Shape::Ellipsoid { semi_axes } => (semi_axes.x, semi_axes.y.min(semi_axes.z)),
        Shape::Capsule { radius, length } => (length / 2.0 + radius, radius),
        Shape::Slab { edges, .. } => (edges.x / 2.0, edges.y.min(edges.z) / 2.0),
        Shape::Cylinder { radius, length } => (length / 2.0, radius),
        Shape::Torus { major, minor } => (minor, major - minor),
        Shape::Frustum { length, start, end } => {
            if end >= start { (length / 2.0, end) } else { (-length / 2.0, start) }
        }
    }
}

/// Whether a filled cell inside another piece lies in the region in front of `piece`'s face.
/// `region(s, r, radius)` takes a point's distance out along the face's normal, its distance from
/// that normal's line, and the face's radius. Every other piece blocks, a mirrored copy of this
/// one included: 29 exempts only the part itself. A face is at its part's far end, so the part
/// reaches the region only by rounding on the face's plane.
fn obstructed(grid: &FormGrid, piece: usize, region: impl Fn(f64, f64, f64) -> bool) -> bool {
    let sdf = grid.sdf();
    let this = &sdf.pieces()[piece];
    let (x, radius) = face(&this.shape);
    let center = this.pose.to_outer(DVec3::X * x);
    let out = this.pose.axis() * x.signum();
    let [nx, ny, nz] = grid.dims();
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                if !grid.filled(i, j, k) {
                    continue;
                }
                let p = grid.cell_center(i, j, k);
                let v = p - center;
                let s = v.dot(out);
                if !region(s, (v - out * s).length(), radius) {
                    continue;
                }
                if (0..sdf.pieces().len()).any(|q| q != piece && sdf.piece_distance(q, p) <= 0.0) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether some cell center near an attached piece is within half a cell's diagonal of both its
/// primitive and its parent's, uncut and unblended, or within a quarter of the joint's blend
/// radius more, the most the fillet reaches. Both fields are Lipschitz 1 even off an ellipsoid,
/// so wherever the two surfaces meet the nearest center passes: a part that touches is never
/// refused. The bound is short beside an ellipsoid's long axes, so there a gap of up to the axis
/// ratio times as much passes too.
fn touches(grid: &FormGrid, piece: usize) -> bool {
    let sdf = grid.sdf();
    let parent = sdf.parent(piece).expect("an attached part has a parent");
    let this = &sdf.pieces()[piece];
    let cell = grid.cell_m();
    let tolerance = cell * 3f64.sqrt() / 2.0 + sdf.blend_m(piece) / 4.0;

    let half = this.shape.extent(this.pose.rotation, tolerance);
    let origin = grid.cell_center(0, 0, 0);
    let dims = grid.dims();
    let lo = ((this.pose.position - half - origin) / cell).to_array();
    let hi = ((this.pose.position + half - origin) / cell).to_array();
    let range = |a: usize| {
        let first = lo[a].ceil().max(0.0) as usize;
        let end = (hi[a].floor() + 1.0).clamp(0.0, dims[a] as f64) as usize;
        first..end
    };
    for k in range(2) {
        for j in range(1) {
            for i in range(0) {
                let p = grid.cell_center(i, j, k);
                if sdf.primitive(piece, p).max(sdf.primitive(parent, p)) <= tolerance {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether the parent's surface is inside an enclosing piece, sampled along the shadow's 162
/// directions and the 26 of a cube's faces, edges and corners, which find a slab's. The encloser is
/// read by [`Shape::estimate`], whose sign is right everywhere and which is accurate to first
/// order near the surface, where a protrusion begins. A protrusion under half a cell's diagonal is
/// forgiven, as the grid could not show it.
fn contains(grid: &FormGrid, piece: usize) -> bool {
    let sdf = grid.sdf();
    let parent = &sdf.pieces()[sdf.parent(piece).expect("an enclosing part has a parent")];
    let this = &sdf.pieces()[piece];
    let tolerance = grid.cell_m() * 3f64.sqrt() / 2.0;
    let lattice = (0..27).filter(|&n| n != 13).map(|n| {
        DVec3::new((n % 3) as f64 - 1.0, ((n / 3) % 3) as f64 - 1.0, (n / 9) as f64 - 1.0).normalize()
    });
    Shadow::directions().iter().copied().chain(lattice).all(|d| {
        let p = parent.pose.to_outer(parent.shape.exit(d).point);
        this.shape.estimate(this.pose.to_local(p)) <= tolerance
    })
}

#[cfg(test)]
mod tests {
    use glam::DVec2;

    use super::*;
    use crate::form::{Placement, Primitive};

    const B: Balance = Balance::DEFAULT;

    fn part_mut(form: &mut Form, id: u16) -> &mut Part {
        form.parts.iter_mut().find(|p| p.id == PartId(id)).unwrap()
    }

    fn hang(id: u16, kind: Kind, primitive: Primitive, volume_m3: f64, parent: u16, mount: Mount) -> Part {
        let placement =
            Placement { parent: PartId(parent), mount, twist: 0.0, tilt: DVec2::ZERO, blend: 0.0, mirror: false };
        Part { id: PartId(id), kind, primitive, volume_m3, placement: Some(placement) }
    }

    fn on(anchor: DVec3, standoff: f64) -> Mount {
        Mount::Attached { anchor, standoff }
    }

    fn refused(form: &Form) -> Vec<FormError> {
        check(form, &B).expect_err("refused")
    }

    #[test]
    fn an_engine_off_the_nose_axis_is_named() {
        let mut form = Form::starting();
        part_mut(&mut form, 2).placement.as_mut().unwrap().tilt = DVec2::new(0.0, 0.1);
        assert_eq!(refused(&form), [FormError::EngineOffAxis(PartId(2))]);
    }

    #[test]
    fn twist_about_the_nose_keeps_an_engine_on_it() {
        let mut form = Form::starting();
        part_mut(&mut form, 2).placement.as_mut().unwrap().twist = 0.7;
        assert!(check(&form, &B).is_ok());
    }

    /// A pod hung on the bell's open face, where the exhaust leaves.
    #[test]
    fn a_part_in_an_engines_cone_blocks_it() {
        let mut form = Form::starting();
        form.parts.push(hang(6, Kind::Living, Primitive::Capsule { length: 1.0 }, 5.0e4, 2, on(DVec3::X, 0.0)));
        assert_eq!(refused(&form), [FormError::EngineBlocked(PartId(2))]);
    }

    /// The cone widens from the face's rim: a ball on the face well off the axis, clear of a
    /// cone from the face's center, is still in the exhaust.
    #[test]
    fn the_cone_is_as_wide_as_the_face() {
        let mut form = Form::starting();
        let Shape::Frustum { length, end, .. } = form.parts[2].shape(B.min_part_m3) else { unreachable!() };
        let r: f64 = 20.0;
        let ball = Primitive::Ellipsoid { axes: DVec3::ONE };
        let volume = 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
        let rim = on(DVec3::new(length / 2.0, 0.6 * end, 0.0), 0.0);
        form.parts.push(hang(6, Kind::Living, ball, volume, 2, rim));
        let tan = libm::tan(B.engine_clear_half_angle_rad);
        assert!(0.6 * end - r > 2.0 * r * tan, "outside a cone from the center");
        assert_eq!(refused(&form), [FormError::EngineBlocked(PartId(2))]);
    }

    /// A slab with an engine on its aft face and a long data pod beside it, reaching past the
    /// engine's face. `beside` is the pod's anchor across the beam; the face is at y = 0.5 of it.
    fn pod_beside_engine(beside: f64) -> Form {
        let slab = Primitive::Slab { edges: DVec3::new(1.0, 6.0, 3.0), corner: 0.0 };
        let bell = Primitive::Cylinder { length: 2.0 };
        let pod = Primitive::Capsule { length: 4.0 };
        Form {
            parts: vec![
                Part::mind(PartId(0), B.min_part_m3),
                hang(1, Kind::Storage, slab, 1.9e7, 0, Mount::Enclosing),
                hang(2, Kind::Engine, bell, 8.0e6, 1, on(DVec3::NEG_X, 0.0)),
                hang(3, Kind::Drone, Primitive::Capsule { length: 1.0 }, 1.0e5, 1, on(DVec3::Z, -0.2)),
                hang(4, Kind::Data, pod, 1.6e6, 1, on(DVec3::new(-1.0, beside, 0.0), 0.0)),
            ],
        }
    }

    /// The cone's edge: the pod reaches about a quarter of the engine's length past its face, its
    /// near side 180 m off the axis where the cone's radius is 124 m, so only a cone narrower than
    /// the half-space in front of the face lets it pass.
    #[test]
    fn a_part_beside_the_cone_does_not_block_it() {
        let form = pod_beside_engine(4.5);
        let Shape::Capsule { radius, length } = form.parts[4].shape(B.min_part_m3) else { unreachable!() };
        let Shape::Cylinder { length: bell, .. } = form.parts[2].shape(B.min_part_m3) else { unreachable!() };
        assert!(length + 2.0 * radius > bell, "the pod reaches past the face");
        if let Err(faults) = check(&form, &B) {
            panic!("{faults:?}");
        }
        assert_eq!(refused(&pod_beside_engine(1.2)), [FormError::EngineBlocked(PartId(2))]);
    }

    fn with_bay(anchor: DVec3) -> Form {
        let mut form = Form::starting();
        form.parts.push(hang(6, Kind::Bay, Primitive::Cylinder { length: 1.0 }, 2.0e5, 1, on(anchor, -0.3)));
        form
    }

    #[test]
    fn a_clear_bay_passes_and_a_blocked_one_is_named() {
        let port = DVec3::new(0.0, 1.0, 0.0);
        assert!(check(&with_bay(port), &B).is_ok());
        let mut form = with_bay(port);
        form.parts.push(hang(7, Kind::Data, Primitive::Capsule { length: 0.5 }, 5.0e4, 6, on(DVec3::X, 0.0)));
        assert_eq!(refused(&form), [FormError::BayBlocked(PartId(6))]);
    }

    /// The mouth is cleared out to its width and no further. A ball on the bay's axis, floating
    /// and so detached, blocks it inside twice the face's radius and not beyond.
    #[test]
    fn a_bays_mouth_is_cleared_to_its_width() {
        let ball = |gap_m: f64| {
            let mut form = with_bay(DVec3::Y);
            let r: f64 = 10.0;
            let volume = 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);
            form.parts.push(hang(7, Kind::Data, Primitive::Ellipsoid { axes: DVec3::ONE }, volume, 6, on(DVec3::X, gap_m / r)));
            form
        };
        let Shape::Cylinder { radius, .. } = with_bay(DVec3::Y).parts[6].shape(B.min_part_m3) else { unreachable!() };
        assert_eq!(refused(&ball(radius)), [FormError::BayBlocked(PartId(6)), FormError::Detached(PartId(7))]);
        assert_eq!(refused(&ball(3.0 * radius)), [FormError::Detached(PartId(7))]);
    }

    #[test]
    fn a_part_floating_off_its_parent_is_named() {
        let mut form = Form::starting();
        // Two data reaches clear of the nose.
        part_mut(&mut form, 5).placement.as_mut().unwrap().mount = on(DVec3::X, 2.0);
        assert_eq!(refused(&form), [FormError::Detached(PartId(5))]);
    }

    #[test]
    fn a_gap_below_the_grids_resolution_is_not_a_detachment() {
        let mut form = Form::starting();
        let reach = part_mut(&mut form, 5).shape(B.min_part_m3).reach();
        let cell = check(&form, &B).unwrap().cell_m();
        part_mut(&mut form, 5).placement.as_mut().unwrap().mount = on(DVec3::X, 0.25 * cell / reach);
        assert!(check(&form, &B).is_ok());
    }

    #[test]
    fn an_encloser_smaller_than_its_parent_is_named() {
        let mut form = Form::starting();
        // A 50 m sphere around the 335 m hull.
        form.parts.push(hang(6, Kind::Living, Primitive::Ellipsoid { axes: DVec3::ONE }, 5.0e5, 1, Mount::Enclosing));
        assert_eq!(refused(&form), [FormError::Uncontained(PartId(6))]);
    }

    #[test]
    fn a_ship_too_short_or_too_long_is_refused() {
        let mut form = Form::starting();
        form.parts.iter_mut().for_each(|p| p.volume_m3 *= 1.0e6);
        assert_eq!(refused(&form), [FormError::Extent]);

        // A lone sphere with drones, a hundred meters long.
        let tiny = Form {
            parts: vec![
                Part::mind(PartId(0), B.min_part_m3),
                hang(1, Kind::Storage, Primitive::Ellipsoid { axes: DVec3::ONE }, 3.0e5, 0, Mount::Enclosing),
                hang(2, Kind::Drone, Primitive::Capsule { length: 1.0 }, B.min_drone_m3, 1, on(DVec3::NEG_Z, -0.2)),
            ],
        };
        assert_eq!(refused(&tiny), [FormError::Extent]);
    }

    #[test]
    fn dust_is_named() {
        let mut form = Form::starting();
        part_mut(&mut form, 5).volume_m3 = 0.999 * B.min_part_m3;
        assert_eq!(sizes(&form, &B), [FormError::TooSmall(PartId(5))]);
        assert_eq!(refused(&form), [FormError::TooSmall(PartId(5))]);
        // The Mind's stored volume is ignored.
        part_mut(&mut form, 5).volume_m3 = B.min_part_m3;
        part_mut(&mut form, 0).volume_m3 = 1.0;
        assert_eq!(sizes(&form, &B), []);
    }

    #[test]
    fn too_few_drones_is_refused_and_a_mirror_counts_both_copies() {
        let mut form = Form::starting();
        part_mut(&mut form, 3).volume_m3 = 0.6 * B.min_drone_m3;
        assert_eq!(sizes(&form, &B), [FormError::TooFewDrones]);
        assert_eq!(refused(&form), [FormError::TooFewDrones]);
        part_mut(&mut form, 3).placement.as_mut().unwrap().mirror = true;
        assert_eq!(sizes(&form, &B), []);
        assert!(check(&form, &B).is_ok());
    }

    #[test]
    fn every_fault_is_returned_in_rule_order() {
        let mut form = Form::starting();
        part_mut(&mut form, 2).placement.as_mut().unwrap().tilt = DVec2::new(0.0, 0.1);
        part_mut(&mut form, 5).placement.as_mut().unwrap().mount = on(DVec3::X, 2.0);
        part_mut(&mut form, 4).volume_m3 = 0.5 * B.min_part_m3;
        let faults = refused(&form);
        assert_eq!(faults.first(), Some(&FormError::EngineOffAxis(PartId(2))), "{faults:?}");
        for fault in [FormError::Detached(PartId(5)), FormError::TooSmall(PartId(4))] {
            assert!(faults.contains(&fault), "{fault:?} in {faults:?}");
        }
    }

    #[test]
    fn a_structural_fault_is_the_only_one() {
        let mut form = Form::starting();
        part_mut(&mut form, 5).placement.as_mut().unwrap().parent = PartId(40);
        part_mut(&mut form, 4).volume_m3 = 1.0;
        assert_eq!(refused(&form), [FormError::MissingParent { part: PartId(5), parent: PartId(40) }]);
    }

    #[test]
    fn each_rule_crosses_the_wire_naming_its_part() {
        use lc_proto::form::PartId as P;
        use lc_proto::FormFault as F;
        let cases = [
            (FormError::EngineOffAxis(PartId(2)), F::EngineOffAxis(P(2))),
            (FormError::EngineBlocked(PartId(3)), F::EngineBlocked(P(3))),
            (FormError::BayBlocked(PartId(4)), F::BayBlocked(P(4))),
            (FormError::Detached(PartId(5)), F::Detached(P(5))),
            (FormError::Uncontained(PartId(6)), F::Uncontained(P(6))),
            (FormError::Extent, F::Extent),
            (FormError::TooSmall(PartId(7)), F::TooSmall(P(7))),
            (FormError::TooFewDrones, F::TooFewDrones),
        ];
        for (error, fault) in cases {
            assert_eq!(F::from(error), fault);
        }
    }
}
