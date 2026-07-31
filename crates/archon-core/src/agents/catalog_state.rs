use std::sync::Arc;

use arc_swap::ArcSwap;
use tracing::warn;

use super::catalog_types::{AgentKey, CatalogSnapshot, DiscoveryError};
use crate::agents::metadata::AgentMetadata;

/// In-memory catalog of discovered agents, indexed by (name, version).
///
/// DashMaps provide concurrent-safe O(1) inserts and reads. The `snapshot()`
/// method clones the current state into an Arc for torn-read-free iteration.
/// Thread-safe for concurrent inserts without locks.
pub struct DiscoveryCatalog {
    pub(super) live: CatalogSnapshot,
    /// Cached snapshot for readers — updated on each insert via ArcSwap.
    cached_snapshot: ArcSwap<CatalogSnapshot>,
}

impl DiscoveryCatalog {
    /// Create an empty catalog.
    pub fn new() -> Self {
        Self {
            live: CatalogSnapshot::default(),
            cached_snapshot: ArcSwap::from_pointee(CatalogSnapshot::default()),
        }
    }

    /// Insert a metadata entry. Rejects entries > 10 MB (EC-DISCOVERY-006).
    /// On same (name, version) collision with different source_path, keeps
    /// first entry and logs WARN (per TECH-AGS-DISCOVERY versioning note).
    pub fn insert(&self, meta: AgentMetadata) -> Result<(), DiscoveryError> {
        let serialized =
            serde_json::to_vec(&meta).map_err(|e| DiscoveryError::Parse(e.to_string()))?;
        if serialized.len() > 10 * 1024 * 1024 {
            return Err(DiscoveryError::MetadataTooLarge {
                path: meta.source_path.clone(),
                size: serialized.len(),
            });
        }

        let key: AgentKey = (meta.name.clone(), meta.version.clone());
        if let Some(existing) = self.live.entries.get(&key)
            && existing.source_path != meta.source_path
        {
            warn!(
                "agent collision: name={} version={} existing={:?} ignored={:?}",
                meta.name, meta.version, existing.source_path, meta.source_path
            );
            return Ok(());
        }

        self.live
            .name_index
            .entry(meta.name.clone())
            .or_default()
            .insert(meta.version.clone());
        for tag in &meta.tags {
            self.live
                .tag_index
                .entry(tag.clone())
                .or_default()
                .insert(key.clone());
        }
        for cap in &meta.capabilities {
            self.live
                .capability_index
                .entry(cap.clone())
                .or_default()
                .insert(key.clone());
        }

        self.live.entries.insert(key, meta);
        Ok(())
    }

    /// Look up a specific (name, version) entry.
    pub fn get(&self, key: &AgentKey) -> Option<AgentMetadata> {
        self.live.entries.get(key).map(|e| e.value().clone())
    }

    /// Return a frozen snapshot of the current catalog state.
    /// Clones the live DashMaps into a new Arc for torn-read-free access.
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        let snap = self.live.clone();
        let arc = Arc::new(snap);
        self.cached_snapshot.store(arc.clone());
        arc
    }

    /// Number of entries in the catalog.
    pub fn len(&self) -> usize {
        self.live.entries.len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for DiscoveryCatalog {
    fn default() -> Self {
        Self::new()
    }
}
