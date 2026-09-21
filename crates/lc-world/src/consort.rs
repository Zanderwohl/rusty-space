//! Closing on a craft that is falling, and keeping station on it.
//!
//! [`crate::pursuit`] reckons a quarry forward in a straight line, and plans in a frame that
//! does not fall. Between the stars that is exactly right. Inside a system it is not: a quarry
//! in low orbit of Jupiter is bent off that line at nearly two gravities, and a pursuer whose
//! plan ignores gravity is off by ninety kilometers a hundred seconds in. Re-solving against
//! that kept it wandering a relative orbit tens to thousands of kilometers across, and a
//! five-kilometer hull could not get inside a hundred thousand.
//!
//! **So the quarry is reckoned along its conic, and the approach is planned in the frame that
//! falls with it** — [`crate::transfer`]'s idea, with a sighting in place of a body. Both craft
//! fall together, so what is left for the plan is the relative motion alone, and a plan that
//! ends at rest there ends matched *and stays matched*: after the approach the pursuer holds
//! its offset for as long as the quarry holds its arc, rather than going ballistic beside it and
//! drifting off by the difference between two orbits.
//!
//! **The frame is still a sighting.** The conic is solved from where the quarry was seen and
//! how fast, so this tells a client nothing its own eyes could not; a quarry that maneuveres
//! leaves its conic and is re-solved against when the news arrives, as a rendezvous is.
//!
//! **Galilean, and bounded to where that is true**, for the reason [`crate::transfer`] gives:
//! [`GALILEAN_BETA`] is three hundred kilometers a second. And the frame is not inertial, so a
//! pursuer holding an offset in it is thrusting against the difference between its own fall and
//! the quarry's — a tidal term, milligravities a thousand kilometers off, the same thing
//! [`crate::motion::thrust_g`] waves away for a ship holding a station.

use glam::DVec3;

use crate::coast::{self, Coast};
use crate::flight::{Aim, C_M_S, Cruise, Drive, FlightState};
use crate::motion::{ShipId, ShipState};
use crate::pursuit::Sighting;
use crate::system::{LocalSystem, M_PER_LY};

/// The fastest quarry reckoned along a conic rather than boosted into. Composing velocities by
/// adding them is good to a part in a million here.
pub const GALILEAN_BETA: f64 = 1.0e-3;

/// A pursuer closing on, or keeping station beside, a quarry on a conic.
#[derive(Clone, Debug, PartialEq)]
pub struct Consort {
    /// The approach, in the quarry's falling frame: offsets from it, velocities relative to it,
    /// and the world's own clock.
    pub cruise: Cruise,
    /// The quarry's conic, solved from the sighting below.
    pub frame: Coast,
    pub seen_ly: DVec3,
    pub seen_beta: DVec3,
    /// Coordinate seconds the light left.
    pub seen_s: f64,
    pub target: ShipId,
}

/// A consort as the arguments it was solved from: what crosses the wire and goes on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Formation {
    pub from_ly: DVec3,
    pub beta0: DVec3,
    pub to_ly: DVec3,
    pub start_s: f64,
    pub drive: Drive,
    pub seen_ly: DVec3,
    pub seen_beta: DVec3,
    pub seen_s: f64,
    pub target: ShipId,
}

impl Formation {
    /// Solve it. `None` where the system has no conic through the sighting, which both ends
    /// agree about because they ask the same question of the same system.
    pub fn solve(&self, system: &LocalSystem, attitude: DVec3) -> Option<Consort> {
        Some(Consort {
            cruise: Cruise::plan_from(
                self.from_ly,
                self.beta0,
                self.to_ly,
                attitude,
                self.start_s,
                self.drive,
            ),
            frame: frame_through(system, self.seen_ly, self.seen_beta, self.seen_s)?,
            seen_ly: self.seen_ly,
            seen_beta: self.seen_beta,
            seen_s: self.seen_s,
            target: self.target,
        })
    }
}

fn frame_through(system: &LocalSystem, at_ly: DVec3, beta: DVec3, at_s: f64) -> Option<Coast> {
    Coast::from_state(system, at_ly, beta * C_M_S, at_s)
}

/// Where a frame is at a coordinate time: light-years and a fraction of `c`.
fn place(frame: &Coast, system: &LocalSystem, now_s: f64) -> Option<(DVec3, DVec3)> {
    let (at, velocity) = frame.at(system, now_s)?;
    Some((at, coast::beta_of(velocity)))
}

/// The conic a quarry is reckoned along, if this is a quarry to reckon that way.
pub fn frame_for(system: &LocalSystem, seen: &Sighting) -> Option<Coast> {
    if seen.beta.length() > GALILEAN_BETA {
        return None;
    }
    frame_through(system, seen.position_ly, seen.beta, seen.emitted_s)
}

impl Consort {
    pub fn recipe(&self) -> Formation {
        Formation {
            from_ly: self.cruise.from_ly,
            beta0: self.cruise.initial_beta(),
            to_ly: self.cruise.to_ly,
            start_s: self.cruise.start_s,
            drive: self.cruise.drive,
            seen_ly: self.seen_ly,
            seen_beta: self.seen_beta,
            seen_s: self.seen_s,
            target: self.target,
        }
    }

    /// Where the quarry is reckoned to be, and how fast.
    pub fn frame_at(&self, system: &LocalSystem, now_s: f64) -> Option<(DVec3, DVec3)> {
        place(&self.frame, system, now_s)
    }

    pub fn state_at(&self, system: &LocalSystem, now_s: f64) -> Option<(DVec3, DVec3)> {
        let (at, beta) = self.frame_at(system, now_s)?;
        let flight = self.flight_at(now_s);
        Some((at + flight.position_ly, beta + flight.beta))
    }

    pub fn flight_at(&self, now_s: f64) -> FlightState {
        self.cruise.at(now_s)
    }

    /// The frame does not rotate and its speed is small, so the cruise's directions are the
    /// world's; see [`crate::transfer::Transfer::aim_at`].
    pub fn aim_at(&self, now_s: f64) -> Aim {
        self.cruise.aim_at(now_s)
    }

    pub fn thrust_at(&self, now_s: f64) -> DVec3 {
        self.cruise.thrust_at(now_s)
    }

    pub fn has_closed(&self, now_s: f64) -> bool {
        self.cruise.has_arrived(now_s)
    }

    /// The crew's seconds since this was taken up. Past the approach they age at the world's
    /// rate, being at rest beside a quarry going a few tens of kilometers a second.
    pub fn proper_s_at(&self, now_s: f64) -> f64 {
        let end = self.cruise.start_s + self.cruise.duration_s();
        if now_s > end {
            self.cruise.at(end).proper_s + (now_s - end)
        } else {
            self.cruise.at(now_s).proper_s
        }
    }

    /// How far off its conic a fresh sighting puts the quarry, meters. `None` when the conic
    /// can no longer be placed, which is as good as not being on it.
    pub fn divergence_m(&self, system: &LocalSystem, seen: &Sighting) -> Option<f64> {
        let (at, _) = place(&self.frame, system, seen.emitted_s)?;
        Some(at.distance(seen.position_ly) * M_PER_LY)
    }
}

/// Plan taking station `standoff_m` off a quarry reckoned along its conic, or `None` where
/// [`frame_for`] finds none and the quarry is a rendezvous's business instead.
///
/// Never refused as already there, unlike a rendezvous: a craft that is at the standoff is
/// still being asked to *stay* there, and this is what stays.
pub fn approach(
    system: &LocalSystem,
    pursuer: &ShipState,
    standoff_m: f64,
    seen: &Sighting,
    now_s: f64,
    drive: Drive,
) -> Option<Consort> {
    let (quarry_at, quarry_beta) = place(&frame_for(system, seen)?, system, now_s)?;
    let offset = pursuer.position_ly - quarry_at;
    // The side the pursuer is already on, as for a rendezvous.
    let side = offset.try_normalize().unwrap_or(DVec3::X);
    Formation {
        from_ly: offset,
        beta0: pursuer.beta - quarry_beta,
        to_ly: side * standoff_m / M_PER_LY,
        start_s: now_s,
        drive,
        seen_ly: seen.position_ly,
        seen_beta: seen.beta,
        seen_s: seen.emitted_s,
        target: seen.target,
    }
    // Through the recipe, so a restored plan and this one are the same trajectory.
    .solve(system, pursuer.attitude)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::{Course, Plane, Waypoint};
    use crate::pursuit::Closeness;

    fn sol() -> LocalSystem {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .expect("the catalogue");
        let sun = crate::sky::StarProvider::stars(&provider)
            .iter()
            .find(|s| s.name.as_deref() == Some("Sol"))
            .expect("the Sun")
            .clone();
        let mut system = LocalSystem::for_star(&sun).expect("the solar system");
        system.advance_to(0.0);
        system
    }

    /// A quarry holding low orbit of Jupiter, seen at `t`.
    fn holding(system: &LocalSystem, t: f64) -> (Waypoint, Sighting) {
        let station = Course::Orbit {
            body: "Jupiter".into(),
            altitude_radii: 0.5,
            plane: Plane::Equatorial,
        }
        .resolve(system, DVec3::ZERO, 0.0)
        .expect("an orbit");
        let seen = sighting_of(system, &station, t);
        (station, seen)
    }

    fn sighting_of(system: &LocalSystem, station: &Waypoint, t: f64) -> Sighting {
        Sighting {
            target: ShipId(2),
            position_ly: station.place_at(system, t).unwrap(),
            beta: coast::beta_of(station.velocity_at(system, t).unwrap()),
            length_m: 500.0,
            emitted_s: t,
        }
    }

    fn drive() -> Drive {
        Drive {
            accel_g: 5.0,
            slew_rate_rad_s: 0.05,
            ..Drive::DEFAULT
        }
    }

    /// **The report this exists for.** A pursuer ten kilometers off a quarry in low orbit of
    /// Jupiter closes to a kilometer and a half and is still there an orbit later, to a few
    /// meters — where a plan that ignored the planet was off by ninety kilometers in a hundred
    /// seconds.
    #[test]
    fn it_closes_to_intimate_range_in_orbit_and_stays() {
        let system = sol();
        let (station, seen) = holding(&system, 0.0);
        let mut pursuer =
            ShipState::at(seen.position_ly + DVec3::new(6.0e3, -8.0e3, 0.0) / M_PER_LY);
        pursuer.beta = seen.beta;
        let standoff = Closeness::Intimate.standoff_m(500.0, 500.0);
        let plan = approach(&system, &pursuer, standoff, &seen, 0.0, drive()).expect("a plan");

        let end = plan.cruise.start_s + plan.cruise.duration_s();
        assert!(end < 3_600.0, "premise: a short hop, took {end} s");
        let period = station
            .period_s(&system, 0.0)
            .expect("an orbit has a period");
        for t in [end, end + 0.25 * period, end + period] {
            let (at, beta) = plan.state_at(&system, t).unwrap();
            // Against where the quarry really is, not against the plan's model of it.
            let quarry = station.place_at(&system, t).unwrap();
            let gap = at.distance(quarry) * M_PER_LY;
            assert!(
                (gap - standoff).abs() < 25.0,
                "{gap} m off a {standoff} m standoff at {t} s"
            );
            let drift =
                (beta - coast::beta_of(station.velocity_at(&system, t).unwrap())).length() * C_M_S;
            assert!(drift < 5.0e-3, "drifting at {drift} m/s at {t} s");
        }
        assert!(plan.has_closed(end));
        assert_eq!(
            plan.thrust_at(end + 1.0),
            DVec3::ZERO,
            "nothing is lit once alongside"
        );
    }

    /// A quarry holding its orbit stays on the conic reckoned for it, so a standing consort is
    /// not thrown away for no reason — not for ten orbits, at the tightest closeness there is.
    #[test]
    fn a_held_orbit_does_not_diverge() {
        let system = sol();
        let (station, seen) = holding(&system, 0.0);
        let pursuer = ShipState::at(seen.position_ly + DVec3::X * 3.0e3 / M_PER_LY);
        let plan = approach(&system, &pursuer, 1_500.0, &seen, 0.0, drive()).expect("a plan");
        let period = station.period_s(&system, 0.0).unwrap();
        let later = sighting_of(&system, &station, 10.0 * period);
        let off = plan.divergence_m(&system, &later).unwrap();
        assert!(
            off < Closeness::Intimate.replan_m(1_500.0),
            "{off} m after ten orbits"
        );
    }

    /// Put back from its arguments, it is the same plan.
    #[test]
    fn a_consort_survives_being_reduced_to_its_arguments() {
        let system = sol();
        let (_, seen) = holding(&system, 120.0);
        let pursuer = ShipState::at(seen.position_ly + DVec3::Y * 9.0e3 / M_PER_LY);
        let plan = approach(&system, &pursuer, 1_500.0, &seen, 300.0, drive()).expect("a plan");
        assert_eq!(plan.recipe().solve(&system, pursuer.attitude), Some(plan));
    }

    /// Past a few hundred kilometers a second it is a job for the boost, not for this.
    #[test]
    fn a_fast_quarry_is_not_reckoned_along_a_conic() {
        let system = sol();
        let (_, seen) = holding(&system, 0.0);
        let fast = Sighting {
            beta: DVec3::Y * 0.01,
            ..seen
        };
        assert!(frame_for(&system, &fast).is_none());
    }
}
