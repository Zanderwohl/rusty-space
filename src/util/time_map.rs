use std::collections::HashMap;
use std::slice::Iter;
use bevy::math::DVec3;
use crate::util::bitfutz;

#[derive(Debug, Clone)]
pub struct TimeMap<V>
{
    map: HashMap<u64, V>,
    time_keys: SortedTimes,
    periodicity: Option<Periodicity>,
}

#[derive(Debug, Clone)]
pub struct Periodicity {
    interval_start: f64,
    interval_size: f64,
}

impl<V: Clone> Default for TimeMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

trait Interpolate {
    fn interpolate(&self, other: &Self, t: f64) -> Self;
}

impl Interpolate for DVec3 {
    fn interpolate(&self, other: &Self, t: f64) -> Self
    {
        let t: f64 = t.into();
        self.lerp(*other, t)
    }
}

impl<V: Clone> TimeMap<V>
{
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            time_keys: SortedTimes::new(),
            periodicity: None,
        }
    }

    pub fn insert(&mut self, time: f64, item: V) {
        self.time_keys.insert(time);
        self.map.insert(bitfutz::f64::to_u64(time), item);
    }

    pub fn get(&self, time: f64) -> Option<&V> {
        let key = bitfutz::f64::to_u64(time);
        self.map.get(&key)
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
