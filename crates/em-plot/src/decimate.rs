//! Reducing more samples than there are pixels.

/// A pixel column's vertical extent, in data units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    pub index: u32,
    pub min: f64,
    pub max: f64,
    pub count: u32,
}

/// The envelope of a series, one column per pixel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Envelope {
    pub columns: Vec<Column>,
}

impl Envelope {
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    /// Smallest value anywhere in the envelope.
    pub fn min(&self) -> Option<f64> {
        self.columns.iter().map(|c| c.min).reduce(f64::min)
    }

    /// Largest value anywhere in the envelope.
    pub fn max(&self) -> Option<f64> {
        self.columns.iter().map(|c| c.max).reduce(f64::max)
    }

    pub fn total_samples(&self) -> u64 {
        self.columns.iter().map(|c| c.count as u64).sum()
    }
}

/// Min/max decimation: for each pixel column, the vertical extent of the samples in it.
///
/// This is not an optimisation, it is the correct algorithm. A light curve has millions of
/// samples against a few hundred pixels, and subsampling — taking every nth point — aliases:
/// a transit one sample wide disappears at some zoom levels and reappears at others, which
/// makes a chart that lies about whether a planet is there. Taking the extent instead
/// preserves the envelope exactly, so a one-sample feature is always drawn.
///
/// Points outside `x_range` are skipped. Non-finite values are skipped. `O(samples)` with a
/// comparison pair each.
pub fn min_max(points: &[(f64, f64)], x_range: (f64, f64), width_px: u32) -> Envelope {
    let (x0, x1) = x_range;
    if width_px == 0 || !(x1 > x0) {
        return Envelope::default();
    }
    let width = width_px as usize;
    let mut slots: Vec<Option<Column>> = vec![None; width];
    let scale = width as f64 / (x1 - x0);

    for &(x, y) in points {
        if !x.is_finite() || !y.is_finite() || x < x0 || x > x1 {
            continue;
        }
        let index = (((x - x0) * scale) as usize).min(width - 1);
        match &mut slots[index] {
            Some(c) => {
                c.min = c.min.min(y);
                c.max = c.max.max(y);
                c.count += 1;
            }
            slot => *slot = Some(Column { index: index as u32, min: y, max: y, count: 1 }),
        }
    }
    Envelope { columns: slots.into_iter().flatten().collect() }
}

/// Mean per column, for drawing a track through a dense envelope.
pub fn means(points: &[(f64, f64)], x_range: (f64, f64), width_px: u32) -> Vec<(u32, f64)> {
    let (x0, x1) = x_range;
    if width_px == 0 || !(x1 > x0) {
        return Vec::new();
    }
    let width = width_px as usize;
    let mut sums = vec![(0.0f64, 0u32); width];
    let scale = width as f64 / (x1 - x0);
    for &(x, y) in points {
        if !x.is_finite() || !y.is_finite() || x < x0 || x > x1 {
            continue;
        }
        let i = (((x - x0) * scale) as usize).min(width - 1);
        sums[i].0 += y;
        sums[i].1 += 1;
    }
    sums.iter()
        .enumerate()
        .filter(|(_, (_, n))| *n > 0)
        .map(|(i, (s, n))| (i as u32, s / *n as f64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A noisy curve with one very narrow, very deep feature — the shape this exists for.
    fn transit_curve(n: usize, dip_at: usize) -> Vec<(f64, f64)> {
        (0..n)
            .map(|i| {
                let x = i as f64 / n as f64;
                let noise = ((i * 2654435761) % 1000) as f64 / 1000.0 * 0.01;
                let y = if i == dip_at { -1.0 } else { 1.0 + noise };
                (x, y)
            })
            .collect()
    }

    #[test]
    fn the_envelope_survives_two_million_samples_in_eight_hundred_pixels() {
        let points = transit_curve(2_000_000, 1_234_567);
        let env = min_max(&points, (0.0, 1.0), 800);
        assert_eq!(env.columns.len(), 800, "every column should have samples");
        assert_eq!(env.total_samples(), 2_000_000);
        assert_eq!(env.min(), Some(-1.0), "the envelope must reach the deepest sample");
        // The noise tops out one part in a thousand below 0.01.
        assert!((env.max().unwrap() - 1.00999).abs() < 1e-9, "{:?}", env.max());
    }

    /// The property that makes this the correct algorithm rather than a faster one.
    #[test]
    fn a_one_sample_transit_survives_every_zoom_level() {
        let n = 2_000_000;
        let dip_at = 1_234_567;
        let points = transit_curve(n, dip_at);
        let dip_x = dip_at as f64 / n as f64;

        for width in [80, 200, 800, 1920, 4000] {
            for span in [1.0, 0.5, 0.1, 0.01, 0.001] {
                let (lo, hi) = (dip_x - span / 2.0, dip_x + span / 2.0);
                let env = min_max(&points, (lo, hi), width);
                assert_eq!(
                    env.min(),
                    Some(-1.0),
                    "the transit vanished at width {width}, span {span}"
                );
            }
        }
    }

    #[test]
    fn subsampling_is_what_this_replaces() {
        // The same data, taking every nth point, loses the feature at most zoom levels.
        let n = 2_000_000;
        let points = transit_curve(n, 1_234_567);
        let stride = n / 800;
        let sampled: Vec<f64> = points.iter().step_by(stride).map(|(_, y)| *y).collect();
        let lowest = sampled.iter().cloned().fold(f64::INFINITY, f64::min);
        assert!(lowest > 0.0, "subsampling happened to keep the dip; pick another index");
        assert_eq!(min_max(&points, (0.0, 1.0), 800).min(), Some(-1.0));
    }

    #[test]
    fn columns_are_ordered_and_only_populated_ones_appear() {
        let points = vec![(0.0, 1.0), (0.05, 2.0), (0.95, 3.0)];
        let env = min_max(&points, (0.0, 1.0), 10);
        assert_eq!(env.columns.len(), 2, "eight columns are empty and should not be drawn");
        assert!(env.columns.windows(2).all(|w| w[0].index < w[1].index));
        assert_eq!(env.columns[0], Column { index: 0, min: 1.0, max: 2.0, count: 2 });
        assert_eq!(env.columns[1].index, 9);
    }

    #[test]
    fn points_outside_the_view_and_non_finite_values_are_skipped() {
        let points = vec![
            (-1.0, 5.0),
            (0.5, 1.0),
            (2.0, 9.0),
            (0.6, f64::NAN),
            (f64::INFINITY, 1.0),
        ];
        let env = min_max(&points, (0.0, 1.0), 10);
        assert_eq!(env.total_samples(), 1);
        assert_eq!(env.max(), Some(1.0));
    }

    #[test]
    fn degenerate_ranges_produce_nothing_rather_than_panicking() {
        let points = vec![(0.0, 1.0)];
        assert!(min_max(&points, (0.0, 0.0), 10).is_empty());
        assert!(min_max(&points, (1.0, 0.0), 10).is_empty());
        assert!(min_max(&points, (0.0, 1.0), 0).is_empty());
        assert!(means(&points, (0.0, 0.0), 10).is_empty());
    }

    #[test]
    fn means_track_through_the_envelope() {
        let points: Vec<(f64, f64)> =
            (0..1000).map(|i| (i as f64 / 1000.0, if i % 2 == 0 { 0.0 } else { 2.0 })).collect();
        let m = means(&points, (0.0, 1.0), 10);
        assert_eq!(m.len(), 10);
        for (_, v) in m {
            assert!((v - 1.0).abs() < 1e-9, "mean of alternating 0 and 2 is 1, got {v}");
        }
    }
}
