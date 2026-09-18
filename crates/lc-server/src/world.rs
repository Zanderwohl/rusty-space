//! What the server knows: things with worldlines, and the events they have made.
//!
//! A craft is [`lc_world::craft::Craft`] — the same model the client runs, folded by the same
//! [`apply`](lc_world::motion::apply). The server used to keep its own impoverished one, a
//! point that was either still or coasting, and could therefore neither validate nor reproduce
//! anything the client actually did.
//!
//! What stays here is what is the *server's* fact rather than the world's: who owns a craft,
//! what has happened, and who is due to be told.

use std::collections::HashMap;
use std::sync::Arc;

use glam::DVec3;
use lc_proto::ShipId;
use lc_spacetime::Worldline;
use lc_world::craft::{Craft, CraftId, Kind};
use lc_world::motion::LIGHT_US_PER_LY;
use lc_world::signal::Beam;
use lc_world::sky::{CatalogueStar, StarId};
use lc_world::system::{LOCAL_SHELL_LY, LocalSystem};

/// The stars a shard is authoritative over, and the systems loaded around them.
///
/// Where the stars come from is the caller's business — a packed sky, an authored galaxy, a
/// handful in a test. The server's business is which one a craft is inside, and that is
/// [`LOCAL_SHELL_LY`] from a star and nothing more: the same rule the client uses, so the two
/// never disagree about whether a ship is in a system.
#[derive(Default)]
pub struct World {
    stars: Vec<CatalogueStar>,
    /// Loaded on first arrival and shared thereafter. Building one is a couple of hundred
    /// bodies out of a preset, and every craft in the same system points at the same copy —
    /// which is only possible because a system is never propagated.
    loaded: HashMap<StarId, Arc<LocalSystem>>,
}

impl World {
    pub fn new(stars: Vec<CatalogueStar>) -> Self {
        Self { stars, loaded: HashMap::new() }
    }

    pub fn is_empty(&self) -> bool {
        self.stars.is_empty()
    }

    /// Where a star is, by its catalogue id.
    ///
    /// The only thing that resolves an id on this side, and the reason `Order::Cross` names a
    /// star rather than a position: a client can ask for a star this shard holds and nothing
    /// else. Both ends hold the same catalogue — see the shard's `--sky`.
    pub fn star_at(&self, id: u64) -> Option<DVec3> {
        self.stars.iter().find(|s| s.id.get() == id).map(|s| s.position_ly)
    }

    /// Where a star is, by the name the catalogue knows it under.
    ///
    /// The other half of [`World::star_at`], and here for the same reason: a scene names the
    /// system it is staged in, and only this side may turn a name into a place.
    pub fn star_named(&self, name: &str) -> Option<DVec3> {
        self.stars
            .iter()
            .find(|s| s.name.as_deref() == Some(name))
            .map(|s| s.position_ly)
    }

    /// Where a craft with nowhere else to be is put.
    ///
    /// Inside the first star's system rather than at the origin, which is empty interstellar
    /// space — a new player dropped there sees nothing at all and has nothing to fly to.
    /// Offset by [`START_OFFSET_AU`] because the star's own position is inside the star.
    ///
    /// Still a crude answer to a game question, as the origin was. It is a better one because
    /// there is something to look at.
    pub fn start(&self) -> Option<DVec3> {
        let star = self.stars.first()?;
        Some(star.position_ly + DVec3::X * START_OFFSET_AU * AU_LY)
    }

    /// The system containing `position_ly`, loaded if this is the first craft to arrive.
    ///
    /// `None` between the stars, which is most of the volume and most of the flying.
    pub fn system_at(&mut self, position_ly: DVec3) -> Option<Arc<LocalSystem>> {
        let star = self
            .stars
            .iter()
            .find(|star| star.position_ly.distance(position_ly) < LOCAL_SHELL_LY)?
            .clone();
        if let Some(system) = self.loaded.get(&star.id) {
            return Some(system.clone());
        }
        let system = Arc::new(LocalSystem::for_star(&star)?);
        self.loaded.insert(star.id, system.clone());
        Some(system)
    }
}

/// Microseconds of coordinate time in one second.
pub const MICROS_PER_SECOND: i64 = 1_000_000;

/// How far from its star a new craft starts, in astronomical units. Far enough out to see the
/// system rather than be inside the star, close enough that its planets are somewhere to go.
pub const START_OFFSET_AU: f64 = 5.0;

/// Light-years in an astronomical unit.
const AU_LY: f64 = 1.495_978_707e11 / 9.460_730_472_580_8e15;

/// A craft at rest at a point, light-microseconds from the world origin.
pub fn still(id: ShipId, at: DVec3) -> Craft {
    Craft::at(CraftId(id.0), Kind::Ship, at / LIGHT_US_PER_LY)
}

/// Moving through `at` at `beta`, from coordinate microsecond `epoch_us` onward.
pub fn coasting(id: ShipId, at: DVec3, beta: DVec3, epoch_us: i64) -> Craft {
    let mut craft = still(id, at);
    craft.motion.beta = beta;
    craft.motion.set_adrift(epoch_us as f64 * 1.0e-6);
    craft
}

/// Something that happened, at a coordinate.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: i64,
    /// The ship it happened to.
    pub source: ShipId,
    /// Coordinate microseconds.
    pub t: i64,
    /// Where, light-microseconds. Taken from the source's worldline at `t` and stored, because
    /// the worldline will have changed by the time anyone is told.
    pub at: DVec3,
    pub kind: i16,
    /// What the source put out, for the inverse-square falloff. Zero for anything silent.
    pub power_w: f64,
    pub payload: String,
}

/// An event on its way to an observer, with the time it gets there.
///
/// Computed when the event is written, not when it is read. That is what makes the tick-time
/// check a range scan rather than a pass over everything that has ever happened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scheduled {
    pub observer: ShipId,
    pub event: i64,
    pub arrive_t: i64,
    pub strength: f32,
}

/// Strength at the receiver: power over the square of the distance travelled, with the
/// distance floored so a coincident source is loud rather than infinite.
pub fn strength(power_w: f64, distance: f64) -> f32 {
    (power_w / distance.max(1.0).powi(2)) as f32
}

/// When an event's light reaches a ship, and how strong it is when it does.
///
/// `None` when it never does — the ship's worldline ends first, the light already went past
/// before it began, or the observer is **off the beam**. The third is why a beam is a parameter
/// here rather than a strength multiplier applied afterwards: a receiver outside the cone gets
/// no delivery row at all, which is what keeps a tight beam's fan-out smaller than a shout's
/// rather than merely quieter.
///
/// The cone is tested against where the observer is when the light **arrives**, not where it
/// was when the light left. Those differ by however far the receiver moved during the flight,
/// which across a system is a great deal — and it is the arrival that decides whether the
/// signal fell on them.
pub fn schedule(event: &Event, beam: &Beam, observer: &Craft) -> Option<Scheduled> {
    let line = observer.worldline();
    let arrive = lc_spacetime::arrival_time_at(event.t as f64, event.at, &line)?;
    // Rounded up. Rounding down would put an arrival a microsecond before its true time, and
    // the gate would then release it a microsecond early -- which is the one error this whole
    // system exists to prevent, however small.
    let arrive_t = arrive.ceil() as i64;
    let offset = line.position_at(arrive) - event.at;
    if !beam.covers(offset) {
        return None;
    }
    Some(Scheduled {
        observer: ShipId(observer.id.0),
        event: event.id,
        arrive_t,
        // The same power, concentrated. A beam is not a bigger transmitter: it is the same
        // watts through a smaller solid angle, which is why the gain is the *only* thing that
        // changes and the inverse square underneath it does not.
        strength: strength(event.power_w * beam.gain(), offset.length()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(id: i64, at: DVec3) -> Craft {
        still(ShipId(id), at)
    }

    fn pulse(t: i64, at: DVec3, power_w: f64) -> Event {
        Event { id: 1, source: ShipId(0), t, at, kind: 1, power_w, payload: "{}".into() }
    }

    /// A light-microsecond of distance is a microsecond of delay, by definition. This is the
    /// unit the whole protocol is in, so it is worth pinning.
    #[test]
    fn a_light_microsecond_of_distance_is_a_microsecond_of_delay() {
        let observer = ship(2, DVec3::new(1_000_000.0, 0.0, 0.0));
        let sent = pulse(5_000, DVec3::ZERO, 1.0);
        let scheduled = schedule(&sent, &Beam::OMNI, &observer).expect("it arrives");
        assert_eq!(scheduled.arrive_t, 1_005_000);
        assert_eq!(scheduled.observer, ShipId(2));
    }

    /// An event at the observer's own position arrives when it happens, and not before.
    #[test]
    fn an_event_at_the_observer_arrives_at_once() {
        let observer = ship(2, DVec3::ZERO);
        let scheduled = schedule(&pulse(7_777, DVec3::ZERO, 1.0), &Beam::OMNI, &observer).unwrap();
        assert_eq!(scheduled.arrive_t, 7_777);
    }

    /// Rounded up, never down. A rounding that put an arrival a microsecond early would make
    /// the gate release it a microsecond early, which is the one failure that matters.
    #[test]
    fn an_arrival_is_never_rounded_earlier_than_it_is() {
        let observer = ship(2, DVec3::new(1_000.5, 0.0, 0.0));
        let scheduled = schedule(&pulse(0, DVec3::ZERO, 1.0), &Beam::OMNI, &observer).unwrap();
        assert_eq!(scheduled.arrive_t, 1_001, "1000.5 became {}", scheduled.arrive_t);
        assert!(scheduled.arrive_t as f64 >= 1_000.5);
    }

    /// A moving observer meets the light earlier or later than a still one at the same place,
    /// and the solve is what says which.
    #[test]
    fn a_moving_observer_meets_the_light_at_a_different_time() {
        let at = DVec3::new(1_000_000.0, 0.0, 0.0);
        let still = ship(2, at);
        // Falling toward the source at a tenth of `c`, from the same place at the same time.
        let closing = coasting(ShipId(3), at, DVec3::new(-0.1, 0.0, 0.0), 0);

        let sent = pulse(0, DVec3::ZERO, 1.0);
        let a = schedule(&sent, &Beam::OMNI, &still).unwrap().arrive_t;
        let b = schedule(&sent, &Beam::OMNI, &closing).unwrap().arrive_t;
        assert!(b < a, "closing on the source should meet its light sooner: {b} against {a}");
        // Closing at beta, the meeting is at d/(1+beta).
        assert!((b as f64 - 1_000_000.0 / 1.1).abs() < 2.0, "{b}");
    }

    /// The point of the whole exercise: a ship whose motion is a *station in a star system*
    /// schedules its arrivals from where it actually is when the light gets there.
    ///
    /// Under the old model this ship was a point that was either still or coasting, so an
    /// orbiting receiver was scheduled as though it were parked. The error is the chord the
    /// ship covers during the light delay, and here that is most of an orbit.
    #[test]
    fn a_ship_in_orbit_is_scheduled_along_its_orbit() {
        use lc_world::navigation::{Course, Plane};
        use lc_world::sky::{AuthoredStars, StarProvider};

        let sky = AuthoredStars::sample();
        let star = sky.stars().first().expect("a star").clone();
        let Some(system) = lc_world::system::LocalSystem::for_star(&star) else { return };
        let Some(body) = system.inventory().iter().find_map(|e| match &e.target {
            lc_world::navigation::Target::Body(name) => Some(name.clone()),
            _ => None,
        }) else {
            return;
        };

        let course = Course::Orbit { body, altitude_radii: 2.0, plane: Plane::Equatorial };
        let Some(waypoint) = course.resolve(&system, system.star_position_ly(), 0.0) else { return };
        let mut motion = lc_world::motion::ShipState::at(system.star_position_ly());
        motion.begin_holding(waypoint);

        let mut orbiting = Craft::at(CraftId(2), Kind::Ship, DVec3::ZERO);
        orbiting.motion = motion.clone();
        orbiting.enter(Some(std::sync::Arc::new(system)), 0.0);
        // The same ship, frozen where it was at t = 0: what the old model could represent.
        let parked = still(ShipId(3), orbiting.position_at(0.0));

        // From far enough away that the delay is many orbits.
        let far = orbiting.position_at(0.0) + DVec3::new(5.0e9, 0.0, 0.0);
        let sent = pulse(0, far, 1.0);
        let moving = schedule(&sent, &Beam::OMNI, &orbiting).expect("it arrives");
        let still_there = schedule(&sent, &Beam::OMNI, &parked).expect("it arrives");

        // Both are about the light-crossing time, and they are not the same instant.
        assert!(moving.arrive_t > 1.0e9 as i64, "{}", moving.arrive_t);
        // Most of a second apart, which is what the old model got wrong. The gate it feeds
        // measures in microseconds.
        let slip = (moving.arrive_t - still_there.arrive_t).abs();
        assert!(
            slip > 100_000,
            "an orbiting receiver was scheduled {slip} microseconds from a parked one",
        );

        // And the moving one is right: at its own arrival, it is exactly a light-delay away.
        let meeting = orbiting.position_at(moving.arrive_t as f64);
        let gap = (meeting - far).length();
        assert!(
            (gap - moving.arrive_t as f64).abs() < 2.0,
            "{gap} light-microseconds away at t = {}",
            moving.arrive_t,
        );
    }

    /// A beam is a cone, and a receiver off its axis gets no delivery row at all. Not a faint
    /// one — none, which is the property that bounds a beam's fan-out.
    #[test]
    fn a_receiver_off_the_beam_is_not_scheduled_at_all() {
        let sent = pulse(0, DVec3::ZERO, 1.0);
        let beam = Beam::along(DVec3::X, 0.05);
        let on_axis = ship(2, DVec3::new(1_000_000.0, 0.0, 0.0));
        let beside = ship(3, DVec3::new(1_000_000.0, 200_000.0, 0.0));

        assert!(schedule(&sent, &beam, &on_axis).is_some());
        assert!(schedule(&sent, &beam, &beside).is_none(), "0.2 rad off a 0.05 rad beam");
        // The same two, shouted at: both hear it. The beam is what excluded one of them.
        assert!(schedule(&sent, &Beam::OMNI, &beside).is_some());
    }

    /// The gain is the same watts through a smaller solid angle, so on axis a beam is louder
    /// by exactly the ratio of the sphere to the cone — and the inverse square underneath it
    /// is untouched.
    #[test]
    fn a_beam_is_louder_on_axis_by_its_gain_and_no_more() {
        let sent = pulse(0, DVec3::ZERO, 1.0);
        let observer = ship(2, DVec3::new(1_000_000.0, 0.0, 0.0));
        let beam = Beam::along(DVec3::X, 1.0e-3);
        let loud = schedule(&sent, &beam, &observer).unwrap().strength;
        let quiet = schedule(&sent, &Beam::OMNI, &observer).unwrap().strength;
        assert!((f64::from(loud / quiet) - beam.gain()).abs() / beam.gain() < 1.0e-6);
    }

    /// Aimed at where the receiver *will be*, not where it was. A beam laid on the old position
    /// of a ship that has been moving for the whole flight time misses it, and that is the
    /// mechanic — so the test that the arrival position is what the cone is judged against has
    /// to be the one that fails when the two are swapped.
    #[test]
    fn the_cone_is_judged_where_the_receiver_is_when_the_light_lands() {
        let away = DVec3::new(1_000_000.0, 0.0, 0.0);
        // Crossing the beam's path at a fifth of `c`: a light-second of flight moves it
        // 200 000 light-microseconds sideways, far outside a milliradian.
        let crossing = coasting(ShipId(2), away, DVec3::new(0.0, 0.2, 0.0), 0);
        let sent = pulse(0, DVec3::ZERO, 1.0);

        let at_it = Beam::along(away, 1.0e-3);
        assert!(schedule(&sent, &at_it, &crossing).is_none(), "aimed where it was seen");

        // Led: the advanced solve says where it will be, and that beam lands.
        let axis =
            lc_world::signal::aim_at(DVec3::ZERO, 0.0, away, DVec3::new(0.0, 0.2, 0.0), 0.0)
                .unwrap();
        assert!(schedule(&sent, &Beam::along(axis, 1.0e-3), &crossing).is_some(), "led");
    }

    #[test]
    fn strength_falls_as_the_inverse_square_and_does_not_divide_by_zero() {
        assert_eq!(strength(100.0, 10.0), 1.0);
        assert_eq!(strength(100.0, 20.0), 0.25);
        assert!(strength(100.0, 0.0).is_finite(), "a coincident source is loud, not infinite");
    }
}
