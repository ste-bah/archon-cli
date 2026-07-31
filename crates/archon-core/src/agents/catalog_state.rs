use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use tracing::warn;

use super::catalog_types::{AgentKey, CatalogSnapshot, DiscoveryError};
use crate::agents::metadata::AgentMetadata;

/// In-memory catalog with a serialized staging state and published snapshots.
pub struct DiscoveryCatalog {
    staging: Mutex<CatalogSnapshot>,
    cached_snapshot: ArcSwap<CatalogSnapshot>,
}

impl DiscoveryCatalog {
    /// Create an empty catalog.
    pub fn new() -> Self {
        Self {
            staging: Mutex::new(CatalogSnapshot::default()),
            cached_snapshot: ArcSwap::from_pointee(CatalogSnapshot::default()),
        }
    }

    /// Insert an entry and publish the resulting complete snapshot.
    pub fn insert(&self, meta: AgentMetadata) -> Result<(), DiscoveryError> {
        self.validate_metadata(&meta)?;
        let mut staging = self.staging.lock().expect("catalog staging lock poisoned");
        self.insert_staged(&mut staging, meta);
        self.publish(&staging);
        Ok(())
    }

    /// Insert entries under one writer lock and publish once for bulk discovery.
    pub fn insert_all(&self, entries: Vec<AgentMetadata>) -> Result<(), DiscoveryError> {
        for meta in &entries {
            self.validate_metadata(meta)?;
        }
        let mut staging = self.staging.lock().expect("catalog staging lock poisoned");
        for meta in entries {
            self.insert_staged(&mut staging, meta);
        }
        self.publish(&staging);
        Ok(())
    }

    fn validate_metadata(&self, meta: &AgentMetadata) -> Result<(), DiscoveryError> {
        let serialized =
            serde_json::to_vec(meta).map_err(|e| DiscoveryError::Parse(e.to_string()))?;
        if serialized.len() > 10 * 1024 * 1024 {
            return Err(DiscoveryError::MetadataTooLarge {
                path: meta.source_path.clone(),
                size: serialized.len(),
            });
        }
        Ok(())
    }

    // Helpers mutate guarded staging directly and never call public APIs, avoiding writer deadlocks.
    fn insert_staged(&self, staging: &mut CatalogSnapshot, meta: AgentMetadata) {
        let key = (meta.name.clone(), meta.version.clone());
        let existing = staging.entries.get(&key).map(|entry| entry.value().clone());
        if let Some(existing) = existing {
            if existing.source_path != meta.source_path {
                warn!(
                    "agent collision: name={} version={} existing={:?} ignored={:?}",
                    meta.name, meta.version, existing.source_path, meta.source_path
                );
                return;
            }
            Self::remove_memberships(staging, &key, &existing);
        }
        staging.entries.insert(key.clone(), meta.clone());
        staging
            .name_index
            .entry(meta.name.clone())
            .or_default()
            .insert(meta.version.clone());
        Self::add_memberships(staging, &key, &meta);
    }

    fn remove_memberships(staging: &mut CatalogSnapshot, key: &AgentKey, meta: &AgentMetadata) {
        for tag in &meta.tags {
            Self::remove_membership(&staging.tag_index, tag, key);
        }
        for capability in &meta.capabilities {
            Self::remove_membership(&staging.capability_index, capability, key);
        }
    }

    fn remove_membership(
        index: &dashmap::DashMap<String, std::collections::HashSet<AgentKey>>,
        membership: &str,
        key: &AgentKey,
    ) {
        if let Some(mut bucket) = index.get_mut(membership) {
            bucket.remove(key);
            if bucket.is_empty() {
                drop(bucket);
                index.remove(membership);
            }
        }
    }

    fn add_memberships(staging: &mut CatalogSnapshot, key: &AgentKey, meta: &AgentMetadata) {
        for tag in &meta.tags {
            staging
                .tag_index
                .entry(tag.clone())
                .or_default()
                .insert(key.clone());
        }
        for capability in &meta.capabilities {
            staging
                .capability_index
                .entry(capability.clone())
                .or_default()
                .insert(key.clone());
        }
    }

    fn publish(&self, staging: &CatalogSnapshot) {
        self.cached_snapshot.store(Arc::new(staging.clone()));
    }

    /// Look up a specific entry from one published snapshot.
    pub fn get(&self, key: &AgentKey) -> Option<AgentMetadata> {
        self.snapshot()
            .entries
            .get(key)
            .map(|entry| entry.value().clone())
    }

    /// Return the current complete catalog snapshot.
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        self.cached_snapshot.load_full()
    }

    /// Number of entries in one published snapshot.
    pub fn len(&self) -> usize {
        self.snapshot().entries.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.snapshot().entries.is_empty()
    }
}

impl Default for DiscoveryCatalog {
    fn default() -> Self {
        Self::new()
    }
}
