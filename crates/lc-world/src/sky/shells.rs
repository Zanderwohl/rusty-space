//! Each system's shell: the sphere that bounds it, inside which a position belongs to the star.
//! See `lightcone/docs/03-world-model.md`, "Sizing".
//!
//! A shell's nominal radius scales with the cube root of the star's mass, as a tidal radius does,
//! and is [`SOLAR_SHELL_LY`] for one solar mass. Where two nominal shells would overlap, both
//! shrink in proportion to their nominal radii until they touch, which for equal masses is half
//! the separation. A published galaxy stores each radius, worked out at publish against every
//! neighbor across chunk faces, because a loader may hold only some chunks; a catalog held whole
//! (HYG, authored stars) assigns them at load. Either way the shard, the client and Galaxy Forge
//! read the same numbers.

use glam::DVec3;

use super::CatalogStar;

/// A solar-mass star's nominal shell. An Oort cloud reaches about a hundred thousand AU, which is
/// 1.6 ly. The Sun's pull there, 5.8e-13 m/s², is about seven times the galactic tide, which
/// matches it near 3.0 ly.
pub const SOLAR_SHELL_LY: f64 = 1.6;

/// The masses a nominal shell is sized from, M☉: a record's range.
const MASS_SOLAR: (f64, f64) = (0.08, 150.0);

/// No shell is larger: the nominal radius of the heaviest star, 8.501 ly, rounded up. What a
/// search for the star holding a position has to reach.
pub const MAX_SHELL_LY: f64 = 8.51;

/// The shell of a star of this mass with no neighbor near enough to clamp it, ly.
pub fn nominal_ly(mass_solar: f64) -> f64 {
    // `libm`, so the shard and every client compute the same bits.
    SOLAR_SHELL_LY * libm::cbrt(mass_solar.clamp(MASS_SOLAR.0, MASS_SOLAR.1))
}

/// Each star's shell radius, ly, given every star's position and mass, in the same order.
pub fn radii(stars: &[(DVec3, f64)]) -> Vec<f64> {
    let nominal: Vec<f64> = stars.iter().map(|(_, m)| nominal_ly(*m)).collect();
    // Cells two maximal shells wide: any pair that can overlap is in neighboring cells.
    let side = 2.0 * MAX_SHELL_LY;
    let cell = |p: DVec3| {
        let c = (p / side).floor();
        [c.x as i64, c.y as i64, c.z as i64]
    };
    let mut cells: std::collections::HashMap<[i64; 3], Vec<usize>> = std::collections::HashMap::new();
    for (i, (p, _)) in stars.iter().enumerate() {
        cells.entry(cell(*p)).or_default().push(i);
    }
    stars
        .iter()
        .enumerate()
        .map(|(i, (p, _))| {
            let [x, y, z] = cell(*p);
            let mut r = nominal[i];
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(near) = cells.get(&[x + dx, y + dy, z + dz]) else { continue };
                        for &j in near.iter().filter(|&&j| j != i) {
                            let d = p.distance(stars[j].0);
                            let reach = nominal[i] + nominal[j];
                            if d < reach {
                                r = r.min(nominal[i] * d / reach);
                            }
                        }
                    }
                }
            }
            r
        })
        .collect()
}

/// Sets every star's [`CatalogStar::shell_ly`] against the others in `stars`.
pub fn assign(stars: &mut [CatalogStar]) {
    let at: Vec<(DVec3, f64)> = stars.iter().map(|s| (s.position_ly, s.mass_solar)).collect();
    for (s, r) in stars.iter_mut().zip(radii(&at)) {
        s.shell_ly = r;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heaviest_shell_fits_under_the_maximum_and_the_sun_keeps_its_own() {
        assert!(nominal_ly(MASS_SOLAR.1) <= MAX_SHELL_LY && nominal_ly(1e6) <= MAX_SHELL_LY);
        assert_eq!(nominal_ly(1.0), SOLAR_SHELL_LY);
    }

    #[test]
    fn overlapping_shells_shrink_in_proportion_until_they_touch() {
        // Two Suns 2 ly apart share it evenly; a lone star keeps its nominal shell.
        let r = radii(&[(DVec3::ZERO, 1.0), (DVec3::X * 2.0, 1.0), (DVec3::X * 100.0, 1.0)]);
        assert_eq!((r[0], r[1], r[2]), (1.0, 1.0, SOLAR_SHELL_LY));
        // An eight-solar-mass star, nominally twice the Sun's shell, keeps two thirds of the gap.
        let r = radii(&[(DVec3::ZERO, 8.0), (DVec3::X * 3.0, 1.0)]);
        assert!((r[0] - 2.0).abs() < 1e-12 && (r[1] - 1.0).abs() < 1e-12, "{r:?}");
        assert!(r[0] + r[1] <= 3.0 + 1e-12);
    }
}
