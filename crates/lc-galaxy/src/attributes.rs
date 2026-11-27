//! A placed star's attributes, drawn from its own seed and its population's statistics.
//!
//! The draws are quantized through [`GalaxyRecord`] before anything is derived from them, and
//! derived with `lc_world::sky::galaxy`, so a baked star matches what the game loads.

use glam::DVec3;
use lc_world::rng;
use lc_world::sky::galaxy::{GalaxyRecord, Kinematics};
use lc_world::sky::generate::SystemProfile;

pub use lc_world::sky::galaxy::tracks;
pub use lc_world::sky::galaxy::tracks::State;

use crate::compiled::{Age, Compiled, Imf, Population};
use crate::feature::FeatureId;
use crate::kinds::MAX_AGE_GYR;
use crate::sample::{StarKey, Tier};

const SALT_ATTRIBUTES: u64 = 0x6174_7472;
/// A tier-A star's draws, apart from tier B's so the two never share a stream.
const SALT_LUMINOUS: u64 = 0x6c75_6d69;

/// Doc 34's `L_A`, L☉: brighter than this, a star is seen from across the galaxy and is tier A.
pub const L_A: f64 = 100.0;

/// Mass and age draws a tier-A star may take to come out luminous before it is dropped. A
/// population's luminous share is under a percent, so a hundred times its inverse.
const LUMINOUS_ATTEMPTS: u64 = 100_000;

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
    /// The system fields here, quantized; a publish turns it into the record's profile index.
    pub profile: SystemProfile,
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
///
/// A tier-B star draws its population in proportion to density and its mass and age once; one
/// that comes out luminous belongs to tier A, and the bake drops it. A tier-A star draws its
/// population in proportion to density times `shares`, each population's chance of a star above
/// [`L_A`], then redraws mass and age until it is luminous, or gives `None` after
/// [`LUMINOUS_ATTEMPTS`].
pub fn draw(compiled: &Compiled, seed: u64, key: StarKey, position: DVec3, tier: Tier, shares: &[f64]) -> Option<Attributes> {
    let salt = if tier == Tier::A { SALT_LUMINOUS } else { SALT_ATTRIBUTES };
    let d = Draws(rng::hash(&[seed, key.cell.packed(), key.index as u64, salt]));
    let sample = compiled.sample(position);
    let weight = |slot: usize, n: f64| if tier == Tier::A { n * shares.get(slot).copied().unwrap_or(0.0) } else { n };
    let total: f64 = sample.density.iter().enumerate().map(|(i, n)| weight(i, *n)).sum();
    let pick = d.u(0) * total;
    let mut acc = 0.0;
    let mut found = None;
    for (slot, (((id, p), n), off)) in
        compiled.populations.iter().zip(&sample.density).zip(&sample.feh_offset).enumerate()
    {
        let n = &weight(slot, *n);
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
    // A held population can outlive its frame; its stars then sit in a static frame at the
    // origin rather than vanish after surviving thinning.
    let radius_kly = compiled.frames.get(&pop.frame).map_or(0.0, |f| {
        let local = f.to_local(position);
        libm::hypot(local.x, local.y) / 1000.0
    });
    let attempts = if tier == Tier::A { LUMINOUS_ATTEMPTS } else { 1 };
    // Tier A draws only from masses that can be luminous at some age the population has.
    let floor = if tier == Tier::A { imf_fraction(&pop.imf, MIN_MASS, luminous_floor(pop, u64::from(population.0))) } else { 0.0 };
    let (record, e) = (0..attempts).find_map(|attempt| {
        // Tier B's first attempt reads draws 1 to 3, as every star did before tiers.
        let n = |k: u64| k + attempt * 4;
        let initial = mass(&pop.imf, floor + d.u(n(1)) * (1.0 - floor));
        let age = match pop.age {
            Age::Uniform { min_gyr, max_gyr } => min_gyr + d.u(n(2)) * (max_gyr - min_gyr),
            Age::Burst { gyr, spread_gyr } => (gyr + spread_gyr * d.gaussian(n(2))).clamp(0.0, MAX_AGE_GYR),
        };
        let feh = pop.feh.zero_age + pop.feh.per_gyr * age + pop.feh.per_kly * radius_kly + pop.feh.scatter * d.gaussian(n(3)) + offset;
        let record = GalaxyRecord::quantize(key.cell.0.to_array(), key.index, position, initial, age, feh, slot as u8, tier == Tier::A);
        let e = tracks::evolve_step(record.initial_mass_solar(), record.age_gyr(), record.feh);
        (tier == Tier::B || e.luminosity_solar >= L_A).then_some((record, e))
    })?;
    let (initial, age) = (record.initial_mass_solar(), record.age_gyr());
    let kin = kinematics(compiled, population).unwrap_or_else(|| Kinematics {
        origin: DVec3::ZERO,
        axes: glam::DMat3::IDENTITY,
        rotation: lc_world::sky::galaxy::Rotation::Static,
        dispersion_km_s: pop.dispersion.zero_age_km_s,
        dispersion_exponent: pop.dispersion.exponent,
    });
    // From the stored position, as the loader computes it.
    let velocity = kin.velocity_km_s(record.position_ly(), age, key.star_id(tier));
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
        profile: sample.profile(),
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
    let (segments, scales) = segments(imf);
    let weights: Vec<f64> = segments.iter().zip(&scales).map(|(&(lo, hi, alpha), s)| s * power_integral(lo, hi, alpha)).collect();
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

/// The IMF's power-law segments `(lo, hi, α)`, and each one's scale, continuous at the breaks.
fn segments(imf: &Imf) -> (&'static [(f64, f64, f64)], Vec<f64>) {
    let segments: &[(f64, f64, f64)] = match imf {
        Imf::Kroupa => &[(MIN_MASS, 0.5, 1.3), (0.5, MAX_MASS, 2.3)],
        Imf::Salpeter => &[(MIN_MASS, MAX_MASS, 2.35)],
    };
    let mut scale = 1.0;
    let scales = segments
        .iter()
        .enumerate()
        .map(|(i, &(_, _, alpha))| {
            if i > 0 {
                let (_, prev_hi, prev_alpha) = segments[i - 1];
                scale *= libm::pow(prev_hi, alpha - prev_alpha);
            }
            scale
        })
        .collect();
    (segments, scales)
}

/// The fraction of stars the IMF draws with initial mass in `[lo, hi]` M☉.
pub fn imf_fraction(imf: &Imf, lo: f64, hi: f64) -> f64 {
    let (segments, scales) = segments(imf);
    let part = |a: f64, b: f64| -> f64 {
        segments
            .iter()
            .zip(&scales)
            .map(|(&(s_lo, s_hi, alpha), s)| {
                let (x, y) = (a.max(s_lo), b.min(s_hi));
                if y > x { s * power_integral(x, y, alpha) } else { 0.0 }
            })
            .sum()
    };
    part(lo, hi) / part(MIN_MASS, MAX_MASS)
}

/// The fraction of a population's stars with age in `[lo, hi]` Gyr.
pub fn age_fraction(age: &Age, lo: f64, hi: f64) -> f64 {
    match *age {
        Age::Uniform { min_gyr, max_gyr } if max_gyr > min_gyr => {
            ((hi.min(max_gyr) - lo.max(min_gyr)) / (max_gyr - min_gyr)).max(0.0)
        }
        Age::Uniform { min_gyr, .. } => f64::from(lo <= min_gyr && min_gyr < hi),
        Age::Burst { gyr, spread_gyr } if spread_gyr > 0.0 => {
            // Clipped to the age range as the draw is, so the clipped tails pile up at its ends.
            let cdf = |x: f64| {
                if x <= 0.0 {
                    0.0
                } else if x >= MAX_AGE_GYR {
                    1.0
                } else {
                    0.5 * (1.0 + libm::erf((x - gyr) / (spread_gyr * std::f64::consts::SQRT_2)))
                }
            };
            cdf(hi) - cdf(lo)
        }
        Age::Burst { gyr, .. } => f64::from(lo <= gyr && gyr < hi),
    }
}

fn power_integral(lo: f64, hi: f64, alpha: f64) -> f64 {
    let k = 1.0 - alpha;
    (libm::pow(hi, k) - libm::pow(lo, k)) / k
}


/// The share of `population`'s stars brighter than [`L_A`]: masses stratified over the IMF and
/// ages hashed, through the tracks at the population's zero-age `[Fe/H]` trend. Ignores the
/// radial gradient and offsets, which move a luminosity far less than mass and age do.
pub fn luminous_share(population: &Population, salt: u64) -> f64 {
    // A function of the population's statistics and the salt alone, and costly: every bake and
    // every edit asks for it. Keyed by the statistics' debug text, since they hold floats.
    static SHARES: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, f64>>> = std::sync::OnceLock::new();
    let key = format!("{salt} {:?} {:?} {:?}", population.imf, population.age, population.feh);
    let shares = SHARES.get_or_init(Default::default);
    if let Some(&share) = shares.lock().unwrap().get(&key) {
        return share;
    }
    let share = compute_luminous_share(population, salt);
    shares.lock().unwrap().insert(key, share);
    share
}

/// A mass under which no star of `population` reaches [`L_A`] at any age it has: one step of a
/// 96-step logarithmic mass grid below the least mass that does, over 64 ages. One step of
/// margin, because a luminous phase as brief as the AGB tip can fall between the ages sampled.
fn luminous_floor(population: &Population, salt: u64) -> f64 {
    static FLOORS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, f64>>> = std::sync::OnceLock::new();
    let key = format!("{salt} {:?} {:?} {:?}", population.imf, population.age, population.feh);
    let floors = FLOORS.get_or_init(Default::default);
    if let Some(&m) = floors.lock().unwrap().get(&key) {
        return m;
    }
    const MASSES: usize = 96;
    const AGES: usize = 64;
    let (young, old) = match population.age {
        Age::Uniform { min_gyr, max_gyr } => (min_gyr, max_gyr),
        Age::Burst { gyr, spread_gyr } => ((gyr - 4.0 * spread_gyr).max(0.0), (gyr + 4.0 * spread_gyr).min(MAX_AGE_GYR)),
    };
    let at = |i: usize| MIN_MASS * (MAX_MASS / MIN_MASS).powf(i as f64 / (MASSES - 1) as f64);
    let lit = |m: f64| {
        (0..AGES).any(|j| {
            let age = young + (old - young) * j as f64 / (AGES - 1) as f64;
            let feh = population.feh.zero_age + population.feh.per_gyr * age;
            let step = (feh / lc_world::sky::galaxy::FEH_STEP).round().clamp(-128.0, 127.0) as i8;
            tracks::evolve_step(m, age, step).luminosity_solar >= L_A
        })
    };
    let floor = (0..MASSES).find(|&i| lit(at(i))).map_or(MAX_MASS, |i| at(i.saturating_sub(1)));
    floors.lock().unwrap().insert(key, floor);
    floor
}

fn compute_luminous_share(population: &Population, salt: u64) -> f64 {
    const DRAWS: u64 = 50_000;
    let lit = (0..DRAWS)
        .filter(|&i| {
            let h = |n: u64| rng::uniform(rng::hash(&[salt, i, n, SALT_LUMINOUS]));
            let initial = mass(&population.imf, (i as f64 + 0.5) / DRAWS as f64);
            let age = match population.age {
                Age::Uniform { min_gyr, max_gyr } => min_gyr + h(1) * (max_gyr - min_gyr),
                Age::Burst { gyr, spread_gyr } => {
                    let g = libm::sqrt(-2.0 * libm::log(h(2).max(f64::MIN_POSITIVE))) * libm::cos(std::f64::consts::TAU * h(3));
                    (gyr + spread_gyr * g).clamp(0.0, MAX_AGE_GYR)
                }
            };
            let feh = population.feh.zero_age + population.feh.per_gyr * age;
            let step = (feh / lc_world::sky::galaxy::FEH_STEP).round().clamp(-128.0, 127.0) as i8;
            tracks::evolve_step(initial, age, step).luminosity_solar >= L_A
        })
        .count();
    lit as f64 / DRAWS as f64
}
