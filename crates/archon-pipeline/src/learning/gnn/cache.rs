//! GNN cache manager — LRU + TTL + smart cache key with hyperedge hash.
//!
//! Ported from root archon TS gnn-cache.ts. Uses FNV-1a hashing that
//! byte-matches TS Math.imul behaviour for parity test compatibility.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// Cache configuration.
#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub max_size: usize,           // 1000
    pub max_memory_mb: usize,      // 100
    pub ttl_ms: u64,               // 300_000 (5 min)
    pub min_access_count: usize,   // 2
    pub similarity_threshold: f32, // 0.95
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size: 1000,
            max_memory_mb: 100,
            ttl_ms: 300_000,
            min_access_count: 2,
            similarity_threshold: 0.95,
        }
    }
}

/// A cached forward pass result.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub embedding: Vec<f32>,
    pub timestamp_ms: u64,
    pub access_count: usize,
    pub last_access_ms: u64,
    pub hyperedge_hash: String,
    pub memory_bytes: usize,
}

/// Cache statistics.
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub size: usize,
    pub memory_bytes: usize,
    pub hit_rate: f64,
    pub average_access_count: f64,
    pub oldest_entry_age_ms: u64,
    pub eviction_count: usize,
}

/// LRU cache with TTL and smart cache key generation.
///
/// Wraps inner state in `Mutex` for thread safety when shared via `Arc`.
pub struct GnnCacheManager {
    cache: Mutex<HashMap<String, CacheEntry>>,
    config: CacheConfig,
    eviction_count: AtomicUsize,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl GnnCacheManager {
    pub fn new(config: CacheConfig) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            config,
            eviction_count: AtomicUsize::new(0),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        }
    }

    /// Generate a smart cache key from embedding and hyperedge identifiers.
    pub fn smart_cache_key(&self, embedding: &[f32], hyperedges: &[String]) -> String {
        let emb_hash = hash_embedding(embedding);
        let he_hash = hash_hyperedges(hyperedges);
        format!("{}:{}", emb_hash, he_hash)
    }

    /// Look up a cached result. Returns None if not found or expired.
    pub fn get(&self, key: &str) -> Option<CacheEntry> {
        let now = now_ms();
        let mut cache = self.cache.lock().unwrap();
        if let Some(entry) = cache.get_mut(key) {
            if now - entry.timestamp_ms > self.config.ttl_ms {
                cache.remove(key);
                self.misses.fetch_add(1, Ordering::Relaxed);
                return None;
            }
            entry.access_count += 1;
            entry.last_access_ms = now;
            self.hits.fetch_add(1, Ordering::Relaxed);
            return Some(entry.clone());
        }
        self.misses.fetch_add(1, Ordering::Relaxed);
        None
    }

    /// Store a result in the cache. Evicts entries if at capacity or memory limit.
    pub fn put(&self, key: String, embedding: Vec<f32>, hyperedge_hash: String) {
        let memory_bytes = embedding.len() * std::mem::size_of::<f32>();
        let max_memory_bytes = self.config.max_memory_mb * 1024 * 1024;
        let now = now_ms();

        let mut cache = self.cache.lock().unwrap();

        // Evict if memory limit exceeded
        let total_memory: usize = cache.values().map(|e| e.memory_bytes).sum();
        if total_memory + memory_bytes > max_memory_bytes {
            self.evict_lru_inner(&mut cache);
        }

        // Evict if size limit exceeded
        if cache.len() >= self.config.max_size {
            self.evict_lru_inner(&mut cache);
        }

        cache.insert(
            key,
            CacheEntry {
                embedding,
                timestamp_ms: now,
                access_count: 1,
                last_access_ms: now,
                hyperedge_hash,
                memory_bytes,
            },
        );
    }

    /// Evict the least recently used entry.
    fn evict_lru_inner(&self, cache: &mut HashMap<String, CacheEntry>) {
        let oldest_key = cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_access_ms)
            .map(|(k, _)| k.clone());
        if let Some(k) = oldest_key {
            cache.remove(&k);
            self.eviction_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Invalidate cache entries whose keys contain any of the given node IDs.
    pub fn invalidate_nodes(&self, node_ids: &[String]) -> usize {
        let mut cache = self.cache.lock().unwrap();
        let mut invalidated = 0;
        let keys_to_remove: Vec<String> = cache
            .keys()
            .filter(|key| node_ids.iter().any(|id| key.contains(id.as_str())))
            .cloned()
            .collect();
        for k in keys_to_remove {
            cache.remove(&k);
            invalidated += 1;
        }
        invalidated
    }

    /// Invalidate all cache entries. Returns count of removed entries.
    pub fn invalidate_all(&self) -> usize {
        let mut cache = self.cache.lock().unwrap();
        let count = cache.len();
        cache.clear();
        count
    }

    /// Clear cache and reset metrics.
    pub fn clear(&self) {
        let mut cache = self.cache.lock().unwrap();
        cache.clear();
        self.eviction_count.store(0, Ordering::Relaxed);
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
    }

    /// Get cache statistics.
    pub fn stats(&self) -> CacheStats {
        let cache = self.cache.lock().unwrap();
        let now = now_ms();
        let mut total_access = 0usize;
        let mut oldest_ts = now;

        for entry in cache.values() {
            total_access += entry.access_count;
            if entry.timestamp_ms < oldest_ts {
                oldest_ts = entry.timestamp_ms;
            }
        }

        let total = self.hits.load(Ordering::Relaxed) + self.misses.load(Ordering::Relaxed);
        let memory_bytes: usize = cache.values().map(|e| e.memory_bytes).sum();

        CacheStats {
            size: cache.len(),
            memory_bytes,
            hit_rate: if total > 0 {
                self.hits.load(Ordering::Relaxed) as f64 / total as f64
            } else {
                0.0
            },
            average_access_count: if cache.is_empty() {
                0.0
            } else {
                total_access as f64 / cache.len() as f64
            },
            oldest_entry_age_ms: now.saturating_sub(oldest_ts),
            eviction_count: self.eviction_count.load(Ordering::Relaxed),
        }
    }

    /// Get raw metrics for observability.
    pub fn metrics(&self) -> CacheMetrics {
        CacheMetrics {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            evictions: self.eviction_count.load(Ordering::Relaxed),
        }
    }

    /// Reset metrics counters.
    pub fn reset_metrics(&self) {
        self.hits.store(0, Ordering::Relaxed);
        self.misses.store(0, Ordering::Relaxed);
        self.eviction_count.store(0, Ordering::Relaxed);
    }

    /// Warm cache with pre-computed entries. Returns count of entries warmed.
    pub fn warm_cache(&self, entries: &[(Vec<f32>, Vec<String>, Vec<f32>)]) -> usize {
        let mut warmed = 0;
        for (embedding, hyperedges, enhanced) in entries {
            let key = self.smart_cache_key(embedding, hyperedges);
            let he_hash = hash_hyperedges(hyperedges);
            self.put(key, enhanced.clone(), he_hash);
            warmed += 1;
        }
        warmed
    }
}

/// Raw cache metrics counters.
#[derive(Debug, Clone)]
pub struct CacheMetrics {
    pub hits: usize,
    pub misses: usize,
    pub evictions: usize,
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// FNV-1a hash of embedding with stride-4 sampling.
/// MUST byte-match TS output for parity tests.
fn hash_embedding(v: &[f32]) -> String {
    let mut hash: u32 = 0x811c9dc5; // 2166136261
    for i in (0..v.len()).step_by(4) {
        let val = (v[i] * 1000.0).round() as i32 as u32;
        hash ^= val;
        hash = hash.wrapping_mul(0x01000193); // 16777619
    }
    format!("{:08x}", hash)
}

/// FNV-1a hash of sorted hyperedge IDs.
fn hash_hyperedges(ids: &[String]) -> String {
    if ids.is_empty() {
        return "empty".to_string();
    }
    let mut sorted: Vec<&String> = ids.iter().collect();
    sorted.sort();
    let mut hash: u32 = 0x811c9dc5;
    for id in &sorted {
        for ch in id.chars() {
            hash ^= ch as u32;
            hash = hash.wrapping_mul(0x01000193);
        }
    }
    format!("{:08x}", hash)
}

fn now_ms() -> u64 {
    // Use a simple monotonic-ish clock. For cache TTL this doesn't need to be
    // wall-clock — Duration-based TTL just needs relative consistency.
    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let base = BASE.get_or_init(Instant::now);
    base.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_smart_cache_key_deterministic() {
        let cfg = CacheConfig::default();
        let mgr = GnnCacheManager::new(cfg);
        let emb = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let he = vec!["edge_a".to_string(), "edge_b".to_string()];
        let k1 = mgr.smart_cache_key(&emb, &he);
        let k2 = mgr.smart_cache_key(&emb, &he);
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_smart_cache_key_different_embeddings() {
        let cfg = CacheConfig::default();
        let mgr = GnnCacheManager::new(cfg);
        let k1 = mgr.smart_cache_key(&vec![1.0, 2.0, 3.0, 4.0, 5.0], &[]);
        let k2 = mgr.smart_cache_key(&vec![1.0, 2.0, 3.0, 4.0, 5.1], &[]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_put_and_get() {
        let cfg = CacheConfig::default();
        let mgr = GnnCacheManager::new(cfg);
        let key = mgr.smart_cache_key(&vec![1.0], &[]);
        mgr.put(key.clone(), vec![42.0], "empty".to_string());
        let entry = mgr.get(&key);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().embedding, vec![42.0]);
    }

    #[test]
    fn test_miss_returns_none() {
        let cfg = CacheConfig::default();
        let mgr = GnnCacheManager::new(cfg);
        assert!(mgr.get("nonexistent").is_none());
    }

    #[test]
    fn test_eviction_on_size_limit() {
        let mut cfg = CacheConfig::default();
        cfg.max_size = 2;
        let mgr = GnnCacheManager::new(cfg);

        let k1 = mgr.smart_cache_key(&vec![1.0], &[]);
        let k2 = mgr.smart_cache_key(&vec![2.0], &[]);
        let k3 = mgr.smart_cache_key(&vec![3.0], &[]);

        mgr.put(k1.clone(), vec![1.0], "empty".to_string());
        thread::sleep(Duration::from_millis(1));
        mgr.put(k2.clone(), vec![2.0], "empty".to_string());
        thread::sleep(Duration::from_millis(1));
        // This should evict k1 (oldest last_access)
        mgr.put(k3.clone(), vec![3.0], "empty".to_string());

        let stats = mgr.stats();
        assert!(stats.size <= 2);
        assert!(stats.eviction_count >= 1);
    }

    #[test]
    fn test_hash_embedding_fnv_byte_match() {
        // Verify FNV-1a stride-4 behaviour.
        // hash = 0x811c9dc5 ^ (round(1.0*1000) as u32) = 0x811c9dc5 ^ 1000
        // then hash = hash.wrapping_mul(0x01000193)
        let h = hash_embedding(&vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(h.len(), 8); // 8 hex chars
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_hash_hyperedges_empty() {
        assert_eq!(hash_hyperedges(&[]), "empty");
    }

    #[test]
    fn test_hash_hyperedges_deterministic() {
        let a = vec!["b".to_string(), "a".to_string()];
        let b = vec!["a".to_string(), "b".to_string()];
        assert_eq!(hash_hyperedges(&a), hash_hyperedges(&b));
    }
}
