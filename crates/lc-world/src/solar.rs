//! Starlight collected by a hull. See `lightcone/docs/20-solar-power.md`.
//!
//! Collectors over a whole hull deliver `η · flux · ∫ max(0, n·ŝ) dA`, and for a convex body that
//! integral is the shadow the hull casts along `ŝ`: the form's shadow table, which also counts a
//! stack of plates shading itself. A ship collects only when it is not under way. Income is held
//! constant over segments of coordinate time — see [`segment_end`] — because distance from the star
//! is not a closed form anyone can integrate cheaply.
//!
//! **Every hull rolls its broadside toward its star**, under way or not, so the broadside's part
//! across the nose faces the star. What is left to say where the star is in the ship's frame is the
//! one angle between it and the nose.

use glam::DVec3;
use lc_proto::form::Geometry;

use crate::form::grid::{Shadow, SHADOW_DIRECTIONS};

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

/// Ship frame: the direction across the nose that `roll_rad` carries +z onto.
fn across_nose(roll_rad: f64) -> DVec3 {
    let (s, c) = libm::sincos(roll_rad);
    DVec3::new(0.0, -s, c)
}

/// The broadside signed so its part across the nose is where the roll carries +z, and that roll in
/// `[−π/2, π/2)`. The roll is the broadside's own azimuth about the nose, not the grid's
/// `broadside_roll_rad`: that one is best for a nose held square to the star, and a broadside that
/// leans off square can lie at another azimuth altogether. A broadside along the nose has no
/// azimuth, and takes the grid's.
fn presented(geometry: &Geometry) -> (DVec3, f64) {
    let b = DVec3::from_array(geometry.broadside);
    if b.y.hypot(b.z) < 1.0e-6 {
        return (b, geometry.broadside_roll_rad);
    }
    let roll = libm::atan2(-b.y, b.z);
    let half = std::f64::consts::FRAC_PI_2;
    if roll >= half {
        (-b, roll - std::f64::consts::PI)
    } else if roll < -half {
        (-b, roll + std::f64::consts::PI)
    } else {
        (b, roll)
    }
}

/// Radians, right-handed about the nose: how far a hull rolls from dorsal-to-the-star.
pub fn roll_rad(geometry: &Geometry) -> f64 {
    presented(geometry).1
}

/// The star in the ship's own frame, for a nose at `cos_nose` to it and the hull at its roll.
pub fn toward_star(geometry: &Geometry, cos_nose: f64) -> DVec3 {
    let c = cos_nose.clamp(-1.0, 1.0);
    DVec3::X * c + across_nose(roll_rad(geometry)) * (1.0 - c * c).sqrt()
}

/// `nose · ŝ` with the broadside turned to the star.
pub fn idle_cos(geometry: &Geometry) -> f64 {
    presented(geometry).0.x.clamp(-1.0, 1.0)
}

/// The hull's shadow toward its star, m², with its nose at `cos_nose` to it.
pub fn shadow_m2(geometry: &Geometry, cos_nose: f64) -> f64 {
    debug_assert_eq!(geometry.shadow_m2.len(), SHADOW_DIRECTIONS, "a geometry the grid did not measure");
    let Ok(m2) = <[f64; SHADOW_DIRECTIONS]>::try_from(geometry.shadow_m2.as_slice()) else { return 0.0 };
    Shadow::from_m2(m2).along(toward_star(geometry, cos_nose))
}

/// The nose nearest `attitude` at `cos_nose` to `to_star`: `attitude`'s own bearing about the star,
/// leaned to that angle. A nose along the star has no bearing about it, and takes the one toward
/// ecliptic north.
pub fn idle_nose(attitude: DVec3, to_star: DVec3, cos_nose: f64) -> DVec3 {
    let s = to_star.normalize_or_zero();
    let across = attitude - s * attitude.dot(s);
    let across = if across.length_squared() > 1.0e-12 {
        across.normalize()
    } else {
        let reference = if s.z.abs() > 0.999 { DVec3::X } else { DVec3::Z };
        (reference - s * reference.dot(s)).normalize_or_zero()
    };
    let c = cos_nose.clamp(-1.0, 1.0);
    (s * c + across * (1.0 - c * c).sqrt()).normalize_or_zero()
}

/// Starlight at `distance_m` from a star putting out `luminosity_w`, W/m².
pub fn flux_w_m2(luminosity_w: f64, distance_m: f64) -> f64 {
    if distance_m <= 0.0 {
        return 0.0;
    }
    luminosity_w / (4.0 * std::f64::consts::PI * distance_m * distance_m)
}

/// What a shadow of `shadow_m2` collects, W: the balance's efficiency and gain times flux times
/// shadow. Server and client both price a segment through here.
pub fn power_w(balance: &crate::fitting::Balance, shadow_m2: f64, luminosity_w: f64, distance_m: f64) -> f64 {
    balance.solar_gain * balance.conversion_efficiency * flux_w_m2(luminosity_w, distance_m) * shadow_m2
}

/// The shadow along `to_star` of the ovoid an unformed craft is drawn as, m², for tests that state a
/// hull by its length. `to_star` in the ovoid's axes: x along the nose, y across the beam, z through
/// the height.
#[cfg(test)]
pub(crate) fn silhouette_m2(length_m: f64, to_star: DVec3) -> f64 {
    let s = to_star.normalize_or_zero();
    let (a, b, c) = semi_axes(length_m);
    std::f64::consts::PI * ((b * c * s.x).powi(2) + (a * c * s.y).powi(2) + (a * b * s.z).powi(2)).sqrt()
}

/// The ovoid's largest shadow: the star along its short axis.
#[cfg(test)]
pub(crate) fn broadside_m2(length_m: f64) -> f64 {
    silhouette_m2(length_m, DVec3::Z)
}

#[cfg(test)]
fn semi_axes(length_m: f64) -> (f64, f64, f64) {
    let a = 0.5 * length_m;
    (a, a * crate::craft::BEAM_PER_LENGTH, a * crate::craft::HEIGHT_PER_LENGTH)
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
    use crate::fitting::{Balance, SOLAR_ANCHOR_AU, STARTING_BROADSIDE_M2};
    use crate::flight::JULIAN_YEAR_S;
    use crate::form::capacity::Capacities;
    use crate::form::grid::FormGrid;
    use crate::form::presets::{named, Builtin};
    use crate::form::{Form, Kind, Mount, Part, PartId, Placement, Primitive};
    use crate::star::Star;
    use glam::DVec2;
    use crate::system::UNIT_M;

    /// **Checked against the integral it replaces**, not against itself: tessellate the ellipsoid
    /// and sum `max(0, n·ŝ) dA` over its faces.
    #[test]
    fn the_silhouette_is_the_integral_of_the_lit_surface() {
        let length = 500.0;
        let (a, b, c) = semi_axes(length);
        let point = |u: f64, v: f64| {
            DVec3::new(a * u.cos() * v.sin(), b * u.sin() * v.sin(), c * v.cos())
        };
        let (nu, nv) = (720, 360);
        let lit = |s: DVec3| {
            let mut sum = 0.0;
            for i in 0..nu {
                for j in 0..nv {
                    let (u0, u1) = (i as f64 / nu as f64 * std::f64::consts::TAU, (i + 1) as f64 / nu as f64 * std::f64::consts::TAU);
                    let (v0, v1) = (j as f64 / nv as f64 * std::f64::consts::PI, (j + 1) as f64 / nv as f64 * std::f64::consts::PI);
                    let (p00, p10, p01, p11) = (point(u0, v0), point(u1, v0), point(u0, v1), point(u1, v1));
                    // Two triangles, each an area-weighted outward normal.
                    for (p, q, r) in [(p00, p01, p11), (p00, p11, p10)] {
                        let area_normal = 0.5 * (q - p).cross(r - p);
                        sum += area_normal.dot(s).max(0.0);
                    }
                }
            }
            sum
        };
        for s in [DVec3::X, DVec3::Y, DVec3::Z, DVec3::new(1.0, 2.0, 3.0).normalize(), DVec3::new(-0.3, 0.1, 0.9).normalize()] {
            let (numeric, closed) = (lit(s), silhouette_m2(length, s));
            assert!((numeric / closed - 1.0).abs() < 1.0e-3, "{s}: {numeric} vs {closed}");
        }
    }

    /// Cauchy: averaged over every direction, a convex body's shadow is a quarter of its surface.
    #[test]
    fn the_average_shadow_is_a_quarter_of_the_surface() {
        let length = 1_000.0;
        let (a, b, c) = semi_axes(length);
        let p = 1.6075_f64;
        let surface = 4.0 * std::f64::consts::PI
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
        assert!((mean / (surface / 4.0) - 1.0).abs() < 0.015, "{mean} vs {}", surface / 4.0);
        assert!((broadside_m2(length) / silhouette_m2(length, DVec3::X) - 5.0).abs() < 1.0e-9);
    }

    #[test]
    fn sunlight_at_one_au_is_the_solar_constant() {
        let flux = flux_w_m2(Star::SOL.luminosity(), UNIT_M);
        assert!((flux / SOLAR_CONSTANT_W_M2 - 1.0).abs() < 0.01, "{flux}");
    }

    fn geometry(form: &Form) -> Geometry {
        FormGrid::new(form, &Balance::DEFAULT).unwrap().geometry(1.0)
    }

    fn sol_w() -> f64 {
        SOLAR_CONSTANT_W_M2 * 4.0 * std::f64::consts::PI * UNIT_M * UNIT_M
    }

    /// Years to fill from empty, net of the drain, collecting over `shadow_m2` at `d_au`.
    fn years_to_fill(form: &Form, shadow_m2: f64, d_au: f64) -> f64 {
        let b = Balance::DEFAULT;
        let caps = Capacities::of(form, &b);
        caps.storage_j / (power_w(&b, shadow_m2, sol_w(), d_au * UNIT_M) - caps.drain_w) / JULIAN_YEAR_S
    }

    #[test]
    fn the_starting_broadside_is_pinned() {
        let solved = FormGrid::new(&Form::starting(), &Balance::DEFAULT).unwrap().broadside_m2();
        assert!((solved / STARTING_BROADSIDE_M2 - 1.0).abs() < 1.0e-9, "pinned {STARTING_BROADSIDE_M2}, the starting form solves to {solved:?}");
    }

    /// 20's anchor, reached the other way round: at 0.1 AU the starting ship fills in a year on the
    /// table it collects on, idle, at the broadside itself, and within a couple of degrees of it.
    /// The table is interpolated, so a direction off its vertices is the one worth checking.
    #[test]
    fn the_starting_ship_fills_in_a_year_at_a_tenth_of_an_au() {
        let form = Form::starting();
        let g = geometry(&form);
        let idle = years_to_fill(&form, shadow_m2(&g, idle_cos(&g)), SOLAR_ANCHOR_AU);
        assert!((idle - 1.0).abs() < 0.01, "idle: {idle} years");
        let table = Shadow::from_m2(g.shadow_m2.clone().try_into().unwrap());
        let broadside = DVec3::from_array(g.broadside);
        let (e1, e2) = (broadside.cross(DVec3::X).normalize(), broadside.cross(DVec3::Y).normalize());
        for off in [DVec3::ZERO, e1, -e1, e2, e2 + e1, -e2 - 0.5 * e1] {
            let s = (broadside + off * 2.0_f64.to_radians()).normalize();
            let years = years_to_fill(&form, table.along(s), SOLAR_ANCHOR_AU);
            assert!((years - 1.0).abs() < 0.01, "{s}: {years} years");
        }
        let exact = years_to_fill(&form, STARTING_BROADSIDE_M2, SOLAR_ANCHOR_AU);
        assert!((exact - 1.0).abs() < 1.0e-9, "{exact}");
    }

    /// A slab pitched nose-up, beamier than it is tall: its largest shadow leans off square, and
    /// across the nose its beam beats its height, so the grid's roll turns it a quarter turn from
    /// the roll that presents that shadow.
    fn pitched_slab() -> Form {
        let b = Balance::DEFAULT;
        let placement = Placement { parent: PartId(0), mount: Mount::Enclosing, twist: 0.0, tilt: DVec2::new(0.65, 0.0), blend: 0.0, mirror: false };
        let slab = Primitive::Ellipsoid { axes: DVec3::new(6.0, 1.2, 1.0) };
        Form {
            parts: vec![
                Part::mind(PartId(0), b.min_part_m3),
                Part { id: PartId(1), kind: Kind::Storage, primitive: slab, volume_m3: 2.4e6, placement: Some(placement) },
            ],
        }
    }

    /// The idle attitude presents the largest shadow the table has, whichever way the form's
    /// broadside leans and rolls.
    #[test]
    fn idle_presents_the_broadside() {
        let g = geometry(&pitched_slab());
        assert!((roll_rad(&g) - g.broadside_roll_rad).abs() > 30.0_f64.to_radians(), "premise: the rolls differ");
        let mut all = vec![Form::starting(), pitched_slab()];
        all.extend(Builtin::ALL.map(Builtin::form));
        for form in all {
            let grid = FormGrid::new(&form, &Balance::DEFAULT).unwrap();
            let g = grid.geometry(1.0);
            let idle = shadow_m2(&g, idle_cos(&g));
            assert!((idle / grid.broadside_m2() - 1.0).abs() < 0.01, "{idle} against {}", grid.broadside_m2());
            assert!(toward_star(&g, idle_cos(&g)).dot(grid.broadside()).abs() > 0.999);
            let largest = g.shadow_m2.iter().copied().fold(0.0, f64::max);
            assert!(idle > 0.99 * largest, "{idle} against the table's {largest}");
        }
    }

    #[test]
    fn a_plate_collects_more_than_a_spindle_of_the_same_volume() {
        let (plate, spindle) = (Builtin::Plate.form(), Builtin::Spindle.form());
        let volume = |form: &Form| form.parts.iter().map(|p| p.volume_m3).sum::<f64>();
        assert!((volume(&plate) / volume(&spindle) - 1.0).abs() < 1.0e-9, "premise: the same volume");
        let collected = |form: &Form| {
            let g = geometry(form);
            shadow_m2(&g, idle_cos(&g))
        };
        let (flat, long) = (collected(&plate), collected(&spindle));
        assert!(flat > 1.4 * long, "the plate casts {flat}, the spindle {long}");
    }

    /// Break-even — collection equal to drain — for 20's three hulls: the starting form, twice and
    /// ten times as long.
    #[test]
    fn break_even_moves_inward_as_hulls_grow() {
        let b = Balance::DEFAULT;
        for (scale, expected_au) in [(1.0, 5.47), (2.0, 3.87), (10.0, 1.73)] {
            let form = named("default", scale).unwrap();
            let g = geometry(&form);
            let at_one_au = power_w(&b, shadow_m2(&g, idle_cos(&g)), sol_w(), UNIT_M);
            let au = (at_one_au / Capacities::of(&form, &b).drain_w).sqrt();
            assert!((au - expected_au).abs() < 0.01, "{scale}× breaks even at {au} AU");
        }
    }

    #[test]
    fn an_idle_nose_keeps_its_bearing_about_the_star() {
        let s = DVec3::X;
        assert_eq!(idle_nose(DVec3::new(1.0, 1.0, 0.0), s, 0.0), DVec3::Y);
        assert_eq!(idle_nose(DVec3::Z, s, 0.0), DVec3::Z);
        let head_on = idle_nose(-DVec3::X, s, 0.0);
        assert!(head_on.dot(s).abs() < 1.0e-12 && head_on.is_normalized(), "{head_on}");
        let leaning = idle_nose(DVec3::new(-1.0, 1.0, 0.0), s, 0.5);
        assert!((leaning.dot(s) - 0.5).abs() < 1.0e-12 && leaning.z == 0.0 && leaning.y > 0.0, "{leaning}");
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
