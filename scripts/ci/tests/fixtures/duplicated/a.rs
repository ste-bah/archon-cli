// Fixture file for jscpd duplicate-detection self-test.
// a.rs and b.rs are intentionally identical to trigger duplication report.
use std::collections::HashMap;

pub struct DataProcessor {
    cache: HashMap<String, Vec<u64>>,
    threshold: u64,
    max_entries: usize,
    name: String,
}

impl DataProcessor {
    pub fn new(name: String, threshold: u64, max_entries: usize) -> Self {
        Self {
            cache: HashMap::new(),
            threshold,
            max_entries,
            name,
        }
    }

    pub fn insert(&mut self, key: String, value: u64) {
        let entry = self.cache.entry(key).or_insert_with(Vec::new);
        if entry.len() < self.max_entries {
            entry.push(value);
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Vec<u64>> {
        self.cache.remove(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.cache.contains_key(key)
    }

    pub fn count_exceeding(&self) -> usize {
        self.cache
            .values()
            .flat_map(|v| v.iter())
            .filter(|&&n| n > self.threshold)
            .count()
    }

    pub fn sum_all(&self) -> u64 {
        self.cache
            .values()
            .flat_map(|v| v.iter())
            .sum()
    }

    pub fn max_value(&self) -> Option<u64> {
        self.cache
            .values()
            .flat_map(|v| v.iter())
            .copied()
            .max()
    }

    pub fn min_value(&self) -> Option<u64> {
        self.cache
            .values()
            .flat_map(|v| v.iter())
            .copied()
            .min()
    }

    pub fn average(&self) -> Option<f64> {
        let values: Vec<u64> = self.cache.values().flat_map(|v| v.iter().copied()).collect();
        if values.is_empty() {
            return None;
        }
        let total: u64 = values.iter().sum();
        Some(total as f64 / values.len() as f64)
    }

    pub fn clear(&mut self) {
        self.cache.clear();
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
