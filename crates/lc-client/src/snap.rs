//! The editor's preferred steps. The server never snaps: this is feel, so it is a table in the
//! client, and tuning it is a change to [`SNAPS`]. See `lightcone/docs/29-ship-form.md` §Snapping.
//!
//! Volumes and proportions go by ratios rather than fixed amounts, so a handle is as fine on a
//! 500 m ship as on a 50 km one.

use glam::DVec3;

/// ISO 3's R10 preferred numbers: ten steps a decade, each about 26% past the last.
pub const R10: [f64; 10] = [1.00, 1.25, 1.60, 2.00, 2.50, 3.15, 4.00, 5.00, 6.30, 8.00];

/// ISO 3's R40, about 6% apart.
pub const R40: [f64; 40] = [
    1.00, 1.06, 1.12, 1.18, 1.25, 1.32, 1.40, 1.50, 1.60, 1.70, 1.80, 1.90, 2.00, 2.12, 2.24, 2.36,
    2.50, 2.65, 2.80, 3.00, 3.15, 3.35, 3.55, 3.75, 4.00, 4.25, 4.50, 4.75, 5.00, 5.30, 5.60, 6.00,
    6.30, 6.70, 7.10, 7.50, 8.00, 8.50, 9.00, 9.50,
];

/// One set of steps: the default, or the one the modifier gives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Steps {
    /// Volume, and proportions as ratios, one decade of it.
    pub ladder: &'static [f64],
    /// Twist and tilt, radians.
    pub angle_rad: f64,
    /// Whether an anchor goes to the parent's axes and diagonals.
    pub anchor_on_axes: bool,
    /// Standoff, in the child's reaches.
    pub standoff: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snaps {
    pub coarse: Steps,
    /// With the modifier held.
    pub fine: Steps,
}

impl Snaps {
    pub fn steps(&self, fine: bool) -> &Steps {
        if fine { &self.fine } else { &self.coarse }
    }
}

pub const SNAPS: Snaps = Snaps {
    coarse: Steps { ladder: &R10, angle_rad: 15.0 * DEG, anchor_on_axes: true, standoff: 0.1 },
    fine: Steps { ladder: &R40, angle_rad: 5.0 * DEG, anchor_on_axes: false, standoff: 0.01 },
};

const DEG: f64 = std::f64::consts::PI / 180.0;

/// `value` on `ladder` repeated every decade from `anchor`, nearest in ratio. Positive values only.
pub fn on_ladder(value: f64, anchor: f64, ladder: &[f64]) -> f64 {
    if !(value > 0.0 && value.is_finite()) {
        return value;
    }
    let decades = (value / anchor).log10();
    let base = decades.floor();
    let within = decades - base;
    // The next decade's first step closes the ladder, so a value just under ten goes up to it.
    let nearest = ladder
        .iter()
        .map(|step| step.log10())
        .chain([1.0])
        .min_by(|a, b| (a - within).abs().total_cmp(&(b - within).abs()))
        .unwrap_or(0.0);
    anchor * 10f64.powf(base + nearest)
}

/// A volume on the ladder anchored at `min_part_m3`, and never below it.
pub fn volume(volume_m3: f64, min_part_m3: f64, fine: bool) -> f64 {
    on_ladder(volume_m3, min_part_m3, SNAPS.steps(fine).ladder).max(min_part_m3)
}

/// A proportion, as a ratio on the same series about one.
pub fn ratio(ratio: f64, fine: bool) -> f64 {
    on_ladder(ratio, 1.0, SNAPS.steps(fine).ladder)
}

/// Twist or tilt, radians.
pub fn angle(rad: f64, fine: bool) -> f64 {
    let step = SNAPS.steps(fine).angle_rad;
    (rad / step).round() * step
}

pub fn standoff(reaches: f64, fine: bool) -> f64 {
    let step = SNAPS.steps(fine).standoff;
    // Rounded again, so a tenth reads as 0.1 and not 0.30000000000000004 in a field.
    ((reaches / step).round() * step * 1.0e6).round() / 1.0e6
}

/// An anchor on the nearest of the parent's six axes, twelve edge diagonals and eight corner
/// diagonals, or unit and otherwise free.
pub fn anchor(direction: DVec3, fine: bool) -> DVec3 {
    let unit = direction.normalize_or(DVec3::X);
    if !SNAPS.steps(fine).anchor_on_axes {
        return unit;
    }
    let mut best = DVec3::X;
    let mut closest = f64::NEG_INFINITY;
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                let candidate = DVec3::new(x as f64, y as f64, z as f64);
                if candidate == DVec3::ZERO {
                    continue;
                }
                // Exact integers, so an anchor on an axis stays exactly on it.
                let cosine = unit.dot(candidate.normalize());
                if cosine > closest {
                    closest = cosine;
                    best = candidate;
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: f64 = 1_000.0;

    /// **The size handle's ladder.** Anchored at the smallest part, ten steps a decade, and the
    /// modifier gives forty.
    #[test]
    fn a_size_snaps_to_the_r10_ladder_and_the_modifier_gives_r40() {
        assert_eq!(volume(1_300.0, MIN, false), 1_250.0);
        // Past the geometric middle of 1.25 and 1.6, which is 1.414.
        assert_eq!(volume(1_450.0, MIN, false), 1_600.0);
        assert_eq!(volume(1_450.0, MIN, true), 1_500.0);
        assert!((volume(2.36e6, MIN, false) - 2.5e6).abs() < 1e-6);
        assert!((volume(2.36e6, MIN, true) - 2.36e6).abs() < 1e-6);
        // Just under a decade goes up to it rather than staying at 8.
        assert!((volume(9_600.0, MIN, false) - 10_000.0).abs() < 1e-9);
        assert_eq!(volume(10.0, MIN, false), MIN, "nothing below the smallest part");
    }

    /// Every step of the ladder is a fixed point, and each is about 26% past the last.
    #[test]
    fn the_ladder_is_its_own_fixed_points() {
        for (i, step) in R10.iter().enumerate() {
            let v = MIN * 1e3 * step;
            assert!((volume(v, MIN, false) - v).abs() < 1e-6 * v, "{v}");
            let next = R10.get(i + 1).copied().unwrap_or(10.0);
            assert!((next / step - 1.26).abs() < 0.03, "{step} to {next}");
        }
        for step in R40 {
            assert!((volume(MIN * step, MIN, true) - MIN * step).abs() < 1e-9);
        }
    }

    #[test]
    fn a_ratio_below_one_is_on_the_same_series() {
        assert!((ratio(0.33, false) - 0.315).abs() < 1e-12);
        assert!((ratio(1.1, false) - 1.0).abs() < 1e-12);
        assert!((ratio(1.1, true) - 1.12).abs() < 1e-12);
    }

    #[test]
    fn angles_go_by_fifteen_degrees_or_five() {
        assert!((angle(22.0 * DEG, false) - 15.0 * DEG).abs() < 1e-12);
        assert!((angle(23.0 * DEG, false) - 30.0 * DEG).abs() < 1e-12);
        assert!((angle(23.0 * DEG, true) - 25.0 * DEG).abs() < 1e-12);
        assert!((angle(-8.0 * DEG, false) - -15.0 * DEG).abs() < 1e-12);
    }

    #[test]
    fn an_anchor_goes_to_an_axis_or_a_diagonal_unless_free() {
        assert_eq!(anchor(DVec3::new(0.9, 0.1, 0.05), false), DVec3::X);
        assert_eq!(anchor(DVec3::new(-1.0, 0.0, -0.9), false), DVec3::new(-1.0, 0.0, -1.0));
        assert_eq!(anchor(DVec3::new(0.6, 0.5, 0.55), false), DVec3::ONE);
        let free = anchor(DVec3::new(0.9, 0.1, 0.05), true);
        assert!((free.length() - 1.0).abs() < 1e-12 && free.y > 0.0);
    }

    #[test]
    fn a_standoff_goes_by_tenths_of_the_reach() {
        assert_eq!(standoff(-0.27, false), -0.3);
        assert_eq!(standoff(0.04, false), 0.0);
        assert_eq!(standoff(-0.27, true), -0.27);
    }
}
