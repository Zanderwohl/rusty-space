//! A bounding hierarchy over sources.
//!
//! Sources number about a hundred thousand against ten billion events, and every event of a
//! source shares that source's worldline. So the light-cone traversal is over this and not over
//! the events: a subtree that cannot reach the observer in time is one bound away from being
//! dropped whole. See [`crate::cursor`].

use glam::DVec3;

/// One source, as the hierarchy needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceNode {
    pub source_id: i64,
    /// Where it is, light-microseconds. Static for now; a moving source means solving the
    /// retarded time per event instead of once per source.
    pub position: DVec3,
    /// The span its events cover, coordinate microseconds.
    pub first_t: f64,
    pub last_t: f64,
    /// The most any one of its events puts out. An upper bound, so a subtree's is the maximum
    /// of its children's and a strength floor can prune on it.
    pub power: f64,
}

/// An axis-aligned box in light-microseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: DVec3,
    pub max: DVec3,
}

impl Bounds {
    pub fn around(points: impl IntoIterator<Item = DVec3>) -> Option<Self> {
        let mut iter = points.into_iter();
        let first = iter.next()?;
        let mut bounds = Self { min: first, max: first };
        for point in iter {
            bounds.min = bounds.min.min(point);
            bounds.max = bounds.max.max(point);
        }
        Some(bounds)
    }

    pub fn join(self, other: Self) -> Self {
        Self { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    /// Shortest distance from this box to any point of a ball. Zero when they overlap.
    ///
    /// The bound the whole traversal rests on: nothing inside the box can reach anywhere the
    /// observer might be in less than this. It must never come out *larger* than the true
    /// shortest distance, or a reception is dropped rather than delayed.
    pub fn distance_to_ball(self, (center, radius): (DVec3, f64)) -> f64 {
        let nearest = center.clamp(self.min, self.max);
        (nearest - center).length() - radius.max(0.0)
    }

    /// Longest distance from this box to any point of a ball. The mirror of
    /// [`Bounds::distance_to_ball`], and it must never come out *smaller* than the truth: it is
    /// used to decide that a subtree's light has already gone past, and an under-estimate would
    /// discard one that had not.
    pub fn farthest_from_ball(self, (center, radius): (DVec3, f64)) -> f64 {
        let low = (self.min - center).abs();
        let high = (self.max - center).abs();
        low.max(high).length() + radius.max(0.0)
    }

    pub fn longest_axis(self) -> usize {
        let size = self.max - self.min;
        if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.z {
            1
        } else {
            2
        }
    }
}

/// One node of the hierarchy.
#[derive(Clone, Debug)]
pub struct Node {
    pub bounds: Bounds,
    /// The earliest any source below this emits. With [`Bounds::distance_to_ball`] this gives
    /// the earliest arrival the subtree can possibly produce, which is its heap key.
    pub earliest_t: f64,
    /// The latest any source below this emits. With [`Bounds::farthest_from_ball`] this gives
    /// the last arrival the subtree can produce, which is what says its light has gone past.
    pub latest_t: f64,
    /// The most any source below this puts out.
    pub strongest: f64,
    /// `None` for a leaf, whose sources are [`Bvh::sources_of`].
    pub children: Option<(usize, usize)>,
    range: std::ops::Range<usize>,
}

/// Sources at most this many to a leaf. Small enough that a leaf is nearly a source, large
/// enough that the tree is not mostly pointers.
pub const LEAF_SIZE: usize = 8;

pub struct Bvh {
    nodes: Vec<Node>,
    sources: Vec<SourceNode>,
}

impl Bvh {
    /// Build over a set of sources. Median split on the longest axis, which needs no heuristic
    /// and gives a balanced tree for the clustered distribution stars actually have.
    pub fn build(mut sources: Vec<SourceNode>) -> Self {
        let mut nodes = Vec::new();
        if !sources.is_empty() {
            let all = 0..sources.len();
            split(&mut nodes, &mut sources, all);
        }
        Self { nodes, sources }
    }

    pub fn root(&self) -> Option<usize> {
        (!self.nodes.is_empty()).then_some(0)
    }

    pub fn node(&self, index: usize) -> &Node {
        &self.nodes[index]
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    pub fn sources(&self) -> &[SourceNode] {
        &self.sources
    }

    /// The sources of a leaf. Empty for an interior node.
    pub fn sources_of(&self, index: usize) -> &[SourceNode] {
        let node = &self.nodes[index];
        match node.children {
            Some(_) => &[],
            None => &self.sources[node.range.clone()],
        }
    }

    /// How many nodes the tree has, for a test that wants to know what a traversal skipped.
    pub fn nodes(&self) -> usize {
        self.nodes.len()
    }
}

/// Build a subtree over `range`, returning its node index.
fn split(nodes: &mut Vec<Node>, sources: &mut [SourceNode], range: std::ops::Range<usize>) -> usize {
    let slice = &sources[range.clone()];
    let bounds = Bounds::around(slice.iter().map(|s| s.position)).expect("a non-empty range");
    let earliest_t = slice.iter().map(|s| s.first_t).fold(f64::INFINITY, f64::min);
    let latest_t = slice.iter().map(|s| s.last_t).fold(f64::NEG_INFINITY, f64::max);
    let strongest = slice.iter().map(|s| s.power).fold(f64::NEG_INFINITY, f64::max);

    let index = nodes.len();
    nodes.push(Node { bounds, earliest_t, latest_t, strongest, children: None, range: range.clone() });

    if range.len() <= LEAF_SIZE {
        return index;
    }
    let axis = bounds.longest_axis();
    let middle = range.start + range.len() / 2;
    sources[range.clone()].select_nth_unstable_by(range.len() / 2, |a, b| {
        a.position[axis].total_cmp(&b.position[axis])
    });
    let left = split(nodes, sources, range.start..middle);
    let right = split(nodes, sources, middle..range.end);
    nodes[index].children = Some((left, right));
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: i64, at: DVec3, first_t: f64) -> SourceNode {
        SourceNode { source_id: id, position: at, first_t, last_t: first_t + 1000.0, power: 1.0 }
    }

    #[test]
    fn an_empty_hierarchy_has_no_root() {
        let bvh = Bvh::build(Vec::new());
        assert!(bvh.root().is_none() && bvh.is_empty());
    }

    /// Every source has to end up in exactly one leaf, or the traversal quietly loses it.
    #[test]
    fn every_source_lands_in_exactly_one_leaf() {
        let sources: Vec<SourceNode> = (0..200i64)
            .map(|i| {
                source(
                    i,
                    DVec3::new((i % 13) as f64 * 90.0, (i % 7) as f64 * 40.0, (i / 13) as f64 * 15.0),
                    i as f64,
                )
            })
            .collect();
        let bvh = Bvh::build(sources.clone());
        let mut seen = std::collections::HashSet::new();
        for index in 0..bvh.nodes() {
            for leaf in bvh.sources_of(index) {
                assert!(seen.insert(leaf.source_id), "source {} is in two leaves", leaf.source_id);
            }
        }
        assert_eq!(seen.len(), sources.len());
    }

    /// A node's box has to contain every source below it and its time has to be the earliest,
    /// because both are used as bounds and a bound that is not one drops receptions.
    #[test]
    fn every_node_bounds_everything_below_it() {
        let sources: Vec<SourceNode> = (0..150i64)
            .map(|i| {
                let f = i as f64;
                source(i, DVec3::new(f.sin() * 1000.0, f.cos() * 700.0, (f * 0.7).sin() * 300.0), f * 3.0)
            })
            .collect();
        let bvh = Bvh::build(sources);
        for index in 0..bvh.nodes() {
            let node = bvh.node(index);
            let below = gather(&bvh, index);
            assert!(!below.is_empty());
            for source in &below {
                assert!(source.position.cmpge(node.bounds.min).all(), "outside the box");
                assert!(source.position.cmple(node.bounds.max).all(), "outside the box");
                assert!(source.first_t >= node.earliest_t, "earlier than the node's earliest");
                assert!(source.last_t <= node.latest_t, "later than the node's latest");
                assert!(source.power <= node.strongest, "louder than the node's loudest");
            }
            let earliest = below.iter().map(|s| s.first_t).fold(f64::INFINITY, f64::min);
            assert_eq!(node.earliest_t, earliest, "the bound is not tight");
        }
    }

    fn gather(bvh: &Bvh, index: usize) -> Vec<SourceNode> {
        match bvh.node(index).children {
            Some((left, right)) => {
                let mut out = gather(bvh, left);
                out.extend(gather(bvh, right));
                out
            }
            None => bvh.sources_of(index).to_vec(),
        }
    }

    /// The distance bound, which is the one thing in the traversal that must never over-state.
    #[test]
    fn the_distance_to_a_ball_is_never_longer_than_the_truth() {
        let bounds = Bounds { min: DVec3::new(-10.0, -20.0, -5.0), max: DVec3::new(10.0, 20.0, 5.0) };
        for (center, radius) in [
            (DVec3::new(100.0, 0.0, 0.0), 0.0),
            (DVec3::new(100.0, 0.0, 0.0), 40.0),
            (DVec3::ZERO, 0.0),
            (DVec3::new(0.0, 0.0, 3.0), 1.0),
            (DVec3::new(-60.0, 55.0, 12.0), 7.0),
        ] {
            let bound = bounds.distance_to_ball((center, radius));
            // Against a sweep of the box: no point of it is nearer to the ball than the bound.
            let mut nearest = f64::INFINITY;
            for i in 0..=8 {
                for j in 0..=8 {
                    for k in 0..=8 {
                        let at = bounds.min
                            + (bounds.max - bounds.min)
                                * DVec3::new(i as f64, j as f64, k as f64)
                                / 8.0;
                        nearest = nearest.min((at - center).length() - radius);
                    }
                }
            }
            assert!(bound <= nearest + 1e-9, "{bound} overstates {nearest}");

            // And the far bound, which must never under-state for the same reason reversed.
            let far = bounds.farthest_from_ball((center, radius));
            let mut furthest: f64 = 0.0;
            for i in 0..=8 {
                for j in 0..=8 {
                    for k in 0..=8 {
                        let at = bounds.min
                            + (bounds.max - bounds.min)
                                * DVec3::new(i as f64, j as f64, k as f64)
                                / 8.0;
                        furthest = furthest.max((at - center).length() + radius);
                    }
                }
            }
            assert!(far >= furthest - 1e-9, "{far} understates {furthest}");
        }
        // Inside the box, the ball is already there.
        assert!(bounds.distance_to_ball((DVec3::ZERO, 0.0)) <= 0.0);
    }

    #[test]
    fn a_single_source_is_a_leaf_and_nothing_else() {
        let bvh = Bvh::build(vec![source(1, DVec3::ZERO, 0.0)]);
        assert_eq!(bvh.nodes(), 1);
        assert!(bvh.node(0).children.is_none());
        assert_eq!(bvh.sources_of(0).len(), 1);
    }
}
