//! The collinear libration points, and how a craft moves near one.
//!
//! L1 and L2 are equilibria of the circular restricted three-body problem, and they are
//! unstable ones — a craft placed exactly there falls off in weeks. What real spacecraft do
//! instead is *orbit the point*: SOHO round Sun-Earth L1, JWST round L2. Those orbits are not
//! conics and have no closed form, but the linearised motion about a collinear point does, and
//! it is what the real trajectories are built on.
//!
//! Everything here is dimensionless. The unit of length is the separation of the two primaries
//! and the unit of time is `1/n`, where `n` is their mean motion, which is what makes the
//! results depend on the mass ratio alone. [`Frequencies`] are multiples of `n`.
//!
//! Richardson (1980), *Analytic construction of periodic orbits about the collinear points*,
//! is the source for the `c_n` expansion and the amplitude ratio.

/// Which collinear point. L3, on the far side of the larger primary, is not modeled: nothing
/// is ever sent there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collinear {
    /// Between the primaries, on the sunward side of the smaller one.
    L1,
    /// Beyond the smaller primary, away from the larger.
    L2,
}

/// How the linearised motion about a collinear point goes, in multiples of the mean motion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frequencies {
    /// In-plane oscillation rate. About `2.09` for Sun-Earth.
    pub planar: f64,
    /// Out-of-plane oscillation rate. About `2.01` for Sun-Earth, and the small difference
    /// from `planar` is why a Lissajous never closes and a halo has to be forced to.
    pub vertical: f64,
    /// Ratio of the along-track amplitude to the radial one. About `3.23` for Sun-Earth: a
    /// libration orbit is three times longer than it is wide.
    pub amplitude_ratio: f64,
}

/// Newton steps for [`gamma`]. The quintic is well behaved from the Hill-radius start; three
/// is already at machine precision for every ratio in a planetary system.
const GAMMA_ITERATIONS: usize = 16;

/// Distance from the *smaller* primary to the point, as a fraction of the separation.
///
/// `mass_ratio` is `m2 / (m1 + m2)`, the smaller primary's share. The root of Lagrange's
/// quintic, not the Hill radius — those differ by about a per cent at Sun-Earth, which is
/// two hundred thousand kilometers.
///
/// `None` for a ratio outside `(0, 1)`, or one the iteration leaves without a positive root.
pub fn gamma(mass_ratio: f64, point: Collinear) -> Option<f64> {
    if !(mass_ratio > 0.0 && mass_ratio < 1.0) {
        return None;
    }
    let mu = mass_ratio;
    // L1: g^5 - (3-mu) g^4 + (3-2mu) g^3 - mu g^2 + 2 mu g - mu = 0
    // L2: g^5 + (3-mu) g^4 + (3-2mu) g^3 - mu g^2 - 2 mu g - mu = 0
    //
    // Two terms flip and they flip *oppositely*: the quartic and the linear. Tying both to one
    // sign put L1 at 0.010105 and L2 at 0.009905 — the right size, the wrong numbers, and in
    // the wrong order, since L1 is the nearer point.
    let quartic = match point {
        Collinear::L1 => -1.0,
        Collinear::L2 => 1.0,
    };
    let linear = -quartic;
    let f = |g: f64| {
        g * g * g * g * g + quartic * (3.0 - mu) * g * g * g * g + (3.0 - 2.0 * mu) * g * g * g
            - mu * g * g
            + linear * 2.0 * mu * g
            - mu
    };
    let df = |g: f64| {
        5.0 * g * g * g * g
            + quartic * 4.0 * (3.0 - mu) * g * g * g
            + 3.0 * (3.0 - 2.0 * mu) * g * g
            - 2.0 * mu * g
            + linear * 2.0 * mu
    };

    // The Hill radius, which is the first term of the same expansion.
    let mut g = (mu / 3.0).cbrt();
    for _ in 0..GAMMA_ITERATIONS {
        let slope = df(g);
        if slope == 0.0 || !slope.is_finite() {
            return None;
        }
        let next = g - f(g) / slope;
        if !next.is_finite() || next <= 0.0 || next >= 1.0 {
            return None;
        }
        g = next;
    }
    Some(g)
}

/// Richardson's `c_2`: the quadratic coefficient of the potential expanded about the point.
///
/// Every frequency below is a function of this one number, which is why it is worth naming.
/// It is about `4.06` at Sun-Earth L1 and `3.94` at L2.
pub fn c2(mass_ratio: f64, gamma: f64, point: Collinear) -> f64 {
    let mu = mass_ratio;
    let far = match point {
        Collinear::L1 => 1.0 - gamma,
        Collinear::L2 => 1.0 + gamma,
    };
    (mu + (1.0 - mu) * gamma.powi(3) / far.powi(3)) / gamma.powi(3)
}

/// The linearised motion about a collinear point, from its `c_2`.
///
/// The planar rate is the oscillatory root of `λ⁴ + (c2 - 2)λ² - (c2 - 1)(1 + 2 c2) = 0`; the
/// other root pair is real, and *that* is the instability these orbits have to be kept against.
/// The vertical rate is simply `sqrt(c2)`, the two directions being uncoupled at this order.
///
/// `None` when `c2` is below the value at which the collinear points exist at all.
pub fn frequencies(c2: f64) -> Option<Frequencies> {
    let discriminant = 9.0 * c2 * c2 - 8.0 * c2;
    // `is_nan` first, spelled out: a NaN compares false against everything, so the ordinary
    // bound alone would let one through into the rates and on into a trajectory.
    if c2.is_nan() || c2 <= 1.0 || discriminant < 0.0 {
        return None;
    }
    let planar = ((2.0 - c2 + discriminant.sqrt()) / 2.0).sqrt();
    if !planar.is_finite() || planar <= 0.0 {
        return None;
    }
    Some(Frequencies {
        planar,
        vertical: c2.sqrt(),
        amplitude_ratio: (planar * planar + 1.0 + 2.0 * c2) / (2.0 * planar),
    })
}

/// [`gamma`] and [`frequencies`] in one, for a mass ratio.
pub fn about(mass_ratio: f64, point: Collinear) -> Option<(f64, Frequencies)> {
    let g = gamma(mass_ratio, point)?;
    Some((g, frequencies(c2(mass_ratio, g, point))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Earth and Moon against the Sun, since the pair is what orbits it.
    const SUN_EARTH: f64 = 3.0034e-6;
    const EARTH_MOON: f64 = 0.012_150_6;

    /// Published values. `gamma` at Sun-Earth L1 is 0.01001, which is 1.491 million km of a
    /// 149.6 million km separation — the distance SOHO sits at.
    #[test]
    fn sun_earth_reproduces_the_published_geometry() {
        let (g1, _) = about(SUN_EARTH, Collinear::L1).expect("L1 exists");
        let (g2, _) = about(SUN_EARTH, Collinear::L2).expect("L2 exists");
        assert!((g1 - 0.009_970_3).abs() < 1.0e-6, "L1 at {g1}");
        assert!((g2 - 0.010_037_0).abs() < 1.0e-6, "L2 at {g2}");

        // In kilometers of the real separation: about a hundredth of an astronomical unit.
        let au_km = 149_597_871.0;
        assert!((g1 * au_km - 1_491_500.0).abs() < 2_000.0, "{} km", g1 * au_km);
        assert!((g2 * au_km - 1_501_500.0).abs() < 2_000.0, "{} km", g2 * au_km);

        // L2 is further out than L1 is in, which is the asymmetry the two quintics carry.
        assert!(g2 > g1);
    }

    /// The frequencies JWST and SOHO actually fly at: a libration period of about 178 days
    /// against Earth's 365, so a little under six months.
    #[test]
    fn sun_earth_frequencies_match_the_missions() {
        let (_, f) = about(SUN_EARTH, Collinear::L2).expect("L2 exists");
        assert!((f.planar - 2.057).abs() < 0.01, "planar {}", f.planar);
        assert!((f.vertical - 1.985).abs() < 0.01, "vertical {}", f.vertical);
        assert!((f.amplitude_ratio - 3.187).abs() < 0.02, "kappa {}", f.amplitude_ratio);

        // In days, against a 365.25-day year.
        let period_days = 365.25 / f.planar;
        assert!((period_days - 177.6).abs() < 2.0, "{period_days} days");

        // Never equal, which is the whole reason a Lissajous does not close on itself.
        assert!(f.planar != f.vertical);
        assert!(f.planar > f.vertical, "the in-plane motion is the faster one");
    }

    /// A heavier secondary pushes the points out and slows the libration. Earth-Moon is four
    /// thousand times the mass ratio of Sun-Earth and its points are a sixth of the way out.
    #[test]
    fn a_heavier_secondary_pushes_the_points_further_out() {
        let (light, fl) = about(SUN_EARTH, Collinear::L1).unwrap();
        let (heavy, fh) = about(EARTH_MOON, Collinear::L1).unwrap();
        assert!(heavy > light * 10.0, "{heavy} against {light}");
        assert!((heavy - 0.1509).abs() < 1.0e-3, "Earth-Moon L1 at {heavy}");
        // And it librates faster per revolution of the pair, not slower: 2.33 against 2.09.
        assert!(fh.planar > fl.planar, "{} against {}", fh.planar, fl.planar);
    }

    /// The quintic is solved, not approximated.
    ///
    /// The Hill radius is the first term of the same expansion and is where the iteration
    /// starts. At Sun-Earth it is out by a third of a per cent — five thousand kilometers, or
    /// three times the libration amplitude a mission actually flies. At Earth-Moon, where the
    /// ratio is four thousand times larger, it is out by five and a half.
    #[test]
    fn the_root_is_not_the_hill_radius() {
        let out_by = |ratio: f64| {
            let hill = (ratio / 3.0).cbrt();
            let exact = gamma(ratio, Collinear::L1).unwrap();
            (hill - exact).abs() / exact
        };
        let near = out_by(SUN_EARTH);
        assert!((near - 0.0034).abs() < 0.0005, "Sun-Earth agree to {near}");
        assert!(near * 1_491_500.0 > 4_000.0, "only {} km", near * 1_491_500.0);

        let far = out_by(EARTH_MOON);
        assert!((far - 0.056).abs() < 0.005, "Earth-Moon agree to {far}");
        assert!(far > near * 10.0, "a heavier secondary is where the approximation gives up");
    }

    /// Nonsense in, `None` out, rather than a NaN that propagates into a trajectory.
    #[test]
    fn an_impossible_ratio_has_no_points() {
        assert!(gamma(0.0, Collinear::L1).is_none());
        assert!(gamma(1.0, Collinear::L2).is_none());
        assert!(gamma(-1.0, Collinear::L1).is_none());
        assert!(gamma(f64::NAN, Collinear::L1).is_none());
        assert!(frequencies(0.5).is_none(), "below the collinear points' own existence");
    }
}
