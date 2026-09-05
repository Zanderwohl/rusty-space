//! Spheres of influence: where one attractor's pull dominates another's.
//!
//! Every radius here is in metres, measured from the smaller body's centre, and each
//! takes the masses in the same order — the body whose sphere this is, then the primary
//! it orbits.

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

/// Hill sphere for a given separation from the primary.
///
/// `separation` is a distance, not an orbit: whatever eccentricity the body's orbit has is
/// already expressed in it. Pass the instantaneous distance for the live radius, or use
/// [`hill_at_periapsis`] for the conservative static one — do not pass a periapsis
/// distance *and* scale by `(1 - e)` again.
pub fn hill_at_separation(separation: f64, body_mass: f64, primary_mass: f64) -> f64 {
    let frac = body_mass / (3.0 * (body_mass + primary_mass));
    separation * f64::cbrt(frac)
}

/// Hill sphere at the orbit's periapsis: the smallest it gets, and so the radius to use
/// when one number has to hold for the whole orbit.
pub fn hill_at_periapsis(
    semi_major_axis: f64,
    eccentricity: f64,
    body_mass: f64,
    primary_mass: f64,
) -> f64 {
    hill_at_separation(semi_major_axis * (1.0 - eccentricity), body_mass, primary_mass)
}

/// Bondi accretion radius of a black hole.
pub fn black_hole(gravitational_constant: f64, mass: f64, velocity_dispersion: f64) -> f64 {
    (gravitational_constant * mass) / (velocity_dispersion * velocity_dispersion)
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: f64 = 6.6743015e-11;
    const M_SUN: f64 = 1.98892e30;
    const M_EARTH: f64 = 5.97219e24;
    const AU: f64 = 1.495978707e11;

    /// Earth's Hill radius is ~1.5e9 m and its Laplace SOI ~9.25e8 m; the Hill sphere is
    /// the larger of the two, which is the whole reason both are offered.
    #[test]
    fn earth_spheres_match_the_textbook_figures() {
        let hill = hill_at_separation(AU, M_EARTH, M_SUN);
        let soi = laplace(AU, M_EARTH, M_SUN);
        assert!((hill - 1.496e9).abs() < 1.0e7, "Hill radius {hill:e}");
        assert!((soi - 9.25e8).abs() < 1.0e7, "Laplace SOI {soi:e}");
        assert!(hill > soi);
    }

    /// The periapsis form is the separation form evaluated at periapsis, and nothing more.
    /// It exists because applying `(1 - e)` twice is the easy mistake here.
    #[test]
    fn periapsis_form_is_the_separation_form_at_periapsis() {
        let (a, e) = (AU, 0.4);
        let at_periapsis = hill_at_periapsis(a, e, M_EARTH, M_SUN);
        let by_hand = hill_at_separation(a * (1.0 - e), M_EARTH, M_SUN);
        assert!((at_periapsis - by_hand).abs() < 1.0);
        assert!(at_periapsis < hill_at_separation(a, M_EARTH, M_SUN));
    }

    /// The angled form is smallest along the primary line and largest across it, and the
    /// undirected [`laplace`] sits between.
    #[test]
    fn angled_laplace_brackets_the_plain_one() {
        let plain = laplace(AU, M_EARTH, M_SUN);
        let along = laplace_angled(AU, M_EARTH, M_SUN, 0.0);
        let across = laplace_angled(AU, M_EARTH, M_SUN, std::f64::consts::FRAC_PI_2);
        assert!(along < plain && plain <= across, "{along:e} {plain:e} {across:e}");
        assert!(laplace_integrated(AU, M_EARTH, M_SUN) < plain);
    }

    /// Every radius scales linearly with the separation, so a sphere is a fixed fraction
    /// of the orbit regardless of its size.
    #[test]
    fn radii_scale_linearly_with_separation() {
        for f in [0.5, 2.0, 10.0] {
            let scaled = hill_at_separation(AU * f, M_EARTH, M_SUN);
            let base = hill_at_separation(AU, M_EARTH, M_SUN);
            assert!((scaled / base - f).abs() < 1e-9);
            let scaled = laplace(AU * f, M_EARTH, M_SUN);
            let base = laplace(AU, M_EARTH, M_SUN);
            assert!((scaled / base - f).abs() < 1e-9);
        }
    }

    #[test]
    fn bondi_radius_falls_off_with_dispersion() {
        let m = 4.3e6 * M_SUN; // Sgr A*
        let near = black_hole(G, m, 1.0e5);
        let far = black_hole(G, m, 2.0e5);
        assert!((near / far - 4.0).abs() < 1e-9, "inverse square in dispersion");
    }
}
