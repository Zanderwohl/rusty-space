use num_traits::Pow;

pub fn unit_circle_xy(x: f64) -> f64 {
    f64::sqrt(1.0 - (x * x))
}

/// Beta parameter from the fourier expansion of true anomaly from mean anomaly
#[inline]
pub fn beta(e: f64) -> f64 {
    (1.0 - f64::sqrt(1.0 - (e * e))) / e
}

#[inline]
pub fn bessel_j(a: f64, x: f64, iterations: f64) -> f64 {
    let mut sum = 0.0;
    let iterations = iterations.round() as i64;
    for m in 0..iterations {
        let numerator = if m % 2 == 0 {1.0} else {-1.0};
        let denominator = factorial(m as f64) * (m as f64 + a + 1.0);
        let last_term_thing = (x / 2.0).pow(2.0 * m as f64 + a);
        let total = (numerator / denominator) * last_term_thing;
        sum += total
    }
    sum
}

fn factorial(n: f64) -> f64 {
    let mut product = 1.0;
    let n = n.round() as i64;
    for factor in 1..(n + 1) {
        product *= factor as f64
    }
    product
}

fn gamma(n: f64) -> f64 {
    if n == 0.0 {
        return f64::INFINITY
    }
    let term1 = 2.0 * (std::f64::consts::PI / n);
    let term2 = n * (n / std::f64::consts::E).ln();
    term1.sqrt() * term2.exp()
}
