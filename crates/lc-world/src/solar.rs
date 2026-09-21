//! Starlight collected by a hull. See `lightcone/docs/20-solar-power.md`.
//!
//! Collectors over a whole convex hull deliver `η · flux · ∫ max(0, n·ŝ) dA`, and for a convex body
//! that integral is the shadow the hull casts along `ŝ`. A ship collects broadside and only when it
//! is not under way. Income is held constant over segments of coordinate time — see [`segment_end`]
//! — because distance from the star is not a closed form anyone can integrate cheaply.

use glam::DVec3;

use crate::craft::{BEAM_PER_LENGTH, HEIGHT_PER_LENGTH};

/// The grid income segments are cut on: one game day, about 2.4 real seconds at the design rate.
///
/// Absolute, so server and client cut the same segments from the same state without being told.
pub const SOLAR_STEP_S: f64 = 86_400.0;

/// The most boundaries one settlement walks. A longer leap — a checkpoint caught up after downtime
/// — takes coarser segments rather than more of them.
pub const SOLAR_MAX_SEGMENTS: f64 = 4_096.0;

/// Sunlight at 1 AU, W/m². Only the balance anchor uses it: at run time a craft reads its own
/// star's luminosity, since luminosity is not `const`.
pub const SOLAR_CONSTANT_W_M2: f64 = 1_361.0;

/// The hull's shadow along `to_star`, m², for a hull of `length_m`.
///
/// `to_star` in the hull's own axes: x along the nose, y across the beam, z through the height.
/// Needs no normalisation of the semi-axes' product form: `π √((bc sₓ)² + (ac s_y)² + (ab s_z)²)`
/// for a unit `s`.
pub fn silhouette_m2(length_m: f64, to_star: DVec3) -> f64 {
    let s = to_star.normalize_or_zero();
    let (a, b, c) = semi_axes(length_m);
    std::f64::consts::PI
        * ((b * c * s.x).powi(2) + (a * c * s.y).powi(2) + (a * b * s.z).powi(2)).sqrt()
}

/// The largest shadow a hull casts: the star along its short axis.
pub fn broadside_m2(length_m: f64) -> f64 {
    silhouette_m2(length_m, DVec3::Z)
}

fn semi_axes(length_m: f64) -> (f64, f64, f64) {
    let a = 0.5 * length_m;
    (a, a * BEAM_PER_LENGTH, a * HEIGHT_PER_LENGTH)
}

/// Starlight at `distance_m` from a star putting out `luminosity_w`, W/m².
pub fn flux_w_m2(luminosity_w: f64, distance_m: f64) -> f64 {
    if distance_m <= 0.0 {
        return 0.0;
    }
    luminosity_w / (4.0 * std::f64::consts::PI * distance_m * distance_m)
}

/// What a hull collects broadside, W: the balance's efficiency and gain times flux times shadow.
pub fn power_w(
    balance: &crate::fitting::Balance,
    length_m: f64,
    luminosity_w: f64,
    distance_m: f64,
) -> f64 {
    balance.solar_gain
        * balance.solar_efficiency
        * flux_w_m2(luminosity_w, distance_m)
        * broadside_m2(length_m)
}

/// Where the segment that starts at `from_s` ends: the next grid boundary strictly after it.
pub fn segment_end(from_s: f64) -> f64 {
    ((from_s / SOLAR_STEP_S).floor() + 1.0) * SOLAR_STEP_S
}

/// The grid a settlement over `from_s .. to_s` walks: one step, or a whole multiple of it when that
/// many steps would be more than [`SOLAR_MAX_SEGMENTS`].
pub fn step_for(from_s: f64, to_s: f64) -> f64 {
    let steps = ((to_s - from_s) / SOLAR_STEP_S).max(0.0);
    SOLAR_STEP_S * (steps / SOLAR_MAX_SEGMENTS).ceil().max(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fitting::{Balance, C2, Loadout};
    use crate::flight::JULIAN_YEAR_S;
    use crate::star::Star;
    use crate::system::UNIT_M;

    /// **Checked against the integral it replaces**, not against itself: tessellate the ellipsoid
    /// and sum `max(0, n·ŝ) dA` over its faces.
    #[test]
    fn the_silhouette_is_the_integral_of_the_lit_surface() {
        let length = 500.0;
        let (a, b, c) = semi_axes(length);
        let point =
            |u: f64, v: f64| DVec3::new(a * u.cos() * v.sin(), b * u.sin() * v.sin(), c * v.cos());
        let (nu, nv) = (720, 360);
        let lit = |s: DVec3| {
            let mut sum = 0.0;
            for i in 0..nu {
                for j in 0..nv {
                    let (u0, u1) = (
                        i as f64 / nu as f64 * std::f64::consts::TAU,
                        (i + 1) as f64 / nu as f64 * std::f64::consts::TAU,
                    );
                    let (v0, v1) = (
                        j as f64 / nv as f64 * std::f64::consts::PI,
                        (j + 1) as f64 / nv as f64 * std::f64::consts::PI,
                    );
                    let (p00, p10, p01, p11) =
                        (point(u0, v0), point(u1, v0), point(u0, v1), point(u1, v1));
                    // Two triangles, each an area-weighted outward normal.
                    for (p, q, r) in [(p00, p01, p11), (p00, p11, p10)] {
                        let area_normal = 0.5 * (q - p).cross(r - p);
                        sum += area_normal.dot(s).max(0.0);
                    }
                }
            }
            sum
        };
        for s in [
            DVec3::X,
            DVec3::Y,
            DVec3::Z,
            DVec3::new(1.0, 2.0, 3.0).normalize(),
            DVec3::new(-0.3, 0.1, 0.9).normalize(),
        ] {
            let (numeric, closed) = (lit(s), silhouette_m2(length, s));
            assert!(
                (numeric / closed - 1.0).abs() < 1.0e-3,
                "{s}: {numeric} vs {closed}"
            );
        }
    }

    /// Cauchy: averaged over every direction, a convex body's shadow is a quarter of its surface.
    #[test]
    fn the_average_shadow_is_a_quarter_of_the_surface() {
        let length = 1_000.0;
        let (a, b, c) = semi_axes(length);
        let p = 1.6075_f64;
        let surface = 4.0
            * std::f64::consts::PI
            * (((a * b).powf(p) + (a * c).powf(p) + (b * c).powf(p)) / 3.0).powf(1.0 / p);
        // A Fibonacci sphere, which covers directions evenly without randomness.
        let n = 20_000;
        let golden = std::f64::consts::PI * (3.0 - 5.0_f64.sqrt());
        let mean: f64 = (0..n)
            .map(|k| {
                let z = 1.0 - 2.0 * (k as f64 + 0.5) / n as f64;
                let r = (1.0 - z * z).sqrt();
                let t = golden * k as f64;
                silhouette_m2(length, DVec3::new(r * t.cos(), r * t.sin(), z))
            })
            .sum::<f64>()
            / n as f64;
        // Thomsen's surface formula is good to about a percent.
        assert!(
            (mean / (surface / 4.0) - 1.0).abs() < 0.015,
            "{mean} vs {}",
            surface / 4.0
        );
        assert!((broadside_m2(length) / silhouette_m2(length, DVec3::X) - 5.0).abs() < 1.0e-9);
    }

    #[test]
    fn sunlight_at_one_au_is_the_solar_constant() {
        let flux = flux_w_m2(Star::SOL.luminosity(), UNIT_M);
        assert!((flux / SOLAR_CONSTANT_W_M2 - 1.0).abs() < 0.01, "{flux}");
    }

    /// The anchor the gain was derived from, reached the other way round: at 0.1 AU the starting
    /// ship's collection, less its drain, fills its storage in one Julian year.
    #[test]
    fn the_starting_ship_fills_in_a_year_at_a_tenth_of_an_au() {
        let b = Balance::DEFAULT;
        let start = Loadout::STARTING;
        let net = power_w(
            &b,
            500.0,
            SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * UNIT_M * UNIT_M,
            0.1 * UNIT_M,
        ) - b.drain_w(&start);
        let years = b.capacity_j(&start) / net / JULIAN_YEAR_S;
        assert!((years - 1.0).abs() < 1.0e-9, "{years}");
        let _ = C2;
    }

    /// Break-even — collection equal to drain — for the doc's three hulls, with its scaled loadouts.
    #[test]
    fn break_even_moves_inward_as_hulls_grow() {
        let b = Balance::DEFAULT;
        let sol = SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * UNIT_M * UNIT_M;
        for (length, expected_au) in [(500.0, 3.87), (1_000.0, 2.74), (5_000.0, 1.23)] {
            let slots = (b.length_m(20) / length).powi(-3) * 20.0;
            let drain = 0.1 * slots * b.living_drain_w;
            let at_one_au = power_w(&b, length, sol, UNIT_M);
            let au = (at_one_au / drain).sqrt();
            assert!(
                (au - expected_au).abs() < 0.01,
                "{length} m breaks even at {au} AU"
            );
        }
    }

    #[test]
    fn segments_end_on_the_grid_and_long_leaps_coarsen() {
        assert_eq!(segment_end(0.0), SOLAR_STEP_S);
        assert_eq!(segment_end(SOLAR_STEP_S), 2.0 * SOLAR_STEP_S);
        assert_eq!(segment_end(SOLAR_STEP_S - 1.0), SOLAR_STEP_S);
        assert_eq!(step_for(0.0, 10.0 * SOLAR_STEP_S), SOLAR_STEP_S);
        let leap = step_for(0.0, 100_000.0 * SOLAR_STEP_S);
        assert!(100_000.0 * SOLAR_STEP_S / leap <= SOLAR_MAX_SEGMENTS);
        assert_eq!(leap % SOLAR_STEP_S, 0.0);
    }
}
