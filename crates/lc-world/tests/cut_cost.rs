//! What a frame costs after the drive is cut.
//!
//! A player reported the page freezing after cutting the drive a few times, with a round trip
//! of fifteen milliseconds — so the cost is on the client, in the frames after the fold, and
//! not on the wire. This times exactly that.

use std::time::Instant as Clock;

use glam::DVec3;
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::motion::{self, Change, Event, Motive, ShipId};
use lc_world::sky::{AuthoredStars, StarProvider};
use lc_world::system::LocalSystem;

/// Sixty frames at the design rate, which is a second of play.
const FRAMES: usize = 60;
const STEP_S: f64 = 8766.0 / 60.0;

fn a_craft_in_a_system() -> Craft {
    let star = AuthoredStars::sample().stars().first().cloned().expect("a star");
    let system = LocalSystem::for_star(&star).expect("a system");
    let mut craft = Craft::at(CraftId(1), Kind::Ship, star.position_ly);
    craft.enter(Some(std::sync::Arc::new(system)), 0.0);
    craft
}

#[test]
fn a_frame_after_a_cut_is_not_a_search() {
    let mut craft = a_craft_in_a_system();

    // Out at a belt's radius rather than at the star's center, and moving at roughly circular
    // speed. At the center the conic is degenerate and the craft goes adrift, which skips the
    // whole patched-conic path — so a test written there measures nothing and says 99 ns.
    const AU_LY: f64 = 1.495_978_707e11 / 9.460_730_472_580_8e15;
    let origin = craft.motion.position_ly;
    craft.motion.position_ly = origin + DVec3::X * 0.5 * AU_LY;
    // ~42 km/s, a circular orbit at half an AU of a sun-like mass, as a fraction of c.
    craft.motion.beta = DVec3::new(0.0, 42_000.0 / 299_792_458.0, 0.0);
    // The motive is what `state_at` reads, and the cut reads it — so setting the fields alone
    // leaves the old motive to overwrite them on the way through.
    craft.motion.set_adrift(0.0);
    let _ = motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event { ship: ShipId(0), at_t: 0.0, change: Change::CutDrive },
    );
    assert!(
        matches!(craft.motion.motive, Motive::Falling(_)),
        "premise: a cut at a belt leaves the craft on a conic, not adrift — {:?}",
        craft.motion.motive,
    );

    let started = Clock::now();
    let mut now = 0.0;
    for _ in 0..FRAMES {
        now += STEP_S;
        craft.advance(now, STEP_S);
    }
    let each = started.elapsed() / FRAMES as u32;

    println!("a frame after a cut costs {each:?}");
    // A frame is sixteen milliseconds. Anything approaching that is the frame, not a step in it.
    assert!(
        each < std::time::Duration::from_millis(4),
        "a frame after a cut costs {each:?}, which is most of a frame",
    );
}

/// The state the report actually describes: a cut part-way through a crossing **toward a star
/// you are already near**, which leaves the craft falling through a system at a large fraction
/// of `c`.
///
/// A craft that fast leaves one sphere of influence and enters another every frame, and each
/// of those is a repatch, and each repatch solves for the next crossing over a horizon of
/// several revolutions. That is a search per frame rather than a step per frame.
#[test]
fn a_frame_after_a_cut_at_speed_is_not_a_search() {
    let mut craft = a_craft_in_a_system();

    const AU_LY: f64 = 1.495_978_707e11 / 9.460_730_472_580_8e15;
    let origin = craft.motion.position_ly;
    craft.motion.position_ly = origin + DVec3::X * 5.0 * AU_LY;
    // Two tenths of `c`, inbound. Two percent into a four light-year crossing is around here.
    craft.motion.beta = DVec3::new(-0.2, 0.0, 0.0);
    craft.motion.set_adrift(0.0);
    let _ = motion::apply(
        &mut craft.motion,
        craft.system.as_deref(),
        &Event { ship: ShipId(0), at_t: 0.0, change: Change::CutDrive },
    );

    let started = Clock::now();
    let mut now = 0.0;
    for _ in 0..FRAMES {
        now += STEP_S;
        craft.advance(now, STEP_S);
    }
    let each = started.elapsed() / FRAMES as u32;

    println!("a frame after a cut at 0.2c costs {each:?}  (motive {:?})", 
        std::mem::discriminant(&craft.motion.motive));
    assert!(
        each < std::time::Duration::from_millis(4),
        "a frame after a cut at speed costs {each:?}, which is most of a frame",
    );
}
