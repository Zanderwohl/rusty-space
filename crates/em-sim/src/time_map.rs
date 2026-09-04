//! A sparse, interpolating map from time to value, and the sorted key list behind it.
//!
//! Keys are typed rather than raw floats. `SortedTimes<Instant>` holds absolute event
//! times (see [`crate::motive::Motive`]); [`TimeMap`] is keyed by [`TimeDelta`], since a
//! trajectory's samples are offsets from its periapsis passage rather than absolute
//! instants.
//!
//! Both used to key a `HashMap` on `f64::to_bits` and binary-search with
//! `partial_cmp().unwrap()`. That panicked on NaN, and silently split `-0.0` from `0.0`
//! into separate entries. `Instant` and `TimeDelta` are `Ord + Eq + Hash` with negative
//! zero normalised, so neither is possible now.

use std::collections::HashMap;
use std::slice::Iter;
use glam::{DVec3, Vec3};
use serde::{Deserialize, Serialize};
use em_foundations::time::{Instant, TimeDelta};

#[derive(Debug, Clone)]
pub struct TimeMap<V: Lerpable> {
    map: HashMap<TimeDelta, V>,
    time_keys: SortedTimes<TimeDelta>,
    periodicity: Option<Periodicity>,
}

#[derive(Debug, Clone, Copy)]
pub struct Periodicity {
    /// Absolute time the cycle starts from — for an orbit, a periapsis passage.
    pub interval_start: Instant,
    /// One full cycle.
    pub interval_size: TimeDelta,
}

pub trait Lerpable {
    fn lerp__(&self, rhs: &Self, t: f64) -> Self;
}

impl Lerpable for f64 {
    fn lerp__(&self, rhs: &Self, t: f64) -> Self {
        self + (rhs - self) * t
    }
}

impl Lerpable for f32 {
    fn lerp__(&self, rhs: &Self, t: f64) -> Self {
        let t = t as f32;
        self + (rhs - self) * t
    }
}

impl Lerpable for Vec3 {
    fn lerp__(&self, rhs: &Self, t: f64) -> Self {
        self.lerp(*rhs, t as f32)
    }
}

impl Lerpable for DVec3 {
    fn lerp__(&self, rhs: &Self, t: f64) -> Self {
        self.lerp(*rhs, t)
    }
}

impl Periodicity {
    /// How far through the current cycle `time` falls, in `[0, 1)`.
    pub fn cycle_fraction(&self, time: Instant) -> f64 {
        let size = self.interval_size.to_seconds();
        if size == 0.0 {
            return 0.0;
        }
        let elapsed = (time - self.interval_start).to_seconds();
        // rem_euclid keeps the result non-negative for times before interval_start.
        (elapsed.rem_euclid(size)) / size
    }
}

impl<V: Clone + Lerpable> Default for TimeMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone + Lerpable> TimeMap<V> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            time_keys: SortedTimes::new(),
            periodicity: None,
        }
    }

    pub fn len(&self) -> usize {
        self.time_keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.time_keys.is_empty()
    }

    pub fn insert(&mut self, time: TimeDelta, item: V) {
        self.time_keys.insert(time);
        self.map.insert(time, item);
    }

    pub fn get(&self, time: TimeDelta) -> Option<&V> {
        self.map.get(&time)
    }

    /// The value at `time`, interpolating between the two surrounding samples when
    /// there is no exact match. `None` outside the sampled range.
    pub fn get_lerp(&self, time: TimeDelta) -> Option<V> {
        if let Some(item) = self.get(time) {
            return Some(item.clone());
        }

        let (a, b) = self.time_keys.get_pair_that_surrounds(time)?;
        let span = (b - a).to_seconds();
        if span == 0.0 {
            return self.get(a).cloned();
        }
        let t = (time - a).to_seconds() / span;
        Some(self.get(a)?.lerp__(self.get(b)?, t))
    }

    pub fn times(&self) -> Vec<TimeDelta> {
        self.time_keys.as_vec()
    }

    pub fn range(&self, start_time: TimeDelta, end_time: TimeDelta) -> TimeMap<V> {
        let restricted_times = self.time_keys.range(start_time, end_time);
        let mut map = HashMap::new();

        for key in restricted_times.iter() {
            if let Some(value) = self.map.get(key) {
                map.insert(*key, value.clone());
            }
        }

        Self {
            map,
            time_keys: restricted_times,
            periodicity: self.periodicity,
        }
    }

    pub fn is_periodic(&self) -> bool {
        self.periodicity.is_some()
    }

    pub fn set_periodicity(&mut self, interval_start: Instant, interval_size: TimeDelta) {
        self.periodicity = Some(Periodicity { interval_start, interval_size });
    }

    pub fn periodicity(&self) -> Option<&Periodicity> {
        self.periodicity.as_ref()
    }

    /// One cycle's worth of samples.
    ///
    /// Keys are offsets from `interval_start`, so this is `[0, interval_size]`. The old
    /// version passed `interval_start` — an absolute instant — as a key bound, which
    /// mixed the two frames; the typed keys make that a compile error. It had no callers.
    pub fn range_one_period(&self) -> Option<TimeMap<V>> {
        let p = self.periodicity?;
        Some(self.range(TimeDelta::ZERO, p.interval_size))
    }

    pub fn iter(&self) -> impl Iterator<Item = (TimeDelta, &V)> {
        self.time_keys
            .iter()
            .filter_map(move |k| self.map.get(k).map(|v| (*k, v)))
    }
}

/// A sorted, deduplicated list of keys supporting binary-search lookups.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortedTimes<K = Instant> {
    in_order: Vec<K>,
}

impl<K: Ord + Copy> Default for SortedTimes<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord + Copy> SortedTimes<K> {
    pub fn new() -> Self {
        Self { in_order: Vec::new() }
    }

    pub fn as_vec(&self) -> Vec<K> {
        self.in_order.clone()
    }

    /// Insert, keeping the list sorted. A duplicate is a no-op.
    pub fn insert(&mut self, value: K) {
        if let Err(pos) = self.in_order.binary_search(&value) {
            self.in_order.insert(pos, value);
        }
    }

    pub fn has(&self, time: K) -> bool {
        self.in_order.binary_search(&time).is_ok()
    }

    pub fn remove_time(&mut self, time: K) -> bool {
        match self.in_order.binary_search(&time) {
            Ok(pos) => {
                self.in_order.remove(pos);
                true
            }
            Err(_) => false,
        }
    }

    pub fn get(&self, index: usize) -> Option<&K> {
        self.in_order.get(index)
    }

    /// The two keys bracketing `value`, or `None` if it falls outside the range or
    /// there are fewer than two keys to interpolate between.
    pub fn get_pair_that_surrounds(&self, value: K) -> Option<(K, K)> {
        let len = self.in_order.len();
        if len < 2 {
            return None;
        }
        if value < self.in_order[0] || value > self.in_order[len - 1] {
            return None;
        }

        let insertion_point = self.in_order.partition_point(|x| *x <= value);

        if insertion_point == 0 {
            Some((self.in_order[0], self.in_order[1]))
        } else if insertion_point == len {
            Some((self.in_order[len - 2], self.in_order[len - 1]))
        } else {
            Some((self.in_order[insertion_point - 1], self.in_order[insertion_point]))
        }
    }

    /// The greatest key `<= time`.
    pub fn get_at_or_before(&self, time: K) -> Option<K> {
        let insertion_point = self.in_order.partition_point(|x| *x <= time);
        if insertion_point == 0 {
            None
        } else {
            Some(self.in_order[insertion_point - 1])
        }
    }

    /// The greatest key strictly `< time`.
    pub fn get_before(&self, time: K) -> Option<K> {
        let insertion_point = self.in_order.partition_point(|x| *x < time);
        if insertion_point == 0 {
            None
        } else {
            Some(self.in_order[insertion_point - 1])
        }
    }

    /// Index of the first key strictly greater than `time`, or `len()` if there is none.
    pub fn get_index_after(&self, time: K) -> usize {
        self.in_order.partition_point(|x| *x <= time)
    }

    pub fn len(&self) -> usize {
        self.in_order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.in_order.is_empty()
    }

    pub fn remove(&mut self, index: usize) -> Option<K> {
        if index < self.in_order.len() {
            Some(self.in_order.remove(index))
        } else {
            None
        }
    }

    pub fn remove_after(&mut self, index: usize) -> Vec<K> {
        if index >= self.in_order.len() {
            return Vec::new();
        }
        self.in_order.drain(index..).collect()
    }

    pub fn range(&self, start: K, end: K) -> SortedTimes<K> {
        Self {
            in_order: self
                .in_order
                .iter()
                .filter(|&&x| x >= start && x <= end)
                .copied()
                .collect(),
        }
    }

    pub fn iter(&self) -> Iter<'_, K> {
        self.in_order.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn td(s: f64) -> TimeDelta { TimeDelta::from_seconds(s) }

    #[test]
    fn sorted_times_stays_sorted_and_deduplicates() {
        let mut t = SortedTimes::new();
        for v in [5.0, -3.0, 5.0, 1.0, 0.0] {
            t.insert(td(v));
        }
        assert_eq!(t.as_vec().iter().map(|x| x.to_seconds()).collect::<Vec<_>>(),
                   vec![-3.0, 0.0, 1.0, 5.0]);
        assert!(t.has(td(1.0)));
        assert!(!t.has(td(2.0)));
    }

    /// The old implementation keyed on `f64::to_bits`, so these landed in two slots.
    #[test]
    fn negative_zero_is_the_same_key_as_zero() {
        let mut t = SortedTimes::new();
        t.insert(td(0.0));
        t.insert(td(-0.0));
        assert_eq!(t.len(), 1);

        let mut m: TimeMap<f64> = TimeMap::new();
        m.insert(td(-0.0), 1.0);
        m.insert(td(0.0), 2.0);
        assert_eq!(m.len(), 1);
        assert_eq!(m.get(td(0.0)), Some(&2.0));
    }

    #[test]
    fn lookups_bracket_correctly() {
        let mut t = SortedTimes::new();
        for v in [0.0, 10.0, 20.0] { t.insert(td(v)); }

        assert_eq!(t.get_at_or_before(td(10.0)).map(|x| x.to_seconds()), Some(10.0));
        assert_eq!(t.get_at_or_before(td(15.0)).map(|x| x.to_seconds()), Some(10.0));
        assert_eq!(t.get_at_or_before(td(-1.0)), None);

        assert_eq!(t.get_before(td(10.0)).map(|x| x.to_seconds()), Some(0.0));
        assert_eq!(t.get_before(td(0.0)), None);

        assert_eq!(t.get_index_after(td(-1.0)), 0);
        assert_eq!(t.get_index_after(td(10.0)), 2);
        assert_eq!(t.get_index_after(td(99.0)), 3);

        let (a, b) = t.get_pair_that_surrounds(td(15.0)).unwrap();
        assert_eq!((a.to_seconds(), b.to_seconds()), (10.0, 20.0));
        assert!(t.get_pair_that_surrounds(td(25.0)).is_none());
        assert!(t.get_pair_that_surrounds(td(-1.0)).is_none());
    }

    #[test]
    fn interpolates_between_samples() {
        let mut m: TimeMap<f64> = TimeMap::new();
        m.insert(td(0.0), 0.0);
        m.insert(td(10.0), 100.0);
        assert_eq!(m.get_lerp(td(0.0)), Some(0.0));
        assert_eq!(m.get_lerp(td(10.0)), Some(100.0));
        assert_eq!(m.get_lerp(td(2.5)), Some(25.0));
        assert_eq!(m.get_lerp(td(11.0)), None);
    }

    #[test]
    fn cycle_fraction_wraps_in_both_directions() {
        let p = Periodicity {
            interval_start: Instant::from_seconds_since_j2000(100.0),
            interval_size: TimeDelta::from_seconds(10.0),
        };
        assert_eq!(p.cycle_fraction(Instant::from_seconds_since_j2000(100.0)), 0.0);
        assert_eq!(p.cycle_fraction(Instant::from_seconds_since_j2000(105.0)), 0.5);
        assert_eq!(p.cycle_fraction(Instant::from_seconds_since_j2000(115.0)), 0.5);
        // Before the start must still land in [0, 1).
        assert_eq!(p.cycle_fraction(Instant::from_seconds_since_j2000(95.0)), 0.5);
    }

    #[test]
    fn remove_after_past_the_end_is_empty() {
        let mut t = SortedTimes::new();
        t.insert(td(1.0));
        assert!(t.remove_after(5).is_empty());
        assert_eq!(t.len(), 1);
    }
}
