//! Gradient noise in `f64` for the bake: Perlin's improved noise (2002) over integer hashing
//! instead of his permutation table, summed into fractal Brownian motion.
//!
//! A published star's existence depends on these values, so they must never change and must
//! agree on every machine. Only `+`, `*` and `floor` are used, no transcendental and no fused
//! multiply-add, and the tests pin outputs. Not texture-graph's noise: that is `f32` and lives in
//! another repository that changes for its own reasons.

use glam::DVec3;

/// Each octave's frequency over the last's.
const LACUNARITY: f64 = 2.0;
/// Each octave's amplitude over the last's.
const GAIN: f64 = 0.5;

/// Bound on `|∇ gradient(p)|` for unit frequency, measured over a dense sample with a margin;
/// the test that measured it keeps it honest.
pub const GRADIENT_LIPSCHITZ: f64 = 4.0;

/// The twelve edge directions of a cube, as Perlin's improved noise uses.
const GRADIENTS: [[f64; 3]; 12] = [
    [1.0, 1.0, 0.0], [-1.0, 1.0, 0.0], [1.0, -1.0, 0.0], [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0], [-1.0, 0.0, 1.0], [1.0, 0.0, -1.0], [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0], [0.0, -1.0, 1.0], [0.0, 1.0, -1.0], [0.0, -1.0, -1.0],
];

fn mix(mut h: u64) -> u64 {
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
}

/// One mix per corner: the three odd multipliers spread neighboring lattice points across the
/// word before it. Eight corners per sample made the nested form the noise's main cost.
fn corner_hash(i: i64, j: i64, k: i64, seed: u64) -> u64 {
    mix(seed
        ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (j as u64).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
        ^ (k as u64).wrapping_mul(0x1656_67b1_9e37_79f9))
}

fn corner(i: i64, j: i64, k: i64, seed: u64) -> [f64; 3] {
    GRADIENTS[(corner_hash(i, j, k, seed) % 12) as usize]
}

/// Three independent gradient noises at `p`, one per axis, from one lattice walk: each corner's
/// hash picks the three gradients from separate bits, so a warp pays for one hash per corner
/// rather than three.
pub fn gradient3(p: DVec3, seed: u64) -> DVec3 {
    let base = p.floor();
    let f = p - base;
    let (i, j, k) = (base.x as i64, base.y as i64, base.z as i64);
    let (u, v, w) = (fade(f.x), fade(f.y), fade(f.z));
    let dot = |di: i64, dj: i64, dk: i64| {
        let h = corner_hash(i + di, j + dj, k + dk, seed);
        let d = DVec3::new(f.x - di as f64, f.y - dj as f64, f.z - dk as f64);
        let g = |shift: u32| {
            let g = GRADIENTS[((h >> shift) % 12) as usize];
            g[0] * d.x + g[1] * d.y + g[2] * d.z
        };
        DVec3::new(g(0), g(21), g(42))
    };
    let lerp = |a: DVec3, b: DVec3, t: f64| a + (b - a) * t;
    let x00 = lerp(dot(0, 0, 0), dot(1, 0, 0), u);
    let x10 = lerp(dot(0, 1, 0), dot(1, 1, 0), u);
    let x01 = lerp(dot(0, 0, 1), dot(1, 0, 1), u);
    let x11 = lerp(dot(0, 1, 1), dot(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Perlin's improved noise at `p`, about `[-1, 1]`, varying over a unit length.
pub fn gradient(p: DVec3, seed: u64) -> f64 {
    let base = p.floor();
    let f = p - base;
    let (i, j, k) = (base.x as i64, base.y as i64, base.z as i64);
    let (u, v, w) = (fade(f.x), fade(f.y), fade(f.z));
    let dot = |di: i64, dj: i64, dk: i64| {
        let g = corner(i + di, j + dj, k + dk, seed);
        g[0] * (f.x - di as f64) + g[1] * (f.y - dj as f64) + g[2] * (f.z - dk as f64)
    };
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;
    let x00 = lerp(dot(0, 0, 0), dot(1, 0, 0), u);
    let x10 = lerp(dot(0, 1, 0), dot(1, 1, 0), u);
    let x01 = lerp(dot(0, 0, 1), dot(1, 0, 1), u);
    let x11 = lerp(dot(0, 1, 1), dot(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// Fractal Brownian motion: `octaves` of [`gradient`], the first varying over `scale` ly,
/// normalized to about `[-1, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fbm {
    pub scale: f64,
    pub octaves: u32,
    pub seed: u64,
}

impl Fbm {
    pub fn at(&self, p: DVec3) -> f64 {
        let (mut sum, mut norm, mut amplitude, mut frequency) = (0.0, 0.0, 1.0, 1.0 / self.scale);
        for octave in 0..self.octaves {
            sum += amplitude * gradient(p * frequency, mix(self.seed ^ u64::from(octave)));
            norm += amplitude;
            amplitude *= GAIN;
            frequency *= LACUNARITY;
        }
        (sum / norm.max(f64::MIN_POSITIVE)).clamp(-1.0, 1.0)
    }

    /// Bound on `|∇ at(p)|`, per ly: each octave doubles the frequency and halves the amplitude,
    /// so each adds the same gradient.
    pub fn lipschitz(&self) -> f64 {
        let norm: f64 = (0..self.octaves).map(|o| GAIN.powi(o as i32)).sum();
        GRADIENT_LIPSCHITZ * f64::from(self.octaves) / (self.scale * norm.max(f64::MIN_POSITIVE))
    }

    /// Three independent fields, one per axis, each component in `[-1, 1]`.
    pub fn vector(&self, p: DVec3) -> DVec3 {
        let (mut sum, mut norm, mut amplitude, mut frequency) = (DVec3::ZERO, 0.0, 1.0, 1.0 / self.scale);
        for octave in 0..self.octaves {
            sum += amplitude * gradient3(p * frequency, mix(self.seed ^ 0xa11e_0000 ^ u64::from(octave)));
            norm += amplitude;
            amplitude *= GAIN;
            frequency *= LACUNARITY;
        }
        (sum / norm.max(f64::MIN_POSITIVE)).clamp(DVec3::splat(-1.0), DVec3::splat(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moving these moves published stars.
    #[test]
    fn values_are_pinned() {
        let n = |x: f64, y: f64, z: f64| gradient(DVec3::new(x, y, z), 7);
        let got = [n(0.3, 0.7, 0.1), n(-12.25, 4.5, 3.75), n(1000.1, -2000.2, 3000.3)];
        let fbm = Fbm { scale: 5.0, octaves: 4, seed: 11 }.at(DVec3::new(3.0, -8.0, 1.5));
        let vector = Fbm { scale: 5.0, octaves: 3, seed: 11 }.vector(DVec3::new(-3.0, 8.0, 0.5));
        assert_eq!(vector.to_array().map(f64::to_bits), PINNED_VECTOR, "{vector}");
        assert_eq!(got.map(f64::to_bits), PINNED, "{got:?}");
        assert_eq!(fbm.to_bits(), PINNED_FBM, "{fbm}");
    }

    const PINNED: [u64; 3] = [4592299264490357748, 13822795700313784320, 4599250037624513667];
    const PINNED_VECTOR: [u64; 3] = [4597099239458411315, 13809141359698438839, 13819629432261112647];
    const PINNED_FBM: u64 = 4586090178363346149;

    #[test]
    fn noise_is_zero_on_the_lattice_and_bounded_between() {
        assert_eq!(gradient(DVec3::new(3.0, -4.0, 5.0), 1), 0.0);
        let mut steepest: f64 = 0.0;
        for i in 0..200_000u64 {
            let f = |k: u64| (mix(i * 3 + k) >> 11) as f64 / (1u64 << 53) as f64 * 40.0 - 20.0;
            let p = DVec3::new(f(0), f(1), f(2));
            let v = gradient(p, 3);
            assert!(v.abs() <= 1.0, "{v} at {p}");
            let h = 1e-5;
            let g = DVec3::new(
                gradient(p + DVec3::X * h, 3) - v,
                gradient(p + DVec3::Y * h, 3) - v,
                gradient(p + DVec3::Z * h, 3) - v,
            ) / h;
            steepest = steepest.max(g.length());
        }
        assert!(steepest < GRADIENT_LIPSCHITZ, "steepest gradient {steepest}");
        assert!(steepest > GRADIENT_LIPSCHITZ * 0.5, "the bound {GRADIENT_LIPSCHITZ} is loose: {steepest}");
    }
}
