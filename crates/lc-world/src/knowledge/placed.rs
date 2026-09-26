//! Where a body is believed to be now, and which way that is uncertain.
//!
//! An orbit makes two kinds of error and they point different ways. How far round the body has
//! got is a phase, which drifts with the period's error and is carried as an angle so it can be
//! drawn along the orbit. The size and the plane are not a phase; they are carried as a
//! covariance across the path. See `lightcone/docs/25-system-knowledge.md`.

use std::f64::consts::{PI, TAU};

use glam::{DMat3, DVec3};

use super::record::{Orbit, Orientation};

/// Where a body is believed to be now, as an offset from its own **star**.
///
/// From the star rather than from the world origin: what an orbit says is where a body sits
/// about its primary, and the star's own position is a separate belief with its own error. A
/// reader that wants an absolute position adds the two and carries both errors.
///
/// From the star even for a moon, whose orbit is about its planet: `body_belief` walks the
/// chain and adds each step, so one reader does not have to know how deep a body sits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placed {
    /// A full orientation and an epoch: enough to say where on the orbit it is.
    Known { offset_au: DVec3, error: PlaceError },
    /// An orbit of known size and nothing else. A sphere of that radius, not a ring in a guessed
    /// plane — rule 4 of doc 25.
    Shell { radius_au: f64, sigma_au: f64 },
    Unknown,
}

/// How a known place is uncertain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlaceError {
    /// Covariance of everything but how far round it has got, AU², simulation axes. A
    /// primary's error is in here too, less its part along this body's path.
    pub across_au2: DMat3,
    /// One sigma of mean anomaly, radians. At π it could be anywhere on its orbit.
    pub along_rad: f64,
    /// `dr/dM`: how far and which way the body moves per radian of mean anomaly, AU.
    pub pace_au: DVec3,
    /// Away from what it goes round, unit.
    pub outward: DVec3,
    /// The orbit's pole, unit.
    pub pole: DVec3,
}

impl PlaceError {
    /// One sigma across the path in `direction`, AU.
    pub fn sigma_au(&self, direction: DVec3) -> f64 {
        let d = direction.normalize_or_zero();
        d.dot(self.across_au2 * d).max(0.0).sqrt()
    }

    /// One sigma in `direction` from every source, the phase linearized: a range's error.
    pub fn toward_au(&self, direction: DVec3) -> f64 {
        let d = direction.normalize_or_zero();
        self.sigma_au(d).hypot(self.pace_au.dot(d) * self.along_rad.min(PI))
    }

    pub fn outward_au(&self) -> f64 {
        self.sigma_au(self.outward)
    }

    pub fn normal_au(&self) -> f64 {
        self.sigma_au(self.pole)
    }

    /// Along the path, linearized: good for a short arc, and a ring's reach past half a turn.
    pub fn along_au(&self) -> f64 {
        self.pace_au.length() * self.along_rad.min(PI)
    }

    pub fn anywhere_on_orbit(&self) -> bool {
        self.along_rad >= PI
    }

    /// Every direction in quadrature, for a reader that wants one number.
    pub fn total_au(&self) -> f64 {
        let c = self.across_au2;
        let along = self.along_au();
        (c.x_axis.x + c.y_axis.y + c.z_axis.z + along * along).max(0.0).sqrt()
    }
}

/// Where a body might be along its orbit: a polyline, AU from its star.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub points_au: Vec<DVec3>,
    /// The whole orbit, so it has no ends.
    pub closed: bool,
}

/// What Kepler's equation is solved to, radians, and how many steps it may take.
///
/// Far tighter than a drawn position needs; Newton on an ellipse converges in a handful of
/// steps, so the cost of asking for more is nothing and the cap is only a guard.
pub(super) const KEPLER_TOLERANCE: f64 = 1.0e-12;
pub(super) const KEPLER_STEPS: u32 = 32;

/// Points in a whole orbit's [`Track`]; a shorter arc gets its share.
const TRACK_SAMPLES: f64 = 64.0;

/// An orbit with enough in it to say where on its path the body is: a full orientation and
/// an epoch.
///
/// `epoch_s` is the periapsis passage. For a circle that is any point, which is why an
/// eccentricity of `None` is read as zero here rather than as an obstacle: a circular orbit has
/// no periapsis to be wrong about.
pub(super) struct Path {
    au: f64,
    e: f64,
    pole: DVec3,
    node: f64,
    periapsis: f64,
    epoch_s: f64,
    period_s: f64,
}

impl Path {
    pub(super) fn of(orbit: &Orbit) -> Option<Self> {
        let (Orientation::Known { pole, node, periapsis, .. }, Some(epoch_s)) =
            (orbit.orientation, orbit.epoch_s)
        else {
            return None;
        };
        let (au, period_s) = (orbit.semi_major_au.0, orbit.period_s.0);
        (period_s > 0.0 && au.is_finite() && au > 0.0).then_some(Self {
            au,
            e: orbit.eccentricity.map_or(0.0, |(e, _)| e).clamp(0.0, 0.999),
            pole,
            node,
            periapsis,
            epoch_s,
            period_s,
        })
    }

    pub(super) fn mean_at(&self, now_s: f64) -> f64 {
        TAU * (now_s - self.epoch_s) / self.period_s
    }

    /// Radians a second.
    pub(super) fn mean_motion(&self) -> f64 {
        TAU / self.period_s
    }

    /// AU from the primary at mean anomaly `mean`, `dr/dM` in AU, and the eccentric anomaly.
    pub(super) fn at(&self, mean: f64) -> Option<(DVec3, DVec3, f64)> {
        let eccentric = em_foundations::kepler::anomaly::eccentric_from_mean_newton(
            mean, self.e, KEPLER_TOLERANCE, KEPLER_STEPS,
        );
        let elements = em_foundations::kepler::state::Elements {
            semi_major_axis: self.au,
            eccentricity: self.e,
            // The pole as an inclination and a node, the same way `sky::generate` writes one.
            inclination: self.pole.z.clamp(-1.0, 1.0).acos(),
            longitude_of_ascending_node: self.node,
            argument_of_periapsis: self.periapsis,
            true_anomaly: em_foundations::kepler::anomaly::true_from_eccentric(eccentric, self.e),
        };
        // `mu = a^3` makes the mean motion one radian per unit time, so the velocity half is
        // `dr/dM` directly.
        let (offset, pace) = em_foundations::kepler::state::to_state(self.au.powi(3), &elements)?;
        Some((offset, pace, eccentric))
    }

    /// The arc `mean ± half`, a whole orbit once `half` reaches π, shifted by `base_au`.
    fn track(&self, mean: f64, half: f64, base_au: DVec3) -> Option<Track> {
        let half = half.min(PI);
        if !(half > 0.0) {
            return None;
        }
        let steps = (TRACK_SAMPLES * half / PI).ceil().max(2.0) as usize;
        let points_au = (0..=steps)
            .map(|i| mean - half + 2.0 * half * i as f64 / steps as f64)
            .map(|m| self.at(m).map(|(offset, _, _)| base_au + offset))
            .collect::<Option<Vec<_>>>()?;
        Some(Track { points_au, closed: half >= PI })
    }
}

/// Where an orbit puts its body at `now_s`, about its own primary.
///
/// Only a shell until the orientation is full and an epoch says where on the ring the body was:
/// an orbit of known size and unknown orientation is a sphere of that radius, and drawing it as
/// a ring in a guessed plane would be a claim nobody measured.
pub(super) fn placed_at(orbit: &Orbit, now_s: f64) -> Placed {
    let (au, sigma_au) = orbit.semi_major_au;
    let shell = Placed::Shell { radius_au: au, sigma_au };
    let (Some(path), Orientation::Known { sigma_rad, .. }) = (Path::of(orbit), orbit.orientation) else {
        return shell;
    };
    let Some((offset_au, pace_au, eccentric)) = path.at(path.mean_at(now_s)) else {
        return shell;
    };
    let r = offset_au.length();
    let outward = offset_au.normalize_or(DVec3::X);

    // `r = a (1 - e cos E)`: the axis's error scales the whole orbit, and the eccentricity's
    // moves the body in and out by `a cos E`.
    let sigma_e = orbit.eccentricity.map_or(0.0, |(_, s)| s);
    let outward_au = (r / au * sigma_au).hypot(au * eccentric.cos() * sigma_e);
    // A tilted pole lifts the body out of its plane, by `r` per radian at most.
    let normal_au = r * sigma_rad;

    // **Where a body has got to is a phase, and a phase drifts.** A period known to a part in
    // a hundred is a body a quarter of the way round its orbit after twenty-five turns, and a
    // belief that reported only the size and the plane said a course could be flown against it.
    // `M = tau (t - epoch) / P`, so the period's error carries `tau |t - epoch| sigma_P / P^2`
    // of anomaly with it. Doc 25: the sigma is grown by how long since it was last seen.
    let (period_s, period_sigma) = orbit.period_s;
    let drift = TAU * (now_s - path.epoch_s).abs() * period_sigma / (period_s * period_s);

    Placed::Known {
        offset_au,
        error: PlaceError {
            across_au2: outer(outward * outward_au) + outer(path.pole * normal_au),
            along_rad: drift.min(PI),
            pace_au,
            outward,
            pole: path.pole,
        },
    }
}

/// One place on top of another, carrying both errors.
pub(super) fn added(primary: Placed, own: Placed) -> Placed {
    match (primary, own) {
        (Placed::Known { offset_au: up, error: a }, Placed::Known { offset_au: here, error: b }) => {
            // The primary's whole error moves this orbit rigidly. What of it lies along this
            // body's own path is a phase error, and is carried as one.
            let mut carried = a.across_au2 + outer(a.pace_au * a.along_rad.min(PI));
            let mut along_rad = b.along_rad;
            let pace = b.pace_au.length();
            if pace > 0.0 {
                let t = b.pace_au / pace;
                along_rad = (along_rad.powi(2) + t.dot(carried * t).max(0.0) / (pace * pace)).sqrt().min(PI);
                let across = DMat3::IDENTITY - outer(t);
                carried = across * carried * across;
            }
            Placed::Known {
                offset_au: up + here,
                error: PlaceError { across_au2: b.across_au2 + carried, along_rad, ..b },
            }
        }
        // A shell about a primary whose own place is known is still a shell, just a wider one:
        // the body is somewhere on a sphere about a point that is itself uncertain.
        (Placed::Known { error: a, .. }, Placed::Shell { radius_au, sigma_au: b }) => {
            Placed::Shell { radius_au, sigma_au: a.total_au().hypot(b) }
        }
        _ => Placed::Unknown,
    }
}

/// Where the body might be along its orbit at `now_s`, given where it was placed.
///
/// `placed` is the whole chain's answer; its offset less this orbit's own is where the primary
/// stands, so the track is not walked twice.
pub(super) fn track(orbit: &Orbit, placed: Placed, now_s: f64) -> Option<Track> {
    let Placed::Known { offset_au, error } = placed else { return None };
    let path = Path::of(orbit)?;
    let mean = path.mean_at(now_s);
    let (own, _, _) = path.at(mean)?;
    path.track(mean, error.along_rad, offset_au - own)
}

fn outer(v: DVec3) -> DMat3 {
    DMat3::from_cols(v * v.x, v * v.y, v * v.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{Lineage, Method, Witness};

    const PERIOD_S: f64 = 3.156e7;

    fn orbit(e: Option<(f64, f64)>, sigma_pole: f64) -> Orbit {
        Orbit {
            witness: Witness(1),
            about: None,
            period_s: (PERIOD_S, PERIOD_S * 0.01),
            semi_major_au: (1.0, 0.001),
            eccentricity: e,
            orientation: Orientation::Known { pole: DVec3::Z, sigma_rad: sigma_pole, node: 0.0, periapsis: 0.0 },
            epoch_s: Some(0.0),
            method: Method::Astrometric,
            stated_s: 0.0,
            lineage: Lineage::new(),
        }
    }

    fn error(orbit: &Orbit, t: f64) -> (DVec3, PlaceError) {
        match placed_at(orbit, t) {
            Placed::Known { offset_au, error } => (offset_au, error),
            other => panic!("{other:?}"),
        }
    }

    /// At the epoch nothing has drifted: the axis is outward, the pole is out of the plane, and
    /// neither leaks into the other or along the path.
    #[test]
    fn each_error_points_its_own_way() {
        let (_, fresh) = error(&orbit(None, 1.0e-3), 0.0);
        assert_eq!(fresh.along_rad, 0.0);
        assert!((fresh.outward_au() - 0.001).abs() < 1.0e-12, "{}", fresh.outward_au());
        assert!((fresh.normal_au() - 1.0e-3).abs() < 1.0e-12, "{}", fresh.normal_au());
        let along = fresh.pace_au.normalize();
        assert!(fresh.sigma_au(along) < 1.0e-12, "nothing across the path lies along it");
    }

    /// The period's error is an angle along the orbit, and it grows with time either way.
    #[test]
    fn the_period_drifts_the_phase_and_nothing_else() {
        let o = orbit(None, 1.0e-4);
        let (_, later) = error(&o, 10.0 * PERIOD_S);
        let want = TAU * 10.0 * 0.01 / 1.0;
        assert!((later.along_rad - want).abs() < 1.0e-9, "{} against {want}", later.along_rad);
        assert_eq!(error(&o, -10.0 * PERIOD_S).1.along_rad, later.along_rad);
        assert!((later.outward_au() - error(&o, 0.0).1.outward_au()).abs() < 1.0e-12);
        assert!(error(&o, 1.0e4 * PERIOD_S).1.anywhere_on_orbit());
    }

    /// `r = a (1 - e cos E)`: at apoapsis the axis's error is stretched by `r / a`, and the
    /// eccentricity's pushes the same way.
    #[test]
    fn an_eccentric_orbit_scales_the_outward_error_with_its_radius() {
        let (offset, apo) = error(&orbit(Some((0.5, 0.0)), 0.0), 0.5 * PERIOD_S);
        assert!((offset.length() - 1.5).abs() < 1.0e-9, "{offset}");
        assert!((apo.outward_au() - 0.0015).abs() < 1.0e-12, "{}", apo.outward_au());

        let (_, with_e) = error(&orbit(Some((0.5, 0.01)), 0.0), 0.5 * PERIOD_S);
        assert!((with_e.outward_au() - 0.0015f64.hypot(0.01)).abs() < 1.0e-12);
    }

    /// The arc is the orbit itself, not a chord: every point is the orbit's own radius and the
    /// ends are the phase's sigma either side.
    #[test]
    fn a_track_follows_the_orbit() {
        let o = orbit(Some((0.3, 0.0)), 0.0);
        let now = 3.0 * PERIOD_S;
        let placed = placed_at(&o, now);
        let Placed::Known { error: e, .. } = placed else { panic!() };
        let track = track(&o, placed, now).expect("it has drifted");
        assert!(!track.closed);
        let path = Path::of(&o).unwrap();
        let mean = path.mean_at(now);
        let (first, last) = (track.points_au[0], *track.points_au.last().unwrap());
        assert!(first.distance(path.at(mean - e.along_rad).unwrap().0) < 1.0e-12);
        assert!(last.distance(path.at(mean + e.along_rad).unwrap().0) < 1.0e-12);
        for p in &track.points_au {
            assert!(p.z.abs() < 1.0e-12 && (0.7 - 1.0e-9..=1.3 + 1.0e-9).contains(&p.length()), "{p}");
        }

        let ancient = 1.0e4 * PERIOD_S;
        let whole = track_at(&o, ancient);
        assert!(whole.closed);
        assert!(whole.points_au[0].distance(*whole.points_au.last().unwrap()) < 1.0e-9);
        assert!(track_at(&o, 0.0).points_au.is_empty(), "no drift at the epoch, no arc");
    }

    fn track_at(o: &Orbit, t: f64) -> Track {
        track(o, placed_at(o, t), t).unwrap_or(Track { points_au: Vec::new(), closed: false })
    }

    /// A primary's error moves its moon's orbit whole. Along the moon's path it is a phase;
    /// across it, it widens the cross.
    #[test]
    fn a_moon_carries_its_planets_error() {
        let planet = placed_at(&Orbit { semi_major_au: (1.0, 0.01), ..orbit(None, 0.0) }, 0.0);
        let moon_orbit = Orbit { semi_major_au: (0.01, 0.0), ..orbit(None, 0.0) };
        let moon = placed_at(&moon_orbit, 0.0);
        let Placed::Known { error: alone, .. } = moon else { panic!() };
        let Placed::Known { error: carried, .. } = added(planet, moon) else { panic!() };

        // Both start on +X going +Y, so the planet's outward error is the moon's outward error.
        assert!((carried.outward_au() - 0.01).abs() < 1.0e-12, "{}", carried.outward_au());
        assert_eq!(alone.along_rad, 0.0);

        // A quarter period on, both are at +Y going -X, and the planet's phase error lies
        // along the moon's path: a hundredth of an AU of it is most of a radian of the moon's.
        let now = 0.25 * PERIOD_S;
        let planet = placed_at(&Orbit { semi_major_au: (1.0, 0.0), ..orbit(None, 0.0) }, now);
        let moon = placed_at(&moon_orbit, now);
        let (Placed::Known { error: p, .. }, Placed::Known { error: alone, .. }) = (planet, moon) else { panic!() };
        let Placed::Known { error: m, .. } = added(planet, moon) else { panic!() };
        let want = alone.along_rad.hypot(p.along_au() / alone.pace_au.length());
        assert!((m.along_rad - want).abs() < 1.0e-9, "{} against {want}", m.along_rad);
        assert!(m.sigma_au(m.pace_au) < 1.0e-9, "and it is not also counted across the path");
    }
}
