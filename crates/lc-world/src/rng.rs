//! Deterministic, stateless, position-in-time addressable noise.
//!
//! Every stochastic quantity in the world is drawn from a hash of its address, never from a
//! sequential generator. Two clients must agree on what a star did, and a value must be
//! recoverable at an arbitrary past time without having generated everything before it.

/// SplitMix64 finaliser.
#[inline]
pub fn mix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Combine an address into one hash.
#[inline]
pub fn hash(parts: &[u64]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for p in parts {
        h = mix(h ^ p.wrapping_mul(0x100_0000_01b3));
    }
    h
}

/// Uniform on `[0, 1)`.
#[inline]
pub fn uniform(h: u64) -> f64 {
    (h >> 11) as f64 / (1u64 << 53) as f64
}

/// Uniform on `[lo, hi)`.
#[inline]
pub fn uniform_in(h: u64, lo: f64, hi: f64) -> f64 {
    lo + uniform(h) * (hi - lo)
}

/// Standard normal, by Box-Muller on two independent hashes of the same address.
pub fn gaussian(h: u64) -> f64 {
    let u1 = uniform(mix(h)).max(f64::MIN_POSITIVE);
    let u2 = uniform(mix(h ^ 0x5bf0_3635));
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

/// Poisson count, by inversion. Fine for the small means this is used with.
pub fn poisson(h: u64, mean: f64) -> u32 {
    if mean <= 0.0 {
        return 0;
    }
    debug_assert!(
        mean < 30.0,
        "inversion is the wrong algorithm above a mean of ~30"
    );
    let target = uniform(h);
    let mut cumulative = (-mean).exp();
    let mut term = cumulative;
    let mut k = 0u32;
    while cumulative < target && k < 64 {
        k += 1;
        term *= mean / k as f64;
        cumulative += term;
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_is_deterministic_and_address_sensitive() {
        assert_eq!(hash(&[1, 2, 3]), hash(&[1, 2, 3]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[1, 2, 4]));
        assert_ne!(hash(&[1, 2, 3]), hash(&[3, 2, 1]));
    }

    #[test]
    fn uniforms_are_in_range_and_flat() {
        let mut buckets = [0u32; 10];
        for i in 0..100_000u64 {
            let u = uniform(hash(&[i]));
            assert!((0.0..1.0).contains(&u));
            buckets[(u * 10.0) as usize] += 1;
        }
        for b in buckets {
            assert!(
                (b as f64 - 10_000.0).abs() < 500.0,
                "bucket {b} is not flat"
            );
        }
    }

    #[test]
    fn gaussians_have_unit_mean_and_variance() {
        let n = 200_000;
        let vals: Vec<f64> = (0..n).map(|i| gaussian(hash(&[i, 7]))).collect();
        let mean = vals.iter().sum::<f64>() / n as f64;
        let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.01, "mean {mean}");
        assert!((var - 1.0).abs() < 0.02, "variance {var}");
    }

    #[test]
    fn poisson_counts_have_the_requested_mean() {
        for lambda in [0.01, 0.5, 3.0] {
            let n = 200_000;
            let total: u64 = (0..n).map(|i| poisson(hash(&[i, 11]), lambda) as u64).sum();
            let got = total as f64 / n as f64;
            assert!(
                (got / lambda - 1.0).abs() < 0.05,
                "lambda {lambda}: got {got}"
            );
        }
        assert_eq!(poisson(hash(&[1]), 0.0), 0);
    }
}
