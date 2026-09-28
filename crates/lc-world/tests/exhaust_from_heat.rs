//! A burn's exhaust comes from the field's heat first and from storage for the rest.
//! `lightcone/docs/31-directed-energy.md` §The drive is the radiator.

use glam::DVec3;
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::fitting::{Account, Balance, Fitting};
use lc_world::form::Form;
use lc_world::motion::{Change, Event, Motive, ShipId};

/// No radiation, drain or starlight, so the exhaust is all that leaves.
fn quiet() -> Balance {
    Balance { living_density_w: 0.0, field_tau_s: 1.0e40, ..Balance::DEFAULT }
}

fn ship(stored_j: f64, heat_j: f64) -> Craft {
    let b = quiet();
    let full = Fitting::full(Form::starting(), b, 0.0);
    let mut craft = Craft::at(CraftId(9), Kind::Ship, DVec3::ZERO);
    craft.fit(Some(Fitting::from_account(&Account { stored_j, heat_j, ..full.account() }, b)));
    craft
}

fn capacity_j() -> f64 {
    Fitting::full(Form::starting(), quiet(), 0.0).hull().capacities.storage_j
}

fn cross(craft: &mut Craft) -> f64 {
    let event = Event {
        ship: ShipId(9),
        at_t: 0.0,
        change: Change::Cross { to_ly: DVec3::X * (lc_world::flight::STANDOFF_LY + 1.0e-4), drive: craft.rated_drive(0.0) },
    };
    let quoted = craft.cost_of(&event).unwrap();
    craft.apply(&event).unwrap();
    quoted
}

fn duration_s(craft: &Craft) -> f64 {
    let Motive::Crossing(cruise) = &craft.motion.motive else { panic!("not crossing") };
    cruise.duration_s()
}

fn heat_j(craft: &Craft, t: f64) -> f64 {
    craft.fitting().unwrap().heat_j_at(&craft.motion, t)
}

fn stored_j(craft: &Craft, t: f64) -> f64 {
    craft.fitting().unwrap().stored_j_at(&craft.motion, t)
}

/// A hot ship and a twin holding the same energy all in storage weigh the same, are quoted the
/// same and fly the same. The hot one pays storage the quote less its heat, and ends as light as
/// the twin: heat is mass, and the rocket law does not ask where the exhaust came from.
#[test]
fn a_hot_crossing_arrives_colder_lighter_and_where_the_plan_said() {
    let capacity_j = capacity_j();
    let mut twin = ship(capacity_j, 0.0);
    let quoted = cross(&mut ship(capacity_j, 0.0));
    let heat_0 = 0.4 * quoted;
    let mut hot = ship(capacity_j - heat_0, heat_0);
    let mass_kg = hot.mass_kg_at(0.0);
    assert_eq!(mass_kg, twin.mass_kg_at(0.0), "premise");
    assert_eq!(cross(&mut hot), cross(&mut twin));
    let Motive::Crossing(cruise) = hot.motion.motive.clone() else { panic!("not crossing") };
    let end_s = duration_s(&hot);

    let n = 500;
    for k in 1..=n {
        let t = (end_s + 1.0) * f64::from(k) / f64::from(n);
        let dt = (end_s + 1.0) / f64::from(n);
        hot.advance(t, dt);
        twin.advance(t, dt);
    }
    let t = end_s + 1.0;
    assert!(!hot.motion.is_under_way(), "{:?}", hot.motion.motive);
    assert!(hot.motion.position_ly.distance(cruise.to_ly) < 1.0e-12, "{:?} {:?}", hot.motion.position_ly, cruise.to_ly);
    assert_eq!(hot.motion.position_ly, twin.motion.position_ly);

    assert_eq!(heat_j(&hot, t), 0.0, "colder: all of it went out");
    let paid_j = (capacity_j - heat_0) - stored_j(&hot, t);
    assert!((paid_j - (quoted - heat_0)).abs() <= 1.0e-9 * quoted, "{paid_j} {}", quoted - heat_0);
    let lighter_kg = mass_kg - quoted / lc_world::fitting::C2;
    assert!(hot.mass_kg_at(t) < mass_kg);
    assert!((hot.mass_kg_at(t) - twin.mass_kg_at(t)).abs() <= 1.0e-12 * mass_kg, "{} {}", hot.mass_kg_at(t), twin.mass_kg_at(t));
    assert!((hot.mass_kg_at(t) - lighter_kg).abs() <= 1.0e-9 * (mass_kg - lighter_kg), "{} {lighter_kg}", hot.mass_kg_at(t));
}

/// What heat pays leaves the commitment as the burn goes without leaving storage, and a cut
/// releases the rest: nothing is ever handed back to storage, because heat's share never left.
#[test]
fn cutting_a_hot_burn_releases_what_heat_paid_as_a_cut_does() {
    let stored_0 = 0.5 * capacity_j();
    let quoted = cross(&mut ship(stored_0, 0.0));
    let (mut hot, mut cold) = (ship(stored_0, 0.5 * quoted), ship(stored_0, 0.0));
    let heat_0 = 0.5 * quoted;
    let quoted_hot = cross(&mut hot);
    cross(&mut cold);
    let cut_s = 0.25 * duration_s(&hot);
    let spent_j = stored_0 - stored_j(&cold, cut_s);
    assert!(spent_j > 0.2 * quoted && spent_j < heat_0, "premise: heat has paid for all of it so far: {spent_j}");

    let free_j = hot.free_j_at(cut_s);
    assert_eq!(stored_j(&hot, cut_s), stored_0, "storage paid nothing");
    assert!(free_j > stored_0 - quoted_hot, "heat's share released as it was spent: {free_j}");
    assert!((heat_0 - heat_j(&hot, cut_s) - (free_j - (stored_0 - quoted_hot))).abs() <= 1.0e-9 * quoted);

    for craft in [&mut hot, &mut cold] {
        craft.apply(&Event { ship: ShipId(9), at_t: cut_s, change: Change::CutDrive }).unwrap();
        assert_eq!(craft.fitting().unwrap().committed_j_at(&craft.motion, cut_s), 0.0);
    }
    assert_eq!(stored_j(&hot, cut_s), stored_0);
    assert_eq!(hot.free_j_at(cut_s), stored_0, "all of it free, and none of it granted");
    // The cold ship paid storage what the hot one's heat paid.
    let ahead_j = hot.free_j_at(cut_s) - cold.free_j_at(cut_s);
    assert!((ahead_j - spent_j).abs() <= 1.0e-9 * spent_j, "{ahead_j} {spent_j}");
}
