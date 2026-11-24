//! What a placed star is: drawn from its own seed and its population's statistics where it
//! fell.
//!
//! The draws go through [`GalaxyRecord`]'s quantization before anything is derived from them,
//! and the derivation is `lc_world::sky::galaxy`'s, so a baked star is exactly what the game
//! loads from a published chunk.

use glam::DVec3;
use lc_world::rng;
use lc_world::sky::galaxy::{GalaxyRecord, Kinematics, tracks};

pub use lc_world::sky::galaxy::tracks::State;

use crate::compiled::{Age, Compiled, Imf};
use crate::feature::FeatureId;
use crate::kinds::MAX_AGE_GYR;
use crate::sample::{StarKey, Tier};

const SALT_ATTRIBUTES: u64 = 0x6174_7472;

/// The IMF's mass range, M☉.
pub const MIN_MASS: f64 = 0.08;
pub const MAX_MASS: f64 = 100.0;

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
    /// What a publish writes; the fields above are derived from it.
    pub record: GalaxyRecord,
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

/// `None` where no population has density, which a surviving candidate never is.
pub fn draw(compiled: &Compiled, seed: u64, key: StarKey, position: DVec3) -> Option<Attributes> {
    let d = Draws(rng::hash(&[seed, key.cell.packed(), key.index as u64, SALT_ATTRIBUTES]));
    let sample = compiled.sample(position);
    let pick = d.u(0) * sample.total();
    let mut acc = 0.0;
    let mut found = None;
    for (slot, (((id, p), n), off)) in
        compiled.populations.iter().zip(&sample.density).zip(&sample.feh_offset).enumerate()
    {
        if *n <= 0.0 {
            continue;
        }
        acc += n;
        // Rounding can leave `pick` a hair past the last sum; the last nonzero one takes it.
        found = Some((slot, *id, p, *off));
        if pick < acc {
            break;
        }
    }
    let (slot, population, pop, offset) = found?;
    let initial = mass(&pop.imf, d.u(1));
    let age = match pop.age {
        Age::Uniform { min_gyr, max_gyr } => min_gyr + d.u(2) * (max_gyr - min_gyr),
        Age::Burst { gyr, spread_gyr } => (gyr + spread_gyr * d.gaussian(2)).clamp(0.0, MAX_AGE_GYR),
    };
    // A held population can outlive its frame; its stars then sit in a static frame at the
    // origin rather than vanish after surviving thinning.
    let radius_kly = compiled.frames.get(&pop.frame).map_or(0.0, |f| {
        let local = f.to_local(position);
        libm::hypot(local.x, local.y) / 1000.0
    });
    let feh = pop.feh.zero_age + pop.feh.per_gyr * age + pop.feh.per_kly * radius_kly + pop.feh.scatter * d.gaussian(3) + offset;
    let record = GalaxyRecord::quantize(key.cell.0.to_array(), key.index, position, initial, age, feh, slot as u8, false);
    let (initial, age) = (record.initial_mass_solar(), record.age_gyr());
    let e = tracks::evolve(initial, age);
    let kin = kinematics(compiled, population).unwrap_or_else(|| Kinematics {
        origin: DVec3::ZERO,
        axes: glam::DMat3::IDENTITY,
        rotation: lc_world::sky::galaxy::Rotation::Static,
        dispersion_km_s: pop.dispersion.zero_age_km_s,
        dispersion_exponent: pop.dispersion.exponent,
    });
    // From the stored position, as the loader computes it.
    let velocity = kin.velocity_km_s(record.position_ly(), age, key.star_id(Tier::B));
    Some(Attributes {
        population: Some(population),
        initial_mass_solar: initial,
        age_gyr: age,
        feh: record.feh(),
        state: e.state,
        mass_solar: e.mass_solar,
        luminosity_solar: e.luminosity_solar,
        teff_k: e.teff_k,
        radius_solar: e.radius_solar,
        velocity_km_s: velocity,
        record,
    })
}

/// What a population's stars need to move, as a chunk carries it.
pub fn kinematics(compiled: &Compiled, population: FeatureId) -> Option<Kinematics> {
    let pop = compiled.populations.get(&population)?;
    let frame = compiled.frames.get(&pop.frame)?;
    Some(Kinematics {
        origin: frame.origin,
        axes: frame.axes,
        rotation: frame.rotation.clone(),
        dispersion_km_s: pop.dispersion.zero_age_km_s,
        dispersion_exponent: pop.dispersion.exponent,
    })
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

