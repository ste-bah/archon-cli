use std::sync::atomic::Ordering;

use super::GnnEnhancer;
use super::cache::CacheStats;
use super::types::GnnConfig;
use super::weights::WeightStore;

impl GnnEnhancer {
    /// Get cache statistics.
    pub fn cache_stats(&self) -> CacheStats {
        self.cache.stats()
    }

    /// Invalidate cache entries for specific node IDs.
    pub fn invalidate_nodes(&self, node_ids: &[String]) -> usize {
        self.cache.invalidate_nodes(node_ids)
    }

    /// Invalidate all cache entries.
    pub fn invalidate_all(&self) -> usize {
        self.cache.invalidate_all()
    }

    /// Clear cache and reset metrics.
    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    /// Get total enhancements count.
    pub fn total_enhancements(&self) -> u64 {
        self.total_enhancements.load(Ordering::Relaxed)
    }

    /// Get total cache hits.
    pub fn total_cache_hits(&self) -> u64 {
        self.total_cache_hits.load(Ordering::Relaxed)
    }

    /// Reset metrics counters.
    pub fn reset_metrics(&self) {
        self.total_enhancements.store(0, Ordering::Relaxed);
        self.total_cache_hits.store(0, Ordering::Relaxed);
        self.total_enhancement_micros.store(0, Ordering::Relaxed);
        self.cache.reset_metrics();
    }

    /// Get the weight seed used for initialization.
    pub fn weight_seed(&self) -> u64 {
        self.weight_seed
    }

    /// Get reference to the weight store.
    pub fn weight_store(&self) -> &WeightStore {
        &self.weights
    }

    /// Get the GNN config.
    pub fn config(&self) -> &GnnConfig {
        &self.config
    }
}
