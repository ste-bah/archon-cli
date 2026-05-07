use std::sync::atomic::{AtomicBool, AtomicU64};

use super::cache::{CacheConfig, GnnCacheManager};
use super::types::GnnConfig;
use super::weights::WeightStore;

/// 3-layer graph attention network for embedding enhancement.
///
/// Uses graph attention (scaled dot-product + adjacency-weighted softmax),
/// residual connections, and layer normalization. Matches root TS behaviour.
pub struct GnnEnhancer {
    pub(super) config: GnnConfig,
    pub(super) cache: GnnCacheManager,
    pub(super) weights: WeightStore,
    pub(super) weight_seed: u64,
    pub(super) weights_loaded: AtomicBool,
    pub(super) total_enhancements: AtomicU64,
    pub(super) total_cache_hits: AtomicU64,
    pub(super) total_enhancement_micros: AtomicU64,
}

impl GnnEnhancer {
    /// Create a new GnnEnhancer with the given config, cache config, seed, and weight store.
    pub fn new(
        config: GnnConfig,
        cache_config: CacheConfig,
        weight_seed: u64,
        weights: WeightStore,
    ) -> Self {
        let enhancer = Self {
            config,
            cache: GnnCacheManager::new(cache_config),
            weights,
            weight_seed,
            weights_loaded: AtomicBool::new(false),
            total_enhancements: AtomicU64::new(0),
            total_cache_hits: AtomicU64::new(0),
            total_enhancement_micros: AtomicU64::new(0),
        };
        enhancer.initialize_layer_weights();
        enhancer
    }

    /// Create a GnnEnhancer with in-memory weights (for tests that don't need persistence).
    pub fn with_in_memory_weights(
        config: GnnConfig,
        cache_config: CacheConfig,
        weight_seed: u64,
    ) -> Self {
        Self::new(
            config,
            cache_config,
            weight_seed,
            WeightStore::with_in_memory(),
        )
    }
}
