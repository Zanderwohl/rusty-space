//! What a placed star is: drawn from its own seed and its population's statistics where it
//! fell.
//!
//! **The tracks are a placeholder**: textbook power laws for the main sequence, one giant
//! branch and fixed remnants, enough to make a star list whose HR diagram has the right shape.
//! The analytic tracks of `34-galaxy-authoring.md` (Hurley, Pols & Tout) replace them.

use glam::DVec3;
use lc_world::rng;

use crate::compiled::{Age, Compiled, Imf, Population};
use crate::feature::FeatureId;
use crate::kinds::MAX_AGE_GYR;
use crate::sample::StarKey;

const SALT_ATTRIBUTES: u64 = 0x6174_7472;

/// The IMF's mass range, M☉.
pub const MIN_MASS: f64 = 0.08;
pub const MAX_MASS: f64 = 100.0;

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

#[derive(Clone, Debug, PartialEq)]
pub struct Attributes {
    pub population: Option<FeatureId>,
    pub initial_mass_solar: f64,
    pub age_gyr: f64,
    pub feh: f64,
    pub state: State,
    pub mass_solar: f64,
    pub luminosity_solar: f64,
    pub teff_k: f64,
    pub radius_solar: f64,
    pub velocity_km_s: DVec3,
}

struct Draws(u64);

impl Draws {
    fn u(&self, n: u64) -> f64 {
        rng::uniform(rng::mix(self.0 ^ n.wrapping_mul(0x9e37_79b9_7f4a_7c15)))
    }

    fn gaussian(&self, n: u64) -> f64 {
        let u1 = self.u(n * 2 + 1000).max(f64::MIN_POSITIVE);
        let u2 = self.u(n * 2 + 1001);
        libm::sqrt(-2.0 * libm::log(u1)) * libm::cos(std::f64::consts::TAU * u2)
    }
}

pub fn draw(compiled: &Compiled, seed: u64, key: StarKey, position: DVec3) -> Attributes {
    let d = Draws(rng::hash(&[seed, key.cell.packed(), key.index as u64, SALT_ATTRIBUTES]));
    let sample = compiled.sample(position);
    let total = sample.total();
    let pick = d.u(0) * total;
    let mut acc = 0.0;
    let mut found = None;
    for (((id, p), n), off) in compiled.populations.iter().zip(&sample.density).zip(&sample.feh_offset) {
        if *n <= 0.0 {
            continue;
        }
        acc += n;
        // Rounding can leave `pick` a hair past the last sum; the last nonzero one takes it.
        found = Some((*id, p, *off));
        if pick < acc {
            break;
        }
    }
    let Some((population, pop, offset)) = found else {
        return Attributes::bare(mass(&Imf::Kroupa, d.u(1)));
    };
    let initial = mass(&pop.imf, d.u(1));
    let age = match pop.age {
        Age::Uniform { min_gyr, max_gyr } => min_gyr + d.u(2) * (max_gyr - min_gyr),
        Age::Burst { gyr, spread_gyr } => (gyr + spread_gyr * d.gaussian(2)).clamp(0.0, MAX_AGE_GYR),
    };
    let frame = compiled.frames.get(&pop.frame);
    let radius_kly = frame.map_or(0.0, |f| {
        let l = f.to_local(position);
        libm::hypot(l.x, l.y) / 1000.0
    });
    let feh = (pop.feh.zero_age + pop.feh.per_gyr * age + pop.feh.per_kly * radius_kly
        + pop.feh.scatter * d.gaussian(3)
        + offset)
        .clamp(-4.0, 1.0);
    let velocity = frame.map_or(DVec3::ZERO, |f| f.circular_velocity(position)) + dispersion(pop, age, &d);
    let (state, mass_now, luminosity, teff) = track(initial, age);
    Attributes {
        population: Some(population),
        initial_mass_solar: initial,
        age_gyr: age,
        feh,
        state,
        mass_solar: mass_now,
        luminosity_solar: luminosity,
        teff_k: teff,
        radius_solar: radius(luminosity, teff, state),
        velocity_km_s: velocity,
    }
}

impl Attributes {
    fn bare(initial: f64) -> Self {
        let (state, mass_solar, luminosity_solar, teff_k) = track(initial, 0.0);
        Self {
            population: None,
            initial_mass_solar: initial,
            age_gyr: 0.0,
            feh: 0.0,
            state,
            mass_solar,
            luminosity_solar,
            teff_k,
            radius_solar: radius(luminosity_solar, teff_k, state),
            velocity_km_s: DVec3::ZERO,
        }
    }
}

fn dispersion(pop: &Population, age: f64, d: &Draws) -> DVec3 {
    let sigma = pop.dispersion.zero_age_km_s * libm::pow(1.0 + age, pop.dispersion.exponent);
    DVec3::new(d.gaussian(4), d.gaussian(5), d.gaussian(6)) * sigma
}

/// Inverts the cumulative of a broken power law `ξ(m) ∝ m^-α`.
pub fn mass(imf: &Imf, u: f64) -> f64 {
    let segments: &[(f64, f64, f64)] = match imf {
        Imf::Kroupa => &[(MIN_MASS, 0.5, 1.3), (0.5, MAX_MASS, 2.3)],
        Imf::Salpeter => &[(MIN_MASS, MAX_MASS, 2.35)],
    };
    // Each segment's weight, continuous at the breaks.
    let mut scale = 1.0;
    let mut weights = Vec::with_capacity(segments.len());
    for (i, &(lo, hi, alpha)) in segments.iter().enumerate() {
        if i > 0 {
            let (_, prev_hi, prev_alpha) = segments[i - 1];
            scale *= libm::pow(prev_hi, alpha - prev_alpha);
        }
        weights.push(scale * power_integral(lo, hi, alpha));
    }
    let total: f64 = weights.iter().sum();
    let mut pick = u * total;
    for (&(lo, hi, alpha), w) in segments.iter().zip(&weights) {
        if pick <= *w {
            let f = pick / w * power_integral(lo, hi, alpha);
            let k = 1.0 - alpha;
            return libm::pow(libm::pow(lo, k) + f * k, 1.0 / k).clamp(lo, hi);
        }
        pick -= w;
    }
    MAX_MASS
}

fn power_integral(lo: f64, hi: f64, alpha: f64) -> f64 {
    let k = 1.0 - alpha;
    (libm::pow(hi, k) - libm::pow(lo, k)) / k
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

/// `(state, current mass M☉, luminosity L☉, T_eff K)`.
fn track(m: f64, age: f64) -> (State, f64, f64, f64) {
    let t_ms = lifetime_gyr(m);
    if age < t_ms {
        let l = main_sequence_luminosity(m);
        let r = main_sequence_radius(m);
        return (State::MainSequence, m, l, T_SUN * libm::pow(l / (r * r), 0.25));
    }
    if age < 1.1 * t_ms {
        return if m < 8.0 { (State::Giant, m, 100.0 * m * m, 4500.0) } else { (State::Giant, m, main_sequence_luminosity(m), 4000.0) };
    }
    if m < 8.0 {
        let cooling = (age - 1.1 * t_ms).max(1e-3);
        let l = (0.01 * libm::pow(cooling / 0.1, -1.4)).min(0.1);
        let r = 0.0126;
        (State::WhiteDwarf, 0.109 * m + 0.394, l, T_SUN * libm::pow(l / (r * r), 0.25))
    } else if m < 25.0 {
        (State::NeutronStar, 1.4, 0.0, 0.0)
    } else {
        (State::BlackHole, 0.3 * m, 0.0, 0.0)
    }
}

/// R☉, from `L = R² (T/T☉)⁴`.
fn radius(luminosity: f64, teff: f64, state: State) -> f64 {
    match state {
        State::NeutronStar => 1.4e-5,
        State::BlackHole => 0.0,
        _ if teff > 0.0 => libm::sqrt(luminosity) * (T_SUN / teff).powi(2),
        _ => 0.0,
    }
}
