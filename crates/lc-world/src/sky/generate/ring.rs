//! A planet's rings, made from its moons by three rules rather than by simulating a disc.
//!
//! 1. **Breakup.** A moon inside its own Roche limit cannot hold together, so its mass becomes a
//!    band where it was. A giant may also keep a band of what its formation disc left.
//! 2. **Spreading.** A ring spreads; what spreads past the Roche limit gathers into a small moon
//!    just outside it, as Saturn's inner moons did.
//! 3. **Gaps.** A moon inside a band clears a gap a few of its Hill radii wide, as Pan clears the
//!    Encke gap, and a heavy moon further out clears one at its 2:1 resonance, as Mimas clears the
//!    Cassini Division. Either splits the band in two.
//!
//! A band's optical depth follows from its mass spread over its area. The measured systems this
//! has to resemble are in [`crate::rings`].

use std::borrow::Cow;
use std::f64::consts::PI;

use super::architecture::Class;
use super::moon::Moon;
use super::planet::Planet;
use super::tuning::Tuning;
use crate::rings::{Band, RingSystem};
use crate::rng;

/// Band names, inside out. More bands than letters share the last.
const NAMES: [&str; 12] = ["A", "B", "C", "D", "E", "F", "G", "H", "J", "K", "L", "M"];

/// One stretch of ring material before it is a band: radii in meters, mass in kilograms.
#[derive(Clone, Copy, Debug)]
struct Piece {
    inner: f64,
    outer: f64,
    mass: f64,
}

impl Piece {
    fn area(&self) -> f64 {
        PI * (self.outer * self.outer - self.inner * self.inner).max(0.0)
    }

    /// The part between `lo` and `hi`, with the mass that area carries.
    fn between(&self, lo: f64, hi: f64) -> Option<Piece> {
        let (inner, outer) = (self.inner.max(lo), self.outer.min(hi));
        (outer > inner).then(|| {
            let part = Piece { inner, outer, mass: 0.0 };
            Piece { mass: self.mass * part.area() / self.area().max(f64::MIN_POSITIVE), ..part }
        })
    }
}

fn density_of(m: &Moon) -> f64 {
    m.mass_kg / (4.0 / 3.0 * PI * m.radius_m.powi(3)).max(f64::MIN_POSITIVE)
}

fn roche_m(planet: &Planet, density: f64, coefficient: f64) -> f64 {
    let planet_density = planet.mass_kg / (4.0 / 3.0 * PI * planet.radius_m.powi(3)).max(f64::MIN_POSITIVE);
    coefficient * planet.radius_m * (planet_density / density.max(1.0)).cbrt()
}

/// Turns the moons inside their Roche limits into rings, adds the moonlets spreading makes, and
/// cuts the gaps. `moons` is left with the moons that survive and the moonlets.
pub fn form(planet: &Planet, moons: &mut Vec<Moon>, h: u64, tuning: &Tuning) -> Option<RingSystem> {
    let t = &tuning.rings;
    let coefficient = tuning.moons.roche_coefficient;
    let g = |tag: u64| rng::hash(&[h, 0x7193, tag]);
    let floor = planet.radius_m * t.inner_edge_radii;
    let mut pieces = Vec::new();

    let mut broken = 0u64;
    moons.retain(|m| {
        if m.semi_major_m * (1.0 - m.eccentricity) > roche_m(planet, density_of(m), coefficient) {
            return true;
        }
        broken += 1;
        let kept = rng::uniform_in(rng::hash(&[h, 0xdeb, broken]), t.debris_kept.0.ln(), t.debris_kept.1.ln()).exp();
        let half = m.semi_major_m * t.debris_width;
        pieces.push(Piece { inner: (m.semi_major_m - half).max(floor), outer: m.semi_major_m + half, mass: m.mass_kg * kept });
        false
    });

    let limit = roche_m(planet, t.particle_density, coefficient);
    if planet.class.is_giant() && rng::uniform(g(1)) < t.giant_chance {
        let share = rng::uniform_in(g(2), t.giant_mass_ratio.0.ln(), t.giant_mass_ratio.1.ln()).exp();
        pieces.push(Piece { inner: floor, outer: limit, mass: planet.mass_kg * share });
    }

    // Spreading: past the limit, and past the densest a ring stays, ring material is a moonlet.
    let mut spilled = 0.0;
    let mut kept = Vec::new();
    for p in pieces.iter().filter(|p| p.outer > p.inner) {
        spilled += p.between(limit, f64::INFINITY).map_or(0.0, |x| x.mass);
        kept.extend(p.between(floor, limit));
    }
    let mut kept = merged(kept);
    for p in &mut kept {
        let most = t.max_depth * 4.0 * t.particle_density * t.particle_radius_m / 3.0 * p.area();
        if p.mass > most {
            spilled += p.mass - most;
            p.mass = most;
        }
    }
    if spilled > 0.0 {
        let radius = (3.0 * spilled / (4.0 * PI * t.particle_density)).cbrt();
        if radius >= t.moonlet_min_radius_m {
            moons.push(Moon {
                name: String::new(),
                semi_major_m: limit * rng::uniform_in(g(3), 1.02, 1.1),
                eccentricity: rng::uniform_in(g(4), 0.0, 0.005),
                inclination_rad: rng::gaussian(g(5)).abs() * 0.001,
                node_rad: rng::uniform_in(g(6), 0.0, std::f64::consts::TAU),
                mean_anomaly_deg: rng::uniform_in(g(7), 0.0, 360.0),
                radius_m: radius,
                mass_kg: spilled,
                regular: true,
                moons: Vec::new(),
            });
        }
    }

    let mut bands = kept;
    for m in moons.iter() {
        // A moon embedded in a band sweeps its path clear.
        let hill = m.semi_major_m * (m.mass_kg / (3.0 * planet.mass_kg)).cbrt();
        bands = cut(bands, m.semi_major_m, t.gap_hill * hill);
        // A heavy moon outside clears its 2:1 resonance inside.
        if m.mass_kg >= t.resonance_min_ratio * planet.mass_kg {
            let at = m.semi_major_m * 2f64.powf(-2.0 / 3.0);
            bands = cut(bands, at, t.resonance_width * at);
        }
    }

    let out: Vec<Band> = bands
        .iter()
        .filter_map(|p| {
            // tau = 3 Sigma / (4 rho r): the particles' cross-section per unit area.
            let depth = 3.0 * (p.mass / p.area().max(f64::MIN_POSITIVE)) / (4.0 * t.particle_density * t.particle_radius_m);
            (depth >= t.min_depth).then_some((p, depth))
        })
        .enumerate()
        .map(|(k, (p, depth))| Band { name: NAMES[k.min(NAMES.len() - 1)], inner_m: p.inner, outer_m: p.outer, optical_depth: depth })
        .collect();
    if out.is_empty() {
        return None;
    }
    let icy = matches!(planet.class, Class::Icy | Class::IceGiant | Class::GasGiant);
    let (lo, hi) = if icy { t.icy_albedo } else { t.rocky_albedo };
    Some(RingSystem { bands: Cow::Owned(out), albedo: rng::uniform_in(g(8), lo, hi) })
}

/// Overlapping pieces as one, inside out.
fn merged(mut pieces: Vec<Piece>) -> Vec<Piece> {
    pieces.sort_by(|a, b| a.inner.total_cmp(&b.inner));
    let mut out: Vec<Piece> = Vec::new();
    for p in pieces {
        match out.last_mut() {
            Some(last) if p.inner <= last.outer => {
                last.outer = last.outer.max(p.outer);
                last.mass += p.mass;
            }
            _ => out.push(p),
        }
    }
    out
}

/// `bands` with the stretch within `half` of `at` cleared, and its mass gone into whatever
/// swept it.
fn cut(bands: Vec<Piece>, at: f64, half: f64) -> Vec<Piece> {
    bands
        .into_iter()
        .flat_map(|p| {
            if at + half <= p.inner || at - half >= p.outer {
                return vec![p];
            }
            [p.between(p.inner, at - half), p.between(at + half, p.outer)].into_iter().flatten().collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cut_splits_a_band_and_keeps_its_density() {
        let band = Piece { inner: 1.0, outer: 3.0, mass: 8.0 * PI };
        let parts = cut(vec![band], 2.0, 0.1);
        assert_eq!(parts.len(), 2);
        assert!((parts[0].outer - 1.9).abs() < 1e-12 && (parts[1].inner - 2.1).abs() < 1e-12);
        let sigma = |p: &Piece| p.mass / p.area();
        assert!((sigma(&parts[0]) - sigma(&band)).abs() < 1e-12 && (sigma(&parts[1]) - sigma(&band)).abs() < 1e-12);
        assert_eq!(cut(vec![band], 5.0, 0.1).len(), 1, "a gap outside the band leaves it whole");
    }

    #[test]
    fn overlapping_pieces_merge_with_their_mass() {
        let m = merged(vec![Piece { inner: 2.0, outer: 4.0, mass: 1.0 }, Piece { inner: 1.0, outer: 3.0, mass: 2.0 }, Piece { inner: 5.0, outer: 6.0, mass: 4.0 }]);
        assert_eq!(m.len(), 2);
        assert_eq!((m[0].inner, m[0].outer, m[0].mass), (1.0, 4.0, 3.0));
    }

    /// Every band lies between the inner edge and the ice Roche limit, inside out without
    /// overlap, at least the faintest depth kept; and giants are ringed far more often than
    /// anything else.
    #[test]
    fn rings_sit_inside_the_roche_limit_and_mostly_round_giants() {
        use crate::sky::generate::{architecture::tests_support::sun_like, system_with};
        let tuning = Tuning::default();
        let t = &tuning.rings;
        let (mut giants, mut ringed_giants, mut small, mut ringed_small) = (0, 0, 0, 0);
        for k in 0..120 {
            for p in system_with(&sun_like(k), &tuning).planets {
                let giant = p.class.is_giant();
                if giant { giants += 1 } else { small += 1 }
                let Some(r) = &p.rings else { continue };
                if giant { ringed_giants += 1 } else { ringed_small += 1 }
                let limit = roche_m(&p, t.particle_density, tuning.moons.roche_coefficient);
                for b in r.bands.iter() {
                    assert!(b.inner_m >= p.radius_m * t.inner_edge_radii * (1.0 - 1e-12) && b.outer_m <= limit * (1.0 + 1e-12), "{}: {b:?}", p.name);
                    assert!(b.outer_m > b.inner_m && b.optical_depth >= t.min_depth, "{}: {b:?}", p.name);
                }
                assert!(r.bands.windows(2).all(|w| w[0].outer_m <= w[1].inner_m), "{}: bands overlap", p.name);
            }
        }
        let share = |n: i32, d: i32| n as f64 / d.max(1) as f64;
        assert!(share(ringed_giants, giants) > 0.5, "{ringed_giants} of {giants} giants ringed");
        assert!(share(ringed_small, small) < share(ringed_giants, giants) / 3.0, "{ringed_small} of {small} small planets ringed");
    }

}
