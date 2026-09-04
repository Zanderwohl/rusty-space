
/// Laplace sphere of influence. Least accurate of the three.
pub fn laplace(semi_major_axis: f64, body_mass: f64, primary_mass: f64) -> f64 {
    semi_major_axis * f64::powf(body_mass / primary_mass, 2.0 / 5.0)
}

/// Laplace SOI at angle `theta` from the primary.
pub fn laplace_angled(semi_major_axis: f64, body_mass: f64, primary_mass: f64, theta: f64) -> f64 {
    let cos = f64::cos(theta);
    let denominator = f64::powf(1.0 + (3.0 * cos * cos), 0.1);
    laplace(semi_major_axis, body_mass, primary_mass) / denominator
}

/// Laplace SOI averaged over all directions; slightly more accurate than [`laplace`].
pub fn laplace_integrated(semi_major_axis: f64, body_mass: f64, primary_mass: f64) -> f64 {
    0.9431 * laplace(semi_major_axis, body_mass, primary_mass)
}

/// Hill sphere. More accurate than Laplace, but depends on instantaneous radius;
/// pass the semi-major axis for a static approximation.
pub fn hill(instantaneous_r_to_primary: f64, eccentricity: f64, body_mass: f64, primary_mass: f64) -> f64 {
    let frac = body_mass / (3.0 * (body_mass + primary_mass));
    instantaneous_r_to_primary * (1.0 - eccentricity) * f64::cbrt(frac)
}

/// Bondi accretion radius of a black hole.
pub fn black_hole(gravitational_constant: f64, mass: f64, velocity_dispersion: f64) -> f64 {
    (gravitational_constant * mass) / (velocity_dispersion * velocity_dispersion)
}
