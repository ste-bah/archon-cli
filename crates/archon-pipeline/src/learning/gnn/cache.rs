//! LRU cache with TTL for GNN forward pass results.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

/// A cached forward pass result.
struct CacheEntry {
    output: Vec<f32>,
    last_access: Instant,
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub entries: usize,
    pub capacity: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub hit_rate: f64,
}

/// Inner mutable state for the cache.
struct CacheInner {
    entries: HashMap<u64, CacheEntry>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

/// LRU cache with TTL for GNN forward pass results.
///
/// Uses `RefCell` for interior mutability so `GNNEnhancer::enhance(&self)` can
/// update the cache without requiring `&mut self`.
pub struct GNNCache {
    inner: RefCell<CacheInner>,
    capacity: usize,
    ttl_secs: u64,
}

impl GNNCache {
    /// Create a new cache with given capacity and TTL in seconds.
    pub fn new(capacity: usize, ttl_secs: u64) -> Self {
        Self {
            inner: RefCell::new(CacheInner {
                entries: HashMap::new(),
                hits: 0,
                misses: 0,
                evictions: 0,
            }),
            capacity,
            ttl_secs,
        }
    }

    /// Look up a cached result for the given input.
    pub fn get(&self, input: &[f32]) -> Option<Vec<f32>> {
        let key = Self::hash_input(input);
        let mut inner = self.inner.borrow_mut();
        let result = if let Some(entry) = inner.entries.get(&key) {
            if entry.last_access.elapsed().as_secs() > self.ttl_secs {
                None
            } else {
                Some(entry.output.clone())
            }
        } else {
            None
        };
        if result.is_some() {
            inner.hits += 1;
        } else {
            inner.misses += 1;
        }
        result
    }

    /// Store a result in the cache. Evicts LRU entries if at capacity.
    pub fn put(&self, input: &[f32], output: &[f32]) {
        let key = Self::hash_input(input);
        let mut inner = self.inner.borrow_mut();

        // Evict expired entries first
        let ttl = self.ttl_secs;
        let before = inner.entries.len();
        inner
            .entries
            .retain(|_, entry| entry.last_access.elapsed().as_secs() <= ttl);
        inner.evictions += (before - inner.entries.len()) as u64;

        // Evict LRU if at capacity
        while inner.entries.len() >= self.capacity {
            let lru_key = inner
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access)
                .map(|(k, _)| *k);
            if let Some(k) = lru_key {
                inner.entries.remove(&k);
                inner.evictions += 1;
            } else {
                break;
            }
        }

        inner.entries.insert(
            key,
            CacheEntry {
                output: output.to_vec(),
                last_access: Instant::now(),
            },
        );
    }

    /// Manually evict all entries.
    pub fn clear(&self) {
        self.inner.borrow_mut().entries.clear();
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let inner = self.inner.borrow();
        let total = inner.hits + inner.misses;
        CacheStats {
            entries: inner.entries.len(),
            capacity: self.capacity,
            hits: inner.hits,
            misses: inner.misses,
            evictions: inner.evictions,
            hit_rate: if total > 0 {
                inner.hits as f64 / total as f64
            } else {
                0.0
            },
        }
    }

    /// Hash an f32 slice to a u64 key using FNV-1a.
    fn hash_input(input: &[f32]) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for val in input {
            let bytes = val.to_le_bytes();
            for b in &bytes {
                hash ^= *b as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        hash
    }
}
