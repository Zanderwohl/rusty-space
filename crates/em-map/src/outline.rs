//! The wire shape of a belt, a ring system or a cloud.
//!
//! One definition, two callers: the reticle draws it over the sky when a swarm is selected, and
//! the map draws it as geometry. It traces the **edge of the material** — the inclination
//! sweeps every element through the same latitude band, so the cross-section is an annular
//! sector and not the ellipse the old surface shader implied, whose corners were several
//! degrees of latitude from where the material actually stops.
//!
//! It degenerates correctly. An isotropic cloud reaches a right angle, its cross-sections close
//! into full meridians, and the whole thing reads as the shell it is.

use glam::DVec3;

/// How many points a curve is sampled at.
///
/// Sixty-four is a fifth of a degree against a true circle at the widest, far below a pixel at
/// any distance one is drawn at. Divisible by four, which the cross-section's legs rely on.
pub const SAMPLES: usize = 64;

/// How many cross-sections are drawn around the torus.
///
/// Four reads as a donut and no more: the two edge circles carry the shape, and these say which
/// way round it is thick.
pub const CROSS_SECTIONS: usize = 4;

/// How far a population reaches, in whatever unit the caller is working in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extent {
    pub inner: f64,
    pub outer: f64,
    /// Half-thickness of the tube as an angle from the plane. A right angle is a shell.
    pub half_angle_rad: f64,
}

/// The curves of the outline, about `center`, in the plane whose normal is `pole`.
///
/// Each curve is a polyline to be drawn as-is. The first two are the inner and outer edges;
/// the rest are cross-sections.
pub fn torus(center: DVec3, pole: DVec3, extent: Extent) -> Vec<Vec<DVec3>> {
    let pole = pole.normalize_or(DVec3::Z);
    let (u, v) = basis(pole);

    let ring = |radius: f64| -> Vec<DVec3> {
        (0..=SAMPLES)
            .map(|i| {
                let theta = std::f64::consts::TAU * i as f64 / SAMPLES as f64;
                center + (u * theta.cos() + v * theta.sin()) * radius
            })
            .collect()
    };

    let mut out = vec![ring(extent.inner), ring(extent.outer)];

    // The cross-section, in a plane containing the pole: out along the far edge, in across the
    // top, back along the near edge, out across the bottom. Four legs of a closed loop.
    let arc = SAMPLES / 4;
    let half = extent.half_angle_rad;
    for k in 0..CROSS_SECTIONS {
        let phi = std::f64::consts::TAU * k as f64 / CROSS_SECTIONS as f64;
        let outward = u * phi.cos() + v * phi.sin();
        let at = |radius: f64, latitude: f64| {
            center + (outward * latitude.cos() + pole * latitude.sin()) * radius
        };
        let leg = |steps: usize, f: &dyn Fn(f64) -> DVec3| {
            (0..steps).map(|i| f(i as f64 / steps as f64)).collect::<Vec<_>>()
        };
        let mut curve = Vec::with_capacity(SAMPLES + 1);
        curve.extend(leg(arc, &|t| at(extent.outer, -half + 2.0 * half * t)));
        curve.extend(leg(arc, &|t| at(extent.outer + (extent.inner - extent.outer) * t, half)));
        curve.extend(leg(arc, &|t| at(extent.inner, half - 2.0 * half * t)));
        curve.extend(leg(arc, &|t| at(extent.inner + (extent.outer - extent.inner) * t, -half)));
        curve.push(curve[0]);
        out.push(curve);
    }
    out
}

/// Two axes spanning the plane whose normal is `pole`.
pub fn basis(pole: DVec3) -> (DVec3, DVec3) {
    let n = pole.normalize_or(DVec3::Z);
    // Any fixed vector not parallel to the pole. Z first because most poles are near it and the
    // cross product is then largest.
    let seed = if n.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let u = seed.cross(n).normalize_or(DVec3::X);
    (u, n.cross(u))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn belt() -> Extent {
        Extent { inner: 2.0, outer: 3.0, half_angle_rad: 0.2 }
    }

    /// The two edge circles are circles, in the plane, at the radii asked for.
    #[test]
    fn the_edges_are_the_radii_they_were_given() {
        for pole in [DVec3::Z, DVec3::new(1.0, 2.0, 3.0).normalize()] {
            let curves = torus(DVec3::ZERO, pole, belt());
            for (curve, radius) in [(&curves[0], 2.0), (&curves[1], 3.0)] {
                for p in curve {
                    assert!((p.length() - radius).abs() < 1e-9, "{p:?} is off radius {radius}");
                    assert!(p.dot(pole).abs() < 1e-9, "{p:?} is out of the plane");
                }
            }
        }
    }

    /// Nothing reaches past the outer edge or inside the inner one, at any latitude. A
    /// cross-section that bulged would be claiming material where there is none.
    #[test]
    fn everything_lies_between_the_two_edges() {
        let curves = torus(DVec3::ZERO, DVec3::Z, belt());
        for p in curves.iter().flatten() {
            let r = p.length();
            assert!(r >= 2.0 - 1e-9 && r <= 3.0 + 1e-9, "{r} is outside [2, 3]");
        }
    }

    /// And the tube is as thick as the inclination says: the highest point sits at the outer
    /// radius times the sine of the half-angle. A shape built from the wrong latitude reads as
    /// a belt that misses its own material.
    #[test]
    fn the_tube_is_as_thick_as_the_inclination() {
        let extent = belt();
        let curves = torus(DVec3::ZERO, DVec3::Z, extent);
        let highest = curves.iter().flatten().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
        let want = extent.outer * extent.half_angle_rad.sin();
        assert!((highest - want).abs() < 1e-9, "{highest} against {want}");
    }

    /// An isotropic cloud is a shell: at a right angle the cross-sections close into meridians
    /// and reach the pole.
    #[test]
    fn an_isotropic_cloud_becomes_a_shell() {
        let shell = Extent { half_angle_rad: std::f64::consts::FRAC_PI_2, ..belt() };
        let curves = torus(DVec3::ZERO, DVec3::Z, shell);
        let highest = curves.iter().flatten().map(|p| p.z).fold(f64::NEG_INFINITY, f64::max);
        assert!((highest - shell.outer).abs() < 1e-9, "a shell should reach its own radius");
    }

    /// Centered where it was put, whatever the pole.
    #[test]
    fn it_sits_where_it_was_centered() {
        let at = DVec3::new(-4.0, 1.5, 9.0);
        let curves = torus(at, DVec3::Y, belt());
        // Less the repeated closing point, which would otherwise pull the mean toward it.
        let loop_ = &curves[1][..curves[1].len() - 1];
        let mean: DVec3 = loop_.iter().sum::<DVec3>() / loop_.len() as f64;
        assert!((mean - at).length() < 1e-9, "{mean:?} against {at:?}");
    }

    /// Every curve is closed, so a tube built from one meets itself.
    #[test]
    fn every_curve_closes() {
        for curve in torus(DVec3::ZERO, DVec3::Z, belt()) {
            let (first, last) = (curve[0], curve[curve.len() - 1]);
            assert!((first - last).length() < 1e-9, "{first:?} to {last:?}");
        }
    }

    #[test]
    fn a_basis_spans_the_plane() {
        for pole in [DVec3::Z, DVec3::X, DVec3::new(0.3, -0.7, 0.2).normalize()] {
            let (u, v) = basis(pole);
            let n = pole.normalize();
            assert!(u.dot(n).abs() < 1e-12 && v.dot(n).abs() < 1e-12);
            assert!((u.cross(v).dot(n) - 1.0).abs() < 1e-12, "left-handed");
        }
    }
}
