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

impl LagrangePoint {
    /// Which way along the parent-to-body direction the point lies: inward for L1, outward
    /// for L2.
    pub fn outward_sign(self) -> f64 {
        match self {
            LagrangePoint::L1 => -1.0,
            LagrangePoint::L2 => 1.0,
        }
    }

    pub fn collinear(self) -> em_foundations::lagrange::Collinear {
        match self {
            LagrangePoint::L1 => em_foundations::lagrange::Collinear::L1,
            LagrangePoint::L2 => em_foundations::lagrange::Collinear::L2,
        }
    }

    /// How far from `body` its point of this kind sits, meters, at a coordinate time.
    ///
    /// The root of Lagrange's quintic, not the Hill radius. The two differ by a third of a per
    /// cent at Sun-Earth — five thousand kilometers — and, worse, the Hill radius is the *same*
    /// number for L1 and L2, which puts them symmetrically either side of the body. They are
    /// not symmetric: L2 is the further out.
    pub fn standoff_m(self, system: &LocalSystem, body: &str, seconds: f64) -> Option<f64> {
        let index = system.body_named(body)?;
        let parent = system.sim().parent(index)?;
        let (at_body, _) = system.body_state_at(index, seconds)?;
        let (at_parent, _) = system.body_state_at(parent, seconds)?;
        let distance = (at_body - at_parent).length();
        let mass = system.sim().mass(index);
        let ratio = mass / (mass + system.sim().mass(parent));
        let gamma = em_foundations::lagrange::gamma(ratio, self.collinear())?;
        (distance > 0.0).then_some(gamma * distance)
    }
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
    /// A libration orbit *about* a collinear point, which is what a craft there actually
    /// flies. See [`crate::libration`].
    Libration(crate::libration::Libration),
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
    /// A libration orbit about L1 or L2, rather than the point itself. What a real mission
    /// flies, and somewhere you can see out from.
    Hangout { body: String, point: LagrangePoint },
    /// Into the middle of a body's rings, in their own plane.
    Rings(String),
    /// Into a population's band, at the radius that carries its light.
    Belt(usize),
    /// Out along the system's axis until everything is behind.
    LeaveSystem,
}

/// The wire's course, and the world's, converted.
///
/// Both matches are exhaustive on purpose: a course the world gains and the protocol has not
/// learned will not compile until the protocol learns it. That is the only thing keeping a
/// mirrored type honest, and it costs nothing to have.
///
/// Total in both directions. A course is a request, and every request the wire can express is
/// one the world can name — whether the *system* has anything answering to it is
/// [`Course::resolve`]'s business, and its refusal is what a client is told.
impl From<&Course> for lc_proto::Course {
    fn from(course: &Course) -> Self {
        match course {
            Course::To(at) => lc_proto::Course::To(at.to_array()),
            Course::Orbit { body, altitude_radii, plane } => lc_proto::Course::Orbit {
                body: body.clone(),
                altitude_radii: *altitude_radii,
                plane: (*plane).into(),
            },
            Course::Lagrange { body, point } => {
                lc_proto::Course::Lagrange { body: body.clone(), point: (*point).into() }
            }
            Course::Hangout { body, point } => {
                lc_proto::Course::Hangout { body: body.clone(), point: (*point).into() }
            }
            Course::Rings(body) => lc_proto::Course::Rings { body: body.clone() },
            // A system with four billion populations is not a system. Saturating rather than
            // wrapping, so a nonsense index refuses at `resolve` instead of naming a real band.
            Course::Belt(index) => {
                lc_proto::Course::Belt { index: u32::try_from(*index).unwrap_or(u32::MAX) }
            }
            Course::LeaveSystem => lc_proto::Course::LeaveSystem,
        }
    }
}

impl From<lc_proto::Course> for Course {
    fn from(course: lc_proto::Course) -> Self {
        match course {
            lc_proto::Course::To(at) => Course::To(DVec3::from_array(at)),
            lc_proto::Course::Orbit { body, altitude_radii, plane } => {
                Course::Orbit { body, altitude_radii, plane: plane.into() }
            }
            lc_proto::Course::Lagrange { body, point } => {
                Course::Lagrange { body, point: point.into() }
            }
            lc_proto::Course::Hangout { body, point } => {
                Course::Hangout { body, point: point.into() }
            }
            lc_proto::Course::Rings { body } => Course::Rings(body),
            lc_proto::Course::Belt { index } => Course::Belt(index as usize),
            lc_proto::Course::LeaveSystem => Course::LeaveSystem,
        }
    }
}

impl From<Plane> for lc_proto::Plane {
    fn from(plane: Plane) -> Self {
        match plane {
            Plane::Equatorial => lc_proto::Plane::Equatorial,
            Plane::Polar => lc_proto::Plane::Polar,
        }
    }
}

impl From<lc_proto::Plane> for Plane {
    fn from(plane: lc_proto::Plane) -> Self {
        match plane {
            lc_proto::Plane::Equatorial => Plane::Equatorial,
            lc_proto::Plane::Polar => Plane::Polar,
        }
    }
}

impl From<LagrangePoint> for lc_proto::LagrangePoint {
    fn from(point: LagrangePoint) -> Self {
        match point {
            LagrangePoint::L1 => lc_proto::LagrangePoint::L1,
            LagrangePoint::L2 => lc_proto::LagrangePoint::L2,
        }
    }
}

impl From<lc_proto::LagrangePoint> for LagrangePoint {
    fn from(point: lc_proto::LagrangePoint) -> Self {
        match point {
            lc_proto::LagrangePoint::L1 => LagrangePoint::L1,
            lc_proto::LagrangePoint::L2 => LagrangePoint::L2,
        }
    }
}

/// Altitudes the interface offers, in radii above the surface.
///
/// Radii rather than kilometers because the same three numbers then mean the same thing at
/// Deimos and at Jupiter, which differ by four orders in size.
pub const ALTITUDES: [(f64, &str); 3] = [(0.2, "low"), (2.0, "high"), (20.0, "distant")];

/// Where a ring station sits: this many times the outer edge, and this far tipped out of the
/// ring plane.
///
/// Outside the rings, not in them. A ring is drawn as a surface with no thickness, so inside
/// the annulus the sheet passes through the camera: the nearest geometry is at no distance at
/// all, and the camera's own offset from the plane is below what f32 render positions can hold
/// — six meters or so at Saturn. The rings then swing between a hairline and a bright wedge
/// from one frame to the next. Outside the annulus, edge-on is a clean line and the jitter is
/// six meters in a hundred and forty thousand kilometers.
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
/// rounds hold a planet to meters. One round is not enough — Earth moves a fiftieth of an
/// astronomical unit during a crossing from Mars.
pub const ARRIVAL_ROUNDS: usize = 3;

impl Waypoint {
    /// Where this is at a coordinate time, light-years from the world origin.
    ///
    /// Bodies are placed analytically rather than read from the arena, so this does not care
    /// where `system`'s clock is — and there is deliberately no form that defaults to it. A
    /// station is a worldline, every caller means a particular instant, and the one that
    /// defaulted was how "the system must be propagated first" became a caller's problem.
    ///
    /// `None` when the body it names has gone, which happens when the ship leaves the system.
    pub fn place_at(&self, system: &LocalSystem, seconds: f64) -> Option<DVec3> {
        match self {
            Waypoint::Fixed(at) => Some(*at),
            Waypoint::Orbit(orbit) => {
                let (center, mu) = orbit.center_of_at(system, seconds)?;
                if orbit.radius_m <= 0.0 || mu <= 0.0 {
                    return None;
                }
                let theta = orbit.angle_at(seconds, mu);
                let (u, v) = basis(orbit.pole);
                Some(center + (u * theta.cos() + v * theta.sin()) * orbit.radius_m / M_PER_LY)
            }
            Waypoint::Libration(libration) => libration.at(system, seconds),
            Waypoint::Lagrange { body, point } => {
                let index = system.body_named(body)?;
                let parent = system.sim().parent(index)?;
                let (at_body, _) = system.body_state_at(index, seconds)?;
                let (at_parent, _) = system.body_state_at(parent, seconds)?;
                let out = (at_body - at_parent).normalize_or_zero();
                let standoff = point.standoff_m(system, body, seconds)?;
                let at = at_body + out * standoff * point.outward_sign();
                Some(system.origin_ly + at / M_PER_LY)
            }
        }
    }

    /// What to call this on screen.
    /// What to call this station, in the crew's own names.
    ///
    /// **Takes [`Labels`] rather than formatting the key**, because a waypoint holds the
    /// generator's key for whatever it is anchored to and that is not a name anybody aboard
    /// knows. A ship sent to `180.5-00.1` must not be reported as holding at `99942-Apophis`.
    /// See [`crate::labels`].
    pub fn label(&self, labels: &crate::labels::Labels) -> String {
        match self {
            Waypoint::Fixed(_) => "a fixed point".to_string(),
            Waypoint::Orbit(orbit) => match &orbit.about {
                Anchor::Star => format!("a band at {:.1} AU", orbit.radius_m / AU),
                Anchor::Body(key) => format!("orbit of {}", labels.of(key)),
            },
            Waypoint::Lagrange { body, point } => format!("{} {point:?}", labels.of(body)),
            Waypoint::Libration(l) => format!("{} {:?} libration", labels.of(&l.body), l.point),
        }
    }

    /// How fast the station is moving, meters a second, world frame.
    ///
    /// **An orbit's is analytic**: the body's velocity as em-sim states it, plus the circle's.
    /// That is the convention [`crate::coast::Coast`] reads and writes a velocity in, so a ship
    /// cutting its drive on a station goes onto that very circle, and a craft reckoning a quarry
    /// on one along a conic — see [`crate::consort`] — stays on it. Differencing positions
    /// instead read em-sim's own inconsistency between the two back in (nine meters a second
    /// about Jupiter), and at four light-years out, where a light-year coordinate is only good to
    /// eight meters, it read a meter and a half a second of rounding as well.
    ///
    /// Anything else is differenced: a closed form, but several different ones, and the same
    /// central difference serves all of them.
    pub fn velocity_at(&self, system: &LocalSystem, seconds: f64) -> Option<DVec3> {
        match self {
            Waypoint::Fixed(_) => return Some(DVec3::ZERO),
            Waypoint::Orbit(orbit) => return orbit.velocity_at(system, seconds),
            Waypoint::Lagrange { .. } | Waypoint::Libration(_) => {}
        }
        let step =
            self.period_s(system, seconds).map(|p| p / 4096.0).unwrap_or(1.0).clamp(1.0e-3, 60.0);
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
    /// `at_s` is the *arrival* time, not now: the phase is fixed against the clock, so the
    /// nearest point is the one the orbit will be presenting when the ship gets there.
    pub fn nearest_to(&self, from_ly: DVec3, system: &LocalSystem, at_s: f64) -> Self {
        let Waypoint::Orbit(orbit) = self else { return self.clone() };
        let Some((center, mu)) = orbit.center_of_at(system, at_s) else { return self.clone() };
        let (u, v) = basis(orbit.pole);
        let approach = from_ly - center;
        // Dropping the component along the pole leaves the direction the circle can actually
        // reach. A ship directly over the pole has no nearest point and gets a fixed one.
        let want = approach.dot(v).atan2(approach.dot(u));
        let mut aimed = orbit.clone();
        aimed.phase_rad = want - orbit.rate(mu) * at_s;
        Waypoint::Orbit(aimed)
    }

    /// What the station is *about*: the body or star it was chosen for.
    ///
    /// A ship on station is looking at something, and it is never the station itself. From an
    /// orbit that is the body underneath; from a libration point it is the planet the point
    /// belongs to; from anywhere else it is the star.
    pub fn focus(&self, system: &LocalSystem, seconds: f64) -> Option<DVec3> {
        match self {
            Waypoint::Fixed(_) => system.star_position_at(seconds),
            Waypoint::Orbit(orbit) => Some(orbit.center_of_at(system, seconds)?.0),
            Waypoint::Lagrange { body, .. } => system.body_position_at(body, seconds),
            Waypoint::Libration(l) => system.body_position_at(&l.body, seconds),
        }
    }

    /// Seconds the ship takes to go once round, or `None` for anything that does not.
    ///
    /// On screen because the clock runs at eight thousand times real time by default, which
    /// turns a low orbit into a blur: the period is what tells a player which rung to pick.
    pub fn period_s(&self, system: &LocalSystem, seconds: f64) -> Option<f64> {
        if let Waypoint::Libration(libration) = self {
            return Some(libration.period_s());
        }
        let Waypoint::Orbit(orbit) = self else { return None };
        let (_, mu) = orbit.center_of_at(system, seconds)?;
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

    /// How fast a craft on it is moving, meters a second, world frame. See
    /// [`Waypoint::velocity_at`].
    fn velocity_at(&self, system: &LocalSystem, seconds: f64) -> Option<DVec3> {
        let (_, mu) = self.center_of_at(system, seconds)?;
        let index = self.center_index(system)?;
        let (_, carried) = system.body_state_at(index, seconds)?;
        if self.radius_m <= 0.0 || mu <= 0.0 {
            return None;
        }
        let theta = self.angle_at(seconds, mu);
        let (u, v) = basis(self.pole);
        let along = v * theta.cos() - u * theta.sin();
        Some(carried + along * self.rate(mu) * self.radius_m)
    }

    fn center_index(&self, system: &LocalSystem) -> Option<em_sim::id::BodyIndex> {
        match &self.about {
            Anchor::Star => Some(system.primary()),
            Anchor::Body(name) => system.body_named(name),
        }
    }

    /// Where the orbit is centered, light-years, and the `mu` that sets its rate.
    ///
    /// `G m` of the center, not `System::mu`, which is the `mu` of the orbit the center itself
    /// is on — `G(M_sun + M_earth)` for Earth. Using it put a low Earth orbit at eleven seconds.
    fn center_of_at(&self, system: &LocalSystem, seconds: f64) -> Option<(DVec3, f64)> {
        let index = self.center_index(system)?;
        let (at_m, _) = system.body_state_at(index, seconds)?;
        let at = system.origin_ly + at_m / M_PER_LY;
        Some((at, system.sim().gravitational_constant() * system.sim().mass(index)))
    }
}

/// Meters in an astronomical unit.
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
    /// `from_ly` matters for two courses: leaving goes straight out from the star, and a polar
    /// orbit is the one through the ship — see [`Course::resolve_moving`], which this is with
    /// the ship at rest.
    pub fn resolve(&self, system: &LocalSystem, from_ly: DVec3, now_s: f64) -> Option<Waypoint> {
        self.resolve_moving(system, from_ly, DVec3::ZERO, now_s)
    }

    /// [`Course::resolve`] for a ship moving at `beta`, which only a polar orbit reads: it runs
    /// the way the ship is already going round the body, so a ship on a polar orbit that asks
    /// for one again is already there.
    pub fn resolve_moving(
        &self,
        system: &LocalSystem,
        from_ly: DVec3,
        beta: DVec3,
        now_s: f64,
    ) -> Option<Waypoint> {
        match self {
            Course::To(at) => Some(Waypoint::Fixed(*at)),
            Course::Orbit { body, altitude_radii, plane } => {
                let index = system.body_named(body)?;
                let radius = system.sim().radius(index);
                if radius <= 0.0 {
                    return None;
                }
                let (at_m, velocity) = system.body_state_at(index, now_s)?;
                let relative = from_ly - (system.origin_ly + at_m / M_PER_LY);
                let moving = beta - crate::coast::beta_of(velocity);
                Some(Waypoint::Orbit(Orbit {
                    about: Anchor::Body(body.clone()),
                    radius_m: radius * (1.0 + altitude_radii.max(0.0)),
                    pole: plane.pole_of(system.body_pole(index), relative, moving),
                    phase_rad: 0.0,
                }))
            }
            Course::Lagrange { body, point } => {
                let index = system.body_named(body)?;
                // A body with no parent has no libration points: there is no second mass.
                system.sim().parent(index)?;
                Some(Waypoint::Lagrange { body: body.clone(), point: *point })
            }
            Course::Hangout { body, point } => Some(Waypoint::Libration(
                crate::libration::Libration::about(system, body, *point, now_s)?,
            )),
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
                let star = system.star_position_at(now_s)?;
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
            Course::Orbit { body, .. }
            | Course::Lagrange { body, .. }
            | Course::Hangout { body, .. }
            | Course::Rings(body) => Some(Target::Body(body.clone())),
            Course::Belt(index) => Some(Target::Band(*index)),
        }
    }

    /// Read a course from a development flag: `orbit:Earth`, `polar:Titan:high`,
    /// `rings:Saturn`, `l2:Earth`, `hang2:Earth`, `belt:0`, `leave`.
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
            "hang1" => Some(Course::Hangout { body: rest.into(), point: LagrangePoint::L1 }),
            "hang2" => Some(Course::Hangout { body: rest.into(), point: LagrangePoint::L2 }),
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
    /// The orbit normal for this plane about a body whose own pole is `pole`, for a ship at
    /// `relative` to the body and moving at `moving` against it, in any units.
    ///
    /// Any normal perpendicular to the pole makes an orbit polar, so the node is free, and a
    /// polar orbit is the one through the ship: it is entered where the ship already is, with
    /// one burn, rather than after a wait for a node or a crossing to one. Of its two
    /// directions, the one that keeps what the ship is doing round the body is the cheaper.
    pub fn pole_of(&self, pole: DVec3, relative: DVec3, moving: DVec3) -> DVec3 {
        let pole = pole.normalize_or(DVec3::Z);
        match self {
            Plane::Equatorial => pole,
            Plane::Polar => {
                // Straight over the pole every polar plane passes through the ship.
                let normal = pole.cross(relative).try_normalize().unwrap_or(basis(pole).0);
                if relative.cross(moving).dot(normal) < 0.0 { -normal } else { normal }
            }
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

/// What a belt or a cloud is called: what it is, and how far out it sits.
///
/// Derived from the population rather than stored, because a population has no name of its
/// own — and in one place rather than two, because the map and the inventory list the same
/// bands and a second copy of this format string is a second answer waiting to happen.
pub fn band_designation(population: &crate::population::Population) -> String {
    format!(
        "{} at {:.1} AU",
        if is_flat(population) { "belt" } else { "cloud" },
        population.thermal_radius() / AU
    )
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
    /// Meters from whatever it orbits, so a moon reads against its planet and a planet against
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
/// Its own name first, then whatever catalog designation it carries, and only then a made-up
/// one: the primary's name and a numeral, which is how an unnamed body has been designated
/// since Galileo. Players will be able to name planets, and that name goes in the first slot.
pub fn designate(
    name: Option<&str>,
    catalog: Option<&str>,
    primary: &str,
    rank: usize,
) -> String {
    match (name, catalog) {
        (Some(name), _) if !name.is_empty() => name.to_string(),
        (_, Some(catalog)) if !catalog.is_empty() => catalog.to_string(),
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
/// grayed. Whether a course can be flown is [`Course::resolve`]'s business and it is asked
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
        for (point, what) in
            [(LagrangePoint::L1, "L1 hangout"), (LagrangePoint::L2, "L2 hangout")]
        {
            out.push((what.to_string(), Course::Hangout { body: name.clone(), point }));
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
///
/// **And on its velocity, not only its place.** A station is an orbit and an orbit moves, so the
/// crossing is planned to end *on* the velocity the station will have — see
/// [`Cruise::plan_onto`]. The last burn is then one burn at one angle that kills the speed the
/// ship came in with and imparts the one it is joining, rather than a brake to a dead stop and a
/// few kilometers a second appearing out of nothing on the next step. That velocity is a third
/// thing the arrival time decides, so it iterates here with the other two.
///
/// **This plans in the world frame, and that is the limit of it.** A station about a moving body
/// runs away at the body's own speed, so the fixed point above only settles while the primary
/// covers less than the orbit's own radius during the transfer. Neptune covers a third of one
/// and a transfer between two of its orbits lands within half a kilometer; Jupiter covers half
/// and lands within a hundred and fifty; Earth covers the whole of one and does not converge at
/// all, and Luna is worse. Damping the iteration does not help, because the trouble is not the
/// step size — it is that [`Waypoint::nearest_to`] is asked which side of a circle is nearest to
/// a point the circle is fleeing, and the answer swings from one side to the other.
///
/// So a ship already inside the destination body's sphere of influence does not come here at
/// all: [`crate::transfer`] plans it in that body's frame, where nothing is running away, and
/// lands it within millimeters. This is what is left — everything flown between one place in a
/// system and another, where the world frame is the right one.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    system: &LocalSystem,
    waypoint: &Waypoint,
    from_ly: DVec3,
    beta0: DVec3,
    attitude0: DVec3,
    start_s: f64,
    drive: Drive,
) -> Option<(Cruise, Waypoint)> {
    let mut aimed = waypoint.clone();
    let mut target = aimed.place_at(system, start_s)?;
    // From whatever the ship is already doing. A course set from an orbit, or from a coast, or
    // in place of one already under way, keeps the speed it has — see `Cruise::plan_from`.
    let mut cruise = Cruise::plan_from(from_ly, beta0, target, attitude0, start_s, drive);
    if matches!(waypoint, Waypoint::Fixed(_)) {
        return Some((cruise, aimed));
    }
    // Asking the waypoint how fast it is going costs two more propagations of it, so it is asked
    // once the arrival time has settled rather than once a round.
    let joining = |aimed: &Waypoint, arrival_s: f64| {
        aimed.velocity_at(system, arrival_s).map(crate::coast::beta_of).unwrap_or(DVec3::ZERO)
    };
    for _ in 0..ARRIVAL_ROUNDS {
        let arrival_s = start_s + cruise.duration_s();
        aimed = waypoint.nearest_to(from_ly, system, arrival_s);
        let Some(next) = aimed.place_at(system, arrival_s) else { break };
        if next == target {
            break;
        }
        target = next;
        cruise = Cruise::plan_onto(
            from_ly,
            beta0,
            target,
            joining(&aimed, arrival_s),
            attitude0,
            start_s,
            drive,
        );
    }
    // One last round on the velocity alone: the arrival time has moved since the plan above was
    // made from it, and the injection is the part of the crossing most sensitive to it.
    let arrival_s = start_s + cruise.duration_s();
    let onto = joining(&aimed, arrival_s);
    if onto != DVec3::ZERO {
        cruise = Cruise::plan_onto(from_ly, beta0, target, onto, attitude0, start_s, drive);
    }
    Some((cruise, aimed))
}

/// What a population is, from its shape alone.
///
/// A belt is flat and a cloud is not, and nothing else about a population distinguishes them.
/// Used for naming one on screen; the Oort analog is the only isotropic one a system has.
pub fn is_flat(population: &Population) -> bool {
    population.inclination.max_inclination() < 1.0
}

#[cfg(test)]
mod tests {
    use crate::sky::{AuthoredStars, StarProvider};

    use super::*;

    /// Every course the interface can offer survives the wire and comes back the same.
    ///
    /// One case per variant, listed by hand. A loop over something generated would pass while
    /// the wire quietly dropped a field, because the thing being checked *is* whether every
    /// field made the trip.
    #[test]
    fn every_course_survives_the_wire() {
        let all = [
            Course::To(DVec3::new(1.5, -2.5, 0.25)),
            Course::Orbit {
                body: "Earth".into(),
                altitude_radii: 2.0,
                plane: Plane::Equatorial,
            },
            Course::Orbit { body: "Luna".into(), altitude_radii: 0.2, plane: Plane::Polar },
            Course::Lagrange { body: "Earth".into(), point: LagrangePoint::L1 },
            Course::Lagrange { body: "Earth".into(), point: LagrangePoint::L2 },
            Course::Hangout { body: "Earth".into(), point: LagrangePoint::L2 },
            Course::Rings("Saturn".into()),
            Course::Belt(3),
            Course::LeaveSystem,
        ];
        for course in all {
            let there: lc_proto::Course = (&course).into();
            let back: Course = there.clone().into();
            assert_eq!(back, course, "{course:?} did not survive as {there:?}");

            // And through postcard, which is what actually goes over the wire.
            let bytes = postcard::to_stdvec(&there).expect("it encodes");
            let decoded: lc_proto::Course = postcard::from_bytes(&bytes).expect("it decodes");
            assert_eq!(Course::from(decoded), course);
        }
    }

    /// The count is the check: a variant added to either side without the other is a compile
    /// error in the conversions, and this is what notices that the *test* was not extended.
    #[test]
    fn the_round_trip_covers_every_variant() {
        let seen = [
            Course::To(DVec3::ZERO),
            Course::Orbit { body: String::new(), altitude_radii: 0.0, plane: Plane::default() },
            Course::Lagrange { body: String::new(), point: LagrangePoint::L1 },
            Course::Hangout { body: String::new(), point: LagrangePoint::L1 },
            Course::Rings(String::new()),
            Course::Belt(0),
            Course::LeaveSystem,
        ];
        // If this fails, a variant was added: extend `every_course_survives_the_wire` too.
        assert_eq!(seen.len(), 7);
    }

    fn sol() -> LocalSystem {
        let provider =
            crate::sky::hyg::HygProvider::load("../../assets/catalogs/hygdata_v42_dist_sort.csv")
                .expect("the catalog");
        let sun = provider
            .stars()
            .iter()
            .find(|s| s.provenance.name.as_deref() == Some("Sol"))
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
        let waypoint = course.resolve(&system, DVec3::ZERO, 0.0).expect("Earth has an orbit");
        let at = waypoint.place_at(&system, 0.0).expect("a position");
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
            let Waypoint::Orbit(orbit) = course.resolve(&system, DVec3::ZERO, 0.0).unwrap() else { panic!() };
            orbit.pole.dot(pole).abs()
        };
        assert!(of(Plane::Equatorial) > 0.999, "an equatorial normal is the pole");
        assert!(of(Plane::Polar) < 1.0e-9, "a polar normal is perpendicular to it");
    }

    /// An orbit is circular, so every sample of it is the same distance out and they are not
    /// all the same place. The second half is what catches a rate of zero.
    #[test]
    fn an_orbit_goes_round() {
        let system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 0.2, plane: Plane::Equatorial };
        let waypoint = course.resolve(&system, DVec3::ZERO, 0.0).unwrap();
        let period = waypoint.period_s(&system, 0.0).expect("a period");
        // Low Earth orbit is about ninety minutes; at 1.2 radii it is a little longer.
        assert!((5000.0..9000.0).contains(&period), "{period} seconds");

        // Both the ship and Earth read at the same instant. Reading one at `t` and the other
        // out of the arena is how a circular orbit comes out fifty thousand kilometers wide.
        let earth = system.body_named("Earth").unwrap();
        let mut seen = Vec::new();
        for step in 0..4 {
            let t = period * step as f64 / 4.0;
            let at = waypoint.place_at(&system, t).unwrap();
            let center =
                system.origin_ly + system.body_state_at(earth, t).unwrap().0 / M_PER_LY;
            seen.push((at, at.distance(center) * M_PER_LY));
        }
        let radius = seen[0].1;
        for (_, r) in &seen {
            assert!((r / radius - 1.0).abs() < 1e-9, "{r} against {radius}");
        }
        let quarter = seen[0].0.distance(seen[1].0) * M_PER_LY;
        assert!(quarter > radius, "a quarter turn moves further than the radius is long");
    }

    /// L1 is sunward of the body and L2 is behind it, both at about the Hill radius. Earth's is
    /// 1.5 million kilometers, which is a number people know.
    #[test]
    fn the_libration_points_straddle_the_body() {
        let system = sol();
        let earth = system.body_position_ly("Earth").unwrap();
        let star = system.star_position_ly();
        let at = |point| {
            Waypoint::Lagrange { body: "Earth".into(), point }
                .place_at(&system, 0.0)
                .expect("a point")
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
        let waypoint = course.resolve(&system, DVec3::ZERO, 0.0).unwrap();
        let from = system.body_position_ly("Mars").expect("Mars");
        let (cruise, waypoint) =
            plan(&system, &waypoint, from, DVec3::ZERO, DVec3::ZERO, 0.0, Drive::DEFAULT).expect("a crossing");

        let wanted = waypoint.place_at(&system, cruise.duration_s()).unwrap();
        let landed = cruise.at(cruise.duration_s()).position_ly;
        let miss = landed.distance(wanted) * M_PER_LY;
        assert!(miss < 6.371e6, "missed by {miss:e} meters, more than a planetary radius");

        // And the naive aim is far worse, which is why the rounds are there.
        let naive = Cruise::plan(from, waypoint.place_at(&system, 0.0).unwrap(), 0.0, Drive::DEFAULT);
        let naive_miss = naive.at(naive.duration_s()).position_ly.distance(wanted) * M_PER_LY;
        assert!(naive_miss > miss * 100.0, "{naive_miss:e} against {miss:e}");
    }

    /// **One orbit of a body to another orbit of the same body**, which is the transfer the
    /// injection exists for: the ship leaves a station that is moving and joins one that is
    /// moving, and the crossing has to account for both ends rather than stopping dead between
    /// them.
    ///
    /// Neptune, because the planner works in the world frame — see [`plan`] — and Neptune runs
    /// only a third of one of these orbits during the transfer. Earth runs a whole one, and the
    /// same transfer about Earth lands tens of thousands of kilometers out.
    #[test]
    fn a_transfer_between_two_orbits_of_one_body_arrives_moving_with_the_second() {
        let system = sol();
        let station = |altitude_radii: f64, plane: Plane| {
            Course::Orbit { body: "Neptune".into(), altitude_radii, plane }
                .resolve(&system, DVec3::ZERO, 0.0)
                .expect("an orbit of Neptune")
        };
        let low = station(0.5, Plane::Equatorial);
        let high = station(4.0, Plane::Equatorial);

        let from = low.place_at(&system, 0.0).unwrap();
        let beta0 = crate::coast::beta_of(low.velocity_at(&system, 0.0).unwrap());
        let (cruise, aimed) =
            plan(&system, &high, from, beta0, DVec3::ZERO, 0.0, Drive::DEFAULT).expect("a transfer");

        // A local transfer, so the injection is the honest form rather than the fallback.
        assert!(
            cruise.peak_beta() < crate::flight::INJECTION_MAX_BETA,
            "premise: {} is not a local hop",
            cruise.peak_beta(),
        );
        let arrival_s = cruise.duration_s();
        let end = cruise.at(arrival_s);

        // It ends *on* the station, and *on* what the station is doing.
        let wanted = aimed.place_at(&system, arrival_s).unwrap();
        let miss = end.position_ly.distance(wanted) * M_PER_LY;
        assert!(miss < 2.4764e7, "missed by {miss:e} m, more than a planetary radius");

        let joining = crate::coast::beta_of(aimed.velocity_at(&system, arrival_s).unwrap());
        let short_m_s = (end.beta - joining).length() * crate::flight::C_M_S;
        assert!(short_m_s < 1.0, "arrived {short_m_s} m/s away from the station's velocity");

        // Which is a real amount of velocity, not a rounding: Neptune's own five kilometers a
        // second round the sun and the station's round Neptune. It used to appear out of nothing
        // on the step after arrival.
        assert!(
            joining.length() * crate::flight::C_M_S > 5.0e3,
            "premise: the station is going somewhere, at {joining:?}",
        );
        // And the last burn is one angle that brakes *and* injects. Most of its work is still
        // killing the speed the ship came in with, so it points back down the track — but it is
        // tilted toward what the ship is joining, which braking to a dead stop never is.
        let aim = cruise.last_aim();
        let heading = cruise
            .aim_at(cruise.start_s + arrival_s)
            .from
            .expect("the boost pointed it somewhere");
        assert!(aim.dot(heading) < 0.0, "it has to be braking: {aim} against {heading}");
        assert!(
            aim.dot(joining) > (-heading).dot(joining),
            "{aim} is no more aimed at {joining} than a plain brake would be",
        );
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
            let Waypoint::Fixed(out) = Course::LeaveSystem.resolve(&system, from, 0.0).unwrap() else {
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
        let waypoint = course.resolve(&system, DVec3::ZERO, 0.0).unwrap();
        let from = system.body_position_ly("Mars").expect("Mars");
        let (cruise, aimed) =
            plan(&system, &waypoint, from, DVec3::ZERO, DVec3::ZERO, 0.0, Drive::DEFAULT).expect("a plan");

        let arrival_s = cruise.duration_s();
        let landed = aimed.place_at(&system, arrival_s).unwrap();
        let reach = from.distance(landed);

        // The nearest point, checked against the rest of the circle rather than against a
        // formula: no other phase of the same orbit is closer to where the ship started.
        let Waypoint::Orbit(orbit) = &aimed else { panic!("an orbit") };
        let mut furthest: f64 = 0.0;
        for step in 1..36 {
            let mut elsewhere = orbit.clone();
            elsewhere.phase_rad += std::f64::consts::TAU * step as f64 / 36.0;
            let other = Waypoint::Orbit(elsewhere).place_at(&system, arrival_s).unwrap();
            let range = from.distance(other);
            assert!(range >= reach * (1.0 - 1e-9), "{range:e} beats the chosen {reach:e}");
            furthest = furthest.max(range);
        }
        // And the far side is a whole diameter further, which is what injecting blind costs.
        let earth = system.origin_ly
            + system.body_state_at(system.body_named("Earth").unwrap(), arrival_s).unwrap().0
                / M_PER_LY;
        let diameter = landed.distance(earth) * 2.0;
        assert!(furthest - reach > diameter * 0.9, "the circle is not being crossed at all");
    }

    /// Aiming decides a phase, and the ship still goes round afterwards: the injection point
    /// is where the orbit *starts*, not a place the ship stops.
    #[test]
    fn an_aimed_orbit_still_turns() {
        let system = sol();
        let course =
            Course::Orbit { body: "Earth".into(), altitude_radii: 2.0, plane: Plane::Equatorial };
        let waypoint = course
            .resolve(&system, DVec3::ZERO, 0.0)
            .unwrap()
            .nearest_to(system.body_position_ly("Mars").unwrap(), &system, 0.0);
        let period = waypoint.period_s(&system, 0.0).unwrap();
        let at_start = waypoint.place_at(&system, 0.0).unwrap();
        let half_way = waypoint.place_at(&system, period * 0.5).unwrap();
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
        let Waypoint::Orbit(orbit) = Course::Rings("Saturn".into()).resolve(&system, DVec3::ZERO, 0.0).unwrap()
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
        assert!(Course::Rings("Earth".into()).resolve(&system, DVec3::ZERO, 0.0).is_none());
        assert!(Course::Rings("Nowhere".into()).resolve(&system, DVec3::ZERO, 0.0).is_none());
        assert!(Course::Orbit { body: "Nowhere".into(), altitude_radii: 1.0, plane: Plane::Polar }
            .resolve(&system, DVec3::ZERO, 0.0)
            .is_none());
        // The primary is the one body with nothing to librate against.
        let star = system.sim().name(system.primary()).to_string();
        assert!(Course::Lagrange { body: star, point: LagrangePoint::L1 }.resolve(&system, DVec3::ZERO, 0.0).is_none());
    }

    /// A generated system has no measured data in it at all, and every course still has to
    /// resolve or refuse cleanly.
    #[test]
    fn a_generated_system_navigates_too() {
        let system = generated();
        assert!(Course::LeaveSystem.resolve(&system, DVec3::ZERO, 0.0).is_some());
        let belts = (0..system.populations.len())
            .filter_map(|i| Course::Belt(i).resolve(&system, DVec3::ZERO, 0.0))
            .count();
        assert_eq!(belts, system.populations.len(), "every population is somewhere to go");
        // How many populations a system has is what its ladder left behind, so the count
        // varies. What does not is that only a cloud is round.
        let round = system.populations.iter().filter(|p| !is_flat(p)).count();
        assert!(round <= 1, "{round} isotropic populations, and only a cloud may be one");
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
        assert!(Course::parse("belt:9").unwrap().resolve(&system, DVec3::ZERO, 0.0).is_none(), "and out of range");
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
    fn a_designation_prefers_the_name_then_the_catalog_then_a_numeral() {
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
        assert!(band[0].1.resolve(&system, DVec3::ZERO, 0.0).is_some(), "and it is a course that resolves");
    }

    /// Every option the interface offers has to resolve, or a Go button lies.
    #[test]
    fn every_offered_option_resolves_into_somewhere() {
        let system = sol();
        for entry in system.inventory().iter().filter(|e| e.major) {
            for (label, course) in options_for(&system, &entry.target) {
                assert!(
                    course.resolve(&system, DVec3::ZERO, 0.0).is_some(),
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

    /// **A station is named in the crew's words, never the generator's key.** A ship flown to
    /// `180.5-00.1` reported as holding at `99942-Apophis` has told the player the name of a
    /// body nobody aboard has identified.
    #[test]
    fn a_station_is_never_labeled_by_the_generators_key() {
        let stars = AuthoredStars::sample();
        let star = StarProvider::stars(&stars)[2].clone();
        let system = LocalSystem::for_star(&star).expect("a generated system");
        let key = system
            .inventory()
            .iter()
            .find_map(|e| match (&e.target, e.depth) {
                // Past the primary: `labels` maps the star itself separately.
                (Target::Body(key), depth) if depth > 0 => Some(key.clone()),
                _ => None,
            })
            .expect("a body");

        // What the crew calls it: one body detected, under its discovery designation.
        let labels = crate::labels::label(&system, "the star", |body| {
            (body == crate::knowledge::BodyId::of(star.id, &key)).then(|| "180.5-00.1".to_string())
        });

        let stations = [
            Waypoint::Lagrange { body: key.clone(), point: LagrangePoint::L1 },
            Waypoint::Orbit(Orbit {
                about: Anchor::Body(key.clone()),
                radius_m: 1.0e7,
                pole: DVec3::Z,
                phase_rad: 0.0,
            }),
        ];
        for station in stations {
            let said = station.label(&labels);
            assert!(!said.contains(&key), "the key leaked: {said}");
            assert!(said.contains("180.5-00.1"), "not the crew's name: {said}");
        }
    }

    /// A body nobody has detected has no name to give, and the key is still not it.
    #[test]
    fn an_undetected_body_is_unidentified_rather_than_keyed() {
        let stars = AuthoredStars::sample();
        let star = StarProvider::stars(&stars)[2].clone();
        let system = LocalSystem::for_star(&star).expect("a generated system");
        let key = system
            .inventory()
            .iter()
            .find_map(|e| match (&e.target, e.depth) {
                // Past the primary: `labels` maps the star itself separately.
                (Target::Body(key), depth) if depth > 0 => Some(key.clone()),
                _ => None,
            })
            .expect("a body");
        let labels = crate::labels::label(&system, "the star", |_| None);
        let said = Waypoint::Lagrange { body: key.clone(), point: LagrangePoint::L1 }.label(&labels);
        assert!(!said.contains(&key), "the key leaked: {said}");
    }
}

