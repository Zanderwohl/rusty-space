//! Getting about inside a system.
//!
//! Five things a ship can be asked to do — cross to a point, take up an orbit, sit at a
//! libration point, drop into a belt or a ring, leave the system along its axis — reduce to
//! two: a [`Waypoint`] that says where to be at any coordinate time, and the same
//! [`crate::flight::Cruise`] that crosses between stars, aimed at where that waypoint will be
//! when the ship gets there.
//!
//! The ship is a torch under constant thrust and none of this is orbital mechanics. It does not
//! transfer, it goes: a burn, a flip and a burn, and then it holds station against whatever it
//! was sent to. Orbital speeds are four orders below `c` and a hold is exact rather than
//! integrated, so nothing here drifts and nothing here needs a fuel budget.

use glam::DVec3;
use crate::population::Population;

use crate::flight::{Cruise, Drive};
use crate::system::{LocalSystem, M_PER_LY};

/// What a circular orbit is measured against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Anchor {
    /// The system's primary. A belt orbits this.
    Star,
    /// One body, by the name `em-sim` knows it under.
    Body(String),
}

/// The two collinear libration points a ship would want.
///
/// L3, L4 and L5 are left out: L3 is behind the star and L4/L5 are sixty degrees round the
/// orbit, which is a long way to go to see the same thing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LagrangePoint {
    /// Between the body and its primary.
    L1,
    /// Directly outside the body, in the shadow.
    L2,
}

/// Which way round a body an orbit runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Plane {
    /// In the body's own equator, which is where its rings and most of its moons are.
    #[default]
    Equatorial,
    /// Over the poles, which is the only way to see all of a body.
    Polar,
}

/// A circular orbit, as the three numbers that fix one.
#[derive(Clone, Debug, PartialEq)]
pub struct Orbit {
    pub about: Anchor,
    pub radius_m: f64,
    /// Unit normal of the orbital plane, simulation axes.
    pub pole: DVec3,
    /// Where on the circle the ship is at the world's epoch, radians.
    ///
    /// Set by [`plan`], not by [`Course::resolve`]: which point of an orbit to inject at is a
    /// question about where the ship is coming from, and a course is chosen before that is
    /// known. Left at zero, an orbit is entered at whatever point it happened to be passing,
    /// which can be the far side of the body.
    pub phase_rad: f64,
}

/// A place the ship can be, evaluated at any coordinate time.
#[derive(Clone, Debug, PartialEq)]
pub enum Waypoint {
    /// Fixed, light-years from the world origin. Where a crossing that is simply going
    /// somewhere ends up, and where a ship that has left the system waits.
    Fixed(DVec3),
    Orbit(Orbit),
    Lagrange { body: String, point: LagrangePoint },
}

/// What the interface asks for, before a system has been consulted about whether it exists.
///
/// Separate from [`Waypoint`] because these are the player's words — "low polar orbit of
/// Titan" — and a waypoint is a position. Resolving one into the other is where a body is
/// looked up, a radius is worked out from an altitude, and a plane becomes a pole.
#[derive(Clone, Debug, PartialEq)]
pub enum Course {
    /// Burn, flip and burn to a point, light-years from the world origin.
    To(DVec3),
    /// Circular orbit at an altitude given in radii above the surface.
    Orbit { body: String, altitude_radii: f64, plane: Plane },
    Lagrange { body: String, point: LagrangePoint },
    /// Into the middle of a body's rings, in their own plane.
    Rings(String),
    /// Into a population's band, at the radius that carries its light.
    Belt(usize),
    /// Out along the system's axis until everything is behind.
    LeaveSystem,
}

/// Altitudes the interface offers, in radii above the surface.
///
/// Radii rather than kilometres because the same three numbers then mean the same thing at
/// Deimos and at Jupiter, which differ by four orders in size.
pub const ALTITUDES: [(f64, &str); 3] = [(0.2, "low"), (2.0, "high"), (20.0, "distant")];

/// Where a ring station sits: this many times the outer edge, and this far tipped out of the
/// ring plane.
///
/// Outside the rings, not in them. A ring is drawn as a surface with no thickness, so inside
/// the annulus the sheet passes through the camera: the nearest geometry is at no distance at
/// all, and the camera's own offset from the plane is below what f32 render positions can hold
/// — six metres or so at Saturn. The rings then swing between a hairline and a bright wedge
/// from one frame to the next. Outside the annulus, edge-on is a clean line and the jitter is
/// six metres in a hundred and forty thousand kilometres.
///
/// Tipped, because a ring seen from within its own plane is a line whichever side of it you
/// stand. A quarter turn of tilt opens them, and the orbit still crosses the plane twice a
/// turn to close them again.
pub const RING_STANDOFF: f64 = 1.25;
pub const RING_TILT_RAD: f64 = 0.45;

/// How far past the outer edge of the cloud a departure stops.
///
/// Just outside, not far outside: the point is to be able to look back at the whole system, and
/// the shell is what separates a star drawn as an object from a star drawn as a point.
pub const DEPARTURE_MARGIN: f64 = 1.05;

/// Rounds of aiming at where the target will be rather than where it is.
///
/// Flip-and-burn time goes as the square root of distance, so re-aiming contracts fast: three
/// rounds hold a planet to metres. One round is not enough — Earth moves a fiftieth of an
/// astronomical unit during a crossing from Mars.
pub const ARRIVAL_ROUNDS: usize = 3;

impl Waypoint {
    /// Where this is, light-years from the world origin, in the system as currently propagated.
    ///
    /// `None` when the body it names has gone, which happens when the ship leaves the system.
    pub fn place(&self, system: &LocalSystem) -> Option<DVec3> {
        self.place_at(system, system.time_s())
    }

    /// The same, at a coordinate time of the caller's choosing.
    ///
    /// Bodies are placed analytically rather than read from the arena, so this does not care
    /// where `system`'s clock is. A station is a worldline and this is how it is read.
    pub fn place_at(&self, system: &LocalSystem, seconds: f64) -> Option<DVec3> {
        match self {
            Waypoint::Fixed(at) => Some(*at),
            Waypoint::Orbit(orbit) => {
                let (centre, mu) = orbit.centre_of_at(system, seconds)?;
                if orbit.radius_m <= 0.0 || mu <= 0.0 {
                    return None;
                }
                let theta = orbit.angle_at(seconds, mu);
                let (u, v) = basis(orbit.pole);
                Some(centre + (u * theta.cos() + v * theta.sin()) * orbit.radius_m / M_PER_LY)
            }
            Waypoint::Lagrange { body, point } => {
                let index = system.body_named(body)?;
                let parent = system.sim().parent(index)?;
                let (at_body, _) = system.body_state_at(index, seconds)?;
                let (at_parent, _) = system.body_state_at(parent, seconds)?;
                let offset = at_body - at_parent;
                let distance = offset.length();
                let mass = system.sim().mass(parent);
                if distance <= 0.0 || mass <= 0.0 {
                    return None;
                }
                // The Hill radius. Exact enough: the collinear points sit within a few per cent
                // of it, and the ship is holding station rather than balancing there.
                let hill = distance * (system.sim().mass(index) / (3.0 * mass)).cbrt();
                let sign = match point {
                    LagrangePoint::L1 => -1.0,
                    LagrangePoint::L2 => 1.0,
                };
                let at = at_body + offset / distance * hill * sign;
                Some(system.origin_ly + at / M_PER_LY)
            }
        }
    }

    /// What to call this on screen.
    pub fn label(&self) -> String {
        match self {
            Waypoint::Fixed(_) => "a fixed point".to_string(),
            Waypoint::Orbit(orbit) => match &orbit.about {
                Anchor::Star => format!("a band at {:.1} AU", orbit.radius_m / AU),
                Anchor::Body(name) => format!("orbit of {name}"),
            },
            Waypoint::Lagrange { body, point } => format!("{body} {point:?}"),
        }
    }

    /// How fast the station is moving, metres a second, world frame.
    ///
    /// Differenced rather than differentiated: a waypoint is a closed form but three different
    /// ones, and the same central difference serves all of them. Only cancelling asks for this,
    /// so the two extra propagations cost nothing anyone can see.
    pub fn velocity_at(&self, system: &LocalSystem) -> Option<DVec3> {
        self.velocity_at_time(system, system.time_s())
    }

    /// The same, at a coordinate time of the caller's choosing.
    pub fn velocity_at_time(&self, system: &LocalSystem, seconds: f64) -> Option<DVec3> {
        if matches!(self, Waypoint::Fixed(_)) {
            return Some(DVec3::ZERO);
        }
        let step = self.period_s(system).map(|p| p / 4096.0).unwrap_or(1.0).clamp(1.0e-3, 60.0);
        let before = self.place_at(system, seconds - step)?;
        let after = self.place_at(system, seconds + step)?;
        Some((after - before) * M_PER_LY / (2.0 * step))
    }

    /// The same place, entered at the point of it nearest `from_ly`.
    ///
    /// An orbit is a circle and a ship arriving at it has a nearest point; arriving anywhere
    /// else is a longer crossing to a worse view, and at the far side of the body it is a
    /// crossing straight through the body. Anything that is not a circle has one point and is
    /// returned unchanged.
    ///
    /// `system` must be propagated to the arrival time: the phase is fixed against the clock,
    /// so it is chosen for when the ship gets there rather than for now.
    pub fn nearest_to(&self, from_ly: DVec3, system: &LocalSystem) -> Self {
        let Waypoint::Orbit(orbit) = self else { return self.clone() };
        let Some((centre, mu)) = orbit.centre_of(system) else { return self.clone() };
        let (u, v) = basis(orbit.pole);
        let approach = from_ly - centre;
        // Dropping the component along the pole leaves the direction the circle can actually
        // reach. A ship directly over the pole has no nearest point and gets a fixed one.
        let want = approach.dot(v).atan2(approach.dot(u));
        let mut aimed = orbit.clone();
        aimed.phase_rad = want - orbit.rate(mu) * system.time_s();
        Waypoint::Orbit(aimed)
    }

    /// What the station is *about*: the body or star it was chosen for.
    ///
    /// A ship on station is looking at something, and it is never the station itself. From an
    /// orbit that is the body underneath; from a libration point it is the planet the point
    /// belongs to; from anywhere else it is the star.
    pub fn focus(&self, system: &LocalSystem) -> Option<DVec3> {
        match self {
            Waypoint::Fixed(_) => Some(system.star_position_ly()),
            Waypoint::Orbit(orbit) => Some(orbit.centre_of(system)?.0),
            Waypoint::Lagrange { body, .. } => system.body_position_ly(body),
        }
    }

    /// Seconds the ship takes to go once round, or `None` for anything that does not.
    ///
    /// On screen because the clock runs at eight thousand times real time by default, which
    /// turns a low orbit into a blur: the period is what tells a player which rung to pick.
    pub fn period_s(&self, system: &LocalSystem) -> Option<f64> {
        let Waypoint::Orbit(orbit) = self else { return None };
        let (_, mu) = orbit.centre_of(system)?;
        (mu > 0.0 && orbit.radius_m > 0.0)
            .then(|| std::f64::consts::TAU * (orbit.radius_m.powi(3) / mu).sqrt())
    }
}

impl Orbit {
    /// Angular rate, radians per second.
    pub fn rate(&self, mu: f64) -> f64 {
        if self.radius_m > 0.0 && mu > 0.0 { (mu / self.radius_m.powi(3)).sqrt() } else { 0.0 }
    }

    /// Where round the circle the ship is at a coordinate time.
    pub fn angle_at(&self, seconds: f64, mu: f64) -> f64 {
        self.rate(mu) * seconds + self.phase_rad
    }

    /// Where the orbit is centred, light-years, and the `mu` that sets its rate.
    ///
    /// `G m` of the centre, not `System::mu`, which is the `mu` of the orbit the centre itself
    /// is on — `G(M_sun + M_earth)` for Earth. Using it put a low Earth orbit at eleven seconds.
    fn centre_of(&self, system: &LocalSystem) -> Option<(DVec3, f64)> {
        self.centre_of_at(system, system.time_s())
    }

    fn centre_of_at(&self, system: &LocalSystem, seconds: f64) -> Option<(DVec3, f64)> {
        let index = match &self.about {
            Anchor::Star => system.primary(),
            Anchor::Body(name) => system.body_named(name)?,
        };
        let (at_m, _) = system.body_state_at(index, seconds)?;
        let at = system.origin_ly + at_m / M_PER_LY;
        Some((at, system.sim().gravitational_constant() * system.sim().mass(index)))
    }
}

/// Metres in an astronomical unit.
pub const AU: f64 = 1.495_978_707e11;

/// `pole` tipped by `angle`, about an axis in the plane it is normal to.
pub fn tilt(pole: DVec3, angle: f64) -> DVec3 {
    let (u, _) = basis(pole);
    (pole.normalize_or(DVec3::Z) * angle.cos() + u * angle.sin()).normalize_or(DVec3::Z)
}

/// Two unit vectors spanning the plane normal to `pole`.
///
/// Deterministic, so a ship sent to the same orbit twice arrives at the same place rather than
/// somewhere that depends on how it was asked.
pub fn basis(pole: DVec3) -> (DVec3, DVec3) {
    let n = pole.normalize_or(DVec3::Z);
    // Any fixed vector not parallel to the pole. Z first because most poles are near it and
    // the cross product is then largest.
    let seed = if n.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let u = seed.cross(n).normalize_or(DVec3::X);
    (u, n.cross(u))
}

impl Course {
    /// Turn a request into a place, against the system the ship is in and where it is in it.
    ///
    /// `from_ly` matters for exactly one course — leaving goes straight out from the star,
    /// which is a different direction depending on where you start.
    pub fn resolve(&self, system: &LocalSystem, from_ly: DVec3) -> Option<Waypoint> {
        match self {
            Course::To(at) => Some(Waypoint::Fixed(*at)),
            Course::Orbit { body, altitude_radii, plane } => {
                let index = system.body_named(body)?;
                let radius = system.sim().radius(index);
                if radius <= 0.0 {
                    return None;
                }
                Some(Waypoint::Orbit(Orbit {
                    about: Anchor::Body(body.clone()),
                    radius_m: radius * (1.0 + altitude_radii.max(0.0)),
                    pole: plane.pole_of(system.body_pole(index)),
                    phase_rad: 0.0,
                }))
            }
            Course::Lagrange { body, point } => {
                let index = system.body_named(body)?;
                // A body with no parent has no libration points: there is no second mass.
                system.sim().parent(index)?;
                Some(Waypoint::Lagrange { body: body.clone(), point: *point })
            }
            Course::Rings(body) => {
                let index = system.body_named(body)?;
                let rings = crate::rings::for_body(system.sim().name(index))?;
                Some(Waypoint::Orbit(Orbit {
                    about: Anchor::Body(body.clone()),
                    radius_m: rings.outer_m() * RING_STANDOFF,
                    pole: tilt(system.body_pole(index), RING_TILT_RAD),
                    phase_rad: 0.0,
                }))
            }
            Course::Belt(index) => {
                let population = system.populations.get(*index)?;
                let radius = population.thermal_radius();
                (radius > 0.0).then_some(Waypoint::Orbit(Orbit {
                    about: Anchor::Star,
                    radius_m: radius,
                    pole: population.pole,
                    phase_rad: 0.0,
                }))
            }
            Course::LeaveSystem => {
                let star = system.star_position_ly();
                // Straight out from the star, along the radius the ship is already on. Not
                // along the star's own axis: that is one fixed direction, and leaving should
                // be the shortest way out of the system rather than a trip to the north pole.
                // A ship exactly at the star has no radius to follow and gets the axis.
                let out = (from_ly - star).normalize_or(system.axis());
                Some(Waypoint::Fixed(star + out * system.reach_ly() * DEPARTURE_MARGIN))
            }
        }
    }

    /// What this course is about, for an interface that wants to show it.
    pub fn target(&self) -> Option<Target> {
        match self {
            Course::To(_) | Course::LeaveSystem => None,
            Course::Orbit { body, .. } | Course::Lagrange { body, .. } | Course::Rings(body) => {
                Some(Target::Body(body.clone()))
            }
            Course::Belt(index) => Some(Target::Band(*index)),
        }
    }

    /// Read a course from a development flag: `orbit:Earth`, `polar:Titan:high`,
    /// `rings:Saturn`, `l2:Earth`, `belt:0`, `leave`.
    ///
    /// An orbit takes an optional altitude, named as [`ALTITUDES`] names it. Only `--station`
    /// uses this; it exists so a screenshot of a place can be asked for on a command line
    /// rather than by flying there, and so the spellings are tested.
    pub fn parse(spec: &str) -> Option<Self> {
        let mut fields = spec.split(':');
        let kind = fields.next()?;
        let rest = fields.next().unwrap_or("");
        match kind {
            "leave" => Some(Course::LeaveSystem),
            "belt" => rest.parse().ok().map(Course::Belt),
            "rings" => Some(Course::Rings(rest.to_string())),
            "l1" => Some(Course::Lagrange { body: rest.into(), point: LagrangePoint::L1 }),
            "l2" => Some(Course::Lagrange { body: rest.into(), point: LagrangePoint::L2 }),
            "orbit" | "polar" => {
                let named = fields.next();
                let altitude = match named {
                    Some(name) => ALTITUDES.iter().find(|(_, n)| *n == name)?.0,
                    None => ALTITUDES[0].0,
                };
                Some(Course::Orbit {
                    body: rest.into(),
                    altitude_radii: altitude,
                    plane: if kind == "polar" { Plane::Polar } else { Plane::Equatorial },
                })
            }
            _ => None,
        }
    }
}

impl Plane {
    /// The orbit normal for this plane about a body whose own pole is `pole`.
    pub fn pole_of(&self, pole: DVec3) -> DVec3 {
        match self {
            Plane::Equatorial => pole,
            // Any normal perpendicular to the pole puts the pole in the orbital plane, which
            // is what makes the orbit polar.
            Plane::Polar => basis(pole).0,
        }
    }
}

/// What kind of thing a destination is, out of the body's own tags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Star,
    Planet,
    Moon,
    /// Asteroids, comets, trans-Neptunians and whatever else is loose.
    Minor,
    /// A population: a belt, a swarm or a cloud.
    Band,
}

impl Kind {
    /// From `em-sim`'s tags, which the solar system preset already carries and the generator
    /// already writes. Nothing here re-derives what the data says.
    pub fn of(tags: &[String]) -> Self {
        let has = |t: &str| tags.iter().any(|x| x == t);
        if has("Star") {
            Kind::Star
        } else if has("Planet") || has("Dwarf Planet") {
            Kind::Planet
        } else if has("Moon") {
            Kind::Moon
        } else {
            Kind::Minor
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Kind::Star => "star",
            Kind::Planet => "planet",
            Kind::Moon => "moon",
            Kind::Minor => "minor body",
            Kind::Band => "band",
        }
    }
}

/// What a destination is, as little as the interface needs to name one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Body(String),
    /// A population, by its index in the system's list.
    Band(usize),
}

/// One place in the system, as the interface lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub designation: String,
    pub kind: Kind,
    /// Metres from whatever it orbits, so a moon reads against its planet and a planet against
    /// the star. Taken at load and never recomputed: a list that reorders itself while a player
    /// is reading it is worse than one that is a few per cent stale.
    pub orbit_radius_m: f64,
    /// Steps down from the primary. Indentation, and nothing else.
    pub depth: usize,
    /// One of the bodies a system is usually described by. The solar system has forty-one of
    /// those and a hundred and eighty-nine others.
    pub major: bool,
    pub target: Target,
}

/// What to call a body.
///
/// Its own name first, then whatever catalogue designation it carries, and only then a made-up
/// one: the primary's name and a numeral, which is how an unnamed body has been designated
/// since Galileo. Players will be able to name planets, and that name goes in the first slot.
pub fn designate(
    name: Option<&str>,
    catalogue: Option<&str>,
    primary: &str,
    rank: usize,
) -> String {
    match (name, catalogue) {
        (Some(name), _) if !name.is_empty() => name.to_string(),
        (_, Some(catalogue)) if !catalogue.is_empty() => catalogue.to_string(),
        _ => format!("{primary} {}", roman(rank)),
    }
}

/// `n` in Roman numerals, one-based. Zero and anything past the table fall back to the digits,
/// which is wrong-looking enough to be noticed rather than silently absurd.
pub fn roman(n: usize) -> String {
    const TABLE: [(usize, &str); 13] = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
        (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    let mut left = n;
    let mut out = String::new();
    for (value, glyph) in TABLE {
        while left >= value {
            out.push_str(glyph);
            left -= value;
        }
    }
    out
}

/// Every course a destination offers, with what to call it.
///
/// The list is what exists, not what is sensible: a body with no parent has no libration points
/// and one without rings has nowhere above them, and neither appears rather than appearing
/// greyed. Whether a course can be flown is [`Course::resolve`]'s business and it is asked
/// again when the player presses Go.
pub fn options_for(system: &LocalSystem, target: &Target) -> Vec<(String, Course)> {
    let mut out = Vec::new();
    let Target::Body(name) = target else {
        let Target::Band(index) = target else { return out };
        out.push(("into the band".to_string(), Course::Belt(*index)));
        return out;
    };
    let Some(index) = system.body_named(name) else { return out };

    for (plane, what) in [(Plane::Equatorial, "equatorial"), (Plane::Polar, "polar")] {
        for (altitude_radii, height) in ALTITUDES {
            out.push((
                format!("{what} orbit, {height}"),
                Course::Orbit { body: name.clone(), altitude_radii, plane },
            ));
        }
    }
    if system.sim().parent(index).is_some() {
        for (point, what) in
            [(LagrangePoint::L1, "L1 companion"), (LagrangePoint::L2, "L2 companion")]
        {
            out.push((what.to_string(), Course::Lagrange { body: name.clone(), point }));
        }
    }
    if crate::rings::for_body(system.sim().name(index)).is_some() {
        out.push(("above the rings".to_string(), Course::Rings(name.clone())));
    }
    if index == system.primary() {
        out.push(("leave the system".to_string(), Course::LeaveSystem));
    }
    out
}

/// Plan a crossing to where a waypoint will be when the ship arrives, and where on it.
///
/// A fixed point needs one pass; anything that moves needs the arrival time, which depends on
/// the distance, which depends on the arrival time. Iterated rather than solved: the map is a
/// strong contraction and [`ARRIVAL_ROUNDS`] of it is exact to well under a body's radius.
///
/// The waypoint comes back aimed — an orbit's phase set so the ship meets it at the point
/// nearest where it started. That is part of the same fixed point: the nearest point depends
/// on the arrival time as much as the body's position does.
pub fn plan(
    system: &LocalSystem,
    waypoint: &Waypoint,
    from_ly: DVec3,
    start_s: f64,
    drive: Drive,
) -> Option<(Cruise, Waypoint)> {
    let mut aimed = waypoint.clone();
    let mut target = aimed.place(system)?;
    let mut cruise = Cruise::plan(from_ly, target, start_s, drive);
    if matches!(waypoint, Waypoint::Fixed(_)) {
        return Some((cruise, aimed));
    }
    for _ in 0..ARRIVAL_ROUNDS {
        let arrival = system.propagated_to(start_s + cruise.duration_s());
        aimed = waypoint.nearest_to(from_ly, &arrival);
        let Some(next) = aimed.place(&arrival) else { break };
        if next == target {
            break;
        }
        target = next;
        cruise = Cruise::plan(from_ly, target, start_s, drive);
    }
    Some((cruise, aimed))
}

/// What a population is, from its shape alone.
///
/// A belt is flat and a cloud is not, and nothing else about a population distinguishes them.
/// Used for naming one on screen; the Oort analogue is the only isotropic one a system has.
pub fn is_flat(population: &Population) -> bool {
    population.inclination.max_inclination() < 1.0
}

#[cfg(test)]
mod tests {
    use crate::sky::{AuthoredStars, StarProvider};

    use super::*;

    fn sol() -> LocalSystem {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .expect("the catalogue");
        let sun = provider
            .stars()
            .iter()
            .find(|s| s.name.as_deref() == Some("Sol"))
            .expect("the Sun")
            .clone();
        let mut system = LocalSystem::for_star(&sun).expect("the solar system");
        system.advance_to(0.0);
        system
    }

    fn generated() -> LocalSystem {
        let stars = AuthoredStars::sample();
        LocalSystem::for_star(&stars.stars()[0]).expect("a generated system")
    }

    /// An altitude in radii is an altitude above the *surface*, which is the only reading of it
    /// that means the same thing at Deimos and at Jupiter.
    #[test]
    fn an_orbit_sits_where_its_altitude_says() {
        let system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: Plane::Equatorial };
        let waypoint = course.resolve(&system, DVec3::ZERO).expect("Earth has an orbit");
        let at = waypoint.place(&system).expect("a position");
        let earth = system.body_position_ly("Earth").expect("Earth");
        let radius = at.distance(earth) * M_PER_LY;
        let expected = 6.371e6 * 3.0;
        assert!((radius / expected - 1.0).abs() < 0.02, "{radius:e} against {expected:e}");
    }

    /// A polar orbit puts the pole in the orbital plane; an equatorial one puts it normal to it.
    /// Getting this backwards is invisible in a still and obvious in motion.
    #[test]
    fn a_polar_orbit_crosses_the_pole_and_an_equatorial_one_does_not() {
        let system = sol();
        let pole = system.body_pole(system.body_named("Earth").unwrap());
        let of = |plane| {
            let course = Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane };
            let Waypoint::Orbit(orbit) = course.resolve(&system, DVec3::ZERO).unwrap() else { panic!() };
            orbit.pole.dot(pole).abs()
        };
        assert!(of(Plane::Equatorial) > 0.999, "an equatorial normal is the pole");
        assert!(of(Plane::Polar) < 1.0e-9, "a polar normal is perpendicular to it");
    }

    /// An orbit is circular, so every sample of it is the same distance out and they are not
    /// all the same place. The second half is what catches a rate of zero.
    #[test]
    fn an_orbit_goes_round() {
        let mut system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 0.2, plane: Plane::Equatorial };
        let waypoint = course.resolve(&system, DVec3::ZERO).unwrap();
        let period = waypoint.period_s(&system).expect("a period");
        // Low Earth orbit is about ninety minutes; at 1.2 radii it is a little longer.
        assert!((5000.0..9000.0).contains(&period), "{period} seconds");

        let mut seen = Vec::new();
        for step in 0..4 {
            system.advance_to(period * step as f64 / 4.0);
            let at = waypoint.place(&system).unwrap();
            let centre = system.body_position_ly("Earth").unwrap();
            seen.push((at, at.distance(centre) * M_PER_LY));
        }
        let radius = seen[0].1;
        for (_, r) in &seen {
            assert!((r / radius - 1.0).abs() < 1e-9, "{r} against {radius}");
        }
        let quarter = seen[0].0.distance(seen[1].0) * M_PER_LY;
        assert!(quarter > radius, "a quarter turn moves further than the radius is long");
    }

    /// L1 is sunward of the body and L2 is behind it, both at about the Hill radius. Earth's is
    /// 1.5 million kilometres, which is a number people know.
    #[test]
    fn the_libration_points_straddle_the_body() {
        let system = sol();
        let earth = system.body_position_ly("Earth").unwrap();
        let star = system.star_position_ly();
        let at = |point| {
            Waypoint::Lagrange { body: "Earth".into(), point }.place(&system).expect("a point")
        };
        let (l1, l2) = (at(LagrangePoint::L1), at(LagrangePoint::L2));
        let out = (earth - star).normalize();
        assert!((l1 - earth).dot(out) < 0.0, "L1 is on the sunward side");
        assert!((l2 - earth).dot(out) > 0.0, "L2 is on the far side");
        for point in [l1, l2] {
            let hill = point.distance(earth) * M_PER_LY;
            assert!((hill / 1.5e9 - 1.0).abs() < 0.1, "{hill:e} is not the Hill radius");
        }
    }

    /// Aiming at where a body is rather than where it will be is the whole of the error here,
    /// and it is not small: Earth runs a fiftieth of an astronomical unit during a crossing.
    #[test]
    fn a_crossing_leads_a_moving_target() {
        let system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: Plane::Equatorial };
        let waypoint = course.resolve(&system, DVec3::ZERO).unwrap();
        let from = system.body_position_ly("Mars").expect("Mars");
        let (cruise, waypoint) = plan(&system, &waypoint, from, 0.0, Drive::DEFAULT).expect("a crossing");

        let arrival = system.propagated_to(cruise.duration_s());
        let wanted = waypoint.place(&arrival).unwrap();
        let landed = cruise.at(cruise.duration_s()).position_ly;
        let miss = landed.distance(wanted) * M_PER_LY;
        assert!(miss < 6.371e6, "missed by {miss:e} metres, more than a planetary radius");

        // And the naive aim is far worse, which is why the rounds are there.
        let naive = Cruise::plan(from, waypoint.place(&system).unwrap(), 0.0, Drive::DEFAULT);
        let naive_miss = naive.at(naive.duration_s()).position_ly.distance(wanted) * M_PER_LY;
        assert!(naive_miss > miss * 100.0, "{naive_miss:e} against {miss:e}");
    }

    /// Leaving goes straight out from the star along the radius the ship is already on, and
    /// ends up outside the shell that makes a star a local object.
    ///
    /// Not along the star's axis. That is one fixed direction whatever the ship is doing, so
    /// every departure rose out of the ecliptic instead of taking the shortest way out.
    #[test]
    fn leaving_the_system_goes_straight_out_from_the_star() {
        let system = sol();
        let star = system.star_position_ly();
        let leave = |from: DVec3| {
            let Waypoint::Fixed(out) = Course::LeaveSystem.resolve(&system, from).unwrap() else {
                panic!("leaving ends at a fixed point")
            };
            out - star
        };
        for body in ["Earth", "Jupiter", "Neptune"] {
            let from = system.body_position_ly(body).unwrap();
            let offset = leave(from);
            let radius = (from - star).normalize();
            assert!(offset.normalize().dot(radius) > 0.999, "{body} did not leave radially");
            assert!(offset.length() > crate::system::LOCAL_SHELL_LY, "still inside the shell");
        }
        // Two bodies on opposite sides of the star leave in opposite directions, which is the
        // whole difference from a fixed axis.
        let earth = leave(system.body_position_ly("Earth").unwrap()).normalize();
        let opposite = leave(star - (system.body_position_ly("Earth").unwrap() - star)).normalize();
        assert!(earth.dot(opposite) < -0.999, "both departures went the same way");

        // A ship at the star itself has no radius to follow, and falls back to the axis.
        assert!(leave(star).normalize().dot(system.axis()) > 0.999);
    }

    /// An orbit is a circle, and a ship arriving at one should meet it at the near side.
    /// Injecting at whatever point the clock had it can mean crossing the body to get there.
    #[test]
    fn a_course_injects_at_the_nearest_point_of_the_orbit() {
        let system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: Plane::Equatorial };
        let waypoint = course.resolve(&system, DVec3::ZERO).unwrap();
        let from = system.body_position_ly("Mars").expect("Mars");
        let (cruise, aimed) = plan(&system, &waypoint, from, 0.0, Drive::DEFAULT).expect("a plan");

        let arrival = system.propagated_to(cruise.duration_s());
        let landed = aimed.place(&arrival).unwrap();
        let reach = from.distance(landed);

        // The nearest point, checked against the rest of the circle rather than against a
        // formula: no other phase of the same orbit is closer to where the ship started.
        let Waypoint::Orbit(orbit) = &aimed else { panic!("an orbit") };
        let mut furthest: f64 = 0.0;
        for step in 1..36 {
            let mut elsewhere = orbit.clone();
            elsewhere.phase_rad += std::f64::consts::TAU * step as f64 / 36.0;
            let other = Waypoint::Orbit(elsewhere).place(&arrival).unwrap();
            let range = from.distance(other);
            assert!(range >= reach * (1.0 - 1e-9), "{range:e} beats the chosen {reach:e}");
            furthest = furthest.max(range);
        }
        // And the far side is a whole diameter further, which is what injecting blind costs.
        let earth = arrival.body_position_ly("Earth").unwrap();
        let diameter = landed.distance(earth) * 2.0;
        assert!(furthest - reach > diameter * 0.9, "the circle is not being crossed at all");
    }

    /// Aiming decides a phase, and the ship still goes round afterwards: the injection point
    /// is where the orbit *starts*, not a place the ship stops.
    #[test]
    fn an_aimed_orbit_still_turns() {
        let mut system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: Plane::Equatorial };
        let waypoint = course
            .resolve(&system, DVec3::ZERO)
            .unwrap()
            .nearest_to(system.body_position_ly("Mars").unwrap(), &system);
        let period = waypoint.period_s(&system).unwrap();
        let at_start = waypoint.place(&system).unwrap();
        system.advance_to(period * 0.5);
        let half_way = waypoint.place(&system).unwrap();
        let earth = system.body_position_ly("Earth").unwrap();
        assert!(
            half_way.distance(at_start) > at_start.distance(earth),
            "half a turn should be most of the way across the orbit",
        );
    }

    /// The ring course has to stay clear of the annulus, and out of its plane.
    ///
    /// Inside it, the sheet passes through the camera and the rings flickered between a
    /// hairline and a wedge every frame; in its plane they are a line however far off you are.
    #[test]
    fn a_ring_course_stands_outside_the_rings_and_out_of_their_plane() {
        let system = sol();
        let Waypoint::Orbit(orbit) = Course::Rings("Saturn".into()).resolve(&system, DVec3::ZERO).unwrap()
        else {
            panic!("rings are an orbit")
        };
        let rings = crate::rings::for_body("Saturn").expect("Saturn has rings");
        assert!(orbit.radius_m > rings.outer_m(), "inside the annulus is the degenerate case");

        // The orbit's own normal is tilted from the ring pole, so the station rises out of the
        // plane rather than running along it.
        let pole = system.body_pole(system.body_named("Saturn").unwrap());
        let tipped = orbit.pole.dot(pole).clamp(-1.0, 1.0).acos();
        assert!((tipped - RING_TILT_RAD).abs() < 1e-9, "tilted by {tipped}");

        // And the highest it gets is a real fraction of the way out of the plane.
        let highest = orbit.radius_m * RING_TILT_RAD.sin();
        assert!(highest > rings.outer_m() * 0.4, "only {highest:e} above the rings");
    }

    #[test]
    fn a_body_without_rings_or_a_parent_refuses_rather_than_guessing() {
        let system = sol();
        assert!(Course::Rings("Earth".into()).resolve(&system, DVec3::ZERO).is_none());
        assert!(Course::Rings("Nowhere".into()).resolve(&system, DVec3::ZERO).is_none());
        assert!(Course::Orbit { body: "Nowhere".into(), altitude_radii: 1.0, plane: Plane::Polar }
            .resolve(&system, DVec3::ZERO)
            .is_none());
        // The primary is the one body with nothing to librate against.
        let star = system.sim().name(system.primary()).to_string();
        assert!(Course::Lagrange { body: star, point: LagrangePoint::L1 }.resolve(&system, DVec3::ZERO).is_none());
    }

    /// A generated system has no measured data in it at all, and every course still has to
    /// resolve or refuse cleanly.
    #[test]
    fn a_generated_system_navigates_too() {
        let system = generated();
        assert!(Course::LeaveSystem.resolve(&system, DVec3::ZERO).is_some());
        let belts = (0..system.populations.len())
            .filter_map(|i| Course::Belt(i).resolve(&system, DVec3::ZERO))
            .count();
        assert_eq!(belts, system.populations.len(), "every population is somewhere to go");
        let flat = system.populations.iter().filter(|p| is_flat(p)).count();
        assert_eq!(flat, system.populations.len() - 1, "all but the cloud are flat");
    }

    #[test]
    fn a_course_spelling_means_what_it_says() {
        let system = sol();
        assert_eq!(Course::parse("leave"), Some(Course::LeaveSystem));
        assert_eq!(Course::parse("belt:1"), Some(Course::Belt(1)));
        assert_eq!(Course::parse("rings:Saturn"), Some(Course::Rings("Saturn".into())));
        assert!(matches!(
            Course::parse("l2:Earth"),
            Some(Course::Lagrange { point: LagrangePoint::L2, .. })
        ));
        let Some(Course::Orbit { plane, .. }) = Course::parse("polar:Earth") else {
            panic!("polar is an orbit")
        };
        assert_eq!(plane, Plane::Polar);
        assert_eq!(Course::parse("nonsense:Earth"), None);
        let Some(Course::Orbit { altitude_radii, .. }) = Course::parse("orbit:Earth:distant")
        else {
            panic!("an altitude may be named")
        };
        assert_eq!(altitude_radii, 20.0);
        assert_eq!(Course::parse("orbit:Earth:enormous"), None, "and only as the table names it");
        assert!(Course::parse("belt:9").unwrap().resolve(&system, DVec3::ZERO).is_none(), "and out of range");
    }

    /// The list a player reads: outward from the star, with each planet's moons behind it.
    #[test]
    fn the_inventory_runs_outward_and_keeps_moons_with_their_planet() {
        let system = sol();
        let inventory = system.inventory();
        let named = |name: &str| {
            inventory.iter().position(|e| e.designation == name).unwrap_or_else(|| {
                panic!("{name} is not in the inventory")
            })
        };
        // The primary comes first, then the planets by their own distance.
        assert_eq!(inventory[0].kind, Kind::Star);
        for pair in ["Mercury", "Venus", "Earth", "Mars", "Jupiter", "Saturn"].windows(2) {
            assert!(named(pair[0]) < named(pair[1]), "{} is not inside {}", pair[0], pair[1]);
        }
        // A moon sits behind its planet and before the next one out.
        assert!(named("Jupiter") < named("Io"));
        assert!(named("Io") < named("Europa"), "and moons run outward too");
        assert!(named("Europa") < named("Ganymede"));
        assert!(named("Ganymede") < named("Saturn"));
        // The asteroid belt lands where it belongs, between Mars and Jupiter.
        let belt = inventory
            .iter()
            .position(|e| e.kind == Kind::Band && e.designation.starts_with("belt"))
            .expect("a belt");
        assert!(named("Mars") < belt && belt < named("Jupiter"), "the belt is misplaced");
        // And a moon is one step down from a planet, which is what indents it.
        assert_eq!(inventory[named("Jupiter")].depth, 1);
        assert_eq!(inventory[named("Io")].depth, 2);
    }

    /// Kinds come from the data's own tags rather than from anything re-derived here.
    #[test]
    fn the_inventory_knows_what_each_thing_is() {
        let system = sol();
        let kind = |name: &str| {
            system.inventory().iter().find(|e| e.designation == name).map(|e| e.kind)
        };
        assert_eq!(kind("Sol"), Some(Kind::Star));
        assert_eq!(kind("Earth"), Some(Kind::Planet));
        assert_eq!(kind("Luna"), Some(Kind::Moon));
        assert_eq!(kind("Pluto"), Some(Kind::Planet), "a dwarf planet is still a planet to fly to");
        // Forty-one of the solar system's two hundred and thirty bodies are the ones it is
        // usually described by, and a list of all of them is unreadable without that.
        let major = system.inventory().iter().filter(|e| e.major).count();
        assert!((30..80).contains(&major), "{major} major bodies is not a system summary");
        assert!(system.inventory().len() > 200, "and the rest are still there to be asked for");
    }

    /// A body is called what it is called; only one with nothing at all gets invented a name.
    #[test]
    fn a_designation_prefers_the_name_then_the_catalogue_then_a_numeral() {
        assert_eq!(designate(Some("Titan"), Some("S VI"), "Saturn", 6), "Titan");
        assert_eq!(designate(None, Some("S/2004 S 13"), "Saturn", 40), "S/2004 S 13");
        assert_eq!(designate(None, None, "Saturn", 7), "Saturn VII");
        assert_eq!(designate(Some(""), None, "Kepler", 3), "Kepler III", "empty is not a name");
    }

    #[test]
    fn roman_numerals_are_roman() {
        for (n, want) in [(1, "I"), (4, "IV"), (9, "IX"), (14, "XIV"), (40, "XL"), (1987, "MCMLXXXVII")] {
            assert_eq!(roman(n), want);
        }
        // Out of range falls back to digits rather than to nonsense.
        assert_eq!(roman(0), "0");
        assert_eq!(roman(4000), "4000");
    }

    /// What is offered is what exists: no libration points without a parent, no rings without
    /// rings, and leaving the system is the star's own option.
    #[test]
    fn the_options_offered_are_the_ones_that_exist() {
        let system = sol();
        let labels = |name: &str| {
            options_for(&system, &Target::Body(name.into()))
                .into_iter()
                .map(|(label, _)| label)
                .collect::<Vec<_>>()
        };
        let earth = labels("Earth");
        assert!(earth.contains(&"equatorial orbit, low".to_string()));
        assert!(earth.contains(&"polar orbit, distant".to_string()));
        assert!(earth.contains(&"L1 companion".to_string()));
        assert!(!earth.iter().any(|l| l.contains("rings")));

        assert!(labels("Saturn").contains(&"above the rings".to_string()));

        let star = labels("Sol");
        assert!(star.contains(&"leave the system".to_string()));
        assert!(!star.iter().any(|l| l.contains("companion")), "the primary has no parent");

        assert_eq!(labels("Nowhere"), Vec::<String>::new());
        let band = options_for(&system, &Target::Band(0));
        assert_eq!(band.len(), 1);
        assert!(band[0].1.resolve(&system, DVec3::ZERO).is_some(), "and it is a course that resolves");
    }

    /// Every option the interface offers has to resolve, or a Go button lies.
    #[test]
    fn every_offered_option_resolves_into_somewhere() {
        let system = sol();
        for entry in system.inventory().iter().filter(|e| e.major) {
            for (label, course) in options_for(&system, &entry.target) {
                assert!(
                    course.resolve(&system, DVec3::ZERO).is_some(),
                    "{} offers {label} and it does not resolve",
                    entry.designation
                );
            }
        }
    }


    /// The basis has to be orthonormal whatever it is handed, including a degenerate pole.    /// The basis has to be orthonormal whatever it is handed, including a degenerate pole.
    #[test]
    fn the_orbital_basis_is_orthonormal_everywhere() {
        for pole in [DVec3::Z, DVec3::X, -DVec3::Z, DVec3::ZERO, DVec3::new(1.0, 1.0, 1.0)] {
            let (u, v) = basis(pole);
            assert!((u.length() - 1.0).abs() < 1e-12, "{pole} gave {u}");
            assert!((v.length() - 1.0).abs() < 1e-12, "{pole} gave {v}");
            assert!(u.dot(v).abs() < 1e-12, "{pole} gave a skew basis");
        }
    }
}
