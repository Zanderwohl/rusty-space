//! Spheres of influence: which attractor owns a region of space.
//!
//! [`em_foundations::patched_conics`] holds the radius formulas. This turns them into a
//! shape that can be asked questions — how far does it reach in *this* direction, does it
//! contain *this* point, which body owns *this* position — and into a hierarchy query.
//!
//! Everything is in metres, in simulation space (right-handed, Z-up, ecliptic of J2000).
//!
//! # Adding a model
//!
//! One arm in [`Soi::radius_at_cos`], and, only if the model is genuinely anisotropic,
//! one arm in the renderer's WGSL shape function. Nothing else changes: containment, the
//! renderer, and the eventual patched-conics search all go through
//! [`Soi::radius_toward`].

use em_foundations::patched_conics;
use em_foundations::time::Instant;
use glam::DVec3;
use serde::{Deserialize, Serialize};

use crate::appearance::Appearance;
use crate::id::BodyIndex;
use crate::motive::MotiveSelection;
use crate::propagate;
use crate::system::System;

/// How a body's sphere of influence is measured.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Default)]
pub enum SoiModel {
    /// No sphere at all: a root body, a massless one, or one heavier than its primary.
    None,
    /// Hill sphere at the live separation. Breathes over an eccentric orbit.
    #[default]
    Hill,
    /// Hill sphere at periapsis: the smallest it gets, and so the one number that holds
    /// for a whole orbit.
    HillPeriapsis,
    Laplace,
    LaplaceIntegrated,
    /// Laplace, flattened toward the primary. A surface of revolution about the
    /// body-primary axis, ~13% narrower along it than across.
    LaplaceAngled,
    /// Bondi accretion radius, for a black hole.
    Bondi { velocity_dispersion: f64 },
    /// An explicit radius in metres, for a scripted or authored override.
    Fixed(f64),
}

/// A body's sphere of influence, frozen at one instant.
///
/// Radii are in metres from [`centre`](Self::centre). Built by [`soi_now`] or [`soi_at`];
/// the fields are public so a caller can build one by hand for a hypothetical.
#[derive(Debug, Clone, Copy)]
pub struct Soi {
    pub body: BodyIndex,
    pub model: SoiModel,
    /// The body's position in simulation space.
    pub centre: DVec3,
    /// Unit vector from the body toward its primary. [`DVec3::ZERO`] when there is none.
    pub to_primary: DVec3,
    /// Live distance to the primary, metres.
    pub separation: f64,
    pub semi_major_axis: f64,
    pub eccentricity: f64,
    pub body_mass: f64,
    pub primary_mass: f64,
    pub gravitational_constant: f64,
}

impl Soi {
    /// Radius toward `dir`, metres. `dir` need not be normalised; a zero vector is read as
    /// "across the primary line", where every model is at its widest.
    pub fn radius_toward(&self, dir: DVec3) -> f64 {
        let cos = if dir.length_squared() > 0.0 && self.to_primary.length_squared() > 0.0 {
            dir.normalize().dot(self.to_primary).clamp(-1.0, 1.0)
        } else {
            0.0
        };
        self.radius_at_cos(cos)
    }

    /// The largest radius over all directions — the bounding sphere, and the number the
    /// renderer scales its unit mesh by.
    pub fn bounding_radius(&self) -> f64 {
        // Anisotropy here is a squash *toward* the primary, so the widest direction is
        // across it.
        self.radius_at_cos(0.0)
    }

    /// The smallest radius over all directions.
    pub fn min_radius(&self) -> f64 {
        self.radius_at_cos(1.0)
    }

    /// Whether the sphere is a sphere. Isotropic models let the renderer skip the
    /// silhouette iteration and use the closed form.
    pub fn is_isotropic(&self) -> bool {
        self.bounding_radius() == self.min_radius()
    }

    /// Whether `point` (simulation space) falls inside.
    pub fn contains(&self, point: DVec3) -> bool {
        if matches!(self.model, SoiModel::None) {
            return false;
        }
        let offset = point - self.centre;
        offset.length() <= self.radius_toward(offset)
    }

    /// The one match. `cos_theta` is the cosine of the angle from the primary direction.
    fn radius_at_cos(&self, cos_theta: f64) -> f64 {
        let (a, e) = (self.semi_major_axis, self.eccentricity);
        let (m, big) = (self.body_mass, self.primary_mass);
        match self.model {
            SoiModel::None => 0.0,
            SoiModel::Hill => patched_conics::hill_at_separation(self.separation, m, big),
            SoiModel::HillPeriapsis => patched_conics::hill_at_periapsis(a, e, m, big),
            SoiModel::Laplace => patched_conics::laplace(a, m, big),
            SoiModel::LaplaceIntegrated => patched_conics::laplace_integrated(a, m, big),
            SoiModel::LaplaceAngled => {
                patched_conics::laplace_angled(a, m, big, cos_theta.acos())
            }
            SoiModel::Bondi { velocity_dispersion } => {
                patched_conics::black_hole(self.gravitational_constant, m, velocity_dispersion)
            }
            SoiModel::Fixed(radius) => radius,
        }
    }
}

/// The model a body gets when nothing overrides it.
///
/// Every heuristic lives here, so retuning which bodies get which sphere is a
/// one-function edit.
pub fn default_model(system: &System, i: BodyIndex) -> SoiModel {
    let mass = system.mass(i);
    if mass <= 0.0 {
        return SoiModel::None; // a massless body dominates nothing
    }
    let Some(primary) = system.parent(i) else {
        // A root's influence is unbounded — see `containment_chain`. There is no surface
        // to draw and no radius to quote.
        return SoiModel::None;
    };
    let primary_mass = system.mass(primary);
    if mass >= primary_mass {
        return SoiModel::None; // the "primary" is the satellite; the formulas invert
    }
    if matches!(system.appearance(i), Appearance::Star(_)) {
        // Laplace assumes m << M, which a companion star violates.
        return SoiModel::Hill;
    }
    if mass / (mass + primary_mass) >= COMPARABLE_MASS_FRACTION {
        return SoiModel::Hill; // Pluto and Charon, and any binary
    }
    if system.is_major(i) {
        SoiModel::LaplaceAngled
    } else {
        SoiModel::Laplace // minor bodies: isotropic, and cheaper to draw
    }
}

/// Above this mass fraction the two-body assumption behind Laplace stops holding.
const COMPARABLE_MASS_FRACTION: f64 = 0.05;

/// How deep the hierarchy may be walked before a cycle is assumed.
const MAX_DEPTH: usize = 64;

/// The sphere of influence from the arena's current state.
///
/// The renderer's hot path: no propagation, no allocation. `None` when the body has no
/// sphere — a root, a massless body, or one whose model is [`SoiModel::None`].
pub fn soi_now(system: &System, i: BodyIndex) -> Option<Soi> {
    soi_now_with(system, i, default_model(system, i))
}

/// [`soi_now`] with the model chosen by the caller.
pub fn soi_now_with(system: &System, i: BodyIndex, model: SoiModel) -> Option<Soi> {
    if matches!(model, SoiModel::None) {
        return None;
    }
    let primary = system.parent(i)?;
    build(system, i, model, system.position(i), system.position(primary), primary,
          system.time())
}

/// The sphere of influence at an arbitrary instant.
///
/// `None` when the body has no sphere, or when it or anything in its parent chain is
/// Newtonian — integrated state has no closed form. This is what patched conics will
/// search over.
pub fn soi_at(system: &System, i: BodyIndex, time: Instant) -> Option<Soi> {
    soi_at_with(system, i, time, default_model(system, i))
}

/// [`soi_at`] with the model chosen by the caller.
pub fn soi_at_with(system: &System, i: BodyIndex, time: Instant, model: SoiModel) -> Option<Soi> {
    if matches!(model, SoiModel::None) {
        return None;
    }
    let primary = primary_at(system, i, time)?;
    let centre = propagate::position_at(system, i, time)?;
    let primary_position = propagate::position_at(system, primary, time)?;
    build(system, i, model, centre, primary_position, primary, time)
}

/// The primary in force at `time`, resolved from the motive rather than the derived parent
/// column, which is only valid for the arena's last rebuild time.
fn primary_at(system: &System, i: BodyIndex, time: Instant) -> Option<BodyIndex> {
    let (_, selection) = system.motive(i).motive_at(time);
    system.by_name(selection.primary_id()?)
}

fn build(
    system: &System,
    i: BodyIndex,
    model: SoiModel,
    centre: DVec3,
    primary_position: DVec3,
    primary: BodyIndex,
    time: Instant,
) -> Option<Soi> {
    let offset = primary_position - centre;
    let separation = offset.length();

    // The Laplace and Hill-at-periapsis forms are written in terms of the orbit, not the
    // instantaneous separation. A body that is not on a Keplerian orbit has no elements,
    // so its current distance stands in for the orbit it does not have.
    let (_, selection) = system.motive(i).motive_at(time);
    let (semi_major_axis, eccentricity) = match selection {
        MotiveSelection::Keplerian(k) => (k.semi_major_axis(), k.eccentricity()),
        _ => (separation, 0.0),
    };

    Some(Soi {
        body: i,
        model,
        centre,
        to_primary: if separation > 0.0 { offset / separation } else { DVec3::ZERO },
        separation,
        semi_major_axis,
        eccentricity,
        body_mass: system.mass(i),
        primary_mass: system.mass(primary),
        gravitational_constant: system.gravitational_constant(),
    })
}

/// The deepest body whose sphere of influence contains `point` at `time`.
pub fn containing(system: &System, point: DVec3, time: Instant) -> Option<BodyIndex> {
    containment_chain(system, point, time).last().copied()
}

/// Every body containing `point` at `time`, root first, deepest last.
///
/// A root has no primary and so no measurable sphere, but it is the thing everything else
/// orbits: its influence is taken to be unbounded, and the nearest root is entered
/// unconditionally. Below that, each level takes the first child whose sphere contains the
/// point. Empty only when the system has no bodies.
pub fn containment_chain(system: &System, point: DVec3, time: Instant) -> Vec<BodyIndex> {
    let mut chain = Vec::new();

    // Nearest root: in a single-star system this is the star, and in a multi-star one the
    // question "whose system am I in" has no better answer without a full SOI for each.
    let Some(mut current) = system.roots().min_by(|&a, &b| {
        let da = propagate::position_at(system, a, time)
            .map(|p| (p - point).length()).unwrap_or(f64::INFINITY);
        let db = propagate::position_at(system, b, time)
            .map(|p| (p - point).length()).unwrap_or(f64::INFINITY);
        da.total_cmp(&db)
    }) else {
        return chain;
    };
    chain.push(current);

    // `topological_order` tolerates a cyclic parent column, so this walk must too.
    for _ in 0..MAX_DEPTH {
        let next = system.children_of(current).find(|&child| {
            soi_at(system, child, time).is_some_and(|soi| soi.contains(point))
        });
        match next {
            Some(child) if !chain.contains(&child) => {
                chain.push(child);
                current = child;
            }
            _ => break,
        }
    }
    chain
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::solar_system;

    fn built() -> System {
        let mut system = System::from_contents(&solar_system()).expect("the bundled system builds");
        propagate::evaluate_at(&mut system, Instant::J2000);
        system
    }

    fn soi_of(system: &System, name: &str, model: SoiModel) -> Soi {
        let i = system.by_name(name).unwrap_or_else(|| panic!("no body named {name}"));
        soi_now_with(system, i, model).expect("a body with a primary has a sphere")
    }

    /// The textbook figures, reached through the `Soi` API rather than the raw formulas —
    /// and the difference between a model keyed on the live separation and one keyed on
    /// the orbit. J2000 is days from Earth's perihelion, so the Hill sphere is measurably
    /// smaller there than the 1 AU figure while the Laplace radius, which uses `a`, is not.
    #[test]
    fn earth_matches_the_textbook_radii() {
        let s = built();
        let hill = soi_of(&s, "Earth", SoiModel::Hill).bounding_radius();
        let laplace = soi_of(&s, "Earth", SoiModel::Laplace).bounding_radius();

        assert!((laplace - 9.25e8).abs() < 1.0e7, "Laplace SOI {laplace:e}");
        assert!(hill > laplace, "the Hill sphere is the larger of the two");

        const HILL_AT_ONE_AU: f64 = 1.496e9;
        assert!(hill < HILL_AT_ONE_AU, "near perihelion the Hill sphere shrinks: {hill:e}");
        assert!(hill > HILL_AT_ONE_AU * 0.98, "but only by Earth's eccentricity: {hill:e}");

        // Keyed on the orbit rather than the moment, the same body is back at the
        // 1 AU figure.
        let periapsis = soi_of(&s, "Earth", SoiModel::HillPeriapsis).bounding_radius();
        assert!(periapsis < hill, "periapsis is the smallest the sphere gets");
    }

    /// The angled model is a squash toward the primary and nothing else.
    #[test]
    fn the_angled_model_is_narrower_along_the_primary_line() {
        let s = built();
        let soi = soi_of(&s, "Earth", SoiModel::LaplaceAngled);
        let across = soi.bounding_radius();
        let along = soi.radius_toward(soi.to_primary);

        assert!(!soi.is_isotropic());
        assert!((along / across - 0.8706).abs() < 1e-3, "{along:e} / {across:e}");
        assert!((soi.min_radius() - along).abs() < 1.0);
        // Away from the primary is the same squash: it is a surface of revolution, and
        // symmetric about the plane through the centre.
        assert!((soi.radius_toward(-soi.to_primary) - along).abs() < 1.0);
        assert!((soi.radius_toward(soi.to_primary.any_orthogonal_vector()) - across).abs() < 1.0);
    }

    #[test]
    fn every_other_model_is_a_sphere() {
        let s = built();
        for model in [
            SoiModel::Hill,
            SoiModel::HillPeriapsis,
            SoiModel::Laplace,
            SoiModel::LaplaceIntegrated,
            SoiModel::Bondi { velocity_dispersion: 1.0e5 },
            SoiModel::Fixed(1.0e9),
        ] {
            let soi = soi_of(&s, "Earth", model);
            assert!(soi.is_isotropic(), "{model:?} should be isotropic");
            assert!((soi.min_radius() - soi.bounding_radius()).abs() < 1e-6, "{model:?}");
        }
    }

    /// Containment must respect the anisotropy, not just the bounding sphere.
    #[test]
    fn containment_follows_the_shape() {
        let s = built();
        let soi = soi_of(&s, "Earth", SoiModel::LaplaceAngled);
        let across = soi.to_primary.any_orthogonal_vector().normalize();

        assert!(soi.contains(soi.centre + soi.to_primary * soi.min_radius() * 0.99));
        assert!(!soi.contains(soi.centre + across * soi.bounding_radius() * 1.01));
        assert!(soi.contains(soi.centre + across * soi.bounding_radius() * 0.999));
        // The point that separates the two models: inside the sphere, outside the squash.
        let probe = soi.centre + soi.to_primary * soi.bounding_radius() * 0.95;
        assert!(!soi.contains(probe), "the bounding sphere is not the shape");
    }

    #[test]
    fn models_are_inferred_from_the_body() {
        let s = built();
        let of = |name: &str| default_model(&s, s.by_name(name).unwrap());
        assert_eq!(of("Sol"), SoiModel::None, "a root has unbounded influence, not a sphere");
        assert_eq!(of("Earth"), SoiModel::LaplaceAngled);
        assert!(soi_now(&s, s.by_name("Sol").unwrap()).is_none());
        assert!(soi_now(&s, s.by_name("Earth").unwrap()).is_some());
    }

    #[test]
    fn the_hierarchy_resolves_a_point_to_its_owner() {
        let s = built();
        let (sol, earth) = (s.by_name("Sol").unwrap(), s.by_name("Earth").unwrap());
        let t = s.time();

        let near_earth = s.position(earth) + DVec3::new(1.0e6, 0.0, 0.0);
        assert_eq!(containing(&s, near_earth, t), Some(earth));
        assert_eq!(containment_chain(&s, near_earth, t), vec![sol, earth]);

        // Interplanetary space belongs to the star.
        let deep_space = s.position(sol) + DVec3::new(7.5e10, 0.0, 0.0);
        assert_eq!(containing(&s, deep_space, t), Some(sol));
    }

    /// Nesting is the whole point: a moon's sphere sits inside its planet's.
    #[test]
    fn a_moon_owns_the_space_at_its_own_centre() {
        let s = built();
        let luna = s.by_name("Luna").expect("the bundled system has Luna");
        let t = s.time();
        let chain = containment_chain(&s, s.position(luna), t);
        assert_eq!(chain.last(), Some(&luna), "chain was {:?}",
            chain.iter().map(|&i| s.name(i)).collect::<Vec<_>>());
        assert!(chain.contains(&s.by_name("Earth").unwrap()));
    }

    /// The two constructors must not drift apart.
    #[test]
    fn the_live_and_predicted_spheres_agree_at_the_current_time() {
        let s = built();
        let earth = s.by_name("Earth").unwrap();
        let now = soi_now(&s, earth).unwrap();
        let predicted = soi_at(&s, earth, s.time()).unwrap();
        assert!((now.bounding_radius() - predicted.bounding_radius()).abs()
            < now.bounding_radius() * 1e-9);
        assert!((now.centre - predicted.centre).length() < 1.0);
    }
}
