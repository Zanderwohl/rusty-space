//! Numbers the editor's Statistics panel shows: what the field promises, and what a bake gave.

use glam::{DVec3, IVec3};

use crate::compiled::{Aabb, Compiled};
use crate::feature::FeatureId;
use crate::sample::BakedStar;

/// The expected number of stars per population in `region`: the integral of the density field
/// by the midpoint rule on `resolution³` points, which is what a bake's count scatters around.
pub fn expected(compiled: &Compiled, region: Aabb, resolution: usize) -> Vec<(FeatureId, f64)> {
    let mut sums = vec![0.0; compiled.populations.len()];
    if !region.is_empty() && resolution > 0 {
        let step = region.size() / resolution as f64;
        let dv = step.x * step.y * step.z;
        for i in 0..resolution {
            for j in 0..resolution {
                for k in 0..resolution {
                    let p = region.min + (DVec3::new(i as f64, j as f64, k as f64) + 0.5) * step;
                    for (s, n) in sums.iter_mut().zip(compiled.sample(p).density) {
                        *s += n * dv;
                    }
                }
            }
        }
    }
    compiled.populations.keys().copied().zip(sums).collect()
}

/// Each star's distance to its nearest neighbor, ly, in `stars` order; infinite for a lone star.
pub fn nearest_neighbors(stars: &[BakedStar]) -> Vec<f64> {
    const BUCKET_LY: f64 = 4.0;
    let bucket = |p: DVec3| (p / BUCKET_LY).floor().as_ivec3();
    let mut grid: std::collections::HashMap<IVec3, Vec<usize>> = std::collections::HashMap::new();
    for (i, s) in stars.iter().enumerate() {
        grid.entry(bucket(s.position_ly)).or_default().push(i);
    }
    let reach = stars.iter().fold(Aabb::EMPTY, |b, s| b.union(Aabb::around(s.position_ly, DVec3::ZERO)));
    let max_ring = if reach.is_empty() { 0 } else { (reach.size().max_element() / BUCKET_LY).ceil() as i32 + 1 };
    stars
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let home = bucket(s.position_ly);
            let mut best = f64::INFINITY;
            for ring in 0..=max_ring {
                // Everything in ring `ring` is at least `(ring - 1)` buckets away.
                if (ring - 1) as f64 * BUCKET_LY > best {
                    break;
                }
                for x in -ring..=ring {
                    for y in -ring..=ring {
                        for z in -ring..=ring {
                            if x.abs().max(y.abs()).max(z.abs()) != ring {
                                continue;
                            }
                            for &j in grid.get(&(home + IVec3::new(x, y, z))).into_iter().flatten() {
                                if j != i {
                                    best = best.min(s.position_ly.distance(stars[j].position_ly));
                                }
                            }
                        }
                    }
                }
            }
            best
        })
        .collect()
}
