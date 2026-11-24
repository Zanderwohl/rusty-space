//! A star's state, luminosity and temperature from its initial mass and age.
//!
//! **A placeholder**: textbook power laws for the main sequence, one giant branch and fixed
//! remnants, enough for a star list whose HR diagram has the right shape. The analytic tracks
//! of `lightcone/docs/34-galaxy-authoring.md` (Hurley, Pols & Tout) replace them. Shared by
//! the bake and the loader, so a published star is as bright in the game as in the editor.

/// Solar effective temperature, K.
const T_SUN: f64 = 5772.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    MainSequence,
    Giant,
    WhiteDwarf,
    NeutronStar,
    BlackHole,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Evolved {
    pub state: State,
    pub mass_solar: f64,
    pub luminosity_solar: f64,
    /// Zero for a remnant that emits nothing.
    pub teff_k: f64,
    pub radius_solar: f64,
}

/// Main-sequence lifetime, Gyr.
pub fn lifetime_gyr(mass: f64) -> f64 {
    10.0 * libm::pow(mass, -2.5)
}

fn main_sequence_luminosity(m: f64) -> f64 {
    if m < 0.43 {
        0.23 * libm::pow(m, 2.3)
    } else if m < 2.0 {
        libm::pow(m, 4.0)
    } else if m < 55.0 {
        1.4 * libm::pow(m, 3.5)
    } else {
        32000.0 * m
    }
}

fn main_sequence_radius(m: f64) -> f64 {
    if m < 1.0 { libm::pow(m, 0.8) } else { libm::pow(m, 0.57) }
}

pub fn evolve(initial_mass: f64, age_gyr: f64) -> Evolved {
    let m = initial_mass;
    let t_ms = lifetime_gyr(m);
    let (state, mass, l, teff) = if age_gyr < t_ms {
        let l = main_sequence_luminosity(m);
        let r = main_sequence_radius(m);
        (State::MainSequence, m, l, T_SUN * libm::pow(l / (r * r), 0.25))
    } else if age_gyr < 1.1 * t_ms {
        if m < 8.0 { (State::Giant, m, 100.0 * m * m, 4500.0) } else { (State::Giant, m, main_sequence_luminosity(m), 4000.0) }
    } else if m < 8.0 {
        let cooling = (age_gyr - 1.1 * t_ms).max(1e-3);
        let l = (0.01 * libm::pow(cooling / 0.1, -1.4)).min(0.1);
        let r = 0.0126;
        (State::WhiteDwarf, 0.109 * m + 0.394, l, T_SUN * libm::pow(l / (r * r), 0.25))
    } else if m < 25.0 {
        (State::NeutronStar, 1.4, 0.0, 0.0)
    } else {
        (State::BlackHole, 0.3 * m, 0.0, 0.0)
    };
    let radius_solar = match state {
        State::NeutronStar => 1.4e-5,
        State::BlackHole => 0.0,
        // `L = R² (T/T☉)⁴`
        _ => libm::sqrt(l) * (T_SUN / teff) * (T_SUN / teff),
    };
    Evolved { state, mass_solar: mass, luminosity_solar: l, teff_k: teff, radius_solar }
}
