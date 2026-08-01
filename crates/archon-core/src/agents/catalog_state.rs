use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use tracing::warn;

use super::catalog_immutable::ImmutableCatalogSnapshot;
use super::catalog_types::{AgentKey, CatalogSnapshot, DiscoveryError};
use crate::agents::metadata::AgentMetadata;

/// In-memory catalog with a serialized staging state and published snapshots.
pub struct DiscoveryCatalog {
    staging: Mutex<ImmutableCatalogSnapshot>,
    cached_snapshot: ArcSwap<ImmutableCatalogSnapshot>,
}

/// The accepted and rejected records from one bulk insertion.
#[derive(Debug, Default)]
pub struct BulkInsertResult {
    pub accepted: AcceptedInsertCounts,
    pub rejected: Vec<BulkInsertRejection>,
}

/// State counts for records accepted by one bulk insertion.
#[derive(Debug, Default)]
pub struct AcceptedInsertCounts {
    pub loaded: usize,
    pub invalid: usize,
}

/// A metadata record rejected from one bulk insertion with its cause.
#[derive(Debug)]
pub struct BulkInsertRejection {
    pub metadata: AgentMetadata,
    pub error: DiscoveryError,
}

fn count_accepted_metadata(counts: &mut AcceptedInsertCounts, meta: &AgentMetadata) {
    match &meta.state {
        crate::agents::metadata::AgentState::Valid | crate::agents::metadata::AgentState::Stale => {
            counts.loaded += 1;
        }
        crate::agents::metadata::AgentState::Invalid(_) => counts.invalid += 1,
    }
}

fn decrement_accepted_count(counts: &mut AcceptedInsertCounts, meta: &AgentMetadata) {
    match &meta.state {
        crate::agents::metadata::AgentState::Valid | crate::agents::metadata::AgentState::Stale => {
            counts.loaded -= 1;
        }
        crate::agents::metadata::AgentState::Invalid(_) => counts.invalid -= 1,
    }
}

fn metadata_key(meta: &AgentMetadata) -> AgentKey {
    (meta.name.clone(), meta.version.clone())
}

impl DiscoveryCatalog {
    /// Create an empty catalog.
    pub fn new() -> Self {
        Self {
            staging: Mutex::new(ImmutableCatalogSnapshot::default()),
            cached_snapshot: ArcSwap::from_pointee(ImmutableCatalogSnapshot::default()),
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
    /// Invalid entries are rejected individually; accepted entries share one publication.
    pub fn insert_all(&self, entries: Vec<AgentMetadata>) -> BulkInsertResult {
        let mut accepted = Vec::with_capacity(entries.len());
        let mut rejected = Vec::new();
        let mut accepted_counts = AcceptedInsertCounts::default();
        for meta in entries {
            match self.validate_metadata(&meta) {
                Ok(()) => {
                    count_accepted_metadata(&mut accepted_counts, &meta);
                    accepted.push(meta);
                }
                Err(error) => rejected.push(BulkInsertRejection {
                    metadata: meta,
                    error,
                }),
            }
        }
        if !accepted.is_empty() {
            let mut staging = self.staging.lock().expect("catalog staging lock poisoned");
            for meta in accepted {
                match self.insert_bulk_staged(&mut staging, meta) {
                    Ok(()) => {}
                    Err(rejected_insert) => {
                        decrement_accepted_count(&mut accepted_counts, &rejected_insert.metadata);
                        rejected.push(*rejected_insert);
                    }
                }
            }
            self.publish(&staging);
        }
        BulkInsertResult {
            accepted: accepted_counts,
            rejected,
        }
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

    fn insert_staged(&self, staging: &mut ImmutableCatalogSnapshot, meta: AgentMetadata) {
        if let Some(existing) = staging.get(&metadata_key(&meta)).cloned() {
            if existing.source_path != meta.source_path {
                warn!(
                    "agent collision: name={} version={} existing={:?} ignored={:?}",
                    meta.name, meta.version, existing.source_path, meta.source_path
                );
                return;
            }
            staging.remove_memberships(&metadata_key(&meta), &existing);
        }
        staging.insert(metadata_key(&meta), meta);
    }

    /// Inserts bulk metadata while preserving rejected contenders for reporting.
    fn insert_bulk_staged(
        &self,
        staging: &mut ImmutableCatalogSnapshot,
        meta: AgentMetadata,
    ) -> Result<(), Box<BulkInsertRejection>> {
        if let Some(existing) = staging.get(&metadata_key(&meta)).cloned() {
            if existing.source_path != meta.source_path {
                return Err(Box::new(BulkInsertRejection {
                    error: DiscoveryError::DuplicateAgent {
                        name: meta.name.clone(),
                        version: meta.version.clone(),
                        existing_path: existing.source_path,
                        rejected_path: meta.source_path.clone(),
                    },
                    metadata: meta,
                }));
            }
            staging.remove_memberships(&metadata_key(&meta), &existing);
        }
        staging.insert(metadata_key(&meta), meta);
        Ok(())
    }

    fn publish(&self, staging: &ImmutableCatalogSnapshot) {
        self.cached_snapshot.store(Arc::new(staging.clone()));
    }

    /// Look up a specific entry from one published snapshot.
    pub fn get(&self, key: &AgentKey) -> Option<AgentMetadata> {
        self.snapshot_immutable().get(key).cloned()
    }

    /// Return the current immutable catalog snapshot without conversion.
    pub fn snapshot_immutable(&self) -> Arc<ImmutableCatalogSnapshot> {
        self.cached_snapshot.load_full()
    }

    /// Return a mutable legacy compatibility snapshot converted from publication.
    ///
    /// This allocates and copies every published index. Mutating the returned
    /// snapshot never affects this catalog, the current immutable publication,
    /// or any future snapshot.
    #[deprecated(note = "use snapshot_immutable() to avoid compatibility conversion")]
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        Arc::new(self.snapshot_immutable().to_legacy())
    }

    /// Number of entries in one published snapshot.
    pub fn len(&self) -> usize {
        self.snapshot_immutable().len()
    }

    /// Whether the catalog is empty.
    pub fn is_empty(&self) -> bool {
        self.snapshot_immutable().is_empty()
    }
}

impl Default for DiscoveryCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod representation_tests {
    use super::*;

    #[test]
    fn staging_uses_immutable_snapshot_representation() {
        let catalog = DiscoveryCatalog::new();
        let _: &Mutex<ImmutableCatalogSnapshot> = &catalog.staging;
    }
}
