//! How much a lit drive may put on its neighbors. See `lightcone/docs/31-directed-energy.md`
//! §Maneuvering near others.
//!
//! Every limit here is a flux, so it holds for a receiver of any size: shadow and rated load both
//! go as size squared. The receiver is assumed Black unless an absorptivity is given, and the
//! emitter a cone as [`crate::emit`] models it.
//!
//! [`Arrival`] is where an approach's legs are chosen. It works on offsets from the quarry in
//! whatever frame the approach is planned in — at rest with it, accelerating with it or falling
//! with it — because that is the frame the cones are fixed in while the pair fly together.

use glam::DVec3;

use crate::emit::{distance_at_flux_m, thrust_power_w};
use crate::fitting::{Balance, RATED_LOAD_AU};
use crate::flight::{C_M_S, Drive, G0};
use crate::motion::ShipId;
use crate::signal::cone_solid_angle_sr;
use crate::solar::SOLAR_CONSTANT_W_M2;

/// The flux a full, Black receiver of any size sits at its rated load in, W/m².
///
/// The gained starlight at [`RATED_LOAD_AU`]. The living drain also counts toward the anchor, but
/// it grows with volume rather than area, so it has no per-area share; on the starting ship it is
/// a part in 10⁴.
pub fn cooking_flux_w_m2(balance: &Balance) -> f64 {
    balance.solar_gain * SOLAR_CONSTANT_W_M2 / (RATED_LOAD_AU * RATED_LOAD_AU)
}

/// The most a courteous maneuver puts on anyone it can see, W/m².
pub fn courtesy_flux_w_m2(balance: &Balance) -> f64 {
    balance.courtesy_fraction * cooking_flux_w_m2(balance)
}

/// Inside this distance, a full receiver of absorptivity `absorptivity` in the cone is past its
/// rated load, meters.
pub fn cooking_distance_m(balance: &Balance, power_w: f64, half_angle_rad: f64, absorptivity: f64) -> f64 {
    distance_at_flux_m(power_w * absorptivity, half_angle_rad, cooking_flux_w_m2(balance))
}

/// Inside this distance, an emission of `power_w` at `half_angle_rad` is discourteous to anyone in
/// its cone, meters.
pub fn courtesy_radius_m(balance: &Balance, power_w: f64, half_angle_rad: f64) -> f64 {
    distance_at_flux_m(power_w, half_angle_rad, courtesy_flux_w_m2(balance))
}

pub fn drive_courtesy_radius_m(balance: &Balance, power_w: f64) -> f64 {
    courtesy_radius_m(balance, power_w, balance.drive_spread_rad)
}

/// What station-keeping thrusters put out at full throttle on a craft of `mass_kg`, watts.
pub fn thrusters_power_w(balance: &Balance, mass_kg: f64) -> f64 {
    thrust_power_w(mass_kg, balance.rcs_accel_g * G0)
}

pub fn thrusters_courtesy_radius_m(balance: &Balance, mass_kg: f64) -> f64 {
    courtesy_radius_m(balance, thrusters_power_w(balance, mass_kg), balance.rcs_spread_rad)
}

/// The most an emission at `half_angle_rad` may carry and stay courteous to someone `distance_m`
/// away, watts: how far thrusters are throttled when holding station close.
pub fn courteous_power_w(balance: &Balance, half_angle_rad: f64, distance_m: f64) -> f64 {
    courtesy_flux_w_m2(balance) * cone_solid_angle_sr(half_angle_rad) * distance_m * distance_m
}

/// Whether a leg is flown on station-keeping thrusters rather than the main drive. A thruster leg
/// is an ordinary cruise at no more than `rcs_accel_g`, so the acceleration is what says so.
pub fn on_thrusters(balance: &Balance, drive: &Drive) -> bool {
    drive.accel_g <= balance.rcs_accel_g
}

/// How an approach arrives. See 31 §Two ways to approach.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arrival {
    /// Burn, flip and brake straight onto a station on the pursuer's side, cone and all.
    Direct,
    Courteous(Manners),
}

/// What a courteous pursuer needs to choose its legs: its own drive's reach and where its
/// station is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manners {
    /// The main drive's courtesy radius at full thrust, with [`INGRESS_MARGIN`], meters. A
    /// main-drive leg never comes nearer the quarry than this.
    pub ingress_m: f64,
    /// The most the thrusters give, g.
    pub thrusters_g: f64,
    /// The thruster acceleration that is courteous one meter off, g/m². It grows as distance
    /// squared.
    pub courteous_g_per_m2: f64,
    /// Where around the axis this pursuer's station sits, radians. See [`flotilla_azimuth_rad`].
    pub azimuth_rad: f64,
}

/// The ingress sphere is this much wider than the courtesy radius, for the difference between a
/// straight-line leg and the path a cruise with a match burn and a turn actually flies.
pub const INGRESS_MARGIN: f64 = 1.1;

/// The share of the courtesy limit a thruster leg is throttled to, leaving room for a station
/// reached a little inside its standoff.
pub const THROTTLE_HEADROOM: f64 = 0.9;

/// Legs that are not the station end at least this many standoffs out, past
/// [`DRIFT_ALLOWANCE`](crate::pursuit::DRIFT_ALLOWANCE) of their own standoff, so
/// [`is_waypoint`] can tell them from the station by where they end.
pub const WAYPOINT_STANDOFFS: f64 = 3.0;

/// The widest turn about the quarry one leg makes. A chord between two points this far apart
/// round a sphere clears the quarry by `cos 30°` of their radius.
pub const HOP_RAD: f64 = std::f64::consts::FRAC_PI_3;

/// A pursuer's azimuth about its quarry's axis, radians: the golden-ratio sequence in its id, so
/// consecutive ids — the usual flotilla — land far apart.
pub fn flotilla_azimuth_rad(id: ShipId) -> f64 {
    const GOLDEN: f64 = 0.618_033_988_749_894_9;
    std::f64::consts::TAU * (id.0 as f64 * GOLDEN).rem_euclid(1.0)
}

impl Manners {
    /// For a pursuer of `mass_kg` whose main drive is `drive`, with id `id`.
    pub fn new(balance: &Balance, mass_kg: f64, drive: Drive, id: ShipId) -> Self {
        let full_w = thrust_power_w(mass_kg, drive.accel_g * G0);
        Self {
            ingress_m: INGRESS_MARGIN * drive_courtesy_radius_m(balance, full_w),
            thrusters_g: balance.rcs_accel_g,
            courteous_g_per_m2: THROTTLE_HEADROOM
                * courteous_power_w(balance, balance.rcs_spread_rad, 1.0)
                / (mass_kg * C_M_S * G0),
            azimuth_rad: flotilla_azimuth_rad(id),
        }
    }

    /// Nearer than this, the thrusters are throttled, meters.
    pub fn full_thrust_m(&self) -> f64 {
        (self.thrusters_g / self.courteous_g_per_m2).sqrt()
    }
}

/// One leg of an approach: where it ends, from the quarry, and what it is flown on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leg {
    pub to_m: DVec3,
    pub drive: Drive,
    pub thrusters: bool,
}

impl Arrival {
    pub fn is_courteous(&self) -> bool {
        matches!(self, Arrival::Courteous(_))
    }

    /// The next leg for a pursuer `offset_m` from its quarry, meters in the approach's frame.
    ///
    /// `side` is the direction to stand off on when there is no other reason to choose, and
    /// `thrust_axis` the quarry's thrust direction if it is lit. Courteously, a quarry under thrust
    /// has its station abeam of that axis at this pursuer's azimuth, so neither cone points at the
    /// other; one that is not has it abeam of the line of arrival.
    ///
    /// Outside the ingress sphere the leg is on the main drive and ends on that sphere at a point
    /// the pursuer can see, so the whole leg stays outside it and its brake passes beside the
    /// quarry. A station more than [`HOP_RAD`] round the sphere from there is reached through
    /// waypoints. Inside it the leg is on thrusters, throttled for the closest the leg comes.
    pub fn leg(
        &self,
        offset_m: DVec3,
        side: DVec3,
        thrust_axis: Option<DVec3>,
        standoff_m: f64,
        drive: Drive,
    ) -> Leg {
        let manners = match self {
            Arrival::Direct => return Leg { to_m: side * standoff_m, drive, thrusters: false },
            Arrival::Courteous(manners) => manners,
        };
        let ingress = manners.ingress_m.max(WAYPOINT_STANDOFFS * standoff_m);
        let d = offset_m.length();
        let beyond = |r: f64| d > r * (1.0 + crate::pursuit::ON_STATION_FRACTION);
        let outside = beyond(ingress);
        let station = match thrust_axis.and_then(DVec3::try_normalize) {
            Some(axis) => abeam(axis, manners.azimuth_rad),
            None if outside => abeam(side, manners.azimuth_rad),
            None => side,
        };
        let theta = side.angle_between(station);
        if outside {
            let cap = (ingress / d).acos();
            let (dir, r) = if theta <= cap {
                (station, ingress)
            } else if theta - cap <= HOP_RAD {
                (toward(side, station, cap), ingress)
            } else {
                (toward(side, station, HOP_RAD), d.max(2.0 * ingress))
            };
            return Leg { to_m: dir * r, drive, thrusters: false };
        }
        // Full thrust as far in as it stays courteous, so only the last stretch is throttled.
        let full_m = manners.full_thrust_m();
        let to_m = if theta <= HOP_RAD {
            let short = full_m > WAYPOINT_STANDOFFS * standoff_m && beyond(full_m);
            station * if short { full_m } else { standoff_m }
        } else {
            toward(side, station, HOP_RAD) * d.max(WAYPOINT_STANDOFFS * standoff_m)
        };
        let clearance = clearance_m(offset_m, to_m);
        let accel_g = (manners.courteous_g_per_m2 * clearance * clearance).min(manners.thrusters_g);
        Leg { to_m, drive: Drive { accel_g, ..drive }, thrusters: true }
    }
}

/// Whether a leg ending `to_m` from the quarry is on the way to a station rather than the station.
pub fn is_waypoint(to_m: f64, standoff_m: f64) -> bool {
    to_m > crate::pursuit::DRIFT_ALLOWANCE * standoff_m
}

/// The unit vector perpendicular to `axis` at `azimuth_rad`, measured from the ecliptic pole's
/// projection, or from +X for an axis near the pole.
pub fn abeam(axis: DVec3, azimuth_rad: f64) -> DVec3 {
    let reference = if axis.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let e1 = reference.reject_from_normalized(axis).normalize();
    let e2 = axis.cross(e1);
    e1 * azimuth_rad.cos() + e2 * azimuth_rad.sin()
}

/// `from` turned `angle` toward `to`, both unit. Antiparallel, the turn is about any axis.
fn toward(from: DVec3, to: DVec3, angle: f64) -> DVec3 {
    let across = to
        .reject_from_normalized(from)
        .try_normalize()
        .unwrap_or_else(|| from.any_orthonormal_vector());
    from * angle.cos() + across * angle.sin()
}

/// The nearest a straight leg from `a` to `b` comes to the origin.
fn clearance_m(a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let along = ab.length_squared();
    let t = if along > 0.0 { (-a.dot(ab) / along).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).length()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::emit::{flux_w_m2, rating_w, received_fraction};
    use crate::fitting::Loadout;
    use crate::solar::broadside_m2;

    const LENGTHS_M: [f64; 3] = [500.0, 5_000.0, 50_000.0];

    /// The starting ship scaled to `length_m`: its drive section's volume and its full mass both
    /// go as length cubed.
    fn scaled(b: &Balance, length_m: f64) -> (f64, f64) {
        let k = (length_m / 500.0).powi(3);
        let drive_m3 = Loadout::STARTING.engines as f64 * b.slot_volume_m3;
        (rating_w(b, drive_m3 * k), b.engine_thrust_n / G0 * k)
    }

    /// Two significant figures, as 31 writes them.
    fn sig2(x: f64) -> String {
        format!("{x:.1e}")
    }

    /// The flux 30's τ anchor fixes: 30's rated load over the starting ship's broadside.
    #[test]
    fn the_cooking_flux_is_the_rated_load_over_the_shadow() {
        let flux = cooking_flux_w_m2(&Balance::DEFAULT);
        assert_eq!(sig2(flux * broadside_m2(500.0)), "7.6e19");
    }

    /// 31 §Exhaust lands on whatever is behind.
    #[test]
    fn exhaust_cooking_distance_table() {
        let b = Balance::DEFAULT;
        let rows = [("1.1e20", "2.6e3"), ("1.1e23", "8.4e4"), ("1.1e26", "2.6e6")];
        for (length, (power, distance)) in LENGTHS_M.into_iter().zip(rows) {
            let (p, _) = scaled(&b, length);
            assert_eq!(sig2(p), power, "{length} m");
            let black = cooking_distance_m(&b, p, b.drive_spread_rad, 1.0);
            assert_eq!(sig2(black), distance, "{length} m");
            // "A Clear receiver's distance is a little over half of these."
            let clear = cooking_distance_m(&b, p, b.drive_spread_rad, b.clear_absorptivity) / black;
            assert!((0.5..0.6).contains(&clear), "{clear}");
        }
    }

    /// 31 §Station-keeping thrusters. At 60° the solid angle and `pi theta^2` part by 10%, and
    /// the doc's first figures, from a flat disk of radius `d tan theta`, were 40% short.
    #[test]
    fn thrusters_cooking_distance_table() {
        let b = Balance::DEFAULT;
        let rows = [("2.1e17", "1.0e1"), ("2.1e20", "3.3e2"), ("2.1e23", "1.0e4")];
        for (length, (power, distance)) in LENGTHS_M.into_iter().zip(rows) {
            let (_, kg) = scaled(&b, length);
            let p = thrusters_power_w(&b, kg);
            assert_eq!(sig2(p), power, "{length} m");
            assert_eq!(sig2(cooking_distance_m(&b, p, b.rcs_spread_rad, 1.0)), distance, "{length} m");
        }
    }

    /// 31 §Courtesy.
    #[test]
    fn drive_courtesy_radius_table() {
        let b = Balance::DEFAULT;
        for (length, radius) in LENGTHS_M.into_iter().zip(["2.6e4", "8.4e5", "2.6e7"]) {
            let (p, _) = scaled(&b, length);
            assert_eq!(sig2(drive_courtesy_radius_m(&b, p)), radius, "{length} m");
        }
    }

    /// The radius is where the flux is the limit, whoever sits there: a receiver of any size at it
    /// absorbs exactly the courtesy flux over its shadow.
    #[test]
    fn at_the_courtesy_radius_any_receiver_takes_the_limit() {
        let b = Balance::DEFAULT;
        let (p, _) = scaled(&b, 5_000.0);
        let r = drive_courtesy_radius_m(&b, p);
        let limit = courtesy_flux_w_m2(&b);
        assert!((flux_w_m2(p, b.drive_spread_rad, r) / limit - 1.0).abs() < 1.0e-12);
        for length in LENGTHS_M {
            let shadow = broadside_m2(length);
            let per_m2 = p * received_fraction(b.drive_spread_rad, shadow, r) / shadow;
            assert!((per_m2 / limit - 1.0).abs() < 1.0e-12, "{length} m: {per_m2}");
        }
        // 31 §Two ways to approach: past the braking cone's reach, eleven radii out, the flux is
        // under a hundredth of the limit.
        assert!(flux_w_m2(p, b.drive_spread_rad, 11.0 * r) < 0.01 * limit);
    }

    /// Throttled to the courteous power at a distance, the thrusters' courtesy radius is that
    /// distance.
    #[test]
    fn courteous_power_is_the_inverse_of_the_courtesy_radius() {
        let b = Balance::DEFAULT;
        let d = 3_000.0;
        let p = courteous_power_w(&b, b.rcs_spread_rad, d);
        let r = courtesy_radius_m(&b, p, b.rcs_spread_rad);
        assert!((r - d).abs() < 1.0e-9, "{r}");
        let (_, kg) = scaled(&b, 50_000.0);
        assert!(p < thrusters_power_w(&b, kg), "a GSV three kilometers off must throttle");
    }

    use crate::escort::{self, Escort};
    use crate::flight::Cruise;
    use crate::motion::ShipState;
    use crate::pursuit::{self, Closeness, Refused, Rendezvous, Sighting};
    use crate::system::M_PER_LY;

    /// Evenly spaced samples per leg, for a closest approach part-way along it.
    const SAMPLES: usize = 4_000;
    /// Samples closing geometrically on each end of a leg, a tenth of a percent nearer each time.
    /// Evenly spaced ones miss a brake's last seconds, where it is nearest the quarry: from four
    /// hundred million kilometers they are forty-five seconds apart and the last is fifty
    /// kilometers short.
    const END_SAMPLES: i32 = 40_000;
    const QUARRY: ShipId = ShipId(1);

    pub(crate) struct Pursuer {
        pub mass_kg: f64,
        pub length_m: f64,
        pub drive: Drive,
        pub arrival: Arrival,
    }

    pub(crate) fn pursuer(b: &Balance, length_m: f64, id: ShipId, courteous: bool) -> Pursuer {
        let (_, mass_kg) = scaled(b, length_m);
        let drive = Drive { slew_rate_rad_s: 1.0, ..Drive::DEFAULT };
        let arrival =
            if courteous { Arrival::Courteous(Manners::new(b, mass_kg, drive, id)) } else { Arrival::Direct };
        Pursuer { mass_kg, length_m, drive, arrival }
    }

    /// What one cone from `at_m` puts on the quarry at the origin, W/m².
    fn onto_quarry(at_m: DVec3, exhaust: DVec3, power_w: f64, half_angle_rad: f64) -> f64 {
        if power_w <= 0.0 || exhaust == DVec3::ZERO || exhaust.angle_between(-at_m) >= half_angle_rad {
            return 0.0;
        }
        flux_w_m2(power_w, half_angle_rad, at_m.length())
    }

    /// The most a leg puts on the quarry at any sampled instant, over the courtesy flux.
    ///
    /// In the frame the leg was planned in, with the quarry at its origin burning at
    /// `quarry_g` there. Beside a burning quarry a thruster leg is two emissions, the main drive
    /// carrying the quarry's acceleration and the thrusters the closing; a main-drive leg is one.
    pub(crate) fn peak(b: &Balance, who: &Pursuer, cruise: &Cruise, quarry_g: DVec3) -> f64 {
        let limit = courtesy_flux_w_m2(b);
        let watts = |g: f64| thrust_power_w(who.mass_kg, g * G0);
        let (start, span) = (cruise.start_s, cruise.duration_s());
        let even = (0..=SAMPLES).map(|k| start + span * k as f64 / SAMPLES as f64);
        let ends = (0..END_SAMPLES).flat_map(|k| {
            let from_end = span * 0.999f64.powi(k);
            [start + from_end, start + span - from_end]
        });
        let mut worst = 0.0f64;
        for t in even.chain(ends) {
            let at = cruise.at(t).position_ly * M_PER_LY;
            let closing = cruise.thrust_at(t) * cruise.drive.accel_g;
            let flux = if on_thrusters(b, &cruise.drive) {
                onto_quarry(at, -quarry_g, watts(quarry_g.length()), b.drive_spread_rad)
                    + onto_quarry(at, -closing, watts(closing.length()), b.rcs_spread_rad)
            } else {
                let push = quarry_g + closing;
                onto_quarry(at, -push, watts(push.length()), b.drive_spread_rad)
            };
            worst = worst.max(flux / limit);
        }
        worst
    }

    fn standoff(who: &Pursuer, closeness: Closeness) -> f64 {
        closeness.standoff_m(who.length_m, 500.0)
    }

    fn at_rest(position_m: DVec3) -> Sighting {
        Sighting { target: QUARRY, position_ly: position_m / M_PER_LY, beta: DVec3::ZERO, length_m: 500.0, emitted_s: 0.0 }
    }

    /// A pursuer closing on a quarry at rest at the origin, leg after leg until it is on
    /// station: every leg, and the worst any of them puts on the quarry.
    fn approach_legs(b: &Balance, who: &Pursuer, from_m: DVec3, closeness: Closeness) -> (Vec<Rendezvous>, f64) {
        let seen = at_rest(DVec3::ZERO);
        let mut ship = ShipState::at(from_m / M_PER_LY);
        let (mut legs, mut worst, mut now) = (Vec::new(), 0.0f64, 0.0);
        loop {
            let plan = match pursuit::approach(&ship, standoff(who, closeness), who.arrival, &seen, now, who.drive) {
                Ok(plan) => plan,
                Err(Refused::AlreadyThere) => return (legs, worst),
                Err(why) => panic!("{why:?}"),
            };
            worst = worst.max(peak(b, who, &plan.cruise, DVec3::ZERO));
            now = plan.cruise.start_s + plan.cruise.duration_s();
            let (at, beta) = plan.state_at(now);
            ship.position_ly = at;
            ship.beta = beta;
            legs.push(plan);
            assert!(legs.len() < 12, "never took station: {:?}", legs.iter().map(|l| l.cruise.to_ly * M_PER_LY).collect::<Vec<_>>());
        }
    }

    /// The same for an escort of a quarry burning at `quarry_g` along `axis`, until the leg ending
    /// on station has closed. Returns that station, in the quarry's frame.
    fn escort_legs(b: &Balance, who: &Pursuer, from_m: DVec3, quarry_g: DVec3) -> (Vec<Escort>, f64) {
        let accel = quarry_g * G0 / C_M_S;
        let mut seen = at_rest(DVec3::ZERO);
        let mut ship = ShipState::at(from_m / M_PER_LY);
        let s = standoff(who, Closeness::Company);
        let (mut legs, mut worst, mut now) = (Vec::new(), 0.0f64, 0.0);
        loop {
            let plan = escort::escort(&ship, s, who.arrival, &seen, accel, now, who.drive).expect("a plan");
            worst = worst.max(peak(b, who, &plan.cruise, quarry_g));
            let end = plan.cruise.start_s + plan.cruise.duration_s();
            now = plan.quarry.since_t + plan.quarry.world_elapsed(end);
            let (at, beta) = plan.state_at(now);
            ship.position_ly = at;
            ship.beta = beta;
            let (q_at, q_beta) = plan.quarry.at(now);
            seen = Sighting { position_ly: q_at, beta: q_beta, emitted_s: now, ..seen };
            let done = !pursuit::on_the_way(plan.cruise.to_ly, s, &who.arrival);
            legs.push(plan);
            if done {
                return (legs, worst);
            }
            assert!(legs.len() < 12, "never took station");
        }
    }

    /// Four hundred million kilometers: well outside the largest hull's ingress sphere, about
    /// thirty thousand.
    const FAR_M: f64 = 4.0e11;

    /// **31 §Tests, Courtesy**, for a quarry at rest. The ingress leg ends on the courtesy sphere,
    /// and what is flown inside it is on thrusters.
    ///
    /// Every hull size, because an abeam station alone keeps the smallest courteous: its brake
    /// cone takes in the quarry only past eleven standoffs, beyond its courtesy radius. Only
    /// larger hulls need the ingress.
    #[test]
    fn a_courteous_approach_never_exceeds_the_courtesy_flux() {
        let b = Balance::DEFAULT;
        for length in LENGTHS_M {
            let who = pursuer(&b, length, ShipId(2), true);
            let (legs, worst) = approach_legs(&b, &who, DVec3::X * FAR_M, Closeness::Company);
            assert!(worst <= 1.0, "{length} m: {worst} of the courtesy flux");
            let last = legs.last().unwrap();
            assert!(on_thrusters(&b, &last.cruise.drive), "{length} m: the last leg is on the main drive");
            assert!(!on_thrusters(&b, &legs[0].cruise.drive), "{length} m: the first leg is on thrusters");
            // Abeam of the line it arrived along.
            let station = last.cruise.to_ly.normalize();
            assert!(station.dot(DVec3::X).abs() < 0.2, "{length} m: station at {station}");
        }
    }

    /// The counter-case that shows the check above can fail: brake straight onto the station and
    /// the brake flames the quarry.
    #[test]
    fn a_direct_approach_exceeds_it() {
        let b = Balance::DEFAULT;
        for length in LENGTHS_M {
            let who = pursuer(&b, length, ShipId(2), false);
            let (legs, worst) = approach_legs(&b, &who, DVec3::X * FAR_M, Closeness::Company);
            assert_eq!(legs.len(), 1);
            assert!(worst > 10.0, "{length} m: {worst} of the courtesy flux");
        }
    }

    /// A fifty-kilometer ship taking station a kilometer off a small one is inside its own
    /// thrusters' courtesy radius there, so the last stretch is throttled.
    #[test]
    fn a_large_ship_closing_in_throttles_its_thrusters() {
        let b = Balance::DEFAULT;
        let who = pursuer(&b, 50_000.0, ShipId(2), true);
        let Arrival::Courteous(manners) = who.arrival else { unreachable!() };
        let s = standoff(&who, Closeness::Intimate);
        assert!(manners.full_thrust_m() > s, "premise: full thrust at the station is discourteous");
        let (legs, worst) = approach_legs(&b, &who, DVec3::Y * FAR_M, Closeness::Intimate);
        assert!(worst <= 1.0, "{worst} of the courtesy flux");
        let last = legs.last().unwrap();
        assert!(last.cruise.drive.accel_g < b.rcs_accel_g, "the last leg was not throttled");
    }

    /// **31 §Tests, Courtesy**, for an escort of a burning quarry, from dead astern where a chase
    /// starts. Its station is abeam of the burn.
    #[test]
    fn a_courteous_escort_never_exceeds_the_courtesy_flux() {
        let b = Balance::DEFAULT;
        let burn = DVec3::X;
        for length in LENGTHS_M {
            let who = pursuer(&b, length, ShipId(2), true);
            let (legs, worst) = escort_legs(&b, &who, -burn * FAR_M, burn);
            assert!(worst <= 1.0, "{length} m: {worst} of the courtesy flux");
            let station = legs.last().unwrap().cruise.to_ly.normalize();
            assert!(station.dot(burn).abs() < 1.0e-9, "{length} m: station at {station}");

            let direct = pursuer(&b, length, ShipId(2), false);
            let (_, flamed) = escort_legs(&b, &direct, -burn * FAR_M, burn);
            assert!(flamed > 1.0, "{length} m: a direct escort from astern only reached {flamed}");
        }
    }

    /// **31 §Tests, Flotillas.** Three followers of one burning leader take three azimuths about
    /// its axis, and none sits in another's cone or the leader's.
    #[test]
    fn three_followers_take_three_stations_out_of_each_others_cones() {
        let b = Balance::DEFAULT;
        let burn = DVec3::new(0.0, 0.6, 0.8);
        let stations: Vec<DVec3> = [3, 4, 5]
            .map(|id| {
                let who = pursuer(&b, 500.0, ShipId(id), true);
                let (legs, worst) = escort_legs(&b, &who, -burn * FAR_M, burn);
                assert!(worst <= 1.0, "follower {id}: {worst} of the courtesy flux");
                legs.last().unwrap().cruise.to_ly * M_PER_LY
            })
            .into();
        let cone = b.drive_spread_rad;
        for (i, a) in stations.iter().enumerate() {
            assert!(a.angle_between(-burn) > cone, "follower {i} is in the leader's exhaust");
            for (j, other) in stations.iter().enumerate().filter(|(j, _)| *j != i) {
                let apart = *other - *a;
                assert!(apart.length() > 1.0, "followers {i} and {j} share a station");
                assert!(apart.angle_between(-burn) > cone, "follower {j} is in {i}'s exhaust");
                assert!(a.angle_between(*other) > 0.5, "followers {i} and {j} are {} apart", a.angle_between(*other));
            }
        }
    }

    /// An escort's leg that ends short of the station is followed by another once it has closed,
    /// and not before; the one ending on the station is kept.
    #[test]
    fn an_escort_moves_on_from_a_leg_only_when_it_has_closed() {
        let b = Balance::DEFAULT;
        let who = pursuer(&b, 500.0, ShipId(2), true);
        let burn = DVec3::X;
        let (legs, _) = escort_legs(&b, &who, -burn * FAR_M, burn);
        let s = standoff(&who, Closeness::Company);
        let replan = Closeness::Company.replan_m(s);
        let wants = |plan: &Escort, tau: f64| {
            let now = plan.quarry.since_t + plan.quarry.world_elapsed(tau);
            let (at, beta) = plan.quarry.at(now);
            let seen = Sighting { target: QUARRY, position_ly: at, beta, length_m: 500.0, emitted_s: now };
            let motive = crate::motion::Motive::Escort(plan.clone());
            pursuit::wants_replan(&motive, &seen, s, replan, &who.arrival, now)
        };
        let (first, last) = (&legs[0], legs.last().unwrap());
        let end = |plan: &Escort| plan.cruise.start_s + plan.cruise.duration_s();
        assert!(!wants(first, 0.5 * (first.cruise.start_s + end(first))), "abandoned part-way");
        assert!(wants(first, end(first) + 1.0), "held at the ingress point");
        assert!(!wants(last, end(last) + 1.0e4), "left the station");
    }

    /// A station further round than a hop from where the pursuer is goes through a waypoint, so
    /// no chord passes nearer the quarry than the leg's own ends allow.
    #[test]
    fn a_station_on_the_far_side_is_reached_round_the_quarry() {
        let b = Balance::DEFAULT;
        let who = pursuer(&b, 500.0, ShipId(2), true);
        let burn = DVec3::X;
        // Dead ahead of the burn and inside the ingress sphere: the station is a quarter turn
        // away and every thruster hop must keep clear.
        let Arrival::Courteous(manners) = who.arrival else { unreachable!() };
        let from = burn * 0.5 * manners.ingress_m;
        let leg = who.arrival.leg(from, burn, Some(burn), standoff(&who, Closeness::Company), who.drive);
        assert!(leg.thrusters);
        assert!(clearance_m(from, leg.to_m) > 0.5 * from.length(), "{}", clearance_m(from, leg.to_m));
    }
}
