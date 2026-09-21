//! Moving between two stations about one body.
//!
//! A crossing is a straight line in the world, and that is the wrong shape for going from one
//! orbit of Earth to another. Earth runs at thirty kilometers a second; over the twenty minutes
//! such a transfer takes it covers thirty thousand kilometers, which is the whole size of the
//! orbits involved. Planned in the world frame the destination is *fleeing*, the ship's own
//! co-motion is thrown away by the match, and the arrival time has no fixed point to find — see
//! [`crate::navigation::plan`], which lands a transfer about Earth tens of thousands of
//! kilometers out and cannot be damped into landing any closer.
//!
//! So it is planned in the body's frame instead, where nothing is fleeing: the station goes
//! round its orbit at a few kilometers a second and the brachistochrone
//! [`crate::flight`] already solves comes out as a transfer. The same idea as
//! [`crate::pursuit`], which plans in a quarry's frame for the same reason.
//!
//! **The frame is tracked, not anchored.** A pursuer has only the light that has reached it, so
//! a rendezvous is pinned to a sighting and dead-reckoned forward. A body is not like that: both
//! ends hold the same system and can place it analytically at any time, so this stores *which
//! body* and asks. The body's own acceleration therefore costs nothing — no drift accumulates
//! against a frozen velocity — which matters, because over a transfer about Earth a frozen one
//! would be five kilometers out by arrival.
//!
//! **Galilean, and bounded to where that is true.** The frame's speed is a body's orbital speed:
//! fifty kilometers a second at the very most, which is under two parts in ten thousand of `c`.
//! Composing velocities by adding them is then exact to a part in a hundred million, and the
//! aberration of the nose is twenty arcseconds. [`crate::pursuit`] needs the real boost because a
//! chase can run at `0.99c`; this cannot, because it only exists inside a sphere of influence.
//!
//! **What rides the frame is riding a falling body.** A body's frame is not inertial, so a ship
//! holding a constant proper acceleration in it is really holding that plus whatever keeps it
//! falling alongside — six milligravities near Earth, against the five gravities of the drive.
//! That is the same bookkeeping [`crate::motion::thrust_g`] already waves away for a ship holding
//! a station, and it is why this is offered only inside the sphere of influence: further out, the
//! difference between the ship's fall and the body's stops being a rounding error.

use glam::DVec3;

use crate::flight::{Aim, Cruise, Drive, FlightState};
use crate::navigation::{ARRIVAL_ROUNDS, Anchor, Waypoint};
use crate::system::{LocalSystem, M_PER_LY};

/// A crossing flown in a body's frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Transfer {
    /// The crossing, in the body's frame: positions relative to the body, velocities relative to
    /// its motion. Its clock is the world's — the frame is Galilean, so there is no second one.
    pub cruise: Cruise,
    /// The body the frame rides, by name, so each end resolves it against its own system.
    pub about: String,
}

impl Transfer {
    /// Where the body is and how fast, at a coordinate time: the frame itself.
    pub fn frame_at(&self, system: &LocalSystem, now_s: f64) -> Option<(DVec3, DVec3)> {
        let index = system.body_named(&self.about)?;
        let (at_m, velocity) = system.body_state_at(index, now_s)?;
        Some((
            system.origin_ly + at_m / M_PER_LY,
            crate::coast::beta_of(velocity),
        ))
    }

    /// Where the ship is and how fast, in world coordinates.
    pub fn state_at(&self, system: &LocalSystem, now_s: f64) -> Option<(DVec3, DVec3)> {
        let (at, beta) = self.frame_at(system, now_s)?;
        let flight = self.cruise.at(now_s);
        Some((at + flight.position_ly, beta + flight.beta))
    }

    /// The crossing itself, which is already in the frame's own terms.
    pub fn flight_at(&self, now_s: f64) -> FlightState {
        self.cruise.at(now_s)
    }

    /// What the plan is asking the nose to do.
    ///
    /// The frame's directions unchanged. Aberration at a body's orbital speed is twenty
    /// arcseconds, which is below the angle a hull subtends at any range it can be seen from.
    pub fn aim_at(&self, now_s: f64) -> Aim {
        self.cruise.aim_at(now_s)
    }

    pub fn thrust_at(&self, now_s: f64) -> DVec3 {
        self.cruise.thrust_at(now_s)
    }

    pub fn has_arrived(&self, now_s: f64) -> bool {
        self.cruise.has_arrived(now_s)
    }

    pub fn duration_s(&self) -> f64 {
        self.cruise.duration_s()
    }
}

/// The body a course would be flown about, if it is one of these at all.
///
/// Both halves have to hold: the destination is an orbit of a body, and the ship is *already*
/// inside that body's sphere of influence. Outside it the body's frame is not one the ship is
/// falling with — see the module docs — and a world-frame crossing is the honest answer even
/// though it arrives less accurately.
///
/// The sphere is asked for directly rather than through
/// [`em_sim::influence::containing`], which answers "whose system is this point in" and so names
/// the star for a point anywhere at all — a root's influence has no outer edge. That is the
/// right answer to its own question and the wrong one to this: it turned a four-light-year
/// crossing into a transfer about the star it was leaving. A body with no bounded sphere gets no
/// transfer, which is also correct for a star, since a star does not run away from its own
/// system.
pub fn primary_for(
    system: &LocalSystem,
    waypoint: &Waypoint,
    from_ly: DVec3,
    now_s: f64,
) -> Option<String> {
    let Waypoint::Orbit(orbit) = waypoint else {
        return None;
    };
    let Anchor::Body(name) = &orbit.about else {
        return None;
    };
    let index = system.body_named(name)?;
    let soi = em_sim::influence::soi_at(
        system.sim(),
        index,
        em_foundations::time::Instant::from_seconds_since_j2000(now_s),
    )?;
    soi.contains((from_ly - system.origin_ly) * M_PER_LY)
        .then(|| name.clone())
}

/// Plan a transfer to where a station about `about` will be, in that body's frame.
///
/// The same fixed point [`crate::navigation::plan`] solves, and it converges here because the
/// target is no longer running away: in the frame the station only goes round its orbit, a few
/// per cent of a circle over the time the transfer takes.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    system: &LocalSystem,
    about: &str,
    waypoint: &Waypoint,
    from_ly: DVec3,
    beta0: DVec3,
    attitude0: DVec3,
    start_s: f64,
    drive: Drive,
) -> Option<(Transfer, Waypoint)> {
    let index = system.body_named(about)?;
    let frame_at = |t: f64| {
        system
            .body_state_at(index, t)
            .map(|(at_m, v)| (system.origin_ly + at_m / M_PER_LY, crate::coast::beta_of(v)))
    };
    let (started_at, started_beta) = frame_at(start_s)?;
    // What the ship is doing relative to the body, which is all the frame cares about.
    let from_rel = from_ly - started_at;
    let beta_rel = beta0 - started_beta;

    let mut aimed = waypoint.clone();
    let mut cruise = None;
    let mut arrival_s = start_s;
    for _ in 0..=ARRIVAL_ROUNDS {
        let (body_ly, body_beta) = frame_at(arrival_s)?;
        // Which point of the orbit is nearest is asked about where the ship will be *if it
        // simply rides along*, not about where it stands in the world. In the world the body
        // has run out from under it and the near side of the orbit is on the wrong side.
        aimed = waypoint.nearest_to(body_ly + from_rel, system, arrival_s);
        let to_rel = aimed.place_at(system, arrival_s)? - body_ly;
        let onto_rel = crate::coast::beta_of(aimed.velocity_at(system, arrival_s)?) - body_beta;
        let next = Cruise::plan_onto(
            from_rel, beta_rel, to_rel, onto_rel, attitude0, start_s, drive,
        );
        arrival_s = start_s + next.duration_s();
        cruise = Some(next);
    }
    Some((
        Transfer {
            cruise: cruise?,
            about: about.to_string(),
        },
        aimed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flight::C_M_S;
    use crate::navigation::{Course, Plane};

    fn sol() -> LocalSystem {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .expect("the catalogue");
        let sun = crate::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))
            .expect("the Sun")
            .clone();
        let mut system = LocalSystem::for_star(&sun).expect("the solar system");
        system.advance_to(0.0);
        system
    }

    fn station(system: &LocalSystem, body: &str, altitude_radii: f64) -> Waypoint {
        Course::Orbit {
            body: body.into(),
            altitude_radii,
            plane: Plane::Equatorial,
        }
        .resolve(system, DVec3::ZERO, 0.0)
        .expect("an orbit")
    }

    /// Where a transfer starts: on a station about the body, moving with it.
    fn departing(system: &LocalSystem, from: &Waypoint) -> (DVec3, DVec3) {
        (
            from.place_at(system, 0.0).unwrap(),
            crate::coast::beta_of(from.velocity_at(system, 0.0).unwrap()),
        )
    }

    /// **The case the world frame could not fly.** Earth runs a whole orbit radius during this
    /// transfer, which is what defeated [`crate::navigation::plan`]; in Earth's own frame it is
    /// an unremarkable hop and lands on the station.
    #[test]
    fn a_transfer_about_earth_lands_where_the_world_frame_could_not() {
        let system = sol();
        let (low, high) = (
            station(&system, "Earth", 0.5),
            station(&system, "Earth", 4.0),
        );
        let (from, beta0) = departing(&system, &low);
        let (transfer, aimed) = plan(
            &system,
            "Earth",
            &high,
            from,
            beta0,
            DVec3::ZERO,
            0.0,
            Drive::DEFAULT,
        )
        .expect("a transfer");

        let arrival_s = transfer.duration_s();
        let (at, beta) = transfer.state_at(&system, arrival_s).expect("a place");
        let wanted = aimed.place_at(&system, arrival_s).unwrap();
        let miss_m = at.distance(wanted) * M_PER_LY;
        // Millimeters, against the forty-five thousand kilometers the world frame misses by.
        assert!(miss_m < 1.0, "missed by {miss_m:e} m");

        // Earth really did run the whole way across the orbit while this was flown, which is the
        // premise: the world frame has no fixed point to find here.
        let ran_m = beta0.length() * C_M_S * arrival_s;
        let radius_m = match &high {
            Waypoint::Orbit(orbit) => orbit.radius_m,
            _ => unreachable!("an orbit"),
        };
        assert!(
            ran_m > radius_m,
            "premise: Earth ran {ran_m:e} m against a {radius_m:e} m orbit"
        );

        // And it arrives *on* the station rather than beside it.
        let joining = crate::coast::beta_of(aimed.velocity_at(&system, arrival_s).unwrap());
        let short_m_s = (beta - joining).length() * C_M_S;
        assert!(
            short_m_s < 1.0e-3,
            "arrived {short_m_s} m/s off the station's velocity"
        );
    }

    /// The frame is the body, so the ship is carried by it the whole way rather than watching it
    /// leave. Halfway through the transfer the ship is still within a few orbit radii of Earth.
    #[test]
    fn the_ship_is_carried_along_rather_than_left_behind() {
        let system = sol();
        let (low, high) = (
            station(&system, "Earth", 0.5),
            station(&system, "Earth", 4.0),
        );
        let (from, beta0) = departing(&system, &low);
        let (transfer, _) = plan(
            &system,
            "Earth",
            &high,
            from,
            beta0,
            DVec3::ZERO,
            0.0,
            Drive::DEFAULT,
        )
        .expect("a transfer");

        let radius_m = match &high {
            Waypoint::Orbit(orbit) => orbit.radius_m,
            _ => unreachable!("an orbit"),
        };
        for k in 0..=20 {
            let t = transfer.duration_s() * k as f64 / 20.0;
            let (at, _) = transfer.state_at(&system, t).expect("a place");
            let earth = system.body_position_at("Earth", t).unwrap();
            let out_m = at.distance(earth) * M_PER_LY;
            assert!(out_m < radius_m * 2.0, "{out_m:e} m from Earth at {t} s");
        }
    }

    /// Only inside the sphere of influence, and only for an orbit of a body.
    #[test]
    fn a_transfer_is_offered_where_the_frame_is_one_the_ship_is_falling_with() {
        let system = sol();
        let low = station(&system, "Earth", 0.5);
        let earth = system.body_position_at("Earth", 0.0).unwrap();
        assert_eq!(
            primary_for(&system, &low, earth, 0.0).as_deref(),
            Some("Earth")
        );

        // From Mars, Earth's frame is not one this ship is falling with.
        let mars = system.body_position_at("Mars", 0.0).unwrap();
        assert_eq!(primary_for(&system, &low, mars, 0.0), None);

        // And a fixed point in space is about nothing at all.
        assert_eq!(
            primary_for(&system, &Waypoint::Fixed(earth), earth, 0.0),
            None
        );

        // Nor does the star get one. Its influence has no outer edge, so asking which body holds
        // a point names it wherever the point is — including four light-years out, where a
        // transfer about it is the last thing anybody wants.
        let sun = station(&system, "Sol", 1.0);
        let far = system.star_position_ly() + DVec3::X * 4.2;
        assert_eq!(primary_for(&system, &sun, far, 0.0), None);
        assert_eq!(
            primary_for(&system, &sun, system.star_position_ly(), 0.0),
            None
        );
    }
}
