//! The light-cone cursor: what an observer can see, in the order it arrives.
//!
//! The naive query — which events lie on this observer's past light cone — has no useful index
//! and is the wrong question besides. An event's reachability is a property of *where its source
//! was*, and sources are structured: a hundred thousand of them against ten billion events, each
//! source's events sharing one worldline. So the traversal goes over sources, and only descends
//! into events once a source is known to be reachable.
//!
//! A binary heap of nodes, keyed by the **earliest arrival that node could possibly produce**.
//! Because the key is a lower bound, popping in heap order yields receptions in arrival order —
//! without sorting the result, and without visiting what a caller stops before reaching. See
//! `lightcone/docs/02-event-store.md`.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use glam::DVec3;
use lc_spacetime::Worldline;

use crate::bvh::{Bvh, SourceNode};
use crate::id::EventId;

/// One event, as the cursor needs it. Positions are the source's, so only time varies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SourceEvent {
    pub event_id: EventId,
    /// Coordinate time of emission, microseconds.
    pub t: f64,
    /// What the source put out, in whatever units the caller's strength model uses.
    pub power: f64,
}

/// One event arriving at one observer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reception {
    pub event_id: EventId,
    pub source_id: i64,
    /// Coordinate time the light arrives, microseconds.
    pub arrive_t: f64,
    /// Unit vector from the observer toward where the source appears to be.
    pub direction: DVec3,
    /// Power over the square of the distance travelled.
    pub strength: f64,
}

/// Where a source's events come from.
///
/// Synchronous, so the cursor can be an `Iterator` as the design says. The traversal is the
/// deliverable here; a server that wants these out of Postgres loads a source's range when the
/// cursor asks for it, which is once per reachable source and never for the rest.
pub trait EventSource {
    /// One source's events with emission time in `[from_t, to_t]`, ascending. Ascending is
    /// required, not preferred: the merge assumes it and would silently reorder without it.
    fn range(&self, source_id: i64, from_t: f64, to_t: f64) -> Vec<SourceEvent>;
}

/// What the traversal cost, for a caller that wants to know whether it stopped early.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Work {
    /// Interior nodes opened.
    pub nodes: usize,
    /// Leaves whose sources were solved for arrival.
    pub sources_solved: usize,
    /// Calls into the event source.
    pub source_reads: usize,
    /// Events handed back by those calls. The one that matters for a tick: a source that emits
    /// continuously is always a candidate, and what a narrow window saves is the range read out
    /// of it rather than the source itself.
    pub events_read: usize,
    /// Nodes and sources dropped on the bound without being looked at.
    pub pruned: usize,
}

/// A heap entry, keyed by the earliest arrival it could still produce.
enum Pending {
    /// A subtree of the source hierarchy.
    Node(usize),
    /// One source's remaining events, already solved and in arrival order.
    Source { source_id: i64, position: DVec3, events: Vec<(f64, SourceEvent)>, next: usize },
}

struct Keyed {
    key: f64,
    pending: Pending,
}

impl PartialEq for Keyed {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for Keyed {}
impl PartialOrd for Keyed {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Keyed {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed: `BinaryHeap` is a max-heap and the smallest possible arrival has to come
        // out first, or the output is not in arrival order.
        other.key.total_cmp(&self.key)
    }
}

/// Streams what an observer can see, ordered by arrival time, cheapest first.
pub struct LightConeCursor<'a, E: EventSource> {
    bvh: &'a Bvh,
    events: &'a E,
    observer: &'a dyn Worldline,
    from_t: f64,
    to_t: f64,
    /// Receptions weaker than this are never yielded, and a node that could not beat it is
    /// never opened.
    floor: f64,
    heap: BinaryHeap<Keyed>,
    /// The ball the observer cannot leave over the window. Every bound is measured to this.
    ball: (DVec3, f64),
    work: Work,
}

impl<'a, E: EventSource> LightConeCursor<'a, E> {
    /// Everything reaching `observer` with arrival in `[from_t, to_t]`.
    pub fn new(
        bvh: &'a Bvh,
        events: &'a E,
        observer: &'a dyn Worldline,
        from_t: f64,
        to_t: f64,
    ) -> Self {
        Self::with_floor(bvh, events, observer, from_t, to_t, 0.0)
    }

    /// The same, ignoring anything that arrives weaker than `floor`.
    ///
    /// The bounding ball bounds `1/r^2` as well as the travel time, so a strength floor prunes
    /// whole subtrees rather than filtering their output: a node whose *nearest* possible source
    /// is too far to be heard cannot contain one that is not.
    pub fn with_floor(
        bvh: &'a Bvh,
        events: &'a E,
        observer: &'a dyn Worldline,
        from_t: f64,
        to_t: f64,
        floor: f64,
    ) -> Self {
        let ball = observer.bounding_ball(from_t, to_t);
        let mut cursor = Self {
            bvh,
            events,
            observer,
            from_t,
            to_t,
            floor,
            heap: BinaryHeap::new(),
            ball,
            work: Work::default(),
        };
        if let Some(root) = bvh.root() {
            cursor.offer_node(root);
        }
        cursor
    }

    pub fn work(&self) -> Work {
        self.work
    }

    /// Push a subtree, unless its best case is outside the window or under the floor.
    fn offer_node(&mut self, index: usize) {
        let node = self.bvh.node(index);
        let gap = node.bounds.distance_to_ball(self.ball);
        let key = node.earliest_t + gap;
        // Three ways a subtree cannot contribute: its first light has not arrived by the end of
        // the window, its last light went past before the start of it, or the nearest it could
        // be is still too far to be heard. The middle one is what makes a tick cheap -- almost
        // everything a world has emitted went past long ago.
        let gone = node.latest_t + node.bounds.farthest_from_ball(self.ball) < self.from_t;
        if key > self.to_t || gone || !self.can_be_heard(node.strongest, gap) {
            self.work.pruned += 1;
            return;
        }
        // The raw bound, not clamped up to the window's start. Clamping would be correct --
        // every reception is at or after `from_t` -- and would flatten every node whose earliest
        // emission predates the window onto one key, which is most of them. The heap would then
        // have nothing to order by and would open the whole tree before yielding a row.
        self.heap.push(Keyed { key, pending: Pending::Node(index) });
    }

    /// Whether anything at `gap` or further, emitting at most `power`, could clear the floor.
    fn can_be_heard(&self, power: f64, gap: f64) -> bool {
        if self.floor <= 0.0 {
            return true;
        }
        // At the nearest the node can be, which is the loudest it can be.
        strength(power, gap) >= self.floor
    }

    /// Solve one source and push whatever it has in the window.
    fn offer_source(&mut self, source: &SourceNode) {
        self.work.sources_solved += 1;
        // The window is on arrival, so the emissions that can land in it are bounded by the
        // light travel time to the nearest and furthest the observer can be.
        let (centre, radius) = self.ball;
        let gap = (source.position - centre).length();
        let earliest_emission = self.from_t - (gap + radius);
        let latest_emission = self.to_t - (gap - radius).max(0.0);
        let from = earliest_emission.max(source.first_t);
        let to = latest_emission.min(source.last_t);
        if from > to {
            self.work.pruned += 1;
            return;
        }

        self.work.source_reads += 1;
        let raw = self.events.range(source.source_id, from, to);
        self.work.events_read += raw.len();
        let mut solved: Vec<(f64, SourceEvent)> = Vec::with_capacity(raw.len());
        for event in raw {
            // The *observer's* worldline. Solving against the source's own gives the time
            // the light reaches where it started, which is when it left.
            let Some(arrive) =
                lc_spacetime::arrival_time_at(event.t, source.position, self.observer)
            else {
                continue;
            };
            if arrive < self.from_t || arrive > self.to_t {
                continue;
            }
            let travelled = (self.observer.position_at(arrive) - source.position).length();
            if !self.can_be_heard(event.power, travelled) {
                continue;
            }
            solved.push((arrive, event));
        }
        if solved.is_empty() {
            return;
        }
        // A static source and a sub-luminal observer cannot reorder emissions, but the
        // arrival order is what the merge needs and asserting it costs one pass.
        solved.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.heap.push(Keyed {
            key: solved[0].0,
            pending: Pending::Source {
                source_id: source.source_id,
                position: source.position,
                events: solved,
                next: 0,
            },
        });
    }
}

/// Power over the square of the distance travelled, with a floor on the distance so a
/// coincident source is bright rather than infinite.
pub fn strength(power: f64, distance: f64) -> f64 {
    power / distance.max(1.0).powi(2)
}

impl<E: EventSource> Iterator for LightConeCursor<'_, E> {
    type Item = Reception;

    fn next(&mut self) -> Option<Reception> {
        loop {
            let Keyed { key, pending } = self.heap.pop()?;
            match pending {
                Pending::Node(index) => {
                    let node = self.bvh.node(index);
                    self.work.nodes += 1;
                    match node.children {
                        Some((left, right)) => {
                            self.offer_node(left);
                            self.offer_node(right);
                        }
                        None => {
                            for source in self.bvh.sources_of(index).to_vec() {
                                self.offer_source(&source);
                            }
                        }
                    }
                }
                Pending::Source { source_id, position, events, next } => {
                    let (arrive, event) = events[next];
                    debug_assert!(key <= arrive + 1e-6, "the heap key was not a lower bound");
                    let at = self.observer.position_at(arrive);
                    let offset = position - at;
                    let travelled = offset.length();
                    if next + 1 < events.len() {
                        let following = events[next + 1].0;
                        self.heap.push(Keyed {
                            key: following,
                            pending: Pending::Source {
                                source_id,
                                position,
                                events,
                                next: next + 1,
                            },
                        });
                    }
                    return Some(Reception {
                        event_id: event.event_id,
                        source_id,
                        arrive_t: arrive,
                        direction: if travelled > 0.0 { offset / travelled } else { DVec3::X },
                        strength: strength(event.power, travelled),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use lc_spacetime::worldline::{Inertial, Static};

    use super::*;
    use crate::bvh::SourceNode;

    /// Events held by source, already in ascending time.
    #[derive(Default)]
    struct Memory {
        by_source: std::collections::HashMap<i64, Vec<SourceEvent>>,
        reads: std::cell::Cell<usize>,
    }

    impl EventSource for Memory {
        fn range(&self, source_id: i64, from_t: f64, to_t: f64) -> Vec<SourceEvent> {
            self.reads.set(self.reads.get() + 1);
            self.by_source
                .get(&source_id)
                .map(|all| {
                    all.iter().filter(|e| e.t >= from_t && e.t <= to_t).copied().collect()
                })
                .unwrap_or_default()
        }
    }

    fn splitmix(state: &mut u64) -> f64 {
        *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A world: sources scattered through a cube a few thousand light-microseconds across,
    /// each emitting a handful of events over the window.
    fn world(sources: usize, per_source: usize) -> (Bvh, Memory) {
        let mut seed = 0x51ed_2701u64;
        let mut nodes = Vec::new();
        let mut memory = Memory::default();
        let mut next_id = 1i64;
        for index in 0..sources as i64 {
            let position = DVec3::new(
                splitmix(&mut seed) * 8000.0 - 4000.0,
                splitmix(&mut seed) * 8000.0 - 4000.0,
                splitmix(&mut seed) * 8000.0 - 4000.0,
            );
            let mut events = Vec::new();
            for _ in 0..per_source {
                events.push(SourceEvent {
                    event_id: EventId::new(next_id, 0, 0).unwrap(),
                    t: splitmix(&mut seed) * 20_000.0 - 10_000.0,
                    power: 1.0 + splitmix(&mut seed) * 9.0,
                });
                next_id += 1;
            }
            events.sort_by(|a, b| a.t.total_cmp(&b.t));
            let first_t = events.first().map(|e| e.t).unwrap_or(0.0);
            let last_t = events.last().map(|e| e.t).unwrap_or(0.0);
            let power = events.iter().map(|e| e.power).fold(0.0f64, f64::max);
            nodes.push(SourceNode { source_id: index, position, first_t, last_t, power });
            memory.by_source.insert(index, events);
        }
        (Bvh::build(nodes), memory)
    }

    /// Everything, worked out the slow and obviously-correct way.
    fn brute_force(
        bvh: &Bvh,
        memory: &Memory,
        observer: &dyn Worldline,
        from_t: f64,
        to_t: f64,
        floor: f64,
    ) -> Vec<Reception> {
        let mut out = Vec::new();
        for source in bvh.sources() {
            for event in memory.by_source.get(&source.source_id).into_iter().flatten() {
                let Some(arrive) =
                    lc_spacetime::arrival_time_at(event.t, source.position, observer)
                else {
                    continue;
                };
                if arrive < from_t || arrive > to_t {
                    continue;
                }
                let travelled = (observer.position_at(arrive) - source.position).length();
                let strength = strength(event.power, travelled);
                if strength < floor {
                    continue;
                }
                out.push(Reception {
                    event_id: event.event_id,
                    source_id: source.source_id,
                    arrive_t: arrive,
                    direction: DVec3::ZERO,
                    strength,
                });
            }
        }
        out.sort_by(|a, b| a.arrive_t.total_cmp(&b.arrive_t));
        out
    }

    /// The first half of the acceptance criterion: arrival order, and every reception that
    /// should be there.
    ///
    /// Against a brute-force pass rather than against itself. The traversal exists to avoid
    /// looking at most of the world, and the failure it can have is not looking at something it
    /// should have — which only a full pass can catch.
    #[test]
    fn the_cursor_yields_everything_in_arrival_order() {
        let (bvh, memory) = world(300, 4);
        let observer = Inertial::new(DVec3::new(500.0, -200.0, 100.0), DVec3::new(0.2, 0.1, -0.05), 0.0);
        let (from_t, to_t) = (-2_000.0, 9_000.0);

        let got: Vec<Reception> =
            LightConeCursor::new(&bvh, &memory, &observer, from_t, to_t).collect();
        assert!(got.windows(2).all(|w| w[0].arrive_t <= w[1].arrive_t), "out of arrival order");

        let want = brute_force(&bvh, &memory, &observer, from_t, to_t, 0.0);
        assert_eq!(got.len(), want.len(), "the traversal lost {} receptions", want.len() - got.len());
        for (a, b) in got.iter().zip(want.iter()) {
            assert_eq!(a.event_id, b.event_id, "a different event at the same place in the order");
            assert!((a.arrive_t - b.arrive_t).abs() < 1e-6);
            assert!((a.strength - b.strength).abs() < 1e-9 * b.strength.max(1.0));
        }
        assert!(!got.is_empty(), "the window caught nothing, so nothing was tested");
    }

    /// The second half: stopping early has to cost less. A cursor that ordered its output by
    /// computing all of it first would pass every test above and fail this one, which is the
    /// whole reason it is a heap and not a sort.
    ///
    /// The window opens before any light could have arrived and closes after all of it, which
    /// is the case the property is claimed for — streaming from the beginning and stopping. A
    /// window that opened *after* the earliest possible arrival is a different matter: the
    /// node bound is then below its start for nearly every node, every node is genuinely a
    /// candidate, and there is nothing to order by. That is a fact about what a summary of a
    /// subtree can know, not a fault in the traversal.
    #[test]
    fn stopping_early_does_the_work_of_stopping_early() {
        let (bvh, memory) = world(600, 4);
        let observer = Static::new(DVec3::new(200.0, 200.0, 200.0));
        let (from_t, to_t) = (-40_000.0, 40_000.0);

        let mut whole = LightConeCursor::new(&bvh, &memory, &observer, from_t, to_t);
        let all: Vec<Reception> = whole.by_ref().collect();
        let full = whole.work();
        assert!(all.len() > 2_000, "only {} receptions to stop early in", all.len());

        let mut short = LightConeCursor::new(&bvh, &memory, &observer, from_t, to_t);
        let first: Vec<Reception> = short.by_ref().take(20).collect();
        let partial = short.work();

        assert_eq!(first.len(), 20);
        // The same first twenty: an early stop must not change what comes out, only how much
        // was done to get it.
        for (a, b) in first.iter().zip(all.iter()) {
            assert_eq!(a.event_id, b.event_id);
        }
        assert!(
            partial.source_reads * 4 < full.source_reads,
            "twenty of {} receptions read {} sources against {}",
            all.len(),
            partial.source_reads,
            full.source_reads,
        );
        assert!(partial.nodes * 2 < full.nodes, "it opened most of the tree anyway");
    }

    /// What a server tick asks for: a narrow slice of arrival time, long after most of the
    /// world emitted anything. Almost every subtree's light went past before the slice opened,
    /// and a subtree that can be dropped on that has to be dropped whole rather than read.
    #[test]
    fn a_tick_sized_window_reads_almost_nothing() {
        let (bvh, memory) = world(600, 4);
        let observer = Static::new(DVec3::new(200.0, 200.0, 200.0));

        let mut whole = LightConeCursor::new(&bvh, &memory, &observer, -40_000.0, 40_000.0);
        let everything: Vec<Reception> = whole.by_ref().collect();
        let full = whole.work();

        // One per cent of the span the world covers.
        let (from_t, to_t) = (5_000.0, 5_800.0);
        let mut tick = LightConeCursor::new(&bvh, &memory, &observer, from_t, to_t);
        let got: Vec<Reception> = tick.by_ref().collect();
        let tick_work = tick.work();

        let want = brute_force(&bvh, &memory, &observer, from_t, to_t, 0.0);
        assert_eq!(got.len(), want.len(), "the tick lost receptions");
        assert!(!got.is_empty() && got.len() < everything.len() / 10, "{} receptions", got.len());

        // Events, not sources. A source that emits continuously is a candidate for any window
        // that its own span reaches, so the hierarchy cannot drop it on time alone — what it
        // drops is by distance and by strength. The narrow window pays off one level down, in
        // the range read out of each source, which is the `(source_id, t)` B-tree the design
        // decomposes onto.
        assert!(
            tick_work.events_read * 8 < full.events_read,
            "a tick read {} events of the {} a full sweep did",
            tick_work.events_read,
            full.events_read,
        );
        assert!(tick_work.pruned > 0, "nothing was dropped on a bound");
    }

    /// A strength floor prunes subtrees rather than filtering output: a node whose nearest
    /// possible source is too far to hear cannot contain one that is not.
    #[test]
    fn a_strength_floor_prunes_rather_than_filters() {
        let (bvh, memory) = world(400, 3);
        let observer = Static::new(DVec3::ZERO);
        let (from_t, to_t) = (-2_000.0, 20_000.0);

        let mut open = LightConeCursor::with_floor(&bvh, &memory, &observer, from_t, to_t, 0.0);
        let everything: Vec<Reception> = open.by_ref().collect();
        let open_work = open.work();

        let floor = 1.0e-6;
        let mut gated = LightConeCursor::with_floor(&bvh, &memory, &observer, from_t, to_t, floor);
        let loud: Vec<Reception> = gated.by_ref().collect();
        let gated_work = gated.work();

        let want = brute_force(&bvh, &memory, &observer, from_t, to_t, floor);
        assert_eq!(loud.len(), want.len(), "the floor changed what is audible, not just how much");
        assert!(loud.len() < everything.len(), "the floor excluded nothing, so nothing was tested");
        assert!(loud.iter().all(|r| r.strength >= floor));
        assert!(
            gated_work.pruned > open_work.pruned,
            "the floor filtered instead of pruning: {gated_work:?} against {open_work:?}",
        );
    }

    /// The window is on arrival, not emission. An event emitted before the window can land
    /// inside it and has to; one emitted inside it can arrive after and must not.
    #[test]
    fn the_window_is_on_arrival_and_not_on_emission() {
        let far = DVec3::new(5_000.0, 0.0, 0.0);
        let bvh = Bvh::build(vec![SourceNode {
            source_id: 1,
            position: far,
            first_t: -10_000.0,
            last_t: 10_000.0,
            power: 1.0,
        }]);
        let mut memory = Memory::default();
        memory.by_source.insert(
            1,
            vec![
                // Emitted long before the window, arriving inside it.
                SourceEvent { event_id: EventId::new(1, 0, 0).unwrap(), t: -4_000.0, power: 1.0 },
                // Emitted inside the window, arriving long after it.
                SourceEvent { event_id: EventId::new(2, 0, 0).unwrap(), t: 500.0, power: 1.0 },
            ],
        );
        let observer = Static::new(DVec3::ZERO);
        let got: Vec<Reception> =
            LightConeCursor::new(&bvh, &memory, &observer, 0.0, 2_000.0).collect();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].event_id, EventId::new(1, 0, 0).unwrap());
        assert!((got[0].arrive_t - 1_000.0).abs() < 1e-6, "{}", got[0].arrive_t);
    }

    /// The direction points at where the source is, which is what a renderer draws along.
    #[test]
    fn a_reception_points_back_at_its_source() {
        let bvh = Bvh::build(vec![SourceNode {
            source_id: 1,
            position: DVec3::new(0.0, 300.0, 0.0),
            first_t: 0.0,
            last_t: 0.0,
            power: 4.0,
        }]);
        let mut memory = Memory::default();
        memory
            .by_source
            .insert(1, vec![SourceEvent { event_id: EventId::new(9, 0, 0).unwrap(), t: 0.0, power: 4.0 }]);
        let observer = Static::new(DVec3::ZERO);
        let got: Vec<Reception> =
            LightConeCursor::new(&bvh, &memory, &observer, 0.0, 1_000.0).collect();
        assert_eq!(got.len(), 1);
        assert!((got[0].direction - DVec3::Y).length() < 1e-12, "{:?}", got[0].direction);
        // Inverse square, from a source three hundred light-microseconds away.
        assert!((got[0].strength - 4.0 / 90_000.0).abs() < 1e-15);
    }

    #[test]
    fn an_empty_world_yields_nothing_rather_than_failing() {
        let bvh = Bvh::build(Vec::new());
        let memory = Memory::default();
        let observer = Static::new(DVec3::ZERO);
        let got: Vec<Reception> =
            LightConeCursor::new(&bvh, &memory, &observer, 0.0, 1_000.0).collect();
        assert!(got.is_empty());
    }
}
