//! Numbers the editor's Statistics panel and `lc-galaxy stats` show: what the field promises, and
//! what a bake gave.

use std::collections::HashMap;
use std::fmt::Write;

use glam::{DVec3, IVec3};
use lc_world::sky::galaxy::{Kinematics, catalog_star};
use lc_world::sky::generate::census::Census;
use lc_world::sky::shells;

use crate::compiled::{Aabb, Compiled};
use crate::feature::FeatureId;
use crate::sample::BakedStar;
use crate::timeline::Timeline;

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


/// One baked star's system, counted, as the game would generate it: through the same record,
/// kinematics and profile the loader uses. `None` for a star the sky leaves out.
pub fn census_of(star: &BakedStar, kinematics: &Kinematics) -> Option<Census> {
    let a = &star.attributes;
    catalog_star(&a.record, kinematics, a.profile).map(|s| Census::of(&s, &a.profile.tuning()))
}

/// How a column of censuses reads in one row of the systems table.
pub type Measure = fn(&[&Census]) -> String;

/// The systems table's rows, in order.
pub fn system_rows() -> [(&'static str, Measure); 15] {
    fn per_star(cs: &[&Census], f: fn(&Census) -> u32) -> String {
        if cs.is_empty() { "–".into() } else { format!("{:.2}", cs.iter().map(|c| f(c) as f64).sum::<f64>() / cs.len() as f64) }
    }
    fn share(cs: &[&Census], f: fn(&Census) -> bool) -> String {
        if cs.is_empty() { "–".into() } else { format!("{:.0}%", 100.0 * cs.iter().filter(|c| f(c)).count() as f64 / cs.len() as f64) }
    }
    [
        ("systems", |cs| cs.len().to_string()),
        ("planets / star", |cs| per_star(cs, |c| c.planets)),
        ("giants / star", |cs| per_star(cs, |c| c.giants)),
        ("with a belt", |cs| share(cs, |c| c.belt)),
        ("with an Oort cloud", |cs| share(cs, |c| c.oort_cloud)),
        ("habitable / star", |cs| per_star(cs, |c| c.habitable)),
        ("ocean worlds", |cs| cs.iter().map(|c| c.ocean_worlds).sum::<u32>().to_string()),
        ("eligible", |cs| cs.iter().filter(|c| c.eligible).count().to_string()),
        ("ocean-first", |cs| cs.iter().filter(|c| c.ocean_first).count().to_string()),
        ("moons / star", |cs| per_star(cs, |c| c.moons)),
        ("captured / star", |cs| per_star(cs, |c| c.captured_moons)),
        // Of all planets, not a per-star mean: a star with no planets has no planet to count.
        ("submoons", |cs| cs.iter().map(|c| c.submoons).sum::<u32>().to_string()),
        ("planets with a moon", |cs| {
            let planets: u32 = cs.iter().map(|c| c.planets).sum();
            if planets == 0 { "–".into() } else { format!("{:.0}%", 100.0 * cs.iter().map(|c| c.planets_with_moons).sum::<u32>() as f64 / planets as f64) }
        }),
        ("ringed planets", |cs| {
            let planets: u32 = cs.iter().map(|c| c.planets).sum();
            if planets == 0 { "–".into() } else { format!("{:.0}%", 100.0 * cs.iter().map(|c| c.ringed_planets).sum::<u32>() as f64 / planets as f64) }
        }),
        ("bands / ringed planet", |cs| {
            let ringed: u32 = cs.iter().map(|c| c.ringed_planets).sum();
            if ringed == 0 { "–".into() } else { format!("{:.1}", cs.iter().map(|c| c.ring_bands).sum::<u32>() as f64 / ringed as f64) }
        }),
    ]
}

/// Shells set by a neighbor, of those the sky keeps, and their mean radius, ly.
pub struct ShellSummary {
    pub clamped: usize,
    pub kept: usize,
    pub mean_ly: f64,
}

/// From [`crate::publish::shells`] of the same stars.
pub fn shell_summary(stars: &[BakedStar], radii: &[f64]) -> Option<ShellSummary> {
    let kept: Vec<(f64, f64)> = radii
        .iter()
        .zip(stars)
        .filter(|(r, _)| r.is_finite())
        .map(|(r, s)| (*r, shells::nominal_ly(s.attributes.mass_solar)))
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(ShellSummary {
        // A clamped shell is a proportional share of a gap, never more than its nominal radius.
        clamped: kept.iter().filter(|(r, nominal)| *r < nominal * (1.0 - 1e-9)).count(),
        kept: kept.len(),
        mean_ly: kept.iter().map(|(r, _)| r).sum::<f64>() / kept.len() as f64,
    })
}

/// Everything above for the stars inside `region`, as text. The bake covers whole cells, which
/// reach past the region; those stars still count as neighbors for distances and shells.
pub fn report(timeline: &Timeline, region: Aabb, threads: usize) -> Result<String, String> {
    let compiled = timeline.compile(timeline.len(), None);
    let name = |id: FeatureId| timeline.feature(id).map_or(format!("{id}"), |f| f.label());
    let start = std::time::Instant::now();
    let baked = crate::bake(&compiled, timeline.seed, region, threads).map_err(|e| e.to_string())?;
    let took = start.elapsed();
    let nearest_all = nearest_neighbors(&baked.stars);
    let shells_all = crate::publish::shells(&baked.stars);
    let inside: Vec<usize> = (0..baked.stars.len()).filter(|&i| region.contains(baked.stars[i].position_ly)).collect();
    let stars: Vec<BakedStar> = inside.iter().map(|&i| baked.stars[i].clone()).collect();
    let mut out = String::new();
    let (lo, hi) = (region.min, region.max);
    let _ = writeln!(out, "box ({:.1}, {:.1}, {:.1}) to ({:.1}, {:.1}, {:.1}) ly", lo.x, lo.y, lo.z, hi.x, hi.y, hi.z);

    let mut counts = vec![vec!["population".to_string(), "expected".into(), "baked".into()]];
    let mut totals = (0.0, 0);
    for (id, e) in expected(&compiled, region, 32) {
        let n = stars.iter().filter(|s| s.attributes.population == Some(id)).count();
        counts.push(vec![name(id), format!("{e:.1}"), n.to_string()]);
        totals = (totals.0 + e, totals.1 + n);
    }
    counts.push(vec!["total".into(), format!("{:.1}", totals.0), totals.1.to_string()]);
    out += &table(&counts);
    let _ = writeln!(
        out,
        "{} cells, {} candidates, {} stars in them, {:.1} ms",
        baked.cells,
        baked.candidates,
        baked.stars.len(),
        took.as_secs_f64() * 1e3
    );

    let nearest: Vec<f64> = inside.iter().map(|&i| nearest_all[i]).filter(|d| d.is_finite()).collect();
    if !nearest.is_empty() {
        let min = nearest.iter().copied().fold(f64::INFINITY, f64::min);
        let floor = stars.iter().map(|s| s.exclusion_ly).fold(f64::INFINITY, f64::min);
        let mean = nearest.iter().sum::<f64>() / nearest.len() as f64;
        let _ = writeln!(out, "nearest neighbor: min {min:.2} ly, mean {mean:.2} ly, hard-core floor {floor:.2} ly");
    }
    let shells_inside: Vec<f64> = inside.iter().map(|&i| shells_all[i]).collect();
    if let Some(sh) = shell_summary(&stars, &shells_inside) {
        let _ = writeln!(
            out,
            "shell radius: clamped {} of {} ({:.0}%), mean {:.2} ly, solar nominal {:.2} ly",
            sh.clamped,
            sh.kept,
            100.0 * sh.clamped as f64 / sh.kept as f64,
            sh.mean_ly,
            shells::SOLAR_SHELL_LY
        );
    }

    let kinematics: HashMap<FeatureId, Kinematics> =
        compiled.populations.keys().filter_map(|id| Some((*id, crate::attributes::kinematics(&compiled, *id)?))).collect();
    let counted = censuses(&stars, &kinematics, threads);
    let of = |id: Option<FeatureId>| -> Vec<&Census> {
        stars.iter().zip(&counted).filter(|(s, _)| id.is_none() || s.attributes.population == id).filter_map(|(_, c)| c.as_ref()).collect()
    };
    let mut columns: Vec<(String, Vec<&Census>)> = compiled.populations.keys().map(|id| (name(*id), of(Some(*id)))).collect();
    columns.push(("total".into(), of(None)));
    let mut systems = vec![std::iter::once(String::new()).chain(columns.iter().map(|(n, _)| n.clone())).collect::<Vec<_>>()];
    for (label, measure) in system_rows() {
        systems.push(std::iter::once(label.to_string()).chain(columns.iter().map(|(_, cs)| measure(cs))).collect());
    }
    out += &table(&systems);
    Ok(out)
}

fn censuses(stars: &[BakedStar], kinematics: &HashMap<FeatureId, Kinematics>, threads: usize) -> Vec<Option<Census>> {
    let share = stars.len().div_ceil(threads.max(1)).max(1);
    std::thread::scope(|scope| {
        let parts: Vec<_> = stars
            .chunks(share)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|s| census_of(s, s.attributes.population.and_then(|p| kinematics.get(&p))?))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        parts.into_iter().flat_map(|p| p.join().expect("a census thread panicked")).collect()
    })
}

/// Rows as columns: the first left-aligned, the rest right-aligned, two spaces apart.
fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let width: Vec<usize> = (0..columns).map(|c| rows.iter().filter_map(|r| r.get(c)).map(|s| s.chars().count()).max().unwrap_or(0)).collect();
    let mut out = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(c, s)| if c == 0 { format!("{s:<w$}", w = width[c]) } else { format!("{s:>w$}", w = width[c]) })
            .collect();
        out += cells.join("  ").trim_end();
        out.push('\n');
    }
    out
}
