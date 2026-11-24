//! A star's state, luminosity and radius from its initial mass, age and metallicity.
//!
//! The zero-age main sequence is Tout et al. (1996, MNRAS 281, 257); everything after it is Hurley,
//! Pols & Tout (2000, MNRAS 315, 543, astro-ph/0001295), equations and Appendix coefficients as
//! published. Every coefficient was checked against the SSE code's data; where the paper's HTML
//! and the code disagreed (eq. 21's divisor, b17's exponent, the blue-loop exponent), the code is
//! taken. Valid for 0.1–100 M☉ and Z = 1e-4 to 0.03; outside that, values are clamped to the edge.
//!
//! No winds: the AGB ends at an initial–final mass relation instead, see [`giants`].
//!
//! Shared by the bake and the loader, so a published star is as bright in the game as in the
//! editor.

mod coefficients;
mod giants;
mod main_sequence;

pub use coefficients::Coefficients;
pub use giants::End;

use coefficients::{Z_REF, pow};

/// Solar effective temperature, K.
const T_SUN: f64 = 5772.0;
const M_RANGE: (f64, f64) = (0.1, 100.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    MainSequence,
    HertzsprungGap,
    GiantBranch,
    CoreHeliumBurning,
    AsymptoticGiantBranch,
    WhiteDwarf,
    NeutronStar,
    BlackHole,
}

impl State {
    pub fn is_remnant(self) -> bool {
        matches!(self, State::WhiteDwarf | State::NeutronStar | State::BlackHole)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Evolved {
    pub state: State,
    pub mass_solar: f64,
    pub luminosity_solar: f64,
    pub teff_k: f64,
    pub radius_solar: f64,
}

/// `[Fe/H]` to Hurley's Z, scaled from the solar Z = 0.02 the fits use.
pub fn metallicity_z(feh: f64) -> f64 {
    Z_REF * pow(10.0, feh)
}

pub fn evolve(initial_mass: f64, age_gyr: f64, feh: f64) -> Evolved {
    let m = initial_mass.clamp(M_RANGE.0, M_RANGE.1);
    let c = Coefficients::new(metallicity_z(feh));
    let t = age_gyr.max(0.0) * 1000.0;
    if t < c.t_ms(m) {
        let (l, r) = c.main_sequence(m, t);
        return Evolved { state: State::MainSequence, mass_solar: initial_mass, luminosity_solar: l, teff_k: teff(l, r), radius_solar: r };
    }
    let life = giants::Life::new(&c, m);
    if t < life.t_end {
        return Evolved { mass_solar: initial_mass, ..life.at(t) };
    }
    remnant(life.end, t - life.t_end, c.z)
}

/// §6.2: white dwarfs cool by Mestel's law, neutron stars by eq. 93, and black holes give the
/// paper's floor. `age` is Myr since the remnant formed.
fn remnant(end: End, age: f64, z: f64) -> Evolved {
    match end {
        End::WhiteDwarf { mass } => {
            // Eq. 90, with A = 15 for a CO (or ONe) core; eq. 91.
            let l = 635.0 * mass * pow(z, 0.4) / pow(15.0 * (age + 0.1), 1.4);
            let k = pow(giants::M_CH / mass, 2.0 / 3.0) - pow(mass / giants::M_CH, 2.0 / 3.0);
            let r = (0.0115 * libm::sqrt(k.max(1.48204e-6))).min(0.1);
            Evolved { state: State::WhiteDwarf, mass_solar: mass, luminosity_solar: l, teff_k: teff(l, r), radius_solar: r }
        }
        End::NeutronStar { mass } => {
            let l = 0.02 * pow(mass, 2.0 / 3.0) / pow(age.max(0.1), 2.0);
            let r = 1.4e-5;
            Evolved { state: State::NeutronStar, mass_solar: mass, luminosity_solar: l, teff_k: teff(l, r), radius_solar: r }
        }
        End::BlackHole { mass } => {
            Evolved { state: State::BlackHole, mass_solar: mass, luminosity_solar: 0.0, teff_k: 0.0, radius_solar: 4.24e-6 * mass }
        }
    }
}

/// K, from `L = R² (T/T☉)⁴` in solar units.
pub(crate) fn teff(l: f64, r: f64) -> f64 {
    if r <= 0.0 { 0.0 } else { T_SUN * pow(l / (r * r), 0.25) }
}

#[cfg(test)]
mod tests;
