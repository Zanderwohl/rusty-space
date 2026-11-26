//! A submoon in a loaded system: em-sim propagates a body whose parent is a moon.

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
