use std::collections::HashMap;
use std::slice::Iter;
use bevy::math::DVec3;
use bevy::prelude::{FloatExt, Vec3};
use crate::util::bitfutz;

#[derive(Debug, Clone)]
pub struct TimeMap<V: Lerpable>
{
    map: HashMap<u64, V>,
    time_keys: SortedTimes,
    periodicity: Option<Periodicity>,
}

#[derive(Debug, Clone)]
pub struct Periodicity {
    pub interval_start: f64,
    pub interval_size: f64,
}

pub trait Lerpable {
    fn lerp__(&self, rhs: &Self, t: f64) -> Self;
}

impl Lerpable for f64 {
    fn lerp__(&self, rhs: &Self, t: f64) -> Self {
        self.lerp(*rhs, t)
    }
}

impl Lerpable for f32 {
    fn lerp__(&self, rhs: &Self, t: f64) -> Self {
        self.lerp(*rhs, t as f32)
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
    /// Returns the fraction (0.0 to 1.0) of the way through the current cycle
    /// for the given time (seconds since J2000)
    pub fn cycle_fraction(&self, time: f64) -> f64 {
        let elapsed = time - self.interval_start;
        let position_in_cycle = elapsed % self.interval_size;
        
        // Ensure result is always positive (handle negative modulo)
        let normalized_position = if position_in_cycle < 0.0 {
            position_in_cycle + self.interval_size
        } else {
            position_in_cycle
        };
        
        normalized_position / self.interval_size
    }
}

impl<V: Clone + Lerpable> Default for TimeMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone + Lerpable> TimeMap<V>
{
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

    pub fn insert(&mut self, time: f64, item: V) {
        self.time_keys.insert(time);
        self.map.insert(bitfutz::f64::to_u64(time), item);
    }

    pub fn get(&self, time: f64) -> Option<&V> {
        let key = bitfutz::f64::to_u64(time);
        self.map.get(&key)
    }

    pub fn get_lerp(&self, time: f64) -> Option<V> {
        if let Some(item) = self.get(time) {
            return Some(item.clone());
        }

        if let Some((a, b)) = self.time_keys.get_pair_that_surrounds(time) {
            let t = (time - a) / (b - a);
            let value = self.get(a)?.lerp__(self.get(b)?, t);
            return Some(value);
        }

        return None;
    }

    pub fn times(&self) -> Vec<f64> {
        self.time_keys.as_vec()
    }

    pub fn range(&self, start_time: f64, end_time: f64) -> TimeMap<V> {
        let restricted_times = self.time_keys.range(start_time, end_time);
        let mut map = HashMap::new();

        for key in restricted_times.iter() {
            let key = bitfutz::f64::to_u64(*key);
            if let Some(value) = self.map.get(&key) {
                let value = (*value).clone();
                map.insert(key, value);
            }
        }

        Self {
            map,
            time_keys: restricted_times,
            periodicity: self.periodicity.clone(),
        }
    }

    pub fn is_periodic(&self) -> bool {
        self.periodicity.is_some()
    }

    pub fn set_periodicity(&mut self, interval_start: f64, interval_size: f64) {
        self.periodicity = Some(Periodicity {
            interval_start,
            interval_size,
        });
    }

    pub fn periodicity(&self) -> Option<&Periodicity> {
        self.periodicity.as_ref()
    }

    pub fn range_one_period(&self) -> Option<TimeMap<V>> {
        match &self.periodicity {
            None => None,
            Some(periodicity) => {
                Some(self.range(periodicity.interval_start, periodicity.interval_start + periodicity.interval_size))
            }
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (f64, &V)> {
        self.time_keys.in_order
            .iter()
            .map(|f| bitfutz::f64::to_u64(*f))
            .filter_map(move |k| self.map.get(&k).map(|v| (bitfutz::u64::to_f64(k), v)))
    }
}

#[derive(Debug, Clone)]
struct SortedTimes {
    in_order: Vec<f64>,
}

impl SortedTimes {
    pub fn as_vec(&self) -> Vec<f64> {
        self.in_order.clone()
    }
}

impl SortedTimes {
    pub fn new() -> Self {
        Self{
            in_order: Vec::new()
        }
    }

    pub fn insert(&mut self, value: f64) {
        match self.in_order.binary_search_by(|other| other.partial_cmp(&value).unwrap()) {
            Ok(_) => {},
            Err(pos) => self.in_order.insert(pos, value),
        }
    }

    pub fn get(&self, index: usize) -> Option<&f64> {
        self.in_order.get(index)
    }

    pub fn get_pair_that_surrounds(&self, value: f64) -> Option<(f64, f64)> {
        if self.in_order.len() < 2 { // If too short, we can't really interpolate anything
            return None;
        }

        if value < self.in_order[0] || value > self.in_order[self.in_order.len() - 1] { // outside bounds
            return None;
        }

        // Binary search for the insertion point where value would go
        let insertion_point = self.in_order.partition_point(|&x| x <= value);

        if insertion_point == 0 {
            // This shouldn't happen given our bounds check
            None
        } else if insertion_point == self.in_order.len() {
            // value equals the last element, return last two values
            let len = self.in_order.len();
            Some((self.in_order[len-2], self.in_order[len-1]))
        } else {
            // Normal case: value is between two elements
            Some((self.in_order[insertion_point-1], self.in_order[insertion_point]))
        }
    }

    pub fn len(&self) -> usize {
        self.in_order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.in_order.is_empty()
    }

    pub fn remove(&mut self, index: usize) -> Option<f64> {
        if index < self.in_order.len() {
            Some(self.in_order.remove(index))
        } else {
            None
        }
    }

    pub fn range(&self, start: f64, end: f64) -> SortedTimes {
        let values = self.in_order.iter()
            .filter(|&&x| x >= start && x <= end)
            .cloned()
            .collect();
        Self {
            in_order: values,
        }
    }

    pub fn iter(&self) -> Iter<'_, f64> {
        self.in_order.iter()
    }
}
