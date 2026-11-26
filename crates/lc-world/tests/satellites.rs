//! Generated satellites in a loaded system: em-sim propagates a body whose parent is a moon, and
//! a generated ring is something a ship can be sent above.

use lc_world::sky::generate::system_for;
use lc_world::sky::{AuthoredStars, CatalogStar, StarId, StarProvider};
use lc_world::system::LocalSystem;

const M_PER_LY: f64 = 9.460_730_472_580_8e15;

/// After a year of propagation a submoon is still within its moon's Hill sphere, and is not on
/// top of the moon: it orbits the moon rather than the planet or the star.
#[test]
fn a_submoon_stays_with_its_moon_through_propagation() {
    let base = AuthoredStars::sample().stars()[1].clone();
    let (star, planet, moon, sub) = (0..2000u64)
        .find_map(|k| {
            let star = CatalogStar { id: StarId::synthesize("submoon", k), ..base.clone() };
            let system = system_for(&star);
            let found = system.planets.iter().find_map(|p| p.moons.iter().find_map(|m| Some((p.clone(), m.clone(), m.moons.first()?.clone()))));
            found.map(|(p, m, s)| (star, p, m, s))
        })
        .expect("a Sun-like star with a submoon among two thousand");
    let system = LocalSystem::for_star(&star).expect("the system loads").propagated_to(3.15e7);
    let at = |name: &str| system.body_position_ly(name).unwrap_or_else(|| panic!("{name} is in the system")) * M_PER_LY;
    let apart = at(&sub.name).distance(at(&moon.name));
    let hill = moon.semi_major_m * (moon.mass_kg / (3.0 * planet.mass_kg)).cbrt();
    assert!(apart < hill && apart > moon.radius_m, "{} is {apart:.3e} m from {}, Hill radius {hill:.3e} m", sub.name, moon.name);
}

/// A generated planet's rings reach the loaded system: its drawable carries them, and
/// navigation offers an orbit above them that resolves outside the outermost band.
#[test]
fn a_generated_ring_can_be_flown_to() {
    use lc_world::navigation::{Course, Target, Waypoint, options_for};
    let base = AuthoredStars::sample().stars()[1].clone();
    let (star, planet) = (0..500u64)
        .find_map(|k| {
            let star = CatalogStar { id: StarId::synthesize("ringed", k), ..base.clone() };
            let planet = system_for(&star).planets.into_iter().find(|p| p.rings.is_some())?;
            Some((star, planet))
        })
        .expect("a Sun-like star with a ringed planet among five hundred");
    let system = LocalSystem::for_star(&star).expect("the system loads");
    let index = system.body_named(&planet.name).expect("the planet is in the system");
    let rings = system.rings_of(index).expect("its rings reached the loaded system");
    assert_eq!(**rings, planet.rings.clone().unwrap());
    let options = options_for(&system, &Target::Body(planet.name.clone()));
    let (_, course) = options.iter().find(|(_, c)| matches!(c, Course::Rings(_))).expect("above the rings is offered");
    let Some(Waypoint::Orbit(orbit)) = course.resolve(&system, star.position_ly, 0.0) else { panic!("rings are an orbit") };
    assert!(orbit.radius_m > rings.outer_m());
}
