use std::collections::HashMap;
use std::hash::Hash;
use std::slice::Iter;
use crate::util::bitfutz;

#[derive(Debug, Clone)]
pub struct TimeMap<V>
{
    map: HashMap<u64, V>,
    time_keys: SortedTimes,
}

impl<V: Clone> Default for TimeMap<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V: Clone> TimeMap<V>
{
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            time_keys: SortedTimes::new(),
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
        }
    }
}

#[derive(Debug, Clone)]
struct SortedTimes {
    vec: Vec<f64>,
}

impl SortedTimes {
    pub fn as_vec(&self) -> Vec<f64> {
        self.vec.clone()
    }
}

impl SortedTimes {
    pub fn new() -> Self {
        Self{
            vec: Vec::new()
        }
    }

    pub fn insert(&mut self, value: f64) {
        match self.vec.binary_search_by(|other| other.partial_cmp(&value).unwrap()) {
            Ok(_) => {},
            Err(pos) => self.vec.insert(pos, value),
        }
    }

    pub fn get(&self, index: usize) -> Option<&f64> {
        self.vec.get(index)
    }

    pub fn len(&self) -> usize {
        self.vec.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vec.is_empty()
    }

    pub fn remove(&mut self, index: usize) -> Option<f64> {
        if index < self.vec.len() {
            Some(self.vec.remove(index))
        } else {
            None
        }
    }

    pub fn range(&self, start: f64, end: f64) -> SortedTimes {
        let values = self.vec.iter()
            .filter(|&&x| x >= start && x <= end)
            .cloned()
            .collect();
        Self {
            vec: values,
        }
    }

    pub fn iter(&self) -> Iter<'_, f64> {
        self.vec.iter()
    }
}
