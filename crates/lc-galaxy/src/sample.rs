//! Placing stars: Matérn type II thinning of an inhomogeneous Poisson process, seeded per cell
//! of a fixed global grid.
//!
//! A candidate survives unless another within `max(r_a, r_b)` has a smaller mark. Exclusion
//! distances are capped at one cell, so a cell's stars depend only on its own candidates and its
//! 26 neighbors': any region bakes alone, in any order, on any number of threads, and gives
//! the stars a whole-galaxy bake would. See `34-galaxy-authoring.md`, "Placing stars".

use std::f64::consts::PI;

use glam::{DVec3, IVec3};
use lc_world::rng;
use lc_world::sky::StarId;

use crate::attributes::{self, Attributes};
use crate::compiled::{Aabb, Compiled};

/// Side of a grid cell in ly. Part of the format: the grid is anchored at the origin and never
/// moves.
pub const CELL_LY: f64 = 8.0;

/// Cells per axis either side of the origin: `2^13 · 8 ly` covers the ±36 500 ly box.
const CELL_HALF_RANGE: i32 = 1 << 13;

/// The sampled intensity bound is raised by this before drawing, and doubled again if a
/// candidate still exceeds it. Deterministic either way: the bound is a function of the field.
const BOUND_MARGIN: f64 = 1.5;
const BOUND_RETRIES: u32 = 8;

/// Lattice points per axis when sampling a cell for its intensity bound: 1 ly apart.
const BOUND_LATTICE: usize = 9;

const SALT_COUNT: u64 = 0x636f_756e_74;
const SALT_CANDIDATE: u64 = 0x6361_6e64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell(pub IVec3);

impl Ord for Cell {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.to_array().cmp(&o.0.to_array())
    }
}

impl PartialOrd for Cell {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}

impl Cell {
    pub fn containing(p: DVec3) -> Self {
        Cell((p / CELL_LY).floor().as_ivec3())
    }

    pub fn min_corner(self) -> DVec3 {
        self.0.as_dvec3() * CELL_LY
    }

    pub fn bounds(self) -> Aabb {
        let min = self.min_corner();
        Aabb { min, max: min + DVec3::splat(CELL_LY) }
    }

    pub fn in_grid(self) -> bool {
        self.0.to_array().iter().all(|v| (-CELL_HALF_RANGE..CELL_HALF_RANGE).contains(v))
    }

    /// 14 bits per axis. Only for a cell [`Cell::in_grid`]; one outside would share bits with
    /// another, which is why the bake refuses such a region.
    pub fn packed(self) -> u64 {
        let axis = |v: i32| ((v + CELL_HALF_RANGE) as u64) & 0x3fff;
        axis(self.0.x) << 28 | axis(self.0.y) << 14 | axis(self.0.z)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Tier {
    /// Luminous stars, published for the whole galaxy at once.
    A,
    B,
}

/// Which provider built an id, in its low bits: distinct from every hashed catalog id only
/// statistically, but distinct from every other built one by construction.
const GALAXY_PROVIDER_TAG: u64 = 1;

/// A star's address: its cell and its candidate's index there, which is what its id is built
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StarKey {
    pub cell: Cell,
    pub index: u16,
}

impl StarKey {
    /// `cell (42) | index (16) | tier (1) | provider (5)`.
    pub fn star_id(self, tier: Tier) -> StarId {
        let tier = match tier {
            Tier::A => 1,
            Tier::B => 0,
        };
        StarId::from_raw(self.cell.packed() << 22 | (self.index as u64) << 6 | tier << 5 | GALAXY_PROVIDER_TAG)
    }

    fn hash(self, seed: u64, salt: u64) -> u64 {
        rng::hash(&[seed, self.cell.packed(), self.index as u64, salt])
    }
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub key: StarKey,
    pub position: DVec3,
    pub mark: f64,
    pub exclusion_ly: f64,
}

#[derive(Clone, Debug)]
pub struct BakedStar {
    pub key: StarKey,
    pub id: StarId,
    pub position_ly: DVec3,
    pub exclusion_ly: f64,
    pub attributes: Attributes,
}

#[derive(Clone, Debug, Default)]
pub struct Bake {
    pub stars: Vec<BakedStar>,
    pub cells: usize,
    /// Candidates kept by the intensity thinning, before the hard-core rule.
    pub candidates: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BakeError {
    /// More candidates than a 16-bit index can name.
    Overfull { cell: Cell, candidates: u64 },
    /// The field exceeded every bound tried; a spike narrower than the sampling lattice.
    Unbounded { cell: Cell },
    /// A cell outside the grid, whose id would collide with another's.
    OutsideGrid { cell: Cell },
}

impl std::fmt::Display for BakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BakeError::Overfull { cell, candidates } => {
                write!(f, "cell {} would draw {candidates} candidates, more than 65 536", cell.0)
            }
            BakeError::Unbounded { cell } => write!(f, "the field in cell {} has a spike the bound cannot find", cell.0),
            BakeError::OutsideGrid { cell } => write!(f, "cell {} is outside the grid", cell.0),
        }
    }
}

impl std::error::Error for BakeError {}

/// Candidates per ly³ that thin to `n` surviving at packing `p = nV`: `-ln(1 - p) / V`.
fn intensity(n: f64, packing: f64) -> f64 {
    if n <= 0.0 || packing <= 0.0 {
        return n.max(0.0);
    }
    n * -libm::log1p(-packing) / packing
}

/// The hard-core distance at density `n` and packing `p`, capped at one cell so that thinning
/// stays local. The cap binds only below about 2e-4 stars per ly³, where neighbors are
/// farther apart than a cell and the rule rarely decides anything.
pub fn exclusion_radius(n: f64, packing: f64) -> f64 {
    if n <= 0.0 || packing <= 0.0 {
        return if packing <= 0.0 { 0.0 } else { CELL_LY };
    }
    libm::cbrt(3.0 * packing / (4.0 * PI * n)).min(CELL_LY)
}

/// Poisson by inversion in pieces small enough that `e^-mean` does not underflow.
fn poisson(h: u64, mean: f64) -> u64 {
    const PIECE: f64 = 20.0;
    let pieces = (mean / PIECE).ceil().max(1.0) as u64;
    let each = mean / pieces as f64;
    (0..pieces).map(|i| poisson_small(rng::mix(h ^ i.wrapping_mul(0x9e37_79b9)), each)).sum()
}

fn poisson_small(h: u64, mean: f64) -> u64 {
    if mean <= 0.0 {
        return 0;
    }
    let target = rng::uniform(h);
    let mut term = libm::exp(-mean);
    let (mut cumulative, mut k) = (term, 0u64);
    while cumulative < target && k < 200 {
        k += 1;
        term *= mean / k as f64;
        cumulative += term;
    }
    k
}

fn bound_for(compiled: &Compiled, cell: Cell) -> f64 {
    let min = cell.min_corner();
    let step = CELL_LY / (BOUND_LATTICE - 1) as f64;
    let mut best = 0.0f64;
    for i in 0..BOUND_LATTICE {
        for j in 0..BOUND_LATTICE {
            for k in 0..BOUND_LATTICE {
                let p = min + DVec3::new(i as f64, j as f64, k as f64) * step;
                let s = compiled.sample(p);
                best = best.max(intensity(s.total(), s.packing));
            }
        }
    }
    best * BOUND_MARGIN
}

/// The candidates of one cell that pass the intensity thinning, in index order.
pub fn candidates(compiled: &Compiled, seed: u64, cell: Cell) -> Result<Vec<Candidate>, BakeError> {
    let half_diagonal = CELL_LY * 3f64.sqrt() / 2.0;
    if !compiled.may_reach(cell.bounds().center(), half_diagonal) {
        return Ok(Vec::new());
    }
    let mut bound = bound_for(compiled, cell);
    if bound == 0.0 {
        return Ok(Vec::new());
    }
    let volume = CELL_LY * CELL_LY * CELL_LY;
    let count_hash = rng::hash(&[seed, cell.packed(), SALT_COUNT]);
    'retry: for _ in 0..BOUND_RETRIES {
        let count = poisson(count_hash, bound * volume);
        if count > u16::MAX as u64 {
            return Err(BakeError::Overfull { cell, candidates: count });
        }
        let mut out = Vec::new();
        for index in 0..count as u16 {
            let key = StarKey { cell, index };
            let h = key.hash(seed, SALT_CANDIDATE);
            let u = |n: u64| rng::uniform(rng::mix(h ^ n));
            let position = cell.min_corner() + DVec3::new(u(1), u(2), u(3)) * CELL_LY;
            let s = compiled.sample(position);
            let lambda = intensity(s.total(), s.packing);
            if lambda > bound {
                bound *= 2.0;
                continue 'retry;
            }
            if u(4) * bound < lambda {
                out.push(Candidate { key, position, mark: u(5), exclusion_ly: exclusion_radius(s.total(), s.packing) });
            }
        }
        return Ok(out);
    }
    Err(BakeError::Unbounded { cell })
}

/// Every cell that overlaps `region`.
pub fn cells_in(region: Aabb) -> Vec<Cell> {
    if region.is_empty() {
        return Vec::new();
    }
    let lo = Cell::containing(region.min).0;
    let hi = Cell::containing(region.max).0;
    let mut out = Vec::new();
    for x in lo.x..=hi.x {
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                out.push(Cell(IVec3::new(x, y, z)));
            }
        }
    }
    out
}

/// Runs `f` over `items` on `threads` threads, keeping input order.
fn par_map<T: Sync, R: Send>(items: &[T], threads: usize, f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = threads.max(1).min(items.len().max(1));
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = items.chunks(chunk).map(|c| scope.spawn(|| c.iter().map(&f).collect::<Vec<_>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("bake worker panicked")).collect()
    })
}

pub fn default_threads() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
}

/// The stars of every cell overlapping `region`.
pub fn bake(compiled: &Compiled, seed: u64, region: Aabb, threads: usize) -> Result<Bake, BakeError> {
    let cells = cells_in(region);
    let ring = cells_in(Aabb { min: region.min - DVec3::splat(CELL_LY), max: region.max + DVec3::splat(CELL_LY) });
    if let Some(&cell) = ring.iter().find(|c| !c.in_grid()) {
        return Err(BakeError::OutsideGrid { cell });
    }
    let drawn = par_map(&ring, threads, |&c| candidates(compiled, seed, c).map(|v| (c, v)));
    let mut by_cell = std::collections::HashMap::with_capacity(drawn.len());
    for d in drawn {
        let (c, v) = d?;
        by_cell.insert(c, v);
    }
    let survivors = par_map(&cells, threads, |&cell| {
        let Some(own) = by_cell.get(&cell) else { return Vec::new() };
        let neighbors: Vec<&Candidate> = neighborhood(cell).filter_map(|c| by_cell.get(&c)).flatten().collect();
        own.iter()
            .filter(|a| {
                !neighbors.iter().any(|b| {
                    b.key != a.key
                        && (b.mark, b.key) < (a.mark, a.key)
                        && a.position.distance_squared(b.position) < a.exclusion_ly.max(b.exclusion_ly).powi(2)
                })
            })
            .map(|c| BakedStar {
                key: c.key,
                id: c.key.star_id(Tier::B),
                position_ly: c.position,
                exclusion_ly: c.exclusion_ly,
                attributes: attributes::draw(compiled, seed, c.key, c.position),
            })
            .collect::<Vec<_>>()
    });
    let candidates = cells.iter().filter_map(|c| by_cell.get(c)).map(Vec::len).sum();
    Ok(Bake { stars: survivors.into_iter().flatten().collect(), cells: cells.len(), candidates })
}

fn neighborhood(cell: Cell) -> impl Iterator<Item = Cell> {
    (-1..=1).flat_map(move |x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| Cell(cell.0 + IVec3::new(x, y, z)))))
}
